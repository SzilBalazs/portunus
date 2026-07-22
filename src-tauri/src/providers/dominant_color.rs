//! Sample a single dominant `#rrggbb` from a result's icon or album art for the
//! accent-bleed look. Raster images decode via the `image` crate; SVGs (many
//! Linux app icons, e.g. LibreOffice) rasterize via resvg (pure-Rust, no system
//! deps). The picker builds a coarse hue histogram weighted by saturation ×
//! coverage, takes the most vibrant bucket, then normalizes lightness into a
//! readable mid band so near-black art still yields a visible accent.

use std::path::Path;

/// Longest-edge pixel budget for the downscaled sampling buffer.
const TARGET_EDGE: u32 = 48;
/// Hue histogram resolution (30° per bucket).
const BUCKETS: usize = 12;

/// Extract a dominant color from an image file. SVGs are rasterized; every other
/// format goes through `image`. None on decode failure or an image with no
/// usable chroma (fully transparent / grayscale).
pub fn extract_from_path(path: &Path) -> Option<String> {
    let is_svg = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("svg"));
    let bytes = std::fs::read(path).ok()?;
    if is_svg || looks_like_svg(&bytes) {
        return extract_svg(&bytes);
    }
    extract_from_bytes(&bytes)
}

/// Extract a dominant color from in-memory image bytes (PNG/JPEG/GIF/WebP…).
/// SVG source bytes are also accepted and routed to the rasterizer.
pub fn extract_from_bytes(bytes: &[u8]) -> Option<String> {
    if looks_like_svg(bytes) {
        return extract_svg(bytes);
    }
    let img = image::load_from_memory(bytes).ok()?;
    let thumb = img.thumbnail(TARGET_EDGE, TARGET_EDGE).to_rgba8();
    dominant(thumb.pixels().map(|p| p.0))
}

/// Rasterize an SVG to a small RGBA buffer and feed the same histogram. Returns
/// None if usvg can't parse the document (feature still works for PNG icons).
fn extract_svg(bytes: &[u8]) -> Option<String> {
    let opt = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_data(bytes, &opt).ok()?;
    let size = tree.size();
    let (w, h) = (size.width(), size.height());
    if !(w > 0.0) || !(h > 0.0) {
        return None;
    }
    let scale = TARGET_EDGE as f32 / w.max(h);
    let pw = ((w * scale).ceil() as u32).max(1);
    let ph = ((h * scale).ceil() as u32).max(1);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(pw, ph)?;
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    // tiny-skia stores premultiplied RGBA; demultiply so partially-transparent
    // edge pixels report their true color.
    dominant(pixmap.pixels().iter().map(|p| {
        let c = p.demultiply();
        [c.red(), c.green(), c.blue(), c.alpha()]
    }))
}

/// Coarse hue-histogram picker. Skips transparent, near-black, near-white and
/// achromatic pixels; buckets the rest by hue weighted by saturation (so the
/// winner is saturation × coverage); then remaps the winner's lightness into a
/// readable band. Returns `#rrggbb`, or None when nothing usable remains.
fn dominant<I: Iterator<Item = [u8; 4]>>(pixels: I) -> Option<String> {
    let mut weight = [0f64; BUCKETS];
    let mut acc_h = [0f64; BUCKETS];
    let mut acc_s = [0f64; BUCKETS];
    let mut acc_v = [0f64; BUCKETS];

    for [r, g, b, a] in pixels {
        if a < 16 {
            continue; // near-transparent
        }
        let (h, s, v) = rgb_to_hsv(r, g, b);
        if v < 0.10 {
            continue; // near-black
        }
        if v > 0.95 && s < 0.10 {
            continue; // near-white
        }
        if s < 0.08 {
            continue; // achromatic
        }
        let bucket = (((h / 360.0) * BUCKETS as f32) as usize).min(BUCKETS - 1);
        let w = s as f64;
        weight[bucket] += w;
        acc_h[bucket] += h as f64 * w;
        acc_s[bucket] += s as f64 * w;
        acc_v[bucket] += v as f64 * w;
    }

    let best = (0..BUCKETS).max_by(|&a, &b| weight[a].partial_cmp(&weight[b]).unwrap())?;
    if weight[best] <= 0.0 {
        return None;
    }
    let h = (acc_h[best] / weight[best]) as f32;
    let s = (acc_s[best] / weight[best]) as f32;
    let v = (acc_v[best] / weight[best]) as f32;
    // Normalize lightness into a readable mid band; keep the hue, lift dark art.
    let s = s.max(0.35);
    let v = v.clamp(0.55, 0.90);
    let (r, g, b) = hsv_to_rgb(h, s, v);
    Some(format!("#{r:02x}{g:02x}{b:02x}"))
}

/// True when the head of `bytes` looks like an SVG document.
fn looks_like_svg(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(512)];
    let s = String::from_utf8_lossy(head);
    let s = s.trim_start();
    s.starts_with("<svg") || (s.starts_with("<?xml") && s.contains("<svg")) || s.starts_with("<!--")
}

/// r,g,b in 0..=255 → (hue 0..360, saturation 0..1, value 0..1).
fn rgb_to_hsv(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let r = r as f32 / 255.0;
    let g = g as f32 / 255.0;
    let b = b as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / d) % 6.0)
    } else if max == g {
        60.0 * (((b - r) / d) + 2.0)
    } else {
        60.0 * (((r - g) / d) + 4.0)
    };
    let h = if h < 0.0 { h + 360.0 } else { h };
    let s = if max == 0.0 { 0.0 } else { d / max };
    (h, s, max)
}

/// hue 0..360, saturation 0..1, value 0..1 → r,g,b in 0..=255.
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h {
        h if h < 60.0 => (c, x, 0.0),
        h if h < 120.0 => (x, c, 0.0),
        h if h < 180.0 => (0.0, c, x),
        h if h < 240.0 => (0.0, x, c),
        h if h < 300.0 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (
        ((r + m) * 255.0).round() as u8,
        ((g + m) * 255.0).round() as u8,
        ((b + m) * 255.0).round() as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode a solid-color RGBA buffer to PNG in memory.
    fn solid_png(rgba: [u8; 4]) -> Vec<u8> {
        let mut img = image::RgbaImage::new(32, 32);
        for px in img.pixels_mut() {
            *px = image::Rgba(rgba);
        }
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .unwrap();
        buf
    }

    fn parse_hex(hex: &str) -> (u8, u8, u8) {
        assert!(hex.starts_with('#') && hex.len() == 7, "bad hex {hex}");
        (
            u8::from_str_radix(&hex[1..3], 16).unwrap(),
            u8::from_str_radix(&hex[3..5], 16).unwrap(),
            u8::from_str_radix(&hex[5..7], 16).unwrap(),
        )
    }

    #[test]
    fn solid_blue_is_blueish() {
        let hex = extract_from_bytes(&solid_png([0, 0, 255, 255])).expect("blue color");
        let (r, g, b) = parse_hex(&hex);
        assert!(b > r && b > g && b > 100, "expected blue-ish, got {hex}");
    }

    #[test]
    fn near_black_blue_still_visible() {
        // Very dark blue art must still surface a visible (mid-band) blue.
        let hex = extract_from_bytes(&solid_png([0, 0, 40, 255])).expect("dark blue color");
        let (r, g, b) = parse_hex(&hex);
        assert!(b > r && b > g && b > 120, "expected lifted blue, got {hex}");
    }

    #[test]
    fn transparent_yields_none() {
        assert!(extract_from_bytes(&solid_png([0, 0, 255, 0])).is_none());
    }

    #[test]
    fn grayscale_yields_none() {
        assert!(extract_from_bytes(&solid_png([128, 128, 128, 255])).is_none());
    }
}
