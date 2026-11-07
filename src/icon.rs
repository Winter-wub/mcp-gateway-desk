//! The app icon, drawn in code (same mark as the window's top bar: three linked dots on
//! a blue rounded square). `mcp-gateway-desk --write-icon <file.png> [size]` writes it,
//! and the packaging script turns it into the .icns for the .app and .dmg.

fn seg_dist(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    ((p.0 - a.0 - t * dx).powi(2) + (p.1 - a.1 - t * dy).powi(2)).sqrt()
}

/// Signed distance to a rounded square (negative inside).
fn rounded_square(p: (f32, f32), c: f32, half: f32, r: f32) -> f32 {
    let qx = (p.0 - c).abs() - (half - r);
    let qy = (p.1 - c).abs() - (half - r);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - r
}

pub fn render(size: u32) -> image::RgbaImage {
    let s = size as f32;
    let k = s / 1024.0;
    let mut img = image::RgbaImage::new(size, size);
    // macOS icon grid: the shape fills about 824/1024 of the canvas.
    let (c, half, radius) = (s / 2.0, 412.0 * k, 185.0 * k);
    let dots = [(352.0 * k, 610.0 * k), (512.0 * k, 360.0 * k), (672.0 * k, 610.0 * k)];
    for y in 0..size {
        for x in 0..size {
            let p = (x as f32 + 0.5, y as f32 + 0.5);
            let shape = (0.5 - rounded_square(p, c, half, radius)).clamp(0.0, 1.0);
            if shape <= 0.0 {
                continue;
            }
            // Blue, a little lighter at the top.
            let t = ((p.1 - (c - half)) / (2.0 * half)).clamp(0.0, 1.0);
            let top = [0x5b as f32, 0x9b as f32, 0xff as f32];
            let bottom = [0x25 as f32, 0x63 as f32, 0xeb as f32];
            let mut rgb = [0.0f32; 3];
            for i in 0..3 {
                rgb[i] = top[i] + (bottom[i] - top[i]) * t;
            }
            let mut white: f32 = 0.0;
            for d in dots {
                let r = ((p.0 - d.0).powi(2) + (p.1 - d.1).powi(2)).sqrt();
                white = white.max((78.0 * k - r + 0.5).clamp(0.0, 1.0));
            }
            for (i, j) in [(0, 1), (1, 2), (0, 2)] {
                white = white.max((24.0 * k - seg_dist(p, dots[i], dots[j]) + 0.5).clamp(0.0, 1.0));
            }
            for v in rgb.iter_mut() {
                *v += (255.0 - *v) * white;
            }
            img.put_pixel(x, y, image::Rgba([rgb[0] as u8, rgb[1] as u8, rgb[2] as u8, (shape * 255.0) as u8]));
        }
    }
    img
}
