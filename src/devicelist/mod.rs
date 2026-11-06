//! 机型代号表（数据驱动）。
//!
//! [`DEVICE_LIST_YAML`] 是机型数据的唯一来源：新增机型只需在 `device_list`
//! 下追加一行 `  CODE: '机型名'`；CLI 默认值、GUI / TUI 预置列表会自动同步。
//! 第一项为默认机型。文件经 `include_str!` 编译期内嵌，无需随程序分发。

use anyhow::{Result, bail};
use std::sync::OnceLock;

/// 编译期内嵌的机型表。
const DEVICE_LIST_YAML: &str = include_str!("devicelist.yaml");

/// 数据文件中存放机型条目的顶层键。
const ROOT_KEY: &str = "device_list";

static DEVICES: OnceLock<Vec<Device>> = OnceLock::new();

/// 一条机型记录：`code` 为写入注册表的机型代号，`name` 为界面展示名。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub code: String,
    pub name: String,
}

impl Device {
    /// 列表展示文本：`代号 · 机型名`。
    pub fn label(&self) -> String {
        format!("{} · {}", self.code, self.name)
    }
}

/// 表中全部机型，顺序与 `devicelist.yaml` 一致。
pub fn devices() -> &'static [Device] {
    DEVICES
        .get_or_init(|| {
            parse(DEVICE_LIST_YAML)
                .unwrap_or_else(|error| panic!("devicelist.yaml 解析失败: {error:#}"))
        })
        .as_slice()
}

/// 默认机型代号：表中第一项（安装流程与 CLI `--model` 的默认值）。
pub fn default_code() -> &'static str {
    devices().first().map_or("", |device| device.code.as_str())
}

/// 按代号查机型名；自定义代号不在表中时返回 `None`。
pub fn name_of(code: &str) -> Option<&'static str> {
    devices()
        .iter()
        .find(|device| device.code == code)
        .map(|device| device.name.as_str())
}

/// 解析受限 YAML：只索引 `device_list` 下机型代号这一层的 `CODE: '名称'` 条目，
/// 其他顶层节、注释以及更深层级的字段都不参与索引。
fn parse(src: &str) -> Result<Vec<Device>> {
    let mut in_list = false;
    let mut entry_indent: Option<usize> = None;
    let mut devices: Vec<Device> = Vec::new();

    for (index, raw) in src.lines().enumerate() {
        let line_no = index + 1;
        let line = strip_comment(raw);
        let content = line.trim();
        if content.is_empty() {
            continue;
        }
        let indent = line.len() - line.trim_start().len();

        // 顶层节：进入 / 离开 device_list，其他节一律忽略。
        if indent == 0 {
            in_list = content
                .strip_suffix(':')
                .is_some_and(|key| key.trim() == ROOT_KEY);
            entry_indent = None;
            continue;
        }
        if !in_list {
            continue;
        }

        // 只索引机型代号这一层；更深的层级（若以后扩展字段）忽略。
        let level = *entry_indent.get_or_insert(indent);
        if indent != level {
            continue;
        }

        let Some((raw_code, raw_name)) = content.split_once(':') else {
            bail!("devicelist.yaml:{line_no}: 机型条目应写成 `CODE: '名称'`，实际为 `{content}`");
        };
        let code = unquote(raw_code.trim());
        let name = unquote(raw_name.trim());
        if code.is_empty() || name.is_empty() {
            bail!("devicelist.yaml:{line_no}: 机型代号与名称都不能为空（`{content}`）");
        }
        if devices.iter().any(|device| device.code == code) {
            bail!("devicelist.yaml:{line_no}: 机型代号 `{code}` 重复");
        }
        devices.push(Device { code, name });
    }

    if devices.is_empty() {
        bail!("devicelist.yaml: `{ROOT_KEY}` 下没有任何机型条目");
    }
    Ok(devices)
}

/// 去掉行尾注释（引号内的 `#` 不当作注释），返回需解析的部分。
fn strip_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut index = 0;
    let mut quote: Option<u8> = None;
    let mut after_space = true;
    while index < bytes.len() {
        let byte = bytes[index];
        if let Some(q) = quote {
            if q == b'\'' && byte == b'\'' && bytes.get(index + 1) == Some(&b'\'') {
                index += 2;
                after_space = false;
                continue;
            }
            if q == b'"' && byte == b'\\' {
                index += 2;
                after_space = false;
                continue;
            }
            if byte == q {
                quote = None;
            }
        } else if byte == b'\'' || byte == b'"' {
            quote = Some(byte);
        } else if byte == b'#' && after_space {
            return &line[..index];
        }
        after_space = byte == b' ' || byte == b'\t';
        index += 1;
    }
    line
}

/// 去掉成对引号并还原常见转义；未加引号的值原样返回。
fn unquote(value: &str) -> String {
    let bytes = value.as_bytes();
    if value.len() < 2 {
        return value.to_string();
    }
    let inner = &value[1..value.len() - 1];
    match (bytes[0], bytes[value.len() - 1]) {
        (b'\'', b'\'') => inner.replace("''", "'"),
        (b'"', b'"') => {
            let mut out = String::with_capacity(inner.len());
            let mut chars = inner.chars();
            while let Some(ch) = chars.next() {
                if ch == '\\' {
                    match chars.next() {
                        Some('n') => out.push('\n'),
                        Some('t') => out.push('\t'),
                        Some(other) => out.push(other),
                        None => out.push('\\'),
                    }
                } else {
                    out.push(ch);
                }
            }
            out
        }
        _ => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_list_is_usable() {
        let devices = super::devices();
        assert!(!devices.is_empty());
        assert_eq!(default_code(), "TM2425");
        assert_eq!(name_of("TM2425"), Some("Redmi Book Pro 14 (2026)"));
        assert_eq!(name_of("UNKNOWN"), None);
        assert!(
            devices
                .iter()
                .all(|device| !device.code.is_empty() && !device.name.is_empty())
        );
        assert_eq!(devices[0].label(), "TM2425 · Redmi Book Pro 14 (2026)");
    }

    #[test]
    fn parses_only_the_model_code_level() {
        let src = "\
# 整行注释
other_top_level:
  TM9999: '不该被索引'
device_list:
  TM2425: 'Redmi Book Pro 14 (2026)' # 行尾注释
  TM2424: \"Xiaomi Book Pro 14: # 2026\"  # 名称里的冒号 / # 号应保留
    nested: '更深层不索引'
  TM2309: Redmi Book 16 (2024)
";
        let devices = parse(src).unwrap();
        assert_eq!(devices.len(), 3);
        assert_eq!(devices[0].code, "TM2425");
        assert_eq!(devices[0].name, "Redmi Book Pro 14 (2026)");
        assert_eq!(devices[1].name, "Xiaomi Book Pro 14: # 2026");
        assert_eq!(devices[2].name, "Redmi Book 16 (2024)");
    }

    #[test]
    fn rejects_broken_entries() {
        assert!(parse("device_list:\n  TM2425: 'A'\n  TM2425: 'B'\n").is_err());
        assert!(parse("device_list:\n  TM2425:\n").is_err());
        assert!(parse("device_list: {}\n").is_err());
        assert!(parse("other:\n  TM2425: 'A'\n").is_err());
    }
}
