mod quota;

use quota::fetch_quota;

fn main() {
    println!("============================================================");
    println!(" Google Antigravity AI Quota Monitor (Rust Native)");
    println!("============================================================");

    match fetch_quota() {
        Ok(q) => {
            println!(" 検出成功! PID: {}, 接続ポート: {}\n", q.process_id, q.port);

            for group in q.groups {
                println!("[{}]", group.display_name);
                if let Some(desc) = group.description {
                    println!("  {}", desc);
                }

                for b in group.buckets {
                    let pct = b.remaining_fraction * 100.0;
                    let reset = b.reset_time.unwrap_or_else(|| "未定".to_string());
                    println!(
                        "  * {:<28}: {:>5.1}% (リセット: {})",
                        b.display_name, pct, reset
                    );
                }
                println!();
            }
        }
        Err(e) => {
            eprintln!(" エラー: {}", e);
        }
    }
}