//! Icon fetching and processing functions.
//!
//! Handles _NET_WM_ICON property queries and image scaling.

use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

use crate::ewmh::Atoms;
use crate::render::CachedIcon;

/// Fetch and process window icon from _NET_WM_ICON property.
/// Returns None if window has no icon.
pub fn fetch_icon(
    conn: &impl Connection,
    atoms: &Atoms,
    window: Window,
    target_size: u32,
) -> Option<CachedIcon> {
    // Request a large amount to get all icon sizes
    let reply = conn
        .get_property(
            false,
            window,
            atoms.net_wm_icon,
            AtomEnum::CARDINAL,
            0,
            u32::MAX / 4, // Max reasonable size
        )
        .ok()?
        .reply()
        .ok()?;

    if reply.value.is_empty() || reply.format != 32 {
        return None;
    }

    // Parse as 32-bit values (format depends on byte order)
    let data: Vec<u32> = reply.value32()?.collect();
    if data.len() < 3 {
        return None;
    }

    // Find the best icon size (closest to target_size x target_size)
    let mut best_icon: Option<(u32, u32, &[u32])> = None;
    let mut best_diff = u32::MAX;
    let mut idx = 0;

    while idx + 2 < data.len() {
        let width = data[idx];
        let height = data[idx + 1];
        let pixel_count = (width as usize).saturating_mul(height as usize);

        if width == 0 || height == 0 || idx + 2 + pixel_count > data.len() {
            break;
        }

        let pixels = &data[idx + 2..idx + 2 + pixel_count];

        // Prefer larger icons (better quality when scaling down)
        let size = width.max(height);
        let diff = if size >= target_size {
            size - target_size
        } else {
            (target_size - size) * 2 // Penalize upscaling
        };

        if diff < best_diff || (diff == best_diff && width >= target_size) {
            best_diff = diff;
            best_icon = Some((width, height, pixels));
        }

        idx += 2 + pixel_count;
    }

    let (src_w, src_h, pixels) = best_icon?;

    // Scale to target size using nearest-neighbor
    let scaled = scale_icon(pixels, src_w, src_h, target_size);

    Some(CachedIcon { pixels: scaled })
}

/// Scale ARGB32 icon to target size and convert to BGRA.
pub fn scale_icon(src: &[u32], src_w: u32, src_h: u32, dst_size: u32) -> Vec<u8> {
    let mut dst = vec![0u8; (dst_size * dst_size * 4) as usize];

    for y in 0..dst_size {
        for x in 0..dst_size {
            let src_x = (x * src_w / dst_size).min(src_w - 1) as usize;
            let src_y = (y * src_h / dst_size).min(src_h - 1) as usize;
            let src_idx = src_y * src_w as usize + src_x;

            if src_idx < src.len() {
                let pixel = src[src_idx];
                // _NET_WM_ICON format: 0xAARRGGBB
                let a = ((pixel >> 24) & 0xFF) as u8;
                let r = ((pixel >> 16) & 0xFF) as u8;
                let g = ((pixel >> 8) & 0xFF) as u8;
                let b = (pixel & 0xFF) as u8;

                // X11 expects BGRA (or BGRX for 24-bit)
                let dst_idx = ((y * dst_size + x) * 4) as usize;
                dst[dst_idx] = b;
                dst[dst_idx + 1] = g;
                dst[dst_idx + 2] = r;
                dst[dst_idx + 3] = a;
            }
        }
    }

    dst
}
