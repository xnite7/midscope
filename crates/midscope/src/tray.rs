//! Tray icon: shows that midscope is running and what it's doing, plus a menu.
//! Also keeps a single instance and a small log for troubleshooting.

use std::io::Write;
use std::sync::atomic::{AtomicU32, Ordering};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HWND, POINT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Shell::{
    ShellExecuteW, Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD,
    NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, LoadIconW, SetForegroundWindow,
    TrackPopupMenu, IDI_APPLICATION, MF_SEPARATOR, MF_STRING, SW_SHOWNORMAL, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, WM_APP,
};

pub const WM_TRAY: u32 = WM_APP + 1;

pub const CMD_TOGGLE: u32 = 1;
pub const CMD_CALIBRATE: u32 = 2;
pub const CMD_FOLDER: u32 = 3;
pub const CMD_EXIT: u32 = 4;

/// Last menu command picked, consumed by the main loop.
pub static COMMAND: AtomicU32 = AtomicU32::new(0);

/// False when another midscope is already running.
pub fn single_instance() -> bool {
    unsafe {
        let _m = CreateMutexW(None, true, w!("Local\\MidscopeOverlaySingleInstance"));
        GetLastError() != ERROR_ALREADY_EXISTS
    }
}

pub fn log(line: &str) {
    let dir = midscope_core::config::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("midscope.log")) {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "[{secs}] {line}");
    }
}

fn copy_wide(dst: &mut [u16], s: &str) {
    let src: Vec<u16> = s.encode_utf16().take(dst.len() - 1).collect();
    dst[..src.len()].copy_from_slice(&src);
    dst[src.len()] = 0;
}

pub struct Tray {
    data: NOTIFYICONDATAW,
}

impl Tray {
    pub fn new(hwnd: HWND, just_updated: bool) -> Tray {
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_INFO,
            uCallbackMessage: WM_TRAY,
            // The midscope icon built into the exe (resource 1), or Windows' default.
            hIcon: unsafe {
                GetModuleHandleW(None)
                    .ok()
                    .and_then(|m| LoadIconW(m, PCWSTR(1 as *const u16)).ok())
                    .or_else(|| LoadIconW(None, IDI_APPLICATION).ok())
                    .unwrap_or_default()
            },
            dwInfoFlags: NIIF_INFO,
            ..Default::default()
        };
        copy_wide(&mut data.szTip, "midscope: starting");
        if just_updated {
            copy_wide(&mut data.szInfoTitle, "midscope updated");
            copy_wide(&mut data.szInfo, &format!("now on v{}. that's it, carry on.", midscope_core::update::current_version()));
        } else {
            copy_wide(&mut data.szInfoTitle, "midscope's on");
            copy_wide(&mut data.szInfo, "it shows up in agent select, on the vs screen, and when you hold tab. settings live in valorant's top bar, next to the store.");
        }
        unsafe {
            let _ = Shell_NotifyIconW(NIM_ADD, &data);
        }
        data.uFlags = NIF_TIP;
        Tray { data }
    }

    pub fn set_status(&mut self, status: &str) {
        copy_wide(&mut self.data.szTip, &format!("midscope: {}", status.to_lowercase()));
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &self.data);
        }
    }

    /// Right-click menu; the picked command lands in COMMAND.
    pub fn menu(hwnd: HWND) {
        unsafe {
            let Ok(m) = CreatePopupMenu() else { return };
            let _ = AppendMenuW(m, MF_STRING, CMD_TOGGLE as usize, w!("show / hide overlay (f6)"));
            let _ = AppendMenuW(m, MF_STRING, CMD_CALIBRATE as usize, w!("calibrate (f7)"));
            let _ = AppendMenuW(m, MF_STRING, CMD_FOLDER as usize, w!("open settings folder"));
            let _ = AppendMenuW(m, MF_SEPARATOR, 0, PCWSTR::null());
            let _ = AppendMenuW(m, MF_STRING, CMD_EXIT as usize, w!("quit midscope"));
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let _ = SetForegroundWindow(hwnd);
            let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, 0, hwnd, None);
            let _ = DestroyMenu(m);
            if cmd.0 > 0 {
                COMMAND.store(cmd.0 as u32, Ordering::Release);
            }
        }
    }

    pub fn open_folder() {
        let dir = midscope_core::config::data_dir();
        let _ = std::fs::create_dir_all(&dir);
        let path: Vec<u16> = dir.as_os_str().to_string_lossy().encode_utf16().chain([0]).collect();
        unsafe {
            ShellExecuteW(None, w!("open"), PCWSTR(path.as_ptr()), None, None, SW_SHOWNORMAL);
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &self.data);
        }
    }
}
