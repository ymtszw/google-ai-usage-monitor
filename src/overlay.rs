use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Mutex;
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreatePen, CreateSolidBrush, DeleteObject, EndPaint, FillRect,
    LineTo, MoveToEx, SelectObject, SetBkMode, SetTextColor, TextOutW, HBRUSH, HDC, HGDIOBJ,
    PAINTSTRUCT, PS_SOLID, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, PostQuitMessage, RegisterClassExW,
    SetLayeredWindowAttributes, SetTimer, SetWindowPos, ShowWindow, HICON, HMENU,
    LWA_ALPHA, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_SHOWNOACTIVATE,
    WM_DESTROY, WM_PAINT, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};

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
static TARGET_ALPHA: AtomicI32 = AtomicI32::new(235);
static GLOBAL_DATA: Mutex<Option<GaugeData>> = Mutex::new(None);

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
                draw_tachometers(hdc);
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
                        (target - cur).clamp(15, 45)
                    } else {
                        (cur - target).clamp(15, 45) * -1
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

unsafe fn draw_meter(
    hdc: HDC,
    cx: i32,
    cy: i32,
    radius: i32,
    pct: f64,
    label: &str,
    val_text: &str,
    color_rgb: (u8, u8, u8),
) {
    unsafe {
        let font_label = CreateFontW(
            13, 0, 0, 0, 600, 0, 0, 0,
            windows::Win32::Graphics::Gdi::FONT_CHARSET(1),
            windows::Win32::Graphics::Gdi::FONT_OUTPUT_PRECISION(0),
            windows::Win32::Graphics::Gdi::FONT_CLIP_PRECISION(0),
            windows::Win32::Graphics::Gdi::FONT_QUALITY(2),
            0,
            w!("Segoe UI"),
        );
        let font_val = CreateFontW(
            15, 0, 0, 0, 700, 0, 0, 0,
            windows::Win32::Graphics::Gdi::FONT_CHARSET(1),
            windows::Win32::Graphics::Gdi::FONT_OUTPUT_PRECISION(0),
            windows::Win32::Graphics::Gdi::FONT_CLIP_PRECISION(0),
            windows::Win32::Graphics::Gdi::FONT_QUALITY(2),
            0,
            w!("Segoe UI"),
        );

        let pen_bg = CreatePen(PS_SOLID, 6, COLORREF(0x003A3A3A));
        let old_pen = SelectObject(hdc, HGDIOBJ(pen_bg.0));

        let start_angle = 140.0;
        let total_sweep = 260.0;
        let segments = 24;

        for i in 0..segments {
            let a1 = (start_angle + (total_sweep / segments as f64) * (i as f64)).to_radians();
            let a2 = (start_angle + (total_sweep / segments as f64) * ((i + 1) as f64)).to_radians();

            let x1 = cx + (radius as f64 * a1.cos()) as i32;
            let y1 = cy + (radius as f64 * a1.sin()) as i32;
            let x2 = cx + (radius as f64 * a2.cos()) as i32;
            let y2 = cy + (radius as f64 * a2.sin()) as i32;

            let _ = MoveToEx(hdc, x1, y1, None);
            let _ = LineTo(hdc, x2, y2);
        }

        let _ = DeleteObject(HGDIOBJ(pen_bg.0));

        let pen_fg = CreatePen(
            PS_SOLID,
            6,
            COLORREF(
                (color_rgb.0 as u32) | ((color_rgb.1 as u32) << 8) | ((color_rgb.2 as u32) << 16),
            ),
        );
        SelectObject(hdc, HGDIOBJ(pen_fg.0));

        let active_segments = ((pct / 100.0) * segments as f64).round() as i32;
        for i in 0..active_segments {
            let a1 = (start_angle + (total_sweep / segments as f64) * (i as f64)).to_radians();
            let a2 = (start_angle + (total_sweep / segments as f64) * ((i + 1) as f64)).to_radians();

            let x1 = cx + (radius as f64 * a1.cos()) as i32;
            let y1 = cy + (radius as f64 * a1.sin()) as i32;
            let x2 = cx + (radius as f64 * a2.cos()) as i32;
            let y2 = cy + (radius as f64 * a2.sin()) as i32;

            let _ = MoveToEx(hdc, x1, y1, None);
            let _ = LineTo(hdc, x2, y2);
        }

        let _ = DeleteObject(HGDIOBJ(pen_fg.0));

        let needle_angle = (start_angle + total_sweep * (pct / 100.0)).to_radians();
        let nx = cx + ((radius - 8) as f64 * needle_angle.cos()) as i32;
        let ny = cy + ((radius - 8) as f64 * needle_angle.sin()) as i32;

        let pen_needle = CreatePen(PS_SOLID, 2, COLORREF(0x00FFFFFF));
        SelectObject(hdc, HGDIOBJ(pen_needle.0));
        let _ = MoveToEx(hdc, cx, cy, None);
        let _ = LineTo(hdc, nx, ny);
        let _ = DeleteObject(HGDIOBJ(pen_needle.0));

        let brush_hub = CreateSolidBrush(COLORREF(0x00FFFFFF));
        let old_brush = SelectObject(hdc, HGDIOBJ(brush_hub.0));
        let mut hub_rect = RECT {
            left: cx - 4,
            top: cy - 4,
            right: cx + 4,
            bottom: cy + 4,
        };
        FillRect(hdc, &mut hub_rect, brush_hub);
        let _ = DeleteObject(HGDIOBJ(brush_hub.0));

        SetBkMode(hdc, TRANSPARENT);

        SelectObject(hdc, HGDIOBJ(font_label.0));
        SetTextColor(hdc, COLORREF(0x00A0A0A0));
        let wide_label: Vec<u16> = label.encode_utf16().collect();
        let _ = TextOutW(hdc, cx - 28, cy - 38, &wide_label);

        SelectObject(hdc, HGDIOBJ(font_val.0));
        SetTextColor(
            hdc,
            COLORREF(
                (color_rgb.0 as u32) | ((color_rgb.1 as u32) << 8) | ((color_rgb.2 as u32) << 16),
            ),
        );
        let wide_val: Vec<u16> = val_text.encode_utf16().collect();
        let _ = TextOutW(hdc, cx - 18, cy + 10, &wide_val);

        SelectObject(hdc, old_pen);
        SelectObject(hdc, old_brush);
        let _ = DeleteObject(HGDIOBJ(font_label.0));
        let _ = DeleteObject(HGDIOBJ(font_val.0));
    }
}

unsafe fn draw_tachometers(hdc: HDC) {
    let data = GLOBAL_DATA.lock().unwrap().clone().unwrap_or_default();

    unsafe {
        let bg_brush = CreateSolidBrush(COLORREF(0x00201A18));
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 280,
            bottom: 85,
        };
        FillRect(hdc, &mut rect, bg_brush);
        let _ = DeleteObject(HGDIOBJ(bg_brush.0));

        let border_pen = CreatePen(PS_SOLID, 1, COLORREF(0x003A3028));
        let old_pen = SelectObject(hdc, HGDIOBJ(border_pen.0));
        let _ = MoveToEx(hdc, 0, 0, None);
        let _ = LineTo(hdc, 279, 0);
        let _ = LineTo(hdc, 279, 84);
        let _ = LineTo(hdc, 0, 84);
        let _ = LineTo(hdc, 0, 0);
        let _ = DeleteObject(HGDIOBJ(border_pen.0));
        SelectObject(hdc, old_pen);

        let weekly_color = if data.weekly_pct > 50.0 {
            (0, 230, 118)
        } else if data.weekly_pct > 20.0 {
            (255, 179, 0)
        } else {
            (255, 61, 0)
        };
        let weekly_val = format!("{:.0}%", data.weekly_pct);
        draw_meter(
            hdc,
            70,
            45,
            32,
            data.weekly_pct,
            "Weekly",
            &weekly_val,
            weekly_color,
        );

        let sep_pen = CreatePen(PS_SOLID, 1, COLORREF(0x002E2620));
        SelectObject(hdc, HGDIOBJ(sep_pen.0));
        let _ = MoveToEx(hdc, 140, 12, None);
        let _ = LineTo(hdc, 140, 72);
        let _ = DeleteObject(HGDIOBJ(sep_pen.0));

        let five_h_color = if data.five_h_pct > 50.0 {
            (0, 229, 255)
        } else if data.five_h_pct > 20.0 {
            (255, 179, 0)
        } else {
            (255, 61, 0)
        };
        let five_h_val = format!("{:.0}%", data.five_h_pct);
        draw_meter(
            hdc,
            210,
            45,
            32,
            data.five_h_pct,
            "5-Hour",
            &five_h_val,
            five_h_color,
        );
    }
}

impl TachometerOverlay {
    pub fn new() -> Result<Self, String> {
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
                hbrBackground: HBRUSH::default(),
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
                280,
                85,
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
                right: 280,
                bottom: 85,
            };
            let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(self.hwnd), Some(&rect), true);
        }
    }

    pub fn fade_in(&self) {
        TARGET_ALPHA.store(235, Ordering::SeqCst);
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
                280,
                85,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}
