//! language_server.exe の挙動をエミュレートするローカル TCP モックサーバー
//!
//! Antigravity 内部の language_server RPC (`RetrieveUserQuotaSummary`) を模倣し、
//! テスト環境でクォータ取得ロジックを検証するためのユーティリティです。

use google_ai_usage_monitor::quota::query_quota_endpoint;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// デフォルトのモック QuotaResponse JSON
pub const DEFAULT_MOCK_QUOTA_JSON: &str = r#"{
  "response": {
    "groups": [
      {
        "displayName": "Gemini 2.5 Pro",
        "description": "Standard Gemini Quota",
        "buckets": [
          {
            "bucketId": "gemini-weekly",
            "displayName": "Weekly Limit",
            "remainingFraction": 0.85,
            "resetTime": "2026-11-20T00:00:00Z"
          },
          {
            "bucketId": "gemini-5h",
            "displayName": "5h Limit",
            "remainingFraction": 0.60,
            "resetTime": "2026-11-13T23:59:59Z"
          }
        ]
      },
      {
        "displayName": "Argon Quota",
        "description": "Argon & Orchestration Limit",
        "buckets": [
          {
            "bucketId": "argon-weekly",
            "displayName": "Weekly Limit",
            "remainingFraction": 0.40,
            "resetTime": "2026-11-20T00:00:00Z"
          }
        ]
      },
      {
        "displayName": "Claude 3.7 Sonnet / GPT-4o",
        "description": "Claude & GPT Limit",
        "buckets": [
          {
            "bucketId": "claude-weekly",
            "displayName": "Weekly Limit",
            "remainingFraction": 0.95,
            "resetTime": "2026-11-20T00:00:00Z"
          }
        ]
      }
    ],
    "description": "Mock user quota summary for testing"
  }
}"#;

pub struct MockLanguageServer {
    port: u16,
    running: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl MockLanguageServer {
    /// ローカル TCP モックサーバーを空きポートで起動
    pub fn start(
        expected_csrf: Option<String>,
        response_json: Option<String>,
    ) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;

        let running = Arc::new(AtomicBool::new(true));
        let running_clone = Arc::clone(&running);

        let body = response_json.unwrap_or_else(|| DEFAULT_MOCK_QUOTA_JSON.to_string());

        let handle = thread::spawn(move || {
            while running_clone.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ =
                            Self::handle_connection(&mut stream, expected_csrf.as_deref(), &body);
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            port,
            running,
            handle: Some(handle),
        })
    }

    /// リッスン中のポート番号を取得
    pub fn port(&self) -> u16 {
        self.port
    }

    fn handle_connection(
        stream: &mut TcpStream,
        expected_csrf: Option<&str>,
        response_body: &str,
    ) -> std::io::Result<()> {
        let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
        let mut buffer = [0u8; 4096];
        let bytes_read = stream.read(&mut buffer)?;
        if bytes_read == 0 {
            return Ok(());
        }

        let request = String::from_utf8_lossy(&buffer[..bytes_read]);

        // パス判定
        let target_path = "/exa.language_server_pb.LanguageServerService/RetrieveUserQuotaSummary";
        let is_target_endpoint = request.starts_with("POST ") && request.contains(target_path);

        if !is_target_endpoint {
            let response =
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            stream.write_all(response.as_bytes())?;
            stream.flush()?;
            return Ok(());
        }

        // CSRF トークン検証 (指定されている場合)
        if let Some(csrf) = expected_csrf {
            let header_lower = format!("x-codeium-csrf-token: {}", csrf.to_lowercase());
            let request_lower = request.to_lowercase();
            if !request_lower.contains(&header_lower) {
                let body = "Forbidden: Invalid CSRF Token";
                let response = format!(
                    "HTTP/1.1 403 Forbidden\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(response.as_bytes())?;
                stream.flush()?;
                return Ok(());
            }
        }

        // 正常応答 (200 OK)
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream.write_all(response.as_bytes())?;
        stream.flush()?;
        Ok(())
    }
}

impl Drop for MockLanguageServer {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[test]
fn test_mock_language_server_success() {
    let csrf_token = "mock-secret-csrf-token-12345";
    let server = MockLanguageServer::start(Some(csrf_token.to_string()), None)
        .expect("Failed to start mock server");

    let quota = query_quota_endpoint(server.port(), csrf_token)
        .expect("query_quota_endpoint should succeed against mock server");

    let groups = quota.response.groups;
    assert_eq!(groups.len(), 3);
    assert_eq!(groups[0].display_name, "Gemini 2.5 Pro");
    assert_eq!(groups[0].buckets.len(), 2);
    assert_eq!(groups[0].buckets[0].remaining_fraction, 0.85);

    assert_eq!(groups[1].display_name, "Argon Quota");
    assert_eq!(groups[1].buckets[0].remaining_fraction, 0.40);

    assert_eq!(groups[2].display_name, "Claude 3.7 Sonnet / GPT-4o");
    assert_eq!(groups[2].buckets[0].remaining_fraction, 0.95);
}

#[test]
fn test_mock_language_server_csrf_rejection() {
    let correct_csrf = "expected-token";
    let server = MockLanguageServer::start(Some(correct_csrf.to_string()), None)
        .expect("Failed to start mock server");

    let res = query_quota_endpoint(server.port(), "invalid-token");
    assert!(res.is_err(), "Invalid CSRF token should be rejected");
    let err_msg = res.err().unwrap();
    assert!(
        err_msg.contains("403") || err_msg.contains("status"),
        "Error should reflect 403 HTTP status"
    );
}

#[test]
fn test_mock_language_server_custom_payload() {
    let custom_json = r#"{
      "response": {
        "groups": [
          {
            "displayName": "Custom Test Model",
            "buckets": [
              {
                "bucketId": "test-bucket",
                "displayName": "Test Limit",
                "remainingFraction": 1.0,
                "resetTime": null
              }
            ]
          }
        ]
      }
    }"#;

    let server = MockLanguageServer::start(None, Some(custom_json.to_string()))
        .expect("Failed to start mock server");

    let quota = query_quota_endpoint(server.port(), "any-token")
        .expect("query_quota_endpoint with custom payload should succeed");

    assert_eq!(quota.response.groups.len(), 1);
    assert_eq!(quota.response.groups[0].display_name, "Custom Test Model");
    assert_eq!(quota.response.groups[0].buckets[0].remaining_fraction, 1.0);
}
