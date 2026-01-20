//! System tray icons
//!
//! Generates colored circular icons for different VPN states.
//! Icons are cached at startup to avoid regeneration on every tray update.

use once_cell::sync::Lazy;

/// Icon type for status indication
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IconType {
    Connected,
    Connecting,
    Disconnected,
    Degraded,
    Failed,
}

/// Pre-computed icons for each state (generated once at startup)
pub static ICON_CONNECTED: Lazy<Vec<ksni::Icon>> =
    Lazy::new(|| generate_icon_data(IconType::Connected));
pub static ICON_CONNECTING: Lazy<Vec<ksni::Icon>> =
    Lazy::new(|| generate_icon_data(IconType::Connecting));
pub static ICON_DISCONNECTED: Lazy<Vec<ksni::Icon>> =
    Lazy::new(|| generate_icon_data(IconType::Disconnected));
pub static ICON_DEGRADED: Lazy<Vec<ksni::Icon>> =
    Lazy::new(|| generate_icon_data(IconType::Degraded));
pub static ICON_FAILED: Lazy<Vec<ksni::Icon>> = Lazy::new(|| generate_icon_data(IconType::Failed));

/// Get cached status icons (avoids regenerating on every tray update)
#[inline]
pub fn get_status_icon(icon_type: IconType) -> Vec<ksni::Icon> {
    match icon_type {
        IconType::Connected => ICON_CONNECTED.clone(),
        IconType::Connecting => ICON_CONNECTING.clone(),
        IconType::Disconnected => ICON_DISCONNECTED.clone(),
        IconType::Degraded => ICON_DEGRADED.clone(),
        IconType::Failed => ICON_FAILED.clone(),
    }
}

/// Generate icon data for a specific type
/// Returns icons in common sizes (16x16, 24x24, 32x32, 48x48) for different DPI scales.
/// The data is in ARGB32 format with network byte order (big endian).
fn generate_icon_data(icon_type: IconType) -> Vec<ksni::Icon> {
    const SIZES: [i32; 4] = [16, 24, 32, 48];

    SIZES
        .iter()
        .map(|&size| {
            let mut data = Vec::with_capacity((size * size * 4) as usize);

            // Color palette
            let (bg_r, bg_g, bg_b, fg_r, fg_g, fg_b) = match icon_type {
                IconType::Connected => (46u8, 160, 67, 255, 255, 255),    // Green, white
                IconType::Connecting => (245, 158, 11, 255, 255, 255),    // Amber, white
                IconType::Disconnected => (100, 116, 139, 255, 255, 255), // Slate, white
                IconType::Degraded => (249, 115, 22, 255, 255, 255),      // Orange, white (warning)
                IconType::Failed => (239, 68, 68, 255, 255, 255),         // Red, white
            };

            let bg_pixel = [255u8, bg_r, bg_g, bg_b]; // ARGB: full alpha, then RGB
            let fg_pixel = [255u8, fg_r, fg_g, fg_b];
            let transparent = [0u8, 0, 0, 0];

            let center = size / 2;
            let radius = (size as f32 * 0.42) as i32;
            let radius_sq = radius * radius;

            for y in 0..size {
                for x in 0..size {
                    let dx = x - center;
                    let dy = y - center;
                    let dist_sq = dx * dx + dy * dy;

                    if dist_sq <= radius_sq {
                        // Inside circle - draw the symbol
                        let pixel = match icon_type {
                            IconType::Connected => draw_check(x, y, center, size, &fg_pixel, &bg_pixel),
                            IconType::Connecting => draw_dots(x, y, center, size, &fg_pixel, &bg_pixel),
                            IconType::Disconnected => draw_dash(x, y, center, size, &fg_pixel, &bg_pixel),
                            IconType::Degraded => draw_exclamation(x, y, center, size, &fg_pixel, &bg_pixel),
                            IconType::Failed => draw_x_mark(x, y, center, size, &fg_pixel, &bg_pixel),
                        };
                        data.extend_from_slice(&pixel);
                    } else {
                        data.extend_from_slice(&transparent);
                    }
                }
            }

            ksni::Icon {
                width: size,
                height: size,
                data,
            }
        })
        .collect()
}

/// Draw a checkmark symbol
fn draw_check(x: i32, y: i32, center: i32, size: i32, fg: &[u8; 4], bg: &[u8; 4]) -> [u8; 4] {
    let rx = x - center;
    let ry = y - center;
    let s = size as f32 / 32.0;

    // Two strokes forming a checkmark
    let on_short = rx >= (-5.0 * s) as i32
        && rx <= (-1.0 * s) as i32
        && ry >= (-1.0 * s) as i32
        && ry <= (5.0 * s) as i32
        && ((ry as f32) - (rx as f32 + 3.0 * s) * 1.0).abs() < 2.5 * s;

    let on_long = rx >= (-2.0 * s) as i32
        && rx <= (7.0 * s) as i32
        && ry >= (-6.0 * s) as i32
        && ry <= (4.0 * s) as i32
        && ((ry as f32) + (rx as f32) * 0.7 - 1.0 * s).abs() < 2.5 * s;

    if on_short || on_long {
        *fg
    } else {
        *bg
    }
}

/// Draw three horizontal dots
fn draw_dots(x: i32, y: i32, center: i32, size: i32, fg: &[u8; 4], bg: &[u8; 4]) -> [u8; 4] {
    let rx = x - center;
    let ry = y - center;
    let s = size as f32 / 32.0;
    let dot_r = (2.5 * s) as i32;
    let dot_r_sq = dot_r * dot_r;

    for dot_offset in [-6, 0, 6] {
        let dot_x = (dot_offset as f32 * s) as i32;
        let dx = rx - dot_x;
        if dx * dx + ry * ry <= dot_r_sq {
            return *fg;
        }
    }
    *bg
}

/// Draw a horizontal dash
fn draw_dash(x: i32, y: i32, center: i32, size: i32, fg: &[u8; 4], bg: &[u8; 4]) -> [u8; 4] {
    let rx = x - center;
    let ry = y - center;
    let s = size as f32 / 32.0;

    let half_w = (8.0 * s) as i32;
    let half_h = (2.5 * s) as i32;

    if rx.abs() <= half_w && ry.abs() <= half_h {
        *fg
    } else {
        *bg
    }
}

/// Draw an X mark
fn draw_x_mark(x: i32, y: i32, center: i32, size: i32, fg: &[u8; 4], bg: &[u8; 4]) -> [u8; 4] {
    let rx = x - center;
    let ry = y - center;
    let s = size as f32 / 32.0;
    let thick = (2.5 * s) as i32;
    let arm = (6.0 * s) as i32;

    let on_d1 = (rx - ry).abs() <= thick && rx.abs() <= arm && ry.abs() <= arm;
    let on_d2 = (rx + ry).abs() <= thick && rx.abs() <= arm && ry.abs() <= arm;

    if on_d1 || on_d2 {
        *fg
    } else {
        *bg
    }
}

/// Draw an exclamation mark (for degraded state)
fn draw_exclamation(x: i32, y: i32, center: i32, size: i32, fg: &[u8; 4], bg: &[u8; 4]) -> [u8; 4] {
    let rx = x - center;
    let ry = y - center;
    let s = size as f32 / 32.0;

    // Vertical bar (upper part of exclamation mark)
    let bar_w = (2.5 * s) as i32;
    let bar_top = (-6.0 * s) as i32;
    let bar_bottom = (2.0 * s) as i32;
    let on_bar = rx.abs() <= bar_w && ry >= bar_top && ry <= bar_bottom;

    // Dot at the bottom
    let dot_y = (5.0 * s) as i32;
    let dot_r = (2.0 * s) as i32;
    let dot_r_sq = dot_r * dot_r;
    let dy = ry - dot_y;
    let on_dot = rx * rx + dy * dy <= dot_r_sq;

    if on_bar || on_dot {
        *fg
    } else {
        *bg
    }
}
