//! 字节序列搜索与单字节定位工具。
//!
//! - [`find_bytes`]：朴素的子串查找，替代分散在各模块中的重复实现。
//! - [`locate_single_byte`]：按「前缀 + 可变值字节 + 后缀」定位唯一的候选字节下标，
//!   覆盖原先 `locate_value_byte` 的场景并支持多候选值匹配与唯一性校验。

use anyhow::{bail, Result};

/// 在 `haystack` 中查找 `needle` 首次出现的位置。
///
/// 返回字节下标，未找到返回 `None`。空 needle 或 needle 长于 haystack 返回 `None`。
///
/// # 示例
///
/// ```
/// use mipcmanager_patch::infra::bytes::find_bytes;
///
/// assert_eq!(find_bytes(b"hello world", b"world"), Some(6));
/// assert_eq!(find_bytes(b"abc", b""), None);
/// assert_eq!(find_bytes(b"ab", b"abc"), None);
/// ```
pub fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// 在海量字节中唯一定位一个候选值字节。
///
/// 搜索模式：`prefix ++ [value ∈ candidates] ++ suffix`。
///
/// - 未找到任何匹配 → 返回错误。
/// - 找到多处匹配 → 返回错误（唯一性约束，防止误改）。
/// - 找到恰好一处 → 返回该候选字节在 `haystack` 中的下标。
///
/// # 示例
///
/// ```
/// use mipcmanager_patch::infra::bytes::locate_single_byte;
///
/// let data = vec![0xAA, 0xBB, 0x47, 0xCC, 0xDD];
/// let idx = locate_single_byte(&data, &[0xAA, 0xBB], &[0xCC, 0xDD], &[0x47, 0x06]).unwrap();
/// assert_eq!(idx, 2);
/// ```
pub fn locate_single_byte(
    haystack: &[u8],
    prefix: &[u8],
    suffix: &[u8],
    candidates: &[u8],
) -> Result<usize> {
    let span = prefix.len() + 1 + suffix.len();
    if haystack.len() < span {
        bail!("缓冲区过小（{}），不足以容纳搜索模式（{span}）", haystack.len());
    }

    let candidates_set: std::collections::BTreeSet<u8> = candidates.iter().copied().collect();

    let mut found: Option<usize> = None;
    for i in 0..=haystack.len() - span {
        if &haystack[i..i + prefix.len()] != prefix {
            continue;
        }
        let vi = i + prefix.len();
        let v = haystack[vi];
        if candidates_set.contains(&v) && &haystack[vi + 1..vi + 1 + suffix.len()] == suffix {
            if found.is_some() {
                bail!("特征不唯一（找到多处匹配），已中止以免误改");
            }
            found = Some(vi);
        }
    }
    found
        .ok_or_else(|| anyhow::anyhow!("未找到匹配的特征序列（候选值：{candidates:?}）"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── find_bytes ───────────────────────────────────────────────

    #[test]
    fn find_bytes_basic() {
        assert_eq!(find_bytes(b"abcdef", b"cd"), Some(2));
        assert_eq!(find_bytes(b"abcdef", b"xyz"), None);
    }

    #[test]
    fn find_bytes_empty_needle() {
        assert_eq!(find_bytes(b"abc", b""), None);
    }

    #[test]
    fn find_bytes_needle_longer_than_haystack() {
        assert_eq!(find_bytes(b"ab", b"abc"), None);
    }

    #[test]
    fn find_bytes_needle_at_start() {
        assert_eq!(find_bytes(b"abc", b"ab"), Some(0));
    }

    #[test]
    fn find_bytes_needle_at_end() {
        assert_eq!(find_bytes(b"abc", b"bc"), Some(1));
    }

    #[test]
    fn find_bytes_haystack_empty() {
        assert_eq!(find_bytes(b"", b"a"), None);
    }

    // ── locate_single_byte ──────────────────────────────────────────

    #[test]
    fn locate_single_byte_finds_unique() {
        let data = vec![0x01, 0x02, 0x47, 0x03, 0x04];
        let idx =
            locate_single_byte(&data, &[0x01, 0x02], &[0x03, 0x04], &[0x47, 0x06]).unwrap();
        assert_eq!(idx, 2);
    }

    #[test]
    fn locate_single_byte_zero_match() {
        let data = vec![0x01, 0x02, 0x99, 0x03, 0x04];
        let result = locate_single_byte(&data, &[0x01, 0x02], &[0x03, 0x04], &[0x47, 0x06]);
        assert!(result.is_err());
    }

    #[test]
    fn locate_single_byte_multiple_matches() {
        let data = vec![0x01, 0x02, 0x47, 0x03, 0x04, 0x01, 0x02, 0x06, 0x03, 0x04];
        let result = locate_single_byte(&data, &[0x01, 0x02], &[0x03, 0x04], &[0x47, 0x06]);
        assert!(result.is_err());
    }

    #[test]
    fn locate_single_byte_haystack_too_small() {
        let data = vec![0x01, 0x02];
        let result = locate_single_byte(&data, &[0x01, 0x02], &[0x03, 0x04], &[0x47]);
        assert!(result.is_err());
    }

    #[test]
    fn locate_single_byte_single_candidate() {
        let data = vec![0xAA, 0xFF, 0xBB];
        let idx = locate_single_byte(&data, &[0xAA], &[0xBB], &[0xFF]).unwrap();
        assert_eq!(idx, 1);
    }
}
