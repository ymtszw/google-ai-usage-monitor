use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsIconic,
    IsWindowVisible, EVENT_OBJECT_LOCATIONCHANGE, EVENT_SYSTEM_MOVESIZEEND,
    EVENT_SYSTEM_MOVESIZESTART, WINEVENT_OUTOFCONTEXT,
};

static TARGET_HWND: AtomicIsize = AtomicIsize::new(0);
static IS_DRAGGING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowBounds {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub width: i32,
    pub height: i32,
}

pub struct WindowTracker {
    hook_movesize_start: HWINEVENTHOOK,
    hook_movesize_end: HWINEVENTHOOK,
    hook_location_change: HWINEVENTHOOK,
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

        if width < 400 || height < 300 {
            return windows::core::BOOL(1);
        }

        let mut title_buf = [0u16; 256];
        let len = GetWindowTextW(hwnd, &mut title_buf);
        let title = String::from_utf16_lossy(&title_buf[..len as usize]);

        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));

        if title.contains("Antigravity") {
            let found_ptr = lparam.0 as *mut HWND;
            *found_ptr = hwnd;
            return windows::core::BOOL(0);
        }

        windows::core::BOOL(1)
    }
}

pub fn find_antigravity_window() -> Option<HWND> {
    let mut found_hwnd = HWND::default();
    unsafe {
        let _ = EnumWindows(
            Some(enum_windows_proc),
            LPARAM(&mut found_hwnd as *mut HWND as isize),
        );
    }

    if found_hwnd.0 != std::ptr::null_mut() {
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

pub fn is_dragging() -> bool {
    IS_DRAGGING.load(Ordering::SeqCst)
}

unsafe extern "system" fn winevent_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _dw_event_thread: u32,
    _dwms_event_time: u32,
) {
    let target = TARGET_HWND.load(Ordering::SeqCst);
    if target == 0 || hwnd.0 as isize != target {
        return;
    }

    match event {
        EVENT_SYSTEM_MOVESIZESTART => {
            IS_DRAGGING.store(true, Ordering::SeqCst);
        }
        EVENT_SYSTEM_MOVESIZEEND => {
            IS_DRAGGING.store(false, Ordering::SeqCst);
        }
        EVENT_OBJECT_LOCATIONCHANGE => {}
        _ => {}
    }
}

impl WindowTracker {
    pub fn install() -> Result<Self, String> {
        unsafe {
            let hook_start = SetWinEventHook(
                EVENT_SYSTEM_MOVESIZESTART,
                EVENT_SYSTEM_MOVESIZESTART,
                None,
                Some(winevent_proc),
                0,
                0,
                WINEVENT_OUTOFCONTEXT,
            );

            let hook_end = SetWinEventHook(
                EVENT_SYSTEM_MOVESIZEEND,
                EVENT_SYSTEM_MOVESIZEEND,
                None,
                Some(winevent_proc),
                0,
                0,
                WINEVENT_OUTOFCONTEXT,
            );

            let hook_loc = SetWinEventHook(
                EVENT_OBJECT_LOCATIONCHANGE,
                EVENT_OBJECT_LOCATIONCHANGE,
                None,
                Some(winevent_proc),
                0,
                0,
                WINEVENT_OUTOFCONTEXT,
            );

            if hook_start.is_invalid() || hook_end.is_invalid() {
                return Err("SetWinEventHook の登録に失敗しました。".to_string());
            }

            Ok(Self {
                hook_movesize_start: hook_start,
                hook_movesize_end: hook_end,
                hook_location_change: hook_loc,
            })
        }
    }
}

impl Drop for WindowTracker {
    fn drop(&mut self) {
        unsafe {
            if !self.hook_movesize_start.is_invalid() {
                let _ = UnhookWinEvent(self.hook_movesize_start);
            }
            if !self.hook_movesize_end.is_invalid() {
                let _ = UnhookWinEvent(self.hook_movesize_end);
            }
            if !self.hook_location_change.is_invalid() {
                let _ = UnhookWinEvent(self.hook_location_change);
            }
        }
    }
}
