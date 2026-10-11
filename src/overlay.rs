use cosmic_text::{Attrs, Color as CColor, FontSystem, Metrics, Shaping, SwashCache, Weight};
use std::sync::Mutex;
use std::sync::atomic::{AtomicI32, Ordering};
use tiny_skia::*;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CreateCompatibleDC, CreateDIBSection,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, HBITMAP, HDC, HGDIOBJ, ReleaseDC, SelectObject,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, HICON, HMENU, PostQuitMessage, RegisterClassExW, SW_HIDE,
    SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetTimer, SetWindowPos,
    ShowWindow, ULW_ALPHA, UpdateLayeredWindow, WM_DESTROY, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::w;

pub const HUD_WIDTH: u32 = 220;
pub const HUD_HEIGHT: u32 = 240;

#[derive(Debug, Clone, Default)]
pub struct GroupQuota {
    pub weekly_pct: f64,
    pub five_h_pct: f64,
    pub weekly_reset: Option<String>,
    pub five_h_reset: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GaugeData {
    pub gemini: GroupQuota,
    pub claude_gpt: GroupQuota,
}

impl Default for GaugeData {
    fn default() -> Self {
        Self {
            gemini: GroupQuota {
                weekly_pct: 100.0,
                five_h_pct: 100.0,
                weekly_reset: None,
                five_h_reset: None,
            },
            claude_gpt: GroupQuota {
                weekly_pct: 100.0,
                five_h_pct: 100.0,
                weekly_reset: None,
                five_h_reset: None,
            },
        }
    }
}

pub struct TachometerOverlay {
    pub hwnd: HWND,
}

struct CachedSurface {
    screen_dc: HDC,
    mem_dc: HDC,
    hbitmap: HBITMAP,
    old_bmp: HGDIOBJ,
    bits: *mut u8,
}

unsafe impl Send for CachedSurface {}
unsafe impl Sync for CachedSurface {}

static CURRENT_ALPHA: AtomicI32 = AtomicI32::new(0);
static TARGET_ALPHA: AtomicI32 = AtomicI32::new(220); // 洗練された透過感 (約86%)
static GLOBAL_DATA: Mutex<Option<GaugeData>> = Mutex::new(None);
static FONT_SYSTEM: Mutex<Option<(FontSystem, SwashCache)>> = Mutex::new(None);
static SURFACE_CACHE: Mutex<Option<CachedSurface>> = Mutex::new(None);

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TIMER => {
            if wparam.0 == 1 {
                let cur = CURRENT_ALPHA.load(Ordering::SeqCst);
                let target = TARGET_ALPHA.load(Ordering::SeqCst);

                if cur != target {
                    let next = if cur < target {
                        // フェードイン: 程よい長さ（約120ms）でスッと気持ちよく立ち上がる
                        let diff = target - cur;
                        let step = ((diff as f32 * 0.38).ceil() as i32).clamp(14, 45);
                        (cur + step).min(target)
                    } else {
                        // フェードアウト: よりスピーディに、わずかにアニメーションが知覚できる速度（約50-60ms）で消える
                        let diff = cur - target;
                        let step = ((diff as f32 * 0.55).ceil() as i32).clamp(35, 80);
                        (cur - step).max(target)
                    };

                    CURRENT_ALPHA.store(next, Ordering::SeqCst);

                    if next == 0 {
                        unsafe {
                            let _ = ShowWindow(hwnd, SW_HIDE);
                        }
                    } else {
                        unsafe {
                            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                        }
                        update_layered_alpha(hwnd, next as u8);
                    }
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            if let Some(surf) = SURFACE_CACHE.lock().ok().and_then(|mut l| l.take()) {
                unsafe {
                    SelectObject(surf.mem_dc, surf.old_bmp);
                    let _ = DeleteObject(HGDIOBJ(surf.hbitmap.0));
                    let _ = DeleteDC(surf.mem_dc);
                    let _ = ReleaseDC(None, surf.screen_dc);
                }
            }
            unsafe {
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// DIBSection キャッシュを取得または初期化してクロージャを実行
fn with_surface<F, R>(f: F) -> R
where
    F: FnOnce(&CachedSurface) -> R,
{
    let mut cache = SURFACE_CACHE.lock().unwrap();
    if cache.is_none() {
        unsafe {
            let screen_dc = GetDC(None);
            let mem_dc = CreateCompatibleDC(Some(screen_dc));

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

            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let hbitmap: HBITMAP =
                CreateDIBSection(Some(mem_dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).unwrap();

            let old_bmp = SelectObject(mem_dc, HGDIOBJ(hbitmap.0));

            *cache = Some(CachedSurface {
                screen_dc,
                mem_dc,
                hbitmap,
                old_bmp,
                bits: bits as *mut u8,
            });
        }
    }
    f(cache.as_ref().unwrap())
}

/// GPU合成（UpdateLayeredWindow）のみを呼び出し、アルファ値（透明度）を瞬時に更新
fn apply_layered_alpha(hwnd: HWND, surf: &CachedSurface, alpha: u8) {
    let pt_src = POINT { x: 0, y: 0 };
    let size_wnd = SIZE {
        cx: HUD_WIDTH as i32,
        cy: HUD_HEIGHT as i32,
    };
    let blend = BLENDFUNCTION {
        BlendOp: 0, // AC_SRC_OVER
        BlendFlags: 0,
        SourceConstantAlpha: alpha,
        AlphaFormat: 1, // AC_SRC_ALPHA
    };

    unsafe {
        let _ = UpdateLayeredWindow(
            hwnd,
            Some(surf.screen_dc),
            None,
            Some(&size_wnd),
            Some(surf.mem_dc),
            Some(&pt_src),
            windows::Win32::Foundation::COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        );
    }
}

/// アニメーション中のアルファ更新（ビットマップ再生成なしで 0ms 高速更新）
fn update_layered_alpha(hwnd: HWND, alpha: u8) {
    with_surface(|surf| {
        apply_layered_alpha(hwnd, surf, alpha);
    });
}

/// クォータデータ更新時などにグラフィックスをオフスクリーンDIBに再描画
#[allow(clippy::chunks_exact_to_as_chunks)]
fn render_to_surface(hwnd: HWND, alpha: u8) {
    let mut pixmap = Pixmap::new(HUD_WIDTH, HUD_HEIGHT).unwrap();
    pixmap.fill(Color::TRANSPARENT);

    render_hud(&mut pixmap);
    let _ = pixmap.save_png("C:\\Users\\yumatsuzawa\\.gemini\\antigravity\\brain\\dda008b2-8fab-4216-9b5f-eb58f87735e9\\scratch\\hud_rendered.png");

    with_surface(|surf| unsafe {
        let src_slice = pixmap.data();
        let dst_slice =
            std::slice::from_raw_parts_mut(surf.bits, (HUD_WIDTH * HUD_HEIGHT * 4) as usize);

        for (src_chunk, dst_chunk) in src_slice.chunks_exact(4).zip(dst_slice.chunks_exact_mut(4)) {
            let r = src_chunk[0];
            let g = src_chunk[1];
            let b = src_chunk[2];
            let a = src_chunk[3];
            // DIBSection は BGRA
            dst_chunk[0] = b;
            dst_chunk[1] = g;
            dst_chunk[2] = r;
            dst_chunk[3] = a;
        }

        apply_layered_alpha(hwnd, surf, alpha);
    });
}

/// 高品位テキスト描画ヘルパー
#[allow(clippy::too_many_arguments)]
fn draw_text(
    pixmap: &mut Pixmap,
    text: &str,
    x: f32,
    y: f32,
    font_size: f32,
    color: Color,
    center: bool,
    weight: Weight,
) {
    let mut lock = FONT_SYSTEM.lock().unwrap();
    let (font_system, cache) = lock.as_mut().unwrap();

    let mut buffer =
        cosmic_text::Buffer::new(font_system, Metrics::new(font_size, font_size * 1.2));
    let mut buffer = buffer.borrow_with(font_system);
    buffer.set_size(Some(200.0), Some(50.0));
    let mut attrs = Attrs::new();
    attrs = attrs.weight(weight);
    buffer.set_text(text, &attrs, Shaping::Advanced, None);
    buffer.shape_until_scroll(true);

    let text_width = buffer.layout_runs().next().map(|r| r.line_w).unwrap_or(0.0);
    let offset_x = if center { x - (text_width / 2.0) } else { x };

    let alpha_u8 = (color.alpha() * 255.0) as u8;
    let r_u8 = (color.red() * 255.0) as u8;
    let g_u8 = (color.green() * 255.0) as u8;
    let b_u8 = (color.blue() * 255.0) as u8;

    buffer.draw(
        cache,
        CColor::rgba(r_u8, g_u8, b_u8, alpha_u8),
        |gx, gy, _w, _h, col| {
            let px = (offset_x + gx as f32).round() as i32;
            let py = (y + gy as f32).round() as i32;

            if px >= 0 && px < HUD_WIDTH as i32 && py >= 0 && py < HUD_HEIGHT as i32 {
                let idx = (py as usize * HUD_WIDTH as usize + px as usize) * 4;
                let data = pixmap.data_mut();

                let a = col.a() as f32 / 255.0;
                let inv_a = 1.0 - a;

                // Premultiplied alpha 合成
                let src_r = col.r() as f32 * a;
                let src_g = col.g() as f32 * a;
                let src_b = col.b() as f32 * a;

                data[idx] = (src_r + (data[idx] as f32 * inv_a)).min(255.0) as u8;
                data[idx + 1] = (src_g + (data[idx + 1] as f32 * inv_a)).min(255.0) as u8;
                data[idx + 2] = (src_b + (data[idx + 2] as f32 * inv_a)).min(255.0) as u8;
                data[idx + 3] =
                    ((col.a() as f32) + (data[idx + 3] as f32 * inv_a)).min(255.0) as u8;
            }
        },
    );
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

#[allow(clippy::too_many_arguments)]
fn render_tachometer(
    pixmap: &mut Pixmap,
    cx: f32,
    cy: f32,
    radius: f32,
    pct: f64,
    label: &str,
    val_text: &str,
    theme_color: Color,
    reset_text: Option<&str>,
) {
    let start_deg = 145.0;
    let sweep_deg = 250.0;
    let track_width = 4.2;

    // 1. メーター上部ラベル (メーター円弧との明確な空間を確保: 25px, 11.0px)
    draw_text(
        pixmap,
        label,
        cx,
        cy - radius - 25.0,
        11.0,
        Color::from_rgba8(225, 232, 245, 250), // 鮮明な高コントラストオフホワイト
        true,
        Weight::MEDIUM,
    );

    // 2. 背景トラック
    if let Some(track_path) = make_arc_path(cx, cy, radius, start_deg, sweep_deg) {
        let mut paint = Paint::default();
        paint.set_color_rgba8(45, 52, 65, 200);
        paint.anti_alias = true;

        let stroke = Stroke {
            width: track_width,
            line_cap: LineCap::Round,
            ..Default::default()
        };
        pixmap.stroke_path(&track_path, &paint, &stroke, Transform::identity(), None);
    }

    // 3. アクティブプログレス円弧
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

    // 4. スポーティな指針 (テーパー針)
    let needle_angle = (start_deg + sweep_deg * (pct / 100.0) as f32).to_radians();
    let needle_len = radius - 3.5;

    let tip_x = cx + needle_len * needle_angle.cos();
    let tip_y = cy + needle_len * needle_angle.sin();

    let perp_angle = needle_angle + std::f32::consts::FRAC_PI_2;
    let base_r = 2.4;
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
        paint.set_color_rgba8(245, 250, 255, 255);
        paint.anti_alias = true;
        pixmap.fill_path(
            &needle_path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }

    // 5. センターハブ (円形)
    if let Some(hub_outer) = PathBuilder::from_circle(cx, cy, 4.0) {
        let mut paint = Paint::default();
        paint.set_color_rgba8(26, 30, 38, 255);
        paint.anti_alias = true;
        pixmap.fill_path(
            &hub_outer,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );

        let stroke = Stroke {
            width: 1.2,
            ..Default::default()
        };
        let mut border_paint = Paint::default();
        border_paint.set_color_rgba8(220, 230, 245, 255);
        border_paint.anti_alias = true;
        pixmap.stroke_path(
            &hub_outer,
            &border_paint,
            &stroke,
            Transform::identity(),
            None,
        );
    }

    // 6. パーセント値 (元の落ち着いたサイズ 11.5px, 力強い太字)
    draw_text(
        pixmap,
        val_text,
        cx,
        cy + 5.5,
        11.5,
        theme_color,
        true,
        Weight::BOLD,
    );

    // 7. リセット日時 (パーセント値の直下に控えめかつクリアに配置)
    if let Some(reset) = reset_text {
        draw_text(
            pixmap,
            reset,
            cx,
            cy + 20.0,
            8.5,
            Color::from_rgba8(160, 174, 192, 240), // 視認性の高いスレートグレー (#a0aec0)
            true,
            Weight::MEDIUM,
        );
    }
}

fn get_quota_color(pct: f64) -> Color {
    if pct > 50.0 {
        Color::from_rgba8(52, 211, 153, 255) // 高視認性エメラルドグリーン (#34d399)
    } else if pct > 20.0 {
        Color::from_rgba8(251, 191, 36, 255) // 鮮やかなアンバーゴールド (#fbbf24)
    } else {
        Color::from_rgba8(248, 113, 113, 255) // 鮮明なコーラルレッド (#f87171)
    }
}

/// HUD 全体の描画
fn render_hud(pixmap: &mut Pixmap) {
    let data = GLOBAL_DATA.lock().unwrap().clone().unwrap_or_default();

    let w = HUD_WIDTH as f32;
    let h = HUD_HEIGHT as f32;
    let r = 12.0;

    // 角丸カードのパス
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
        // 背景: Antigravity に完璧に溶け込む半透明ダークアクリル (#151821, アルファ ~82%)
        let mut bg_paint = Paint::default();
        bg_paint.set_color_rgba8(21, 24, 33, 205);
        bg_paint.anti_alias = true;
        pixmap.fill_path(
            &panel_path,
            &bg_paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );

        // 繊細なアクリル風ハイライトボーダー (角丸に沿って滑らかに)
        let mut border_paint = Paint::default();
        border_paint.set_color_rgba8(70, 80, 105, 140);
        border_paint.anti_alias = true;
        let stroke = Stroke {
            width: 1.0,
            ..Default::default()
        };
        pixmap.stroke_path(
            &panel_path,
            &border_paint,
            &stroke,
            Transform::identity(),
            None,
        );
    }

    // メーターの X 座標: HUD 幅 220px に対して左右を心地よく配置（間隔 92px）
    let left_cx = 64.0;
    let right_cx = 156.0;
    let meter_r = 19.5;

    // ==========================================
    // 上段: GEMINI MODELS (中央見出し + 適切な余白, 12.0px)
    // ==========================================
    draw_text(
        pixmap,
        "GEMINI MODELS",
        w / 2.0,
        12.0,
        12.0,
        Color::from_rgba8(96, 165, 250, 255), // 高視認性 Google Blue (#60a5fa)
        true,
        Weight::SEMIBOLD,
    );

    let gem_weekly_color = get_quota_color(data.gemini.weekly_pct);
    let gem_weekly_val = format!("{:.0}%", data.gemini.weekly_pct);
    render_tachometer(
        pixmap,
        left_cx,
        74.0,
        meter_r,
        data.gemini.weekly_pct,
        "Weekly Limit",
        &gem_weekly_val,
        gem_weekly_color,
        data.gemini.weekly_reset.as_deref(),
    );

    let gem_5h_color = get_quota_color(data.gemini.five_h_pct);
    let gem_5h_val = format!("{:.0}%", data.gemini.five_h_pct);
    render_tachometer(
        pixmap,
        right_cx,
        74.0,
        meter_r,
        data.gemini.five_h_pct,
        "5-Hour Burst",
        &gem_5h_val,
        gem_5h_color,
        data.gemini.five_h_reset.as_deref(),
    );

    // ==========================================
    // 水平ディバイダーライン
    // ==========================================
    let mut sep_pb = PathBuilder::new();
    sep_pb.move_to(18.0, 119.0);
    sep_pb.line_to(w - 18.0, 119.0);
    if let Some(sep_path) = sep_pb.finish() {
        let mut sep_paint = Paint::default();
        sep_paint.set_color_rgba8(55, 65, 80, 120);
        sep_paint.anti_alias = true;
        let stroke = Stroke {
            width: 1.0,
            ..Default::default()
        };
        pixmap.stroke_path(&sep_path, &sep_paint, &stroke, Transform::identity(), None);
    }

    // ==========================================
    // 下段: CLAUDE & GPT (中央見出し + 適切な余白, 12.0px)
    // ==========================================
    draw_text(
        pixmap,
        "CLAUDE & GPT",
        w / 2.0,
        132.0,
        12.0,
        Color::from_rgba8(251, 191, 36, 255), // 高視認性 Warm Amber (#fbbf24)
        true,
        Weight::SEMIBOLD,
    );

    let cg_weekly_color = get_quota_color(data.claude_gpt.weekly_pct);
    let cg_weekly_val = format!("{:.0}%", data.claude_gpt.weekly_pct);
    render_tachometer(
        pixmap,
        left_cx,
        194.0,
        meter_r,
        data.claude_gpt.weekly_pct,
        "Weekly Limit",
        &cg_weekly_val,
        cg_weekly_color,
        data.claude_gpt.weekly_reset.as_deref(),
    );

    let cg_5h_color = get_quota_color(data.claude_gpt.five_h_pct);
    let cg_5h_val = format!("{:.0}%", data.claude_gpt.five_h_pct);
    render_tachometer(
        pixmap,
        right_cx,
        194.0,
        meter_r,
        data.claude_gpt.five_h_pct,
        "5-Hour Burst",
        &cg_5h_val,
        cg_5h_color,
        data.claude_gpt.five_h_reset.as_deref(),
    );
}

impl TachometerOverlay {
    pub fn new() -> Result<Self, String> {
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
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_TRANSPARENT,
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

            // 16ms = 60fps
            SetTimer(Some(hwnd), 1, 16, None);

            let overlay = Self { hwnd };
            render_to_surface(hwnd, 0);

            Ok(overlay)
        }
    }

    pub fn update_data(&self, new_data: GaugeData) {
        if let Ok(mut lock) = GLOBAL_DATA.lock() {
            *lock = Some(new_data);
        }
        let alpha = CURRENT_ALPHA.load(Ordering::SeqCst);
        render_to_surface(self.hwnd, alpha.max(0) as u8);
    }

    pub fn fade_in(&self) {
        TARGET_ALPHA.store(220, Ordering::SeqCst);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_hud_with_reset_timestamps() {
        {
            let mut lock = FONT_SYSTEM.lock().unwrap();
            if lock.is_none() {
                let font_system = FontSystem::new();
                let cache = SwashCache::new();
                *lock = Some((font_system, cache));
            }
        }

        let test_data = GaugeData {
            gemini: GroupQuota {
                weekly_pct: 78.0,
                five_h_pct: 100.0,
                weekly_reset: Some("10/17 21:32".to_string()),
                five_h_reset: Some("10/11 21:27".to_string()),
            },
            claude_gpt: GroupQuota {
                weekly_pct: 42.0,
                five_h_pct: 15.0,
                weekly_reset: Some("10/17 21:32".to_string()),
                five_h_reset: Some("10/11 18:45".to_string()),
            },
        };

        if let Ok(mut lock) = GLOBAL_DATA.lock() {
            *lock = Some(test_data);
        }

        let mut pixmap = Pixmap::new(HUD_WIDTH, HUD_HEIGHT).unwrap();
        pixmap.fill(Color::TRANSPARENT);
        render_hud(&mut pixmap);

        let out_path = "C:\\Users\\yumatsuzawa\\.gemini\\antigravity\\brain\\dda008b2-8fab-4216-9b5f-eb58f87735e9\\scratch\\hud_rendered.png";
        pixmap
            .save_png(out_path)
            .expect("Failed to save render test PNG");
    }
}
