use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleDC, CreateDIBSection,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, HBITMAP, HDC, HGDIOBJ, ReleaseDC, SRCCOPY,
    SelectObject,
};

/// 画面上の特定矩形領域の「視覚的複雑度（エントロピー/エッジ量）」を計測
/// 背景が単色（何もないエディタ余白）ならスコアは極小（0に近い）、
/// ボタン・テキスト・アイコン等のUI要素がある場合は高スコアになる。
fn measure_visual_complexity(
    screen_dc: HDC,
    mem_dc: HDC,
    bits: *const u8,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> f64 {
    unsafe {
        let ok = BitBlt(
            mem_dc,
            0,
            0,
            width as i32,
            height as i32,
            Some(screen_dc),
            x,
            y,
            SRCCOPY,
        );
        if ok.is_err() {
            return 999_999.0;
        }

        let total_pixels = (width * height) as usize;
        let slice = std::slice::from_raw_parts(bits, total_pixels * 4);

        // 4ピクセルおき（サブサンプリング）で高速にエッジと輝度分散を評価
        let step = 2usize;
        let w = width as usize;
        let h = height as usize;

        let mut count = 0usize;
        let mut edge_sum = 0.0f64;

        for r in (0..h - 1).step_by(step) {
            for c in (0..w - 1).step_by(step) {
                let idx = (r * w + c) * 4;
                let b = slice[idx] as f64;
                let g = slice[idx + 1] as f64;
                let r_col = slice[idx + 2] as f64;
                let lum = 0.299 * r_col + 0.587 * g + 0.114 * b;

                // 右隣ピクセルとの輝度差
                let idx_right = (r * w + (c + 1)) * 4;
                let b_r = slice[idx_right] as f64;
                let g_r = slice[idx_right + 1] as f64;
                let r_r = slice[idx_right + 2] as f64;
                let lum_right = 0.299 * r_r + 0.587 * g_r + 0.114 * b_r;

                // 下隣ピクセルとの輝度差
                let idx_down = ((r + 1) * w + c) * 4;
                let b_d = slice[idx_down] as f64;
                let g_d = slice[idx_down + 1] as f64;
                let r_d = slice[idx_down + 2] as f64;
                let lum_down = 0.299 * r_d + 0.587 * g_d + 0.114 * b_d;

                let dx = (lum - lum_right).abs();
                let dy = (lum - lum_down).abs();

                edge_sum += dx + dy;
                count += 1;
            }
        }

        if count == 0 {
            0.0
        } else {
            edge_sum / count as f64
        }
    }
}

/// HUDの背後にUI要素（ボタン・テキスト・入力欄等）が被らないよう、
/// 候補位置（最下部、中段、上段）の中から最も視覚的複雑度が低い（＝空いている）Y座標を選択する。
pub fn select_best_overlay_y(
    screen_x: i32,
    base_y: i32,
    window_top: i32,
    width: u32,
    height: u32,
) -> i32 {
    let margin_step = (height as i32) + 16;
    let min_y = window_top + 45; // ウィンドウ上部のタイトルバー等を避ける最小Y

    // 候補スロット（最下部、1段上、2段上）
    let mut candidates = Vec::new();
    candidates.push(base_y);

    let pos1 = base_y - margin_step;
    if pos1 >= min_y {
        candidates.push(pos1);
    }

    let pos2 = base_y - margin_step * 2;
    if pos2 >= min_y {
        candidates.push(pos2);
    }

    if candidates.len() == 1 {
        return base_y;
    }

    unsafe {
        let screen_dc = GetDC(None);
        let mem_dc = CreateCompatibleDC(Some(screen_dc));

        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let hbitmap: HBITMAP =
            match CreateDIBSection(Some(mem_dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
                Ok(h) => h,
                Err(_) => {
                    let _ = DeleteDC(mem_dc);
                    let _ = ReleaseDC(None, screen_dc);
                    return base_y;
                }
            };

        let old_bmp = SelectObject(mem_dc, HGDIOBJ(hbitmap.0));

        let mut best_y = base_y;
        let mut min_score = f64::MAX;
        let mut default_score = 0.0;

        for (i, &cand_y) in candidates.iter().enumerate() {
            let score = measure_visual_complexity(
                screen_dc,
                mem_dc,
                bits as *const u8,
                screen_x,
                cand_y,
                width,
                height,
            );

            if i == 0 {
                default_score = score;
            }

            if score < min_score {
                min_score = score;
                best_y = cand_y;
            }
        }

        SelectObject(mem_dc, old_bmp);
        let _ = DeleteObject(HGDIOBJ(hbitmap.0));
        let _ = DeleteDC(mem_dc);
        let _ = ReleaseDC(None, screen_dc);

        // スロット0（デフォルト最下部）の複雑度が十分に低い（例: 平均エッジ強度 < 4.5）なら、
        // わずかな差で動かさずデフォルト位置を優先（安定性を保つ）
        if default_score <= 4.5 {
            return base_y;
        }

        // デフォルト位置にUI要素があり、より空いているスロットがあればそちらを採用
        best_y
    }
}
