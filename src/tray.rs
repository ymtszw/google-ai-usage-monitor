use crate::overlay::GroupQuota;
use crate::startup;
use std::env;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use tiny_skia::*;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT};
use windows::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject, HGDIOBJ};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, DestroyIcon, DestroyMenu, GetCursorPos,
    HICON, ICONINFO, MF_CHECKED, MF_DISABLED, MF_GRAYED, MF_SEPARATOR, MF_STRING, MF_UNCHECKED,
    PostQuitMessage, SetForegroundWindow, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TrackPopupMenu,
};
use windows::core::{BOOL, PCWSTR};

pub const WM_TRAYICON: u32 = windows::Win32::UI::WindowsAndMessaging::WM_USER + 100;

const IDM_HEADER: usize = 1001;
const IDM_TOGGLE_HUD: usize = 1002;
const IDM_STARTUP: usize = 1003;
const IDM_RESTART: usize = 1004;
const IDM_EXIT: usize = 1005;

static HUD_VISIBLE: AtomicBool = AtomicBool::new(true);

pub struct TrayIcon {
    hwnd: HWND,
    current_icon: Option<HICON>,
}

fn to_u16_slice_128(s: &str) -> [u16; 128] {
    let mut buf = [0u16; 128];
    for (i, c) in s.encode_utf16().take(127).enumerate() {
        buf[i] = c;
    }
    buf
}

/// 32x32 ピクセルに Gemini 5-hour burst の残量 % をタコメーター風/数値バッジで動的描画
fn generate_tray_hicon(pct: f64) -> Option<HICON> {
    let size = 32u32;
    let mut pixmap = Pixmap::new(size, size)?;
    pixmap.fill(Color::TRANSPARENT);

    let cx = 16.0;
    let cy = 16.0;
    let r = 13.5;

    // 1. 円形ダーク背景 (#151821, アルファ ~95%)
    if let Some(bg_path) = PathBuilder::from_circle(cx, cy, r) {
        let mut paint = Paint::default();
        paint.set_color_rgba8(21, 24, 33, 245);
        paint.anti_alias = true;
        pixmap.fill_path(
            &bg_path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );

        // 微細な境界線
        let mut border_paint = Paint::default();
        border_paint.set_color_rgba8(70, 80, 105, 180);
        border_paint.anti_alias = true;
        let stroke = Stroke {
            width: 1.0,
            ..Default::default()
        };
        pixmap.stroke_path(
            &bg_path,
            &border_paint,
            &stroke,
            Transform::identity(),
            None,
        );
    }

    // 2. 残量プログレスリング (外周円弧: 270度)
    let start_deg = 135.0f32;
    let sweep_deg = 270.0f32;
    let track_r = 10.5;

    // 背景トラック
    if let Some(track_path) = make_arc_path(cx, cy, track_r, start_deg, sweep_deg) {
        let mut paint = Paint::default();
        paint.set_color_rgba8(45, 52, 65, 200);
        paint.anti_alias = true;
        let stroke = Stroke {
            width: 2.8,
            line_cap: LineCap::Round,
            ..Default::default()
        };
        pixmap.stroke_path(&track_path, &paint, &stroke, Transform::identity(), None);
    }

    // アクティブトラック
    let active_sweep = (sweep_deg * (pct / 100.0) as f32).clamp(0.1, sweep_deg);
    if let Some(active_path) = make_arc_path(cx, cy, track_r, start_deg, active_sweep) {
        let (r_u8, g_u8, b_u8) = if pct > 50.0 {
            (52, 211, 153) // エメラルドグリーン
        } else if pct > 20.0 {
            (251, 191, 36) // アンバー
        } else {
            (248, 113, 113) // コーラルレッド
        };

        let mut paint = Paint::default();
        paint.set_color_rgba8(r_u8, g_u8, b_u8, 255);
        paint.anti_alias = true;
        let stroke = Stroke {
            width: 2.8,
            line_cap: LineCap::Round,
            ..Default::default()
        };
        pixmap.stroke_path(&active_path, &paint, &stroke, Transform::identity(), None);
    }

    // 3. 中央に「G」または数値をドットマトリクス/簡易グリフで描画
    // 中央の 10x10 領域にシンプルな残量インジケータ（中心ドット）
    let center_dot_r = 3.5;
    if let Some(dot_path) = PathBuilder::from_circle(cx, cy, center_dot_r) {
        let (r_u8, g_u8, b_u8) = if pct > 50.0 {
            (52, 211, 153)
        } else if pct > 20.0 {
            (251, 191, 36)
        } else {
            (248, 113, 113)
        };
        let mut paint = Paint::default();
        paint.set_color_rgba8(r_u8, g_u8, b_u8, 255);
        paint.anti_alias = true;
        pixmap.fill_path(
            &dot_path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }

    // 4. BGRA バッファ作成 & HICON 生成
    let mut bgra = Vec::with_capacity((size * size * 4) as usize);
    for chunk in pixmap.data().as_chunks::<4>().0 {
        let r = chunk[0];
        let g = chunk[1];
        let b = chunk[2];
        let a = chunk[3];
        bgra.push(b);
        bgra.push(g);
        bgra.push(r);
        bgra.push(a);
    }

    unsafe {
        let hbm_color = CreateBitmap(
            size as i32,
            size as i32,
            1,
            32,
            Some(bgra.as_ptr() as *const _),
        );
        let mask_bytes = vec![0u8; ((size * size) / 8) as usize];
        let hbm_mask = CreateBitmap(
            size as i32,
            size as i32,
            1,
            1,
            Some(mask_bytes.as_ptr() as *const _),
        );

        let icon_info = ICONINFO {
            fIcon: BOOL(1),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: hbm_mask,
            hbmColor: hbm_color,
        };

        let hicon = CreateIconIndirect(&icon_info).ok();
        let _ = DeleteObject(HGDIOBJ(hbm_color.0));
        let _ = DeleteObject(HGDIOBJ(hbm_mask.0));
        hicon
    }
}

fn make_arc_path(cx: f32, cy: f32, radius: f32, start_deg: f32, sweep_deg: f32) -> Option<Path> {
    let mut pb = PathBuilder::new();
    let steps = (sweep_deg.abs() / 2.0).ceil().max(2.0) as usize;

    for i in 0..=steps {
        let angle = (start_deg + (sweep_deg / steps as f32) * (i as f32)).to_radians();
        let x = cx + radius * angle.cos();
        let y = cy + radius * angle.sin();

        if i == 0 {
            pb.move_to(x, y);
        } else {
            pb.line_to(x, y);
        }
    }
    pb.finish()
}

impl TrayIcon {
    pub fn new(hwnd: HWND) -> Self {
        let mut tray = Self {
            hwnd,
            current_icon: None,
        };
        tray.init_tray();
        tray
    }

    fn init_tray(&mut self) {
        let hicon = generate_tray_hicon(100.0);
        self.current_icon = hicon;

        let mode = crate::runtime::RuntimeMode::detect();
        let mode_tag = if mode.is_dev() { " [DEV]" } else { "" };
        let tip_str = format!("Google AI Usage Monitor{}\nGemini: 100%", mode_tag);
        let tip = to_u16_slice_128(&tip_str);

        let nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: 1,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: WM_TRAYICON,
            hIcon: self.current_icon.unwrap_or_default(),
            szTip: tip,
            ..Default::default()
        };

        unsafe {
            let _ = Shell_NotifyIconW(NIM_ADD, &nid);
        }
    }

    pub fn update(&mut self, gemini: &GroupQuota, argon: &GroupQuota, claude: &GroupQuota) {
        let hicon = generate_tray_hicon(gemini.five_h_pct);
        if let Some(old) = self.current_icon.replace(hicon.unwrap_or_default()) {
            unsafe {
                let _ = DestroyIcon(old);
            }
        }

        let mode = crate::runtime::RuntimeMode::detect();
        let mode_tag = if mode.is_dev() { " [DEV]" } else { "" };
        let tip_str = format!(
            "Google AI Usage Monitor{}\nGemini 5h: {:.0}% | W: {:.0}% | Argon: {:.0}%\nClaude 5h: {:.0}% | W: {:.0}%",
            mode_tag,
            gemini.five_h_pct,
            gemini.weekly_pct,
            argon.weekly_pct,
            claude.five_h_pct,
            claude.weekly_pct
        );
        let tip = to_u16_slice_128(&tip_str);

        let nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: 1,
            uFlags: NIF_ICON | NIF_TIP,
            hIcon: self.current_icon.unwrap_or_default(),
            szTip: tip,
            ..Default::default()
        };

        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }

    pub fn is_hud_visible() -> bool {
        HUD_VISIBLE.load(Ordering::SeqCst)
    }

    pub fn toggle_hud() {
        HUD_VISIBLE.fetch_xor(true, Ordering::SeqCst);
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        let nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: 1,
            ..Default::default()
        };
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
            if let Some(icon) = self.current_icon.take() {
                let _ = DestroyIcon(icon);
            }
        }
    }
}

fn to_wide_str(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// トレイアイコンのメッセージ処理（右クリックメニュー・クリック等）
pub unsafe fn handle_tray_message(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    let msg = (lparam.0 & 0xffff) as u32;

    match msg {
        windows::Win32::UI::WindowsAndMessaging::WM_RBUTTONUP
        | windows::Win32::UI::WindowsAndMessaging::WM_CONTEXTMENU => {
            crate::request_refresh();
            unsafe {
                show_tray_menu(hwnd);
            }
            LRESULT(0)
        }
        windows::Win32::UI::WindowsAndMessaging::WM_LBUTTONUP => {
            crate::request_refresh();
            TrayIcon::toggle_hud();
            LRESULT(0)
        }
        _ => LRESULT(0),
    }
}

unsafe fn show_tray_menu(hwnd: HWND) {
    let mut pt = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut pt);

        let hmenu = match CreatePopupMenu() {
            Ok(m) => m,
            Err(_) => return,
        };

        let _ = SetForegroundWindow(hwnd);

        // 1. タイトル/情報ヘッダー
        let mode = crate::runtime::RuntimeMode::detect();
        let header_text = if mode.is_dev() {
            format!("Google AI Usage Monitor ({})", mode.label())
        } else {
            "Google AI Usage Monitor (Release)".to_string()
        };
        let header_str = to_wide_str(&header_text);
        let _ = AppendMenuW(
            hmenu,
            MF_STRING | MF_DISABLED | MF_GRAYED,
            IDM_HEADER,
            PCWSTR(header_str.as_ptr()),
        );
        let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null());

        // 2. HUD 表示 / 非表示
        let is_visible = TrayIcon::is_hud_visible();
        let toggle_label = if is_visible {
            to_wide_str("HUD を非表示にする")
        } else {
            to_wide_str("HUD を表示する")
        };
        let _ = AppendMenuW(
            hmenu,
            MF_STRING,
            IDM_TOGGLE_HUD,
            PCWSTR(toggle_label.as_ptr()),
        );

        // 3. スタートアップ登録 (チェックマーク付き)
        let is_autostart = startup::is_startup_enabled();
        let autostart_flag = if is_autostart {
            MF_CHECKED
        } else {
            MF_UNCHECKED
        };
        let autostart_str = to_wide_str("Windows 起動時に自動起動");
        let _ = AppendMenuW(
            hmenu,
            MF_STRING | autostart_flag,
            IDM_STARTUP,
            PCWSTR(autostart_str.as_ptr()),
        );

        let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null());

        // 4. 再起動
        let restart_str = to_wide_str("再起動 (Restart)");
        let _ = AppendMenuW(hmenu, MF_STRING, IDM_RESTART, PCWSTR(restart_str.as_ptr()));

        // 5. 終了
        let exit_str = to_wide_str("終了 (Exit)");
        let _ = AppendMenuW(hmenu, MF_STRING, IDM_EXIT, PCWSTR(exit_str.as_ptr()));

        let cmd = TrackPopupMenu(
            hmenu,
            TPM_BOTTOMALIGN
                | TPM_LEFTALIGN
                | windows::Win32::UI::WindowsAndMessaging::TPM_RETURNCMD,
            pt.x,
            pt.y,
            Some(0),
            hwnd,
            None,
        );

        let _ = DestroyMenu(hmenu);

        match cmd.0 as usize {
            IDM_TOGGLE_HUD => {
                TrayIcon::toggle_hud();
            }
            IDM_STARTUP => {
                let current = startup::is_startup_enabled();
                let _ = startup::set_startup_enabled(!current);
            }
            IDM_RESTART => {
                restart_application();
            }
            IDM_EXIT => {
                PostQuitMessage(0);
            }
            _ => {}
        }
    }
}

fn restart_application() {
    if let Ok(exe_path) = env::current_exe() {
        let _ = Command::new(exe_path).spawn();
    }
    std::process::exit(0);
}
