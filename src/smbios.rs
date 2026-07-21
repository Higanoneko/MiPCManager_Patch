//! SMBIOS 原始表解析。
//!
//! 从 `GetSystemFirmwareTable('RSMB', 0, ...)` 返回的原始字节中
//! 提取 Type 1 (System Information) 和 Type 2 (Baseboard) 的关键字符串字段，
//! 生成 trampoline 中使用的等长替换对。

use anyhow::{Result, bail};

pub struct SmbiosTable {
    raw: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct SmbiosField {
    pub role: SmbiosFieldRole,
    pub offset: usize,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmbiosFieldRole {
    Type1Manufacturer,
    Type1ProductName,
    Type2Manufacturer,
    Type2Product,
}

impl SmbiosTable {
    pub fn new(data: Vec<u8>) -> Self {
        Self { raw: data }
    }

    pub fn extract_fields(&self) -> Result<Vec<SmbiosField>> {
        let mut fields = Vec::new();
        let mut pos = 0;
        loop {
            if pos + 4 > self.raw.len() {
                break;
            }
            let ty = self.raw[pos];
            let hdr_len = self.raw[pos + 1] as usize;
            if hdr_len < 4 || pos + hdr_len > self.raw.len() {
                break;
            }

            let strings = pos + hdr_len;
            let strings_end = self.raw[strings..]
                .windows(2)
                .position(|w| w == [0, 0])
                .map(|p| strings + p + 2)
                .unwrap_or(self.raw.len());

            match ty {
                1 => self.collect_type1(pos, strings, strings_end, &mut fields),
                2 => self.collect_type2(pos, strings, strings_end, &mut fields),
                127 => break,
                _ => {}
            }

            pos = strings_end;
        }

        if fields.is_empty() {
            bail!("未在 SMBIOS 表中找到 Type 1 或 Type 2 结构");
        }
        Ok(fields)
    }

    fn collect_type1(
        &self,
        header: usize,
        strings: usize,
        strings_end: usize,
        fields: &mut Vec<SmbiosField>,
    ) {
        // Type 1 formatted area: mfr@+4, product@+5
        let mfr_idx = self.raw.get(header + 4).copied().unwrap_or(0) as usize;
        self.push_field(
            mfr_idx,
            strings,
            strings_end,
            fields,
            SmbiosFieldRole::Type1Manufacturer,
        );

        let prod_idx = self.raw.get(header + 5).copied().unwrap_or(0) as usize;
        self.push_field(
            prod_idx,
            strings,
            strings_end,
            fields,
            SmbiosFieldRole::Type1ProductName,
        );
    }

    fn collect_type2(
        &self,
        header: usize,
        strings: usize,
        strings_end: usize,
        fields: &mut Vec<SmbiosField>,
    ) {
        // Type 2 formatted area: mfr@+4, product@+5
        let mfr_idx = self.raw.get(header + 4).copied().unwrap_or(0) as usize;
        self.push_field(
            mfr_idx,
            strings,
            strings_end,
            fields,
            SmbiosFieldRole::Type2Manufacturer,
        );

        let prod_idx = self.raw.get(header + 5).copied().unwrap_or(0) as usize;
        self.push_field(
            prod_idx,
            strings,
            strings_end,
            fields,
            SmbiosFieldRole::Type2Product,
        );
    }

    fn push_field(
        &self,
        index: usize,
        strings: usize,
        strings_end: usize,
        fields: &mut Vec<SmbiosField>,
        role: SmbiosFieldRole,
    ) {
        if index == 0 {
            return;
        }
        let (value, offset) = read_smbios_string(&self.raw, strings, strings_end, index);
        if !value.is_empty() {
            fields.push(SmbiosField {
                role,
                offset,
                value,
            });
        }
    }
}

impl SmbiosFieldRole {
    pub fn target_value<'a>(&self, model_code: &'a str) -> &'a str {
        match self {
            Self::Type1Manufacturer | Self::Type2Manufacturer => "XIAOMI",
            Self::Type2Product => model_code,
            Self::Type1ProductName => match model_code {
                "TM2424" => "Xiaomi Book Pro 14 2026",
                "TM2309" => "Redmi Book 16 2024",
                other => other, // 原样返回，由调用方处理长度
            },
        }
    }
}

/// 从 SMBIOS 字符串表读取第 `index` 条（1-based）字符串。
///
/// 返回 `(value, offset_in_raw_buffer)`。
fn read_smbios_string(data: &[u8], start: usize, end: usize, index: usize) -> (String, usize) {
    let mut nth = 0usize;
    let mut seg_start = start;
    for (i, &b) in data[start..end].iter().enumerate() {
        let abs = start + i;
        if b == 0 {
            nth += 1;
            if nth == index {
                return (
                    String::from_utf8_lossy(&data[seg_start..abs]).into_owned(),
                    seg_start,
                );
            }
            seg_start = abs + 1;
            if nth > index {
                break;
            }
        }
    }
    (String::new(), 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_fixture() -> Vec<u8> {
        let mut data = Vec::new();

        // Type 0: [0, 4, handle(2)]  + strings "A\0" "B\0" "\0"
        data.extend_from_slice(&[0, 4, 0, 0, b'A', 0, b'B', 0, 0, 0]);

        // Type 1: 27-byte header, mfr@+4=1, product@+5=2
        let t1: [u8; 27] = [
            1, 0x1B, 0, 0, 0, 0, 0, 0, 0, 0, 1, // mfr = string #1
            2, // product = string #2
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        data.extend_from_slice(&t1);
        data.extend_from_slice(b"System manufacturer\0");
        data.extend_from_slice(b"System Product Name\0");
        data.push(0); // double-null

        // Type 2: 8-byte header, mfr@+4=1, product@+5=2
        data.extend_from_slice(&[2, 8, 1, 0, 1, 2, 0, 0]);
        data.extend_from_slice(b"ASUSTeK COMPUTER INC.\0");
        data.extend_from_slice(b"TUF B360M-PLUS GAMING S\0");
        data.push(0);

        // EOT
        data.extend_from_slice(&[127, 4, 0, 0]);
        data
    }

    #[test]
    fn extracts_all_fields() {
        let table = SmbiosTable::new(make_fixture());
        let fields = table.extract_fields().unwrap();
        assert_eq!(fields.len(), 4);

        assert_eq!(fields[0].role, SmbiosFieldRole::Type1Manufacturer);
        assert_eq!(fields[0].value, "System manufacturer");

        assert_eq!(fields[1].role, SmbiosFieldRole::Type1ProductName);
        assert_eq!(fields[1].value, "System Product Name");

        assert_eq!(fields[2].role, SmbiosFieldRole::Type2Manufacturer);
        assert_eq!(fields[2].value, "ASUSTeK COMPUTER INC.");

        assert_eq!(fields[3].role, SmbiosFieldRole::Type2Product);
        assert_eq!(fields[3].value, "TUF B360M-PLUS GAMING S");
    }

    #[test]
    fn target_values() {
        let table = SmbiosTable::new(make_fixture());
        let fields = table.extract_fields().unwrap();
        let t2 = fields
            .iter()
            .find(|f| f.role == SmbiosFieldRole::Type2Product)
            .unwrap();
        assert_eq!(t2.role.target_value("TM2424"), "TM2424");
        let t1m = fields
            .iter()
            .find(|f| f.role == SmbiosFieldRole::Type1Manufacturer)
            .unwrap();
        assert_eq!(t1m.role.target_value("TM2424"), "XIAOMI");
    }
}
