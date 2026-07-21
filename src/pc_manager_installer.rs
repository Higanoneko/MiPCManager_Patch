use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// 安装包所属产品。
///
/// - `XiaomiPcManager`：完整版小米电脑管家，支持全部补丁（含设备伪装）。
/// - `PcContinuity`：小米互联 / 互联互通，仅支持地区伪装，不支持设备伪装。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallerKind {
    XiaomiPcManager,
    PcContinuity,
}

impl InstallerKind {
    /// 启动安装包前是否需要部署设备伪装代理 `msimg32.dll`。
    ///
    /// 两类安装包都会做机型校验（小米互联会显示「暂不支持本设备」），
    /// 因此一律释放代理 DLL 并写入 SpoofDevice 注册表。
    pub fn deploys_device_proxy(self) -> bool {
        true
    }

    /// 面向用户的产品名称。
    pub fn label(self) -> &'static str {
        match self {
            InstallerKind::XiaomiPcManager => "小米电脑管家 (XiaomiPCManager)",
            InstallerKind::PcContinuity => "小米互联 / 互联互通 (PcContinuity)",
        }
    }
}

/// 小米互联安装包文件名中的中文标识（互联互通 / PcContinuity）。
const PC_CONTINUITY_MARKER: &str = "小米互联";

/// 根据文件名判定安装包所属产品；非安装包返回 `None`。
///
/// 识别两类命名：
/// - `*_XiaomiPCManager_*.exe`（完整版小米电脑管家）。
/// - 含「小米互联」且以 `.exe` 结尾（小米互联最新版本_1.1.2.36_d887cad6.exe 等）。
pub fn classify_installer_filename(name: &str) -> Option<InstallerKind> {
    if !name.to_ascii_lowercase().ends_with(".exe") {
        return None;
    }
    // 小米互联为中文名，直接用原文匹配标识（大小写无关的 ascii 转换不影响中文）。
    if name.contains(PC_CONTINUITY_MARKER) {
        return Some(InstallerKind::PcContinuity);
    }
    let lower = name.to_ascii_lowercase();
    let stem = lower.strip_suffix(".exe")?;
    if stem
        .split_once("_xiaomipcmanager_")
        .is_some_and(|(prefix, suffix)| !prefix.is_empty() && !suffix.is_empty())
    {
        return Some(InstallerKind::XiaomiPcManager);
    }
    None
}

/// 判定安装包路径所属产品；无法识别时按小米电脑管家处理（沿用旧行为：部署代理）。
pub fn classify_installer(installer: &Path) -> InstallerKind {
    installer
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(classify_installer_filename)
        .unwrap_or(InstallerKind::XiaomiPcManager)
}

/// 在 Patcher 同目录中查找小米电脑管家 / 小米互联安装包。
pub fn find_local_installers(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut installers = Vec::new();
    let current_executable = std::env::current_exe()
        .ok()
        .and_then(|path| path.canonicalize().ok());
    for entry in fs::read_dir(dir).with_context(|| format!("无法读取目录 {}", dir.display()))?
    {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        if current_executable
            .as_ref()
            .is_some_and(|current| entry.path().canonicalize().ok().as_ref() == Some(current))
        {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if is_installer_filename(&name) {
            installers.push(entry.path());
        }
    }
    installers.sort_by_key(|path| {
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase()
    });
    Ok(installers)
}

fn is_installer_filename(name: &str) -> bool {
    classify_installer_filename(name).is_some()
}

/// 在安装包同目录释放内嵌 `msimg32.dll`。
pub fn prepare_installer(installer: &Path) -> Result<PathBuf> {
    if !installer.is_file() {
        bail!("指定的安装包不存在：{}", installer.display());
    }
    let is_exe = installer
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"));
    if !is_exe {
        bail!("安装包必须是 .exe 文件：{}", installer.display());
    }
    let parent = installer.parent().context("无法确定安装包所在目录")?;
    crate::device_spoof::deploy_proxy(parent)
}

/// 返回 Patcher 可执行文件所在目录。
pub fn patcher_dir() -> Result<PathBuf> {
    let executable = std::env::current_exe().context("无法确定 Patcher 可执行文件路径")?;
    executable
        .parent()
        .map(Path::to_path_buf)
        .context("无法确定 Patcher 所在目录")
}

/// 从 HTTP(S) URL 推导安全的本地安装包文件名。
pub fn download_filename(url: &str) -> Result<String> {
    let lower = url.to_ascii_lowercase();
    if !lower.starts_with("https://") && !lower.starts_with("http://") {
        bail!("下载地址必须使用 http:// 或 https://");
    }
    let without_query = url.split(['?', '#']).next().unwrap_or(url);
    let candidate = without_query.rsplit('/').next().unwrap_or_default();
    let filename = Path::new(candidate)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| {
            name.to_ascii_lowercase().ends_with(".exe")
                && name
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
        })
        .unwrap_or("XiaomiPCManagerInstaller.exe");
    Ok(filename.to_string())
}

/// 使用 Windows PowerShell 的 Invoke-WebRequest 将安装包下载到 Patcher 同目录。
pub fn download_installer(url: &str, target_dir: &Path) -> Result<PathBuf> {
    let target = target_dir.join(download_filename(url)?);
    if target.exists() {
        bail!("下载目标已存在，为避免覆盖已取消：{}", target.display());
    }
    let mut temporary_name = target.as_os_str().to_os_string();
    temporary_name.push(".download.tmp");
    let temporary = PathBuf::from(temporary_name);
    if temporary.exists() {
        fs::remove_file(&temporary)
            .with_context(|| format!("无法清理临时下载文件 {}", temporary.display()))?;
    }

    // URL 和目标路径通过环境变量传递，避免将用户输入拼接进 PowerShell 脚本。
    let script = "$ErrorActionPreference = 'Stop'; \
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; \
        Invoke-WebRequest -UseBasicParsing -Uri $env:MIPCM_DOWNLOAD_URL -OutFile $env:MIPCM_DOWNLOAD_TARGET";
    let powershell = system_powershell_path()?;
    let status = Command::new(&powershell)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ])
        .env("MIPCM_DOWNLOAD_URL", url)
        .env("MIPCM_DOWNLOAD_TARGET", &temporary)
        .status()
        .context("无法启动 Windows PowerShell 下载安装包")?;
    if !status.success() {
        let _ = fs::remove_file(&temporary);
        bail!("Windows PowerShell 下载失败（退出码：{status}）");
    }
    fs::rename(&temporary, &target)
        .with_context(|| format!("无法将下载文件保存为 {}", target.display()))?;
    Ok(target)
}

#[cfg(windows)]
fn system_powershell_path() -> Result<PathBuf> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW;

    let mut buffer = vec![0_u16; 32_768];
    // SAFETY: buffer 指向可写的 u16 数组，长度以 u32 准确传入。
    let length = unsafe { GetWindowsDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) };
    if length == 0 {
        return Err(std::io::Error::last_os_error()).context("无法获取 Windows 系统目录");
    }
    if length as usize >= buffer.len() {
        bail!("Windows 系统目录路径过长");
    }
    let windows_dir = PathBuf::from(OsString::from_wide(&buffer[..length as usize]));
    let powershell = windows_dir
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    if !powershell.is_file() {
        bail!("未找到系统 Windows PowerShell：{}", powershell.display());
    }
    Ok(powershell)
}

#[cfg(not(windows))]
fn system_powershell_path() -> Result<PathBuf> {
    bail!("安装包下载仅支持 Windows PowerShell")
}

/// 启动安装包，返回子进程 PID。
///
/// 小米互联安装器**不**导入 `msimg32.dll`。启动时会做两道门闸：
/// 1. `CSetupHandler::WinVersionMatch` — 要求 CurrentBuildNumber >= 22000（Win11）
/// 2. `BI_MatchProductModelPreload` — WMI 机型白名单
///
/// 流程：释放代理 → 写 SpoofDevice → 挂起启动 → 注入代理 → **旁路上述门闸** → 恢复主线程。
pub fn launch_installer(installer: &Path) -> Result<u32> {
    let installer = installer
        .canonicalize()
        .with_context(|| format!("无法解析安装包路径 {}", installer.display()))?;
    let parent = installer.parent().context("无法确定安装包所在目录")?;
    let proxy = parent.join(crate::device_spoof::PROXY_DLL_NAME);
    let previous_proxy = match fs::read(&proxy) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error).context(format!("无法读取 {}", proxy.display())),
    };
    let backup = crate::install::backup_path(&proxy);
    let backup_existed = backup.exists();
    let temporary = patch_temporary_path(&proxy);
    let temporary_existed = temporary.exists();
    if let Err(error) = prepare_installer(&installer) {
        let deploy_error = error.context(format!("无法为安装包部署 {}", proxy.display()));
        return Err(rollback_after_error(
            deploy_error,
            &proxy,
            previous_proxy.as_deref(),
            &backup,
            backup_existed,
            &temporary,
            temporary_existed,
        ));
    }
    if let Err(error) = crate::device_spoof::ensure_spoof_model(crate::device_spoof::DEFAULT_MODEL) {
        let reg_error = error.context("无法写入 SpoofDevice 伪装机型");
        return Err(rollback_after_error(
            reg_error,
            &proxy,
            previous_proxy.as_deref(),
            &backup,
            backup_existed,
            &temporary,
            temporary_existed,
        ));
    }

    let proxy = proxy
        .canonicalize()
        .with_context(|| format!("无法解析代理路径 {}", proxy.display()))?;

    match launch_suspended_inject_and_patch(&installer, parent, &proxy) {
        Ok(pid) => Ok(pid),
        Err(error) => {
            let launch_error =
                error.context(format!("无法启动并处理安装包 {}", installer.display()));
            Err(rollback_after_error(
                launch_error,
                &proxy,
                previous_proxy.as_deref(),
                &backup,
                backup_existed,
                &temporary,
                temporary_existed,
            ))
        }
    }
}

/// 旁路补丁：函数入口改为 `mov al,1; ret`（MatchProduct / WinVersionMatch）。
const MATCH_BYPASS_PATCH: [u8; 3] = [0xB0, 0x01, 0xC3];

/// 以 CREATE_SUSPENDED 启动 → 注入代理 DLL → 旁路机型/Win11 门闸 → ResumeThread。
#[cfg(windows)]
fn launch_suspended_inject_and_patch(
    installer: &Path,
    work_dir: &Path,
    proxy_dll: &Path,
) -> Result<u32> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Diagnostics::Debug::WriteProcessMemory;
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_EXECUTE_READWRITE, PAGE_READWRITE,
        VirtualAllocEx, VirtualFreeEx, VirtualProtectEx,
    };
    use windows_sys::Win32::System::Threading::{
        CREATE_SUSPENDED, CreateProcessW, CreateRemoteThread, GetExitCodeThread,
        PROCESS_INFORMATION, ResumeThread, STARTUPINFOW, WaitForSingleObject,
    };

    // 先在文件里定位补丁点（避免进程起来后才发现特征不匹配）。
    let pe_bytes =
        fs::read(installer).with_context(|| format!("无法读取安装包 {}", installer.display()))?;
    let patch_rvas = find_match_product_patch_rvas(&pe_bytes)?;

    let app_path: Vec<u16> = installer.as_os_str().encode_wide().chain(Some(0)).collect();
    let cwd: Vec<u16> = work_dir.as_os_str().encode_wide().chain(Some(0)).collect();
    let dll_path: Vec<u16> = proxy_dll.as_os_str().encode_wide().chain(Some(0)).collect();
    let dll_bytes = dll_path.len() * 2;

    let mut si = unsafe { std::mem::zeroed::<STARTUPINFOW>() };
    si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    let mut pi = unsafe { std::mem::zeroed::<PROCESS_INFORMATION>() };

    // SAFETY: 路径缓冲以 NUL 结尾；CREATE_SUSPENDED 使主线程在入口处暂停。
    let ok = unsafe {
        CreateProcessW(
            app_path.as_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
            CREATE_SUSPENDED,
            std::ptr::null_mut(),
            cwd.as_ptr(),
            &si,
            &mut pi,
        )
    };
    if ok == 0 {
        let code = unsafe { GetLastError() } as i32;
        let err = std::io::Error::from_raw_os_error(code);
        if code == 740 {
            return Err(err).context(
                "CreateProcessW 需要管理员权限（安装包要求提升）。请用管理员身份运行本工具后重试",
            );
        }
        return Err(err).context("CreateProcessW(CREATE_SUSPENDED) 失败");
    }

    let pid = pi.dwProcessId;
    let cleanup_handles = || unsafe {
        CloseHandle(pi.hThread);
        CloseHandle(pi.hProcess);
    };

    let inject_result = (|| -> Result<()> {
        let kernel32_name: Vec<u16> = "kernel32.dll\0".encode_utf16().collect();
        // SAFETY: kernel32 在会话内基址对各进程一致，可用本进程地址作远程线程入口。
        let kernel32 = unsafe { GetModuleHandleW(kernel32_name.as_ptr()) };
        if kernel32.is_null() {
            bail!("GetModuleHandleW(kernel32) 失败");
        }
        let load_library = unsafe { GetProcAddress(kernel32, b"LoadLibraryW\0".as_ptr()) };
        let Some(load_library) = load_library else {
            bail!("GetProcAddress(LoadLibraryW) 失败");
        };

        let remote = unsafe {
            VirtualAllocEx(
                pi.hProcess,
                std::ptr::null_mut(),
                dll_bytes,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_READWRITE,
            )
        };
        if remote.is_null() {
            bail!(
                "VirtualAllocEx 失败：{}",
                std::io::Error::from_raw_os_error(unsafe { GetLastError() } as i32)
            );
        }

        let write_ok = unsafe {
            WriteProcessMemory(
                pi.hProcess,
                remote,
                dll_path.as_ptr() as *const _,
                dll_bytes,
                std::ptr::null_mut(),
            )
        };
        if write_ok == 0 {
            unsafe {
                VirtualFreeEx(pi.hProcess, remote, 0, MEM_RELEASE);
            }
            bail!(
                "WriteProcessMemory 失败：{}",
                std::io::Error::from_raw_os_error(unsafe { GetLastError() } as i32)
            );
        }

        let thread = unsafe {
            CreateRemoteThread(
                pi.hProcess,
                std::ptr::null_mut(),
                0,
                Some(std::mem::transmute::<
                    *const (),
                    unsafe extern "system" fn(*mut core::ffi::c_void) -> u32,
                >(load_library as *const ())),
                remote,
                0,
                std::ptr::null_mut(),
            )
        };
        if thread.is_null() {
            unsafe {
                VirtualFreeEx(pi.hProcess, remote, 0, MEM_RELEASE);
            }
            bail!(
                "CreateRemoteThread(LoadLibraryW) 失败：{}",
                std::io::Error::from_raw_os_error(unsafe { GetLastError() } as i32)
            );
        }

        let wait = unsafe { WaitForSingleObject(thread, 15_000) };
        let mut exit_code = 0u32;
        let got_exit = unsafe { GetExitCodeThread(thread, &mut exit_code) };
        unsafe {
            CloseHandle(thread);
            VirtualFreeEx(pi.hProcess, remote, 0, MEM_RELEASE);
        }
        if wait != WAIT_OBJECT_0 {
            bail!("等待 LoadLibraryW 远程线程超时或失败（wait={wait}）");
        }
        if got_exit == 0 || exit_code == 0 {
            bail!(
                "LoadLibraryW 注入失败（远程线程返回 0）。代理 DLL 可能未能加载：{}",
                proxy_dll.display()
            );
        }

        // 旁路 MatchProduct* + WinVersionMatch（Win11 build>=22000）。
        let image_base = remote_main_image_base(pid)?;
        for patch_rva in &patch_rvas {
            let remote_patch = (image_base as u64)
                .checked_add(u64::from(*patch_rva))
                .context("补丁地址溢出")?;
            let mut old_protect = 0u32;
            let protect_ok = unsafe {
                VirtualProtectEx(
                    pi.hProcess,
                    remote_patch as *mut _,
                    MATCH_BYPASS_PATCH.len(),
                    PAGE_EXECUTE_READWRITE,
                    &mut old_protect,
                )
            };
            if protect_ok == 0 {
                bail!(
                    "VirtualProtectEx(RVA {patch_rva:#x}) 失败：{}",
                    std::io::Error::from_raw_os_error(unsafe { GetLastError() } as i32)
                );
            }
            let write_ok = unsafe {
                WriteProcessMemory(
                    pi.hProcess,
                    remote_patch as *mut _,
                    MATCH_BYPASS_PATCH.as_ptr() as *const _,
                    MATCH_BYPASS_PATCH.len(),
                    std::ptr::null_mut(),
                )
            };
            let _ = unsafe {
                VirtualProtectEx(
                    pi.hProcess,
                    remote_patch as *mut _,
                    MATCH_BYPASS_PATCH.len(),
                    old_protect,
                    &mut old_protect,
                )
            };
            if write_ok == 0 {
                bail!(
                    "写入旁路失败 (RVA {patch_rva:#x})：{}",
                    std::io::Error::from_raw_os_error(unsafe { GetLastError() } as i32)
                );
            }
        }

        let resumed = unsafe { ResumeThread(pi.hThread) };
        if resumed == u32::MAX {
            bail!(
                "ResumeThread 失败：{}",
                std::io::Error::from_raw_os_error(unsafe { GetLastError() } as i32)
            );
        }
        Ok(())
    })();

    match inject_result {
        Ok(()) => {
            cleanup_handles();
            Ok(pid)
        }
        Err(error) => {
            unsafe {
                windows_sys::Win32::System::Threading::TerminateProcess(pi.hProcess, 1);
            }
            cleanup_handles();
            Err(error)
        }
    }
}

/// 读取挂起进程主模块基址（ASLR）。
#[cfg(windows)]
fn remote_main_image_base(pid: u32) -> Result<usize> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, MODULEENTRY32W, Module32FirstW, Module32NextW,
        TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32,
    };

    // SAFETY: Toolhelp 快照句柄按文档 CloseHandle 释放。
    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid) };
    if snap == INVALID_HANDLE_VALUE {
        bail!(
            "CreateToolhelp32Snapshot 失败：{}",
            std::io::Error::last_os_error()
        );
    }
    let mut entry = unsafe { std::mem::zeroed::<MODULEENTRY32W>() };
    entry.dwSize = std::mem::size_of::<MODULEENTRY32W>() as u32;
    let ok = unsafe { Module32FirstW(snap, &mut entry) };
    if ok == 0 {
        unsafe {
            CloseHandle(snap);
        }
        bail!(
            "Module32FirstW 失败：{}",
            std::io::Error::last_os_error()
        );
    }
    // 主模块通常是快照中的第一项；再核对路径以免拿错。
    let mut base = entry.modBaseAddr as usize;
    loop {
        let name = String::from_utf16_lossy(
            &entry.szModule[..entry
                .szModule
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szModule.len())],
        );
        if name.to_ascii_lowercase().ends_with(".exe") {
            base = entry.modBaseAddr as usize;
            break;
        }
        if unsafe { Module32NextW(snap, &mut entry) } == 0 {
            break;
        }
    }
    unsafe {
        CloseHandle(snap);
    }
    if base == 0 {
        bail!("无法确定安装进程映像基址");
    }
    Ok(base)
}

#[cfg(not(windows))]
fn launch_suspended_inject_and_patch(
    _installer: &Path,
    _work_dir: &Path,
    _proxy_dll: &Path,
) -> Result<u32> {
    bail!("安装包注入启动仅支持 Windows")
}

/// 在 PE 中定位需旁路的函数入口 RVA：
/// - `BI_MatchProductModelPreload` / `BI_MatchProductModel`（机型白名单）
/// - `CSetupHandler::WinVersionMatch`（要求 CurrentBuildNumber >= 22000，即 Win11）
fn find_match_product_patch_rvas(pe: &[u8]) -> Result<Vec<u32>> {
    let (sections, _image_base) = parse_pe_sections(pe)?;
    let (text_rva, text_off, text_size) = sections
        .iter()
        .find(|(name, ..)| name.starts_with(b".text"))
        .map(|(_, va, _, rawptr, rawsize)| (*va, *rawptr, *rawsize))
        .context("PE 中无 .text 节")?;
    let text = pe
        .get(text_off..text_off + text_size)
        .context(".text 节超出文件范围")?;

    // (签名前缀, 期望序言)
    let targets: &[(&[u8], &[u8])] = &[
        (
            b"bool __cdecl BI_MatchProductModelPreload",
            &[0x40, 0x56, 0x48, 0x83, 0xEC, 0x40],
        ),
        (
            b"bool __cdecl BI_MatchProductModel(",
            &[0x40, 0x56, 0x48, 0x83, 0xEC, 0x40],
        ),
        // Win11 门闸：CurrentBuildNumber >= 22000
        (
            b"bool __cdecl CSetupHandler::WinVersionMatch(void)",
            &[0x4C, 0x8B, 0xDC],
        ),
    ];

    let mut rvas = Vec::new();
    for (sig, prologue) in targets {
        let Some(sig_off) = find_bytes(pe, sig) else {
            continue;
        };
        let sig_rva = off_to_rva(&sections, sig_off)
            .with_context(|| format!("签名不在任何节中：{}", String::from_utf8_lossy(sig)))?;

        let lea = find_lea_to_rva(text, text_rva, sig_rva).with_context(|| {
            format!(
                "未找到对 {} 的代码引用",
                String::from_utf8_lossy(&sig[..sig.len().min(48)])
            )
        })?;

        let func_off_in_text = find_func_start_before(text, lea, prologue).with_context(|| {
            format!(
                "无法回溯到函数入口：{}",
                String::from_utf8_lossy(&sig[..sig.len().min(48)])
            )
        })?;
        let file_off = text_off + func_off_in_text;
        let actual = pe
            .get(file_off..file_off + prologue.len())
            .context("函数入口超出文件范围")?;
        if actual != *prologue {
            bail!(
                "函数序言不符（{} off={file_off:#x}）：{:02X?}",
                String::from_utf8_lossy(&sig[..sig.len().min(40)]),
                actual
            );
        }
        let rva = off_to_rva(&sections, file_off).context("函数入口 RVA 计算失败")?;
        if !rvas.contains(&rva) {
            rvas.push(rva);
        }
    }

    if rvas.is_empty() {
        bail!("安装包中未找到 MatchProduct / WinVersionMatch");
    }
    Ok(rvas)
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn find_lea_to_rva(text: &[u8], text_rva: u32, target_rva: u32) -> Option<usize> {
    let mut i = 0;
    while i + 7 <= text.len() {
        let rex = text[i];
        if matches!(rex, 0x48 | 0x4C) && text[i + 1] == 0x8D && text[i + 2] & 0xC7 == 0x05 {
            let disp = i32::from_le_bytes(text[i + 3..i + 7].try_into().unwrap());
            let instr_rva = i64::from(text_rva) + i as i64;
            let target = instr_rva + 7 + i64::from(disp);
            if target == i64::from(target_rva) {
                return Some(i);
            }
            i += 7;
            continue;
        }
        i += 1;
    }
    None
}

fn find_func_start_before(text: &[u8], lea_off: usize, prologue: &[u8]) -> Option<usize> {
    let start = lea_off.saturating_sub(0x400);
    let plen = prologue.len();
    for i in (start..=lea_off).rev() {
        if i + plen > text.len() {
            continue;
        }
        if &text[i..i + plen] != prologue {
            continue;
        }
        if i > 0 && matches!(text[i - 1], 0xCC | 0xC3) {
            return Some(i);
        }
    }
    // 回退：窗口内直接匹配序言
    for i in (start..=lea_off).rev() {
        if i + plen <= text.len() && &text[i..i + plen] == prologue {
            return Some(i);
        }
    }
    None
}

type PeSection = (Vec<u8>, u32, u32, usize, usize); // name, va, vsize, rawptr, rawsize

fn parse_pe_sections(pe: &[u8]) -> Result<(Vec<PeSection>, u64)> {
    if pe.len() < 0x40 || &pe[0..2] != b"MZ" {
        bail!("不是有效的 PE 文件");
    }
    let e_lfanew = u32::from_le_bytes(pe[0x3C..0x40].try_into().unwrap()) as usize;
    if pe.len() < e_lfanew + 24 || &pe[e_lfanew..e_lfanew + 4] != b"PE\0\0" {
        bail!("PE 头损坏");
    }
    let coff = e_lfanew + 4;
    let nsec = u16::from_le_bytes(pe[coff + 2..coff + 4].try_into().unwrap()) as usize;
    let opt = coff + 20;
    let magic = u16::from_le_bytes(pe[opt..opt + 2].try_into().unwrap());
    if magic != 0x20B {
        bail!("仅支持 PE32+（64 位）安装包");
    }
    let image_base = u64::from_le_bytes(pe[opt + 24..opt + 32].try_into().unwrap());
    let size_opt = u16::from_le_bytes(pe[coff + 16..coff + 18].try_into().unwrap()) as usize;
    let sec_off = opt + size_opt;
    let mut sections = Vec::with_capacity(nsec);
    for i in 0..nsec {
        let o = sec_off + i * 40;
        if pe.len() < o + 40 {
            bail!("节表越界");
        }
        let name = pe[o..o + 8].to_vec();
        let vsize = u32::from_le_bytes(pe[o + 8..o + 12].try_into().unwrap());
        let va = u32::from_le_bytes(pe[o + 12..o + 16].try_into().unwrap());
        let rawsize = u32::from_le_bytes(pe[o + 16..o + 20].try_into().unwrap()) as usize;
        let rawptr = u32::from_le_bytes(pe[o + 20..o + 24].try_into().unwrap()) as usize;
        sections.push((name, va, vsize, rawptr, rawsize));
    }
    Ok((sections, image_base))
}

fn off_to_rva(sections: &[PeSection], off: usize) -> Option<u32> {
    for (_, va, _vsize, rawptr, rawsize) in sections {
        if *rawptr <= off && off < *rawptr + *rawsize {
            return Some(va + (off - rawptr) as u32);
        }
    }
    None
}

fn rollback_after_error(
    error: anyhow::Error,
    proxy: &Path,
    previous: Option<&[u8]>,
    backup: &Path,
    backup_existed: bool,
    temporary: &Path,
    temporary_existed: bool,
) -> anyhow::Error {
    match rollback_proxy(
        proxy,
        previous,
        backup,
        backup_existed,
        temporary,
        temporary_existed,
    ) {
        Ok(()) => error,
        Err(rollback_error) => error.context(format!(
            "回滚 {} 也失败：{rollback_error:#}",
            proxy.display()
        )),
    }
}

fn rollback_proxy(
    proxy: &Path,
    previous: Option<&[u8]>,
    backup: &Path,
    backup_existed: bool,
    temporary: &Path,
    temporary_existed: bool,
) -> Result<()> {
    if let Some(bytes) = previous {
        fs::write(proxy, bytes).with_context(|| format!("无法恢复 {}", proxy.display()))?;
    } else if proxy.exists() {
        fs::remove_file(proxy).with_context(|| format!("无法移除 {}", proxy.display()))?;
    }
    if !backup_existed && backup.exists() {
        fs::remove_file(backup).with_context(|| format!("无法移除 {}", backup.display()))?;
    }
    if !temporary_existed && temporary.is_file() {
        fs::remove_file(temporary).with_context(|| format!("无法移除 {}", temporary.display()))?;
    }
    Ok(())
}

fn patch_temporary_path(file: &Path) -> PathBuf {
    let mut path = file.as_os_str().to_os_string();
    path.push(".patch.tmp");
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "mipcm_installer_{label}_{}_{}",
            std::process::id(),
            nonce
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn discovers_only_xiaomi_pc_manager_installer_executables() {
        let dir = fixture_dir("discovery");
        let expected = [
            "AAA_XiaomiPCManager_stable_5.8.0.75.exe",
            "NeR5_XiaomiPCManager_feature_p52_5.8.0.74_9900fa23.exe",
        ];
        for name in expected {
            fs::write(dir.join(name), b"fixture").unwrap();
        }
        fs::write(dir.join("XiaomiPCManager.exe"), b"fixture").unwrap();
        fs::write(
            dir.join("NeR5_XiaomiPCManager_feature_p52_5.8.0.74.txt"),
            b"fixture",
        )
        .unwrap();
        fs::create_dir(dir.join("Fake_XiaomiPCManager_folder.exe")).unwrap();

        let found = find_local_installers(&dir).unwrap();

        assert_eq!(
            found,
            expected.map(|name| dir.join(name)).to_vec(),
            "安装包应按文件名稳定排序"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn classifies_installer_names_by_product() {
        assert_eq!(
            classify_installer_filename("小米互联最新版本_1.1.2.36_d887cad6.exe"),
            Some(InstallerKind::PcContinuity)
        );
        assert_eq!(
            classify_installer_filename("小米互联.exe"),
            Some(InstallerKind::PcContinuity)
        );
        assert_eq!(
            classify_installer_filename("NeR5_XiaomiPCManager_feature_p52_5.8.0.74_9900fa23.exe"),
            Some(InstallerKind::XiaomiPcManager)
        );
        assert_eq!(
            classify_installer_filename("AAA_XiaomiPCManager_stable_5.8.0.75.exe"),
            Some(InstallerKind::XiaomiPcManager)
        );
        // 非安装包命名不应被识别。
        assert_eq!(classify_installer_filename("XiaomiPCManager.exe"), None);
        assert_eq!(classify_installer_filename("小米互联最新版本_1.1.2.36.txt"), None);
        // 两类安装包都需要部署代理以绕过机型校验。
        assert!(InstallerKind::PcContinuity.deploys_device_proxy());
        assert!(InstallerKind::XiaomiPcManager.deploys_device_proxy());
    }

    #[test]
    fn discovers_pc_continuity_installer_by_chinese_name() {
        let dir = fixture_dir("discovery_continuity");
        let continuity = "小米互联最新版本_1.1.2.36_d887cad6.exe";
        let manager = "AAA_XiaomiPCManager_stable_5.8.0.75.exe";
        fs::write(dir.join(continuity), b"fixture").unwrap();
        fs::write(dir.join(manager), b"fixture").unwrap();
        fs::write(dir.join("readme.txt"), b"fixture").unwrap();

        let found = find_local_installers(&dir).unwrap();

        assert!(found.contains(&dir.join(continuity)), "应识别小米互联安装包");
        assert!(found.contains(&dir.join(manager)), "应识别小米电脑管家安装包");
        assert_eq!(found.len(), 2, "仅应识别两个安装包");
        assert_eq!(
            classify_installer(&dir.join(continuity)),
            InstallerKind::PcContinuity
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn deploys_proxy_for_pc_continuity_installer_before_launch() {
        let dir = fixture_dir("continuity_with_proxy");
        let installer = dir.join("小米互联最新版本_1.1.2.36_d887cad6.exe");
        fs::write(&installer, b"not a Windows executable").unwrap();

        // 非法 exe 会导致启动失败，但启动前应已释放 msimg32.dll；失败后回滚删除。
        assert!(launch_installer(&installer).is_err());

        let proxy = dir.join(crate::device_spoof::PROXY_DLL_NAME);
        // 启动失败会回滚代理；验证 prepare 路径对小米互联同样生效。
        let _ = prepare_installer(&installer).unwrap();
        assert!(
            crate::device_spoof::proxy_is_current(&dir),
            "小米互联安装包也应释放设备伪装代理"
        );
        assert!(proxy.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn prepares_embedded_proxy_next_to_installer() {
        let dir = fixture_dir("proxy");
        let installer = dir.join("NeR5_XiaomiPCManager_feature_5.8.0.74.exe");
        fs::write(&installer, b"fixture").unwrap();

        let proxy = prepare_installer(&installer).unwrap();

        assert_eq!(proxy, dir.join(crate::device_spoof::PROXY_DLL_NAME));
        assert!(crate::device_spoof::proxy_is_current(&dir));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn derives_safe_download_name_from_http_url() {
        assert_eq!(
            download_filename(
                "https://example.com/releases/NeR5_XiaomiPCManager_feature_5.8.0.74.exe?token=abc"
            )
            .unwrap(),
            "NeR5_XiaomiPCManager_feature_5.8.0.74.exe"
        );
        assert_eq!(
            download_filename("https://example.com/download?id=123").unwrap(),
            "XiaomiPCManagerInstaller.exe"
        );
        assert!(download_filename("ftp://example.com/setup.exe").is_err());
    }

    #[test]
    fn downloads_installer_with_windows_powershell() {
        let dir = fixture_dir("download");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\npayload",
                )
                .unwrap();
        });
        let url = format!("http://{address}/Test_XiaomiPCManager_feature_5.8.0.74.exe");

        let downloaded = download_installer(&url, &dir).unwrap();

        server.join().unwrap();
        assert_eq!(fs::read(downloaded).unwrap(), b"payload");
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn resolves_powershell_from_the_windows_system_directory() {
        let powershell = system_powershell_path().unwrap();
        assert!(powershell.is_absolute());
        assert_eq!(
            powershell.file_name().unwrap().to_string_lossy(),
            "powershell.exe"
        );
        assert!(powershell.is_file());
    }

    #[test]
    fn restores_existing_proxy_when_installer_fails_to_launch() {
        let dir = fixture_dir("launch_rollback");
        let installer = dir.join("Broken_XiaomiPCManager_fixture.exe");
        let proxy = dir.join(crate::device_spoof::PROXY_DLL_NAME);
        fs::write(&installer, b"not a Windows executable").unwrap();
        fs::write(&proxy, b"original proxy").unwrap();

        assert!(launch_installer(&installer).is_err());

        assert_eq!(fs::read(&proxy).unwrap(), b"original proxy");
        assert!(!crate::install::backup_path(&proxy).exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn restores_existing_proxy_when_proxy_deployment_fails() {
        let dir = fixture_dir("deploy_rollback");
        let installer = dir.join("Broken_XiaomiPCManager_fixture.exe");
        let proxy = dir.join(crate::device_spoof::PROXY_DLL_NAME);
        let temporary = patch_temporary_path(&proxy);
        fs::write(&installer, b"not a Windows executable").unwrap();
        fs::write(&proxy, b"original proxy").unwrap();
        fs::create_dir(&temporary).unwrap();

        assert!(launch_installer(&installer).is_err());

        assert_eq!(fs::read(&proxy).unwrap(), b"original proxy");
        assert!(!crate::install::backup_path(&proxy).exists());
        assert!(temporary.is_dir(), "预先存在的临时路径不应被删除");
        fs::remove_dir_all(dir).unwrap();
    }

    /// 用本机真实小米互联安装包验证旁路点定位（可选）。
    #[test]
    fn locates_match_product_in_real_pc_continuity_installer() {
        let path = PathBuf::from(
            r"C:\Users\32099\Documents\小米互联PC\小米互联最新版本_1.1.2.36_d887cad6.exe",
        );
        if !path.is_file() {
            eprintln!("skip: installer fixture not present");
            return;
        }
        let pe = fs::read(&path).unwrap();
        let rvas = find_match_product_patch_rvas(&pe).expect("应能定位旁路点");
        // 1.1.2.36：Preload / Match / WinVersionMatch
        assert!(
            rvas.contains(&0x000C_9340),
            "缺少 Preload 入口：{rvas:x?}"
        );
        assert!(
            rvas.contains(&0x000C_9440),
            "缺少 MatchProductModel 入口：{rvas:x?}"
        );
        assert!(
            rvas.contains(&0x000C_CA40),
            "缺少 WinVersionMatch 入口：{rvas:x?}"
        );
        assert_eq!(rvas.len(), 3, "应正好定位 3 个旁路点：{rvas:x?}");
    }
}
