//! 管理员提权兜底：release exe 通过 manifest 启动即触发 UAC。
//!
//! 修改 `C:\Program Files\...` 下的安装文件需要管理员权限。若构建产物未嵌入
//! manifest，本模块仍会在程序入口检测当前进程权限，并用 `runas` 重新启动同一命令。
//!
//! 设置环境变量 `MIPCM_NO_ELEVATE=1` 可跳过运行时兜底（用于测试）。

/// 入口处调用：必要时提权重启。已提权或设置了跳过变量则直接返回。
pub fn ensure_elevated() {
    #[cfg(windows)]
    {
        if std::env::var_os("MIPCM_NO_ELEVATE").is_some() {
            return;
        }
        if !is_elevated() {
            relaunch_elevated_and_exit();
        }
    }
}

#[cfg(windows)]
fn is_elevated() -> bool {
    use std::ptr;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{
        GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token: HANDLE = ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut ret_len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut _ as *mut core::ffi::c_void,
            core::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret_len,
        );
        CloseHandle(token);
        ok != 0 && elevation.TokenIsElevated != 0
    }
}

/// 以管理员身份重启当前命令（转发参数），随后退出当前进程。
#[cfg(windows)]
fn relaunch_elevated_and_exit() -> ! {
    use std::ptr;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_NORMAL;

    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => std::process::exit(1),
    };
    let params = std::env::args()
        .skip(1)
        .map(|a| quote_arg(&a))
        .collect::<Vec<_>>()
        .join(" ");

    let verb = to_wide("runas");
    let file = to_wide(&exe.to_string_lossy());
    let args = to_wide(&params);

    let r = unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            if params.is_empty() {
                ptr::null()
            } else {
                args.as_ptr()
            },
            ptr::null(),
            SW_NORMAL,
        )
    };
    // ShellExecuteW 返回值 > 32 表示成功。
    if (r as isize) <= 32 {
        eprintln!("\x1b[31m需要管理员权限，但提权被取消或失败。\x1b[0m");
        std::process::exit(1);
    }
    std::process::exit(0);
}

/// 将字符串转为以 NUL 结尾的 UTF-16。
#[cfg(windows)]
fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 为命令行参数做最简引号转义（含空格/引号时加双引号）。
#[cfg(windows)]
fn quote_arg(a: &str) -> String {
    if a.is_empty() || a.contains([' ', '\t', '"']) {
        let escaped = a.replace('"', "\\\"");
        format!("\"{escaped}\"")
    } else {
        a.to_string()
    }
}
