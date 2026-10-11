use std::env;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ, RegCloseKey, RegDeleteValueW,
    RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::core::{PCWSTR, w};

const RUN_SUBKEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const APP_REG_NAME: PCWSTR = w!("GoogleAIUsageMonitor");

fn str_to_u16_vec(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

/// 現在スタートアップに登録されているかを確認
pub fn is_startup_enabled() -> bool {
    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, RUN_SUBKEY, Some(0), KEY_READ, &mut hkey)
            != ERROR_SUCCESS
        {
            return false;
        }

        let mut data_len = 0u32;
        let res = RegQueryValueExW(hkey, APP_REG_NAME, None, None, None, Some(&mut data_len));

        let _ = RegCloseKey(hkey);
        res == ERROR_SUCCESS
    }
}

/// スタートアップ登録の有効/無効を切り替え
pub fn set_startup_enabled(enabled: bool) -> Result<(), String> {
    unsafe {
        let mut hkey = HKEY::default();
        let open_res = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            RUN_SUBKEY,
            Some(0),
            KEY_READ | KEY_WRITE,
            &mut hkey,
        );
        if open_res != ERROR_SUCCESS {
            return Err(format!(
                "レジストリキーのオープンに失敗 (エラーコード: {:?})",
                open_res
            ));
        }

        let result = if enabled {
            let current_exe =
                env::current_exe().map_err(|e| format!("実行ファイルのパス取得に失敗: {}", e))?;
            let exe_path_wide = str_to_u16_vec(current_exe.as_os_str());

            let set_res = RegSetValueExW(
                hkey,
                APP_REG_NAME,
                Some(0),
                REG_SZ,
                Some(std::slice::from_raw_parts(
                    exe_path_wide.as_ptr() as *const u8,
                    exe_path_wide.len() * 2,
                )),
            );
            if set_res == ERROR_SUCCESS {
                Ok(())
            } else {
                Err(format!(
                    "スタートアップ登録の書き込みに失敗 (エラーコード: {:?})",
                    set_res
                ))
            }
        } else {
            let del_res = RegDeleteValueW(hkey, APP_REG_NAME);
            if del_res == ERROR_SUCCESS || del_res == ERROR_FILE_NOT_FOUND {
                Ok(())
            } else {
                Err(format!(
                    "スタートアップ解除に失敗 (エラーコード: {:?})",
                    del_res
                ))
            }
        };

        let _ = RegCloseKey(hkey);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_startup_check() {
        let _ = is_startup_enabled();
    }
}
