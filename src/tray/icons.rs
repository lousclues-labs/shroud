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

            // For connected state, draw a standalone lock icon (no circle background)
            if icon_type == IconType::Connected {
                let green = [255u8, 46, 160, 67]; // Solid green
                let green_border = [180u8, 30, 120, 50]; // Semi-transparent darker green
                let transparent = [0u8, 0, 0, 0];

                let center = size / 2;
                let s = size as f32 / 32.0;

                for y in 0..size {
                    for x in 0..size {
                        let pixel = draw_standalone_lock(
                            x,
                            y,
                            center,
                            s,
                            &green,
                            &green_border,
                            &transparent,
                        );
                        data.extend_from_slice(&pixel);
                    }
                }
            } else {
                // For other states, use the circle with symbol approach
                let (bg_r, bg_g, bg_b, fg_r, fg_g, fg_b) = match icon_type {
                    IconType::Connected => unreachable!(),
                    IconType::Connecting => (245, 158, 11, 255, 255, 255), // Amber, white
                    IconType::Disconnected => (100, 116, 139, 255, 255, 255), // Slate, white
                    IconType::Degraded => (249, 115, 22, 255, 255, 255),   // Orange, white
                    IconType::Failed => (239, 68, 68, 255, 255, 255),      // Red, white
                };

                let bg_pixel = [255u8, bg_r, bg_g, bg_b];
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
                            let pixel = match icon_type {
                                IconType::Connected => unreachable!(),
                                IconType::Connecting => {
                                    draw_dots(x, y, center, size, &fg_pixel, &bg_pixel)
                                }
                                IconType::Disconnected => {
                                    draw_dash(x, y, center, size, &fg_pixel, &bg_pixel)
                                }
                                IconType::Degraded => {
                                    draw_exclamation(x, y, center, size, &fg_pixel, &bg_pixel)
                                }
                                IconType::Failed => {
                                    draw_x_mark(x, y, center, size, &fg_pixel, &bg_pixel)
                                }
                            };
                            data.extend_from_slice(&pixel);
                        } else {
                            data.extend_from_slice(&transparent);
                        }
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

/// Draw a standalone lock icon (no background circle)
fn draw_standalone_lock(
    x: i32,
    y: i32,
    center: i32,
    s: f32,
    fill: &[u8; 4],
    border: &[u8; 4],
    transparent: &[u8; 4],
) -> [u8; 4] {
    let rx = (x - center) as f32;
    let ry = (y - center) as f32;

    // Scale up - make lock fill most of the icon
    let scale = 1.6;

    // Lock body dimensions (rectangle with slight rounding effect via border)
    let body_half_w = 9.0 * s * scale;
    let body_top = -2.0 * s * scale;
    let body_bottom = 10.0 * s * scale;
    let border_width = 1.2 * s * scale;

    // Shackle dimensions (thick arc at top)
    let shackle_center_y = -2.0 * s * scale;
    let shackle_outer_r = 7.0 * s * scale;
    let shackle_inner_r = 3.5 * s * scale;

    // Check if in body
    let in_body = rx.abs() <= body_half_w && ry >= body_top && ry <= body_bottom;
    let in_body_inner = rx.abs() <= (body_half_w - border_width)
        && ry >= (body_top + border_width)
        && ry <= (body_bottom - border_width);

    // Check if in shackle (thick arc, only top half)
    let dy = ry - shackle_center_y;
    let dist = (rx * rx + dy * dy).sqrt();
    let in_shackle = ry <= body_top && dist <= shackle_outer_r && dist >= shackle_inner_r;
    let in_shackle_inner = ry <= body_top
        && dist <= (shackle_outer_r - border_width * 0.8)
        && dist >= (shackle_inner_r + border_width * 0.8);

    // Keyhole - circle on top, rectangle slot below
    let keyhole_circle_y = 2.5 * s * scale;
    let keyhole_circle_r = 2.0 * s * scale;
    let keyhole_slot_top = keyhole_circle_y;
    let keyhole_slot_bottom = 7.0 * s * scale;
    let keyhole_slot_half_w = 1.2 * s * scale;

    let kdy = ry - keyhole_circle_y;
    let keyhole_dist = (rx * rx + kdy * kdy).sqrt();
    let in_keyhole_circle = keyhole_dist <= keyhole_circle_r;
    let in_keyhole_slot =
        rx.abs() <= keyhole_slot_half_w && ry >= keyhole_slot_top && ry <= keyhole_slot_bottom;
    let in_keyhole = in_keyhole_circle || in_keyhole_slot;

    // Determine pixel color with layering
    if in_body_inner && !in_keyhole {
        *fill
    } else if in_body && !in_keyhole {
        *border // Darker border around body edge
    } else if in_shackle_inner {
        *fill
    } else if in_shackle {
        *border // Darker border around shackle
    } else {
        *transparent
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
