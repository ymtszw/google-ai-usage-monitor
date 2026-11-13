//! クォータ取得スナップショットの履歴記録モジュール (CSV 追記保存)

use crate::overlay::GaugeData;
use chrono::{DateTime, Utc};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// CSV ファイルのヘッダー行定義
pub const CSV_HEADER: &str =
    "timestamp,gemini_weekly,gemini_5h,argon_weekly,claude_gpt_weekly,claude_gpt_5h";

/// 取得したクォータのスナップショット情報
#[derive(Debug, Clone, PartialEq)]
pub struct QuotaSnapshot {
    /// 取得日時 (RFC3339 形式, 例: 2026-11-13T14:30:00Z)
    pub timestamp: String,
    /// Gemini Weekly クォータ残量 (%)
    pub gemini_weekly: f64,
    /// Gemini 5-Hour Burst クォータ残量 (%)
    pub gemini_5h: f64,
    /// Argon / Orchestrator Weekly クォータ残量 (%)
    pub argon_weekly: f64,
    /// Claude / GPT Weekly クォータ残量 (%)
    pub claude_gpt_weekly: f64,
    /// Claude / GPT 5-Hour Burst クォータ残量 (%)
    pub claude_gpt_5h: f64,
}

impl QuotaSnapshot {
    /// 新規スナップショットを作成
    pub fn new(
        timestamp: String,
        gemini_weekly: f64,
        gemini_5h: f64,
        argon_weekly: f64,
        claude_gpt_weekly: f64,
        claude_gpt_5h: f64,
    ) -> Self {
        Self {
            timestamp,
            gemini_weekly,
            gemini_5h,
            argon_weekly,
            claude_gpt_weekly,
            claude_gpt_5h,
        }
    }

    /// 現在時刻 (UTC) と GaugeData からスナップショットを生成
    pub fn from_gauge_data(data: &GaugeData) -> Self {
        let now: DateTime<Utc> = Utc::now();
        Self::from_gauge_data_with_time(data, now.to_rfc3339())
    }

    /// 指定したタイムスタンプ文字列と GaugeData からスナップショットを生成
    pub fn from_gauge_data_with_time(data: &GaugeData, timestamp: String) -> Self {
        Self {
            timestamp,
            gemini_weekly: data.gemini.weekly_pct,
            gemini_5h: data.gemini.five_h_pct,
            argon_weekly: data.argon.weekly_pct,
            claude_gpt_weekly: data.claude_gpt.weekly_pct,
            claude_gpt_5h: data.claude_gpt.five_h_pct,
        }
    }

    /// CSV 行文字列 (末尾改行なし) へ変換
    pub fn to_csv_row(&self) -> String {
        format!(
            "{},{:.1},{:.1},{:.1},{:.1},{:.1}",
            self.timestamp,
            self.gemini_weekly,
            self.gemini_5h,
            self.argon_weekly,
            self.claude_gpt_weekly,
            self.claude_gpt_5h
        )
    }

    /// CSV 行文字列からパース
    pub fn from_csv_row(line: &str) -> Result<Self, String> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Err("空の行です".to_string());
        }

        let parts: Vec<&str> = trimmed.split(',').map(|s| s.trim()).collect();
        if parts.len() != 6 {
            return Err(format!(
                "カラム数が不正です (期待値: 6, 実際: {}): {}",
                parts.len(),
                line
            ));
        }

        let timestamp = parts[0].to_string();
        let gemini_weekly = parts[1]
            .parse::<f64>()
            .map_err(|e| format!("gemini_weekly のパース失敗: {}", e))?;
        let gemini_5h = parts[2]
            .parse::<f64>()
            .map_err(|e| format!("gemini_5h のパース失敗: {}", e))?;
        let argon_weekly = parts[3]
            .parse::<f64>()
            .map_err(|e| format!("argon_weekly のパース失敗: {}", e))?;
        let claude_gpt_weekly = parts[4]
            .parse::<f64>()
            .map_err(|e| format!("claude_gpt_weekly のパース失敗: {}", e))?;
        let claude_gpt_5h = parts[5]
            .parse::<f64>()
            .map_err(|e| format!("claude_gpt_5h のパース失敗: {}", e))?;

        Ok(Self {
            timestamp,
            gemini_weekly,
            gemini_5h,
            argon_weekly,
            claude_gpt_weekly,
            claude_gpt_5h,
        })
    }
}

/// クォータ履歴を CSV ファイルへ保存・管理するロガー構造体
#[derive(Debug, Clone)]
pub struct HistoryLogger {
    log_path: PathBuf,
}

impl HistoryLogger {
    /// 指定されたパスをターゲットとする履歴ロガーを作成
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        Self {
            log_path: path.as_ref().to_path_buf(),
        }
    }

    /// デフォルトの保存先パス (`target/quota_history.csv`) を取得
    pub fn default_path() -> PathBuf {
        PathBuf::from("target").join("quota_history.csv")
    }

    /// 対象のファイルパスを取得
    pub fn path(&self) -> &Path {
        &self.log_path
    }

    /// スナップショットを CSV ファイル末尾に追記
    /// ファイルが存在しない、またはサイズ 0 の場合はヘッダー行を自動書き込みします。
    pub fn append(&self, snapshot: &QuotaSnapshot) -> std::io::Result<()> {
        if let Some(parent) = self.log_path.parent()
            && !parent.as_os_str().is_empty()
            && !parent.exists()
        {
            std::fs::create_dir_all(parent)?;
        }

        let needs_header = match std::fs::metadata(&self.log_path) {
            Ok(meta) => meta.len() == 0,
            Err(_) => true,
        };

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)?;

        if needs_header {
            writeln!(file, "{}", CSV_HEADER)?;
        }

        writeln!(file, "{}", snapshot.to_csv_row())?;
        file.flush()?;
        Ok(())
    }

    /// CSV ファイル内の全スナップショットを読み込み
    pub fn read_all(&self) -> Result<Vec<QuotaSnapshot>, String> {
        if !self.log_path.exists() {
            return Ok(Vec::new());
        }

        let file =
            File::open(&self.log_path).map_err(|e| format!("ファイルオープン失敗: {}", e))?;
        let reader = BufReader::new(file);
        let mut snapshots = Vec::new();

        for (idx, line_res) in reader.lines().enumerate() {
            let line = line_res.map_err(|e| format!("行読み込み失敗 (行 {}): {}", idx + 1, e))?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            // 最初のヘッダー行をスキップ
            if idx == 0 && trimmed.starts_with("timestamp,") {
                continue;
            }

            let snapshot = QuotaSnapshot::from_csv_row(trimmed)
                .map_err(|e| format!("行 {} のパース失敗: {}", idx + 1, e))?;
            snapshots.push(snapshot);
        }

        Ok(snapshots)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::GroupQuota;
    use std::io::Write;

    #[test]
    fn test_quota_snapshot_row_conversion_and_parse() {
        let snapshot = QuotaSnapshot::new(
            "2026-11-13T14:30:00Z".to_string(),
            85.4,
            100.0,
            52.0,
            90.5,
            75.0,
        );

        let row = snapshot.to_csv_row();
        assert_eq!(row, "2026-11-13T14:30:00Z,85.4,100.0,52.0,90.5,75.0");

        let parsed = QuotaSnapshot::from_csv_row(&row).expect("パース成功すべき");
        assert_eq!(parsed, snapshot);
    }

    #[test]
    fn test_quota_snapshot_parse_errors() {
        // カラム不足
        let err1 = QuotaSnapshot::from_csv_row("2026-11-13T14:30:00Z,85.0,100.0");
        assert!(err1.is_err());
        assert!(err1.unwrap_err().contains("カラム数が不正です"));

        // 数値パース不正
        let err2 = QuotaSnapshot::from_csv_row("2026-11-13T14:30:00Z,invalid,100.0,50.0,90.0,70.0");
        assert!(err2.is_err());
        assert!(err2.unwrap_err().contains("gemini_weekly のパース失敗"));

        // 空行
        let err3 = QuotaSnapshot::from_csv_row("   ");
        assert!(err3.is_err());
        assert!(err3.unwrap_err().contains("空の行です"));
    }

    #[test]
    fn test_from_gauge_data_with_time() {
        let gauge_data = GaugeData {
            gemini: GroupQuota {
                weekly_pct: 78.5,
                five_h_pct: 100.0,
                weekly_reset: None,
                five_h_reset: None,
            },
            argon: GroupQuota {
                weekly_pct: 45.0,
                five_h_pct: 100.0,
                weekly_reset: None,
                five_h_reset: None,
            },
            claude_gpt: GroupQuota {
                weekly_pct: 95.0,
                five_h_pct: 80.0,
                weekly_reset: None,
                five_h_reset: None,
            },
        };

        let snapshot = QuotaSnapshot::from_gauge_data_with_time(
            &gauge_data,
            "2026-11-13T14:35:00Z".to_string(),
        );

        assert_eq!(snapshot.timestamp, "2026-11-13T14:35:00Z");
        assert_eq!(snapshot.gemini_weekly, 78.5);
        assert_eq!(snapshot.gemini_5h, 100.0);
        assert_eq!(snapshot.argon_weekly, 45.0);
        assert_eq!(snapshot.claude_gpt_weekly, 95.0);
        assert_eq!(snapshot.claude_gpt_5h, 80.0);
    }

    #[test]
    fn test_history_logger_append_and_read_all() {
        let temp_dir = std::env::temp_dir().join(format!("test_logger_{}", std::process::id()));
        let csv_path = temp_dir.join("subdir").join("test_history.csv");

        // クリーンアップ
        if csv_path.exists() {
            let _ = std::fs::remove_file(&csv_path);
        }

        let logger = HistoryLogger::new(&csv_path);

        // ファイルが存在しない初期状態では空の Vec を返す
        let initial = logger.read_all().expect("初期読み込み成功");
        assert!(initial.is_empty());

        let s1 = QuotaSnapshot::new(
            "2026-11-13T14:00:00Z".to_string(),
            90.0,
            100.0,
            60.0,
            80.0,
            70.0,
        );
        let s2 = QuotaSnapshot::new(
            "2026-11-13T14:05:00Z".to_string(),
            88.5,
            95.0,
            58.0,
            80.0,
            65.0,
        );

        // 1回目の追記 (ディレクトリ作成およびヘッダー書き込みの検証)
        logger.append(&s1).expect("1回目の append 成功");

        // ファイル内容を直接検証 (ヘッダー + 1行)
        let content = std::fs::read_to_string(&csv_path).expect("ファイル読み取り成功");
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], CSV_HEADER);
        assert_eq!(lines[1], s1.to_csv_row());

        // 2回目の追記 (ヘッダーが再出力されず、行が追加される検証)
        logger.append(&s2).expect("2回目の append 成功");

        let records = logger.read_all().expect("read_all 成功");
        assert_eq!(records.len(), 2);
        assert_eq!(records[0], s1);
        assert_eq!(records[1], s2);

        // 後始末
        let _ = std::fs::remove_file(&csv_path);
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_history_logger_handles_empty_existing_file() {
        let temp_dir =
            std::env::temp_dir().join(format!("test_logger_empty_{}", std::process::id()));
        let csv_path = temp_dir.join("empty.csv");

        std::fs::create_dir_all(&temp_dir).expect("ディレクトリ作成");
        // 空ファイル作成
        let mut f = File::create(&csv_path).expect("空ファイル作成");
        f.flush().unwrap();
        drop(f);

        let logger = HistoryLogger::new(&csv_path);
        let s = QuotaSnapshot::new(
            "2026-11-13T14:10:00Z".to_string(),
            100.0,
            100.0,
            100.0,
            100.0,
            100.0,
        );

        logger.append(&s).expect("空ファイルへの追記成功");

        let content = std::fs::read_to_string(&csv_path).expect("ファイル読み込み");
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], CSV_HEADER);
        assert_eq!(lines[1], s.to_csv_row());

        let _ = std::fs::remove_file(&csv_path);
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
