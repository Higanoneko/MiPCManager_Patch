//! 最小化的 PE (Portable Executable) 读取与变换工具。
//!
//! 仅实现本项目所需的能力：解析节表、RVA↔文件偏移映射、读取数据目录、
//! 追加新节并把一段载荷（重定位后的方法体）写入其中，同时维护 `SizeOfImage`、
//! 节数量、丢弃 Authenticode 证书并重算 PE 校验和。
//!
//! 仅支持 PE32+（x64），这正是小米电脑管家所有目标程序集的格式。

use anyhow::{Context, Result, ensure};

/// PE32+ 可选头中各字段相对“可选头起始”的偏移。
mod opt {
    pub const MAGIC: usize = 0;
    pub const SECTION_ALIGNMENT: usize = 32;
    pub const FILE_ALIGNMENT: usize = 36;
    pub const SIZE_OF_IMAGE: usize = 56;
    pub const SIZE_OF_HEADERS: usize = 60;
    pub const CHECKSUM: usize = 64;
    pub const DATA_DIRECTORIES: usize = 112;
}

/// 数据目录索引。
pub mod dir {
    pub const SECURITY: usize = 4; // Authenticode 证书（值为文件偏移，非 RVA）
    pub const CLR: usize = 14; // COM 描述符（.NET CLI 头）
}

const PE32PLUS_MAGIC: u16 = 0x20B;
const SECTION_HEADER_SIZE: usize = 40;

/// 一份载入内存、可就地修改的 PE 镜像。
pub struct PeImage {
    pub data: Vec<u8>,
    coff_offset: usize,
    opt_offset: usize,
    section_table_offset: usize,
    num_sections: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct Section {
    pub virtual_size: u32,
    pub virtual_address: u32,
    pub raw_size: u32,
    pub raw_pointer: u32,
}

#[inline]
fn rd_u16(d: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([d[o], d[o + 1]])
}
#[inline]
fn rd_u32(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

impl PeImage {
    pub fn parse(data: Vec<u8>) -> Result<Self> {
        ensure!(data.len() > 0x40, "文件过小，不是有效 PE");
        ensure!(&data[0..2] == b"MZ", "缺少 MZ 头");
        let pe_offset = rd_u32(&data, 0x3C) as usize;
        ensure!(
            pe_offset + 24 < data.len() && &data[pe_offset..pe_offset + 4] == b"PE\0\0",
            "缺少 PE 签名"
        );
        let coff_offset = pe_offset + 4;
        let opt_offset = coff_offset + 20;
        let magic = rd_u16(&data, opt_offset + opt::MAGIC);
        ensure!(
            magic == PE32PLUS_MAGIC,
            "仅支持 PE32+ (x64)，magic=0x{magic:X}"
        );
        let num_sections = rd_u16(&data, coff_offset + 2) as usize;
        let size_of_optional = rd_u16(&data, coff_offset + 16) as usize;
        let section_table_offset = opt_offset + size_of_optional;
        Ok(Self {
            data,
            coff_offset,
            opt_offset,
            section_table_offset,
            num_sections,
        })
    }

    fn opt_u32(&self, field: usize) -> u32 {
        rd_u32(&self.data, self.opt_offset + field)
    }
    fn set_opt_u32(&mut self, field: usize, v: u32) {
        self.data[self.opt_offset + field..self.opt_offset + field + 4]
            .copy_from_slice(&v.to_le_bytes());
    }

    pub fn file_alignment(&self) -> u32 {
        self.opt_u32(opt::FILE_ALIGNMENT)
    }
    pub fn section_alignment(&self) -> u32 {
        self.opt_u32(opt::SECTION_ALIGNMENT)
    }
    pub fn size_of_headers(&self) -> u32 {
        self.opt_u32(opt::SIZE_OF_HEADERS)
    }

    pub fn section(&self, i: usize) -> Section {
        let o = self.section_table_offset + i * SECTION_HEADER_SIZE;
        let d = &self.data;
        Section {
            virtual_size: rd_u32(d, o + 8),
            virtual_address: rd_u32(d, o + 12),
            raw_size: rd_u32(d, o + 16),
            raw_pointer: rd_u32(d, o + 20),
        }
    }

    pub fn sections(&self) -> Vec<Section> {
        (0..self.num_sections).map(|i| self.section(i)).collect()
    }

    /// RVA → 文件偏移。
    pub fn rva_to_offset(&self, rva: u32) -> Option<usize> {
        for s in self.sections() {
            let span = s.virtual_size.max(s.raw_size);
            if rva >= s.virtual_address && rva < s.virtual_address + span {
                return Some((s.raw_pointer + (rva - s.virtual_address)) as usize);
            }
        }
        None
    }

    /// 读取数据目录项 (rva/offset, size)。
    pub fn data_directory(&self, index: usize) -> (u32, u32) {
        let o = self.opt_offset + opt::DATA_DIRECTORIES + index * 8;
        (rd_u32(&self.data, o), rd_u32(&self.data, o + 4))
    }
    fn set_data_directory(&mut self, index: usize, rva: u32, size: u32) {
        let o = self.opt_offset + opt::DATA_DIRECTORIES + index * 8;
        self.data[o..o + 4].copy_from_slice(&rva.to_le_bytes());
        self.data[o + 4..o + 8].copy_from_slice(&size.to_le_bytes());
    }

    /// 在指定文件偏移写入一个小端 u32。
    pub fn write_u32_at(&mut self, offset: usize, v: u32) {
        self.data[offset..offset + 4].copy_from_slice(&v.to_le_bytes());
    }

    /// 追加一个新节并写入载荷，返回新节的 RVA（即载荷起始 RVA）。
    ///
    /// 同时：丢弃文件尾部的 Authenticode 证书（若有）并清零安全目录，
    /// 因为任何修改都会使签名失效；维护节数量、`SizeOfImage`；
    /// 最后重算 PE 校验和。
    pub fn append_section(
        &mut self,
        name: &str,
        payload: &[u8],
        characteristics: u32,
    ) -> Result<u32> {
        let file_align = self.file_alignment();
        let sect_align = self.section_alignment();
        ensure!(file_align != 0 && sect_align != 0, "对齐值非法");

        // 1) 确认节表后仍有 40 字节容纳新节头（不能超过 SizeOfHeaders）。
        let new_header_end =
            self.section_table_offset + (self.num_sections + 1) * SECTION_HEADER_SIZE;
        ensure!(
            new_header_end <= self.size_of_headers() as usize,
            "节头区空间不足，无法新增节（需要扩展 SizeOfHeaders，当前未实现）"
        );

        // 2) 丢弃尾部证书：截断到证书前并清零安全目录。
        let (sec_off, sec_size) = self.data_directory(dir::SECURITY);
        if sec_off != 0 && sec_size != 0 {
            let end = (sec_off + sec_size) as usize;
            if end == self.data.len() {
                self.data.truncate(sec_off as usize);
            }
            self.set_data_directory(dir::SECURITY, 0, 0);
        }

        // 3) 计算新节的 RVA 与文件位置。
        let mut max_rva_end = 0u32;
        for s in self.sections() {
            max_rva_end = max_rva_end.max(s.virtual_address + s.virtual_size.max(s.raw_size));
        }
        let new_va = align_up(max_rva_end, sect_align);
        let raw_ptr = align_up(self.data.len() as u32, file_align);
        let raw_size = align_up(payload.len() as u32, file_align);
        let virtual_size = payload.len() as u32;

        // 4) 追加原始数据（含对齐填充）。
        self.data.resize(raw_ptr as usize, 0);
        self.data.extend_from_slice(payload);
        self.data.resize((raw_ptr + raw_size) as usize, 0);

        // 5) 写入新节头。
        let ho = self.section_table_offset + self.num_sections * SECTION_HEADER_SIZE;
        let mut nm = [0u8; 8];
        let nb = name.as_bytes();
        nm[..nb.len().min(8)].copy_from_slice(&nb[..nb.len().min(8)]);
        self.data[ho..ho + 8].copy_from_slice(&nm);
        self.data[ho + 8..ho + 12].copy_from_slice(&virtual_size.to_le_bytes());
        self.data[ho + 12..ho + 16].copy_from_slice(&new_va.to_le_bytes());
        self.data[ho + 16..ho + 20].copy_from_slice(&raw_size.to_le_bytes());
        self.data[ho + 20..ho + 24].copy_from_slice(&raw_ptr.to_le_bytes());
        // PointerToRelocations/LineNumbers/Numbers = 0
        for b in &mut self.data[ho + 24..ho + 36] {
            *b = 0;
        }
        self.data[ho + 36..ho + 40].copy_from_slice(&characteristics.to_le_bytes());

        // 6) 更新节数量与 SizeOfImage。
        self.num_sections += 1;
        let new_count = self.num_sections as u16;
        self.data[self.coff_offset + 2..self.coff_offset + 4]
            .copy_from_slice(&new_count.to_le_bytes());
        let new_size_of_image = align_up(new_va + virtual_size, sect_align);
        self.set_opt_u32(opt::SIZE_OF_IMAGE, new_size_of_image);

        // 7) 重算校验和。
        self.update_checksum();

        Ok(new_va)
    }

    /// 按标准算法重算 PE 校验和并写回。
    pub fn update_checksum(&mut self) {
        let checksum_field = self.opt_offset + opt::CHECKSUM;
        // 计算时校验和字段视为 0。
        self.data[checksum_field..checksum_field + 4].copy_from_slice(&[0, 0, 0, 0]);
        let mut sum: u64 = 0;
        let mut i = 0;
        let len = self.data.len();
        while i + 1 < len {
            sum += rd_u16(&self.data, i) as u64;
            if sum > 0xFFFF_FFFF {
                sum = (sum & 0xFFFF_FFFF) + (sum >> 32);
            }
            i += 2;
        }
        if i < len {
            sum += self.data[i] as u64;
        }
        sum = (sum & 0xFFFF) + (sum >> 16);
        sum += sum >> 16;
        sum &= 0xFFFF;
        let checksum = sum as u32 + len as u32;
        self.write_u32_at(checksum_field, checksum);
    }

    /// 定位 .NET 元数据根 (`BSJB`) 的文件偏移。
    pub fn metadata_root_offset(&self) -> Result<usize> {
        let (clr_rva, clr_size) = self.data_directory(dir::CLR);
        ensure!(
            clr_rva != 0 && clr_size >= 0x48,
            "不是 .NET 程序集（无 CLR 头）"
        );
        let cli = self
            .rva_to_offset(clr_rva)
            .context("CLR 头 RVA 无法映射到文件偏移")?;
        // CLI 头：MetaData 目录在偏移 +8 (rva) / +12 (size)
        let meta_rva = rd_u32(&self.data, cli + 8);
        let moff = self
            .rva_to_offset(meta_rva)
            .context("元数据 RVA 无法映射到文件偏移")?;
        ensure!(
            &self.data[moff..moff + 4] == b"BSJB",
            "元数据签名 BSJB 缺失"
        );
        Ok(moff)
    }
}

#[inline]
pub fn align_up(v: u32, align: u32) -> u32 {
    (v + align - 1) & !(align - 1)
}
