//! Tab bar drawing primitives.
//!
//! Low-level X11 drawing functions for tab bar UI elements.

use anyhow::Result;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

/// Draw a filled rectangle with rounded top corners.
///
/// Uses X11 arcs to create smooth quarter-circle corners at the top-left
/// and top-right, with the bottom edge remaining square.
pub fn draw_rounded_top_rect(
    conn: &impl Connection,
    gc: Gcontext,
    drawable: Drawable,
    x: i16,
    y: i16,
    width: u32,
    height: u32,
    radius: u32,
) -> Result<()> {
    let r = radius.min(width / 2).min(height / 2) as i16;
    let w = width as i16;
    let h = height as i16;

    // Draw the main body (below the rounded corners)
    conn.poly_fill_rectangle(
        drawable,
        gc,
        &[Rectangle {
            x,
            y: y + r,
            width: width as u16,
            height: (h - r) as u16,
        }],
    )?;

    // Draw the top middle section (between the two corners)
    if w > 2 * r {
        conn.poly_fill_rectangle(
            drawable,
            gc,
            &[Rectangle {
                x: x + r,
                y,
                width: (w - 2 * r) as u16,
                height: r as u16,
            }],
        )?;
    }

    // Draw top-left corner arc (quarter circle)
    // Arc angles are in 1/64th of a degree, starting from 3 o'clock going counterclockwise
    // Top-left: start at 90°, sweep 90° counterclockwise
    conn.poly_fill_arc(
        drawable,
        gc,
        &[Arc {
            x,
            y,
            width: (2 * r) as u16,
            height: (2 * r) as u16,
            angle1: 90 * 64,  // Start at 12 o'clock
            angle2: 90 * 64,  // Sweep 90° counterclockwise to 9 o'clock
        }],
    )?;

    // Draw top-right corner arc
    // Top-right: start at 0°, sweep 90° counterclockwise
    conn.poly_fill_arc(
        drawable,
        gc,
        &[Arc {
            x: x + w - 2 * r,
            y,
            width: (2 * r) as u16,
            height: (2 * r) as u16,
            angle1: 0,        // Start at 3 o'clock
            angle2: 90 * 64,  // Sweep 90° counterclockwise to 12 o'clock
        }],
    )?;

    Ok(())
}

/// Draw a filled rectangle with rounded corners on the left side only.
///
/// Used for vertical tabs (left edge of frame). Creates rounded corners
/// at top-left and bottom-left, with the right edge remaining square.
pub fn draw_rounded_left_rect(
    conn: &impl Connection,
    gc: Gcontext,
    drawable: Drawable,
    x: i16,
    y: i16,
    width: u32,
    height: u32,
    radius: u32,
) -> Result<()> {
    let r = radius.min(width / 2).min(height / 2) as i16;
    let w = width as i16;
    let h = height as i16;

    // Draw the main body (to the right of the rounded corners)
    conn.poly_fill_rectangle(
        drawable,
        gc,
        &[Rectangle {
            x: x + r,
            y,
            width: (w - r) as u16,
            height: height as u16,
        }],
    )?;

    // Draw the left middle section (between the two corners)
    if h > 2 * r {
        conn.poly_fill_rectangle(
            drawable,
            gc,
            &[Rectangle {
                x,
                y: y + r,
                width: r as u16,
                height: (h - 2 * r) as u16,
            }],
        )?;
    }

    // Draw top-left corner arc (quarter circle)
    // Arc angles are in 1/64th of a degree, starting from 3 o'clock going counterclockwise
    // Top-left: start at 90°, sweep 90° counterclockwise to 180°
    conn.poly_fill_arc(
        drawable,
        gc,
        &[Arc {
            x,
            y,
            width: (2 * r) as u16,
            height: (2 * r) as u16,
            angle1: 90 * 64,  // Start at 12 o'clock
            angle2: 90 * 64,  // Sweep 90° counterclockwise to 9 o'clock
        }],
    )?;

    // Draw bottom-left corner arc
    // Bottom-left: start at 180°, sweep 90° counterclockwise to 270°
    conn.poly_fill_arc(
        drawable,
        gc,
        &[Arc {
            x,
            y: y + h - 2 * r,
            width: (2 * r) as u16,
            height: (2 * r) as u16,
            angle1: 180 * 64, // Start at 9 o'clock
            angle2: 90 * 64,  // Sweep 90° counterclockwise to 6 o'clock
        }],
    )?;

    Ok(())
}
