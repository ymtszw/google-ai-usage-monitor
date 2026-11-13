//! ワンショット CLI による `language_server.exe` 健全性診断モジュール

use crate::quota::{
    extract_csrf_token, find_all_language_server_pids, get_listening_ports_for_pid,
    get_process_command_line, query_quota_endpoint,
};

/// 診断項目の判定ステータス
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
    Ok,
    Warn,
    Err,
}

impl HealthStatus {
    /// CLI 表示用バッジ文字列
    pub fn badge(&self) -> &'static str {
        match self {
            HealthStatus::Ok => "[OK]",
            HealthStatus::Warn => "[WARN]",
            HealthStatus::Err => "[ERR]",
        }
    }
}

/// 個別診断項目の結果
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticItem {
    pub name: &'static str,
    pub status: HealthStatus,
    pub message: String,
}

/// 診断対象システムから収集された情報
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerDiagnosticInfo {
    pub pids: Vec<u32>,
    pub selected_pid: Option<u32>,
    pub csrf_token: Option<String>,
    pub ports: Vec<u16>,
    pub endpoint_result: Result<usize, String>,
}

/// 診断レポート全体
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticReport {
    pub items: Vec<DiagnosticItem>,
    pub overall: HealthStatus,
    pub summary: String,
}

impl DiagnosticReport {
    /// ターミナル向けの見やすい診断テーブルを整形
    pub fn format_cli(&self) -> String {
        let mut out = String::new();
        out.push_str("============================================================\n");
        out.push_str(" Antigravity Language Server Diagnostics\n");
        out.push_str("============================================================\n");
        for item in &self.items {
            out.push_str(&format!(
                " {:<6} {:<11} : {}\n",
                item.status.badge(),
                item.name,
                item.message
            ));
        }
        out.push_str("------------------------------------------------------------\n");
        let overall_label = match self.overall {
            HealthStatus::Ok => "HEALTHY (All checks passed)",
            HealthStatus::Warn => "WARNING (Review warnings above)",
            HealthStatus::Err => "ERROR (Diagnostics failed)",
        };
        out.push_str(&format!(" Status  : {}\n", overall_label));
        out.push_str(&format!(" Summary : {}\n", self.summary));
        out.push_str("============================================================");
        out
    }
}

/// 収集された診断情報を元に各項目の健全性と総合判定を評価
pub fn evaluate_diagnostics(info: &ServerDiagnosticInfo) -> DiagnosticReport {
    let mut items = Vec::new();

    // 1. プロセス存在チェック
    if info.pids.is_empty() {
        items.push(DiagnosticItem {
            name: "Process",
            status: HealthStatus::Err,
            message: "language_server.exe is not running".to_string(),
        });
        items.push(DiagnosticItem {
            name: "CSRF Token",
            status: HealthStatus::Err,
            message: "Skipped (no process found)".to_string(),
        });
        items.push(DiagnosticItem {
            name: "TCP Ports",
            status: HealthStatus::Err,
            message: "Skipped (no process found)".to_string(),
        });
        items.push(DiagnosticItem {
            name: "Endpoint",
            status: HealthStatus::Err,
            message: "Skipped (no process found)".to_string(),
        });

        return DiagnosticReport {
            items,
            overall: HealthStatus::Err,
            summary: "language_server.exe process is not running. Launch Antigravity first."
                .to_string(),
        };
    }

    let pid = info.selected_pid.unwrap_or(info.pids[0]);
    if info.pids.len() == 1 {
        items.push(DiagnosticItem {
            name: "Process",
            status: HealthStatus::Ok,
            message: format!("language_server.exe is running (PID: {})", pid),
        });
    } else {
        items.push(DiagnosticItem {
            name: "Process",
            status: HealthStatus::Ok,
            message: format!(
                "language_server.exe is running (Active PID: {}, Total instances: {:?})",
                pid, info.pids
            ),
        });
    }

    // 2. CSRF トークンチェック
    match &info.csrf_token {
        Some(token) if !token.is_empty() => {
            let masked = if token.len() > 8 {
                format!("{}...{}", &token[..4], &token[token.len() - 4..])
            } else {
                "***".to_string()
            };
            items.push(DiagnosticItem {
                name: "CSRF Token",
                status: HealthStatus::Ok,
                message: format!(
                    "Present in command line arguments ({}, len: {})",
                    masked,
                    token.len()
                ),
            });
        }
        _ => {
            items.push(DiagnosticItem {
                name: "CSRF Token",
                status: HealthStatus::Err,
                message: "Missing --csrf_token in process command line".to_string(),
            });
        }
    }

    // 3. リッスンポートチェック
    if info.ports.is_empty() {
        items.push(DiagnosticItem {
            name: "TCP Ports",
            status: HealthStatus::Warn,
            message: format!("No listening IPv4 TCP ports found for PID {}", pid),
        });
    } else {
        items.push(DiagnosticItem {
            name: "TCP Ports",
            status: HealthStatus::Ok,
            message: format!("Listening on port(s): {:?}", info.ports),
        });
    }

    // 4. API エンドポイント疎通チェック
    match &info.endpoint_result {
        Ok(groups_count) => {
            items.push(DiagnosticItem {
                name: "Endpoint",
                status: HealthStatus::Ok,
                message: format!(
                    "RetrieveUserQuotaSummary responded successfully ({} quota groups)",
                    groups_count
                ),
            });
        }
        Err(err) => {
            items.push(DiagnosticItem {
                name: "Endpoint",
                status: HealthStatus::Err,
                message: format!("RPC query failed: {}", err),
            });
        }
    }

    // 総合ステータス判定
    let has_err = items.iter().any(|i| i.status == HealthStatus::Err);
    let has_warn = items.iter().any(|i| i.status == HealthStatus::Warn);

    let (overall, summary) = if has_err {
        (
            HealthStatus::Err,
            "One or more diagnostic checks failed. Check process, CSRF token, and ports."
                .to_string(),
        )
    } else if has_warn {
        (
            HealthStatus::Warn,
            "Diagnostics succeeded with warnings. Check port binding and firewalls.".to_string(),
        )
    } else {
        (
            HealthStatus::Ok,
            "All components are healthy. language_server.exe is ready for quota polling."
                .to_string(),
        )
    };

    DiagnosticReport {
        items,
        overall,
        summary,
    }
}

/// 実際のシステムからプロセス、CSRF トークン、ポート、エンドポイント情報を収集
pub fn collect_diagnostic_info() -> ServerDiagnosticInfo {
    let pids = find_all_language_server_pids();
    if pids.is_empty() {
        return ServerDiagnosticInfo {
            pids: Vec::new(),
            selected_pid: None,
            csrf_token: None,
            ports: Vec::new(),
            endpoint_result: Err("Process not running".to_string()),
        };
    }

    // 複数のプロセスがある場合、CSRF トークンとリッスンポートが存在するものを優先選択
    let mut selected_pid = None;
    let mut selected_csrf = None;
    let mut selected_ports = Vec::new();

    for &pid in &pids {
        let csrf = get_process_command_line(pid).and_then(|cmd| extract_csrf_token(&cmd));
        let ports = get_listening_ports_for_pid(pid);

        if csrf.is_some() && !ports.is_empty() {
            selected_pid = Some(pid);
            selected_csrf = csrf;
            selected_ports = ports;
            break;
        }
    }

    // 両方揃ったプロセスがなければ、最初のプロセスを診断対象とする
    let target_pid = selected_pid.unwrap_or(pids[0]);
    let target_csrf = selected_csrf
        .or_else(|| get_process_command_line(target_pid).and_then(|cmd| extract_csrf_token(&cmd)));
    let target_ports = if selected_ports.is_empty() {
        get_listening_ports_for_pid(target_pid)
    } else {
        selected_ports
    };

    // エンドポイント疎通確認
    let endpoint_result = match (&target_csrf, target_ports.first()) {
        (Some(csrf), Some(&port)) => {
            let mut last_err = String::new();
            let mut success_groups = None;

            for &p in &target_ports {
                match query_quota_endpoint(p, csrf) {
                    Ok(wrapper) => {
                        success_groups = Some(wrapper.response.groups.len());
                        break;
                    }
                    Err(e) => {
                        last_err = e;
                    }
                }
            }

            match success_groups {
                Some(count) => Ok(count),
                None => Err(if last_err.is_empty() {
                    format!("Failed to connect on port {}", port)
                } else {
                    last_err
                }),
            }
        }
        (None, _) => Err("Cannot test endpoint: CSRF token unavailable".to_string()),
        (_, None) => Err("Cannot test endpoint: No listening ports detected".to_string()),
    };

    ServerDiagnosticInfo {
        pids,
        selected_pid: Some(target_pid),
        csrf_token: target_csrf,
        ports: target_ports,
        endpoint_result,
    }
}

/// 診断を実行してレポートを生成
pub fn diagnose_language_server() -> DiagnosticReport {
    let info = collect_diagnostic_info();
    evaluate_diagnostics(&info)
}

/// `--status` サブコマンドのメイン実行ルーチン
pub fn run_status() -> Result<(), String> {
    let report = diagnose_language_server();
    println!("{}", report.format_cli());

    if report.overall == HealthStatus::Err {
        Err("診断でエラーが検出されました。".to_string())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_evaluate_diagnostics_all_ok() {
        let info = ServerDiagnosticInfo {
            pids: vec![12345],
            selected_pid: Some(12345),
            csrf_token: Some("abcde-12345-67890-test-token".to_string()),
            ports: vec![54321],
            endpoint_result: Ok(3),
        };

        let report = evaluate_diagnostics(&info);
        assert_eq!(report.overall, HealthStatus::Ok);
        assert_eq!(report.items.len(), 4);
        assert_eq!(report.items[0].status, HealthStatus::Ok);
        assert_eq!(report.items[1].status, HealthStatus::Ok);
        assert_eq!(report.items[2].status, HealthStatus::Ok);
        assert_eq!(report.items[3].status, HealthStatus::Ok);
        assert!(report.summary.contains("healthy"));

        let formatted = report.format_cli();
        assert!(formatted.contains("[OK]"));
        assert!(formatted.contains("Process"));
        assert!(formatted.contains("CSRF Token"));
        assert!(formatted.contains("TCP Ports"));
        assert!(formatted.contains("Endpoint"));
        assert!(formatted.contains("HEALTHY"));
        assert!(formatted.contains("12345"));
        assert!(formatted.contains("54321"));
    }

    #[test]
    fn test_evaluate_diagnostics_no_process() {
        let info = ServerDiagnosticInfo {
            pids: Vec::new(),
            selected_pid: None,
            csrf_token: None,
            ports: Vec::new(),
            endpoint_result: Err("No process".to_string()),
        };

        let report = evaluate_diagnostics(&info);
        assert_eq!(report.overall, HealthStatus::Err);
        assert_eq!(report.items[0].status, HealthStatus::Err);
        assert!(report.items[0].message.contains("not running"));
        assert!(report.summary.contains("Launch Antigravity first"));

        let formatted = report.format_cli();
        assert!(formatted.contains("[ERR]"));
        assert!(formatted.contains("Process"));
        assert!(formatted.contains("ERROR"));
    }

    #[test]
    fn test_evaluate_diagnostics_missing_csrf() {
        let info = ServerDiagnosticInfo {
            pids: vec![12345],
            selected_pid: Some(12345),
            csrf_token: None,
            ports: vec![54321],
            endpoint_result: Err("CSRF token unavailable".to_string()),
        };

        let report = evaluate_diagnostics(&info);
        assert_eq!(report.overall, HealthStatus::Err);
        assert_eq!(report.items[0].status, HealthStatus::Ok);
        assert_eq!(report.items[1].status, HealthStatus::Err);
        assert!(report.items[1].message.contains("Missing --csrf_token"));
    }

    #[test]
    fn test_evaluate_diagnostics_no_ports_and_endpoint_err() {
        let info = ServerDiagnosticInfo {
            pids: vec![12345],
            selected_pid: Some(12345),
            csrf_token: Some("valid-token".to_string()),
            ports: Vec::new(),
            endpoint_result: Err("No listening ports detected".to_string()),
        };

        let report = evaluate_diagnostics(&info);
        assert_eq!(report.overall, HealthStatus::Err);
        assert_eq!(report.items[2].status, HealthStatus::Warn);
        assert_eq!(report.items[3].status, HealthStatus::Err);
    }

    #[test]
    fn test_evaluate_diagnostics_multiple_pids() {
        let info = ServerDiagnosticInfo {
            pids: vec![12345, 67890],
            selected_pid: Some(67890),
            csrf_token: Some("my-token-12345678".to_string()),
            ports: vec![50000, 50001],
            endpoint_result: Ok(2),
        };

        let report = evaluate_diagnostics(&info);
        assert_eq!(report.overall, HealthStatus::Ok);
        assert!(report.items[0].message.contains("Active PID: 67890"));
        assert!(
            report.items[0]
                .message
                .contains("Total instances: [12345, 67890]")
        );
    }
}
