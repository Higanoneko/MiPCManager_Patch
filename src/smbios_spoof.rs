//! SMBIOS 设备身份补丁（call 指令重定向）。
//!
//! `micont_rtm.dll` 通过 `GetSystemFirmwareTable` 读取主板 SMBIOS，
//! 将真实 `project_id` 通过 Lyra 上报给手机，导致妙播无法发现非小米设备。
//!
//! 方案：找到 `call [rip+disp32]` 指令（指向 GetSystemFirmwareTable 的 IAT），
//! 原地改写为 `E8 rel32 + NOP`（6 字节不变），直接跳转到 `.mipatch` 节中的
//! trampoline；trampoline 通过原始 IAT 调用原函数并替换 SMBIOS buffer 字段。
//!
//! 与 LocaleSpoof 均修改 `micont_rtm.dll`，互不冲突。

use crate::dotnet::pe::PeImage;
use crate::{install, smbios, x64_trampoline};
use anyhow::{Context, Result, bail};
use std::path::Path;

pub const TARGET_DLL: &str = "micont_rtm.dll";
pub const DEFAULT_MODEL: &str = "TM2424";

const SECTION_NAME: &str = ".mipatch";
const SECTION_CHARACTERISTICS: u32 = 0x6000_0020;

#[derive(Debug, PartialEq, Eq)]
pub enum PatchOutcome {
    Patched,
    AlreadyPatched,
}

pub fn apply(dll_path: &Path, model: Option<&str>) -> Result<PatchOutcome> {
    let model_code = model.unwrap_or(DEFAULT_MODEL);

    let smbios_raw = read_system_smbios()?;
    let table = smbios::SmbiosTable::new(smbios_raw);
    let fields = table.extract_fields()?;

    let entries: Vec<_> = fields
        .iter()
        .map(|f| build_replace_entry(f, model_code))
        .collect();
    if entries.is_empty() {
        bail!("未找到可替换的 SMBIOS 字段");
    }

    install::ensure_backup(dll_path)?;
    let data = std::fs::read(dll_path)?;
    let mut pe = PeImage::parse(data)?;

    let (_, iat_rva, _) = pe
        .find_iat_entry("kernel32", "GetSystemFirmwareTable")
        .context("未找到 kernel32!GetSystemFirmwareTable")?;

    let (call_off, call_rva) = pe
        .find_call_to_iat(iat_rva)
        .context("未找到指向 GetSystemFirmwareTable IAT 的 call 指令")?;

    // 构建 trampoline；IAT 引用 disp32 的字节偏移在 iat_disp_off
    let tc = x64_trampoline::build_trampoline(iat_rva, &entries);

    // 追加 .mipatch 节
    let trampoline_rva = pe.append_section(SECTION_NAME, &tc.bytes, SECTION_CHARACTERISTICS)?;

    // 原地修正 IAT 引用：disp = iat_rva - (actual_rva + iat_disp_off + 7)
    let sections = pe.sections();
    let sec = sections.last().context("append_section 未产生节")?;
    let raw_start = sec.raw_pointer as usize;
    let next_rip = trampoline_rva.wrapping_add(tc.iat_disp_byte_offset as u32 + 7);
    let correct_disp = (iat_rva as i64).wrapping_sub(next_rip as i64) as i32;
    pe.data[raw_start + tc.iat_disp_byte_offset..raw_start + tc.iat_disp_byte_offset + 4]
        .copy_from_slice(&correct_disp.to_le_bytes());

    // 改写 call 指令：E8 rel32 + NOP
    // E8 disp32 = trampoline_rva - (call_rva + 5)
    let call_disp = (trampoline_rva as i64).wrapping_sub(call_rva.wrapping_add(5) as i64) as i32;
    pe.data[call_off] = 0xE8;
    pe.data[call_off + 1..call_off + 5].copy_from_slice(&call_disp.to_le_bytes());
    pe.data[call_off + 5] = 0x90;

    pe.update_checksum();
    install::write_file_atomic(dll_path, &pe.data)?;

    Ok(PatchOutcome::Patched)
}

pub fn revert(dll_path: &Path) -> Result<()> {
    install::restore_backup(dll_path)
}

pub fn is_patched(dll_path: &Path) -> bool {
    install::backup_path(dll_path).exists()
}

// ── helpers ────────────────────────────────────────────────

fn build_replace_entry(
    field: &smbios::SmbiosField,
    model_code: &str,
) -> x64_trampoline::ReplaceEntry {
    let target = field.role.target_value(model_code);
    let mut verify = field.value.as_bytes().to_vec();
    verify.push(0);
    let mut replace = target.as_bytes().to_vec();
    if replace.len() < verify.len() {
        replace.resize(verify.len(), 0);
    }
    x64_trampoline::ReplaceEntry {
        offset: field.offset as u32,
        verify,
        replace,
    }
}

#[cfg(windows)]
fn read_system_smbios() -> Result<Vec<u8>> {
    use std::ffi::c_void;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetSystemFirmwareTable(
            provider: u32,
            table_id: u32,
            buffer: *mut c_void,
            buffer_size: u32,
        ) -> u32;
    }

    const RSMB: u32 = 0x52534D42;

    // SAFETY: FFI call to kernel32; parameters are trivially safe.
    unsafe {
        let size = GetSystemFirmwareTable(RSMB, 0, std::ptr::null_mut(), 0);
        if size == 0 {
            bail!("GetSystemFirmwareTable('RSMB') 返回 0");
        }
        let mut buf = vec![0u8; size as usize];
        let written = GetSystemFirmwareTable(RSMB, 0, buf.as_mut_ptr().cast(), size);
        if written == 0 || written > size {
            bail!("GetSystemFirmwareTable 读取失败: written={written}, expected={size}");
        }
        buf.truncate(written as usize);
        Ok(buf)
    }
}

#[cfg(not(windows))]
fn read_system_smbios() -> Result<Vec<u8>> {
    bail!("SMBIOS 补丁仅支持 Windows")
}
