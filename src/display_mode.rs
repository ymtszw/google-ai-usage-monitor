use std::sync::atomic::{AtomicU8, Ordering};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::core::{PCWSTR, w};

const SETTINGS_SUBKEY: PCWSTR = w!("Software\\GoogleAIUsageMonitor");
const DISPLAY_MODE_VALUE: PCWSTR = w!("DisplayMode");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayMode {
    Overlay = 0,
    Taskbar = 1,
}

impl DisplayMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Overlay => "overlay",
            Self::Taskbar => "taskbar",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "overlay" | "hud" => Some(Self::Overlay),
            "taskbar" | "bar" => Some(Self::Taskbar),
            _ => None,
        }
    }
}

static CURRENT_MODE: AtomicU8 = AtomicU8::new(0);
static INITIALIZED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn load_from_registry() -> DisplayMode {
    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            SETTINGS_SUBKEY,
            Some(0),
            KEY_READ,
            &mut hkey,
        ) != ERROR_SUCCESS
        {
            return DisplayMode::Overlay;
        }

        let mut buf = [0u16; 64];
        let mut byte_len = (buf.len() * 2) as u32;
        let res = RegQueryValueExW(
            hkey,
            DISPLAY_MODE_VALUE,
            None,
            None,
            Some(buf.as_mut_ptr() as *mut u8),
            Some(&mut byte_len),
        );
        let _ = RegCloseKey(hkey);

        if res == ERROR_SUCCESS && byte_len >= 2 {
            let u16_len = (byte_len as usize / 2).min(buf.len());
            let s = String::from_utf16_lossy(&buf[..u16_len])
                .trim_end_matches('\0')
                .to_string();
            return DisplayMode::parse(&s).unwrap_or(DisplayMode::Overlay);
        }
        DisplayMode::Overlay
    }
}

fn save_to_registry(mode: DisplayMode) {
    unsafe {
        let mut hkey = HKEY::default();
        let res = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            SETTINGS_SUBKEY,
            Some(0),
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut hkey,
            None,
        );
        if res != ERROR_SUCCESS {
            return;
        }

        let wide: Vec<u16> = mode
            .as_str()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let _ = RegSetValueExW(
            hkey,
            DISPLAY_MODE_VALUE,
            Some(0),
            REG_SZ,
            Some(std::slice::from_raw_parts(
                wide.as_ptr() as *const u8,
                wide.len() * 2,
            )),
        );
        let _ = RegCloseKey(hkey);
    }
}

/// 現在の表示モードを取得（初回はレジストリから読み込み）
pub fn get_display_mode() -> DisplayMode {
    if !INITIALIZED.swap(true, Ordering::SeqCst) {
        let loaded = load_from_registry();
        CURRENT_MODE.store(loaded as u8, Ordering::SeqCst);
    }
    match CURRENT_MODE.load(Ordering::SeqCst) {
        1 => DisplayMode::Taskbar,
        _ => DisplayMode::Overlay,
    }
}

/// 表示モードを変更し、レジストリへ永続化
pub fn set_display_mode(mode: DisplayMode) {
    INITIALIZED.store(true, Ordering::SeqCst);
    CURRENT_MODE.store(mode as u8, Ordering::SeqCst);
    save_to_registry(mode);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_mode_parse() {
        assert_eq!(DisplayMode::parse("overlay"), Some(DisplayMode::Overlay));
        assert_eq!(DisplayMode::parse("Taskbar"), Some(DisplayMode::Taskbar));
        assert_eq!(DisplayMode::parse("unknown"), None);
    }
}
