//! Urgent window management.
//!
//! Tracks windows requesting attention (urgent hints) and manages the visual
//! indicator for cross-workspace urgent windows.

use anyhow::Result;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

/// Manager for tracking urgent windows and the visual indicator.
///
/// Windows are stored in FIFO order (oldest first) so that FocusUrgent
/// always focuses the window that has been waiting longest.
pub struct UrgentManager {
    /// Urgent windows in FIFO order (oldest first)
    windows: Vec<Window>,
    /// Overlay window for cross-workspace urgent indicator
    indicator: Option<Window>,
}

impl UrgentManager {
    /// Create a new empty urgent manager.
    pub fn new() -> Self {
        Self {
            windows: Vec::new(),
            indicator: None,
        }
    }

    /// Add a window to the urgent list (appends to end = newest).
    pub fn add(&mut self, window: Window) {
        if !self.windows.contains(&window) {
            self.windows.push(window);
        }
    }

    /// Remove a window from the urgent list.
    pub fn remove(&mut self, window: Window) {
        self.windows.retain(|&w| w != window);
    }

    /// Check if a window is in the urgent list.
    pub fn contains(&self, window: Window) -> bool {
        self.windows.contains(&window)
    }

    /// Get the oldest urgent window (first in FIFO order).
    pub fn first(&self) -> Option<Window> {
        self.windows.first().copied()
    }

    /// Check if there are no urgent windows.
    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    /// Iterate over urgent windows.
    pub fn iter(&self) -> impl Iterator<Item = &Window> {
        self.windows.iter()
    }

    /// Get the list of urgent windows for IPC.
    pub fn windows(&self) -> &[Window] {
        &self.windows
    }

    /// Get the indicator window ID if it exists.
    pub fn indicator(&self) -> Option<Window> {
        self.indicator
    }

    /// Set the indicator window ID.
    pub fn set_indicator(&mut self, window: Window) {
        self.indicator = Some(window);
    }
}

impl Default for UrgentManager {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Indicator UI Functions
// =============================================================================

/// Size of the urgent indicator circle in pixels.
const INDICATOR_SIZE: u16 = 16;

/// Margin from screen edge in pixels.
const INDICATOR_MARGIN: i16 = 10;

/// Create the urgent indicator window (does not map it).
///
/// The indicator is a small window in the upper-right corner that shows
/// when there are urgent windows on other workspaces.
pub fn create_indicator(
    conn: &impl Connection,
    root: Window,
    color: u32,
    screen_width: u16,
) -> Result<Window> {
    let window = conn.generate_id()?;
    let x = screen_width as i16 - INDICATOR_SIZE as i16 - INDICATOR_MARGIN;
    let y = INDICATOR_MARGIN;

    conn.create_window(
        x11rb::COPY_DEPTH_FROM_PARENT,
        window,
        root,
        x,
        y,
        INDICATOR_SIZE,
        INDICATOR_SIZE,
        0,
        WindowClass::INPUT_OUTPUT,
        x11rb::COPY_FROM_PARENT,
        &CreateWindowAux::new()
            .background_pixel(color)
            .override_redirect(1), // Don't manage this window
    )?;

    Ok(window)
}

/// Show and draw the urgent indicator.
///
/// Maps the window, raises it to the top, and draws a filled circle.
pub fn show_indicator(
    conn: &impl Connection,
    gc: Gcontext,
    window: Window,
    color: u32,
) -> Result<()> {
    conn.map_window(window)?;
    conn.configure_window(window, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE))?;

    // Draw a filled circle
    conn.change_gc(gc, &ChangeGCAux::new().foreground(color))?;
    conn.poly_fill_arc(
        window,
        gc,
        &[Arc {
            x: 0,
            y: 0,
            width: INDICATOR_SIZE,
            height: INDICATOR_SIZE,
            angle1: 0,
            angle2: 360 * 64, // Full circle (angles in 1/64 degree units)
        }],
    )?;
    conn.flush()?;

    Ok(())
}

/// Hide the urgent indicator by unmapping it.
pub fn hide_indicator(conn: &impl Connection, window: Window) -> Result<()> {
    conn.unmap_window(window)?;
    conn.flush()?;
    Ok(())
}
