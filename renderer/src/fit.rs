//! A typeset formula placed on a canvas of whole terminal cells, so the
//! terminal never rescales it and its baseline sits on the text baseline of
//! one cell row. Ported from the Python backend's `fit()`, number for number:
//! the same row search, the same rounding (Python rounds half to even).

use base64::Engine;
use fast_image_resize::images::Image;
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};

use crate::protocol::{Cell, ItemResult, MAX_BOX, MIN_SCALE, Options};

pub const SUPERSAMPLE: u32 = 3;
const PAD: f64 = 2.0; // transparent pixels left and right of a formula
const MAX_PNG: usize = 2 * 1024 * 1024;
pub const MAX_PIXELS: u64 = 32_000_000; // largest picture an engine may hand over

/// Ink in straight-alpha RGBA8, `depth` pixels of it below the baseline.
#[derive(Debug, Clone)]
pub struct Ink {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub depth: i64,
}

fn round_half_even(x: f64) -> f64 {
    x.round_ties_even()
}

/// The box around the ink: (left, top, right, bottom), right and bottom exclusive.
fn ink_box(ink: &Ink) -> Option<(usize, usize, usize, usize)> {
    let (w, h) = (ink.width as usize, ink.height as usize);
    let (mut left, mut top, mut right, mut bottom) = (usize::MAX, usize::MAX, 0usize, 0usize);
    for y in 0..h {
        for x in 0..w {
            if ink.rgba[(y * w + x) * 4 + 3] != 0 {
                left = left.min(x);
                right = right.max(x + 1);
                top = top.min(y);
                bottom = bottom.max(y + 1);
            }
        }
    }
    (left != usize::MAX).then_some((left, top, right, bottom))
}

/// The transparent border cut off, keeping the baseline inside.
pub fn crop_to_ink(ink: &Ink) -> Ink {
    let w = ink.width as usize;
    let h = ink.height as usize;
    let Some((left, top, right, bottom)) = ink_box(ink) else {
        return ink.clone();
    };
    let base_row = ink.height as i64 - ink.depth;
    let top = (top as i64).min(base_row - 1);
    let bottom = (bottom as i64).max(base_row);
    let (cw, ch) = (right - left, (bottom - top) as usize);
    let mut rgba = vec![0u8; cw * ch * 4];
    for y in 0..ch {
        let sy = top + y as i64;
        if sy < 0 || sy >= h as i64 {
            continue;
        }
        let src = (sy as usize * w + left) * 4;
        rgba[y * cw * 4..(y + 1) * cw * 4].copy_from_slice(&ink.rgba[src..src + cw * 4]);
    }

    Ink {
        rgba,
        width: cw as u32,
        height: ch as u32,
        depth: bottom - base_row,
    }
}

/// Alpha a becomes 1 - (1 - a)^power: TeX's thin strokes, antialiased to a
/// pixel or less, read as grey beside a terminal font; this firms them up
/// without moving any edge.
fn thicken(rgba: &mut [u8], power: f64) {
    if power == 1.0 {
        return;
    }
    let table: Vec<u8> = (0..256)
        .map(|a| round_half_even(255.0 * (1.0 - (1.0 - f64::from(a) / 255.0).powf(power))) as u8)
        .collect();
    for px in rgba.as_chunks_mut::<4>().0 {
        px[3] = table[px[3] as usize];
    }
}

fn resize(ink: &Ink, w: u32, h: u32) -> Result<Vec<u8>, String> {
    if (w, h) == (ink.width, ink.height) {
        return Ok(ink.rgba.clone());
    }
    let src =
        Image::from_vec_u8(ink.width, ink.height, ink.rgba.clone(), PixelType::U8x4).map_err(|e| e.to_string())?;
    let mut dst = Image::new(w, h, PixelType::U8x4);
    let options = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3));
    Resizer::new()
        .resize(&src, &mut dst, &options)
        .map_err(|e| e.to_string())?;

    Ok(dst.into_vec())
}

fn encode(rgba: &[u8], w: u32, h: u32) -> Option<String> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(rgba).ok()?;
    }
    (out.len() <= MAX_PNG).then(|| base64::engine::general_purpose::STANDARD.encode(&out))
}

/// The picture for `ink` (drawn at SUPERSAMPLE times the terminal's scale).
pub fn fit(ink: &Ink, display: bool, cfg: &Options, cell: Cell) -> ItemResult {
    fit_at(ink, f64::from(SUPERSAMPLE), display, cfg, cell)
}

/// The picture for `ink` drawn at `ratio` times the terminal's scale (the
/// TeX fallback's raster is SUPERSAMPLE times it but for dpi rounding, or
/// less for a formula too large to draw whole).
pub fn fit_at(ink: &Ink, ratio: f64, display: bool, cfg: &Options, cell: Cell) -> ItemResult {
    let (cw, ch) = (f64::from(cell.width), f64::from(cell.height));
    let baseline = cfg.baseline * ch;
    let ink = crop_to_ink(ink);
    let ascent = (ink.height as f64 - ink.depth as f64) / ratio;
    let descent = (ink.depth as f64 / ratio).max(1e-9);
    let width = f64::from(ink.width) / ratio;
    let max_scale = 1f64
        .min((f64::from(cfg.max_columns) * cw - 2.0 * PAD - 1.0) / width.max(1.0))
        .min((f64::from(MAX_BOX) * ch - 2.0) / (ascent + descent).max(1.0));
    if max_scale < MIN_SCALE {
        return ItemResult::failure("too large");
    }

    // The fewest rows that hold the formula at no less than minShrink of its
    // size (all of it for display), and in those rows the text row that lets
    // it be largest (on a tie, the lower one, as Python's max of tuples).
    let floor = (if display { 1.0 } else { cfg.min_shrink }).min(max_scale);
    let mut found = None;
    for rows in 1..=MAX_BOX {
        let mut best = (f64::NEG_INFINITY, 0u32);
        for r in 0..rows {
            let rf = f64::from(r);
            let scale = max_scale
                .min((rf * ch + baseline - 1.0) / ascent.max(1.0))
                .min((f64::from(rows - 1 - r) * ch + ch - baseline - 1.0) / descent);
            if scale > best.0 || (scale == best.0 && r > best.1) {
                best = (scale, r);
            }
        }
        if best.0 >= floor - 1e-9 {
            found = Some((rows, best.0, best.1));
            break;
        }
    }
    let Some((rows, scale, text_row)) = found else {
        return ItemResult::failure("too large");
    };

    let w = round_half_even(width * scale).max(1.0) as u32;
    let h = round_half_even(f64::from(ink.height) / ratio * scale).max(1.0) as u32;
    let columns = ((f64::from(w) + 2.0 * PAD) / cw).ceil().clamp(1.0, f64::from(MAX_BOX)) as u32;
    let (canvas_w, canvas_h) = (columns * cell.width, rows * cell.height);
    let top = round_half_even(f64::from(text_row) * ch + baseline - ascent * scale) as i64;
    let left = (i64::from(canvas_w) - i64::from(w)).div_euclid(2);
    let mut small = match resize(&ink, w, h) {
        Ok(pixels) => pixels,
        Err(err) => return ItemResult::failure(format!("renderer picture failed: {err}")),
    };
    thicken(&mut small, cfg.thicken);
    let mut canvas = vec![0u8; canvas_w as usize * canvas_h as usize * 4];
    for y in 0..h as i64 {
        let cy = top + y;
        if cy < 0 || cy >= i64::from(canvas_h) {
            continue;
        }
        for x in 0..w as i64 {
            let cx = left + x;
            if cx < 0 || cx >= i64::from(canvas_w) {
                continue;
            }
            let s = ((y * i64::from(w) + x) * 4) as usize;
            let d = ((cy * i64::from(canvas_w) + cx) * 4) as usize;
            canvas[d..d + 4].copy_from_slice(&small[s..s + 4]);
        }
    }
    match encode(&canvas, canvas_w, canvas_h) {
        Some(png) => ItemResult::Picture {
            png,
            columns,
            rows,
            text_row,
            engine: String::new(),
        },
        None => ItemResult::failure("too large"),
    }
}

/// The picture for a block (a TikZ picture) drawn at `ratio` times the
/// terminal's scale: cropped to its ink, scaled down to fit `max_columns`
/// and `max_rows`, centred on a canvas of whole cells. No baseline: it
/// stands alone, as a display formula does.
pub fn fit_block(ink: &Ink, ratio: f64, cfg: &Options, cell: Cell, max_rows: u32) -> ItemResult {
    let (cw, ch) = (f64::from(cell.width), f64::from(cell.height));
    let Some((left, top, right, bottom)) = ink_box(ink) else {
        return ItemResult::failure("the picture is empty");
    };
    let (iw, ih) = (right - left, bottom - top);
    let mut cropped = vec![0u8; iw * ih * 4];
    for y in 0..ih {
        let src = ((top + y) * ink.width as usize + left) * 4;
        cropped[y * iw * 4..(y + 1) * iw * 4].copy_from_slice(&ink.rgba[src..src + iw * 4]);
    }
    let ink = Ink {
        rgba: cropped,
        width: iw as u32,
        height: ih as u32,
        depth: 0,
    };
    let (width, height) = (iw as f64 / ratio, ih as f64 / ratio);
    let scale = 1f64
        .min((f64::from(cfg.max_columns) * cw - 2.0 * PAD - 1.0) / width.max(1.0))
        .min((f64::from(max_rows) * ch - 2.0) / height.max(1.0));
    if scale < MIN_SCALE {
        return ItemResult::failure("too large to fit the reply");
    }
    let w = round_half_even(width * scale).max(1.0) as u32;
    let h = round_half_even(height * scale).max(1.0) as u32;
    let columns = ((f64::from(w) + 2.0 * PAD) / cw).ceil().clamp(1.0, f64::from(MAX_BOX)) as u32;
    let rows = ((f64::from(h) + 2.0) / ch).ceil().clamp(1.0, f64::from(max_rows)) as u32;
    let (canvas_w, canvas_h) = (columns * cell.width, rows * cell.height);
    let mut small = match resize(&ink, w, h) {
        Ok(pixels) => pixels,
        Err(err) => return ItemResult::failure(format!("renderer picture failed: {err}")),
    };
    thicken(&mut small, cfg.thicken);
    let left = (canvas_w.saturating_sub(w) / 2) as usize;
    let top = (canvas_h.saturating_sub(h) / 2) as usize;
    let mut canvas = vec![0u8; canvas_w as usize * canvas_h as usize * 4];
    for y in 0..(h as usize).min(canvas_h as usize - top) {
        let n = (w as usize).min(canvas_w as usize - left) * 4;
        let s = y * w as usize * 4;
        let d = ((top + y) * canvas_w as usize + left) * 4;
        canvas[d..d + n].copy_from_slice(&small[s..s + n]);
    }
    match encode(&canvas, canvas_w, canvas_h) {
        Some(png) => ItemResult::Picture {
            png,
            columns,
            rows,
            text_row: 0,
            engine: String::new(),
        },
        None => ItemResult::failure("too large to pass back"),
    }
}

/// A PNG's width and height from its header, before any decoding.
pub fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    if png.len() < 24 || &png[..8] != b"\x89PNG\r\n\x1a\n" || &png[12..16] != b"IHDR" {
        return None;
    }
    let at = |i: usize| u32::from_be_bytes([png[i], png[i + 1], png[i + 2], png[i + 3]]);
    Some((at(16), at(20)))
}

/// A PNG as straight-alpha RGBA8: (pixels, width, height). One larger than
/// MAX_PIXELS is refused from its header.
pub fn decode(png: &[u8]) -> Result<(Vec<u8>, u32, u32), String> {
    let (w, h) = png_size(png).ok_or("renderer picture failed: not a PNG")?;
    if u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err("too large".into());
    }
    let mut decoder = png::Decoder::new(png);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|e| format!("renderer picture failed: {e}"))?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("renderer picture failed: {e}"))?;
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buf
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err("renderer picture failed: indexed PNG".into()),
    };

    Ok((rgba, info.width, info.height))
}
