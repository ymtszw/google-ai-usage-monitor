//! ワンショット CLI によるクォータ情報の JSON エクスポートモジュール

use crate::quota::{AntigravityQuota, fetch_quota};

/// クォータ構造体を人間が読みやすいインデント付き JSON 文字列へシリアライズ
pub fn format_quota_json(quota: &AntigravityQuota) -> Result<String, String> {
    serde_json::to_string_pretty(quota).map_err(|e| format!("JSON シリアライズ失敗: {}", e))
}

/// `--dump-json` サブコマンドのメイン実行ルーチン
///
/// 現在稼働中の `language_server.exe` からクォータ情報を取得し、
/// 整形 JSON 文字列として標準出力へ即時ダンプします。
pub fn run_dump_json() -> Result<(), String> {
    let quota = fetch_quota()?;
    let json = format_quota_json(&quota)?;
    println!("{}", json);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quota::{BucketInfo, GroupInfo};

    #[test]
    fn test_format_quota_json_structure() {
        let dummy_quota = AntigravityQuota {
            process_id: 12345,
            port: 54321,
            groups: vec![GroupInfo {
                display_name: "Gemini 2.5 Pro".to_string(),
                description: Some("Model quota group".to_string()),
                buckets: vec![
                    BucketInfo {
                        bucket_id: Some("gemini-weekly".to_string()),
                        display_name: "Weekly Limit".to_string(),
                        description: None,
                        window: Some("WEEKLY".to_string()),
                        remaining_fraction: 0.85,
                        reset_time: Some("2026-11-20T00:00:00Z".to_string()),
                    },
                    BucketInfo {
                        bucket_id: Some("gemini-5h".to_string()),
                        display_name: "5h Limit".to_string(),
                        description: None,
                        window: Some("WINDOW_5H".to_string()),
                        remaining_fraction: 0.50,
                        reset_time: Some("2026-11-13T23:59:59Z".to_string()),
                    },
                ],
            }],
        };

        let json = format_quota_json(&dummy_quota).expect("Serialization should succeed");
        assert!(json.contains("\"process_id\": 12345"));
        assert!(json.contains("\"port\": 54321"));
        assert!(json.contains("\"displayName\": \"Gemini 2.5 Pro\""));
        assert!(json.contains("\"bucketId\": \"gemini-weekly\""));
        assert!(json.contains("\"remainingFraction\": 0.85"));

        // JSON として正しくデシリアライズできるか逆検証
        let parsed: AntigravityQuota =
            serde_json::from_str(&json).expect("Deserialization should match");
        assert_eq!(parsed.process_id, 12345);
        assert_eq!(parsed.port, 54321);
        assert_eq!(parsed.groups.len(), 1);
        assert_eq!(parsed.groups[0].buckets.len(), 2);
    }
}
