mod overlay;
mod quota;
mod tracker;

use overlay::{GaugeData, TachometerOverlay};
use quota::fetch_quota;
use std::time::{Duration, Instant};
use tracker::{find_antigravity_window, get_target_bounds};
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetCursorPos, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
};

fn extract_group_quota(group: Option<&quota::GroupInfo>) -> overlay::GroupQuota {
    let mut weekly_pct = 100.0;
    let mut five_h_pct = 100.0;

    if let Some(g) = group {
        for b in &g.buckets {
            if let Some(id) = &b.bucket_id {
                if id.contains("weekly") {
                    weekly_pct = b.remaining_fraction * 100.0;
                } else if id.contains("5h") {
                    five_h_pct = b.remaining_fraction * 100.0;
                }
            }
        }
    }

    overlay::GroupQuota {
        weekly_pct,
        five_h_pct,
    }
}

fn extract_all_gauge_data() -> Option<GaugeData> {
    let quota = fetch_quota().ok()?;

    let gemini_group = quota
        .groups
        .iter()
        .find(|g| g.display_name.to_lowercase().contains("gemini"));

    let claude_gpt_group = quota.groups.iter().find(|g| {
        g.display_name.to_lowercase().contains("claude")
            || g.display_name.to_lowercase().contains("gpt")
    });

    Some(GaugeData {
        gemini: extract_group_quota(gemini_group),
        claude_gpt: extract_group_quota(claude_gpt_group),
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
            if let Some(data) = extract_all_gauge_data() {
                println!(
                    " [Quota Update] Gemini (W: {:.1}%, 5h: {:.1}%) | Claude/GPT (W: {:.1}%, 5h: {:.1}%)",
                    data.gemini.weekly_pct,
                    data.gemini.five_h_pct,
                    data.claude_gpt.weekly_pct,
                    data.claude_gpt.five_h_pct
                );
                overlay.update_data(data);
            }
            last_fetch = Instant::now();
        }

        // Antigravity ウィンドウの探索・追従
        if let Some(target) = find_antigravity_window() {
            if let Some(bounds) = get_target_bounds(target) {
                // Antigravity GUI の右下に配置 (右マージン 24px, 下マージン 32px)
                let overlay_x = bounds.right - overlay::HUD_WIDTH as i32 - 24;
                let overlay_y = bounds.bottom - overlay::HUD_HEIGHT as i32 - 32;

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

                // マウスカーソルが HUD の上または近傍（外側 +20px）にあるか検知
                let mut cursor_pt = POINT::default();
                let _ = unsafe { GetCursorPos(&mut cursor_pt) };

                let is_mouse_near = cursor_pt.x >= (overlay_x - 20)
                    && cursor_pt.x <= (overlay_x + overlay::HUD_WIDTH as i32 + 20)
                    && cursor_pt.y >= (overlay_y - 20)
                    && cursor_pt.y <= (overlay_y + overlay::HUD_HEIGHT as i32 + 20);

                if is_moving {
                    // 移動中: 最後の座標変化から 200ms 完全に静止したら定常状態とみなす (debounce)
                    if last_move_time.elapsed() >= Duration::from_millis(200) {
                        overlay.set_position(overlay_x, overlay_y);
                        is_moving = false;
                        if !is_mouse_near {
                            overlay.fade_in();
                        }
                    }
                } else if is_mouse_near {
                    // マウスが HUD の近くまたは上にある -> 背後UI操作の邪魔にならないよう完全に消す
                    overlay.fade_out();
                } else {
                    // 通常時 (静止中かつマウスが離れている) -> フェードイン表示
                    overlay.fade_in();
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
