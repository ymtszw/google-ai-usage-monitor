use cosmic_text::{Attrs, Color as CColor, FontSystem, Metrics, Shaping, SwashCache};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Mutex;
use tiny_skia::*;
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, SetDIBitsToDevice, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, PostQuitMessage, RegisterClassExW,
    SetLayeredWindowAttributes, SetTimer, SetWindowPos, ShowWindow, HICON, HMENU,
    LWA_ALPHA, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_SHOWNOACTIVATE,
    WM_DESTROY, WM_PAINT, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};

pub const HUD_WIDTH: u32 = 320;
pub const HUD_HEIGHT: u32 = 110;

#[derive(Debug, Clone)]
pub struct GaugeData {
    pub weekly_pct: f64,
    pub five_h_pct: f64,
}

impl Default for GaugeData {
    fn default() -> Self {
        Self {
            weekly_pct: 100.0,
            five_h_pct: 100.0,
        }
    }
}

pub struct TachometerOverlay {
    pub hwnd: HWND,
}

static CURRENT_ALPHA: AtomicI32 = AtomicI32::new(0);
static TARGET_ALPHA: AtomicI32 = AtomicI32::new(245);
static GLOBAL_DATA: Mutex<Option<GaugeData>> = Mutex::new(None);
static FONT_SYSTEM: Mutex<Option<(FontSystem, SwashCache)>> = Mutex::new(None);

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            unsafe {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);

                // tiny-skia による超高品位アンチエイリアス描画
                let mut pixmap = Pixmap::new(HUD_WIDTH, HUD_HEIGHT).unwrap();
                render_hud(&mut pixmap);

                // RGBA -> BGRA に変換 (DIB用)
                let mut bgra_data = pixmap.data().to_vec();
                for chunk in bgra_data.chunks_exact_mut(4) {
                    let r = chunk[0];
                    let b = chunk[2];
                    chunk[0] = b;
                    chunk[2] = r;
                }

                let bmi = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: HUD_WIDTH as i32,
                        biHeight: -(HUD_HEIGHT as i32), // トップダウン
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: BI_RGB.0,
                        ..Default::default()
                    },
                    ..Default::default()
                };

                let _ = SetDIBitsToDevice(
                    hdc,
                    0,
                    0,
                    HUD_WIDTH,
                    HUD_HEIGHT,
                    0,
                    0,
                    0,
                    HUD_HEIGHT,
                    bgra_data.as_ptr() as *const _,
                    &bmi,
                    DIB_RGB_COLORS,
                );

                let _ = EndPaint(hwnd, &ps);
            }
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == 1 {
                let cur = CURRENT_ALPHA.load(Ordering::SeqCst);
                let target = TARGET_ALPHA.load(Ordering::SeqCst);

                if cur != target {
                    let step = if cur < target {
                        ((target - cur) / 2).max(40)
                    } else {
                        ((cur - target) / 2).max(50) * -1
                    };
                    let next = (cur + step).clamp(0, 255);
                    CURRENT_ALPHA.store(next, Ordering::SeqCst);

                    unsafe {
                        let _ = SetLayeredWindowAttributes(
                            hwnd,
                            COLORREF(0),
                            next as u8,
                            LWA_ALPHA,
                        );

                        if next == 0 {
                            let _ = ShowWindow(hwnd, SW_HIDE);
                        } else {
                            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                        }
                    }
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe {
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// 高品位テキスト描画ヘルパー
fn draw_text(
    pixmap: &mut Pixmap,
    text: &str,
    x: f32,
    y: f32,
    font_size: f32,
    color: Color,
    center: bool,
) {
    let mut lock = FONT_SYSTEM.lock().unwrap();
    let (font_system, cache) = lock.as_mut().unwrap();

    let mut buffer = cosmic_text::Buffer::new(
        font_system,
        Metrics::new(font_size, font_size * 1.2),
    );
    let mut buffer = buffer.borrow_with(font_system);
    buffer.set_size(Some(200.0), Some(50.0));
    buffer.set_text(text, &Attrs::new(), Shaping::Advanced, None);
    buffer.shape_until_scroll(true);

    let text_width = buffer.layout_runs().next().map(|r| r.line_w).unwrap_or(0.0);
    let offset_x = if center { x - (text_width / 2.0) } else { x };

    let alpha_u8 = (color.alpha() * 255.0) as u8;
    let r_u8 = (color.red() * 255.0) as u8;
    let g_u8 = (color.green() * 255.0) as u8;
    let b_u8 = (color.blue() * 255.0) as u8;

    buffer.draw(cache, CColor::rgba(r_u8, g_u8, b_u8, alpha_u8), |gx, gy, _w, _h, col| {
        let px = (offset_x + gx as f32).round() as i32;
        let py = (y + gy as f32).round() as i32;

        if px >= 0 && px < HUD_WIDTH as i32 && py >= 0 && py < HUD_HEIGHT as i32 {
            let idx = (py as usize * HUD_WIDTH as usize + px as usize) * 4;
            let data = pixmap.data_mut();

            // アルファブレンド
            let a = col.a() as f32 / 255.0;
            let inv_a = 1.0 - a;

            data[idx + 0] = ((col.r() as f32 * a) + (data[idx + 0] as f32 * inv_a)) as u8;
            data[idx + 1] = ((col.g() as f32 * a) + (data[idx + 1] as f32 * inv_a)) as u8;
            data[idx + 2] = ((col.b() as f32 * a) + (data[idx + 2] as f32 * inv_a)) as u8;
            data[idx + 3] = 255;
        }
    });
}

/// 扇形円弧パスを精密生成 (start_angle, sweep_angle は度数法)
fn make_arc_path(cx: f32, cy: f32, radius: f32, start_deg: f32, sweep_deg: f32) -> Option<Path> {
    let mut pb = PathBuilder::new();
    let steps = (sweep_deg.abs() / 3.0).ceil().max(2.0) as usize;

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

/// 参考画像をベースにした美しいタコメーターメーターのレンダリング
fn render_tachometer(
    pixmap: &mut Pixmap,
    cx: f32,
    cy: f32,
    radius: f32,
    pct: f64,
    label: &str,
    val_text: &str,
    theme_color: Color,
) {
    let start_deg = 145.0;
    let sweep_deg = 250.0;
    let track_width = 7.0;

    // 1. ラベル (メーターの上部に余裕をもって配置)
    draw_text(
        pixmap,
        label,
        cx,
        cy - radius - 18.0,
        12.0,
        Color::from_rgba8(200, 205, 215, 255),
        true,
    );

    // 2. 背景トラック (暗いスレートグレーの円弧・ラウンドキャップ)
    if let Some(track_path) = make_arc_path(cx, cy, radius, start_deg, sweep_deg) {
        let mut paint = Paint::default();
        paint.set_color_rgba8(45, 50, 60, 255);
        paint.anti_alias = true;

        let stroke = Stroke {
            width: track_width,
            line_cap: LineCap::Round,
            ..Default::default()
        };
        pixmap.stroke_path(&track_path, &paint, &stroke, Transform::identity(), None);
    }

    // 3. アクティブプログレス円弧 (ラウンドキャップ付き)
    let active_sweep = (sweep_deg * (pct / 100.0) as f32).clamp(0.1, sweep_deg);
    if let Some(active_path) = make_arc_path(cx, cy, radius, start_deg, active_sweep) {
        let mut paint = Paint::default();
        paint.set_color(theme_color);
        paint.anti_alias = true;

        let stroke = Stroke {
            width: track_width,
            line_cap: LineCap::Round,
            ..Default::default()
        };
        pixmap.stroke_path(&active_path, &paint, &stroke, Transform::identity(), None);
    }

    // 4. 指針 (参考画像風の美しいテーパー状ニードル)
    let needle_angle = (start_deg + sweep_deg * (pct / 100.0) as f32).to_radians();
    let needle_len = radius - 5.0;

    let tip_x = cx + needle_len * needle_angle.cos();
    let tip_y = cy + needle_len * needle_angle.sin();

    // 指針の根元 (少し幅を持たせてスポーティに)
    let perp_angle = needle_angle + std::f32::consts::FRAC_PI_2;
    let base_r = 3.5;
    let b1_x = cx + base_r * perp_angle.cos();
    let b1_y = cy + base_r * perp_angle.sin();
    let b2_x = cx - base_r * perp_angle.cos();
    let b2_y = cy - base_r * perp_angle.sin();

    let mut needle_pb = PathBuilder::new();
    needle_pb.move_to(b1_x, b1_y);
    needle_pb.line_to(tip_x, tip_y);
    needle_pb.line_to(b2_x, b2_y);
    needle_pb.close();

    if let Some(needle_path) = needle_pb.finish() {
        let mut paint = Paint::default();
        paint.set_color_rgba8(240, 245, 255, 255); // 白く発光するニードル
        paint.anti_alias = true;
        pixmap.fill_path(&needle_path, &paint, FillRule::Winding, Transform::identity(), None);
    }

    // 5. センターハブ (円形・メタル風)
    if let Some(hub_outer) = PathBuilder::from_circle(cx, cy, 6.0) {
        let mut paint = Paint::default();
        paint.set_color_rgba8(30, 34, 42, 255);
        paint.anti_alias = true;
        pixmap.fill_path(&hub_outer, &paint, FillRule::Winding, Transform::identity(), None);

        let stroke = Stroke {
            width: 1.5,
            ..Default::default()
        };
        let mut border_paint = Paint::default();
        border_paint.set_color_rgba8(220, 225, 235, 255);
        border_paint.anti_alias = true;
        pixmap.stroke_path(&hub_outer, &border_paint, &stroke, Transform::identity(), None);
    }

    // 6. パーセント値 (メーターの中央下に大きく・明瞭に配置)
    draw_text(
        pixmap,
        val_text,
        cx,
        cy + 10.0,
        15.0,
        theme_color,
        true,
    );
}

/// HUD全体のアンチエイリアス描画
fn render_hud(pixmap: &mut Pixmap) {
    let data = GLOBAL_DATA.lock().unwrap().clone().unwrap_or_default();

    // 背景: Antigravity に馴染む角丸ダークパネル (#1E222B)
    let w = HUD_WIDTH as f32;
    let h = HUD_HEIGHT as f32;
    let r = 12.0;

    let mut pb = PathBuilder::new();
    pb.move_to(r, 0.0);
    pb.line_to(w - r, 0.0);
    pb.quad_to(w, 0.0, w, r);
    pb.line_to(w, h - r);
    pb.quad_to(w, h, w - r, h);
    pb.line_to(r, h);
    pb.quad_to(0.0, h, 0.0, h - r);
    pb.line_to(0.0, r);
    pb.quad_to(0.0, 0.0, r, 0.0);
    pb.close();

    if let Some(panel_path) = pb.finish() {
        let mut bg_paint = Paint::default();
        bg_paint.set_color_rgba8(24, 28, 36, 245);
        bg_paint.anti_alias = true;
        pixmap.fill_path(&panel_path, &bg_paint, FillRule::Winding, Transform::identity(), None);

        // 微細なアクリル風ハイライトボーダー
        let mut border_paint = Paint::default();
        border_paint.set_color_rgba8(65, 75, 95, 180);
        border_paint.anti_alias = true;
        let stroke = Stroke {
            width: 1.2,
            ..Default::default()
        };
        pixmap.stroke_path(&panel_path, &border_paint, &stroke, Transform::identity(), None);
    }

    // メーター1: Gemini Weekly
    let weekly_color = if data.weekly_pct > 50.0 {
        Color::from_rgba8(0, 230, 118, 255) // エメラルドグリーン
    } else if data.weekly_pct > 20.0 {
        Color::from_rgba8(255, 179, 0, 255) // アンバー
    } else {
        Color::from_rgba8(255, 61, 0, 255)  // レッド
    };
    let weekly_val = format!("{:.0}%", data.weekly_pct);
    render_tachometer(
        pixmap,
        80.0,
        58.0,
        32.0,
        data.weekly_pct,
        "Weekly Limit",
        &weekly_val,
        weekly_color,
    );

    // セパレータ (繊細なグラデーション風垂直ライン)
    let mut sep_pb = PathBuilder::new();
    sep_pb.move_to(160.0, 18.0);
    sep_pb.line_to(160.0, h - 18.0);
    if let Some(sep_path) = sep_pb.finish() {
        let mut sep_paint = Paint::default();
        sep_paint.set_color_rgba8(55, 65, 80, 160);
        sep_paint.anti_alias = true;
        let stroke = Stroke {
            width: 1.0,
            ..Default::default()
        };
        pixmap.stroke_path(&sep_path, &sep_paint, &stroke, Transform::identity(), None);
    }

    // メーター2: Gemini 5-Hour
    let five_h_color = if data.five_h_pct > 50.0 {
        Color::from_rgba8(0, 229, 255, 255) // スカイブルー / シアン
    } else if data.five_h_pct > 20.0 {
        Color::from_rgba8(255, 179, 0, 255)
    } else {
        Color::from_rgba8(255, 61, 0, 255)
    };
    let five_h_val = format!("{:.0}%", data.five_h_pct);
    render_tachometer(
        pixmap,
        240.0,
        58.0,
        32.0,
        data.five_h_pct,
        "5-Hour Burst",
        &five_h_val,
        five_h_color,
    );
}

impl TachometerOverlay {
    pub fn new() -> Result<Self, String> {
        // フォントシステムの初期化 (システムフォントを自動ロード)
        {
            let mut lock = FONT_SYSTEM.lock().unwrap();
            if lock.is_none() {
                let font_system = FontSystem::new();
                let cache = SwashCache::new();
                *lock = Some((font_system, cache));
            }
        }

        unsafe {
            let hinstance = GetModuleHandleW(None).map_err(|e| e.to_string())?;
            let class_name = w!("AntigravityQuotaTachometerHUD");

            let wnd_class = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: windows::Win32::UI::WindowsAndMessaging::WNDCLASS_STYLES(0),
                lpfnWndProc: Some(wnd_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: hinstance.into(),
                hIcon: HICON::default(),
                hCursor: windows::Win32::UI::WindowsAndMessaging::HCURSOR::default(),
                hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH::default(),
                lpszMenuName: windows::core::PCWSTR::null(),
                lpszClassName: class_name,
                hIconSm: HICON::default(),
            };

            RegisterClassExW(&wnd_class);

            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
                class_name,
                w!("Antigravity Quota HUD"),
                WS_POPUP,
                -1000,
                -1000,
                HUD_WIDTH as i32,
                HUD_HEIGHT as i32,
                None,
                Some(HMENU::default()),
                Some(hinstance.into()),
                None,
            )
            .map_err(|e| e.to_string())?;

            SetLayeredWindowAttributes(hwnd, COLORREF(0), 0, LWA_ALPHA)
                .map_err(|e| e.to_string())?;

            SetTimer(Some(hwnd), 1, 16, None);

            Ok(Self { hwnd })
        }
    }

    pub fn update_data(&self, new_data: GaugeData) {
        if let Ok(mut lock) = GLOBAL_DATA.lock() {
            *lock = Some(new_data);
        }
        unsafe {
            let rect = RECT {
                left: 0,
                top: 0,
                right: HUD_WIDTH as i32,
                bottom: HUD_HEIGHT as i32,
            };
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(self.hwnd), Some(&rect), false);
        }
    }

    pub fn fade_in(&self) {
        TARGET_ALPHA.store(245, Ordering::SeqCst);
    }

    pub fn fade_out(&self) {
        TARGET_ALPHA.store(0, Ordering::SeqCst);
    }

    pub fn set_position(&self, x: i32, y: i32) {
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                None,
                x,
                y,
                HUD_WIDTH as i32,
                HUD_HEIGHT as i32,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}
