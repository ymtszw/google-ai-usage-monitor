use std::sync::atomic::{AtomicIsize, Ordering};
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
};

static TARGET_HWND: AtomicIsize = AtomicIsize::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowBounds {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub width: i32,
    pub height: i32,
}

/// Antigravity.exe のプロセスID一覧を取得
fn get_antigravity_pids() -> Vec<u32> {
    let mut pids = Vec::new();
    unsafe {
        if let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            let mut entry = std::mem::zeroed::<PROCESSENTRY32W>();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

            if Process32FirstW(snapshot, &mut entry).is_ok() {
                loop {
                    let name = String::from_utf16_lossy(&entry.szExeFile)
                        .trim_matches('\0')
                        .to_string();

                    if name.eq_ignore_ascii_case("Antigravity.exe") {
                        pids.push(entry.th32ProcessID);
                    }

                    if Process32NextW(snapshot, &mut entry).is_err() {
                        break;
                    }
                }
            }
            let _ = windows::Win32::Foundation::CloseHandle(snapshot);
        }
    }
    pids
}

unsafe extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return windows::core::BOOL(1);
        }

        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return windows::core::BOOL(1);
        }

        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;

        // メインウィンドウとして妥当なサイズ
        if width < 500 || height < 350 {
            return windows::core::BOOL(1);
        }

        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));

        let pids = &*(lparam.0 as *const (Vec<u32>, *mut HWND));
        if pids.0.contains(&pid) {
            *(pids.1) = hwnd;
            return windows::core::BOOL(0); // 見つかったので探索終了
        }

        windows::core::BOOL(1)
    }
}

pub fn find_antigravity_window() -> Option<HWND> {
    let pids = get_antigravity_pids();
    if pids.is_empty() {
        return None;
    }

    let mut found_hwnd = HWND::default();
    let param = (pids, &mut found_hwnd as *mut HWND);

    unsafe {
        let _ = EnumWindows(Some(enum_windows_proc), LPARAM(&param as *const _ as isize));
    }

    if !found_hwnd.0.is_null() {
        TARGET_HWND.store(found_hwnd.0 as isize, Ordering::SeqCst);
        Some(found_hwnd)
    } else {
        None
    }
}

pub fn get_target_bounds(hwnd: HWND) -> Option<WindowBounds> {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return None;
        }

        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_ok() {
            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;
            if width > 0 && height > 0 {
                return Some(WindowBounds {
                    left: rect.left,
                    top: rect.top,
                    right: rect.right,
                    bottom: rect.bottom,
                    width,
                    height,
                });
            }
        }
    }
    None
}
