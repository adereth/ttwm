//! Tab bar management and rendering.
//!
//! This module provides:
//! - TabBarManager: Stateful manager for tab bar windows, pixmaps, and icons
//! - Low-level X11 drawing functions for tab bar UI elements
//! - Rounded rectangle shapes for tabs
//! - Background fills and separators

use std::collections::{HashMap, HashSet};
use anyhow::Result;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

use crate::config::LayoutConfig;
use crate::ewmh::Atoms;
use crate::icon;
use crate::layout::{NodeId, Rect};
use crate::monitor::MonitorId;
use crate::render::{CachedIcon, FontRenderer, DEFAULT_ICON};
use crate::window_query;

// =============================================================================
// TabBarManager - Stateful tab bar management
// =============================================================================

/// Key for identifying tab bar and empty frame windows
pub type TabBarKey = (MonitorId, usize, NodeId);

/// Tab bar state and rendering manager.
///
/// Owns all tab bar-related state including window handles, pixmap buffers,
/// empty frame placeholders, and the icon cache. Methods that need X11
/// connection take it as a parameter.
pub struct TabBarManager {
    /// Map from (monitor, workspace, frame) to tab bar window
    pub windows: HashMap<TabBarKey, Window>,
    /// Map from tab bar window to its double-buffer pixmap
    pub pixmaps: HashMap<Window, u32>,
    /// Map from (monitor, workspace, frame) to empty frame placeholder window
    pub empty_frame_windows: HashMap<TabBarKey, Window>,
    /// Cached window icons
    pub icon_cache: HashMap<Window, CachedIcon>,
    /// Font renderer for tab text
    pub font_renderer: FontRenderer,
    /// Graphics context for drawing
    pub gc: Gcontext,
    /// Screen color depth
    pub screen_depth: u8,
}

impl TabBarManager {
    /// Create a new tab bar manager.
    pub fn new(font_renderer: FontRenderer, gc: Gcontext, screen_depth: u8) -> Self {
        Self {
            windows: HashMap::new(),
            pixmaps: HashMap::new(),
            empty_frame_windows: HashMap::new(),
            icon_cache: HashMap::new(),
            font_renderer,
            gc,
            screen_depth,
        }
    }

    // =========================================================================
    // Tab bar window lifecycle
    // =========================================================================

    /// Get or create a tab bar window for a frame.
    pub fn get_or_create_window(
        &mut self,
        conn: &impl Connection,
        root: Window,
        config: &LayoutConfig,
        key: TabBarKey,
        rect: &Rect,
        vertical: bool,
    ) -> Result<Window> {
        // Calculate dimensions based on orientation
        let (x, y, width, height) = if vertical {
            // Vertical: left side of frame, full height
            (rect.x, rect.y, config.vertical_tab_width, rect.height)
        } else {
            // Horizontal: top of frame, full width
            (rect.x, rect.y, rect.width, config.tab_bar_height)
        };

        if let Some(&window) = self.windows.get(&key) {
            // Update position and size
            conn.configure_window(
                window,
                &ConfigureWindowAux::new()
                    .x(x)
                    .y(y)
                    .width(width)
                    .height(height),
            )?;
            // Invalidate pixmap buffer (size may have changed)
            if let Some(pixmap) = self.pixmaps.remove(&window) {
                let _ = conn.free_pixmap(pixmap);
            }
            return Ok(window);
        }

        // Create new tab bar window
        let window = conn.generate_id()?;
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            window,
            root,
            x as i16,
            y as i16,
            width as u16,
            height as u16,
            0, // border width
            WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new()
                .background_pixel(config.tab_bar_bg)
                .event_mask(EventMask::EXPOSURE | EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE),
        )?;

        conn.map_window(window)?;
        self.windows.insert(key, window);

        Ok(window)
    }

    /// Get or create a pixmap buffer for double-buffered tab bar rendering.
    pub fn get_or_create_pixmap(
        &mut self,
        conn: &impl Connection,
        window: Window,
        width: u16,
        height: u16,
    ) -> Result<u32> {
        // Always free existing pixmap to ensure correct dimensions
        if let Some(old_pixmap) = self.pixmaps.remove(&window) {
            let _ = conn.free_pixmap(old_pixmap);
        }

        // Create new pixmap with requested dimensions
        let pixmap = conn.generate_id()?;
        conn.create_pixmap(self.screen_depth, pixmap, window, width, height)?;
        self.pixmaps.insert(window, pixmap);
        Ok(pixmap)
    }

    /// Remove tab bar windows for frames that no longer exist.
    pub fn cleanup(
        &mut self,
        conn: &impl Connection,
        mon_id: MonitorId,
        ws_idx: usize,
        valid_frames: &HashSet<NodeId>,
    ) {
        let to_remove: Vec<_> = self.windows
            .keys()
            .filter(|(m_id, idx, frame_id)| {
                *m_id == mon_id && *idx == ws_idx && !valid_frames.contains(frame_id)
            })
            .copied()
            .collect();

        for key in to_remove {
            if let Some(window) = self.windows.remove(&key) {
                // Free associated pixmap buffer
                if let Some(pixmap) = self.pixmaps.remove(&window) {
                    let _ = conn.free_pixmap(pixmap);
                }
                if let Err(e) = conn.destroy_window(window) {
                    log::error!("Failed to destroy tab bar window: {}", e);
                }
            }
        }
    }

    // =========================================================================
    // Empty frame window lifecycle
    // =========================================================================

    /// Get or create a placeholder window for an empty frame (shows border).
    pub fn get_or_create_empty_frame(
        &mut self,
        conn: &impl Connection,
        root: Window,
        config: &LayoutConfig,
        key: TabBarKey,
        rect: &Rect,
        is_focused: bool,
    ) -> Result<Window> {
        let border = config.border_width;
        let client_y = rect.y;
        let client_height = rect.height;
        let border_color = if is_focused {
            config.border_focused
        } else {
            config.border_unfocused
        };

        if let Some(&window) = self.empty_frame_windows.get(&key) {
            // Update position, size, and border color
            conn.configure_window(
                window,
                &ConfigureWindowAux::new()
                    .x(rect.x)
                    .y(client_y)
                    .width(rect.width.saturating_sub(border * 2))
                    .height(client_height.saturating_sub(border * 2))
                    .border_width(border),
            )?;
            conn.change_window_attributes(
                window,
                &ChangeWindowAttributesAux::new().border_pixel(border_color),
            )?;
            // Re-map in case it was hidden (e.g., workspace switch)
            conn.map_window(window)?;
            return Ok(window);
        }

        // Create new empty frame placeholder window
        let window = conn.generate_id()?;
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            window,
            root,
            rect.x as i16,
            client_y as i16,
            (rect.width.saturating_sub(border * 2)) as u16,
            (client_height.saturating_sub(border * 2)) as u16,
            border as u16,
            WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new()
                .background_pixel(config.tab_bar_bg)
                .border_pixel(border_color)
                .event_mask(EventMask::BUTTON_PRESS),
        )?;

        conn.map_window(window)?;
        self.empty_frame_windows.insert(key, window);

        Ok(window)
    }

    /// Destroy an empty frame placeholder window if it exists.
    pub fn destroy_empty_frame(&mut self, conn: &impl Connection, key: TabBarKey) {
        if let Some(window) = self.empty_frame_windows.remove(&key) {
            if let Err(e) = conn.destroy_window(window) {
                log::error!("Failed to destroy empty frame window: {}", e);
            }
        }
    }

    /// Clean up empty frame windows for removed frames.
    pub fn cleanup_empty_frames(
        &mut self,
        conn: &impl Connection,
        mon_id: MonitorId,
        ws_idx: usize,
        valid_frames: &HashSet<NodeId>,
    ) {
        let to_remove: Vec<_> = self.empty_frame_windows
            .keys()
            .filter(|(m_id, idx, frame_id)| {
                *m_id == mon_id && *idx == ws_idx && !valid_frames.contains(frame_id)
            })
            .copied()
            .collect();

        for key in to_remove {
            if let Some(window) = self.empty_frame_windows.remove(&key) {
                if let Err(e) = conn.destroy_window(window) {
                    log::error!("Failed to destroy empty frame window: {}", e);
                }
            }
        }
    }

    // =========================================================================
    // Icon management
    // =========================================================================

    /// Get window icon, fetching from X11 if not cached.
    /// Returns a reference to the default icon if the window has no icon.
    pub fn get_icon(&mut self, conn: &impl Connection, atoms: &Atoms, window: Window) -> &CachedIcon {
        const ICON_SIZE: u32 = 20;

        // Check cache first
        if self.icon_cache.contains_key(&window) {
            return self.icon_cache.get(&window).unwrap();
        }

        // Try to fetch _NET_WM_ICON - only cache if we get an actual icon
        if let Some(icon) = icon::fetch_icon(conn, atoms, window, ICON_SIZE) {
            self.icon_cache.insert(window, icon);
            return self.icon_cache.get(&window).unwrap();
        }

        // Return default icon for windows without _NET_WM_ICON
        &DEFAULT_ICON
    }

    /// Invalidate cached icon for a window (call when PropertyNotify for _NET_WM_ICON).
    pub fn invalidate_icon(&mut self, window: Window) {
        self.icon_cache.remove(&window);
    }

    // =========================================================================
    // Helper methods
    // =========================================================================

    /// Calculate tab widths based on window titles (Chrome-style content-based sizing).
    /// Returns a vector of (x_position, width) for each tab.
    pub fn calculate_tab_layout(
        &self,
        conn: &impl Connection,
        atoms: &Atoms,
        config: &LayoutConfig,
        windows: &[Window],
    ) -> Vec<(i16, u32)> {
        const MIN_TAB_WIDTH: u32 = 80;
        const MAX_TAB_WIDTH: u32 = 200;
        const H_PADDING: u32 = 24; // Total horizontal padding (12px each side)
        const ICON_SIZE: u32 = 20;
        const ICON_PADDING: u32 = 4; // Padding after icon

        // Extra width for icon when enabled
        let icon_width = if config.show_tab_icons {
            ICON_SIZE + ICON_PADDING
        } else {
            0
        };

        let mut result = Vec::new();
        let mut x_offset: i16 = 0;

        for &client_window in windows {
            let title = window_query::get_window_title(conn, atoms, client_window);
            let title_width = self.font_renderer.measure_text(&title);
            let tab_width = (title_width + H_PADDING + icon_width)
                .clamp(MIN_TAB_WIDTH + icon_width, MAX_TAB_WIDTH + icon_width);

            result.push((x_offset, tab_width));
            x_offset += tab_width as i16;
        }

        result
    }

    /// Sample the root window background at the given position.
    /// Returns the pixel data that can be drawn with put_image.
    pub fn sample_root_background(
        conn: &impl Connection,
        root: Window,
        x: i16,
        y: i16,
        width: u16,
        height: u16,
    ) -> Option<Vec<u8>> {
        let reply = conn.get_image(
            ImageFormat::Z_PIXMAP,
            root,
            x,
            y,
            width,
            height,
            !0, // all planes
        ).ok()?.reply().ok()?;
        Some(reply.data)
    }
}

// =============================================================================
// Low-level drawing primitives
// =============================================================================

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

/// Fill a drawable with a solid color rectangle.
///
/// This is a simple wrapper around poly_fill_rectangle for filling
/// entire pixmaps or windows with a background color.
/// Note: The GC foreground color must be set before calling this function.
pub fn fill_solid(
    conn: &impl Connection,
    gc: Gcontext,
    drawable: Drawable,
    width: u16,
    height: u16,
) -> Result<()> {
    conn.poly_fill_rectangle(
        drawable,
        gc,
        &[Rectangle {
            x: 0,
            y: 0,
            width,
            height,
        }],
    )?;
    Ok(())
}

/// Draw a vertical separator line (used between unfocused tabs).
///
/// Draws a 1-pixel wide vertical line at the specified position.
/// Note: The GC foreground color must be set to the separator color before calling.
pub fn draw_vertical_separator(
    conn: &impl Connection,
    gc: Gcontext,
    drawable: Drawable,
    x: i16,
    y: i16,
    height: u16,
) -> Result<()> {
    conn.poly_fill_rectangle(
        drawable,
        gc,
        &[Rectangle {
            x,
            y,
            width: 1,
            height,
        }],
    )?;
    Ok(())
}

/// Draw a horizontal separator line (used in vertical tab bars).
///
/// Draws a 1-pixel tall horizontal line at the specified position.
/// Note: The GC foreground color must be set to the separator color before calling.
pub fn draw_horizontal_separator(
    conn: &impl Connection,
    gc: Gcontext,
    drawable: Drawable,
    x: i16,
    y: i16,
    width: u16,
) -> Result<()> {
    conn.poly_segment(
        drawable,
        gc,
        &[Segment {
            x1: x,
            y1: y,
            x2: x + width as i16 - 1,
            y2: y,
        }],
    )?;
    Ok(())
}

/// Clear a rectangular area with a solid color.
///
/// Used to clear ghost tabs or empty areas in the tab bar.
/// Note: The GC foreground color must be set to the background color before calling.
pub fn clear_area(
    conn: &impl Connection,
    gc: Gcontext,
    drawable: Drawable,
    x: i16,
    y: i16,
    width: u16,
    height: u16,
) -> Result<()> {
    conn.poly_fill_rectangle(
        drawable,
        gc,
        &[Rectangle {
            x,
            y,
            width,
            height,
        }],
    )?;
    Ok(())
}
