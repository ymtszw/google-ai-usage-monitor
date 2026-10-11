mod overlay;
mod quota;
mod tracker;

use overlay::{GaugeData, TachometerOverlay};
use quota::fetch_quota;
use std::time::{Duration, Instant};
use tracker::{find_antigravity_window, get_target_bounds, is_dragging, WindowTracker};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
};

fn extract_gemini_gauge_data() -> Option<GaugeData> {
    let quota = fetch_quota().ok()?;
    let gemini = quota
        .groups
        .into_iter()
        .find(|g| g.display_name == "Gemini Models")?;

    let mut weekly_pct = 100.0;
    let mut five_h_pct = 100.0;
    let mut weekly_reset = String::new();
    let mut five_h_reset = String::new();

    for b in gemini.buckets {
        if b.bucket_id.as_deref() == Some("gemini-weekly") {
            weekly_pct = b.remaining_fraction * 100.0;
            weekly_reset = b.reset_time.unwrap_or_default();
        } else if b.bucket_id.as_deref() == Some("gemini-5h") {
            five_h_pct = b.remaining_fraction * 100.0;
            five_h_reset = b.reset_time.unwrap_or_default();
        }
    }

    Some(GaugeData {
        weekly_pct,
        five_h_pct,
        weekly_reset,
        five_h_reset,
    })
}

fn main() {
    println!("============================================================");
    println!(" Antigravity Quota HUD (Rust Native Tachometer Overlay)");
    println!("============================================================");

    // 1. ウィンドウイベントフックの導入 (ドラッグ/リサイズ開始・終了のリアルタイム監視)
    let _tracker = match WindowTracker::install() {
        Ok(t) => {
            println!(" [OK] WinEventHook (ドラッグ/リサイズ監視) をインストールしました。");
            t
        }
        Err(e) => {
            eprintln!(" [WARN] Hook のインストールに失敗: {}", e);
            return;
        }
    };

    // 2. タコメーターオーバーレイウィンドウの作成
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

    println!(" Antigravity ウィンドウの検出待機中...\n");

    let mut last_fetch = Instant::now() - Duration::from_secs(300);
    let mut last_target_hwnd = None;
    let mut was_dragging = false;

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
            last_target_hwnd = Some(target);

            if let Some(bounds) = get_target_bounds(target) {
                // ドラッグ中かどうかの判定
                let currently_dragging = is_dragging();

                if currently_dragging {
                    // ドラッグ/リサイズ中はフェードアウト (負荷削減 & チラつき防止)
                    overlay.fade_out();
                    was_dragging = true;
                } else {
                    // Antigravity の右上ヘッダー部分に吸着
                    let overlay_x = bounds.right - 280 - 150; // ウィンドウ右端から少し内側
                    let overlay_y = bounds.top + 38; // タイトルバー直下

                    overlay.set_position(overlay_x, overlay_y);

                    if was_dragging {
                        // ドラッグが落ち着いたのでフェードイン再表示
                        overlay.fade_in();
                        was_dragging = false;
                    } else {
                        overlay.fade_in();
                    }
                }
            } else {
                // 最小化中などはフェードアウト
                overlay.fade_out();
            }
        } else {
            // Antigravity が存在しない場合は隠す
            overlay.fade_out();
        }

        std::thread::sleep(Duration::from_millis(30)); // 30ms 周期の追従判定 (低負荷)
    }
}