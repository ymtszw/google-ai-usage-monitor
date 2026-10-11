mod overlay;
mod quota;
mod tracker;

use overlay::{GaugeData, TachometerOverlay};
use quota::fetch_quota;
use std::time::{Duration, Instant};
use tracker::{find_antigravity_window, get_target_bounds};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
};

fn extract_gemini_gauge_data() -> Option<GaugeData> {
    let quota = fetch_quota().ok()?;
    let gemini = quota
        .groups
        .into_iter()
        .find(|g| g.display_name == "Gemini Models")?;

    let mut weekly_pct = 100.0;
    let mut five_h_pct = 100.0;

    for b in gemini.buckets {
        if b.bucket_id.as_deref() == Some("gemini-weekly") {
            weekly_pct = b.remaining_fraction * 100.0;
        } else if b.bucket_id.as_deref() == Some("gemini-5h") {
            five_h_pct = b.remaining_fraction * 100.0;
        }
    }

    Some(GaugeData {
        weekly_pct,
        five_h_pct,
    })
}

fn main() {
    println!("============================================================");
    println!(" Antigravity Quota HUD (Rust Native Tachometer Overlay)");
    println!("============================================================");

    // 1. タコメーターオーバーレイウィンドウの作成
    let overlay = match TachometerOverlay::new() {
        Ok(o) => {
            println!(" [OK] タコメーターHUD ウィンドウを作成しました。");
            o
        }
        Err(e) => {
            eprintln!(" [ERROR] HUD 作成に失敗: {}", e);
            return;
        }
    };

    println!(" Antigravity ウィンドウを検索・追従中...\n");

    let mut last_fetch = Instant::now() - Duration::from_secs(300);
    let mut last_bounds = None;
    let mut last_move_time = Instant::now() - Duration::from_secs(10);
    let mut is_moving = false;

    // メッセージループ & 追従ループ
    loop {
        // Windows メッセージのディスパッチ
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        // 5分ごとにクォータデータを最新化 (初回は即時)
        if last_fetch.elapsed() >= Duration::from_secs(300) {
            if let Some(data) = extract_gemini_gauge_data() {
                println!(
                    " [Quota Update] Weekly: {:.1}%, 5-Hour: {:.1}%",
                    data.weekly_pct, data.five_h_pct
                );
                overlay.update_data(data);
            }
            last_fetch = Instant::now();
        }

        // Antigravity ウィンドウの探索・追従
        if let Some(target) = find_antigravity_window() {
            if let Some(bounds) = get_target_bounds(target) {
                let overlay_x = bounds.right - overlay::HUD_WIDTH as i32 - 140;
                let overlay_y = bounds.top + 38;

                if let Some(prev) = last_bounds {
                    if prev != bounds {
                        // 移動またはリサイズ検知 -> 即座にフェードアウトを開始し、移動完了まで待機
                        if !is_moving {
                            overlay.fade_out();
                            is_moving = true;
                        }
                        last_move_time = Instant::now();
                    }
                } else {
                    // 初回検知時: 新位置へ配置してフェードイン
                    overlay.set_position(overlay_x, overlay_y);
                    overlay.fade_in();
                }

                last_bounds = Some(bounds);

                if is_moving {
                    // 移動中: 最後の座標変化から 280ms 完全に静止したら定常状態とみなす (debounce)
                    if last_move_time.elapsed() >= Duration::from_millis(280) {
                        // 静止確定: 落ち着いた新位置へテレポートさせてから上品にフェードイン
                        overlay.set_position(overlay_x, overlay_y);
                        overlay.fade_in();
                        is_moving = false;
                    }
                }
            } else {
                if !is_moving {
                    overlay.fade_out();
                    is_moving = true;
                }
                last_bounds = None;
            }
        } else {
            if !is_moving {
                overlay.fade_out();
                is_moving = true;
            }
            last_bounds = None;
        }

        std::thread::sleep(Duration::from_millis(16)); // 60fps (16ms) 追従
    }
}
