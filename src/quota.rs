use serde::{Deserialize, Serialize};
use std::ffi::c_void;
use std::time::Duration;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, MIB_TCPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_ALL,
};
use windows::Win32::Networking::WinSock::AF_INET;
use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ};

unsafe extern "system" {
    fn NtQueryInformationProcess(
        process_handle: HANDLE,
        process_information_class: u32,
        process_information: *mut c_void,
        process_information_length: u32,
        return_length: *mut u32,
    ) -> i32;
}

#[repr(C)]
struct ProcessBasicInformation {
    _exit_status: i32,
    peb_base_address: *mut c_void,
    _affinity_mask: usize,
    _base_priority: i32,
    _unique_process_id: usize,
    _inherited_from_unique_process_id: usize,
}

#[repr(C)]
struct UnicodeString {
    length: u16,
    _maximum_length: u16,
    buffer: *mut u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BucketInfo {
    #[serde(rename = "bucketId")]
    pub bucket_id: Option<String>,
    #[serde(rename = "displayName")]
    pub display_name: String,
    pub description: Option<String>,
    pub window: Option<String>,
    #[serde(rename = "remainingFraction")]
    pub remaining_fraction: f64,
    #[serde(rename = "resetTime")]
    pub reset_time: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupInfo {
    #[serde(rename = "displayName")]
    pub display_name: String,
    pub description: Option<String>,
    pub buckets: Vec<BucketInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaResponseWrapper {
    pub response: QuotaResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaResponse {
    pub groups: Vec<GroupInfo>,
    pub description: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct AntigravityQuota {
    pub process_id: u32,
    pub port: u16,
    pub groups: Vec<GroupInfo>,
}

/// プロセスの PEB から正確なコマンドライン文字列を取得
pub fn get_process_command_line(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid).ok()?;
        let mut pbi = std::mem::zeroed::<ProcessBasicInformation>();
        let mut return_len = 0;
        let status = NtQueryInformationProcess(
            handle,
            0,
            &mut pbi as *mut _ as *mut c_void,
            std::mem::size_of::<ProcessBasicInformation>() as u32,
            &mut return_len,
        );

        if status != 0 || pbi.peb_base_address.is_null() {
            let _ = CloseHandle(handle);
            return None;
        }

        let peb = pbi.peb_base_address as *const u8;
        let mut proc_params_ptr: *mut c_void = std::ptr::null_mut();

        if ReadProcessMemory(
            handle,
            peb.add(0x20) as *const c_void,
            &mut proc_params_ptr as *mut _ as *mut c_void,
            std::mem::size_of::<*mut c_void>(),
            None,
        )
        .is_err()
            || proc_params_ptr.is_null()
        {
            let _ = CloseHandle(handle);
            return None;
        }

        let mut cmd_line_str = std::mem::zeroed::<UnicodeString>();
        if ReadProcessMemory(
            handle,
            (proc_params_ptr as *const u8).add(0x70) as *const c_void,
            &mut cmd_line_str as *mut _ as *mut c_void,
            std::mem::size_of::<UnicodeString>(),
            None,
        )
        .is_err()
            || cmd_line_str.buffer.is_null()
            || cmd_line_str.length == 0
        {
            let _ = CloseHandle(handle);
            return None;
        }

        let char_count = (cmd_line_str.length / 2) as usize;
        let mut buffer: Vec<u16> = vec![0; char_count];
        if ReadProcessMemory(
            handle,
            cmd_line_str.buffer as *const c_void,
            buffer.as_mut_ptr() as *mut c_void,
            cmd_line_str.length as usize,
            None,
        )
        .is_err()
        {
            let _ = CloseHandle(handle);
            return None;
        }

        let _ = CloseHandle(handle);
        Some(String::from_utf16_lossy(&buffer))
    }
}

/// language_server.exe の PID と CSRF トークンを検索
pub fn find_language_server_info() -> Option<(u32, String)> {
    let re_csrf = regex::Regex::new(r"--csrf_token\s+([a-zA-Z0-9\-]+)").ok()?;

    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
        let mut entry = std::mem::zeroed::<PROCESSENTRY32W>();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let name = String::from_utf16_lossy(&entry.szExeFile)
                    .trim_matches('\0')
                    .to_string();

                if name.eq_ignore_ascii_case("language_server.exe") {
                    let pid = entry.th32ProcessID;
                    if let Some(cmd) = get_process_command_line(pid)
                        && let Some(csrf) = re_csrf.captures(&cmd).and_then(|caps| caps.get(1))
                    {
                        let _ = CloseHandle(snapshot);
                        return Some((pid, csrf.as_str().to_string()));
                    }
                }

                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
    }
    None
}

/// 指定 PID がリッスンしている IPv4 TCP ポートの一覧を取得
pub fn get_listening_ports_for_pid(target_pid: u32) -> Vec<u16> {
    let mut ports = Vec::new();
    let mut size: u32 = 0;

    unsafe {
        let _ = GetExtendedTcpTable(
            None,
            &mut size,
            true,
            AF_INET.0 as u32,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        );

        if size == 0 {
            return ports;
        }

        let mut buffer: Vec<u8> = vec![0; size as usize];
        let res = GetExtendedTcpTable(
            Some(buffer.as_mut_ptr() as *mut _),
            &mut size,
            true,
            AF_INET.0 as u32,
            TCP_TABLE_OWNER_PID_ALL,
            0,
        );

        if res == 0 {
            let table = &*(buffer.as_ptr() as *const MIB_TCPTABLE_OWNER_PID);
            let entries =
                std::slice::from_raw_parts(table.table.as_ptr(), table.dwNumEntries as usize);

            for entry in entries {
                if entry.dwOwningPid == target_pid && entry.dwState == 2 {
                    let port = u16::from_be(entry.dwLocalPort as u16);
                    if !ports.contains(&port) {
                        ports.push(port);
                    }
                }
            }
        }
    }

    ports
}

/// Antigravity ローカル API に接続してクォータを取得
pub fn fetch_quota() -> Result<AntigravityQuota, String> {
    let (pid, csrf) = find_language_server_info()
        .ok_or_else(|| "Antigravity language_server.exe が検出できませんでした。".to_string())?;

    let ports = get_listening_ports_for_pid(pid);
    if ports.is_empty() {
        return Err(format!(
            "プロセス (PID: {}) のリッスンポートが見つかりませんでした。",
            pid
        ));
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|e| e.to_string())?;

    for port in ports {
        let url = format!(
            "http://127.0.0.1:{}/exa.language_server_pb.LanguageServerService/RetrieveUserQuotaSummary",
            port
        );

        let res = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("X-Codeium-Csrf-Token", &csrf)
            .body("{}")
            .send();

        if let Ok(resp) = res
            && let Ok(wrapper) = resp
                .error_for_status()
                .and_then(|r| r.json::<QuotaResponseWrapper>())
        {
            return Ok(AntigravityQuota {
                process_id: pid,
                port,
                groups: wrapper.response.groups,
            });
        }
    }

    Err("有効な language_server API ポートと通信できませんでした。".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fetch_both_quota_groups() {
        if let Ok(quota) = fetch_quota() {
            assert!(!quota.groups.is_empty(), "Quota groups should not be empty");
            let has_gemini = quota
                .groups
                .iter()
                .any(|g| g.display_name.contains("Gemini"));
            let has_claude = quota
                .groups
                .iter()
                .any(|g| g.display_name.contains("Claude") || g.display_name.contains("GPT"));
            assert!(has_gemini, "Should contain Gemini quota group");
            assert!(has_claude, "Should contain Claude/GPT quota group");
        }
    }
}
