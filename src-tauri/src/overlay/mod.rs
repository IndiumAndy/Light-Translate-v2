use crate::extraction::resolver::OverlayBlockSeed;
use crate::extraction::ScreenRect;
use serde::{Deserialize, Serialize};

/// Lossless RGBA8 screen capture buffer paired with its logical screen coordinates.
#[derive(Debug, Clone)]
pub struct CapturedImage {
    pub rect: ScreenRect,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub rgba: Vec<u8>,
    /// Optional pre-encoded JPEG preview for zero-latency fullscreen freeze-frame display.
    pub preview_jpeg: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OverlayToolbarPlacement {
    Above,
    Below,
    InsideTop,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayWindowLayout {
    pub window_rect: ScreenRect,
    pub image_offset_x: f64,
    pub image_offset_y: f64,
    pub image_width: f64,
    pub image_height: f64,
    pub toolbar_placement: OverlayToolbarPlacement,
}

pub const OVERLAY_TOOLBAR_SPACE: f64 = 42.0;
pub const MIN_OVERLAY_WINDOW_WIDTH: f64 = 312.0;

/// Computes the exact borderless window frame so that the captured region image sits at
/// `(window_rect.x + image_offset_x, window_rect.y + image_offset_y) == (selection.x, selection.y)`
/// pixel-for-pixel on top of the original screen content, while guaranteeing enough room for the
/// floating control pill toolbar without clipping on narrow boxes or screen edges.
pub fn compute_overlay_window_layout(
    selection: ScreenRect,
    monitor: ScreenRect,
) -> OverlayWindowLayout {
    let margin = 8.0;

    let win_width = selection.width.max(MIN_OVERLAY_WINDOW_WIDTH);
    let win_x = if selection.width >= MIN_OVERLAY_WINDOW_WIDTH {
        selection.x
    } else {
        // Keep image anchored at `selection.x` while shifting wider window frame inside monitor bounds
        let max_win_x = (monitor.max_x() - margin - win_width).max(monitor.min_x() + margin);
        let ideal = selection.x;
        let clamped = ideal.min(max_win_x).max(monitor.min_x() + margin);
        // Ensure `selection` still lies inside `[win_x, win_x + win_width]`
        clamped
            .min(selection.x)
            .max(selection.max_x() - win_width)
    };
    let image_offset_x = (selection.x - win_x).max(0.0);

    let can_fit_above = selection.y - OVERLAY_TOOLBAR_SPACE >= monitor.min_y() + margin;
    let can_fit_below = selection.max_y() + OVERLAY_TOOLBAR_SPACE <= monitor.max_y() - margin;

    let (win_y, win_height, image_offset_y, toolbar_placement) = if can_fit_above {
        (
            selection.y - OVERLAY_TOOLBAR_SPACE,
            selection.height + OVERLAY_TOOLBAR_SPACE,
            OVERLAY_TOOLBAR_SPACE,
            OverlayToolbarPlacement::Above,
        )
    } else if can_fit_below {
        (
            selection.y,
            selection.height + OVERLAY_TOOLBAR_SPACE,
            0.0,
            OverlayToolbarPlacement::Below,
        )
    } else {
        (
            selection.y,
            selection.height,
            0.0,
            OverlayToolbarPlacement::InsideTop,
        )
    };

    OverlayWindowLayout {
        window_rect: ScreenRect::new(win_x, win_y, win_width, win_height),
        image_offset_x,
        image_offset_y,
        image_width: selection.width,
        image_height: selection.height,
        toolbar_placement,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayBlockViewState {
    pub id: usize,
    pub source_text: String,
    pub translation: String,
    pub is_loading: bool,
    pub rect: ScreenRect,
    pub line_rects: Vec<ScreenRect>,
    pub line_count: usize,
    pub estimated_font_px: f64,
    pub bg_color: String,
    pub text_color: String,
    pub is_dark_bg: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureOverlayViewState {
    pub visible: bool,
    pub generation: u64,
    pub pinned: bool,
    pub is_refining: bool,
    pub error: Option<String>,
    pub image_offset_x: f64,
    pub image_offset_y: f64,
    pub image_width: f64,
    pub image_height: f64,
    pub toolbar_placement: OverlayToolbarPlacement,
    pub blocks: Vec<OverlayBlockViewState>,
}

impl Default for CaptureOverlayViewState {
    fn default() -> Self {
        Self {
            visible: false,
            generation: 0,
            pinned: false,
            is_refining: false,
            error: None,
            image_offset_x: 0.0,
            image_offset_y: 0.0,
            image_width: 0.0,
            image_height: 0.0,
            toolbar_placement: OverlayToolbarPlacement::Above,
            blocks: Vec::new(),
        }
    }
}

pub fn build_initial_overlay_blocks(
    seeds: Vec<OverlayBlockSeed>,
    cropped_image: Option<&CapturedImage>,
) -> Vec<OverlayBlockViewState> {
    seeds
        .into_iter()
        .enumerate()
        .map(|(id, seed)| {
            let (bg_color, text_color, is_dark_bg) = match cropped_image {
                Some(img) => sample_block_appearance(img, seed.rect, &seed.line_rects),
                None => ("#FFFFFF".into(), "#111318".into(), false),
            };
            OverlayBlockViewState {
                id,
                source_text: seed.source_text,
                translation: String::new(),
                is_loading: true,
                rect: seed.rect,
                line_rects: seed.line_rects,
                line_count: seed.line_count,
                estimated_font_px: seed.estimated_font_px,
                bg_color,
                text_color,
                is_dark_bg,
            }
        })
        .collect()
}

/// Crops `sub_rect` (in logical screen coordinates) from a full-monitor `CapturedImage`.
pub fn crop_captured_image(full: &CapturedImage, sub_rect: ScreenRect) -> Option<CapturedImage> {
    if !full.rect.is_valid()
        || !sub_rect.is_valid()
        || full.pixel_width == 0
        || full.pixel_height == 0
        || full.rgba.len() < (full.pixel_width as usize) * (full.pixel_height as usize) * 4
    {
        return None;
    }

    let scale_x = (full.pixel_width as f64) / full.rect.width;
    let scale_y = (full.pixel_height as f64) / full.rect.height;

    let rel_x0 = (sub_rect.min_x() - full.rect.min_x()).max(0.0);
    let rel_y0 = (sub_rect.min_y() - full.rect.min_y()).max(0.0);
    let rel_x1 = (sub_rect.max_x() - full.rect.min_x()).min(full.rect.width);
    let rel_y1 = (sub_rect.max_y() - full.rect.min_y()).min(full.rect.height);

    if rel_x1 <= rel_x0 || rel_y1 <= rel_y0 {
        return None;
    }

    let px0 = ((rel_x0 * scale_x).round() as u32).min(full.pixel_width.saturating_sub(1));
    let py0 = ((rel_y0 * scale_y).round() as u32).min(full.pixel_height.saturating_sub(1));
    let px1 = ((rel_x1 * scale_x).round() as u32)
        .clamp(px0 + 1, full.pixel_width);
    let py1 = ((rel_y1 * scale_y).round() as u32)
        .clamp(py0 + 1, full.pixel_height);

    let cw = px1 - px0;
    let ch = py1 - py0;
    let src_stride = (full.pixel_width as usize) * 4;
    let dst_stride = (cw as usize) * 4;

    let mut out_rgba = Vec::with_capacity((ch as usize) * dst_stride);
    for row in py0..py1 {
        let start = (row as usize) * src_stride + (px0 as usize) * 4;
        let end = start + dst_stride;
        out_rgba.extend_from_slice(&full.rgba[start..end]);
    }

    let actual_rect = ScreenRect::new(
        full.rect.min_x() + rel_x0,
        full.rect.min_y() + rel_y0,
        rel_x1 - rel_x0,
        rel_y1 - rel_y0,
    );

    Some(CapturedImage {
        rect: actual_rect,
        pixel_width: cw,
        pixel_height: ch,
        rgba: out_rgba,
        preview_jpeg: None,
    })
}

/// Fast lossless PNG encoding (`Compression::Fast` + `FilterType::NoFilter`) for IPC binary transfer.
pub fn encode_rgba_to_png(img: &CapturedImage) -> Result<Vec<u8>, String> {
    if img.pixel_width == 0 || img.pixel_height == 0 {
        return Err("空图像尺寸".into());
    }
    let expected = (img.pixel_width as usize) * (img.pixel_height as usize) * 4;
    if img.rgba.len() < expected {
        return Err("图像像素缓冲区不完整".into());
    }

    let mut buf = Vec::with_capacity(expected / 3);
    {
        let mut encoder = png::Encoder::new(&mut buf, img.pixel_width, img.pixel_height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        encoder.set_filter(png::FilterType::NoFilter);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer
            .write_image_data(&img.rgba[..expected])
            .map_err(|e| e.to_string())?;
    }
    Ok(buf)
}

fn srgb_channel_to_linear(c: u8) -> f64 {
    let v = (c as f64) / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

pub fn relative_luminance(r: u8, g: u8, b: u8) -> f64 {
    0.2126 * srgb_channel_to_linear(r)
        + 0.7152 * srgb_channel_to_linear(g)
        + 0.0722 * srgb_channel_to_linear(b)
}

pub fn contrast_ratio(l1: f64, l2: f64) -> f64 {
    let (lighter, darker) = if l1 >= l2 { (l1, l2) } else { (l2, l1) };
    (lighter + 0.05) / (darker + 0.05)
}

fn rgb_hex(r: u8, g: u8, b: u8) -> String {
    format!("#{r:02X}{g:02X}{b:02X}")
}

/// Samples the perimeter pixels around an OCR block to determine the exact underlying surface
/// background color (via channel-wise median, immune to stray underlines or adjacent icons) and
/// extracts a WCAG >= 5.0:1 contrasting ink color matching the original text's visual tone.
pub fn sample_block_appearance(
    img: &CapturedImage,
    block_local_rect: ScreenRect,
    line_rects: &[ScreenRect],
) -> (String, String, bool) {
    let pw = img.pixel_width as i32;
    let ph = img.pixel_height as i32;
    if pw <= 0
        || ph <= 0
        || !img.rect.is_valid()
        || img.rgba.len() < (pw as usize) * (ph as usize) * 4
    {
        return ("#FFFFFF".into(), "#111318".into(), false);
    }

    let sx = (pw as f64) / img.rect.width.max(1.0);
    let sy = (ph as f64) / img.rect.height.max(1.0);

    let read_px = |x: i32, y: i32| -> (u8, u8, u8) {
        let cx = x.clamp(0, pw - 1) as usize;
        let cy = y.clamp(0, ph - 1) as usize;
        let idx = (cy * (pw as usize) + cx) * 4;
        (img.rgba[idx], img.rgba[idx + 1], img.rgba[idx + 2])
    };

    let targets: &[ScreenRect] = if line_rects.is_empty() {
        std::slice::from_ref(&block_local_rect)
    } else {
        line_rects
    };

    let mut rs = Vec::with_capacity(256);
    let mut gs = Vec::with_capacity(256);
    let mut bs = Vec::with_capacity(256);

    for r in targets {
        let x0 = (r.min_x() * sx).floor() as i32;
        let y0 = (r.min_y() * sy).floor() as i32;
        let x1 = (r.max_x() * sx).ceil() as i32;
        let y1 = (r.max_y() * sy).ceil() as i32;

        let span_x = (x1 - x0).max(1);
        let span_y = (y1 - y0).max(1);
        let step_x = (span_x / 24).max(1);
        let step_y = (span_y / 10).max(1);

        for offset in [2i32, 4i32] {
            let mut x = x0 - offset;
            while x <= x1 + offset {
                let (pr, pg, pb) = read_px(x, y0 - offset);
                rs.push(pr);
                gs.push(pg);
                bs.push(pb);

                let (qr, qg, qb) = read_px(x, y1 + offset);
                rs.push(qr);
                gs.push(qg);
                bs.push(qb);

                x += step_x;
            }
            let mut y = y0;
            while y <= y1 {
                let (pr, pg, pb) = read_px(x0 - offset, y);
                rs.push(pr);
                gs.push(pg);
                bs.push(pb);

                let (qr, qg, qb) = read_px(x1 + offset, y);
                rs.push(qr);
                gs.push(qg);
                bs.push(qb);

                y += step_y;
            }
        }
    }

    if rs.is_empty() {
        return ("#FFFFFF".into(), "#111318".into(), false);
    }

    rs.sort_unstable();
    gs.sort_unstable();
    bs.sort_unstable();
    let mid = rs.len() / 2;
    let (bg_r, bg_g, bg_b) = (rs[mid], gs[mid], bs[mid]);
    let bg_lum = relative_luminance(bg_r, bg_g, bg_b);
    let is_dark_bg = bg_lum < 0.36;

    // Sample interior glyph pixels (top highest-distance pixels from `bg`) to preserve colored ink when legible
    let bx0 = ((block_local_rect.min_x() * sx).floor() as i32).clamp(0, pw - 1);
    let by0 = ((block_local_rect.min_y() * sy).floor() as i32).clamp(0, ph - 1);
    let bx1 = ((block_local_rect.max_x() * sx).ceil() as i32).clamp(bx0 + 1, pw);
    let by1 = ((block_local_rect.max_y() * sy).ceil() as i32).clamp(by0 + 1, ph);

    let step_ix = ((bx1 - bx0) / 28).max(1);
    let step_iy = ((by1 - by0) / 14).max(1);
    let mut interior: Vec<(u32, u8, u8, u8)> = Vec::new();
    let mut iy = by0;
    while iy < by1 {
        let mut ix = bx0;
        while ix < bx1 {
            let (r, g, b) = read_px(ix, iy);
            let dr = (r as i32) - (bg_r as i32);
            let dg = (g as i32) - (bg_g as i32);
            let db = (b as i32) - (bg_b as i32);
            let dist_sq = (dr * dr + dg * dg + db * db) as u32;
            interior.push((dist_sq, r, g, b));
            ix += step_ix;
        }
        iy += step_iy;
    }

    interior.sort_unstable_by(|a, b| b.0.cmp(&a.0));
    let top_n = (interior.len() / 10).clamp(1, 16);
    if interior.len() >= 4 && top_n > 0 {
        let mut ir = Vec::with_capacity(top_n);
        let mut ig = Vec::with_capacity(top_n);
        let mut ib = Vec::with_capacity(top_n);
        for &(_, r, g, b) in interior.iter().take(top_n) {
            ir.push(r);
            ig.push(g);
            ib.push(b);
        }
        ir.sort_unstable();
        ig.sort_unstable();
        ib.sort_unstable();
        let (fg_r, fg_g, fg_b) = (ir[top_n / 2], ig[top_n / 2], ib[top_n / 2]);
        let fg_lum = relative_luminance(fg_r, fg_g, fg_b);
        if contrast_ratio(bg_lum, fg_lum) >= 5.0 {
            return (
                rgb_hex(bg_r, bg_g, bg_b),
                rgb_hex(fg_r, fg_g, fg_b),
                is_dark_bg,
            );
        }
    }

    // High-contrast WCAG fallback ink
    let default_fg = if is_dark_bg {
        "#F8FAFC".to_string()
    } else {
        "#111318".to_string()
    };
    (rgb_hex(bg_r, bg_g, bg_b), default_fg, is_dark_bg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_window_layout_preserves_exact_screen_alignment_in_all_positions() {
        let monitor = ScreenRect::new(0.0, 0.0, 1440.0, 900.0);

        // 1. Normal mid-screen paragraph
        let sel_mid = ScreenRect::new(300.0, 240.0, 480.0, 160.0);
        let l1 = compute_overlay_window_layout(sel_mid, monitor);
        assert_eq!(l1.toolbar_placement, OverlayToolbarPlacement::Above);
        assert!((l1.window_rect.x + l1.image_offset_x - sel_mid.x).abs() < 1e-6);
        assert!((l1.window_rect.y + l1.image_offset_y - sel_mid.y).abs() < 1e-6);

        // 2. Narrow button near right edge of monitor (`width = 70 < MIN_OVERLAY_WINDOW_WIDTH`)
        let sel_right = ScreenRect::new(1350.0, 200.0, 70.0, 32.0);
        let l2 = compute_overlay_window_layout(sel_right, monitor);
        assert!(l2.window_rect.width >= MIN_OVERLAY_WINDOW_WIDTH);
        assert!(l2.window_rect.max_x() <= monitor.max_x());
        assert!((l2.window_rect.x + l2.image_offset_x - sel_right.x).abs() < 1e-6);
        assert!((l2.window_rect.y + l2.image_offset_y - sel_right.y).abs() < 1e-6);

        // 3. Top-edge selection where toolbar cannot fit above -> flips below cleanly
        let sel_top = ScreenRect::new(200.0, 12.0, 400.0, 120.0);
        let l3 = compute_overlay_window_layout(sel_top, monitor);
        assert_eq!(l3.toolbar_placement, OverlayToolbarPlacement::Below);
        assert!((l3.window_rect.x + l3.image_offset_x - sel_top.x).abs() < 1e-6);
        assert!((l3.window_rect.y + l3.image_offset_y - sel_top.y).abs() < 1e-6);
    }

    #[test]
    fn color_sampler_detects_dark_and_light_backgrounds_and_enforces_wcag_contrast() {
        // Build a synthetic 80x40 dark IDE region (#181A20) with bright sky-blue glyphs (#56B6C2) in the center
        let pw = 80u32;
        let ph = 40u32;
        let mut rgba = vec![0u8; (pw * ph * 4) as usize];
        for y in 0..ph {
            for x in 0..pw {
                let idx = ((y * pw + x) * 4) as usize;
                let is_glyph = (20..60).contains(&x) && (14..26).contains(&y) && (x + y) % 3 == 0;
                if is_glyph {
                    rgba[idx] = 0x56;
                    rgba[idx + 1] = 0xB6;
                    rgba[idx + 2] = 0xC2;
                } else {
                    rgba[idx] = 0x18;
                    rgba[idx + 1] = 0x1A;
                    rgba[idx + 2] = 0x20;
                }
                rgba[idx + 3] = 255;
            }
        }
        let img = CapturedImage {
            rect: ScreenRect::new(0.0, 0.0, 80.0, 40.0),
            pixel_width: pw,
            pixel_height: ph,
            rgba,
            preview_jpeg: None,
        };
        let block = ScreenRect::new(18.0, 12.0, 44.0, 16.0);
        let (bg, fg, is_dark) = sample_block_appearance(&img, block, &[block]);
        assert_eq!(bg, "#181A20");
        assert!(is_dark);
        assert_eq!(fg, "#56B6C2");

        // Also test cropping + PNG encoding roundtrip
        let cropped = crop_captured_image(&img, ScreenRect::new(10.0, 5.0, 40.0, 20.0)).unwrap();
        assert_eq!(cropped.pixel_width, 40);
        assert_eq!(cropped.pixel_height, 20);
        let png_bytes = encode_rgba_to_png(&cropped).unwrap();
        assert!(png_bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    }
}
