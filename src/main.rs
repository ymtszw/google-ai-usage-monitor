#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod overlay;
mod quota;
mod runtime;
mod startup;
mod tracker;
mod tray;

use overlay::{GaugeData, TachometerOverlay};
use quota::fetch_quota;
use std::time::{Duration, Instant};
use tracker::{find_antigravity_window, get_target_bounds};
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetCursorPos, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
};

fn format_reset_time(iso_str: Option<&str>) -> Option<String> {
    let s = iso_str?;
    let utc_dt = chrono::DateTime::parse_from_rfc3339(s).ok()?;
    let now = chrono::Utc::now();
    let duration = utc_dt.signed_duration_since(now);

    if duration.num_seconds() <= 0 {
        return Some("Reset: Ready".to_string());
    }

    let total_mins = duration.num_minutes();
    let hours = total_mins / 60;
    let mins = total_mins % 60;

    if hours < 24 {
        Some(format!("Reset in: {:02}:{:02}", hours, mins))
    } else {
        let days = hours / 24;
        let rem_hours = hours % 24;
        Some(format!("Reset in: {}d {}h", days, rem_hours))
    }
}

fn extract_group_quota(group: Option<&quota::GroupInfo>) -> overlay::GroupQuota {
    let mut weekly_pct = 100.0;
    let mut five_h_pct = 100.0;
    let mut weekly_reset = None;
    let mut five_h_reset = None;

    if let Some(g) = group {
        for b in &g.buckets {
            if let Some(id) = &b.bucket_id {
                if id.contains("weekly") {
                    weekly_pct = b.remaining_fraction * 100.0;
                    weekly_reset = format_reset_time(b.reset_time.as_deref());
                } else if id.contains("5h") {
                    five_h_pct = b.remaining_fraction * 100.0;
                    five_h_reset = format_reset_time(b.reset_time.as_deref());
                }
            }
        }
    }

    overlay::GroupQuota {
        weekly_pct,
        five_h_pct,
        weekly_reset,
        five_h_reset,
    }
}

fn extract_all_gauge_data() -> Option<GaugeData> {
    let quota = fetch_quota().ok()?;

    let gemini_group = quota.groups.iter().find(|g| {
        let name = g.display_name.to_lowercase();
        name.contains("gemini") && !name.contains("argon")
    });

    let argon_group = quota
        .groups
        .iter()
        .find(|g| g.display_name.to_lowercase().contains("argon"));

    let claude_gpt_group = quota.groups.iter().find(|g| {
        let name = g.display_name.to_lowercase();
        name.contains("claude") || name.contains("gpt")
    });

    Some(GaugeData {
        gemini: extract_group_quota(gemini_group),
        argon: extract_group_quota(argon_group),
        claude_gpt: extract_group_quota(claude_gpt_group),
    })
}

static REFRESH_REQUESTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// トレイクリックや状態変化時にクォータ更新を即時要求
pub fn request_refresh() {
    REFRESH_REQUESTED.store(true, std::sync::atomic::Ordering::SeqCst);
}

fn main() {
    // CLI 引数のチェック (スタートアップ登録・解除)
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        // コマンドライン実行時は親コンソール (cmd / PowerShell) に標準出力をアタッチ
        unsafe {
            let _ = windows::Win32::System::Console::AttachConsole(
                windows::Win32::System::Console::ATTACH_PARENT_PROCESS,
            );
        }

        match args[1].as_str() {
            "--register-startup" => {
                match startup::set_startup_enabled(true) {
                    Ok(()) => println!(" [OK] Windows 起動時の自動起動を登録しました。"),
                    Err(e) => eprintln!(" [ERROR] 登録失敗: {}", e),
                }
                return;
            }
            "--unregister-startup" => {
                match startup::set_startup_enabled(false) {
                    Ok(()) => println!(" [OK] Windows 起動時の自動起動を解除しました。"),
                    Err(e) => eprintln!(" [ERROR] 解除失敗: {}", e),
                }
                return;
            }
            "--status-startup" => {
                println!(
                    " スタートアップ登録状態: {}",
                    if startup::is_startup_enabled() {
                        "有効 (Enabled)"
                    } else {
                        "無効 (Disabled)"
                    }
                );
                return;
            }
            "--help" | "-h" => {
                println!("使用法: google-ai-usage-monitor [オプション]");
                println!("  --register-startup    Windows 起動時の自動起動を有効化");
                println!("  --unregister-startup  Windows 起動時の自動起動を無効化");
                println!("  --status-startup      現在の自動起動設定状態を表示");
                return;
            }
            _ => {}
        }
    }

    let runtime_mode = runtime::RuntimeMode::detect();
    println!("============================================================");
    println!(" Antigravity Quota HUD (Rust Native Tachometer Overlay)");
    println!(" Mode: {}", runtime_mode.label());
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

    // 2. システムトレイアイコンの登録
    let mut tray = tray::TrayIcon::new(overlay.hwnd);
    println!(" [OK] システムトレイアイコンを登録しました。");

    println!(" Antigravity ウィンドウを検索・追従中...\n");

    let mut last_fetch = Instant::now() - Duration::from_secs(300);
    let mut last_bounds = None;
    let mut last_move_time = Instant::now() - Duration::from_secs(10);
    let mut is_moving = false;
    let mut was_hud_shown = false;

    // メッセージループ & 追従ループ
    loop {
        // Windows メッセージのディスパッチ
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == windows::Win32::UI::WindowsAndMessaging::WM_QUIT {
                    return;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        // クォータデータの最新化:
        // 1. 定期5分経過
        // 2. トレイクリックまたは非表示->表示遷移による明示的更新要求 (最短3秒デバウンス)
        let is_periodic = last_fetch.elapsed() >= Duration::from_secs(300);
        let is_requested = REFRESH_REQUESTED.load(std::sync::atomic::Ordering::SeqCst)
            && last_fetch.elapsed() >= Duration::from_secs(3);

        if is_periodic || is_requested {
            REFRESH_REQUESTED.store(false, std::sync::atomic::Ordering::SeqCst);
            if let Some(data) = extract_all_gauge_data() {
                println!(
                    " [Quota Update] Gemini (W: {:.1}%, 5h: {:.1}%, Argon W: {:.1}%) | Claude/GPT (W: {:.1}%, 5h: {:.1}%)",
                    data.gemini.weekly_pct,
                    data.gemini.five_h_pct,
                    data.argon.weekly_pct,
                    data.claude_gpt.weekly_pct,
                    data.claude_gpt.five_h_pct
                );
                overlay.update_data(data.clone());
                tray.update(&data.gemini, &data.argon, &data.claude_gpt);
            }
            last_fetch = Instant::now();
        }

        let mut is_hud_shown = false;
        let hud_user_visible = tray::TrayIcon::is_hud_visible();

        // Antigravity ウィンドウの探索・追従
        if hud_user_visible {
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
                        // 初回検知時: 新位置へ配置し Z-order を同期してフェードイン
                        overlay.update_position(overlay_x, overlay_y, target);
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
                            overlay.update_position(overlay_x, overlay_y, target);
                            is_moving = false;
                            if !is_mouse_near {
                                overlay.fade_in();
                                is_hud_shown = true;
                            }
                        }
                    } else if is_mouse_near {
                        // マウスが HUD の近くまたは上にある -> 背後UI操作の邪魔にならないよう完全に消す
                        overlay.fade_out();
                    } else {
                        // 通常時 (静止中かつマウスが離れている) -> Z-order を Antigravity 直上に同期しつつフェードイン表示
                        overlay.update_position(overlay_x, overlay_y, target);
                        overlay.fade_in();
                        is_hud_shown = true;
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
        } else {
            // トレイメニューで「HUD を非表示」に設定中
            overlay.fade_out();
        }

        // 非表示状態から表示状態に遷移した瞬間にクォータ最新化をリクエスト
        if !was_hud_shown && is_hud_shown {
            request_refresh();
        }
        was_hud_shown = is_hud_shown;

        std::thread::sleep(Duration::from_millis(16)); // 60fps (16ms) 追従
    }
}
