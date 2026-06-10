//! .NET 方法体（method body）的解析与“前置守卫”重定位构造。
//!
//! 用途：在方法体最前面注入一段等价于 `if (arg1 == value) return;` 的 IL 守卫，
//! 用于从源头抑制特定相机异常 Toast。由于注入会使方法体增长，调用方需把构造出的
//! 新方法体写入新节并改写 `MethodDef.RVA`（见 `camera_toast`）。

use anyhow::{Result, ensure};

#[inline]
fn rd_u16(d: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([d[o], d[o + 1]])
}
#[inline]
fn rd_u32(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

const CORILMETHOD_TINY: u8 = 0x02;
const CORILMETHOD_FAT: u8 = 0x03;
const CORILMETHOD_MORE_SECTS: u16 = 0x08;
const CORILMETHOD_INIT_LOCALS: u16 = 0x10;

/// 解析得到的方法体。
pub struct MethodBody {
    pub max_stack: u16,
    pub local_var_sig_tok: u32,
    pub init_locals: bool,
    pub il: Vec<u8>,
    /// 异常处理段原始字节（已按需要重排 0 个或多个 clause）；空表示无 EH。
    eh_clauses: Vec<EhClause>,
}

#[derive(Clone, Copy)]
struct EhClause {
    flags: u32,
    try_offset: u32,
    try_length: u32,
    handler_offset: u32,
    handler_length: u32,
    class_token_or_filter: u32,
}

impl MethodBody {
    /// 从文件偏移处解析方法体（tiny 或 fat 头）。
    pub fn parse(data: &[u8], off: usize) -> Result<Self> {
        let first = data[off];
        if first & 0x3 == CORILMETHOD_TINY {
            let code_size = (first >> 2) as usize;
            let il = data[off + 1..off + 1 + code_size].to_vec();
            return Ok(Self {
                max_stack: 8,
                local_var_sig_tok: 0,
                init_locals: false,
                il,
                eh_clauses: Vec::new(),
            });
        }
        ensure!(first & 0x3 == CORILMETHOD_FAT, "未知的方法体头类型");
        let flags_size = rd_u16(data, off);
        let header_words = (flags_size >> 12) as usize;
        let header_len = header_words * 4;
        let max_stack = rd_u16(data, off + 2);
        let code_size = rd_u32(data, off + 4) as usize;
        let local_var_sig_tok = rd_u32(data, off + 8);
        let init_locals = flags_size & CORILMETHOD_INIT_LOCALS != 0;
        let il_off = off + header_len;
        let il = data[il_off..il_off + code_size].to_vec();

        let mut eh_clauses = Vec::new();
        if flags_size & CORILMETHOD_MORE_SECTS != 0 {
            let mut sect = (il_off + code_size + 3) & !3; // 4 字节对齐
            loop {
                let kind = data[sect];
                let is_fat = kind & 0x40 != 0;
                let more = kind & 0x80 != 0;
                if is_fat {
                    let data_size = (rd_u32(data, sect) >> 8) as usize; // 3 字节
                    let n = (data_size - 4) / 24;
                    let mut p = sect + 4;
                    for _ in 0..n {
                        eh_clauses.push(EhClause {
                            flags: rd_u32(data, p),
                            try_offset: rd_u32(data, p + 4),
                            try_length: rd_u32(data, p + 8),
                            handler_offset: rd_u32(data, p + 12),
                            handler_length: rd_u32(data, p + 16),
                            class_token_or_filter: rd_u32(data, p + 20),
                        });
                        p += 24;
                    }
                } else {
                    let data_size = data[sect + 1] as usize;
                    let n = (data_size - 4) / 12;
                    let mut p = sect + 4;
                    for _ in 0..n {
                        eh_clauses.push(EhClause {
                            flags: rd_u16(data, p) as u32,
                            try_offset: rd_u16(data, p + 2) as u32,
                            try_length: data[p + 4] as u32,
                            handler_offset: rd_u16(data, p + 5) as u32,
                            handler_length: data[p + 7] as u32,
                            class_token_or_filter: rd_u32(data, p + 8),
                        });
                        p += 12;
                    }
                }
                if !more {
                    break;
                }
                // 下一段紧随其后并 4 字节对齐
                let consumed = if is_fat {
                    (rd_u32(data, sect) >> 8) as usize
                } else {
                    data[sect + 1] as usize
                };
                sect = (sect + consumed + 3) & !3;
            }
        }

        Ok(Self {
            max_stack,
            local_var_sig_tok,
            init_locals,
            il,
            eh_clauses,
        })
    }

    /// 构造在 IL 前注入守卫 `if (arg1 == value) return;` 的新方法体（始终输出 fat 头）。
    ///
    /// 守卫消耗 2 个求值栈槽，会相应抬高 `max_stack`。若原方法含 EH，则其 try/handler/
    /// filter 偏移整体后移 `guard.len()` 字节（catch 的类型 token 不变）。
    pub fn build_with_guard(&self, arg_index: u8, value: i32) -> Vec<u8> {
        let guard = build_guard(arg_index, value);
        let glen = guard.len() as u32;

        let mut new_il = Vec::with_capacity(guard.len() + self.il.len());
        new_il.extend_from_slice(&guard);
        new_il.extend_from_slice(&self.il);

        let code_size = new_il.len() as u32;
        let max_stack = self.max_stack.max(2);
        let has_eh = !self.eh_clauses.is_empty();

        let mut flags: u16 = CORILMETHOD_FAT as u16;
        if self.init_locals {
            flags |= CORILMETHOD_INIT_LOCALS;
        }
        if has_eh {
            flags |= CORILMETHOD_MORE_SECTS;
        }
        let flags_size = (3u16 << 12) | flags; // 头 3 个 dword

        let mut out = Vec::new();
        out.extend_from_slice(&flags_size.to_le_bytes());
        out.extend_from_slice(&max_stack.to_le_bytes());
        out.extend_from_slice(&code_size.to_le_bytes());
        out.extend_from_slice(&self.local_var_sig_tok.to_le_bytes());
        out.extend_from_slice(&new_il);

        if has_eh {
            while out.len() % 4 != 0 {
                out.push(0);
            }
            // 统一输出 fat 格式 EH 段。
            let n = self.eh_clauses.len();
            let data_size = 4 + n * 24;
            let kind = 0x01u32 | 0x40u32; // EHTable | FatFormat
            let header = kind | ((data_size as u32) << 8);
            out.extend_from_slice(&header.to_le_bytes());
            for c in &self.eh_clauses {
                let is_filter = c.flags & 0x1 != 0; // COR_ILEXCEPTION_CLAUSE_FILTER
                out.extend_from_slice(&c.flags.to_le_bytes());
                out.extend_from_slice(&(c.try_offset + glen).to_le_bytes());
                out.extend_from_slice(&c.try_length.to_le_bytes());
                out.extend_from_slice(&(c.handler_offset + glen).to_le_bytes());
                out.extend_from_slice(&c.handler_length.to_le_bytes());
                let last = if is_filter {
                    c.class_token_or_filter + glen
                } else {
                    c.class_token_or_filter
                };
                out.extend_from_slice(&last.to_le_bytes());
            }
        }
        out
    }
}

/// 生成守卫 IL：`ldarg.<n>; ldc.i4 <value>; bne.un.s +1; ret`。
pub fn build_guard(arg_index: u8, value: i32) -> Vec<u8> {
    let mut g = Vec::new();
    // ldarg.0..3 短编码为 0x02..0x05；更大用 ldarg.s。
    match arg_index {
        0..=3 => g.push(0x02 + arg_index),
        n => {
            g.push(0x0E); // ldarg.s
            g.push(n);
        }
    }
    // ldc.i4 <value>
    match value {
        0..=8 => g.push(0x16 + value as u8), // ldc.i4.0..8
        -1 => g.push(0x15),                  // ldc.i4.m1
        -128..=127 => {
            g.push(0x1F); // ldc.i4.s
            g.push(value as i8 as u8);
        }
        _ => {
            g.push(0x20); // ldc.i4
            g.extend_from_slice(&value.to_le_bytes());
        }
    }
    g.push(0x33); // bne.un.s
    g.push(0x01); // 跳过随后的 1 字节 ret，落到原 IL 起点
    g.push(0x2A); // ret
    g
}
