use crate::overlay::{
    DesignTokens, GaugeData, draw_text, ensure_font_system, get_quota_color, make_arc_path,
    push_pixmap_to_layered_window,
};
use crate::tray::WM_TRAYICON;
use cosmic_text::Weight;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tiny_skia::*;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, TreeScope_Descendants,
    UIA_AutomationIdPropertyId,
};
use windows::Win32::UI::Controls::{
    ICC_WIN95_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx, TOOLTIPS_CLASSW, TTF_IDISHWND,
    TTF_SUBCLASS, TTM_ADDTOOLW, TTM_SETMAXTIPWIDTH, TTM_UPDATETIPTEXTW, TTS_ALWAYSTIP,
    TTS_NOPREFIX, TTTOOLINFOW,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};

const WM_MOUSELEAVE: u32 = 0x02A3;
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, FindWindowExW, FindWindowW, GW_HWNDPREV,
    GWL_EXSTYLE, GetClientRect, GetWindow, GetWindowLongW, GetWindowRect, HCURSOR, HICON, HMENU,
    HWND_TOPMOST, IDC_ARROW, IsWindowVisible, LoadCursorW, MA_NOACTIVATE, MoveWindow, PostMessageW,
    RegisterClassExW, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_SHOWWINDOW, SendMessageW, SetWindowPos, ShowWindow, WINDOW_STYLE, WM_LBUTTONUP,
    WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_RBUTTONUP, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_LAYOUTRTL,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{BSTR, w};

static TASKBAR_HOVER: AtomicBool = AtomicBool::new(false);
static TASKBAR_NOTIFY_HWND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
static TASKBAR_NEEDS_REPAINT: AtomicBool = AtomicBool::new(false);

fn is_light_taskbar_theme() -> bool {
    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            Some(0),
            KEY_READ,
            &mut hkey,
        ) != windows::Win32::Foundation::ERROR_SUCCESS
        {
            return false;
        }
        let mut val = 0u32;
        let mut len = std::mem::size_of::<u32>() as u32;
        let res = RegQueryValueExW(
            hkey,
            w!("SystemUsesLightTheme"),
            None,
            None,
            Some(&mut val as *mut u32 as *mut u8),
            Some(&mut len),
        );
        let _ = RegCloseKey(hkey);
        res == windows::Win32::Foundation::ERROR_SUCCESS && val != 0
    }
}

/// タスクバー用の無次元トークンを DPI スケールに合わせて構築
fn taskbar_tokens(dpi_scale: f32) -> DesignTokens {
    let base = DesignTokens::HUD_COMPACT;
    DesignTokens {
        base_space: base.base_space * dpi_scale,
        base_text: base.base_text * dpi_scale,
    }
}

/// タスクバー帯の幅を DesignTokens から算出
pub fn compute_taskbar_strip_width(tokens: &DesignTokens, is_dev: bool) -> u32 {
    let pad_x = tokens.space_lg(); // 12px@100%
    let two_val_group_w = tokens.base_space * 11.5; // 92px@100%
    let one_val_group_w = tokens.base_space * 6.75; // 54px@100%
    let sep_gap = tokens.space_xl(); // 16px@100%
    let dev_w = if is_dev {
        tokens.base_space * 4.5 // 36px@100%
    } else {
        0.0
    };

    let total = pad_x * 2.0
        + two_val_group_w
        + sep_gap
        + one_val_group_w
        + sep_gap
        + two_val_group_w
        + dev_w;
    total.ceil() as u32
}

fn round_rect_path(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<Path> {
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.quad_to(x + w, y, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.quad_to(x + w, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.quad_to(x, y + h, x, y + h - r);
    pb.line_to(x, y + r);
    pb.quad_to(x, y, x + r, y);
    pb.close();
    pb.finish()
}

fn fill_round_rect(pixmap: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, color: Color) {
    if let Some(path) = round_rect_path(x, y, w, h, r) {
        let mut paint = Paint::default();
        paint.set_color(color);
        paint.anti_alias = true;
        pixmap.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn stroke_round_rect(
    pixmap: &mut Pixmap,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    r: f32,
    width: f32,
    color: Color,
) {
    if let Some(path) = round_rect_path(x, y, w, h, r) {
        let mut paint = Paint::default();
        paint.set_color(color);
        paint.anti_alias = true;
        let stroke = Stroke {
            width,
            ..Default::default()
        };
        pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_mini_group(
    pixmap: &mut Pixmap,
    tokens: &DesignTokens,
    x: f32,
    cy: f32,
    ring_pct: f64,
    label: &str,
    value_text: &str,
    light_theme: bool,
) {
    let ring_r = tokens.base_space * 1.15; // ~9.2px@100%
    let ring_cx = x + ring_r;
    let stroke_w = tokens.space_xs() * 1.1; // ~2.2px@100%

    // 背景リング
    if let Some(track) = make_arc_path(ring_cx, cy, ring_r, 135.0, 270.0) {
        let mut paint = Paint::default();
        if light_theme {
            paint.set_color_rgba8(0, 0, 0, 38);
        } else {
            paint.set_color_rgba8(60, 68, 85, 190);
        }
        paint.anti_alias = true;
        let stroke = Stroke {
            width: stroke_w,
            line_cap: LineCap::Round,
            ..Default::default()
        };
        pixmap.stroke_path(&track, &paint, &stroke, Transform::identity(), None);
    }

    // アクティブリング
    let active_color = get_quota_color(ring_pct);
    let sweep = (270.0 * (ring_pct.clamp(0.0, 100.0) as f32 / 100.0)).max(2.0);
    if let Some(arc) = make_arc_path(ring_cx, cy, ring_r, 135.0, sweep) {
        let mut paint = Paint::default();
        paint.set_color(active_color);
        paint.anti_alias = true;
        let stroke = Stroke {
            width: stroke_w,
            line_cap: LineCap::Round,
            ..Default::default()
        };
        pixmap.stroke_path(&arc, &paint, &stroke, Transform::identity(), None);
    }

    // テキスト列 (ラベル上段 + 数値下段)
    let text_x = ring_cx + ring_r + tokens.space_sm();
    let label_color = if light_theme {
        Color::from_rgba8(75, 85, 99, 255)
    } else {
        Color::from_rgba8(165, 178, 198, 255)
    };
    let label_y = cy - tokens.font_caption() - (tokens.space_xs() * 0.75);
    let value_y = cy - (tokens.space_xs() * 0.25);

    draw_text(
        pixmap,
        label,
        text_x,
        label_y,
        tokens.font_caption(),
        label_color,
        false,
        Weight::SEMIBOLD,
    );

    let value_color = if light_theme {
        // ライトテーマでは白背景上でも視認性を保つため濃いめの色を使用
        if ring_pct > 50.0 {
            Color::from_rgba8(5, 150, 105, 255)
        } else if ring_pct > 20.0 {
            Color::from_rgba8(217, 119, 6, 255)
        } else {
            Color::from_rgba8(220, 38, 38, 255)
        }
    } else {
        active_color
    };

    draw_text(
        pixmap,
        value_text,
        text_x,
        value_y,
        tokens.font_label(),
        value_color,
        false,
        Weight::BOLD,
    );
}

/// タスクバー帯の描画 (オフスクリーンテストでも共通利用)
pub fn render_taskbar_strip(
    pixmap: &mut Pixmap,
    data: &GaugeData,
    dpi_scale: f32,
    hovered: bool,
    light_theme: bool,
    is_dev: bool,
) {
    ensure_font_system();
    let tokens = taskbar_tokens(dpi_scale);
    let w = pixmap.width() as f32;
    let h = pixmap.height() as f32;

    // ほぼ透明なベース (ヒットテスト用 alpha=1)
    for px in pixmap.data_mut().as_chunks_mut::<4>().0 {
        px[0] = 0;
        px[1] = 0;
        px[2] = 0;
        px[3] = 1;
    }

    // 角丸カプセル背景
    let inset_y = tokens.space_sm(); // 4px@100%
    let inset_x = tokens.space_xs(); // 2px@100%
    let cap_w = (w - inset_x * 2.0).max(1.0);
    let cap_h = (h - inset_y * 2.0).max(1.0);
    let radius = tokens.space_md(); // 8px@100%

    let bg_color = if light_theme {
        if hovered {
            Color::from_rgba8(255, 255, 255, 240)
        } else {
            Color::from_rgba8(243, 244, 248, 215)
        }
    } else if hovered {
        Color::from_rgba8(32, 37, 50, 235)
    } else {
        Color::from_rgba8(21, 24, 33, 205)
    };
    fill_round_rect(pixmap, inset_x, inset_y, cap_w, cap_h, radius, bg_color);

    let border_color = if light_theme {
        Color::from_rgba8(180, 190, 205, 160)
    } else if hovered {
        Color::from_rgba8(110, 125, 155, 180)
    } else {
        Color::from_rgba8(70, 80, 105, 130)
    };
    stroke_round_rect(
        pixmap,
        inset_x,
        inset_y,
        cap_w,
        cap_h,
        radius,
        dpi_scale.max(1.0),
        border_color,
    );

    let cy = h / 2.0;
    let pad_x = tokens.space_lg();
    let sep_gap = tokens.space_xl();
    let two_val_w = tokens.base_space * 11.5;
    let one_val_w = tokens.base_space * 6.75;

    let mut cursor_x = pad_x;

    // 1. Gemini (5h / Weekly)
    let gem_min = data.gemini.five_h_pct.min(data.gemini.weekly_pct);
    let gem_val = format!(
        "{:.0}% / {:.0}%",
        data.gemini.five_h_pct, data.gemini.weekly_pct
    );
    draw_mini_group(
        pixmap,
        &tokens,
        cursor_x,
        cy,
        gem_min,
        "Gemini",
        &gem_val,
        light_theme,
    );
    cursor_x += two_val_w;

    // 区切り線 1
    let div_x1 = cursor_x + sep_gap * 0.5;
    draw_vertical_divider(
        pixmap,
        div_x1,
        inset_y + tokens.space_md(),
        h - inset_y - tokens.space_md(),
        light_theme,
    );
    cursor_x += sep_gap;

    // 2. Argon (Weekly)
    let arg_val = format!("{:.0}%", data.argon.weekly_pct);
    draw_mini_group(
        pixmap,
        &tokens,
        cursor_x,
        cy,
        data.argon.weekly_pct,
        "Argon",
        &arg_val,
        light_theme,
    );
    cursor_x += one_val_w;

    // 区切り線 2
    let div_x2 = cursor_x + sep_gap * 0.5;
    draw_vertical_divider(
        pixmap,
        div_x2,
        inset_y + tokens.space_md(),
        h - inset_y - tokens.space_md(),
        light_theme,
    );
    cursor_x += sep_gap;

    // 3. Claude / GPT (5h / Weekly)
    let cld_min = data.claude_gpt.five_h_pct.min(data.claude_gpt.weekly_pct);
    let cld_val = format!(
        "{:.0}% / {:.0}%",
        data.claude_gpt.five_h_pct, data.claude_gpt.weekly_pct
    );
    draw_mini_group(
        pixmap,
        &tokens,
        cursor_x,
        cy,
        cld_min,
        "Claude",
        &cld_val,
        light_theme,
    );
    cursor_x += two_val_w;

    // DEV バッジ
    if is_dev {
        let badge_w = tokens.base_space * 3.5;
        let badge_h = tokens.base_space * 1.75;
        let badge_x = cursor_x;
        let badge_y = cy - badge_h / 2.0;
        fill_round_rect(
            pixmap,
            badge_x,
            badge_y,
            badge_w,
            badge_h,
            tokens.space_xs(),
            Color::from_rgba8(245, 158, 11, 45),
        );
        stroke_round_rect(
            pixmap,
            badge_x,
            badge_y,
            badge_w,
            badge_h,
            tokens.space_xs(),
            1.0,
            Color::from_rgba8(245, 158, 11, 200),
        );
        draw_text(
            pixmap,
            "DEV",
            badge_x + badge_w / 2.0,
            badge_y + tokens.space_xs() * 0.5,
            tokens.font_caption() * 0.9,
            Color::from_rgba8(251, 191, 36, 255),
            true,
            Weight::BOLD,
        );
    }
}

fn draw_vertical_divider(pixmap: &mut Pixmap, x: f32, y1: f32, y2: f32, light_theme: bool) {
    let mut pb = PathBuilder::new();
    pb.move_to(x, y1);
    pb.line_to(x, y2);
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        if light_theme {
            paint.set_color_rgba8(0, 0, 0, 35);
        } else {
            paint.set_color_rgba8(70, 80, 105, 110);
        }
        let stroke = Stroke {
            width: 1.0,
            ..Default::default()
        };
        pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
}

unsafe extern "system" fn taskbar_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_MOUSEMOVE => {
            if !TASKBAR_HOVER.swap(true, Ordering::SeqCst) {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                unsafe {
                    let _ = TrackMouseEvent(&mut tme);
                }
                TASKBAR_NEEDS_REPAINT.store(true, Ordering::SeqCst);
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            TASKBAR_HOVER.store(false, Ordering::SeqCst);
            TASKBAR_NEEDS_REPAINT.store(true, Ordering::SeqCst);
            LRESULT(0)
        }
        WM_LBUTTONUP | WM_RBUTTONUP => {
            let notify_raw = TASKBAR_NOTIFY_HWND.load(Ordering::SeqCst);
            if notify_raw != 0 {
                let notify_hwnd = HWND(notify_raw as *mut std::ffi::c_void);
                unsafe {
                    let _ = PostMessageW(
                        Some(notify_hwnd),
                        WM_TRAYICON,
                        WPARAM(0),
                        LPARAM(WM_RBUTTONUP as isize),
                    );
                }
            }
            LRESULT(0)
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

pub struct TaskbarWidget {
    hwnd: Option<HWND>,
    taskbar_hwnd: Option<HWND>,
    rebar_hwnd: Option<HWND>,
    tip_hwnd: Option<HWND>,
    notify_hwnd: HWND,
    data: GaugeData,
    visible: bool,
    last_tray: RECT,
    last_bar: RECT,
    last_widgets: RECT,
    last_w: i32,
    last_dpi: u32,
    last_left: i32,
    width: i32,
    height: i32,
    last_tip_text: String,
    uia: Option<IUIAutomation>,
    widgets_btn: Option<IUIAutomationElement>,
    last_uia_check: Option<Instant>,
    cached_widgets_rect: RECT,
}

static CLASS_REGISTERED: Mutex<bool> = Mutex::new(false);

impl TaskbarWidget {
    pub fn new(notify_hwnd: HWND) -> Self {
        TASKBAR_NOTIFY_HWND.store(notify_hwnd.0 as isize, Ordering::SeqCst);
        Self {
            hwnd: None,
            taskbar_hwnd: None,
            rebar_hwnd: None,
            tip_hwnd: None,
            notify_hwnd,
            data: GaugeData::default(),
            visible: false,
            last_tray: RECT::default(),
            last_bar: RECT::default(),
            last_widgets: RECT::default(),
            last_w: -1,
            last_dpi: 0,
            last_left: -1,
            width: 0,
            height: 0,
            last_tip_text: String::new(),
            uia: None,
            widgets_btn: None,
            last_uia_check: None,
            cached_widgets_rect: RECT::default(),
        }
    }

    pub fn is_alive(&self) -> bool {
        unsafe {
            match (self.hwnd, self.taskbar_hwnd) {
                (Some(w), Some(tb)) => {
                    windows::Win32::UI::WindowsAndMessaging::IsWindow(Some(w)).as_bool()
                        && windows::Win32::UI::WindowsAndMessaging::IsWindow(Some(tb)).as_bool()
                }
                _ => false,
            }
        }
    }

    pub fn create(&mut self) -> bool {
        self.destroy();
        unsafe {
            let Ok(tb) = FindWindowW(w!("Shell_TrayWnd"), None) else {
                return false;
            };
            if tb.0.is_null() {
                return false;
            }
            let Ok(tray) = FindWindowExW(Some(tb), None, w!("TrayNotifyWnd"), None) else {
                return false;
            };
            if tray.0.is_null() {
                return false;
            }

            let mut tb_rect = RECT::default();
            if GetWindowRect(tb, &mut tb_rect).is_err() {
                return false;
            }
            // 縦向きタスクバーは対象外
            if tb_rect.bottom - tb_rect.top > tb_rect.right - tb_rect.left {
                return false;
            }

            // Win11 XAML タスクバーか Win10 ReBarWindow32 かを判定
            let is_win11_xaml = FindWindowExW(
                Some(tb),
                None,
                w!("Windows.UI.Composition.DesktopWindowContentBridge"),
                None,
            )
            .map(|h| !h.0.is_null())
            .unwrap_or(false);

            self.rebar_hwnd = if is_win11_xaml {
                None
            } else {
                FindWindowExW(Some(tb), None, w!("ReBarWindow32"), None)
                    .ok()
                    .filter(|h| !h.0.is_null())
            };

            let Ok(hinst) = GetModuleHandleW(None) else {
                return false;
            };

            {
                let mut reg = CLASS_REGISTERED.lock().unwrap();
                if !*reg {
                    let wc = WNDCLASSEXW {
                        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                        lpfnWndProc: Some(taskbar_wnd_proc),
                        hInstance: hinst.into(),
                        hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or(HCURSOR::default()),
                        lpszClassName: w!("AntigravityQuotaTaskbarWidget"),
                        hIcon: HICON::default(),
                        ..Default::default()
                    };
                    if RegisterClassExW(&wc) != 0 {
                        *reg = true;
                    }
                }
            }

            let Ok(wnd) = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_NOACTIVATE,
                w!("AntigravityQuotaTaskbarWidget"),
                w!(""),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(hinst.into()),
                None,
            ) else {
                return false;
            };

            self.hwnd = Some(wnd);
            self.taskbar_hwnd = Some(tb);
            self.visible = true;

            // ツールチップ初期化
            let icc = INITCOMMONCONTROLSEX {
                dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_WIN95_CLASSES,
            };
            let _ = InitCommonControlsEx(&icc);

            if let Ok(tip) = CreateWindowExW(
                WS_EX_TOPMOST,
                TOOLTIPS_CLASSW,
                None,
                WS_POPUP | WINDOW_STYLE(TTS_ALWAYSTIP | TTS_NOPREFIX),
                0,
                0,
                0,
                0,
                Some(self.notify_hwnd),
                Some(HMENU::default()),
                Some(hinst.into()),
                None,
            ) {
                let mut title: Vec<u16> =
                    "Antigravity Quota".encode_utf16().chain(Some(0)).collect();
                let ti = TTTOOLINFOW {
                    cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
                    uFlags: TTF_IDISHWND | TTF_SUBCLASS,
                    hwnd: self.notify_hwnd,
                    uId: wnd.0 as usize,
                    lpszText: windows::core::PWSTR(title.as_mut_ptr()),
                    ..Default::default()
                };
                SendMessageW(
                    tip,
                    TTM_ADDTOOLW,
                    Some(WPARAM(0)),
                    Some(LPARAM(&ti as *const _ as isize)),
                );
                SendMessageW(tip, TTM_SETMAXTIPWIDTH, Some(WPARAM(0)), Some(LPARAM(600)));
                self.tip_hwnd = Some(tip);
            }

            self.last_dpi = self.current_dpi();
            self.paint();
            self.last_w = -1;
            self.reposition();
            let _ = ShowWindow(wnd, SW_SHOWNOACTIVATE);
            true
        }
    }

    pub fn destroy(&mut self) {
        unsafe {
            if let (Some(rebar), Some(tb)) = (self.rebar_hwnd, self.taskbar_hwnd)
                && windows::Win32::UI::WindowsAndMessaging::IsWindow(Some(rebar)).as_bool()
                && windows::Win32::UI::WindowsAndMessaging::IsWindow(Some(tb)).as_bool()
            {
                let mut r = RECT::default();
                let _ = GetClientRect(tb, &mut r);
                let _ = PostMessageW(
                    Some(tb),
                    windows::Win32::UI::WindowsAndMessaging::WM_SIZE,
                    WPARAM(0),
                    LPARAM((((r.bottom & 0xffff) << 16) | (r.right & 0xffff)) as isize),
                );
            }
            self.rebar_hwnd = None;
            self.widgets_btn = None;
            if let Some(tip) = self.tip_hwnd.take() {
                let _ = DestroyWindow(tip);
            }
            if let Some(wnd) = self.hwnd.take() {
                let _ = DestroyWindow(wnd);
            }
            self.taskbar_hwnd = None;
            self.visible = false;
            TASKBAR_HOVER.store(false, Ordering::SeqCst);
        }
    }

    pub fn set_visible(&mut self, show: bool) {
        if self.visible == show && (!show || self.is_alive()) {
            return;
        }
        if show {
            if !self.is_alive() {
                self.create();
            } else if let Some(wnd) = self.hwnd {
                unsafe {
                    let _ = ShowWindow(wnd, SW_SHOWNOACTIVATE);
                }
                self.visible = true;
                self.reposition();
            }
        } else if let Some(wnd) = self.hwnd {
            unsafe {
                let _ = ShowWindow(wnd, SW_HIDE);
            }
            self.visible = false;
        }
    }

    pub fn update_data(&mut self, data: GaugeData) {
        self.data = data;
        self.update_tooltip();
        if self.is_alive() && self.visible {
            self.paint();
            self.reposition();
        }
    }

    pub fn tick(&mut self) {
        if !self.visible {
            return;
        }
        if !self.is_alive() {
            self.create();
            return;
        }
        if TASKBAR_NEEDS_REPAINT.swap(false, Ordering::SeqCst) {
            self.paint();
        }
        self.reposition();
    }

    fn current_dpi(&self) -> u32 {
        unsafe {
            self.taskbar_hwnd
                .map(|tb| GetDpiForWindow(tb))
                .filter(|&d| d > 0)
                .unwrap_or(96)
        }
    }

    fn update_tooltip(&mut self) {
        let Some(tip) = self.tip_hwnd else {
            return;
        };
        let Some(wnd) = self.hwnd else {
            return;
        };
        let gem_r = self.data.gemini.five_h_reset.as_deref().unwrap_or("Ready");
        let cld_r = self
            .data
            .claude_gpt
            .five_h_reset
            .as_deref()
            .unwrap_or("Ready");
        let text = format!(
            "Gemini: 5h {:.0}% / Weekly {:.0}% ({})\nArgon: Weekly {:.0}%\nClaude/GPT: 5h {:.0}% / Weekly {:.0}% ({})\nClick: Refresh | Right-Click: Menu",
            self.data.gemini.five_h_pct,
            self.data.gemini.weekly_pct,
            gem_r,
            self.data.argon.weekly_pct,
            self.data.claude_gpt.five_h_pct,
            self.data.claude_gpt.weekly_pct,
            cld_r
        );
        if text == self.last_tip_text {
            return;
        }
        self.last_tip_text = text.clone();
        let mut wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        let ti = TTTOOLINFOW {
            cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
            hwnd: self.notify_hwnd,
            uId: wnd.0 as usize,
            lpszText: windows::core::PWSTR(wide.as_mut_ptr()),
            ..Default::default()
        };
        unsafe {
            SendMessageW(
                tip,
                TTM_UPDATETIPTEXTW,
                Some(WPARAM(0)),
                Some(LPARAM(&ti as *const _ as isize)),
            );
        }
    }

    fn paint(&mut self) {
        let (Some(wnd), Some(tb)) = (self.hwnd, self.taskbar_hwnd) else {
            return;
        };
        let mut bar = RECT::default();
        unsafe {
            if GetClientRect(tb, &mut bar).is_err() {
                return;
            }
        }
        let h = (bar.bottom - bar.top).max(1) as u32;
        let dpi_scale = self.current_dpi() as f32 / 96.0;
        let tokens = taskbar_tokens(dpi_scale);
        let is_dev = crate::runtime::RuntimeMode::detect().is_dev();
        let w = compute_taskbar_strip_width(&tokens, is_dev);
        if w == 0 || h == 0 {
            return;
        }

        let Some(mut pixmap) = Pixmap::new(w, h) else {
            return;
        };
        let hovered = TASKBAR_HOVER.load(Ordering::SeqCst);
        let light = is_light_taskbar_theme();
        render_taskbar_strip(&mut pixmap, &self.data, dpi_scale, hovered, light, is_dev);

        push_pixmap_to_layered_window(wnd, &pixmap, 255);
        self.width = w as i32;
        self.height = h as i32;
        self.last_w = -1;
    }

    fn widgets_button_rect(&mut self) -> RECT {
        if let Some(last) = self.last_uia_check
            && last.elapsed() < Duration::from_secs(5)
        {
            return self.cached_widgets_rect;
        }
        self.last_uia_check = Some(Instant::now());
        self.cached_widgets_rect = RECT::default();

        let Some(tb) = self.taskbar_hwnd else {
            return RECT::default();
        };

        unsafe {
            if self.uia.is_none() {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                if let Ok(uia) =
                    CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
                {
                    self.uia = Some(uia);
                } else {
                    return RECT::default();
                }
            }

            let Some(uia) = self.uia.as_ref() else {
                return RECT::default();
            };

            if self.widgets_btn.is_none() {
                let Ok(root) = uia.ElementFromHandle(tb) else {
                    return RECT::default();
                };
                let bstr = BSTR::from("WidgetsButton");
                let var = VARIANT::from(bstr);
                if let Ok(cond) = uia.CreatePropertyCondition(UIA_AutomationIdPropertyId, &var)
                    && let Ok(btn) = root.FindFirst(TreeScope_Descendants, &cond)
                {
                    self.widgets_btn = Some(btn);
                }
            }

            if let Some(btn) = self.widgets_btn.as_ref() {
                if let Ok(rect) = btn.CurrentBoundingRectangle() {
                    self.cached_widgets_rect = rect;
                } else {
                    self.widgets_btn = None;
                }
            }
        }

        self.cached_widgets_rect
    }

    fn reposition(&mut self) {
        if !self.is_alive() {
            return;
        }
        let dpi = self.current_dpi();
        if dpi != self.last_dpi {
            self.last_dpi = dpi;
            self.paint();
        }
        let Some(tb) = self.taskbar_hwnd else {
            return;
        };
        let Some(wnd) = self.hwnd else {
            return;
        };

        unsafe {
            let Ok(tray) = FindWindowExW(Some(tb), None, w!("TrayNotifyWnd"), None) else {
                return;
            };
            if tray.0.is_null() {
                return;
            }

            let mut tr = RECT::default();
            let mut bar = RECT::default();
            if GetWindowRect(tray, &mut tr).is_err() || GetWindowRect(tb, &mut bar).is_err() {
                return;
            }

            let rtl = (GetWindowLongW(tb, GWL_EXSTYLE) as u32 & WS_EX_LAYOUTRTL.0) != 0;
            let mut edge = if rtl { tr.right } else { tr.left };

            // Win11 左寄せ時の Widgets ボタンとの重なりを回避
            let wb = self.widgets_button_rect();
            let px2 = ((2 * dpi as i32) / 96).max(2);
            if wb.right > wb.left
                && wb.top < bar.bottom
                && wb.bottom > bar.top
                && (if rtl {
                    wb.left < edge + self.width + px2 && wb.right > edge
                } else {
                    wb.right > edge - self.width - px2 && wb.left < edge
                })
            {
                edge = if rtl { wb.right } else { wb.left };
            }

            let screen_left = if rtl {
                edge + px2
            } else {
                edge - self.width - px2
            };

            // Win10 ReBarWindow32 調整
            if let Some(rebar) = self.rebar_hwnd
                && windows::Win32::UI::WindowsAndMessaging::IsWindow(Some(rebar)).as_bool()
            {
                let mut rr = RECT::default();
                if GetWindowRect(rebar, &mut rr).is_ok() {
                    let mut tl = POINT {
                        x: if rtl { rr.right } else { rr.left },
                        y: rr.top,
                    };
                    let _ = ScreenToClient(tb, &mut tl);
                    let mut lc = POINT {
                        x: screen_left,
                        y: bar.top,
                    };
                    let _ = ScreenToClient(tb, &mut lc);
                    if tl.x + (rr.right - rr.left) > lc.x && lc.x > tl.x {
                        let _ =
                            MoveWindow(rebar, tl.x, tl.y, lc.x - tl.x, rr.bottom - rr.top, true);
                    }
                }
            }

            // タスクバーが wnd より手前に来ていないか確認 (タスクバークリック等で背面に隠れるのを防止)
            let mut tb_is_above = false;
            let mut cur = GetWindow(wnd, GW_HWNDPREV).ok();
            let mut steps = 0;
            while let Some(prev) = cur {
                if prev.0.is_null() || steps >= 32 {
                    break;
                }
                if prev == tb {
                    tb_is_above = true;
                    break;
                }
                cur = GetWindow(prev, GW_HWNDPREV).ok();
                steps += 1;
            }
            let needs_z_restore = tb_is_above || !IsWindowVisible(wnd).as_bool();

            let rect_unchanged = tr == self.last_tray
                && bar == self.last_bar
                && wb == self.last_widgets
                && self.last_w == self.width
                && self.last_left == screen_left;

            if rect_unchanged && !needs_z_restore {
                return;
            }

            if rect_unchanged && needs_z_restore {
                let _ = SetWindowPos(
                    wnd,
                    Some(HWND_TOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_NOMOVE | SWP_NOSIZE,
                );
                return;
            }

            self.last_tray = tr;
            self.last_bar = bar;
            self.last_widgets = wb;
            self.last_w = self.width;
            self.last_left = screen_left;

            let _ = SetWindowPos(
                wnd,
                Some(HWND_TOPMOST),
                screen_left,
                bar.top,
                self.width,
                self.height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::GroupQuota;

    #[test]
    fn test_render_taskbar_strip() {
        let data = GaugeData {
            gemini: GroupQuota {
                weekly_pct: 84.0,
                five_h_pct: 100.0,
                weekly_reset: Some("Reset in: 2d 5h".to_string()),
                five_h_reset: Some("Reset in: 04:12".to_string()),
            },
            argon: GroupQuota {
                weekly_pct: 90.0,
                five_h_pct: 100.0,
                weekly_reset: Some("Reset in: 2d 5h".to_string()),
                five_h_reset: None,
            },
            claude_gpt: GroupQuota {
                weekly_pct: 68.0,
                five_h_pct: 42.0,
                weekly_reset: Some("Reset in: 1d 8h".to_string()),
                five_h_reset: Some("Reset in: 02:30".to_string()),
            },
        };

        let dpi_scale = 1.0f32;
        let tokens = taskbar_tokens(dpi_scale);
        let w = compute_taskbar_strip_width(&tokens, false);
        let h = 48u32;
        let mut pixmap = Pixmap::new(w, h).unwrap();
        render_taskbar_strip(&mut pixmap, &data, dpi_scale, false, false, false);

        std::fs::create_dir_all("target").unwrap();
        pixmap.save_png("target/taskbar_rendered.png").unwrap();
    }
}
