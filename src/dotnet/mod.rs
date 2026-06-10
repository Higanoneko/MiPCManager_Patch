//! 纯 Rust 的 .NET 程序集补丁能力：定位方法 → 在方法体前注入守卫 → 重定位到新节。
//!
//! 之所以采用“重定位到新节 + 改写 MethodDef.RVA”，是因为注入会令方法体增长、
//! 无法原地扩展；而该方案不触碰元数据堆（不新增字符串/成员引用），因此能绕开
//! 现有纯 Rust 元数据写库无法回写大型 WinRT 程序集的限制。

pub mod metadata;
pub mod method_body;
pub mod pe;

use anyhow::Result;
use method_body::MethodBody;
use pe::PeImage;

/// 注入结果。
#[derive(Debug, PartialEq, Eq)]
pub enum InjectOutcome {
    /// 本次成功打入补丁。
    Patched,
    /// 已存在相同守卫，未作改动（幂等）。
    AlreadyPatched,
}

/// IL 方法体所在节的特征值：含代码 | 可执行 | 可读（与 `.text` 一致，最稳妥）。
const IL_SECTION_CHARACTERISTICS: u32 = 0x6000_0020;

/// 在 `type_simple_name` 类型中名字以 `method_name_suffix` 结尾的方法体最前面，
/// 注入 `if (arg[arg_index] == value) return;` 守卫。
///
/// 成功后 `pe` 的 `data` 即为补丁后的完整文件字节（调用方负责落盘）。
pub fn inject_guard_return(
    pe: &mut PeImage,
    type_simple_name: &str,
    method_name_suffix: &str,
    arg_index: u8,
    value: i32,
    section_name: &str,
) -> Result<InjectOutcome> {
    let loc = metadata::find_method(pe, type_simple_name, method_name_suffix)?;
    let body_off = pe
        .rva_to_offset(loc.body_rva)
        .ok_or_else(|| anyhow::anyhow!("方法体 RVA 0x{:X} 无法映射", loc.body_rva))?;

    let body = MethodBody::parse(&pe.data, body_off)?;

    // 幂等：若 IL 已以相同守卫开头，则跳过。
    let guard = method_body::build_guard(arg_index, value);
    if body.il.len() >= guard.len() && body.il[..guard.len()] == guard[..] {
        return Ok(InjectOutcome::AlreadyPatched);
    }

    let new_body = body.build_with_guard(arg_index, value);

    // 先改写 RVA 字段，再追加新节（追加流程会在最后重算校验和，覆盖全部改动）。
    let new_rva = {
        // append_section 会改变 data，但 rva_field_offset 落在 .text 内、位置不变，
        // 因此先暂存目标值，待新节落定拿到 new_rva 后再写。
        let new_rva = pe.append_section(section_name, &new_body, IL_SECTION_CHARACTERISTICS)?;
        pe.write_u32_at(loc.rva_field_offset, new_rva);
        new_rva
    };
    let _ = new_rva;

    // RVA 字段在追加新节后才写入，需再次重算校验和。
    pe.update_checksum();

    Ok(InjectOutcome::Patched)
}
