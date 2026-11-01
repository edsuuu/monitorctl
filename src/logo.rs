// App icon, drawn with signed distance fields so it stays crisp at every size (16 px to 256 px).
// Shared by the window icon (gui/mod.rs) and the .exe icon resource (build.rs, via include!).
// Design space: 256 x 256 units. A dark tile, a teal monitor and the ◐ brightness/contrast mark.

/// Straight (not premultiplied) RGBA, 0.0..=1.0.
type Color = [f32; 4];

fn color(rgb: u32) -> Color {
    let channel = |shift: u32| ((rgb >> shift) & 0xFF) as f32 / 255.0;
    [channel(16), channel(8), channel(0), 1.0]
}

fn mix(top: Color, bottom: Color, amount: f32) -> Color {
    let amount = amount.clamp(0.0, 1.0);
    [0, 1, 2, 3].map(|channel| top[channel] + (bottom[channel] - top[channel]) * amount)
}

/// Signed distance to a rounded rectangle (negative inside).
fn rounded_box(point_x: f32, point_y: f32, center: (f32, f32), half_size: (f32, f32), radius: f32) -> f32 {
    let outside_x = (point_x - center.0).abs() - half_size.0 + radius;
    let outside_y = (point_y - center.1).abs() - half_size.1 + radius;
    let corner = outside_x.max(0.0).hypot(outside_y.max(0.0));
    corner + outside_x.max(outside_y).min(0.0) - radius
}

fn circle(point_x: f32, point_y: f32, center: (f32, f32), radius: f32) -> f32 {
    (point_x - center.0).hypot(point_y - center.1) - radius
}

/// The icon as RGBA bytes, `size` x `size`.
pub fn pixels(size: u32) -> Vec<u8> {
    let units_per_pixel = 256.0 / size as f32;
    let tile_top = color(0x232C3A);
    let tile_bottom = color(0x0E1116);
    let teal_top = color(0x6CEBDD);
    let teal_bottom = color(0x27A497);
    let screen_top = color(0x141922);
    let screen_bottom = color(0x07090C);
    let mut image = Vec::with_capacity((size * size * 4) as usize);
    for row in 0..size {
        for column in 0..size {
            let point_x = (column as f32 + 0.5) * units_per_pixel;
            let point_y = (row as f32 + 0.5) * units_per_pixel;
            // (distance, color) from back to front
            let mark_center = (128.0, 110.0);
            let layers: [(f32, Color); 7] = [
                (rounded_box(point_x, point_y, (128.0, 128.0), (120.0, 120.0), 56.0), mix(tile_top, tile_bottom, point_y / 256.0)),
                (rounded_box(point_x, point_y, (128.0, 110.0), (88.0, 62.0), 18.0), mix(teal_top, teal_bottom, (point_y - 48.0) / 124.0)),
                (rounded_box(point_x, point_y, (128.0, 110.0), (74.0, 48.0), 8.0), mix(screen_top, screen_bottom, (point_y - 62.0) / 96.0)),
                (rounded_box(point_x, point_y, (128.0, 186.0), (10.0, 16.0), 3.0), teal_bottom),
                (rounded_box(point_x, point_y, (128.0, 204.0), (42.0, 8.0), 8.0), mix(teal_top, teal_bottom, 0.8)),
                // ◐: a ring plus its left half filled
                ((circle(point_x, point_y, mark_center, 26.0) + 4.0).abs() - 4.0, teal_top),
                (circle(point_x, point_y, mark_center, 26.0).max(point_x - mark_center.0), teal_top),
            ];
            // premultiplied "over" compositing, 1-pixel antialiasing from the distance
            let mut pixel = [0.0f32; 4];
            for (distance, layer_color) in layers {
                let coverage = (0.5 - distance / units_per_pixel).clamp(0.0, 1.0) * layer_color[3];
                for channel in 0..3 {
                    pixel[channel] = layer_color[channel] * coverage + pixel[channel] * (1.0 - coverage);
                }
                pixel[3] = coverage + pixel[3] * (1.0 - coverage);
            }
            let alpha = pixel[3];
            let straight = |channel: usize| if alpha > 0.0 { pixel[channel] / alpha } else { 0.0 };
            image.extend([straight(0), straight(1), straight(2), alpha].map(|value| (value * 255.0).round() as u8));
        }
    }
    image
}

/// A Windows .ico with one 32-bit bitmap per size.
#[allow(dead_code)] // used by build.rs
pub fn ico(sizes: &[u32]) -> Vec<u8> {
    let bitmaps: Vec<Vec<u8>> = sizes.iter().map(|&size| bitmap(size)).collect();
    let mut file = vec![0, 0, 1, 0];
    file.extend((sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len();
    for (&size, bitmap) in sizes.iter().zip(&bitmaps) {
        let side = if size >= 256 { 0 } else { size as u8 }; // 0 means 256
        file.extend([side, side, 0, 0]);
        file.extend(1u16.to_le_bytes()); // color planes
        file.extend(32u16.to_le_bytes()); // bits per pixel
        file.extend((bitmap.len() as u32).to_le_bytes());
        file.extend((offset as u32).to_le_bytes());
        offset += bitmap.len();
    }
    for bitmap in bitmaps {
        file.extend(bitmap);
    }
    file
}

/// One .ico entry: BITMAPINFOHEADER, bottom-up BGRA rows, then an empty AND mask (alpha does the work).
#[allow(dead_code)]
fn bitmap(size: u32) -> Vec<u8> {
    let rgba = pixels(size);
    let mut bitmap = Vec::new();
    for value in [40, size, size * 2] {
        bitmap.extend(value.to_le_bytes()); // header size, width, height of color + mask
    }
    bitmap.extend(1u16.to_le_bytes());
    bitmap.extend(32u16.to_le_bytes());
    bitmap.extend([0u8; 24]); // no compression, default resolution and palette
    for row in (0..size).rev() {
        for column in 0..size {
            let index = ((row * size + column) * 4) as usize;
            bitmap.extend([rgba[index + 2], rgba[index + 1], rgba[index], rgba[index + 3]]);
        }
    }
    let mask_row_bytes = size.div_ceil(32) * 4;
    bitmap.extend(vec![0u8; (mask_row_bytes * size) as usize]);
    bitmap
}
