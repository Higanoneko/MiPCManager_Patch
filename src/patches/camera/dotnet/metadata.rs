//! 极简 ECMA-335 元数据读取器。
//!
//! 目标单一：在不依赖任何 .NET 运行时的前提下，按"类型简单名 + 方法名后缀"
//! 定位某个方法，返回它的方法体 RVA 以及 `MethodDef` 行中 RVA 字段的文件偏移
//! （以便重定位后改写指向）。仅解析定位所需的表（Module..MethodDef）。

use crate::infra::pe::PeImage;
use anyhow::{Context, Result, bail, ensure};

#[inline]
fn rd_u16(d: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([d[o], d[o + 1]])
}
#[inline]
fn rd_u32(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

/// 元数据表流（`#~`）中定位方法所需的上下文。
struct Tables<'a> {
    data: &'a [u8],
    strings_off: usize,
    /// 各表（0..=6）数据区的文件偏移；None 表示该表不存在。
    table_offset: [Option<usize>; 7],
    row_count: [u32; 64],
    str_w: usize,
    blob_w: usize,
    guid_w: usize,
}

/// 定位结果。
pub struct MethodLocation {
    /// 方法体 RVA（指向方法头）。
    pub body_rva: u32,
    /// `MethodDef` 行内 RVA 字段（行首 4 字节）的文件偏移。
    pub rva_field_offset: usize,
}

impl<'a> Tables<'a> {
    fn parse(pe: &'a PeImage) -> Result<Self> {
        let data = &pe.data;
        let moff = pe.metadata_root_offset()?;
        // 元数据根：Signature(4) MajorVer(2) MinorVer(2) Reserved(4) VersionLength(4) Version(...)
        let ver_len = rd_u32(data, moff + 12) as usize;
        let mut p = moff + 16 + ver_len;
        // Flags(2) Streams(2)
        let n_streams = rd_u16(data, p + 2) as usize;
        p += 4;
        let mut tilde: Option<usize> = None;
        let mut strings_off: Option<usize> = None;
        for _ in 0..n_streams {
            let off = rd_u32(data, p) as usize;
            let _size = rd_u32(data, p + 4);
            let mut q = p + 8;
            let name_start = q;
            while data[q] != 0 {
                q += 1;
            }
            let name = &data[name_start..q];
            q += 1;
            q = (q + 3) & !3; // 4 字节对齐
            match name {
                b"#~" | b"#-" => tilde = Some(moff + off),
                b"#Strings" => strings_off = Some(moff + off),
                _ => {}
            }
            p = q;
        }
        let t = tilde.context("缺少 #~/#- 表流")?;
        let strings_off = strings_off.context("缺少 #Strings 堆")?;

        // #~ 头：Reserved(4) Major(1) Minor(1) HeapSizes(1) Reserved(1) Valid(8) Sorted(8)
        let heap_sizes = data[t + 6];
        let valid = u64::from_le_bytes(data[t + 8..t + 16].try_into().unwrap());
        let mut rowp = t + 24;
        let mut row_count = [0u32; 64];
        let present: Vec<usize> = (0..64).filter(|i| (valid >> i) & 1 == 1).collect();
        for &i in &present {
            row_count[i] = rd_u32(data, rowp);
            rowp += 4;
        }
        let tables_data = rowp;

        let str_w = if heap_sizes & 1 != 0 { 4 } else { 2 };
        let guid_w = if heap_sizes & 2 != 0 { 4 } else { 2 };
        let blob_w = if heap_sizes & 4 != 0 { 4 } else { 2 };

        let mut me = Tables {
            data,
            strings_off,
            table_offset: [None; 7],
            row_count,
            str_w,
            blob_w,
            guid_w,
        };

        // 依表 id 升序累加偏移，直到表 6（MethodDef）为止。
        let mut cur = tables_data;
        for &i in &present {
            if i > 6 {
                break;
            }
            me.table_offset[i] = Some(cur);
            if i < 6 {
                cur += me.row_count[i] as usize * me.row_size(i)?;
            }
        }
        Ok(me)
    }

    fn rows(&self, tbl: usize) -> u32 {
        self.row_count[tbl]
    }
    /// 简单索引宽度：目标表行数 > 0xFFFF 则 4 字节，否则 2 字节。
    fn simple_w(&self, tbl: usize) -> usize {
        if self.rows(tbl) > 0xFFFF { 4 } else { 2 }
    }
    /// 编码索引宽度。
    fn coded_w(&self, tables: &[usize], tag_bits: u32) -> usize {
        let max = tables.iter().map(|&t| self.rows(t)).max().unwrap_or(0);
        if max as u64 > (1u64 << (16 - tag_bits)) {
            4
        } else {
            2
        }
    }

    fn row_size(&self, tbl: usize) -> Result<usize> {
        // 编码索引集合（仅本范围用到的两个）。
        const TYPE_DEF_OR_REF: &[usize] = &[2, 1, 27]; // TypeDef,TypeRef,TypeSpec
        const RESOLUTION_SCOPE: &[usize] = &[0, 26, 35, 1]; // Module,ModuleRef,AssemblyRef,TypeRef
        let s = self.str_w;
        let g = self.guid_w;
        let b = self.blob_w;
        Ok(match tbl {
            0 => 2 + s + 3 * g,                             // Module
            1 => self.coded_w(RESOLUTION_SCOPE, 2) + 2 * s, // TypeRef
            2 => 4 + 2 * s + self.coded_w(TYPE_DEF_OR_REF, 2) + self.simple_w(4) + self.simple_w(6), // TypeDef
            3 => self.simple_w(4),                     // FieldPtr
            4 => 2 + s + b,                            // Field
            5 => self.simple_w(6),                     // MethodPtr
            6 => 4 + 2 + 2 + s + b + self.simple_w(8), // MethodDef
            other => bail!("未实现表 {other} 的行大小计算"),
        })
    }

    fn read_index(&self, o: usize, w: usize) -> u32 {
        if w == 2 {
            rd_u16(self.data, o) as u32
        } else {
            rd_u32(self.data, o)
        }
    }

    fn get_string(&self, idx: u32) -> &str {
        let s = self.strings_off + idx as usize;
        let mut e = s;
        while self.data[e] != 0 {
            e += 1;
        }
        std::str::from_utf8(&self.data[s..e]).unwrap_or("")
    }

    /// 在 TypeDef 表中按简单名查找类型，返回其 0 基行号。
    fn find_type(&self, simple_name: &str) -> Option<u32> {
        let off = self.table_offset[2]?;
        let rs = self.row_size(2).ok()?;
        let n = self.rows(2);
        for r in 0..n {
            let o = off + r as usize * rs + 4; // 跳过 Flags(4)
            let name_idx = self.read_index(o, self.str_w);
            if self.get_string(name_idx) == simple_name {
                return Some(r);
            }
        }
        None
    }

    /// 取 TypeDef 行的 MethodList（1 基起始索引）。
    fn type_method_list(&self, row: u32) -> u32 {
        let off = self.table_offset[2].unwrap();
        let rs = self.row_size(2).unwrap();
        let extends_w = self.coded_w(&[2, 1, 27], 2);
        let field_w = self.simple_w(4);
        let o = off + row as usize * rs + 4 + 2 * self.str_w + extends_w + field_w;
        self.read_index(o, self.simple_w(6))
    }

    fn method_rva_field_offset(&self, row0: u32) -> usize {
        let off = self.table_offset[6].unwrap();
        let rs = self.row_size(6).unwrap();
        off + row0 as usize * rs
    }
    fn method_name(&self, row0: u32) -> &str {
        let base = self.method_rva_field_offset(row0);
        let name_idx = self.read_index(base + 8, self.str_w); // RVA(4)+ImplFlags(2)+Flags(2)
        self.get_string(name_idx)
    }
}

/// 在程序集中定位 `<type_simple_name>` 内、名字以 `<method_name_suffix>` 结尾的方法。
///
/// 返回其方法体 RVA 与 `MethodDef.RVA` 字段的文件偏移。显式接口实现的方法名在
/// 元数据中带接口全名前缀（如 `Ns.IFoo.Bar`），因此用"后缀匹配"。
pub fn find_method(
    pe: &PeImage,
    type_simple_name: &str,
    method_name_suffix: &str,
) -> Result<MethodLocation> {
    let t = Tables::parse(pe)?;
    ensure!(t.table_offset[2].is_some(), "无 TypeDef 表");
    ensure!(t.table_offset[6].is_some(), "无 MethodDef 表");

    let type_row = t
        .find_type(type_simple_name)
        .with_context(|| format!("未找到类型 {type_simple_name}"))?;
    let n_td = t.rows(2);
    let n_md = t.rows(6);
    let start = t.type_method_list(type_row); // 1 基
    let end = if type_row + 1 < n_td {
        t.type_method_list(type_row + 1)
    } else {
        n_md + 1
    };

    let mut found: Option<u32> = None;
    for ridx in start..end {
        let row0 = ridx - 1;
        if t.method_name(row0).ends_with(method_name_suffix) {
            found = Some(row0);
            break;
        }
    }
    let row0 = found
        .with_context(|| format!("类型 {type_simple_name} 中未找到方法 *{method_name_suffix}"))?;

    let rva_field_offset = t.method_rva_field_offset(row0);
    let body_rva = rd_u32(&pe.data, rva_field_offset);
    ensure!(body_rva != 0, "方法无方法体（abstract/extern）");
    Ok(MethodLocation {
        body_rva,
        rva_field_offset,
    })
}
