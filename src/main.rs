//! ttwm - Tabbed Tiling Window Manager
//!
//! A minimal X11 tiling window manager inspired by Notion.
//! Milestone 5: Tabs with tab bar rendering.
//! Milestone 6: IPC interface for debugability and scriptability.

mod config;
mod ewmh;
mod ipc;
mod layout;
mod render;
mod state;
mod tracing;
mod types;
mod workspaces;

use std::collections::HashMap;
use std::process::Command;

use anyhow::{Context, Result};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use config::{parse_color, Config, ParsedBinding, WmAction};
use ewmh::Atoms;
use ipc::{IpcCommand, IpcResponse, IpcServer, WmStateSnapshot, WindowInfo};
use layout::{Direction, NodeId, Rect, SplitDirection};
use workspaces::{WorkspaceManager, NUM_WORKSPACES};
use render::{CachedIcon, FontRenderer, blend_icon_with_background, lighten_color, darken_color};
use state::{StateTransition, UnmanageReason};
use tracing::EventTracer;

/// Layout configuration
struct LayoutConfig {
    /// Gap between windows
    gap: u32,
    /// Outer gap (margin from screen edge)
    outer_gap: u32,
    /// Border width
    border_width: u32,
    /// Tab bar height
    tab_bar_height: u32,
    /// Tab bar background color
    tab_bar_bg: u32,
    /// Tab bar focused tab color
    tab_focused_bg: u32,
    /// Tab bar unfocused tab color
    tab_unfocused_bg: u32,
    /// Visible tab in unfocused frame color
    tab_visible_unfocused_bg: u32,
    /// Tagged tab background color
    tab_tagged_bg: u32,
    /// Tab bar text color
    tab_text_color: u32,
    /// Tab bar text color for background tabs
    tab_text_unfocused: u32,
    /// Tab separator color
    tab_separator: u32,
    /// Border color for focused window
    border_focused: u32,
    /// Border color for unfocused window
    border_unfocused: u32,
    /// Show application icons in tabs
    show_tab_icons: bool,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            gap: 8,
            outer_gap: 8,
            border_width: 2,
            tab_bar_height: 28,
            tab_bar_bg: 0x000000,       // Black (fallback)
            tab_focused_bg: 0x5294e2,   // Blue (matching border)
            tab_unfocused_bg: 0x3a3a3a, // Darker gray
            tab_visible_unfocused_bg: 0x4a6a9a, // Muted blue
            tab_tagged_bg: 0xe06c75,    // Soft red
            tab_text_color: 0xffffff,   // White
            tab_text_unfocused: 0x888888, // Dim gray
            tab_separator: 0x4a4a4a,    // Subtle separator
            border_focused: 0x5294e2,   // Blue
            border_unfocused: 0x3a3a3a, // Gray
            show_tab_icons: true,
        }
    }
}

/// Drag state for tab drag-and-drop or resize operations
enum DragState {
    /// Dragging a tab between frames
    Tab {
        /// Window being dragged
        window: Window,
        /// Original frame the window was in
        source_frame: NodeId,
        /// Original tab index
        source_index: usize,
    },
    /// Resizing a split by dragging the gap
    Resize {
        /// The split node being resized
        split_id: NodeId,
        /// Direction of the split
        direction: SplitDirection,
        /// Starting position of the resizable area (x for horizontal, y for vertical)
        split_start: i32,
        /// Total size in the split direction
        total_size: u32,
    },
}

/// The main window manager state
struct Wm {
    conn: RustConnection,
    screen_num: usize,
    root: Window,
    atoms: Atoms,
    /// Workspaces (virtual desktops) - each has its own layout tree
    workspaces: WorkspaceManager,
    /// Currently focused window (if any)
    focused_window: Option<Window>,
    /// WM check window for EWMH
    check_window: Window,
    /// Layout configuration
    config: LayoutConfig,
    /// Tab bar windows for each frame ((workspace_idx, NodeId) -> Window)
    tab_bar_windows: HashMap<(usize, NodeId), Window>,
    /// Windows we've intentionally unmapped (hidden tabs) - don't unmanage on UnmapNotify
    hidden_windows: std::collections::HashSet<Window>,
    /// Graphics context for drawing
    gc: Gcontext,
    /// Whether we should keep running
    running: bool,
    /// IPC server for external control
    ipc: Option<IpcServer>,
    /// Event tracer for debugging
    tracer: EventTracer,
    /// Parsed keybindings (action -> binding)
    keybindings: HashMap<WmAction, ParsedBinding>,
    /// Current drag operation (if any)
    drag_state: Option<DragState>,
    /// Font renderer for tab text
    font_renderer: FontRenderer,
    /// Horizontal resize cursor
    cursor_resize_h: Cursor,
    /// Vertical resize cursor
    cursor_resize_v: Cursor,
    /// Screen depth (for put_image)
    screen_depth: u8,
    /// Icon cache for tab icons (window -> cached icon, None = no icon available)
    icon_cache: HashMap<Window, Option<CachedIcon>>,
    /// Windows that are currently tagged for batch operations
    tagged_windows: std::collections::HashSet<Window>,
    /// Suppress EnterNotify focus changes (set after explicit focus operations)
    suppress_enter_focus: bool,
}

impl Wm {
    /// Connect to X11 and set up the window manager
    fn new() -> Result<Self> {
        // Connect to X11 server
        let (conn, screen_num) = RustConnection::connect(None)
            .context("Failed to connect to X11 server")?;

        let screen = &conn.setup().roots[screen_num];
        let root = screen.root;
        let screen_depth = screen.root_depth;

        log::info!(
            "Connected to X11, screen {}, root window 0x{:x}, {}x{}",
            screen_num,
            root,
            screen.width_in_pixels,
            screen.height_in_pixels
        );

        // Create atoms for EWMH
        let atoms = Atoms::new(&conn)?;

        // Create a small check window for EWMH _NET_SUPPORTING_WM_CHECK
        let check_window = conn.generate_id()?;
        conn.create_window(
            0, // depth (copy from parent)
            check_window,
            root,
            -1, -1, 1, 1, 0, // x, y, w, h, border
            WindowClass::INPUT_ONLY,
            0, // visual (copy from parent)
            &CreateWindowAux::new(),
        )?;

        // Create graphics context for drawing tab bars
        let gc = conn.generate_id()?;
        conn.create_gc(
            gc,
            root,
            &CreateGCAux::new()
                .foreground(screen.white_pixel)
                .background(screen.black_pixel),
        )?;

        // Initialize IPC server (non-fatal if it fails)
        let ipc = match IpcServer::bind() {
            Ok(server) => Some(server),
            Err(e) => {
                log::warn!("Failed to start IPC server: {}. IPC will be disabled.", e);
                None
            }
        };

        // Load user configuration
        let user_config = Config::load();
        let keybindings = user_config.parse_keybindings();

        // Initialize font renderer
        let font_renderer = FontRenderer::new(
            &user_config.appearance.tab_font,
            user_config.appearance.tab_font_size,
        ).context("Failed to initialize font renderer")?;

        // Build LayoutConfig from user config
        let config = LayoutConfig {
            gap: user_config.appearance.gap,
            outer_gap: user_config.appearance.outer_gap,
            border_width: user_config.appearance.border_width,
            tab_bar_height: user_config.appearance.tab_bar_height,
            tab_bar_bg: parse_color(&user_config.colors.tab_bar_bg).unwrap_or(0x2e2e2e),
            tab_focused_bg: parse_color(&user_config.colors.tab_focused_bg).unwrap_or(0x5294e2),
            tab_unfocused_bg: parse_color(&user_config.colors.tab_unfocused_bg).unwrap_or(0x3a3a3a),
            tab_visible_unfocused_bg: parse_color(&user_config.colors.tab_visible_unfocused_bg).unwrap_or(0x4a6a9a),
            tab_tagged_bg: parse_color(&user_config.colors.tab_tagged_bg).unwrap_or(0xe06c75),
            tab_text_color: parse_color(&user_config.colors.tab_text).unwrap_or(0xffffff),
            tab_text_unfocused: parse_color(&user_config.colors.tab_text_unfocused).unwrap_or(0x888888),
            tab_separator: parse_color(&user_config.colors.tab_separator).unwrap_or(0x4a4a4a),
            border_focused: parse_color(&user_config.colors.border_focused).unwrap_or(0x5294e2),
            border_unfocused: parse_color(&user_config.colors.border_unfocused).unwrap_or(0x3a3a3a),
            show_tab_icons: user_config.appearance.show_tab_icons,
        };

        // Create resize cursors from the cursor font
        let cursor_font = conn.generate_id()?;
        conn.open_font(cursor_font, b"cursor")?;

        // XC_sb_h_double_arrow = 108 (horizontal resize)
        let cursor_resize_h = conn.generate_id()?;
        conn.create_glyph_cursor(
            cursor_resize_h,
            cursor_font,
            cursor_font,
            108,      // source glyph (arrow shape)
            108 + 1,  // mask glyph (solid fill)
            0, 0, 0,  // foreground RGB (black)
            0xFFFF, 0xFFFF, 0xFFFF,  // background RGB (white)
        )?;

        // XC_sb_v_double_arrow = 116 (vertical resize)
        let cursor_resize_v = conn.generate_id()?;
        conn.create_glyph_cursor(
            cursor_resize_v,
            cursor_font,
            cursor_font,
            116,      // source glyph (arrow shape)
            116 + 1,  // mask glyph (solid fill)
            0, 0, 0,  // foreground RGB (black)
            0xFFFF, 0xFFFF, 0xFFFF,  // background RGB (white)
        )?;

        conn.close_font(cursor_font)?;

        Ok(Self {
            conn,
            screen_num,
            root,
            atoms,
            workspaces: WorkspaceManager::new(),
            focused_window: None,
            check_window,
            config,
            tab_bar_windows: HashMap::new(),
            hidden_windows: std::collections::HashSet::new(),
            gc,
            running: true,
            ipc,
            tracer: EventTracer::new(),
            keybindings,
            drag_state: None,
            font_renderer,
            cursor_resize_h,
            cursor_resize_v,
            screen_depth,
            icon_cache: HashMap::new(),
            tagged_windows: std::collections::HashSet::new(),
            suppress_enter_focus: false,
        })
    }

    /// Get screen info
    fn screen(&self) -> &Screen {
        &self.conn.setup().roots[self.screen_num]
    }

    /// Become the window manager by requesting SubstructureRedirect on root
    fn become_wm(&self) -> Result<()> {
        // Set event mask on root window
        // SubstructureRedirect is the key - it makes us the WM
        let event_mask = EventMask::SUBSTRUCTURE_REDIRECT
            | EventMask::SUBSTRUCTURE_NOTIFY
            | EventMask::ENTER_WINDOW  // For focus-follows-mouse
            | EventMask::STRUCTURE_NOTIFY
            | EventMask::BUTTON_PRESS; // For gap resize detection

        let result = self.conn.change_window_attributes(
            self.root,
            &ChangeWindowAttributesAux::new().event_mask(event_mask),
        );

        // Flush and check for errors
        self.conn.flush()?;

        if let Err(e) = result?.check() {
            anyhow::bail!(
                "Another window manager is already running! Error: {}",
                e
            );
        }

        log::info!("Successfully became the window manager");
        Ok(())
    }

    /// Set up EWMH properties on root window
    fn setup_ewmh(&self) -> Result<()> {
        // Set _NET_SUPPORTED - list of supported EWMH atoms
        let supported = [
            self.atoms.net_supported,
            self.atoms.net_client_list,
            self.atoms.net_active_window,
            self.atoms.net_close_window,
            self.atoms.net_wm_name,
            self.atoms.net_supporting_wm_check,
            self.atoms.net_current_desktop,
            self.atoms.net_number_of_desktops,
            self.atoms.net_desktop_names,
            self.atoms.net_wm_desktop,
        ];
        self.conn.change_property32(
            PropMode::REPLACE,
            self.root,
            self.atoms.net_supported,
            AtomEnum::ATOM,
            &supported,
        )?;

        // Set _NET_SUPPORTING_WM_CHECK on root and check window
        self.conn.change_property32(
            PropMode::REPLACE,
            self.root,
            self.atoms.net_supporting_wm_check,
            AtomEnum::WINDOW,
            &[self.check_window],
        )?;
        self.conn.change_property32(
            PropMode::REPLACE,
            self.check_window,
            self.atoms.net_supporting_wm_check,
            AtomEnum::WINDOW,
            &[self.check_window],
        )?;

        // Set _NET_WM_NAME on check window
        self.conn.change_property8(
            PropMode::REPLACE,
            self.check_window,
            self.atoms.net_wm_name,
            self.atoms.utf8_string,
            b"ttwm",
        )?;

        // Initialize empty _NET_CLIENT_LIST
        self.conn.change_property32(
            PropMode::REPLACE,
            self.root,
            self.atoms.net_client_list,
            AtomEnum::WINDOW,
            &[],
        )?;

        // Set _NET_NUMBER_OF_DESKTOPS
        self.conn.change_property32(
            PropMode::REPLACE,
            self.root,
            self.atoms.net_number_of_desktops,
            AtomEnum::CARDINAL,
            &[NUM_WORKSPACES as u32],
        )?;

        // Set _NET_CURRENT_DESKTOP
        self.conn.change_property32(
            PropMode::REPLACE,
            self.root,
            self.atoms.net_current_desktop,
            AtomEnum::CARDINAL,
            &[0u32],
        )?;

        // Set _NET_DESKTOP_NAMES
        let names = (1..=NUM_WORKSPACES).map(|i| format!("{}\0", i)).collect::<String>();
        self.conn.change_property8(
            PropMode::REPLACE,
            self.root,
            self.atoms.net_desktop_names,
            self.atoms.utf8_string,
            names.as_bytes(),
        )?;

        self.conn.flush()?;
        log::info!("EWMH properties set up");
        Ok(())
    }

    /// Update _NET_CURRENT_DESKTOP
    fn update_current_desktop(&self) -> Result<()> {
        self.conn.change_property32(
            PropMode::REPLACE,
            self.root,
            self.atoms.net_current_desktop,
            AtomEnum::CARDINAL,
            &[self.workspaces.current_index() as u32],
        )?;
        self.conn.flush()?;
        Ok(())
    }

    /// Set _NET_WM_DESKTOP for a window
    fn set_window_desktop(&self, window: Window, desktop: usize) -> Result<()> {
        self.conn.change_property32(
            PropMode::REPLACE,
            window,
            self.atoms.net_wm_desktop,
            AtomEnum::CARDINAL,
            &[desktop as u32],
        )?;
        Ok(())
    }

    /// Switch to the next workspace
    fn workspace_next(&mut self) -> Result<()> {
        let old_idx = self.workspaces.next();
        self.perform_workspace_switch(old_idx)?;
        Ok(())
    }

    /// Switch to the previous workspace
    fn workspace_prev(&mut self) -> Result<()> {
        let old_idx = self.workspaces.prev();
        self.perform_workspace_switch(old_idx)?;
        Ok(())
    }

    /// Toggle tag on the focused window
    fn tag_focused_window(&mut self) -> Result<()> {
        if let Some(window) = self.focused_window {
            if self.tagged_windows.contains(&window) {
                self.tagged_windows.remove(&window);
                log::info!("Untagged window 0x{:x}", window);
            } else {
                self.tagged_windows.insert(window);
                log::info!("Tagged window 0x{:x}", window);
            }
            self.apply_layout()?;
        }
        Ok(())
    }

    /// Move all tagged windows to the currently focused frame and untag them
    fn move_tagged_to_focused_frame(&mut self) -> Result<()> {
        let target_frame = self.workspaces.current().layout.focused;
        let tagged: Vec<Window> = self.tagged_windows.iter().copied().collect();
        let count = tagged.len();

        let mut last_moved: Option<Window> = None;
        for window in tagged {
            if let Some(source_frame) = self.workspaces.current_mut().layout.find_window(window) {
                if source_frame != target_frame {
                    self.workspaces.current_mut().layout.move_window_to_frame(
                        window,
                        source_frame,
                        target_frame,
                    );
                    last_moved = Some(window);
                }
            }
        }

        self.tagged_windows.clear();
        self.apply_layout()?;

        // Focus the last moved window
        if let Some(window) = last_moved {
            self.suppress_enter_focus = true;
            self.focus_window(window)?;
        }

        log::info!("Moved {} tagged windows to focused frame", count);
        Ok(())
    }

    /// Untag all windows without moving them
    fn untag_all_windows(&mut self) -> Result<()> {
        let count = self.tagged_windows.len();
        self.tagged_windows.clear();
        self.apply_layout()?;
        log::info!("Untagged {} windows", count);
        Ok(())
    }

    /// Perform the workspace switch after index has been changed
    fn perform_workspace_switch(&mut self, old_idx: usize) -> Result<()> {
        let new_idx = self.workspaces.current_index();
        log::info!("Switching from workspace {} to workspace {}", old_idx + 1, new_idx + 1);

        // Save current workspace's focused window
        self.workspaces.workspaces[old_idx].last_focused_window = self.focused_window;

        // Hide all windows from old workspace
        for window in self.workspaces.workspaces[old_idx].layout.all_windows() {
            self.hidden_windows.insert(window);
            self.conn.unmap_window(window)?;
        }

        // Hide tab bars from old workspace
        for (&(ws_idx, _), &tab_window) in &self.tab_bar_windows {
            if ws_idx == old_idx {
                self.conn.unmap_window(tab_window)?;
            }
        }

        // Show windows from new workspace
        for window in self.workspaces.current().layout.all_windows() {
            self.hidden_windows.remove(&window);
        }

        // Clear focused window (will be restored below)
        self.focused_window = None;

        // Apply layout for new workspace
        self.apply_layout()?;

        // Restore focus to last focused window in new workspace
        if let Some(w) = self.workspaces.current().last_focused_window {
            if self.workspaces.current().layout.find_window(w).is_some() {
                self.focus_window(w)?;
            }
        } else if let Some(frame) = self.workspaces.current().layout.focused_frame() {
            if let Some(w) = frame.focused_window() {
                self.focus_window(w)?;
            }
        }

        // Update EWMH
        self.update_current_desktop()?;

        self.conn.flush()?;
        Ok(())
    }

    /// Update _NET_CLIENT_LIST with current windows (from all workspaces)
    fn update_client_list(&self) -> Result<()> {
        let windows: Vec<Window> = self.workspaces.workspaces.iter()
            .flat_map(|ws| ws.layout.all_windows())
            .collect();
        self.conn.change_property32(
            PropMode::REPLACE,
            self.root,
            self.atoms.net_client_list,
            AtomEnum::WINDOW,
            &windows,
        )?;
        Ok(())
    }

    /// Update _NET_ACTIVE_WINDOW
    fn update_active_window(&self) -> Result<()> {
        let active = self.focused_window.unwrap_or(0);
        self.conn.change_property32(
            PropMode::REPLACE,
            self.root,
            self.atoms.net_active_window,
            AtomEnum::WINDOW,
            &[active],
        )?;
        Ok(())
    }

    /// Get the usable screen area (with outer gaps)
    fn usable_screen(&self) -> Rect {
        let screen = self.screen();
        let gap = self.config.outer_gap;
        Rect::new(
            gap as i32,
            gap as i32,
            (screen.width_in_pixels as u32).saturating_sub(gap * 2),
            (screen.height_in_pixels as u32).saturating_sub(gap * 2),
        )
    }

    /// Get or create a tab bar window for a frame
    fn get_or_create_tab_bar(&mut self, frame_id: NodeId, rect: &Rect) -> Result<Window> {
        let ws_idx = self.workspaces.current_index();
        let key = (ws_idx, frame_id);

        if let Some(&window) = self.tab_bar_windows.get(&key) {
            // Update position and size
            self.conn.configure_window(
                window,
                &ConfigureWindowAux::new()
                    .x(rect.x)
                    .y(rect.y)
                    .width(rect.width)
                    .height(self.config.tab_bar_height),
            )?;
            return Ok(window);
        }

        // Create new tab bar window
        let window = self.conn.generate_id()?;
        self.conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            window,
            self.root,
            rect.x as i16,
            rect.y as i16,
            rect.width as u16,
            self.config.tab_bar_height as u16,
            0, // border width
            WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new()
                .background_pixel(self.config.tab_bar_bg)
                .event_mask(EventMask::EXPOSURE | EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE),
        )?;

        self.conn.map_window(window)?;
        self.tab_bar_windows.insert(key, window);

        Ok(window)
    }

    /// Calculate tab widths based on window titles (Chrome-style content-based sizing)
    /// Returns a vector of (x_position, width) for each tab
    fn calculate_tab_layout(&self, frame_id: NodeId) -> Vec<(i16, u32)> {
        const MIN_TAB_WIDTH: u32 = 80;
        const MAX_TAB_WIDTH: u32 = 200;
        const H_PADDING: u32 = 24;  // Total horizontal padding (12px each side)
        const ICON_SIZE: u32 = 20;
        const ICON_PADDING: u32 = 4;  // Padding after icon

        let frame = match self.workspaces.current().layout.get(frame_id).and_then(|n| n.as_frame()) {
            Some(f) => f,
            None => return Vec::new(),
        };

        // Extra width for icon when enabled
        let icon_width = if self.config.show_tab_icons {
            ICON_SIZE + ICON_PADDING
        } else {
            0
        };

        let mut result = Vec::new();
        let mut x_offset: i16 = 0;

        for &client_window in &frame.windows {
            let title = self.get_window_title(client_window);
            let title_width = self.font_renderer.measure_text(&title);
            let tab_width = (title_width + H_PADDING + icon_width).clamp(MIN_TAB_WIDTH + icon_width, MAX_TAB_WIDTH + icon_width);

            result.push((x_offset, tab_width));
            x_offset += tab_width as i16;
        }

        result
    }

    /// Draw a filled rectangle with rounded top corners
    fn draw_rounded_top_rect(
        &self,
        window: Window,
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
        self.conn.poly_fill_rectangle(
            window,
            self.gc,
            &[Rectangle {
                x,
                y: y + r,
                width: width as u16,
                height: (h - r) as u16,
            }],
        )?;

        // Draw the top middle section (between the two corners)
        if w > 2 * r {
            self.conn.poly_fill_rectangle(
                window,
                self.gc,
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
        self.conn.poly_fill_arc(
            window,
            self.gc,
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
        self.conn.poly_fill_arc(
            window,
            self.gc,
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

    /// Sample the root window background at the given position
    /// Returns the pixel data that can be drawn with put_image
    fn sample_root_background(&self, x: i16, y: i16, width: u16, height: u16) -> Option<Vec<u8>> {
        let reply = self.conn.get_image(
            ImageFormat::Z_PIXMAP,
            self.root,
            x, y,
            width, height,
            !0,  // all planes
        ).ok()?.reply().ok()?;
        Some(reply.data)
    }

    /// Draw the pseudo-transparent background for a tab bar.
    fn draw_tab_bar_background(&mut self, window: Window, rect: &Rect) -> Result<()> {
        let height = self.config.tab_bar_height;

        // Clear with solid color first to ensure old content is erased
        self.conn.change_gc(self.gc, &ChangeGCAux::new().foreground(self.config.tab_bar_bg))?;
        self.conn.poly_fill_rectangle(
            window,
            self.gc,
            &[Rectangle {
                x: 0,
                y: 0,
                width: rect.width as u16,
                height: height as u16,
            }],
        )?;

        // Sample and draw root background on top (pseudo-transparency)
        if let Some(pixels) = self.sample_root_background(
            rect.x as i16,
            rect.y as i16,
            rect.width as u16,
            height as u16,
        ) {
            self.conn.put_image(
                ImageFormat::Z_PIXMAP,
                window,
                self.gc,
                rect.width as u16,
                height as u16,
                0, 0,  // destination x, y
                0,     // left_pad
                self.screen_depth,
                &pixels,
            )?;
        }

        Ok(())
    }

    /// Draw the focus indicator for an empty frame.
    fn draw_empty_frame_indicator(&mut self, window: Window, rect: &Rect) -> Result<()> {
        let accent_height = 3u16;
        self.conn.change_gc(self.gc, &ChangeGCAux::new().foreground(self.config.tab_focused_bg))?;
        self.conn.poly_fill_rectangle(
            window,
            self.gc,
            &[Rectangle {
                x: 0,
                y: 0,
                width: rect.width as u16,
                height: accent_height,
            }],
        )?;
        Ok(())
    }

    /// Draw a single tab in the tab bar.
    #[allow(clippy::too_many_arguments)]
    fn draw_single_tab(
        &mut self,
        window: Window,
        x: i16,
        tab_width: u32,
        client_window: Window,
        is_focused: bool,
        is_last: bool,
        is_tagged: bool,
        is_focused_frame: bool,
        show_icons: bool,
    ) -> Result<()> {
        let height = self.config.tab_bar_height;
        let h_padding: i16 = 12;    // Horizontal text padding
        let corner_radius: u32 = 6; // Rounded corner radius
        let icon_size: u32 = 20;    // Icon size in pixels
        let icon_padding: i16 = 4;  // Padding after icon

        // Tab background color (4 states: tagged, focused frame visible, unfocused frame visible, background)
        let bg_color = if is_tagged {
            self.config.tab_tagged_bg
        } else if is_focused {
            if is_focused_frame {
                self.config.tab_focused_bg
            } else {
                self.config.tab_visible_unfocused_bg
            }
        } else {
            self.config.tab_unfocused_bg
        };

        // Draw drop shadow for focused tabs (before tab background so it appears behind)
        if is_focused {
            let shadow_color = darken_color(bg_color, 0.3);
            self.conn.change_gc(self.gc, &ChangeGCAux::new().foreground(shadow_color))?;
            self.conn.poly_fill_rectangle(
                window,
                self.gc,
                &[Rectangle {
                    x: x + 2,
                    y: (height - 2) as i16,
                    width: tab_width as u16,
                    height: 3,
                }],
            )?;
        }

        // Draw tab background with rounded top corners
        self.conn.change_gc(self.gc, &ChangeGCAux::new().foreground(bg_color))?;
        self.draw_rounded_top_rect(window, x, 0, tab_width, height, corner_radius)?;

        // Draw bevel effect for 3D raised appearance
        let bevel_light = lighten_color(bg_color, 0x20);
        let bevel_dark = darken_color(bg_color, 0.7);

        // Top highlight (inside rounded corners)
        self.conn.change_gc(self.gc, &ChangeGCAux::new().foreground(bevel_light))?;
        self.conn.poly_fill_rectangle(
            window,
            self.gc,
            &[Rectangle {
                x: x + corner_radius as i16,
                y: 1,
                width: (tab_width - corner_radius * 2) as u16,
                height: 1,
            }],
        )?;

        // Bottom shadow line
        self.conn.change_gc(self.gc, &ChangeGCAux::new().foreground(bevel_dark))?;
        self.conn.poly_fill_rectangle(
            window,
            self.gc,
            &[Rectangle {
                x: x,
                y: (height - 1) as i16,
                width: tab_width as u16,
                height: 1,
            }],
        )?;

        // Draw separator on right edge for unfocused tabs (except last)
        if !is_focused && !is_last {
            self.conn.change_gc(self.gc, &ChangeGCAux::new().foreground(self.config.tab_separator))?;
            self.conn.poly_fill_rectangle(
                window,
                self.gc,
                &[Rectangle {
                    x: x + tab_width as i16 - 1,
                    y: 4,
                    width: 1,
                    height: (height - 8) as u16,
                }],
            )?;
        }

        // Calculate content offset (shifts right if icon is present)
        let mut content_offset: i16 = 0;

        // Draw icon if enabled
        if show_icons {
            if let Some(icon) = self.get_window_icon(client_window) {
                // Blend icon with tab background and render
                let blended = blend_icon_with_background(&icon.pixels, bg_color, icon_size);

                let icon_x = x + h_padding;
                let icon_y = ((height - icon_size) / 2) as i16;

                self.conn.put_image(
                    ImageFormat::Z_PIXMAP,
                    window,
                    self.gc,
                    icon_size as u16,
                    icon_size as u16,
                    icon_x,
                    icon_y,
                    0,
                    24, // 24-bit depth
                    &blended,
                )?;

                content_offset = icon_size as i16 + icon_padding;
            } else {
                // No icon available, but still reserve space for consistency
                content_offset = icon_size as i16 + icon_padding;
            }
        }

        // Get window title and truncate if needed
        let title = self.get_window_title(client_window);
        let available_width = (tab_width as i32 - h_padding as i32 * 2 - content_offset as i32).max(0) as u32;
        let display_title = self.font_renderer.truncate_text_to_width(&title, available_width);

        // Text color (dimmer for background tabs)
        let text_color = if is_focused {
            self.config.tab_text_color
        } else {
            self.config.tab_text_unfocused
        };

        // Render text with FreeType
        let (pixels, text_width, text_height) = self.font_renderer.render_text(
            &display_title,
            text_color,
            bg_color,
        );

        if !pixels.is_empty() && text_width > 0 && text_height > 0 {
            // Calculate text position (vertically centered, after icon)
            let text_x = x + h_padding + content_offset;
            let text_y = ((height - text_height) / 2) as i16;

            // Draw text using put_image
            self.conn.put_image(
                ImageFormat::Z_PIXMAP,
                window,
                self.gc,
                text_width as u16,
                text_height as u16,
                text_x,
                text_y,
                0,
                24, // depth (24-bit color, will be padded to 32)
                &pixels,
            )?;
        }

        Ok(())
    }

    /// Draw the tab bar for a frame (Chrome-style with content-based tab widths)
    fn draw_tab_bar(&mut self, frame_id: NodeId, window: Window, rect: &Rect) -> Result<()> {
        // Extract all needed data from frame before any mutable calls
        let (windows, focused_tab, is_empty) = {
            let frame = match self.workspaces.current().layout.get(frame_id).and_then(|n| n.as_frame()) {
                Some(f) => f,
                None => return Ok(()),
            };
            (frame.windows.clone(), frame.focused, frame.windows.is_empty())
        };

        // Draw pseudo-transparent background
        self.draw_tab_bar_background(window, rect)?;

        // Empty frame - show focus indicator if focused
        if is_empty {
            let is_focused_frame = frame_id == self.workspaces.current().layout.focused;
            if is_focused_frame {
                self.draw_empty_frame_indicator(window, rect)?;
            }
            return Ok(());
        }

        // Get tab layout (content-based widths, left-aligned)
        let tab_layout = self.calculate_tab_layout(frame_id);

        // Check if this frame is the focused frame
        let is_focused_frame = frame_id == self.workspaces.current().layout.focused;
        let show_icons = self.config.show_tab_icons;
        let num_tabs = windows.len();

        // Draw each tab
        for (i, &client_window) in windows.iter().enumerate() {
            let (x, tab_width) = tab_layout[i];
            let is_focused = i == focused_tab;
            let is_last = i == num_tabs - 1;
            let is_tagged = self.tagged_windows.contains(&client_window);

            self.draw_single_tab(
                window,
                x,
                tab_width,
                client_window,
                is_focused,
                is_last,
                is_tagged,
                is_focused_frame,
                show_icons,
            )?;
        }

        Ok(())
    }

    /// Get window title (WM_NAME or _NET_WM_NAME)
    fn get_window_title(&self, window: Window) -> String {
        // Try _NET_WM_NAME first
        if let Ok(reply) = self.conn.get_property(
            false,
            window,
            self.atoms.net_wm_name,
            self.atoms.utf8_string,
            0,
            1024,
        ) {
            if let Ok(reply) = reply.reply() {
                if !reply.value.is_empty() {
                    if let Ok(s) = String::from_utf8(reply.value) {
                        return s;
                    }
                }
            }
        }

        // Fall back to WM_NAME
        if let Ok(reply) = self.conn.get_property(
            false,
            window,
            AtomEnum::WM_NAME,
            AtomEnum::STRING,
            0,
            1024,
        ) {
            if let Ok(reply) = reply.reply() {
                if !reply.value.is_empty() {
                    if let Ok(s) = String::from_utf8(reply.value) {
                        return s;
                    }
                }
            }
        }

        // Default title
        format!("0x{:x}", window)
    }

    /// Get window icon from _NET_WM_ICON property, scaled to 20x20 BGRA
    fn get_window_icon(&mut self, window: Window) -> Option<&CachedIcon> {
        const ICON_SIZE: u32 = 20;

        // Check cache first
        if self.icon_cache.contains_key(&window) {
            return self.icon_cache.get(&window).and_then(|o| o.as_ref());
        }

        // Try to fetch _NET_WM_ICON
        let icon = self.fetch_and_process_icon(window, ICON_SIZE);
        self.icon_cache.insert(window, icon);
        self.icon_cache.get(&window).and_then(|o| o.as_ref())
    }

    /// Fetch _NET_WM_ICON and process it into a 20x20 BGRA image
    fn fetch_and_process_icon(&self, window: Window, target_size: u32) -> Option<CachedIcon> {
        // Request a large amount to get all icon sizes
        let reply = self.conn.get_property(
            false,
            window,
            self.atoms.net_wm_icon,
            AtomEnum::CARDINAL,
            0,
            u32::MAX / 4, // Max reasonable size
        ).ok()?.reply().ok()?;

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
        let scaled = Self::scale_icon(pixels, src_w, src_h, target_size);

        Some(CachedIcon { pixels: scaled })
    }

    /// Scale ARGB32 icon to target size and convert to BGRA
    fn scale_icon(src: &[u32], src_w: u32, src_h: u32, dst_size: u32) -> Vec<u8> {
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

    /// Redraw tab bars that contain a specific window (used when icon changes)
    fn redraw_tabs_for_window(&mut self, window: Window) -> Result<()> {
        let ws_idx = self.workspaces.current_index();

        // Find the frame containing this window
        if let Some(frame_id) = self.workspaces.current().layout.find_window(window) {
            // Get tab bar window for this frame
            if let Some(&tab_window) = self.tab_bar_windows.get(&(ws_idx, frame_id)) {
                // Get frame geometry
                let screen_rect = self.usable_screen();
                let geometries = self.workspaces.current().layout.calculate_geometries(
                    screen_rect,
                    self.config.gap,
                );

                if let Some(rect) = geometries.iter().find(|(fid, _)| *fid == frame_id).map(|(_, r)| r.clone()) {
                    self.draw_tab_bar(frame_id, tab_window, &rect)?;
                    self.conn.flush()?;
                }
            }
        }

        Ok(())
    }

    /// Remove tab bar windows for frames that no longer exist
    fn cleanup_tab_bars(&mut self) {
        let ws_idx = self.workspaces.current_index();
        let valid_frames: std::collections::HashSet<_> = self.workspaces.current().layout.all_frames().into_iter().collect();
        let to_remove: Vec<_> = self.tab_bar_windows
            .keys()
            .filter(|(idx, frame_id)| *idx == ws_idx && !valid_frames.contains(frame_id))
            .copied()
            .collect();

        for key in to_remove {
            if let Some(window) = self.tab_bar_windows.remove(&key) {
                if let Err(e) = self.conn.destroy_window(window) {
                    log::error!("Failed to destroy tab bar window: {}", e);
                }
            }
        }
    }

    /// Apply the current layout to all windows
    fn apply_layout(&mut self) -> Result<()> {
        let screen_rect = self.usable_screen();
        let geometries = self.workspaces.current().layout.calculate_geometries(screen_rect, self.config.gap);

        // Collect frame info for tab bar management
        let mut frames_with_tabs: Vec<(NodeId, Rect, usize)> = Vec::new();

        for (frame_id, rect) in &geometries {
            if let Some(frame) = self.workspaces.current().layout.get(*frame_id).and_then(|n| n.as_frame()) {
                let border = self.config.border_width;
                let tab_bar_height = self.config.tab_bar_height;

                // Calculate client area (below tab bar)
                // Always show tab bar, even for empty frames (to allow middle-click removal)
                let has_tabs = true;
                let client_y = if has_tabs {
                    rect.y + tab_bar_height as i32
                } else {
                    rect.y
                };
                let client_height = if has_tabs {
                    rect.height.saturating_sub(tab_bar_height)
                } else {
                    rect.height
                };

                if has_tabs {
                    log::debug!("Frame {:?} has {} windows, will show tab bar", frame_id, frame.windows.len());
                    frames_with_tabs.push((*frame_id, rect.clone(), frame.windows.len()));
                } else {
                    // Hide tab bar for single-window frames
                    let ws_idx = self.workspaces.current_index();
                    if let Some(&tab_window) = self.tab_bar_windows.get(&(ws_idx, *frame_id)) {
                        self.conn.unmap_window(tab_window)?;
                    }
                }

                // Only show the focused window, unmap others
                for (i, &window) in frame.windows.iter().enumerate() {
                    if i == frame.focused {
                        // Configure and map the focused window
                        self.conn.configure_window(
                            window,
                            &ConfigureWindowAux::new()
                                .x(rect.x)
                                .y(client_y)
                                .width(rect.width.saturating_sub(border * 2))
                                .height(client_height.saturating_sub(border * 2))
                                .border_width(border),
                        )?;
                        self.conn.map_window(window)?;
                        // Remove from hidden set since it's now visible
                        self.hidden_windows.remove(&window);
                    } else {
                        // Unmap non-focused windows (tabs)
                        // Track that we intentionally hid this window
                        self.hidden_windows.insert(window);
                        self.conn.unmap_window(window)?;
                    }
                }
            }
        }

        // Create/update tab bars for frames with multiple windows
        for (frame_id, rect, _) in frames_with_tabs {
            let tab_window = self.get_or_create_tab_bar(frame_id, &rect)?;
            log::info!("Tab bar window 0x{:x} for frame {:?} at ({}, {}) {}x{}",
                tab_window, frame_id, rect.x, rect.y, rect.width, self.config.tab_bar_height);
            self.conn.map_window(tab_window)?;
            // Raise the tab bar above client windows
            self.conn.configure_window(
                tab_window,
                &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
            )?;
            self.draw_tab_bar(frame_id, tab_window, &rect)?;
        }

        // Clean up tab bars for removed frames
        self.cleanup_tab_bars();

        self.conn.flush()?;
        Ok(())
    }

    /// Grab keys we want to handle
    fn grab_keys(&self) -> Result<()> {
        // Get keyboard mapping to find keycodes
        let setup = self.conn.setup();
        let min_keycode = setup.min_keycode;
        let max_keycode = setup.max_keycode;

        let mapping = self
            .conn
            .get_keyboard_mapping(min_keycode, max_keycode - min_keycode + 1)?
            .reply()?;

        let keysyms_per_keycode = mapping.keysyms_per_keycode as usize;

        // Build keysym -> keycode map
        let mut keysym_to_keycode: HashMap<u32, Keycode> = HashMap::new();
        for (i, chunk) in mapping.keysyms.chunks(keysyms_per_keycode).enumerate() {
            for keysym in chunk {
                if *keysym != 0 {
                    keysym_to_keycode
                        .entry(*keysym)
                        .or_insert(min_keycode + i as u8);
                }
            }
        }

        // Grab all configured keybindings
        for (action, binding) in &self.keybindings {
            if let Some(&keycode) = keysym_to_keycode.get(&binding.keysym) {
                let modmask = ModMask::from(binding.modifiers);
                self.grab_key(keycode, modmask)?;
                log::info!(
                    "Grabbed {:?} (keycode {}, mods 0x{:x})",
                    action,
                    keycode,
                    binding.modifiers
                );
            } else {
                log::warn!(
                    "Could not find keycode for {:?} (keysym 0x{:x})",
                    action,
                    binding.keysym
                );
            }
        }

        self.conn.flush()?;
        Ok(())
    }

    /// Grab a single key combination
    fn grab_key(&self, keycode: Keycode, modifiers: ModMask) -> Result<()> {
        // Grab with and without NumLock/CapsLock to handle those states
        let numlock = ModMask::M2; // NumLock is usually Mod2
        let capslock = ModMask::LOCK;

        for extra_mods in [
            ModMask::from(0u16),
            capslock,
            numlock,
            capslock | numlock,
        ] {
            self.conn.grab_key(
                false, // owner_events
                self.root,
                modifiers | extra_mods,
                keycode,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
            )?;
        }
        Ok(())
    }

    /// Manage any existing windows
    fn scan_existing_windows(&mut self) -> Result<()> {
        let tree = self.conn.query_tree(self.root)?.reply()?;

        for &window in &tree.children {
            let attrs = self.conn.get_window_attributes(window)?.reply()?;

            // Skip windows that are:
            // - override_redirect (popups, menus, etc.)
            // - not viewable (unmapped)
            if attrs.override_redirect || attrs.map_state != MapState::VIEWABLE {
                continue;
            }

            log::info!("Found existing window 0x{:x}", window);
            self.manage_window(window)?;
        }

        Ok(())
    }

    /// Start managing a window
    fn manage_window(&mut self, window: Window) -> Result<()> {
        // Check if already managed
        if self.workspaces.current().layout.find_window(window).is_some() {
            return Ok(());
        }

        log::info!("Managing window 0x{:x}", window);

        // Set border color
        self.conn.change_window_attributes(
            window,
            &ChangeWindowAttributesAux::new()
                .border_pixel(self.config.border_focused),
        )?;

        // Subscribe to events on this window
        self.conn.change_window_attributes(
            window,
            &ChangeWindowAttributesAux::new()
                .event_mask(EventMask::ENTER_WINDOW | EventMask::FOCUS_CHANGE | EventMask::PROPERTY_CHANGE),
        )?;

        // Map the window (make it visible)
        self.conn.map_window(window)?;

        // Add to the focused frame in our layout
        self.workspaces.current_mut().layout.add_window(window);

        // Trace the window being managed
        if let Some(frame_id) = self.workspaces.current().layout.find_window(window) {
            self.tracer.trace_transition(&StateTransition::WindowManaged {
                window,
                frame: format!("{:?}", frame_id),
            });
        }

        // Apply layout to position all windows
        self.apply_layout()?;

        // Update EWMH client list
        self.update_client_list()?;

        // Focus this window
        self.focus_window(window)?;

        self.conn.flush()?;
        Ok(())
    }

    /// Unmanage a window
    fn unmanage_window(&mut self, window: Window) -> Result<()> {
        // Cancel drag if we're dragging this window
        if let Some(DragState::Tab { window: dragged_window, .. }) = self.drag_state {
            if dragged_window == window {
                // Ungrab pointer and clear drag state
                self.conn.ungrab_pointer(x11rb::CURRENT_TIME)?;
                self.drag_state = None;
                log::info!("Cancelled drag - dragged window was destroyed");
            }
        }

        // Remove from hidden set if present
        self.hidden_windows.remove(&window);

        // Remove from tagged set if present
        self.tagged_windows.remove(&window);

        // Trace before removing
        if self.workspaces.current().layout.find_window(window).is_some() {
            self.tracer.trace_transition(&StateTransition::WindowUnmanaged {
                window,
                reason: UnmanageReason::ClientDestroyed,
            });
        }

        if let Some(_frame_id) = self.workspaces.current_mut().layout.remove_window(window) {
            log::info!("Unmanaging window 0x{:x}", window);

            // Update EWMH client list
            self.update_client_list()?;

            // If this was focused, focus another window
            if self.focused_window == Some(window) {
                self.focused_window = None;

                // Try to focus the window in the focused frame
                if let Some(frame) = self.workspaces.current().layout.focused_frame() {
                    if let Some(w) = frame.focused_window() {
                        self.focus_window(w)?;
                    }
                }

                // If still no focus, try any window
                if self.focused_window.is_none() {
                    let windows = self.workspaces.current().layout.all_windows();
                    if let Some(&w) = windows.first() {
                        self.focus_window(w)?;
                    } else {
                        self.update_active_window()?;
                    }
                }
            }

            // Re-apply layout
            self.apply_layout()?;
        }

        Ok(())
    }

    /// Cycle focus to the next/previous window (across all frames)
    fn cycle_focus(&mut self, forward: bool) -> Result<()> {
        let windows = self.workspaces.current().layout.all_windows();
        if windows.is_empty() {
            return Ok(());
        }

        let current_idx = self.focused_window
            .and_then(|w| windows.iter().position(|&x| x == w))
            .unwrap_or(0);

        let next_idx = if forward {
            (current_idx + 1) % windows.len()
        } else {
            if current_idx == 0 {
                windows.len() - 1
            } else {
                current_idx - 1
            }
        };

        let window = windows[next_idx];
        self.focus_window(window)?;

        Ok(())
    }

    /// Cycle tabs within the focused frame
    fn cycle_tab(&mut self, forward: bool) -> Result<()> {
        // Capture old tab index for tracing
        let old_tab = self.workspaces.current().layout.focused_frame().map(|f| f.focused);

        if let Some(window) = self.workspaces.current_mut().layout.cycle_tab(forward) {
            // Trace the tab switch
            if let (Some(old), Some(frame)) = (old_tab, self.workspaces.current().layout.focused_frame()) {
                self.tracer.trace_transition(&StateTransition::TabSwitched {
                    frame: format!("{:?}", self.workspaces.current().layout.focused),
                    from: old,
                    to: frame.focused,
                });
            }

            self.apply_layout()?;
            self.focus_window(window)?;
            log::info!("Cycled to {} tab", if forward { "next" } else { "previous" });
        }
        Ok(())
    }

    /// Focus a specific tab by number (1-based for user, 0-based internally)
    fn focus_tab(&mut self, num: usize) -> Result<()> {
        // Capture old tab index for tracing
        let old_tab = self.workspaces.current().layout.focused_frame().map(|f| f.focused);

        if let Some(window) = self.workspaces.current_mut().layout.focus_tab(num.saturating_sub(1)) {
            // Trace the tab switch
            if let (Some(old), Some(frame)) = (old_tab, self.workspaces.current().layout.focused_frame()) {
                if old != frame.focused {
                    self.tracer.trace_transition(&StateTransition::TabSwitched {
                        frame: format!("{:?}", self.workspaces.current().layout.focused),
                        from: old,
                        to: frame.focused,
                    });
                }
            }

            self.apply_layout()?;
            self.focus_window(window)?;
            log::info!("Focused tab {}", num);
        }
        Ok(())
    }

    /// Split the focused frame
    fn split_focused(&mut self, direction: SplitDirection) -> Result<()> {
        let old_frame = self.workspaces.current().layout.focused;
        self.workspaces.current_mut().layout.split_focused(direction);
        let new_frame = self.workspaces.current().layout.focused;

        // Trace the split
        self.tracer.trace_transition(&StateTransition::FrameSplit {
            original_frame: format!("{:?}", old_frame),
            new_frame: format!("{:?}", new_frame),
            direction: format!("{:?}", direction),
        });

        self.apply_layout()?;
        log::info!("Split {:?}", direction);
        Ok(())
    }

    /// Focus frame in the given spatial direction
    fn focus_frame(&mut self, direction: Direction) -> Result<()> {
        let old_focused_frame = self.workspaces.current().layout.focused;
        let screen_rect = self.usable_screen();
        let geometries = self.workspaces.current().layout.calculate_geometries(screen_rect, self.config.gap);

        if self.workspaces.current_mut().layout.focus_spatial(direction, &geometries) {
            let new_focused_frame = self.workspaces.current().layout.focused;

            // Focus the window in the new frame
            if let Some(frame) = self.workspaces.current().layout.focused_frame() {
                if let Some(window) = frame.focused_window() {
                    self.focus_window(window)?;
                }
            }

            // Redraw tab bars for old and new focused frames
            if old_focused_frame != new_focused_frame {
                let geometry_map: std::collections::HashMap<_, _> = geometries.into_iter().collect();
                let ws_idx = self.workspaces.current_index();

                if let Some(&tab_window) = self.tab_bar_windows.get(&(ws_idx, old_focused_frame)) {
                    if let Some(rect) = geometry_map.get(&old_focused_frame) {
                        self.draw_tab_bar(old_focused_frame, tab_window, rect)?;
                    }
                }
                if let Some(&tab_window) = self.tab_bar_windows.get(&(ws_idx, new_focused_frame)) {
                    if let Some(rect) = geometry_map.get(&new_focused_frame) {
                        self.draw_tab_bar(new_focused_frame, tab_window, rect)?;
                    }
                }

                self.conn.flush()?;
            }
        }
        Ok(())
    }

    /// Focus a window
    fn focus_window(&mut self, window: Window) -> Result<()> {
        // Capture old focus for tracing
        let old_focused = self.focused_window;

        // Unfocus the previously focused window
        if let Some(old) = self.focused_window {
            if old != window && self.workspaces.current().layout.find_window(old).is_some() {
                self.conn.change_window_attributes(
                    old,
                    &ChangeWindowAttributesAux::new()
                        .border_pixel(self.config.border_unfocused),
                )?;
            }
        }

        // Focus the new window
        self.conn.set_input_focus(InputFocus::POINTER_ROOT, window, x11rb::CURRENT_TIME)?;

        // Raise the window
        self.conn.configure_window(
            window,
            &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
        )?;

        // Set focused border color
        self.conn.change_window_attributes(
            window,
            &ChangeWindowAttributesAux::new()
                .border_pixel(self.config.border_focused),
        )?;

        self.focused_window = Some(window);

        // Trace focus change
        if old_focused != Some(window) {
            self.tracer.trace_transition(&StateTransition::FocusChanged {
                from: old_focused,
                to: Some(window),
            });
        }

        // Also update the layout's focused frame to match
        if let Some(frame_id) = self.workspaces.current().layout.find_window(window) {
            let old_focused_frame = self.workspaces.current().layout.focused;
            self.workspaces.current_mut().layout.focused = frame_id;
            let ws_idx = self.workspaces.current_index();

            // Re-raise the tab bar if this frame has one (so it stays above the window)
            if let Some(&tab_window) = self.tab_bar_windows.get(&(ws_idx, frame_id)) {
                self.conn.configure_window(
                    tab_window,
                    &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
                )?;
            }

            // Redraw tab bars (always redraw current frame, also old frame if different)
            let screen_rect = self.usable_screen();
            let geometries = self.workspaces.current().layout.calculate_geometries(screen_rect, self.config.gap);
            let geometry_map: std::collections::HashMap<_, _> = geometries.into_iter().collect();

            // Redraw old focused frame's tab bar if it changed
            if old_focused_frame != frame_id {
                if let Some(&tab_window) = self.tab_bar_windows.get(&(ws_idx, old_focused_frame)) {
                    if let Some(rect) = geometry_map.get(&old_focused_frame) {
                        self.draw_tab_bar(old_focused_frame, tab_window, rect)?;
                    }
                }
            }

            // Always redraw current frame's tab bar (new tabs, focus changes, etc.)
            if let Some(&tab_window) = self.tab_bar_windows.get(&(ws_idx, frame_id)) {
                if let Some(rect) = geometry_map.get(&frame_id) {
                    self.draw_tab_bar(frame_id, tab_window, rect)?;
                }
            }
        }

        // Update EWMH active window
        self.update_active_window()?;

        self.conn.flush()?;

        Ok(())
    }

    /// Check if a window supports the WM_DELETE_WINDOW protocol
    fn supports_delete_protocol(&self, window: Window) -> bool {
        // Get WM_PROTOCOLS property
        if let Ok(cookie) = self.conn.get_property(
            false,
            window,
            self.atoms.wm_protocols,
            AtomEnum::ATOM,
            0,
            32,
        ) {
            if let Ok(reply) = cookie.reply() {
                if let Some(atoms) = reply.value32() {
                    return atoms.into_iter().any(|a| a == self.atoms.wm_delete_window);
                }
            }
        }
        false
    }

    /// Send WM_DELETE_WINDOW client message to request graceful close
    fn send_delete_window(&self, window: Window) -> Result<()> {
        let data = ClientMessageData::from([self.atoms.wm_delete_window, 0u32, 0u32, 0u32, 0u32]);
        let event = ClientMessageEvent {
            response_type: CLIENT_MESSAGE_EVENT,
            format: 32,
            sequence: 0,
            window,
            type_: self.atoms.wm_protocols,
            data,
        };
        self.conn.send_event(
            false,
            window,
            EventMask::NO_EVENT,
            event,
        )?;
        self.conn.flush()?;
        Ok(())
    }

    /// Close the focused window gracefully
    fn close_focused_window(&self) -> Result<()> {
        if let Some(window) = self.focused_window {
            log::info!("Closing window 0x{:x}", window);

            if self.supports_delete_protocol(window) {
                log::debug!("Using WM_DELETE_WINDOW protocol");
                self.send_delete_window(window)?;
            } else {
                log::debug!("Window doesn't support WM_DELETE_WINDOW, killing client");
                self.conn.kill_client(window)?;
                self.conn.flush()?;
            }
        }
        Ok(())
    }

    /// Handle EWMH ClientMessage events
    fn handle_client_message(&mut self, event: ClientMessageEvent) -> Result<()> {
        let msg_type = event.type_;
        self.tracer.trace_x11_event("ClientMessage", Some(event.window), &format!("type={}", msg_type));

        if msg_type == self.atoms.net_active_window {
            // _NET_ACTIVE_WINDOW: Focus the window
            let window = event.window;
            log::info!("ClientMessage: _NET_ACTIVE_WINDOW for 0x{:x}", window);

            // Check if window is on current workspace
            if self.workspaces.current().layout.find_window(window).is_some() {
                self.suppress_enter_focus = true;
                self.focus_window(window)?;
            } else {
                // Check other workspaces and switch if found
                for (idx, ws) in self.workspaces.workspaces.iter().enumerate() {
                    if ws.layout.find_window(window).is_some() {
                        // Switch to that workspace, then focus
                        if let Some(old_idx) = self.workspaces.switch_to(idx) {
                            self.perform_workspace_switch(old_idx)?;
                            self.suppress_enter_focus = true;
                            self.focus_window(window)?;
                        }
                        break;
                    }
                }
            }
        } else if msg_type == self.atoms.net_close_window {
            // _NET_CLOSE_WINDOW: Close the window
            let window = event.window;
            log::info!("ClientMessage: _NET_CLOSE_WINDOW for 0x{:x}", window);

            if self.supports_delete_protocol(window) {
                self.send_delete_window(window)?;
            } else {
                self.conn.kill_client(window)?;
                self.conn.flush()?;
            }
        } else if msg_type == self.atoms.net_current_desktop {
            // _NET_CURRENT_DESKTOP: Switch to workspace
            let desktop = event.data.as_data32()[0] as usize;
            log::info!("ClientMessage: _NET_CURRENT_DESKTOP to {}", desktop);

            if let Some(old_idx) = self.workspaces.switch_to(desktop) {
                self.perform_workspace_switch(old_idx)?;
            }
        } else if msg_type == self.atoms.net_wm_desktop {
            // _NET_WM_DESKTOP: Move window to workspace
            let window = event.window;
            let desktop = event.data.as_data32()[0] as usize;
            log::info!("ClientMessage: _NET_WM_DESKTOP move 0x{:x} to {}", window, desktop);

            self.move_window_to_workspace(window, desktop)?;
        }

        Ok(())
    }

    /// Move a window to a different workspace
    fn move_window_to_workspace(&mut self, window: Window, target: usize) -> Result<()> {
        if target >= 9 {
            return Ok(());
        }

        let current_ws = self.workspaces.current_index();

        // Find which workspace has this window
        let source_ws = self.workspaces.workspaces.iter()
            .enumerate()
            .find(|(_, ws)| ws.layout.find_window(window).is_some())
            .map(|(idx, _)| idx);

        let Some(source_ws) = source_ws else {
            return Ok(()); // Window not found
        };

        if source_ws == target {
            return Ok(()); // Already on target workspace
        }

        // Remove from source workspace
        self.workspaces.workspaces[source_ws].layout.remove_window(window);
        self.workspaces.workspaces[source_ws].layout.remove_empty_frames();

        // Add to target workspace
        self.workspaces.workspaces[target].layout.add_window(window);

        // Update window's _NET_WM_DESKTOP property
        self.set_window_desktop(window, target)?;

        // If moving from current workspace, hide the window
        if source_ws == current_ws {
            self.hidden_windows.insert(window);
            self.conn.unmap_window(window)?;

            // If this was the focused window, focus something else
            if self.focused_window == Some(window) {
                self.focused_window = None;
                if let Some(frame) = self.workspaces.current().layout.focused_frame() {
                    if let Some(w) = frame.focused_window() {
                        self.focus_window(w)?;
                    }
                }
            }
        }

        // If moving to current workspace, show and map the window
        if target == current_ws {
            self.hidden_windows.remove(&window);
        }

        self.apply_layout()?;
        self.update_client_list()?;

        log::info!("Moved window 0x{:x} from workspace {} to {}", window, source_ws + 1, target + 1);
        Ok(())
    }

    /// Handle an X11 event
    fn handle_event(&mut self, event: Event) -> Result<()> {
        match event {
            Event::MapRequest(e) => {
                self.tracer.trace_x11_event("MapRequest", Some(e.window), "");
                log::debug!("MapRequest for window 0x{:x}", e.window);
                self.manage_window(e.window)?;
            }

            Event::UnmapNotify(e) => {
                self.tracer.trace_x11_event("UnmapNotify", Some(e.window), "");
                log::debug!("UnmapNotify for window 0x{:x}", e.window);
                // Only unmanage if the event is about a window we manage
                // and not from a reparent operation
                // Also skip if we intentionally hid this window (it's a hidden tab)
                if e.event == self.root && !self.hidden_windows.contains(&e.window) {
                    if let Err(e) = self.unmanage_window(e.window) {
                        log::error!("Failed to unmanage window: {}", e);
                    }
                }
            }

            Event::DestroyNotify(e) => {
                self.tracer.trace_x11_event("DestroyNotify", Some(e.window), "");
                log::debug!("DestroyNotify for window 0x{:x}", e.window);
                if let Err(err) = self.unmanage_window(e.window) {
                    log::error!("Failed to unmanage window: {}", err);
                }
            }

            Event::ConfigureRequest(e) => {
                self.tracer.trace_x11_event("ConfigureRequest", Some(e.window), "");
                // For now, allow all configure requests
                log::debug!("ConfigureRequest for window 0x{:x}", e.window);

                // If we're managing this window, re-apply layout (ignore client's request)
                if self.workspaces.current().layout.find_window(e.window).is_some() {
                    self.apply_layout()?;
                } else {
                    // Unmanaged window - allow the configure
                    let aux = ConfigureWindowAux::from_configure_request(&e);
                    self.conn.configure_window(e.window, &aux)?;
                    self.conn.flush()?;
                }
            }

            Event::EnterNotify(e) => {
                self.tracer.trace_x11_event("EnterNotify", Some(e.event), "");
                // Focus follows mouse (unless suppressed after explicit focus)
                if !self.suppress_enter_focus {
                    if self.workspaces.current().layout.find_window(e.event).is_some() {
                        log::debug!("EnterNotify for window 0x{:x}", e.event);
                        self.focus_window(e.event)?;
                    }
                }
                self.suppress_enter_focus = false;
            }

            Event::KeyPress(e) => {
                self.tracer.trace_x11_event("KeyPress", None, &format!("keycode={}", e.detail));
                self.handle_key_press(e)?;
            }

            Event::Expose(e) => {
                self.tracer.trace_x11_event("Expose", Some(e.window), "");
                // Redraw tab bar if it's one of ours
                self.handle_expose(e)?;
            }

            Event::PropertyNotify(e) => {
                // Invalidate icon cache if _NET_WM_ICON changed
                if e.atom == self.atoms.net_wm_icon {
                    self.icon_cache.remove(&e.window);
                    // Redraw tab bars that might show this window
                    self.redraw_tabs_for_window(e.window)?;
                }
                // Redraw tab bar if title changed
                if e.atom == self.atoms.net_wm_name || e.atom == u32::from(AtomEnum::WM_NAME) {
                    self.redraw_tabs_for_window(e.window)?;
                }
            }

            Event::ButtonPress(e) => {
                self.tracer.trace_x11_event("ButtonPress", Some(e.event), &format!("button={}", e.detail));
                // Handle clicks on tab bars
                self.handle_button_press(e)?;
            }

            Event::ButtonRelease(e) => {
                self.tracer.trace_x11_event("ButtonRelease", Some(e.event), &format!("button={}", e.detail));
                // Handle end of drag
                self.handle_button_release(e)?;
            }

            Event::MotionNotify(e) => {
                // Handle resize drag - update split ratio in real-time
                if let Some(DragState::Resize { split_id, direction, split_start, total_size }) = &self.drag_state {
                    // Calculate new ratio from mouse position
                    let mouse_pos = match direction {
                        SplitDirection::Horizontal => e.root_x as i32,
                        SplitDirection::Vertical => e.root_y as i32,
                    };
                    let ratio = ((mouse_pos - split_start) as f32) / (*total_size as f32);

                    // Update split and relayout
                    if self.workspaces.current_mut().layout.set_split_ratio(*split_id, ratio) {
                        self.apply_layout()?;
                    }
                }
                // Tab drags don't need motion processing - drop target determined at release
            }

            Event::ClientMessage(e) => {
                self.handle_client_message(e)?;
            }

            Event::MappingNotify(e) => {
                self.tracer.trace_x11_event("MappingNotify", None, &format!("request={:?}", e.request));
                // Re-grab keys when keyboard mapping changes (Modifier or Keyboard, not Pointer)
                if e.request != Mapping::POINTER {
                    log::info!("Keyboard mapping changed, re-grabbing keys");
                    self.grab_keys()?;
                }
            }

            _ => {
                // Ignore other events for now
            }
        }

        Ok(())
    }

    /// Handle expose event (redraw tab bar)
    fn handle_expose(&mut self, event: ExposeEvent) -> Result<()> {
        let ws_idx = self.workspaces.current_index();
        // Find which frame this tab bar belongs to
        for (&(idx, frame_id), &tab_window) in &self.tab_bar_windows {
            if idx == ws_idx && tab_window == event.window {
                // Get frame geometry to redraw
                let screen_rect = self.usable_screen();
                let geometries = self.workspaces.current().layout.calculate_geometries(screen_rect, self.config.gap);
                for (fid, rect) in geometries {
                    if fid == frame_id {
                        self.draw_tab_bar(frame_id, tab_window, &rect)?;
                        self.conn.flush()?;
                        break;
                    }
                }
                break;
            }
        }
        Ok(())
    }

    /// Try to handle a gap resize drag initiation.
    /// Returns Ok(true) if the click started a resize operation, Ok(false) otherwise.
    fn try_handle_gap_resize(&mut self, event: &ButtonPressEvent) -> Result<bool> {
        // Only handle left-clicks on root window
        if event.event != self.root || event.detail != 1 {
            return Ok(false);
        }

        let screen = self.usable_screen();
        if let Some((split_id, direction, split_start, total_size)) =
            self.workspaces.current().layout.find_split_at_gap(screen, self.config.gap, event.root_x as i32, event.root_y as i32)
        {
            // Select the appropriate resize cursor based on split direction
            let resize_cursor = match direction {
                SplitDirection::Horizontal => self.cursor_resize_h,
                SplitDirection::Vertical => self.cursor_resize_v,
            };

            // Start resize drag - grab pointer to track motion
            self.conn.grab_pointer(
                false,
                self.root,
                EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
                x11rb::NONE,  // confine_to
                resize_cursor,
                x11rb::CURRENT_TIME,
            )?;

            self.drag_state = Some(DragState::Resize {
                split_id,
                direction,
                split_start,
                total_size,
            });

            log::info!("Started gap resize for {:?} split", direction);
            return Ok(true);
        }

        Ok(false)
    }

    /// Try to handle a click on an empty frame area.
    /// Returns Ok(true) if an empty frame was focused, Ok(false) otherwise.
    fn try_handle_empty_frame_click(&mut self, event: &ButtonPressEvent) -> Result<bool> {
        // Only handle left-clicks on root window
        if event.event != self.root || event.detail != 1 {
            return Ok(false);
        }

        let screen = self.usable_screen();
        let geometries = self.workspaces.current().layout.calculate_geometries(screen, self.config.gap);

        for (frame_id, rect) in &geometries {
            if let Some(frame) = self.workspaces.current().layout.get(*frame_id).and_then(|n| n.as_frame()) {
                if frame.is_empty() {
                    let click_x = event.root_x as i32;
                    let click_y = event.root_y as i32;
                    if click_x >= rect.x && click_x < rect.x + rect.width as i32 &&
                       click_y >= rect.y && click_y < rect.y + rect.height as i32 {
                        // Focus this empty frame
                        self.workspaces.current_mut().layout.focused = *frame_id;
                        self.apply_layout()?;
                        return Ok(true);
                    }
                }
            }
        }

        Ok(false)
    }

    /// Handle a click on a tab bar (tab selection, drag, or middle-click removal).
    fn handle_tab_click(&mut self, event: &ButtonPressEvent, frame_id: NodeId) -> Result<()> {
        let ws_idx = self.workspaces.current_index();

        // Handle middle click - remove empty frame
        if event.detail == 2 {
            if let Some(frame) = self.workspaces.current().layout.get(frame_id).and_then(|n| n.as_frame()) {
                if frame.is_empty() {
                    // Remove tab bar window
                    if let Some(tab_window) = self.tab_bar_windows.remove(&(ws_idx, frame_id)) {
                        self.conn.destroy_window(tab_window)?;
                    }
                    // Remove empty frame from layout
                    self.workspaces.current_mut().layout.remove_empty_frames();
                    self.apply_layout()?;
                    log::info!("Removed empty frame via middle-click");
                }
            }
            return Ok(());
        }

        // Only handle left click for tab selection/drag
        if event.detail != 1 {
            return Ok(());
        }

        // Get frame and handle click
        if let Some(frame) = self.workspaces.current().layout.get(frame_id).and_then(|n| n.as_frame()) {
            let num_tabs = frame.windows.len();
            if num_tabs == 0 {
                // Focus the empty frame
                self.workspaces.current_mut().layout.focused = frame_id;
                self.apply_layout()?;
                return Ok(());
            }

            // Calculate which tab was clicked using content-based layout
            let tab_layout = self.calculate_tab_layout(frame_id);
            let click_x = event.event_x as i16;
            let clicked_tab = tab_layout.iter().enumerate()
                .find(|(_, (x, w))| click_x >= *x && click_x < *x + *w as i16)
                .map(|(i, _)| i);

            if let Some(clicked_tab) = clicked_tab {
                // Get the window at this tab
                let window = frame.windows[clicked_tab];

                // Focus this tab immediately
                if let Some(w) = self.workspaces.current_mut().layout.focus_tab(clicked_tab) {
                    self.apply_layout()?;
                    self.focus_window(w)?;
                }

                // Start drag operation - grab pointer to track motion
                self.conn.grab_pointer(
                    false,
                    self.root,
                    EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                    x11rb::NONE,  // confine_to
                    x11rb::NONE,  // cursor
                    x11rb::CURRENT_TIME,
                )?;

                self.drag_state = Some(DragState::Tab {
                    window,
                    source_frame: frame_id,
                    source_index: clicked_tab,
                });

                log::info!("Started drag for tab {} (window 0x{:x})", clicked_tab + 1, window);
            }
        }

        Ok(())
    }

    /// Handle button press event (click on tab bar or gap for resize)
    fn handle_button_press(&mut self, event: ButtonPressEvent) -> Result<()> {
        // Check for gap resize or empty frame click on root window
        if self.try_handle_gap_resize(&event)? {
            return Ok(());
        }
        if self.try_handle_empty_frame_click(&event)? {
            return Ok(());
        }

        // Find which frame's tab bar was clicked
        let ws_idx = self.workspaces.current_index();
        let clicked_frame = self.tab_bar_windows.iter()
            .find(|(&(idx, _), &tab_window)| idx == ws_idx && tab_window == event.event)
            .map(|(&(_, frame_id), _)| frame_id);

        if let Some(frame_id) = clicked_frame {
            self.handle_tab_click(&event, frame_id)?;
        }

        Ok(())
    }

    /// Find the drop target for a drag operation
    /// Returns (frame_id, tab_index) - tab_index is the position to insert at
    fn find_drop_target(&self, root_x: i16, root_y: i16) -> Result<(Option<NodeId>, Option<usize>)> {
        let ws_idx = self.workspaces.current_index();
        // Check each tab bar window first (higher priority than content area)
        for (&(idx, frame_id), &tab_window) in &self.tab_bar_windows {
            if idx != ws_idx {
                continue;
            }
            let geom = self.conn.get_geometry(tab_window)?.reply()?;
            let coords = self.conn.translate_coordinates(tab_window, self.root, 0, 0)?.reply()?;

            let tab_x = coords.dst_x as i16;
            let tab_y = coords.dst_y as i16;

            if root_x >= tab_x && root_x < tab_x + geom.width as i16 &&
               root_y >= tab_y && root_y < tab_y + geom.height as i16 {
                // Cursor is over this tab bar
                // Calculate which tab position using content-based layout
                let tab_layout = self.calculate_tab_layout(frame_id);
                let local_x = root_x - tab_x;
                let target_index = tab_layout.iter().enumerate()
                    .find(|(_, (x, w))| local_x >= *x && local_x < *x + *w as i16)
                    .map(|(i, _)| i);

                if let Some(idx) = target_index {
                    return Ok((Some(frame_id), Some(idx)));
                }
                return Ok((Some(frame_id), None));
            }
        }

        // Check frame content areas (for dropping into single-window frames or frames without visible tab bars)
        let screen_rect = self.usable_screen();
        let geometries = self.workspaces.current().layout.calculate_geometries(screen_rect, self.config.gap);

        for (frame_id, rect) in geometries {
            if (root_x as i32) >= rect.x && (root_x as i32) < rect.x + rect.width as i32 &&
               (root_y as i32) >= rect.y && (root_y as i32) < rect.y + rect.height as i32 {
                return Ok((Some(frame_id), None));
            }
        }

        Ok((None, None))
    }

    /// Handle button release event (end of drag)
    fn handle_button_release(&mut self, event: ButtonReleaseEvent) -> Result<()> {
        // Only handle left button
        if event.detail != 1 {
            return Ok(());
        }

        // Ungrab pointer
        self.conn.ungrab_pointer(x11rb::CURRENT_TIME)?;
        self.conn.flush()?;

        let drag = match self.drag_state.take() {
            Some(d) => d,
            None => return Ok(()),
        };

        match drag {
            DragState::Tab { window, source_frame, source_index } => {
                // Find what's under the cursor at root coordinates
                let (target_frame, target_index) = self.find_drop_target(event.root_x, event.root_y)?;

                if let Some(target_frame) = target_frame {
                    if target_frame == source_frame {
                        // Reorder within same frame
                        if let Some(target_idx) = target_index {
                            if target_idx != source_index {
                                self.workspaces.current_mut().layout.reorder_tab(target_frame, source_index, target_idx);
                                log::info!("Reordered tab from {} to {}", source_index + 1, target_idx + 1);
                            }
                        }
                    } else {
                        // Move to different frame
                        self.workspaces.current_mut().layout.move_window_to_frame(window, source_frame, target_frame);

                        log::info!("Moved window 0x{:x} to different frame", window);
                    }

                    self.apply_layout()?;
                    self.suppress_enter_focus = true;
                    self.focus_window(window)?;
                } else {
                    log::info!("Drag cancelled - released outside any frame");
                }
            }
            DragState::Resize { .. } => {
                // Resize is complete - nothing more to do
                // (resizing happens during motion, not on release)
                log::info!("Resize drag completed");
            }
        }

        Ok(())
    }

    /// Resize the current split
    fn resize_split(&mut self, grow: bool) -> Result<()> {
        let delta = if grow { 0.05 } else { -0.05 };
        if self.workspaces.current_mut().layout.resize_focused_split(delta) {
            // Trace the resize (simplified - we don't track exact ratios)
            self.tracer.trace_transition(&StateTransition::SplitResized {
                split: format!("{:?}", self.workspaces.current().layout.focused),
                old_ratio: 0.5, // placeholder
                new_ratio: 0.5 + delta,
            });
            self.apply_layout()?;
            log::info!("Resized split by {}", delta);
        }
        Ok(())
    }

    /// Move the focused window to an adjacent frame
    fn move_window(&mut self, forward: bool) -> Result<()> {
        // Capture source frame before move
        let from_frame = self.workspaces.current().layout.focused;

        if let Some(window) = self.workspaces.current_mut().layout.move_window_to_adjacent(forward) {
            // Trace the move
            let to_frame = self.workspaces.current().layout.focused;
            self.tracer.trace_transition(&StateTransition::WindowMoved {
                window,
                from_frame: format!("{:?}", from_frame),
                to_frame: format!("{:?}", to_frame),
            });

            // Clean up empty frames
            self.workspaces.current_mut().layout.remove_empty_frames();
            self.apply_layout()?;
            self.suppress_enter_focus = true;
            self.focus_window(window)?;
            log::info!("Moved window 0x{:x} to {} frame", window, if forward { "next" } else { "previous" });
        }
        Ok(())
    }

    /// Handle a key press event
    fn handle_key_press(&mut self, event: KeyPressEvent) -> Result<()> {
        // Convert state to u16 and mask out NumLock and CapsLock for comparison
        let state_u16 = u16::from(event.state);
        let clean_state = state_u16 & !(u16::from(ModMask::M2) | u16::from(ModMask::LOCK));

        // Get the keysym for this keycode
        let setup = self.conn.setup();
        let min_keycode = setup.min_keycode;
        let max_keycode = setup.max_keycode;

        let mapping = self
            .conn
            .get_keyboard_mapping(min_keycode, max_keycode - min_keycode + 1)?
            .reply()?;

        let keysyms_per_keycode = mapping.keysyms_per_keycode as usize;
        let idx = (event.detail - min_keycode) as usize * keysyms_per_keycode;
        let keysym = mapping.keysyms.get(idx).copied().unwrap_or(0);

        log::debug!(
            "KeyPress: keycode={}, keysym=0x{:x}, state=0x{:x}, clean_state=0x{:x}",
            event.detail,
            keysym,
            state_u16,
            clean_state
        );

        // Find matching action from configured keybindings
        let mut matched_action = None;
        for (action, binding) in &self.keybindings {
            if binding.keysym == keysym && binding.modifiers == clean_state {
                matched_action = Some(action.clone());
                break;
            }
        }

        if let Some(action) = matched_action {
            self.execute_action(action)?;
        }

        Ok(())
    }

    /// Execute a window manager action
    fn execute_action(&mut self, action: WmAction) -> Result<()> {
        match action {
            WmAction::Spawn(ref command) => {
                log::info!("Spawning: {}", command);
                let parts: Vec<&str> = command.split_whitespace().collect();
                if let Some((program, args)) = parts.split_first() {
                    let mut cmd = Command::new(program);
                    cmd.args(args);
                    if let Err(e) = cmd.spawn() {
                        log::error!("Failed to spawn {}: {}", command, e);
                    }
                }
            }
            WmAction::CycleTabForward => self.cycle_tab(true)?,
            WmAction::CycleTabBackward => self.cycle_tab(false)?,
            WmAction::FocusNext => self.cycle_focus(true)?,
            WmAction::FocusPrev => self.cycle_focus(false)?,
            WmAction::FocusFrameLeft => self.focus_frame(Direction::Left)?,
            WmAction::FocusFrameRight => self.focus_frame(Direction::Right)?,
            WmAction::FocusFrameUp => self.focus_frame(Direction::Up)?,
            WmAction::FocusFrameDown => self.focus_frame(Direction::Down)?,
            WmAction::MoveWindowLeft => self.move_window(false)?,
            WmAction::MoveWindowRight => self.move_window(true)?,
            WmAction::ResizeShrink => self.resize_split(false)?,
            WmAction::ResizeGrow => self.resize_split(true)?,
            WmAction::SplitHorizontal => self.split_focused(SplitDirection::Horizontal)?,
            WmAction::SplitVertical => self.split_focused(SplitDirection::Vertical)?,
            WmAction::CloseWindow => self.close_focused_window()?,
            WmAction::Quit => {
                log::info!("Quitting window manager");
                self.running = false;
            }
            WmAction::FocusTab(n) => self.focus_tab(n)?,
            WmAction::WorkspaceNext => self.workspace_next()?,
            WmAction::WorkspacePrev => self.workspace_prev()?,
            WmAction::TagWindow => self.tag_focused_window()?,
            WmAction::MoveTaggedToFrame => self.move_tagged_to_focused_frame()?,
            WmAction::UntagAll => self.untag_all_windows()?,
        }
        Ok(())
    }

    /// Main event loop
    fn run(&mut self) -> Result<()> {
        log::info!("Entering event loop");

        while self.running {
            // Poll IPC commands (non-blocking)
            // We need to take the ipc out temporarily to avoid borrow conflicts
            if let Some(ipc) = self.ipc.take() {
                // Collect all pending commands
                let mut pending_commands = Vec::new();
                while let Some((cmd, client)) = ipc.poll() {
                    pending_commands.push((cmd, client));
                }

                // Put ipc back
                self.ipc = Some(ipc);

                // Now handle each command
                for (cmd, mut client) in pending_commands {
                    let response = self.handle_ipc(cmd);
                    if let Err(e) = client.respond(response) {
                        log::warn!("Failed to send IPC response: {}", e);
                    }
                }
            }

            // Poll for X11 events (non-blocking)
            match self.conn.poll_for_event() {
                Ok(Some(event)) => {
                    if let Err(e) = self.handle_event(event) {
                        log::error!("Error handling event: {}", e);
                    }
                }
                Ok(None) => {
                    // No event, sleep briefly to avoid busy-waiting
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(e) => {
                    log::error!("Error polling for X11 event: {}", e);
                }
            }
        }

        log::info!("Exiting window manager");
        Ok(())
    }

    /// Handle an IPC command and return a response
    fn handle_ipc(&mut self, cmd: IpcCommand) -> IpcResponse {
        log::debug!("Handling IPC command: {:?}", cmd);

        // Capture command name for tracing
        let cmd_name = format!("{:?}", cmd);

        let response = match cmd {
            IpcCommand::GetState => {
                IpcResponse::State {
                    data: self.snapshot_state(),
                }
            }
            IpcCommand::GetLayout => {
                let geometries = self.workspaces.current().layout.calculate_geometries(
                    self.usable_screen(),
                    self.config.gap,
                );
                IpcResponse::Layout {
                    data: self.workspaces.current().layout.snapshot(Some(&geometries)),
                }
            }
            IpcCommand::GetWindows => {
                IpcResponse::Windows {
                    data: self.get_window_info_list(),
                }
            }
            IpcCommand::GetFocused => {
                IpcResponse::Focused {
                    window: self.focused_window,
                }
            }
            IpcCommand::ValidateState => {
                let violations = self.validate_state();
                IpcResponse::Validation {
                    valid: violations.is_empty(),
                    violations,
                }
            }
            IpcCommand::GetEventLog { count } => {
                let entries = match count {
                    Some(n) => self.tracer.get_last(n),
                    None => self.tracer.get_all(),
                };
                IpcResponse::EventLog { entries }
            }
            IpcCommand::FocusWindow { window } => {
                match self.focus_window(window) {
                    Ok(()) => IpcResponse::Ok,
                    Err(e) => IpcResponse::Error {
                        code: "focus_failed".to_string(),
                        message: e.to_string(),
                    },
                }
            }
            IpcCommand::FocusTab { index } => {
                match self.focus_tab(index) {
                    Ok(()) => IpcResponse::Ok,
                    Err(e) => IpcResponse::Error {
                        code: "focus_tab_failed".to_string(),
                        message: e.to_string(),
                    },
                }
            }
            IpcCommand::FocusFrame { direction } => {
                let dir = match direction.to_lowercase().as_str() {
                    "left" | "l" => Direction::Left,
                    "right" | "r" => Direction::Right,
                    "up" | "u" => Direction::Up,
                    "down" | "d" => Direction::Down,
                    _ => {
                        return IpcResponse::Error {
                            code: "invalid_direction".to_string(),
                            message: format!("Unknown direction: {}. Use left, right, up, or down.", direction),
                        };
                    }
                };
                match self.focus_frame(dir) {
                    Ok(()) => IpcResponse::Ok,
                    Err(e) => IpcResponse::Error {
                        code: "focus_frame_failed".to_string(),
                        message: e.to_string(),
                    },
                }
            }
            IpcCommand::Split { direction } => {
                let dir = match direction.to_lowercase().as_str() {
                    "horizontal" | "h" => SplitDirection::Horizontal,
                    "vertical" | "v" => SplitDirection::Vertical,
                    _ => {
                        return IpcResponse::Error {
                            code: "invalid_direction".to_string(),
                            message: format!("Invalid split direction: {}", direction),
                        }
                    }
                };
                match self.split_focused(dir) {
                    Ok(()) => IpcResponse::Ok,
                    Err(e) => IpcResponse::Error {
                        code: "split_failed".to_string(),
                        message: e.to_string(),
                    },
                }
            }
            IpcCommand::MoveWindow { forward } => {
                match self.move_window(forward) {
                    Ok(()) => IpcResponse::Ok,
                    Err(e) => IpcResponse::Error {
                        code: "move_failed".to_string(),
                        message: e.to_string(),
                    },
                }
            }
            IpcCommand::ResizeSplit { delta } => {
                match self.resize_split(delta > 0.0) {
                    Ok(()) => IpcResponse::Ok,
                    Err(e) => IpcResponse::Error {
                        code: "resize_failed".to_string(),
                        message: e.to_string(),
                    },
                }
            }
            IpcCommand::CloseWindow => {
                match self.close_focused_window() {
                    Ok(()) => IpcResponse::Ok,
                    Err(e) => IpcResponse::Error {
                        code: "close_failed".to_string(),
                        message: e.to_string(),
                    },
                }
            }
            IpcCommand::CycleTab { forward } => {
                match self.cycle_tab(forward) {
                    Ok(()) => IpcResponse::Ok,
                    Err(e) => IpcResponse::Error {
                        code: "cycle_tab_failed".to_string(),
                        message: e.to_string(),
                    },
                }
            }
            IpcCommand::TagWindow { window } => {
                let target = window.or(self.focused_window);
                if let Some(w) = target {
                    self.tagged_windows.insert(w);
                    log::info!("Tagged window 0x{:x} via IPC", w);
                    if self.apply_layout().is_err() {
                        return IpcResponse::Error {
                            code: "layout_failed".to_string(),
                            message: "Failed to apply layout".to_string(),
                        };
                    }
                    IpcResponse::Ok
                } else {
                    IpcResponse::Error {
                        code: "no_window".to_string(),
                        message: "No window specified and no focused window".to_string(),
                    }
                }
            }
            IpcCommand::UntagWindow { window } => {
                let target = window.or(self.focused_window);
                if let Some(w) = target {
                    self.tagged_windows.remove(&w);
                    log::info!("Untagged window 0x{:x} via IPC", w);
                    if self.apply_layout().is_err() {
                        return IpcResponse::Error {
                            code: "layout_failed".to_string(),
                            message: "Failed to apply layout".to_string(),
                        };
                    }
                    IpcResponse::Ok
                } else {
                    IpcResponse::Error {
                        code: "no_window".to_string(),
                        message: "No window specified and no focused window".to_string(),
                    }
                }
            }
            IpcCommand::ToggleTag { window } => {
                let target = window.or(self.focused_window);
                if let Some(w) = target {
                    if self.tagged_windows.contains(&w) {
                        self.tagged_windows.remove(&w);
                        log::info!("Untagged window 0x{:x} via IPC", w);
                    } else {
                        self.tagged_windows.insert(w);
                        log::info!("Tagged window 0x{:x} via IPC", w);
                    }
                    if self.apply_layout().is_err() {
                        return IpcResponse::Error {
                            code: "layout_failed".to_string(),
                            message: "Failed to apply layout".to_string(),
                        };
                    }
                    IpcResponse::Ok
                } else {
                    IpcResponse::Error {
                        code: "no_window".to_string(),
                        message: "No window specified and no focused window".to_string(),
                    }
                }
            }
            IpcCommand::MoveTagged => {
                match self.move_tagged_to_focused_frame() {
                    Ok(()) => IpcResponse::Ok,
                    Err(e) => IpcResponse::Error {
                        code: "move_tagged_failed".to_string(),
                        message: e.to_string(),
                    },
                }
            }
            IpcCommand::UntagAll => {
                match self.untag_all_windows() {
                    Ok(()) => IpcResponse::Ok,
                    Err(e) => IpcResponse::Error {
                        code: "untag_all_failed".to_string(),
                        message: e.to_string(),
                    },
                }
            }
            IpcCommand::GetTagged => {
                let tagged: Vec<u32> = self.tagged_windows.iter().copied().collect();
                IpcResponse::Tagged { windows: tagged }
            }
            IpcCommand::SwitchWorkspace { index } => {
                if let Some(old_idx) = self.workspaces.switch_to(index) {
                    match self.perform_workspace_switch(old_idx) {
                        Ok(()) => IpcResponse::Ok,
                        Err(e) => IpcResponse::Error {
                            code: "workspace_switch_failed".to_string(),
                            message: e.to_string(),
                        },
                    }
                } else {
                    IpcResponse::Ok // Already on that workspace or invalid
                }
            }
            IpcCommand::WorkspaceNext => {
                match self.workspace_next() {
                    Ok(()) => IpcResponse::Ok,
                    Err(e) => IpcResponse::Error {
                        code: "workspace_next_failed".to_string(),
                        message: e.to_string(),
                    },
                }
            }
            IpcCommand::WorkspacePrev => {
                match self.workspace_prev() {
                    Ok(()) => IpcResponse::Ok,
                    Err(e) => IpcResponse::Error {
                        code: "workspace_prev_failed".to_string(),
                        message: e.to_string(),
                    },
                }
            }
            IpcCommand::GetCurrentWorkspace => {
                IpcResponse::Workspace {
                    index: self.workspaces.current_index(),
                    total: 9,
                }
            }
            IpcCommand::MoveToWorkspace { window, workspace } => {
                let target_window = window.or(self.focused_window);
                if let Some(w) = target_window {
                    match self.move_window_to_workspace(w, workspace) {
                        Ok(()) => IpcResponse::Ok,
                        Err(e) => IpcResponse::Error {
                            code: "move_to_workspace_failed".to_string(),
                            message: e.to_string(),
                        },
                    }
                } else {
                    IpcResponse::Error {
                        code: "no_window".to_string(),
                        message: "No window specified and no focused window".to_string(),
                    }
                }
            }
            IpcCommand::Screenshot { path } => {
                match self.capture_screenshot(&path) {
                    Ok(()) => IpcResponse::Screenshot { path },
                    Err(e) => IpcResponse::Error {
                        code: "screenshot_failed".to_string(),
                        message: e.to_string(),
                    },
                }
            }
            IpcCommand::Quit => {
                log::info!("Quit requested via IPC");
                self.running = false;
                IpcResponse::Ok
            }
        };

        // Trace the IPC interaction
        let result_status = match &response {
            IpcResponse::Ok => "ok",
            IpcResponse::Error { .. } => "error",
            _ => "success",
        };
        self.tracer.trace_ipc(&cmd_name, result_status);

        response
    }

    /// Create a snapshot of the current WM state for IPC
    fn snapshot_state(&self) -> WmStateSnapshot {
        let geometries = self.workspaces.current().layout.calculate_geometries(
            self.usable_screen(),
            self.config.gap,
        );
        WmStateSnapshot {
            focused_window: self.focused_window,
            focused_frame: self.workspaces.current().layout.focused_frame_id(),
            window_count: self.workspaces.current().layout.all_windows().len(),
            frame_count: self.workspaces.current().layout.all_frames().len(),
            layout: self.workspaces.current().layout.snapshot(Some(&geometries)),
            windows: self.get_window_info_list(),
        }
    }

    /// Get information about all managed windows
    fn get_window_info_list(&self) -> Vec<WindowInfo> {
        let mut windows = Vec::new();
        let all_frames = self.workspaces.current().layout.all_frames();

        for frame_id in all_frames {
            if let Some(frame) = self.workspaces.current().layout.get(frame_id).and_then(|n| n.as_frame()) {
                let is_focused_frame = frame_id == self.workspaces.current().layout.focused;
                for (tab_index, &window) in frame.windows.iter().enumerate() {
                    let is_focused_tab = tab_index == frame.focused;
                    windows.push(WindowInfo {
                        id: window,
                        title: self.get_window_title(window),
                        frame: format!("{:?}", frame_id),
                        tab_index,
                        is_focused: is_focused_frame && is_focused_tab && self.focused_window == Some(window),
                        is_visible: is_focused_tab, // Only the focused tab is visible
                        is_tagged: self.tagged_windows.contains(&window),
                    });
                }
            }
        }

        windows
    }

    /// Validate WM state invariants
    fn validate_state(&self) -> Vec<String> {
        let mut violations = Vec::new();

        // Check: focused window should be in layout
        if let Some(w) = self.focused_window {
            if self.workspaces.current().layout.find_window(w).is_none() {
                violations.push(format!("Focused window 0x{:x} is not in layout", w));
            }
        }

        // Check: focused frame should exist
        if self.workspaces.current().layout.get(self.workspaces.current().layout.focused).is_none() {
            violations.push(format!("Focused frame {:?} does not exist", self.workspaces.current().layout.focused));
        }

        // Check: all hidden windows should be in layout
        for &w in &self.hidden_windows {
            if self.workspaces.current().layout.find_window(w).is_none() {
                violations.push(format!("Hidden window 0x{:x} is not in layout", w));
            }
        }

        // Check: tab bar windows should correspond to existing frames
        let ws_idx = self.workspaces.current_index();
        for &(idx, frame_id) in self.tab_bar_windows.keys() {
            if idx == ws_idx && self.workspaces.current().layout.get(frame_id).is_none() {
                violations.push(format!("Tab bar for non-existent frame {:?}", frame_id));
            }
        }

        violations
    }

    /// Capture a screenshot and save it to the specified path
    fn capture_screenshot(&self, path: &str) -> Result<()> {
        use image::{ImageBuffer, Rgba};

        let geometry = self.conn.get_geometry(self.root)?.reply()?;

        let image_reply = self.conn.get_image(
            ImageFormat::Z_PIXMAP,
            self.root,
            0,
            0,
            geometry.width,
            geometry.height,
            !0, // all planes
        )?.reply()?;

        // Convert the image data to RGBA
        // X11 typically returns BGRA format for 32-bit depth
        let depth = image_reply.depth;
        let data = &image_reply.data;

        let mut img: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::new(
            geometry.width as u32,
            geometry.height as u32,
        );

        if depth == 24 || depth == 32 {
            // BGRA or BGR format
            let bytes_per_pixel = if depth == 32 { 4 } else { 3 };
            let stride = geometry.width as usize * bytes_per_pixel;

            for y in 0..geometry.height as usize {
                for x in 0..geometry.width as usize {
                    let offset = y * stride + x * bytes_per_pixel;
                    if offset + 2 < data.len() {
                        let b = data[offset];
                        let g = data[offset + 1];
                        let r = data[offset + 2];
                        let a = if bytes_per_pixel == 4 && offset + 3 < data.len() {
                            data[offset + 3]
                        } else {
                            255
                        };
                        img.put_pixel(x as u32, y as u32, Rgba([r, g, b, a]));
                    }
                }
            }
        } else {
            return Err(anyhow::anyhow!("Unsupported color depth: {}", depth));
        }

        img.save(path).context("Failed to save screenshot")?;
        log::info!("Screenshot saved to {}", path);

        Ok(())
    }
}

fn main() -> Result<()> {
    // Initialize logging
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info")
    ).init();

    log::info!("Starting ttwm - Tabbed Tiling Window Manager");

    // Create window manager
    let mut wm = Wm::new()?;

    // Become the window manager
    wm.become_wm()?;

    // Set up EWMH properties
    wm.setup_ewmh()?;

    // Grab our keybindings
    wm.grab_keys()?;

    // Manage any existing windows
    wm.scan_existing_windows()?;

    // Run the event loop
    wm.run()?;

    Ok(())
}
