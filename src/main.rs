//! ttwm - Tabbed Tiling Window Manager
//!
//! A minimal X11 tiling window manager inspired by Notion.
//! Milestone 5: Tabs with tab bar rendering.
//! Milestone 6: IPC interface for debugability and scriptability.

mod config;
mod ewmh;
mod icon;
mod ipc;
mod layout;
mod monitor;
mod render;
mod startup;
mod state;
mod tab_bar;
mod tracing;
mod types;
mod window_query;
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
use monitor::{MonitorId, MonitorManager};
use workspaces::{WorkspaceManager, NUM_WORKSPACES};
use render::{CachedIcon, FontRenderer, blend_icon_with_background, lighten_color, darken_color, DEFAULT_ICON};
use state::{StateTransition, UnmanageReason};
use tracing::EventTracer;
use types::StrutPartial;

/// Layout configuration
struct LayoutConfig {
    /// Gap between windows
    gap: u32,
    /// Outer gap (margin from screen edge)
    outer_gap: u32,
    /// Border width
    border_width: u32,
    /// Tab bar height (for horizontal tabs)
    tab_bar_height: u32,
    /// Vertical tab bar width (for vertical tabs)
    vertical_tab_width: u32,
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
    /// Urgent tab background color
    tab_urgent_bg: u32,
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
            vertical_tab_width: 28,
            tab_bar_bg: 0x000000,       // Black (fallback)
            tab_focused_bg: 0x5294e2,   // Blue (matching border)
            tab_unfocused_bg: 0x3a3a3a, // Darker gray
            tab_visible_unfocused_bg: 0x4a6a9a, // Muted blue
            tab_tagged_bg: 0xe06c75,    // Soft red
            tab_urgent_bg: 0xd19a66,    // Orange/amber
            tab_text_color: 0xffffff,   // White
            tab_text_unfocused: 0x888888, // Dim gray
            tab_separator: 0x4a4a4a,    // Subtle separator
            border_focused: 0x5294e2,   // Blue
            border_unfocused: 0x3a3a3a, // Gray
            show_tab_icons: true,
        }
    }
}

/// Edge or corner of a floating window for resizing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResizeEdge {
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
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
    /// Moving a floating window
    FloatMove {
        /// The window being moved
        window: Window,
        /// Mouse start position (root coordinates)
        start_x: i32,
        start_y: i32,
        /// Window start position
        win_x: i32,
        win_y: i32,
    },
    /// Resizing a floating window
    FloatResize {
        /// The window being resized
        window: Window,
        /// Which edge/corner is being dragged
        edge: ResizeEdge,
        /// Mouse start position (root coordinates)
        start_x: i32,
        start_y: i32,
        /// Original window geometry
        original_x: i32,
        original_y: i32,
        original_w: u32,
        original_h: u32,
    },
}

/// The main window manager state
struct Wm {
    conn: RustConnection,
    screen_num: usize,
    root: Window,
    atoms: Atoms,
    /// Multi-monitor support - each monitor has its own workspaces
    monitors: MonitorManager,
    /// Currently focused window (if any)
    focused_window: Option<Window>,
    /// WM check window for EWMH
    check_window: Window,
    /// Layout configuration
    config: LayoutConfig,
    /// Tab bar windows for each frame ((monitor_id, workspace_idx, NodeId) -> Window)
    tab_bar_windows: HashMap<(MonitorId, usize, NodeId), Window>,
    /// Pixmap buffers for double-buffered tab bar rendering (tab_bar_window -> pixmap)
    tab_bar_pixmaps: HashMap<Window, u32>,
    /// Empty frame placeholder windows ((monitor_id, workspace_idx, NodeId) -> Window)
    empty_frame_windows: HashMap<(MonitorId, usize, NodeId), Window>,
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
    /// Default arrow cursor
    cursor_default: Cursor,
    /// Top-left corner resize cursor
    cursor_resize_tl: Cursor,
    /// Top-right corner resize cursor
    cursor_resize_tr: Cursor,
    /// Bottom-left corner resize cursor
    cursor_resize_bl: Cursor,
    /// Bottom-right corner resize cursor
    cursor_resize_br: Cursor,
    /// Currently displayed cursor (to avoid redundant changes)
    current_cursor: Cursor,
    /// Screen depth (for put_image)
    screen_depth: u8,
    /// Icon cache for tab icons (only caches windows that have icons)
    icon_cache: HashMap<Window, CachedIcon>,
    /// Windows that are currently tagged for batch operations
    tagged_windows: std::collections::HashSet<Window>,
    /// Suppress EnterNotify focus changes (set after explicit focus operations)
    suppress_enter_focus: bool,
    /// Skip tab bar redraw in focus_window() when apply_layout() just did it
    skip_focus_tab_bar_redraw: bool,
    /// Windows that are urgent (FIFO order - oldest first)
    urgent_windows: Vec<Window>,
    /// Overlay window for cross-workspace urgent indicator
    urgent_indicator: Option<Window>,
    /// Dock windows (polybar, etc.) and their strut reservations
    dock_windows: HashMap<Window, StrutPartial>,
    /// Startup manager for initial layout and app spawning
    startup_manager: startup::StartupManager,
    /// User configuration (kept for startup config reference)
    user_config: Config,
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
            vertical_tab_width: user_config.appearance.vertical_tab_width,
            tab_bar_bg: parse_color(&user_config.colors.tab_bar_bg).unwrap_or(0x2e2e2e),
            tab_focused_bg: parse_color(&user_config.colors.tab_focused_bg).unwrap_or(0x5294e2),
            tab_unfocused_bg: parse_color(&user_config.colors.tab_unfocused_bg).unwrap_or(0x3a3a3a),
            tab_visible_unfocused_bg: parse_color(&user_config.colors.tab_visible_unfocused_bg).unwrap_or(0x4a6a9a),
            tab_tagged_bg: parse_color(&user_config.colors.tab_tagged_bg).unwrap_or(0xe06c75),
            tab_urgent_bg: parse_color(&user_config.colors.tab_urgent_bg).unwrap_or(0xd19a66),
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

        // XC_left_ptr = 68 (default arrow)
        let cursor_default = conn.generate_id()?;
        conn.create_glyph_cursor(
            cursor_default,
            cursor_font,
            cursor_font,
            68,
            68 + 1,
            0, 0, 0,
            0xFFFF, 0xFFFF, 0xFFFF,
        )?;

        // XC_top_left_corner = 134
        let cursor_resize_tl = conn.generate_id()?;
        conn.create_glyph_cursor(
            cursor_resize_tl,
            cursor_font,
            cursor_font,
            134,
            134 + 1,
            0, 0, 0,
            0xFFFF, 0xFFFF, 0xFFFF,
        )?;

        // XC_top_right_corner = 136
        let cursor_resize_tr = conn.generate_id()?;
        conn.create_glyph_cursor(
            cursor_resize_tr,
            cursor_font,
            cursor_font,
            136,
            136 + 1,
            0, 0, 0,
            0xFFFF, 0xFFFF, 0xFFFF,
        )?;

        // XC_bottom_left_corner = 12
        let cursor_resize_bl = conn.generate_id()?;
        conn.create_glyph_cursor(
            cursor_resize_bl,
            cursor_font,
            cursor_font,
            12,
            12 + 1,
            0, 0, 0,
            0xFFFF, 0xFFFF, 0xFFFF,
        )?;

        // XC_bottom_right_corner = 14
        let cursor_resize_br = conn.generate_id()?;
        conn.create_glyph_cursor(
            cursor_resize_br,
            cursor_font,
            cursor_font,
            14,
            14 + 1,
            0, 0, 0,
            0xFFFF, 0xFFFF, 0xFFFF,
        )?;

        conn.close_font(cursor_font)?;

        // Initialize monitor manager with RandR
        use x11rb::protocol::randr;

        // Select RandR events for hotplug detection
        randr::select_input(
            &conn,
            root,
            randr::NotifyMask::SCREEN_CHANGE | randr::NotifyMask::OUTPUT_CHANGE,
        )?;
        conn.flush()?;

        let mut monitors = MonitorManager::new();
        monitors.refresh(&conn, root)?;
        log::info!("Initialized {} monitor(s)", monitors.count());

        Ok(Self {
            conn,
            screen_num,
            root,
            atoms,
            monitors,
            focused_window: None,
            check_window,
            config,
            tab_bar_windows: HashMap::new(),
            tab_bar_pixmaps: HashMap::new(),
            empty_frame_windows: HashMap::new(),
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
            cursor_default,
            cursor_resize_tl,
            cursor_resize_tr,
            cursor_resize_bl,
            cursor_resize_br,
            current_cursor: cursor_default,
            screen_depth,
            icon_cache: HashMap::new(),
            tagged_windows: std::collections::HashSet::new(),
            suppress_enter_focus: false,
            skip_focus_tab_bar_redraw: false,
            urgent_windows: Vec::new(),
            urgent_indicator: None,
            dock_windows: HashMap::new(),
            startup_manager: startup::StartupManager::new(),
            user_config,
        })
    }

    /// Get screen info
    fn screen(&self) -> &Screen {
        &self.conn.setup().roots[self.screen_num]
    }

    /// Get the workspace manager for the focused monitor
    fn workspaces(&self) -> &WorkspaceManager {
        &self.monitors.focused().workspaces
    }

    /// Get the workspace manager for the focused monitor (mutable)
    fn workspaces_mut(&mut self) -> &mut WorkspaceManager {
        &mut self.monitors.focused_mut().workspaces
    }

    /// Find a frame by name across all workspaces/monitors
    /// Returns (MonitorId, workspace_index, NodeId) if found
    fn find_frame_by_name_global(&self, name: &str) -> Option<(MonitorId, usize, NodeId)> {
        for (monitor_id, monitor) in self.monitors.iter() {
            for (ws_idx, ws) in monitor.workspaces.workspaces.iter().enumerate() {
                if let Some(node_id) = ws.layout.find_frame_by_name(name) {
                    return Some((monitor_id, ws_idx, node_id));
                }
            }
        }
        None
    }

    /// Get the appropriate cursor for a resize edge
    fn cursor_for_edge(&self, edge: ResizeEdge) -> Cursor {
        match edge {
            ResizeEdge::Left | ResizeEdge::Right => self.cursor_resize_h,
            ResizeEdge::Top | ResizeEdge::Bottom => self.cursor_resize_v,
            ResizeEdge::TopLeft => self.cursor_resize_tl,
            ResizeEdge::TopRight => self.cursor_resize_tr,
            ResizeEdge::BottomLeft => self.cursor_resize_bl,
            ResizeEdge::BottomRight => self.cursor_resize_br,
        }
    }

    /// Update cursor based on what's under the mouse (for hover feedback)
    fn update_hover_cursor(&mut self, x: i32, y: i32) -> Result<()> {
        let screen = self.usable_screen();
        let gap = self.config.gap;

        // Check if over a split gap
        let new_cursor = if let Some((_, direction, _, _)) =
            self.workspaces().current().layout.find_split_at_gap(screen, gap, x, y)
        {
            match direction {
                SplitDirection::Horizontal => self.cursor_resize_h,
                SplitDirection::Vertical => self.cursor_resize_v,
            }
        } else {
            self.cursor_default
        };

        // Only update if cursor changed
        if new_cursor != self.current_cursor {
            self.conn.change_window_attributes(
                self.root,
                &ChangeWindowAttributesAux::new().cursor(new_cursor),
            )?;
            self.current_cursor = new_cursor;
            self.conn.flush()?;
        }
        Ok(())
    }

    /// Become the window manager by requesting SubstructureRedirect on root
    fn become_wm(&self) -> Result<()> {
        // Set event mask on root window
        // SubstructureRedirect is the key - it makes us the WM
        let event_mask = EventMask::SUBSTRUCTURE_REDIRECT
            | EventMask::SUBSTRUCTURE_NOTIFY
            | EventMask::ENTER_WINDOW  // For focus-follows-mouse
            | EventMask::STRUCTURE_NOTIFY
            | EventMask::BUTTON_PRESS  // For gap resize detection
            | EventMask::POINTER_MOTION; // For hover cursor feedback

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
        ewmh::update_current_desktop(
            &self.conn,
            &self.atoms,
            self.root,
            self.workspaces().current_index(),
        )
    }

    /// Set _NET_WM_DESKTOP for a window
    fn set_window_desktop(&self, window: Window, desktop: usize) -> Result<()> {
        ewmh::set_window_desktop(&self.conn, &self.atoms, window, desktop)
    }

    /// Switch to the next workspace
    fn workspace_next(&mut self) -> Result<()> {
        let old_idx = self.workspaces_mut().next();
        self.perform_workspace_switch(old_idx)?;
        Ok(())
    }

    /// Switch to the previous workspace
    fn workspace_prev(&mut self) -> Result<()> {
        let old_idx = self.workspaces_mut().prev();
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
        let current_ws = self.workspaces().current_index();
        let target_frame = self.workspaces().current().layout.focused;
        let tagged: Vec<Window> = self.tagged_windows.iter().copied().collect();
        let count = tagged.len();

        let mut last_moved: Option<Window> = None;
        for window in tagged {
            // Search ALL workspaces for this window
            let source_ws = self.monitors.focused().workspaces.workspaces.iter()
                .enumerate()
                .find(|(_, ws)| ws.layout.find_window(window).is_some())
                .map(|(idx, _)| idx);

            if let Some(source_ws) = source_ws {
                if source_ws == current_ws {
                    // Same workspace - use existing move logic
                    if let Some(source_frame) = self.workspaces_mut().current_mut().layout.find_window(window) {
                        if source_frame != target_frame {
                            self.workspaces_mut().current_mut().layout.move_window_to_frame(
                                window,
                                source_frame,
                                target_frame,
                            );
                            last_moved = Some(window);
                        }
                    }
                } else {
                    // Different workspace - cross-workspace move
                    // 1. Hide window (it's moving to current workspace)
                    self.conn.unmap_window(window)?;
                    // 2. Remove from source workspace
                    self.monitors.focused_mut().workspaces.workspaces[source_ws].layout.remove_window(window);
                    // 3. Add to target frame on current workspace
                    self.workspaces_mut().current_mut().layout.add_window_to_frame(window, target_frame);
                    // 4. Update _NET_WM_DESKTOP property
                    self.set_window_desktop(window, current_ws)?;
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
        let new_idx = self.workspaces().current_index();
        log::info!("Switching from workspace {} to workspace {}", old_idx + 1, new_idx + 1);

        // Save current workspace's focused window
        self.monitors.focused_mut().workspaces.workspaces[old_idx].last_focused_window = self.focused_window;

        // Hide all tiled windows from old workspace
        for window in self.monitors.focused_mut().workspaces.workspaces[old_idx].layout.all_windows() {
            self.hidden_windows.insert(window);
            self.conn.unmap_window(window)?;
        }

        // Hide all floating windows from old workspace
        for floating in &self.monitors.focused_mut().workspaces.workspaces[old_idx].floating_windows {
            self.hidden_windows.insert(floating.window);
            self.conn.unmap_window(floating.window)?;
        }

        // Hide tab bars from old workspace (on focused monitor)
        let mon_id = self.monitors.focused_id();
        for (&(m_id, ws_idx, _), &tab_window) in &self.tab_bar_windows {
            if m_id == mon_id && ws_idx == old_idx {
                self.conn.unmap_window(tab_window)?;
            }
        }

        // Hide empty frame windows from old workspace (on focused monitor)
        for (&(m_id, ws_idx, _), &empty_window) in &self.empty_frame_windows {
            if m_id == mon_id && ws_idx == old_idx {
                self.conn.unmap_window(empty_window)?;
            }
        }

        // Show windows from new workspace (remove from hidden set)
        // Collect window IDs first to avoid borrow conflicts
        let tiled_windows = self.workspaces().current().layout.all_windows();
        let floating_windows = self.workspaces().current().floating_window_ids();
        for window in tiled_windows {
            self.hidden_windows.remove(&window);
        }
        for window in floating_windows {
            self.hidden_windows.remove(&window);
        }

        // Clear focused window (will be restored below)
        self.focused_window = None;

        // Apply layout for new workspace (handles both tiled and floating)
        self.apply_layout()?;

        // Restore focus to last focused window in new workspace
        if let Some(w) = self.workspaces().current().last_focused_window {
            // Check if window exists (either tiled or floating)
            let is_tiled = self.workspaces().current().layout.find_window(w).is_some();
            let is_floating = self.workspaces().current().is_floating(w);
            if is_tiled || is_floating {
                self.focus_window(w)?;
            }
        }

        // If no focus restored, try to focus something
        if self.focused_window.is_none() {
            self.focus_next_available_window()?;
        }

        // Update EWMH
        self.update_current_desktop()?;

        // Update urgent indicator (may need to show/hide based on new workspace)
        self.update_urgent_indicator()?;

        self.conn.flush()?;
        Ok(())
    }

    /// Update _NET_CLIENT_LIST with current windows (from all workspaces)
    fn update_client_list(&self) -> Result<()> {
        let mut windows: Vec<Window> = self.monitors.focused().workspaces.workspaces.iter()
            .flat_map(|ws| ws.layout.all_windows())
            .collect();
        // Also include floating windows
        for ws in &self.monitors.focused().workspaces.workspaces {
            windows.extend(ws.floating_window_ids());
        }
        ewmh::update_client_list(&self.conn, &self.atoms, self.root, &windows)
    }

    /// Update _NET_ACTIVE_WINDOW
    fn update_active_window(&self) -> Result<()> {
        ewmh::update_active_window(&self.conn, &self.atoms, self.root, self.focused_window)
    }

    /// Get the usable screen area for the focused monitor (with outer gaps)
    fn usable_screen(&self) -> Rect {
        self.usable_area(self.monitors.focused_id())
    }

    /// Get the usable area for a specific monitor (with outer gaps and struts)
    fn usable_area(&self, monitor_id: MonitorId) -> Rect {
        let gap = self.config.outer_gap;
        let base = if let Some(monitor) = self.monitors.get(monitor_id) {
            monitor.geometry
        } else {
            // Fallback to full screen if monitor not found
            let screen = self.screen();
            Rect::new(0, 0, screen.width_in_pixels as u32, screen.height_in_pixels as u32)
        };

        // Aggregate struts from all dock windows (take max of each edge)
        let (strut_left, strut_right, strut_top, strut_bottom) =
            self.dock_windows.values().fold((0u32, 0u32, 0u32, 0u32), |acc, s| {
                (
                    acc.0.max(s.left),
                    acc.1.max(s.right),
                    acc.2.max(s.top),
                    acc.3.max(s.bottom),
                )
            });

        Rect::new(
            base.x + gap as i32 + strut_left as i32,
            base.y + gap as i32 + strut_top as i32,
            base.width.saturating_sub(gap * 2 + strut_left + strut_right),
            base.height.saturating_sub(gap * 2 + strut_top + strut_bottom),
        )
    }

    /// Get or create a tab bar window for a frame
    fn get_or_create_tab_bar(&mut self, frame_id: NodeId, rect: &Rect, vertical: bool) -> Result<Window> {
        let mon_id = self.monitors.focused_id();
        let ws_idx = self.workspaces().current_index();
        let key = (mon_id, ws_idx, frame_id);

        // Calculate dimensions based on orientation
        let (x, y, width, height) = if vertical {
            // Vertical: left side of frame, full height
            (rect.x, rect.y, self.config.vertical_tab_width, rect.height)
        } else {
            // Horizontal: top of frame, full width
            (rect.x, rect.y, rect.width, self.config.tab_bar_height)
        };

        if let Some(&window) = self.tab_bar_windows.get(&key) {
            // Update position and size
            self.conn.configure_window(
                window,
                &ConfigureWindowAux::new()
                    .x(x)
                    .y(y)
                    .width(width)
                    .height(height),
            )?;
            // Invalidate pixmap buffer (size may have changed)
            if let Some(pixmap) = self.tab_bar_pixmaps.remove(&window) {
                let _ = self.conn.free_pixmap(pixmap);
            }
            return Ok(window);
        }

        // Create new tab bar window
        let window = self.conn.generate_id()?;
        self.conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            window,
            self.root,
            x as i16,
            y as i16,
            width as u16,
            height as u16,
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

    /// Get or create a pixmap buffer for double-buffered tab bar rendering
    fn get_or_create_tab_bar_pixmap(&mut self, window: Window, width: u16, height: u16) -> Result<u32> {
        // Always free existing pixmap to ensure correct dimensions
        // (pixmap might have been created with different dimensions)
        if let Some(old_pixmap) = self.tab_bar_pixmaps.remove(&window) {
            let _ = self.conn.free_pixmap(old_pixmap);
        }

        // Create new pixmap with requested dimensions
        let pixmap = self.conn.generate_id()?;
        self.conn.create_pixmap(self.screen_depth, pixmap, window, width, height)?;
        self.tab_bar_pixmaps.insert(window, pixmap);
        Ok(pixmap)
    }

    /// Get or create a placeholder window for an empty frame (shows border)
    fn get_or_create_empty_frame_window(&mut self, frame_id: NodeId, rect: &Rect, is_focused: bool) -> Result<Window> {
        let mon_id = self.monitors.focused_id();
        let ws_idx = self.workspaces().current_index();
        let key = (mon_id, ws_idx, frame_id);

        let border = self.config.border_width;
        // Empty frames use the full rect (no tab bar)
        let client_y = rect.y;
        let client_height = rect.height;
        let border_color = if is_focused {
            self.config.border_focused
        } else {
            self.config.border_unfocused
        };

        if let Some(&window) = self.empty_frame_windows.get(&key) {
            // Update position, size, and border color
            self.conn.configure_window(
                window,
                &ConfigureWindowAux::new()
                    .x(rect.x)
                    .y(client_y)
                    .width(rect.width.saturating_sub(border * 2))
                    .height(client_height.saturating_sub(border * 2))
                    .border_width(border),
            )?;
            self.conn.change_window_attributes(
                window,
                &ChangeWindowAttributesAux::new()
                    .border_pixel(border_color),
            )?;
            // Re-map in case it was hidden (e.g., workspace switch)
            self.conn.map_window(window)?;
            return Ok(window);
        }

        // Create new empty frame placeholder window
        let window = self.conn.generate_id()?;
        self.conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            window,
            self.root,
            rect.x as i16,
            client_y as i16,
            (rect.width.saturating_sub(border * 2)) as u16,
            (client_height.saturating_sub(border * 2)) as u16,
            border as u16,
            WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new()
                .background_pixel(self.config.tab_bar_bg)
                .border_pixel(border_color)
                .event_mask(EventMask::BUTTON_PRESS),
        )?;

        self.conn.map_window(window)?;
        self.empty_frame_windows.insert(key, window);

        Ok(window)
    }

    /// Destroy an empty frame placeholder window if it exists
    fn destroy_empty_frame_window(&mut self, frame_id: NodeId) {
        let mon_id = self.monitors.focused_id();
        let ws_idx = self.workspaces().current_index();
        let key = (mon_id, ws_idx, frame_id);

        if let Some(window) = self.empty_frame_windows.remove(&key) {
            if let Err(e) = self.conn.destroy_window(window) {
                log::error!("Failed to destroy empty frame window: {}", e);
            }
        }
    }

    /// Clean up empty frame windows for removed frames
    fn cleanup_empty_frame_windows(&mut self) {
        let mon_id = self.monitors.focused_id();
        let ws_idx = self.workspaces().current_index();
        let valid_frames: std::collections::HashSet<_> = self.workspaces().current().layout.all_frames().into_iter().collect();
        let to_remove: Vec<_> = self.empty_frame_windows
            .keys()
            .filter(|(m_id, idx, frame_id)| *m_id == mon_id && *idx == ws_idx && !valid_frames.contains(frame_id))
            .copied()
            .collect();

        for key in to_remove {
            if let Some(window) = self.empty_frame_windows.remove(&key) {
                if let Err(e) = self.conn.destroy_window(window) {
                    log::error!("Failed to destroy empty frame window: {}", e);
                }
            }
        }
    }

    /// Calculate tab widths based on window titles (Chrome-style content-based sizing)
    /// Returns a vector of (x_position, width) for each tab
    fn calculate_tab_layout(&self, frame_id: NodeId) -> Vec<(i16, u32)> {
        const MIN_TAB_WIDTH: u32 = 80;
        const MAX_TAB_WIDTH: u32 = 200;
        const H_PADDING: u32 = 24;  // Total horizontal padding (12px each side)
        const ICON_SIZE: u32 = 20;
        const ICON_PADDING: u32 = 4;  // Padding after icon

        let frame = match self.workspaces().current().layout.get(frame_id).and_then(|n| n.as_frame()) {
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
            let title = window_query::get_window_title(&self.conn, &self.atoms, client_window);
            let title_width = self.font_renderer.measure_text(&title);
            let tab_width = (title_width + H_PADDING + icon_width).clamp(MIN_TAB_WIDTH + icon_width, MAX_TAB_WIDTH + icon_width);

            result.push((x_offset, tab_width));
            x_offset += tab_width as i16;
        }

        result
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
    fn draw_tab_bar_background(&mut self, pixmap: u32, rect: &Rect, pix_width: u16, pix_height: u16) -> Result<()> {
        // Clear with solid color first to ensure old content is erased
        self.conn.change_gc(self.gc, &ChangeGCAux::new().foreground(self.config.tab_bar_bg))?;
        self.conn.poly_fill_rectangle(
            pixmap,
            self.gc,
            &[Rectangle {
                x: 0,
                y: 0,
                width: pix_width,
                height: pix_height,
            }],
        )?;

        // Sample and draw root background on top (pseudo-transparency)
        if let Some(pixels) = self.sample_root_background(
            rect.x as i16,
            rect.y as i16,
            pix_width,
            pix_height,
        ) {
            self.conn.put_image(
                ImageFormat::Z_PIXMAP,
                pixmap,
                self.gc,
                pix_width,
                pix_height,
                0, 0,  // destination x, y
                0,     // left_pad
                self.screen_depth,
                &pixels,
            )?;
        }

        Ok(())
    }

    /// Draw the background for a vertical tab bar.
    fn draw_vertical_tab_bar_background(&mut self, pixmap: u32, rect: &Rect, pix_width: u16, pix_height: u16) -> Result<()> {
        // Clear with solid color first
        self.conn.change_gc(self.gc, &ChangeGCAux::new().foreground(self.config.tab_bar_bg))?;
        self.conn.poly_fill_rectangle(
            pixmap,
            self.gc,
            &[Rectangle {
                x: 0,
                y: 0,
                width: pix_width,
                height: pix_height,
            }],
        )?;

        // Sample and draw root background on top (pseudo-transparency)
        if let Some(pixels) = self.sample_root_background(
            rect.x as i16,
            rect.y as i16,
            pix_width,
            pix_height,
        ) {
            self.conn.put_image(
                ImageFormat::Z_PIXMAP,
                pixmap,
                self.gc,
                pix_width,
                pix_height,
                0, 0,
                0,
                self.screen_depth,
                &pixels,
            )?;
        }

        Ok(())
    }

    /// Draw a single vertical tab (icon-only).
    #[allow(clippy::too_many_arguments)]
    fn draw_single_vertical_tab(
        &mut self,
        window: Window,
        y: i16,
        tab_size: u32,
        client_window: Window,
        is_focused: bool,
        is_last: bool,
        is_tagged: bool,
        is_focused_frame: bool,
    ) -> Result<()> {
        let width = tab_size;
        let height = tab_size;
        let corner_radius: u32 = 4; // Smaller radius for vertical tabs (vs 6px for horizontal)

        // Determine background color (same priority as horizontal)
        let is_urgent = self.urgent_windows.contains(&client_window);
        let bg_color = if is_tagged {
            self.config.tab_tagged_bg
        } else if is_focused && is_focused_frame {
            self.config.tab_focused_bg
        } else if is_urgent {
            self.config.tab_urgent_bg
        } else if is_focused {
            self.config.tab_visible_unfocused_bg
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
                    x: 2,
                    y: y + 2,
                    width: width as u16,
                    height: height as u16,
                }],
            )?;
        }

        // Draw tab background with rounded left corners
        self.conn.change_gc(self.gc, &ChangeGCAux::new().foreground(bg_color))?;
        tab_bar::draw_rounded_left_rect(&self.conn, self.gc, window, 0, y, width, height, corner_radius)?;

        // Draw bevel effect for 3D raised appearance
        let bevel_light = lighten_color(bg_color, 0x20);
        let bevel_dark = darken_color(bg_color, 0.7);

        // Left highlight (inside rounded corners)
        self.conn.change_gc(self.gc, &ChangeGCAux::new().foreground(bevel_light))?;
        self.conn.poly_fill_rectangle(
            window,
            self.gc,
            &[Rectangle {
                x: 1,
                y: y + corner_radius as i16,
                width: 1,
                height: (height - corner_radius * 2) as u16,
            }],
        )?;

        // Right shadow line
        self.conn.change_gc(self.gc, &ChangeGCAux::new().foreground(bevel_dark))?;
        self.conn.poly_fill_rectangle(
            window,
            self.gc,
            &[Rectangle {
                x: (width - 1) as i16,
                y,
                width: 1,
                height: height as u16,
            }],
        )?;

        // Draw separator line below (unless last tab or focused)
        if !is_last && !is_focused {
            self.conn.change_gc(self.gc, &ChangeGCAux::new().foreground(self.config.tab_separator))?;
            self.conn.poly_segment(
                window,
                self.gc,
                &[Segment {
                    x1: corner_radius as i16,
                    y1: y + height as i16 - 1,
                    x2: (width - 1) as i16,
                    y2: y + height as i16 - 1,
                }],
            )?;
        }

        // Draw icon centered in tab
        const ICON_SIZE: u32 = 20;
        let icon = self.get_window_icon(client_window);
        let blended = blend_icon_with_background(&icon.pixels, bg_color, ICON_SIZE);
        let icon_x = ((width - ICON_SIZE) / 2) as i16;
        let icon_y = y + ((height - ICON_SIZE) / 2) as i16;

        self.conn.put_image(
            ImageFormat::Z_PIXMAP,
            window,
            self.gc,
            ICON_SIZE as u16,
            ICON_SIZE as u16,
            icon_x,
            icon_y,
            0,
            24,
            &blended,
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

        // Tab background color (5 states: tagged, focused, urgent, visible-unfocused, background)
        // Priority: tagged > focused > urgent > visible-unfocused > background
        let is_urgent = self.urgent_windows.contains(&client_window);
        let bg_color = if is_tagged {
            self.config.tab_tagged_bg                 // #1 - Tagged
        } else if is_focused && is_focused_frame {
            self.config.tab_focused_bg                // #2 - Focused in focused frame
        } else if is_urgent {
            self.config.tab_urgent_bg                 // #3 - Urgent (even if visible in unfocused frame)
        } else if is_focused {
            self.config.tab_visible_unfocused_bg      // #4 - Visible in unfocused frame
        } else {
            self.config.tab_unfocused_bg              // #5 - Background tab
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
        tab_bar::draw_rounded_top_rect(&self.conn, self.gc, window, x, 0, tab_width, height, corner_radius)?;

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
            let icon = self.get_window_icon(client_window);
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
        }

        // Get window title and truncate if needed
        let title = window_query::get_window_title(&self.conn, &self.atoms, client_window);
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
    fn draw_tab_bar(&mut self, frame_id: NodeId, window: Window, rect: &Rect, vertical: bool) -> Result<()> {
        // Calculate pixmap dimensions based on orientation
        let (pix_width, pix_height) = if vertical {
            (self.config.vertical_tab_width as u16, rect.height as u16)
        } else {
            (rect.width as u16, self.config.tab_bar_height as u16)
        };

        // Get or create pixmap buffer for double-buffered rendering
        // (pixmap is always recreated fresh, and background fill covers entire area)
        let pixmap = self.get_or_create_tab_bar_pixmap(window, pix_width, pix_height)?;

        // Extract all needed data from frame before any mutable calls
        let (windows, focused_tab, is_empty) = {
            let frame = match self.workspaces().current().layout.get(frame_id).and_then(|n| n.as_frame()) {
                Some(f) => f,
                None => return Ok(()),
            };
            (frame.windows.clone(), frame.focused, frame.windows.is_empty())
        };

        // Draw background to pixmap
        if vertical {
            self.draw_vertical_tab_bar_background(pixmap, rect, pix_width, pix_height)?;
        } else {
            self.draw_tab_bar_background(pixmap, rect, pix_width, pix_height)?;
        }

        // Empty frame - just copy the background pixmap
        if is_empty {
            self.conn.copy_area(pixmap, window, self.gc, 0, 0, 0, 0, pix_width, pix_height)?;
            return Ok(());
        }

        // Check if this frame is the focused frame
        let is_focused_frame = frame_id == self.workspaces().current().layout.focused;

        if vertical {
            // Draw vertical tabs (icon-only) to pixmap
            let tab_size = self.config.vertical_tab_width;
            let num_tabs = windows.len();

            for (i, &client_window) in windows.iter().enumerate() {
                let y = (i as u32 * tab_size) as i16;
                let is_focused = i == focused_tab;
                let is_last = i == num_tabs - 1;
                let is_tagged = self.tagged_windows.contains(&client_window);

                self.draw_single_vertical_tab(
                    pixmap,
                    y,
                    tab_size,
                    client_window,
                    is_focused,
                    is_last,
                    is_tagged,
                    is_focused_frame,
                )?;
            }

            // Clear area after last tab on the WINDOW to remove ghost tabs
            let clear_start = (num_tabs as u32 * tab_size) as i16;
            if (clear_start as u16) < pix_height {
                self.conn.copy_area(pixmap, window, self.gc, 0, 0, 0, 0, pix_width, pix_height)?;
                self.conn.change_gc(self.gc, &ChangeGCAux::new().foreground(self.config.tab_bar_bg))?;
                self.conn.poly_fill_rectangle(
                    window,
                    self.gc,
                    &[Rectangle {
                        x: 0,
                        y: clear_start,
                        width: pix_width,
                        height: pix_height - clear_start as u16,
                    }],
                )?;
                return Ok(());
            }
        } else {
            // Draw horizontal tabs (with text) to pixmap
            let tab_layout = self.calculate_tab_layout(frame_id);
            let show_icons = self.config.show_tab_icons;
            let num_tabs = windows.len();

            for (i, &client_window) in windows.iter().enumerate() {
                let (x, tab_width) = tab_layout[i];
                let is_focused = i == focused_tab;
                let is_last = i == num_tabs - 1;
                let is_tagged = self.tagged_windows.contains(&client_window);

                self.draw_single_tab(
                    pixmap,
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

            // Save tab_layout info for clearing ghost tabs after copy
            if let Some(&(last_x, last_width)) = tab_layout.last() {
                let clear_start = last_x + last_width as i16;
                if (clear_start as u16) < pix_width {
                    // Copy pixmap to window first
                    self.conn.copy_area(pixmap, window, self.gc, 0, 0, 0, 0, pix_width, pix_height)?;
                    // Then clear the empty area on the WINDOW to remove ghost tabs
                    self.conn.change_gc(self.gc, &ChangeGCAux::new().foreground(self.config.tab_bar_bg))?;
                    self.conn.poly_fill_rectangle(
                        window,
                        self.gc,
                        &[Rectangle {
                            x: clear_start,
                            y: 0,
                            width: pix_width - clear_start as u16,
                            height: pix_height,
                        }],
                    )?;
                    return Ok(());
                }
            }
        }

        // Copy the rendered pixmap to window (double buffering)
        self.conn.copy_area(pixmap, window, self.gc, 0, 0, 0, 0, pix_width, pix_height)?;

        Ok(())
    }

    /// Get window icon from _NET_WM_ICON property, scaled to 20x20 BGRA.
    /// Returns a static default icon if the window has no icon.
    fn get_window_icon(&mut self, window: Window) -> &CachedIcon {
        const ICON_SIZE: u32 = 20;

        // Check cache first
        if self.icon_cache.contains_key(&window) {
            return self.icon_cache.get(&window).unwrap();
        }

        // Try to fetch _NET_WM_ICON - only cache if we get an actual icon
        if let Some(icon) = icon::fetch_icon(&self.conn, &self.atoms, window, ICON_SIZE) {
            self.icon_cache.insert(window, icon);
            return self.icon_cache.get(&window).unwrap();
        }

        // Return default icon for windows without _NET_WM_ICON
        &DEFAULT_ICON
    }

    /// Redraw tab bars that contain a specific window (used when icon changes)
    fn redraw_tabs_for_window(&mut self, window: Window) -> Result<()> {
        let mon_id = self.monitors.focused_id();
        let ws_idx = self.workspaces().current_index();

        // Find the frame containing this window
        if let Some(frame_id) = self.workspaces().current().layout.find_window(window) {
            // Get vertical_tabs state
            let vertical = self.workspaces().current().layout.get(frame_id)
                .and_then(|n| n.as_frame())
                .map(|f| f.vertical_tabs)
                .unwrap_or(false);

            // Get tab bar window for this frame
            if let Some(&tab_window) = self.tab_bar_windows.get(&(mon_id, ws_idx, frame_id)) {
                // Get frame geometry
                let screen_rect = self.usable_screen();
                let geometries = self.workspaces().current().layout.calculate_geometries(
                    screen_rect,
                    self.config.gap,
                );

                if let Some(rect) = geometries.iter().find(|(fid, _)| *fid == frame_id).map(|(_, r)| r.clone()) {
                    self.draw_tab_bar(frame_id, tab_window, &rect, vertical)?;
                    self.conn.flush()?;
                }
            }
        }

        Ok(())
    }

    /// Remove tab bar windows for frames that no longer exist
    fn cleanup_tab_bars(&mut self) {
        let mon_id = self.monitors.focused_id();
        let ws_idx = self.workspaces().current_index();
        let valid_frames: std::collections::HashSet<_> = self.workspaces().current().layout.all_frames().into_iter().collect();
        let to_remove: Vec<_> = self.tab_bar_windows
            .keys()
            .filter(|(m_id, idx, frame_id)| *m_id == mon_id && *idx == ws_idx && !valid_frames.contains(frame_id))
            .copied()
            .collect();

        for key in to_remove {
            if let Some(window) = self.tab_bar_windows.remove(&key) {
                // Free associated pixmap buffer
                if let Some(pixmap) = self.tab_bar_pixmaps.remove(&window) {
                    let _ = self.conn.free_pixmap(pixmap);
                }
                if let Err(e) = self.conn.destroy_window(window) {
                    log::error!("Failed to destroy tab bar window: {}", e);
                }
            }
        }
    }

    /// Apply the current layout to all windows
    fn apply_layout(&mut self) -> Result<()> {
        // Check for fullscreen window first - it takes over the entire screen
        if let Some(fullscreen_window) = self.workspaces().current().fullscreen_window {
            // Get the raw monitor geometry (no gaps, no struts)
            let monitor = self.monitors.focused();
            let geom = monitor.geometry;

            // Configure fullscreen window to cover entire monitor
            self.conn.configure_window(
                fullscreen_window,
                &ConfigureWindowAux::new()
                    .x(geom.x)
                    .y(geom.y)
                    .width(geom.width)
                    .height(geom.height)
                    .border_width(0)
                    .stack_mode(StackMode::ABOVE),
            )?;
            self.conn.map_window(fullscreen_window)?;
            self.conn.flush()?;

            // Hide all tab bars and empty frame placeholders
            let mon_id = self.monitors.focused_id();
            let ws_idx = self.workspaces().current_index();
            for (&(mid, wsidx, _), &tab_win) in &self.tab_bar_windows {
                if mid == mon_id && wsidx == ws_idx {
                    self.conn.unmap_window(tab_win)?;
                }
            }
            for (&(mid, wsidx, _), &empty_win) in &self.empty_frame_windows {
                if mid == mon_id && wsidx == ws_idx {
                    self.conn.unmap_window(empty_win)?;
                }
            }
            self.conn.flush()?;

            return Ok(());
        }

        let screen_rect = self.usable_screen();
        let geometries = self.workspaces().current().layout.calculate_geometries(screen_rect, self.config.gap);

        // Get the focused frame id
        let focused_frame_id = self.workspaces().current().layout.focused;

        // Collect frame info for tab bar management (frame_id, rect, window_count, vertical_tabs)
        let mut frames_with_tabs: Vec<(NodeId, Rect, usize, bool)> = Vec::new();
        // Track empty frames for placeholder windows
        let mut empty_frames: Vec<(NodeId, Rect, bool)> = Vec::new();
        // Track non-empty frames to destroy their placeholder windows
        let mut non_empty_frames: Vec<NodeId> = Vec::new();

        // Collect frame data upfront to avoid borrow conflicts
        struct FrameData {
            frame_id: NodeId,
            rect: Rect,
            windows: Vec<Window>,
            focused_idx: usize,
            vertical_tabs: bool,
        }
        let frame_data: Vec<FrameData> = geometries.iter()
            .filter_map(|(frame_id, rect)| {
                self.workspaces().current().layout.get(*frame_id)
                    .and_then(|n| n.as_frame())
                    .map(|frame| FrameData {
                        frame_id: *frame_id,
                        rect: rect.clone(),
                        windows: frame.windows.clone(),
                        focused_idx: frame.focused,
                        vertical_tabs: frame.vertical_tabs,
                    })
            })
            .collect();

        let border = self.config.border_width;
        let tab_bar_height = self.config.tab_bar_height;
        let vertical_tab_width = self.config.vertical_tab_width;

        for fd in &frame_data {
            // Calculate client area based on tab orientation
            // Only show tab bar for frames with windows
            let has_tabs = !fd.windows.is_empty();
            let (client_x, client_y, client_width, client_height) = if !has_tabs {
                // Empty frame: use full area (no tab bar)
                (fd.rect.x, fd.rect.y, fd.rect.width, fd.rect.height)
            } else if fd.vertical_tabs {
                // Vertical tabs: client area is to the right of the tab bar
                (
                    fd.rect.x + vertical_tab_width as i32,
                    fd.rect.y,
                    fd.rect.width.saturating_sub(vertical_tab_width),
                    fd.rect.height,
                )
            } else {
                // Horizontal tabs: client area is below the tab bar
                (
                    fd.rect.x,
                    fd.rect.y + tab_bar_height as i32,
                    fd.rect.width,
                    fd.rect.height.saturating_sub(tab_bar_height),
                )
            };

            if has_tabs {
                log::debug!("Frame {:?} has {} windows, will show tab bar (vertical={})", fd.frame_id, fd.windows.len(), fd.vertical_tabs);
                frames_with_tabs.push((fd.frame_id, fd.rect.clone(), fd.windows.len(), fd.vertical_tabs));
            } else {
                // Hide tab bar for single-window frames
                let mon_id = self.monitors.focused_id();
                let ws_idx = self.workspaces().current_index();
                if let Some(&tab_window) = self.tab_bar_windows.get(&(mon_id, ws_idx, fd.frame_id)) {
                    self.conn.unmap_window(tab_window)?;
                }
            }

            // Track empty vs non-empty frames for placeholder window management
            if fd.windows.is_empty() {
                let is_focused = fd.frame_id == focused_frame_id;
                empty_frames.push((fd.frame_id, fd.rect.clone(), is_focused));
            } else {
                non_empty_frames.push(fd.frame_id);
            }

            // Map focused window FIRST to reduce flicker (show new before hiding old)
            for (i, &window) in fd.windows.iter().enumerate() {
                if i == fd.focused_idx {
                    self.conn.configure_window(
                        window,
                        &ConfigureWindowAux::new()
                            .x(client_x)
                            .y(client_y)
                            .width(client_width.saturating_sub(border * 2))
                            .height(client_height.saturating_sub(border * 2))
                            .border_width(border),
                    )?;
                    self.conn.change_window_attributes(
                        window,
                        &ChangeWindowAttributesAux::new()
                            .border_pixel(self.config.border_focused),
                    )?;
                    self.conn.map_window(window)?;
                    self.hidden_windows.remove(&window);
                }
            }

            // Then unmap non-focused windows (hidden tabs)
            for (i, &window) in fd.windows.iter().enumerate() {
                if i != fd.focused_idx {
                    self.hidden_windows.insert(window);
                    self.conn.unmap_window(window)?;
                }
            }
        }

        // Create/update tab bars for frames with multiple windows
        for (frame_id, rect, _, vertical) in frames_with_tabs {
            let tab_window = self.get_or_create_tab_bar(frame_id, &rect, vertical)?;
            let (w, h) = if vertical {
                (self.config.vertical_tab_width, rect.height)
            } else {
                (rect.width, self.config.tab_bar_height)
            };
            log::info!("Tab bar window 0x{:x} for frame {:?} at ({}, {}) {}x{} (vertical={})",
                tab_window, frame_id, rect.x, rect.y, w, h, vertical);
            self.conn.map_window(tab_window)?;
            // Raise the tab bar above client windows
            self.conn.configure_window(
                tab_window,
                &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
            )?;
            self.draw_tab_bar(frame_id, tab_window, &rect, vertical)?;
        }

        // Create/update empty frame placeholder windows (with borders)
        for (frame_id, rect, is_focused) in empty_frames {
            self.get_or_create_empty_frame_window(frame_id, &rect, is_focused)?;
        }

        // Destroy empty frame windows for non-empty frames
        for frame_id in non_empty_frames {
            self.destroy_empty_frame_window(frame_id);
        }

        // Clean up tab bars for removed frames
        self.cleanup_tab_bars();

        // Clean up empty frame windows for removed frames
        self.cleanup_empty_frame_windows();

        // Apply floating window layout
        self.apply_floating_layout()?;

        self.conn.flush()?;
        Ok(())
    }

    /// Apply layout for floating windows in the current workspace
    fn apply_floating_layout(&mut self) -> Result<()> {
        let border = self.config.border_width;

        // Get floating windows for current workspace
        let floating_windows: Vec<_> = self.workspaces().current()
            .floating_windows
            .iter()
            .map(|f| (f.window, f.x, f.y, f.width, f.height))
            .collect();

        for (window, x, y, width, height) in floating_windows {
            // Configure window geometry
            self.conn.configure_window(
                window,
                &ConfigureWindowAux::new()
                    .x(x)
                    .y(y)
                    .width(width.saturating_sub(border * 2))
                    .height(height.saturating_sub(border * 2))
                    .border_width(border)
                    .stack_mode(StackMode::ABOVE),
            )?;

            // Make sure window is mapped
            self.conn.map_window(window)?;

            log::debug!(
                "Applied floating layout for 0x{:x}: ({}, {}) {}x{}",
                window, x, y, width, height
            );
        }

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

    /// Check if a window is currently floating
    fn is_floating(&self, window: Window) -> bool {
        self.workspaces().current().is_floating(window)
    }

    /// Find which workspace contains a window (including floating)
    fn find_window_workspace(&self, window: Window) -> Option<usize> {
        for (idx, ws) in self.monitors.focused().workspaces.workspaces.iter().enumerate() {
            // Check floating windows
            if ws.is_floating(window) {
                return Some(idx);
            }
            // Check tiled windows
            if ws.layout.find_window(window).is_some() {
                return Some(idx);
            }
        }
        None
    }

    /// Update the urgent indicator visibility based on urgent windows on other workspaces
    fn update_urgent_indicator(&mut self) -> Result<()> {
        let current_ws = self.workspaces().current_index();
        let has_other_ws_urgent = self.urgent_windows.iter().any(|&w| {
            self.find_window_workspace(w) != Some(current_ws)
        });

        if has_other_ws_urgent {
            self.show_urgent_indicator()?;
        } else {
            self.hide_urgent_indicator()?;
        }
        Ok(())
    }

    /// Show the urgent indicator in the upper-right corner
    fn show_urgent_indicator(&mut self) -> Result<()> {
        let screen = self.screen();
        let size = 16u16;
        let x = screen.width_in_pixels as i16 - size as i16 - 10;
        let y = 10i16;

        if self.urgent_indicator.is_none() {
            let window = self.conn.generate_id()?;
            self.conn.create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                window,
                self.root,
                x, y,
                size, size,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &CreateWindowAux::new()
                    .background_pixel(self.config.tab_urgent_bg)
                    .override_redirect(1), // Don't manage this window
            )?;
            self.urgent_indicator = Some(window);
        }

        let window = self.urgent_indicator.unwrap();
        self.conn.map_window(window)?;
        self.conn.configure_window(window, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE))?;

        // Draw a filled circle
        self.conn.change_gc(self.gc, &ChangeGCAux::new().foreground(self.config.tab_urgent_bg))?;
        self.conn.poly_fill_arc(
            window,
            self.gc,
            &[Arc {
                x: 0,
                y: 0,
                width: size,
                height: size,
                angle1: 0,
                angle2: 360 * 64, // Full circle (angles in 1/64 degree units)
            }],
        )?;
        self.conn.flush()?;
        Ok(())
    }

    /// Hide the urgent indicator
    fn hide_urgent_indicator(&mut self) -> Result<()> {
        if let Some(window) = self.urgent_indicator {
            self.conn.unmap_window(window)?;
            self.conn.flush()?;
        }
        Ok(())
    }

    /// Focus the oldest urgent window (FIFO order)
    fn focus_urgent(&mut self) -> Result<()> {
        if let Some(&window) = self.urgent_windows.first() {
            // Find which workspace contains this window
            if let Some(workspace_idx) = self.find_window_workspace(window) {
                // Switch to that workspace if needed
                if let Some(old_idx) = self.workspaces_mut().switch_to(workspace_idx) {
                    self.perform_workspace_switch(old_idx)?;
                }
                // Focus the window (which will clear its urgent state)
                self.suppress_enter_focus = true;
                self.focus_window(window)?;
            }
        }
        Ok(())
    }

    /// Start managing a window
    fn manage_window(&mut self, window: Window) -> Result<()> {
        // Check if already managed (either tiled or floating)
        if self.workspaces().current().layout.find_window(window).is_some() {
            return Ok(());
        }
        if self.workspaces().current().is_floating(window) {
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

        // Check if window is a dock (status bar like polybar)
        if window_query::is_dock_window(&self.conn, &self.atoms, window) {
            let struts = window_query::read_struts(&self.conn, &self.atoms, window);
            log::info!(
                "Managing dock 0x{:x}: top={}, bottom={}, left={}, right={}",
                window, struts.top, struts.bottom, struts.left, struts.right
            );
            self.dock_windows.insert(window, struts);
            // Keep dock windows above others
            self.conn.configure_window(
                window,
                &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
            )?;
            self.apply_layout()?;
            return Ok(());
        }

        // Check if window should float (based on _NET_WM_WINDOW_TYPE)
        if window_query::should_float(&self.conn, &self.atoms, window) {
            // Get window geometry for floating placement
            let geom = self.conn.get_geometry(window)?.reply()?;
            let screen = &self.conn.setup().roots[self.screen_num];

            // Center the window if it's at 0,0 (common for dialogs)
            let (x, y) = if geom.x == 0 && geom.y == 0 {
                // Center on screen
                let x = (screen.width_in_pixels as i32 - geom.width as i32) / 2;
                let y = (screen.height_in_pixels as i32 - geom.height as i32) / 2;
                (x.max(0), y.max(0))
            } else {
                (geom.x as i32, geom.y as i32)
            };

            // Add to floating windows
            self.workspaces_mut().current_mut().add_floating(
                window,
                x,
                y,
                geom.width as u32,
                geom.height as u32,
            );

            log::info!(
                "Managing floating window 0x{:x} at ({}, {}) {}x{}",
                window, x, y, geom.width, geom.height
            );

            // Trace the window being managed as floating
            self.tracer.trace_transition(&StateTransition::WindowManaged {
                window,
                frame: "floating".to_string(),
            });
        } else {
            // Add to the focused frame in our layout (tiled)
            self.workspaces_mut().current_mut().layout.add_window(window);

            // Trace the window being managed
            if let Some(frame_id) = self.workspaces().current().layout.find_window(window) {
                self.tracer.trace_transition(&StateTransition::WindowManaged {
                    window,
                    frame: format!("{:?}", frame_id),
                });
            }
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

        // Remove from icon cache to prevent stale icons when X11 reuses window IDs
        self.icon_cache.remove(&window);

        // Remove from urgent list if present
        if self.urgent_windows.contains(&window) {
            self.urgent_windows.retain(|&w| w != window);
            self.update_urgent_indicator()?;
        }

        // Remove from dock windows if present
        if self.dock_windows.remove(&window).is_some() {
            log::info!("Unmanaging dock window 0x{:x}", window);
            self.apply_layout()?;
            return Ok(());
        }

        // Clear fullscreen if this window was fullscreen (check all workspaces)
        for ws in &mut self.monitors.focused_mut().workspaces.workspaces {
            if ws.fullscreen_window == Some(window) {
                ws.fullscreen_window = None;
                log::info!("Cleared fullscreen state for destroyed window 0x{:x}", window);
                break;
            }
        }

        // Find which workspace contains this window (search ALL workspaces)
        let ws_idx = self.find_window_workspace(window);

        if let Some(ws_idx) = ws_idx {
            // Check if floating on that workspace
            if self.monitors.focused().workspaces.workspaces[ws_idx].is_floating(window) {
                self.tracer.trace_transition(&StateTransition::WindowUnmanaged {
                    window,
                    reason: UnmanageReason::ClientDestroyed,
                });

                self.monitors.focused_mut().workspaces.workspaces[ws_idx].remove_floating(window);
                log::info!("Unmanaging floating window 0x{:x} from workspace {}", window, ws_idx + 1);
            } else {
                // Tiled window
                self.tracer.trace_transition(&StateTransition::WindowUnmanaged {
                    window,
                    reason: UnmanageReason::ClientDestroyed,
                });

                self.monitors.focused_mut().workspaces.workspaces[ws_idx].layout.remove_window(window);
                log::info!("Unmanaging window 0x{:x} from workspace {}", window, ws_idx + 1);
            }

            // Update EWMH client list
            self.update_client_list()?;

            // If this was focused, focus another window
            if self.focused_window == Some(window) {
                self.focused_window = None;
                self.focus_next_available_window()?;
            }

            // Re-apply layout
            self.apply_layout()?;
        }

        Ok(())
    }

    /// Focus the next available window (floating or tiled)
    fn focus_next_available_window(&mut self) -> Result<()> {
        // First try floating windows
        let floating_windows = self.workspaces().current().floating_window_ids();
        if let Some(&w) = floating_windows.first() {
            return self.focus_window(w);
        }

        // Try to focus the window in the focused frame
        if let Some(frame) = self.workspaces().current().layout.focused_frame() {
            if let Some(w) = frame.focused_window() {
                return self.focus_window(w);
            }
        }

        // If still no focus, try any tiled window
        let windows = self.workspaces().current().layout.all_windows();
        if let Some(&w) = windows.first() {
            self.focus_window(w)?;
        } else {
            self.update_active_window()?;
        }

        Ok(())
    }

    /// Toggle a window between floating and tiled states
    /// If window is None, uses the focused window
    fn toggle_float(&mut self, window: Option<Window>) -> Result<()> {
        let window = match window.or(self.focused_window) {
            Some(w) => w,
            None => {
                log::info!("No window to toggle float");
                return Ok(());
            }
        };

        if self.workspaces().current().is_floating(window) {
            // Currently floating -> make it tiled
            if let Some(float_info) = self.workspaces_mut().current_mut().remove_floating(window) {
                log::info!(
                    "Tiling floating window 0x{:x} (was at {}, {} {}x{})",
                    window, float_info.x, float_info.y, float_info.width, float_info.height
                );

                // Add to the focused frame in the layout
                self.workspaces_mut().current_mut().layout.add_window(window);

                // Apply layout and focus
                self.apply_layout()?;
                self.focus_window(window)?;
            }
        } else {
            // Currently tiled -> make it floating
            // Get current geometry before removing from layout
            let geom = self.conn.get_geometry(window)?.reply()?;

            // Remove from tiled layout
            if let Some(_frame_id) = self.workspaces_mut().current_mut().layout.remove_window(window) {
                log::info!(
                    "Floating window 0x{:x} at ({}, {}) {}x{}",
                    window, geom.x, geom.y, geom.width, geom.height
                );

                // Add to floating windows with current geometry
                self.workspaces_mut().current_mut().add_floating(
                    window,
                    geom.x as i32,
                    geom.y as i32,
                    geom.width as u32,
                    geom.height as u32,
                );

                // Apply layout and focus
                self.apply_layout()?;
                self.focus_window(window)?;
            }
        }

        Ok(())
    }

    /// Toggle fullscreen mode for a window
    /// If window is None, uses the focused window
    fn toggle_fullscreen(&mut self, window: Option<Window>) -> Result<()> {
        let window = match window.or(self.focused_window) {
            Some(w) => w,
            None => {
                log::info!("No window to toggle fullscreen");
                return Ok(());
            }
        };

        let is_fullscreen = self.workspaces().current().fullscreen_window == Some(window);

        if is_fullscreen {
            // Exit fullscreen
            log::info!("Exiting fullscreen for window 0x{:x}", window);
            self.workspaces_mut().current_mut().fullscreen_window = None;

            // Update _NET_WM_STATE to remove fullscreen
            self.update_wm_state(window, false)?;
        } else {
            // Enter fullscreen
            log::info!("Entering fullscreen for window 0x{:x}", window);
            self.workspaces_mut().current_mut().fullscreen_window = Some(window);

            // Update _NET_WM_STATE to add fullscreen
            self.update_wm_state(window, true)?;
        }

        self.apply_layout()?;
        self.focus_window(window)?;
        Ok(())
    }

    /// Update _NET_WM_STATE property for fullscreen
    fn update_wm_state(&self, window: Window, fullscreen: bool) -> Result<()> {
        ewmh::update_wm_state_fullscreen(&self.conn, &self.atoms, window, fullscreen)
    }

    /// Toggle vertical tabs on the focused frame
    fn toggle_vertical_tabs(&mut self) -> Result<()> {
        let is_vertical = self.workspaces_mut().current_mut().layout.toggle_vertical_tabs();
        log::info!("Toggled tabs to {}", if is_vertical { "vertical" } else { "horizontal" });
        self.apply_layout()?;
        Ok(())
    }

    /// Cycle focus to the next/previous window (across all frames and floating windows)
    fn cycle_focus(&mut self, forward: bool) -> Result<()> {
        // Build a list of all windows: tiled first, then floating
        let mut windows = self.workspaces().current().layout.all_windows();
        windows.extend(self.workspaces().current().floating_window_ids());

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
        let old_tab = self.workspaces().current().layout.focused_frame().map(|f| f.focused);

        if let Some(window) = self.workspaces_mut().current_mut().layout.cycle_tab(forward) {
            // Trace the tab switch
            if let (Some(old), Some(frame)) = (old_tab, self.workspaces().current().layout.focused_frame()) {
                self.tracer.trace_transition(&StateTransition::TabSwitched {
                    frame: format!("{:?}", self.workspaces().current().layout.focused),
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
        let old_tab = self.workspaces().current().layout.focused_frame().map(|f| f.focused);

        if let Some(window) = self.workspaces_mut().current_mut().layout.focus_tab(num.saturating_sub(1)) {
            // Trace the tab switch
            if let (Some(old), Some(frame)) = (old_tab, self.workspaces().current().layout.focused_frame()) {
                if old != frame.focused {
                    self.tracer.trace_transition(&StateTransition::TabSwitched {
                        frame: format!("{:?}", self.workspaces().current().layout.focused),
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
        let old_frame = self.workspaces().current().layout.focused;
        self.workspaces_mut().current_mut().layout.split_focused(direction);
        let new_frame = self.workspaces().current().layout.focused;

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
        let old_focused_frame = self.workspaces().current().layout.focused;
        let screen_rect = self.usable_screen();
        let geometries = self.workspaces().current().layout.calculate_geometries(screen_rect, self.config.gap);

        if self.workspaces_mut().current_mut().layout.focus_spatial(direction, &geometries) {
            let new_focused_frame = self.workspaces().current().layout.focused;

            // Focus the window in the new frame
            if let Some(frame) = self.workspaces().current().layout.focused_frame() {
                if let Some(window) = frame.focused_window() {
                    self.focus_window(window)?;
                }
            }

            // Redraw tab bars and update empty frame borders for old and new focused frames
            if old_focused_frame != new_focused_frame {
                let geometry_map: std::collections::HashMap<_, _> = geometries.into_iter().collect();
                let mon_id = self.monitors.focused_id();
                let ws_idx = self.workspaces().current_index();

                if let Some(&tab_window) = self.tab_bar_windows.get(&(mon_id, ws_idx, old_focused_frame)) {
                    if let Some(rect) = geometry_map.get(&old_focused_frame) {
                        let vertical = self.workspaces().current().layout.get(old_focused_frame)
                            .and_then(|n| n.as_frame())
                            .map(|f| f.vertical_tabs)
                            .unwrap_or(false);
                        self.draw_tab_bar(old_focused_frame, tab_window, rect, vertical)?;
                    }
                }
                if let Some(&tab_window) = self.tab_bar_windows.get(&(mon_id, ws_idx, new_focused_frame)) {
                    if let Some(rect) = geometry_map.get(&new_focused_frame) {
                        let vertical = self.workspaces().current().layout.get(new_focused_frame)
                            .and_then(|n| n.as_frame())
                            .map(|f| f.vertical_tabs)
                            .unwrap_or(false);
                        self.draw_tab_bar(new_focused_frame, tab_window, rect, vertical)?;
                    }
                }

                // Update empty frame window borders
                if let Some(&empty_window) = self.empty_frame_windows.get(&(mon_id, ws_idx, old_focused_frame)) {
                    self.conn.change_window_attributes(
                        empty_window,
                        &ChangeWindowAttributesAux::new()
                            .border_pixel(self.config.border_unfocused),
                    )?;
                }
                if let Some(&empty_window) = self.empty_frame_windows.get(&(mon_id, ws_idx, new_focused_frame)) {
                    self.conn.change_window_attributes(
                        empty_window,
                        &ChangeWindowAttributesAux::new()
                            .border_pixel(self.config.border_focused),
                    )?;
                }

                self.conn.flush()?;
            }
        }
        Ok(())
    }

    /// Focus a specific monitor by ID
    fn focus_monitor(&mut self, monitor_id: MonitorId) -> Result<()> {
        let old_monitor_id = self.monitors.focused_id();
        if old_monitor_id == monitor_id {
            return Ok(()); // Already focused
        }

        // Save current focused window to old monitor's workspace
        if let Some(window) = self.focused_window {
            self.monitors.focused_mut().workspaces.current_mut().last_focused_window = Some(window);
        }

        // Switch to new monitor
        if !self.monitors.set_focused(monitor_id) {
            log::warn!("Failed to focus monitor {:?} - monitor not found", monitor_id);
            return Ok(());
        }

        log::info!("Focused monitor {:?}", monitor_id);

        // Restore focus to new monitor's last focused window
        let last_focused = self.monitors.focused().workspaces.current().last_focused_window;
        if let Some(window) = last_focused {
            self.focus_window(window)?;
        } else {
            // No last focused window - try to focus first window in current workspace
            if let Some(frame) = self.workspaces().current().layout.focused_frame() {
                if let Some(window) = frame.focused_window() {
                    self.focus_window(window)?;
                }
            }
        }

        Ok(())
    }

    /// Focus monitor in the given direction
    fn focus_monitor_direction(&mut self, direction: Direction) -> Result<()> {
        if let Some(target_monitor) = self.monitors.monitor_in_direction(direction) {
            self.focus_monitor(target_monitor)?;
        }
        Ok(())
    }

    /// Focus a window
    fn focus_window(&mut self, window: Window) -> Result<()> {
        // Capture old focus for tracing
        let old_focused = self.focused_window;

        // Unfocus the previously focused window
        if let Some(old) = self.focused_window {
            if old != window {
                // Check if old window is tiled or floating
                let is_tiled = self.workspaces().current().layout.find_window(old).is_some();
                let is_floating = self.workspaces().current().is_floating(old);
                if is_tiled || is_floating {
                    self.conn.change_window_attributes(
                        old,
                        &ChangeWindowAttributesAux::new()
                            .border_pixel(self.config.border_unfocused),
                    )?;
                }
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

        // Clear urgent state if the window was urgent
        if let Some(pos) = self.urgent_windows.iter().position(|&w| w == window) {
            self.urgent_windows.remove(pos);
            log::info!("Cleared urgent state for window 0x{:x}", window);
            self.redraw_tabs_for_window(window)?;
            self.update_urgent_indicator()?;
        }

        // Trace focus change
        if old_focused != Some(window) {
            self.tracer.trace_transition(&StateTransition::FocusChanged {
                from: old_focused,
                to: Some(window),
            });
        }

        // For floating windows, just update EWMH and return
        if self.workspaces().current().is_floating(window) {
            log::info!("Focused floating window 0x{:x}", window);
            self.update_active_window()?;
            self.conn.flush()?;
            return Ok(());
        }

        // Also update the layout's focused frame to match (for tiled windows)
        if let Some(frame_id) = self.workspaces().current().layout.find_window(window) {
            let old_focused_frame = self.workspaces().current().layout.focused;
            self.workspaces_mut().current_mut().layout.focused = frame_id;
            let mon_id = self.monitors.focused_id();
            let ws_idx = self.workspaces().current_index();

            // Re-raise the tab bar if this frame has one (so it stays above the window)
            if let Some(&tab_window) = self.tab_bar_windows.get(&(mon_id, ws_idx, frame_id)) {
                self.conn.configure_window(
                    tab_window,
                    &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
                )?;
            }

            // Redraw tab bars (always redraw current frame, also old frame if different)
            let screen_rect = self.usable_screen();
            let geometries = self.workspaces().current().layout.calculate_geometries(screen_rect, self.config.gap);
            let geometry_map: std::collections::HashMap<_, _> = geometries.into_iter().collect();

            // Redraw old focused frame's tab bar if it changed
            if old_focused_frame != frame_id {
                if let Some(&tab_window) = self.tab_bar_windows.get(&(mon_id, ws_idx, old_focused_frame)) {
                    if let Some(rect) = geometry_map.get(&old_focused_frame) {
                        let vertical = self.workspaces().current().layout.get(old_focused_frame)
                            .and_then(|n| n.as_frame())
                            .map(|f| f.vertical_tabs)
                            .unwrap_or(false);
                        self.draw_tab_bar(old_focused_frame, tab_window, rect, vertical)?;
                    }
                }
            }

            // Redraw current frame's tab bar (unless apply_layout() just did it)
            if !self.skip_focus_tab_bar_redraw {
                if let Some(&tab_window) = self.tab_bar_windows.get(&(mon_id, ws_idx, frame_id)) {
                    if let Some(rect) = geometry_map.get(&frame_id) {
                        let vertical = self.workspaces().current().layout.get(frame_id)
                            .and_then(|n| n.as_frame())
                            .map(|f| f.vertical_tabs)
                            .unwrap_or(false);
                        self.draw_tab_bar(frame_id, tab_window, rect, vertical)?;
                    }
                }
            }
        }

        // Update EWMH active window
        self.update_active_window()?;

        self.conn.flush()?;

        Ok(())
    }

    /// Close the focused window gracefully
    fn close_focused_window(&self) -> Result<()> {
        if let Some(window) = self.focused_window {
            log::info!("Closing window 0x{:x}", window);

            if window_query::supports_delete_protocol(&self.conn, &self.atoms, window) {
                log::debug!("Using WM_DELETE_WINDOW protocol");
                window_query::send_delete_window(&self.conn, &self.atoms, window)?;
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
            if self.workspaces().current().layout.find_window(window).is_some() {
                self.suppress_enter_focus = true;
                self.focus_window(window)?;
            } else {
                // Check other workspaces and switch if found
                for (idx, ws) in self.monitors.focused().workspaces.workspaces.iter().enumerate() {
                    if ws.layout.find_window(window).is_some() {
                        // Switch to that workspace, then focus
                        if let Some(old_idx) = self.workspaces_mut().switch_to(idx) {
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

            if window_query::supports_delete_protocol(&self.conn, &self.atoms, window) {
                window_query::send_delete_window(&self.conn, &self.atoms, window)?;
            } else {
                self.conn.kill_client(window)?;
                self.conn.flush()?;
            }
        } else if msg_type == self.atoms.net_current_desktop {
            // _NET_CURRENT_DESKTOP: Switch to workspace
            let desktop = event.data.as_data32()[0] as usize;
            log::info!("ClientMessage: _NET_CURRENT_DESKTOP to {}", desktop);

            if let Some(old_idx) = self.workspaces_mut().switch_to(desktop) {
                self.perform_workspace_switch(old_idx)?;
            }
        } else if msg_type == self.atoms.net_wm_desktop {
            // _NET_WM_DESKTOP: Move window to workspace
            let window = event.window;
            let desktop = event.data.as_data32()[0] as usize;
            log::info!("ClientMessage: _NET_WM_DESKTOP move 0x{:x} to {}", window, desktop);

            self.move_window_to_workspace(window, desktop)?;
        } else if msg_type == self.atoms.net_wm_state {
            // _NET_WM_STATE: Change window state (fullscreen, etc.)
            // data[0]: action (0=remove, 1=add, 2=toggle)
            // data[1], data[2]: state atoms to change
            let data = event.data.as_data32();
            let action = data[0];
            let state1 = data[1];
            let state2 = data[2];
            let window = event.window;

            log::info!(
                "ClientMessage: _NET_WM_STATE for 0x{:x}, action={}, state1={}, state2={}",
                window, action, state1, state2
            );

            // Check if fullscreen state is being changed
            let fullscreen_atom = self.atoms.net_wm_state_fullscreen;
            if state1 == fullscreen_atom || state2 == fullscreen_atom {
                let is_fullscreen = self.workspaces().current().fullscreen_window == Some(window);
                let should_fullscreen = match action {
                    0 => false,        // _NET_WM_STATE_REMOVE
                    1 => true,         // _NET_WM_STATE_ADD
                    2 => !is_fullscreen, // _NET_WM_STATE_TOGGLE
                    _ => is_fullscreen,
                };

                if should_fullscreen != is_fullscreen {
                    self.toggle_fullscreen(Some(window))?;
                }
            }
        }

        Ok(())
    }

    /// Move a window to a different workspace
    fn move_window_to_workspace(&mut self, window: Window, target: usize) -> Result<()> {
        if target >= 9 {
            return Ok(());
        }

        let current_ws = self.workspaces().current_index();

        // Find which workspace has this window
        let source_ws = self.monitors.focused().workspaces.workspaces.iter()
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
        self.monitors.focused_mut().workspaces.workspaces[source_ws].layout.remove_window(window);

        // Add to target workspace
        self.monitors.focused_mut().workspaces.workspaces[target].layout.add_window(window);

        // Update window's _NET_WM_DESKTOP property
        self.set_window_desktop(window, target)?;

        // If moving from current workspace, hide the window
        if source_ws == current_ws {
            self.hidden_windows.insert(window);
            self.conn.unmap_window(window)?;

            // If this was the focused window, focus something else
            if self.focused_window == Some(window) {
                self.focused_window = None;
                if let Some(frame) = self.workspaces().current().layout.focused_frame() {
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
                if self.workspaces().current().layout.find_window(e.window).is_some() {
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
                    // Check if window is tiled or floating
                    let is_tiled = self.workspaces().current().layout.find_window(e.event).is_some();
                    let is_floating = self.workspaces().current().is_floating(e.event);
                    if is_tiled || is_floating {
                        log::debug!("EnterNotify for window 0x{:x}", e.event);
                        self.focus_window(e.event)?;
                    }
                }
                self.suppress_enter_focus = false;

                // Update hover cursor when entering root window (gap area) or leaving a window
                if self.drag_state.is_none() {
                    self.update_hover_cursor(e.root_x as i32, e.root_y as i32)?;
                }
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
                self.tracer.trace_x11_event("PropertyNotify", Some(e.window), &format!("atom={}", e.atom));
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
                // Handle urgent state changes (EWMH _NET_WM_STATE or legacy WM_HINTS)
                if e.atom == self.atoms.net_wm_state || e.atom == u32::from(AtomEnum::WM_HINTS) {
                    let was_urgent = self.urgent_windows.contains(&e.window);
                    let is_urgent = window_query::is_window_urgent(&self.conn, &self.atoms, e.window);
                    if is_urgent && !was_urgent {
                        self.urgent_windows.push(e.window); // Add to end (newest)
                        log::info!("Window 0x{:x} is now urgent", e.window);
                        self.redraw_tabs_for_window(e.window)?;
                        self.update_urgent_indicator()?;
                    } else if !is_urgent && was_urgent {
                        self.urgent_windows.retain(|&w| w != e.window);
                        log::info!("Window 0x{:x} is no longer urgent", e.window);
                        self.redraw_tabs_for_window(e.window)?;
                        self.update_urgent_indicator()?;
                    }
                }
                // Handle strut changes for dock windows
                if e.atom == self.atoms.net_wm_strut || e.atom == self.atoms.net_wm_strut_partial {
                    if self.dock_windows.contains_key(&e.window) {
                        let new_struts = window_query::read_struts(&self.conn, &self.atoms, e.window);
                        log::info!(
                            "Dock 0x{:x} struts changed: top={}, bottom={}, left={}, right={}",
                            e.window, new_struts.top, new_struts.bottom, new_struts.left, new_struts.right
                        );
                        self.dock_windows.insert(e.window, new_struts);
                        self.apply_layout()?;
                    }
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
                    // Copy values to avoid borrow conflict
                    let split_id = *split_id;
                    let direction = *direction;
                    let split_start = *split_start;
                    let total_size = *total_size;

                    // Calculate new ratio from mouse position
                    let mouse_pos = match direction {
                        SplitDirection::Horizontal => e.root_x as i32,
                        SplitDirection::Vertical => e.root_y as i32,
                    };
                    let ratio = ((mouse_pos - split_start) as f32) / (total_size as f32);

                    // Update split and relayout
                    if self.workspaces_mut().current_mut().layout.set_split_ratio(split_id, ratio) {
                        self.apply_layout()?;
                    }
                }
                // Handle floating window move
                else if let Some(DragState::FloatMove { window, start_x, start_y, win_x, win_y }) = &self.drag_state {
                    let dx = e.root_x as i32 - start_x;
                    let dy = e.root_y as i32 - start_y;
                    let new_x = win_x + dx;
                    let new_y = win_y + dy;

                    let window = *window;
                    if let Some(float) = self.workspaces_mut().current_mut().find_floating_mut(window) {
                        float.x = new_x;
                        float.y = new_y;
                    }
                    self.apply_floating_layout()?;
                    self.conn.flush()?;
                }
                // Handle floating window resize
                else if let Some(DragState::FloatResize { window, edge, start_x, start_y, original_x, original_y, original_w, original_h }) = &self.drag_state {
                    let dx = e.root_x as i32 - start_x;
                    let dy = e.root_y as i32 - start_y;

                    let edge = *edge;
                    let window = *window;
                    let original_x = *original_x;
                    let original_y = *original_y;
                    let original_w = *original_w;
                    let original_h = *original_h;

                    // Calculate new geometry based on which edge is being dragged
                    const MIN_SIZE: u32 = 100;
                    let (mut new_x, mut new_y, mut new_w, mut new_h) = (original_x, original_y, original_w, original_h);

                    match edge {
                        ResizeEdge::Left | ResizeEdge::TopLeft | ResizeEdge::BottomLeft => {
                            let max_dx = (original_w as i32 - MIN_SIZE as i32).max(0);
                            let clamped_dx = dx.min(max_dx);
                            new_x = original_x + clamped_dx;
                            new_w = (original_w as i32 - clamped_dx).max(MIN_SIZE as i32) as u32;
                        }
                        ResizeEdge::Right | ResizeEdge::TopRight | ResizeEdge::BottomRight => {
                            new_w = (original_w as i32 + dx).max(MIN_SIZE as i32) as u32;
                        }
                        _ => {}
                    }

                    match edge {
                        ResizeEdge::Top | ResizeEdge::TopLeft | ResizeEdge::TopRight => {
                            let max_dy = (original_h as i32 - MIN_SIZE as i32).max(0);
                            let clamped_dy = dy.min(max_dy);
                            new_y = original_y + clamped_dy;
                            new_h = (original_h as i32 - clamped_dy).max(MIN_SIZE as i32) as u32;
                        }
                        ResizeEdge::Bottom | ResizeEdge::BottomLeft | ResizeEdge::BottomRight => {
                            new_h = (original_h as i32 + dy).max(MIN_SIZE as i32) as u32;
                        }
                        _ => {}
                    }

                    if let Some(float) = self.workspaces_mut().current_mut().find_floating_mut(window) {
                        float.x = new_x;
                        float.y = new_y;
                        float.width = new_w;
                        float.height = new_h;
                    }
                    self.apply_floating_layout()?;
                    self.conn.flush()?;
                }
                // Tab drags don't need motion processing - drop target determined at release
                else if self.drag_state.is_none() {
                    // No drag in progress - update cursor based on hover position
                    self.update_hover_cursor(e.root_x as i32, e.root_y as i32)?;
                }
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
        let mon_id = self.monitors.focused_id();
        let ws_idx = self.workspaces().current_index();
        // Find which frame this tab bar belongs to
        for (&(m, idx, frame_id), &tab_window) in &self.tab_bar_windows {
            if m == mon_id && idx == ws_idx && tab_window == event.window {
                // Get vertical_tabs state
                let vertical = self.workspaces().current().layout.get(frame_id)
                    .and_then(|n| n.as_frame())
                    .map(|f| f.vertical_tabs)
                    .unwrap_or(false);

                // Get frame geometry to redraw
                let screen_rect = self.usable_screen();
                let geometries = self.workspaces().current().layout.calculate_geometries(screen_rect, self.config.gap);
                for (fid, rect) in geometries {
                    if fid == frame_id {
                        self.draw_tab_bar(frame_id, tab_window, &rect, vertical)?;
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
            self.workspaces().current().layout.find_split_at_gap(screen, self.config.gap, event.root_x as i32, event.root_y as i32)
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
        let geometries = self.workspaces().current().layout.calculate_geometries(screen, self.config.gap);

        for (frame_id, rect) in &geometries {
            if let Some(frame) = self.workspaces().current().layout.get(*frame_id).and_then(|n| n.as_frame()) {
                if frame.is_empty() {
                    let click_x = event.root_x as i32;
                    let click_y = event.root_y as i32;
                    if click_x >= rect.x && click_x < rect.x + rect.width as i32 &&
                       click_y >= rect.y && click_y < rect.y + rect.height as i32 {
                        // Focus this empty frame
                        self.workspaces_mut().current_mut().layout.focused = *frame_id;
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
        let mon_id = self.monitors.focused_id();
        let ws_idx = self.workspaces().current_index();

        // Handle middle click - remove empty frame
        if event.detail == 2 {
            if let Some(frame) = self.workspaces().current().layout.get(frame_id).and_then(|n| n.as_frame()) {
                if frame.is_empty() {
                    // Remove tab bar window and its pixmap buffer
                    if let Some(tab_window) = self.tab_bar_windows.remove(&(mon_id, ws_idx, frame_id)) {
                        if let Some(pixmap) = self.tab_bar_pixmaps.remove(&tab_window) {
                            let _ = self.conn.free_pixmap(pixmap);
                        }
                        self.conn.destroy_window(tab_window)?;
                    }
                    // Remove this specific empty frame from layout
                    self.workspaces_mut().current_mut().layout.remove_frame_by_id(frame_id);
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
        if let Some(frame) = self.workspaces().current().layout.get(frame_id).and_then(|n| n.as_frame()) {
            let num_tabs = frame.windows.len();
            let is_vertical = frame.vertical_tabs;
            if num_tabs == 0 {
                // Focus the empty frame
                self.workspaces_mut().current_mut().layout.focused = frame_id;
                self.apply_layout()?;
                return Ok(());
            }

            // Calculate which tab was clicked
            let clicked_tab = if is_vertical {
                // Vertical tabs: each tab is a square of vertical_tab_width size
                let tab_size = self.config.vertical_tab_width;
                let click_y = event.event_y as u32;
                let index = click_y / tab_size;
                if (index as usize) < num_tabs {
                    Some(index as usize)
                } else {
                    None
                }
            } else {
                // Horizontal tabs: use content-based layout
                let tab_layout = self.calculate_tab_layout(frame_id);
                let click_x = event.event_x as i16;
                tab_layout.iter().enumerate()
                    .find(|(_, (x, w))| click_x >= *x && click_x < *x + *w as i16)
                    .map(|(i, _)| i)
            };

            if let Some(clicked_tab) = clicked_tab {
                // Get the window at this tab
                let window = frame.windows[clicked_tab];

                // Focus this tab immediately
                if let Some(w) = self.workspaces_mut().current_mut().layout.focus_tab(clicked_tab) {
                    self.apply_layout()?;
                    // Skip redundant tab bar redraw - apply_layout() just did it
                    self.skip_focus_tab_bar_redraw = true;
                    self.focus_window(w)?;
                    self.skip_focus_tab_bar_redraw = false;
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

        // Check for click on a floating window
        if self.try_handle_float_click(&event)? {
            return Ok(());
        }

        let mon_id = self.monitors.focused_id();
        let ws_idx = self.workspaces().current_index();

        // Check for click on an empty frame window (content area)
        let clicked_empty_frame = self.empty_frame_windows.iter()
            .find(|(&(m, idx, _), &empty_window)| m == mon_id && idx == ws_idx && empty_window == event.event)
            .map(|(&(_, _, frame_id), _)| frame_id);

        if let Some(frame_id) = clicked_empty_frame {
            if event.detail == 2 {
                // Middle-click: remove empty frame
                if let Some(empty_window) = self.empty_frame_windows.remove(&(mon_id, ws_idx, frame_id)) {
                    self.conn.destroy_window(empty_window)?;
                }
                if let Some(tab_window) = self.tab_bar_windows.remove(&(mon_id, ws_idx, frame_id)) {
                    if let Some(pixmap) = self.tab_bar_pixmaps.remove(&tab_window) {
                        let _ = self.conn.free_pixmap(pixmap);
                    }
                    self.conn.destroy_window(tab_window)?;
                }
                self.workspaces_mut().current_mut().layout.remove_frame_by_id(frame_id);
                self.apply_layout()?;
                log::info!("Removed empty frame via middle-click on content area");
                return Ok(());
            } else if event.detail == 1 {
                // Left-click: focus the empty frame
                self.workspaces_mut().current_mut().layout.focused = frame_id;
                self.apply_layout()?;
                return Ok(());
            }
        }

        // Find which frame's tab bar was clicked
        let clicked_frame = self.tab_bar_windows.iter()
            .find(|(&(m, idx, _), &tab_window)| m == mon_id && idx == ws_idx && tab_window == event.event)
            .map(|(&(_, _, frame_id), _)| frame_id);

        if let Some(frame_id) = clicked_frame {
            self.handle_tab_click(&event, frame_id)?;
        }

        Ok(())
    }

    /// Try to handle a click on a floating window
    /// Returns Ok(true) if a floating window was clicked, Ok(false) otherwise
    fn try_handle_float_click(&mut self, event: &ButtonPressEvent) -> Result<bool> {
        // Only handle left-click (button 1)
        if event.detail != 1 {
            return Ok(false);
        }

        // Check if the clicked window is floating
        let clicked_window = event.event;
        if !self.workspaces().current().is_floating(clicked_window) {
            return Ok(false);
        }

        // Get the floating window info
        let float_info = match self.workspaces().current().find_floating(clicked_window) {
            Some(f) => *f,
            None => return Ok(false),
        };

        // Detect if click is near an edge for resizing
        const EDGE_SIZE: i32 = 8;
        let local_x = event.event_x as i32;
        let local_y = event.event_y as i32;
        let w = float_info.width as i32;
        let h = float_info.height as i32;

        let at_left = local_x < EDGE_SIZE;
        let at_right = local_x >= w - EDGE_SIZE;
        let at_top = local_y < EDGE_SIZE;
        let at_bottom = local_y >= h - EDGE_SIZE;

        let edge = match (at_top, at_bottom, at_left, at_right) {
            (true, false, true, false) => Some(ResizeEdge::TopLeft),
            (true, false, false, true) => Some(ResizeEdge::TopRight),
            (false, true, true, false) => Some(ResizeEdge::BottomLeft),
            (false, true, false, true) => Some(ResizeEdge::BottomRight),
            (true, false, false, false) => Some(ResizeEdge::Top),
            (false, true, false, false) => Some(ResizeEdge::Bottom),
            (false, false, true, false) => Some(ResizeEdge::Left),
            (false, false, false, true) => Some(ResizeEdge::Right),
            _ => None,
        };

        // Focus the floating window
        self.focus_window(clicked_window)?;

        if let Some(resize_edge) = edge {
            // Start resize drag
            log::info!("Starting float resize on 0x{:x} edge {:?}", clicked_window, resize_edge);

            self.conn.grab_pointer(
                false,
                self.root,
                EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
                x11rb::NONE,
                self.cursor_for_edge(resize_edge),
                x11rb::CURRENT_TIME,
            )?;

            self.drag_state = Some(DragState::FloatResize {
                window: clicked_window,
                edge: resize_edge,
                start_x: event.root_x as i32,
                start_y: event.root_y as i32,
                original_x: float_info.x,
                original_y: float_info.y,
                original_w: float_info.width,
                original_h: float_info.height,
            });
        } else {
            // Start move drag
            log::info!("Starting float move on 0x{:x}", clicked_window);

            self.conn.grab_pointer(
                false,
                self.root,
                EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
                x11rb::NONE,
                x11rb::NONE,
                x11rb::CURRENT_TIME,
            )?;

            self.drag_state = Some(DragState::FloatMove {
                window: clicked_window,
                start_x: event.root_x as i32,
                start_y: event.root_y as i32,
                win_x: float_info.x,
                win_y: float_info.y,
            });
        }

        self.conn.flush()?;
        Ok(true)
    }

    /// Find the drop target for a drag operation
    /// Returns (frame_id, tab_index) - tab_index is the position to insert at
    fn find_drop_target(&self, root_x: i16, root_y: i16) -> Result<(Option<NodeId>, Option<usize>)> {
        let mon_id = self.monitors.focused_id();
        let ws_idx = self.workspaces().current_index();
        // Check each tab bar window first (higher priority than content area)
        for (&(m, idx, frame_id), &tab_window) in &self.tab_bar_windows {
            if m != mon_id || idx != ws_idx {
                continue;
            }
            let geom = self.conn.get_geometry(tab_window)?.reply()?;
            let coords = self.conn.translate_coordinates(tab_window, self.root, 0, 0)?.reply()?;

            let tab_x = coords.dst_x as i16;
            let tab_y = coords.dst_y as i16;

            if root_x >= tab_x && root_x < tab_x + geom.width as i16 &&
               root_y >= tab_y && root_y < tab_y + geom.height as i16 {
                // Cursor is over this tab bar
                // Check if this frame uses vertical tabs
                let is_vertical = self.workspaces().current().layout
                    .get(frame_id)
                    .and_then(|n| n.as_frame())
                    .map(|f| f.vertical_tabs)
                    .unwrap_or(false);

                let target_index = if is_vertical {
                    // Vertical tabs: use y position
                    let local_y = root_y - tab_y;
                    let tab_size = self.config.vertical_tab_width;
                    if local_y >= 0 {
                        let num_tabs = self.workspaces().current().layout
                            .get(frame_id)
                            .and_then(|n| n.as_frame())
                            .map(|f| f.windows.len())
                            .unwrap_or(0);
                        let index = (local_y as u32) / tab_size;
                        if (index as usize) < num_tabs {
                            Some(index as usize)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    // Horizontal tabs: use content-based layout
                    let tab_layout = self.calculate_tab_layout(frame_id);
                    let local_x = root_x - tab_x;
                    tab_layout.iter().enumerate()
                        .find(|(_, (x, w))| local_x >= *x && local_x < *x + *w as i16)
                        .map(|(i, _)| i)
                };

                if let Some(idx) = target_index {
                    return Ok((Some(frame_id), Some(idx)));
                }
                return Ok((Some(frame_id), None));
            }
        }

        // Check frame content areas (for dropping into single-window frames or frames without visible tab bars)
        let screen_rect = self.usable_screen();
        let geometries = self.workspaces().current().layout.calculate_geometries(screen_rect, self.config.gap);

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
                                self.workspaces_mut().current_mut().layout.reorder_tab(target_frame, source_index, target_idx);
                                log::info!("Reordered tab from {} to {}", source_index + 1, target_idx + 1);
                            }
                        }
                    } else {
                        // Move to different frame
                        self.workspaces_mut().current_mut().layout.move_window_to_frame(window, source_frame, target_frame);

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
            DragState::FloatMove { window, .. } => {
                log::info!("Float move completed for window 0x{:x}", window);
            }
            DragState::FloatResize { window, edge, .. } => {
                log::info!("Float resize completed for window 0x{:x} (edge: {:?})", window, edge);
            }
        }

        Ok(())
    }

    /// Resize the current split
    fn resize_split(&mut self, grow: bool) -> Result<()> {
        let delta = if grow { 0.05 } else { -0.05 };
        if self.workspaces_mut().current_mut().layout.resize_focused_split(delta) {
            // Trace the resize (simplified - we don't track exact ratios)
            self.tracer.trace_transition(&StateTransition::SplitResized {
                split: format!("{:?}", self.workspaces().current().layout.focused),
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
        let from_frame = self.workspaces().current().layout.focused;

        if let Some(window) = self.workspaces_mut().current_mut().layout.move_window_to_adjacent(forward) {
            // Trace the move
            let to_frame = self.workspaces().current().layout.focused;
            self.tracer.trace_transition(&StateTransition::WindowMoved {
                window,
                from_frame: format!("{:?}", from_frame),
                to_frame: format!("{:?}", to_frame),
            });

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
            WmAction::ToggleFloat => self.toggle_float(None)?,
            WmAction::ToggleFullscreen => self.toggle_fullscreen(None)?,
            WmAction::ToggleVerticalTabs => self.toggle_vertical_tabs()?,
            WmAction::FocusUrgent => self.focus_urgent()?,
            WmAction::FocusMonitorLeft => self.focus_monitor_direction(Direction::Left)?,
            WmAction::FocusMonitorRight => self.focus_monitor_direction(Direction::Right)?,
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
                let geometries = self.workspaces().current().layout.calculate_geometries(
                    self.usable_screen(),
                    self.config.gap,
                );
                IpcResponse::Layout {
                    data: self.workspaces().current().layout.snapshot(Some(&geometries)),
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
            IpcCommand::ToggleFloat { window } => {
                match self.toggle_float(window.map(|w| w as Window)) {
                    Ok(()) => IpcResponse::Ok,
                    Err(e) => IpcResponse::Error {
                        code: "toggle_float_failed".to_string(),
                        message: e.to_string(),
                    },
                }
            }
            IpcCommand::GetFloating => {
                let floating: Vec<u32> = self.workspaces().current().floating_window_ids();
                IpcResponse::Floating { windows: floating }
            }
            IpcCommand::ToggleFullscreen { window } => {
                match self.toggle_fullscreen(window.map(|w| w as Window)) {
                    Ok(()) => IpcResponse::Ok,
                    Err(e) => IpcResponse::Error {
                        code: "toggle_fullscreen_failed".to_string(),
                        message: e.to_string(),
                    },
                }
            }
            IpcCommand::GetFullscreen => {
                let fullscreen = self.workspaces().current().fullscreen_window.map(|w| w as u32);
                IpcResponse::Fullscreen { window: fullscreen }
            }
            IpcCommand::GetUrgent => {
                let urgent: Vec<u32> = self.urgent_windows.iter().map(|&w| w as u32).collect();
                IpcResponse::Urgent { windows: urgent }
            }
            IpcCommand::FocusUrgent => {
                match self.focus_urgent() {
                    Ok(()) => IpcResponse::Ok,
                    Err(e) => IpcResponse::Error {
                        code: "focus_urgent_failed".to_string(),
                        message: e.to_string(),
                    },
                }
            }
            IpcCommand::SwitchWorkspace { index } => {
                if let Some(old_idx) = self.workspaces_mut().switch_to(index) {
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
                    index: self.workspaces().current_index(),
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
            IpcCommand::GetMonitors => {
                let monitors: Vec<_> = self.monitors.iter()
                    .map(|(id, monitor)| {
                        ipc::MonitorInfo {
                            name: monitor.name.clone(),
                            x: monitor.geometry.x,
                            y: monitor.geometry.y,
                            width: monitor.geometry.width,
                            height: monitor.geometry.height,
                            is_primary: monitor.primary,
                            is_focused: id == self.monitors.focused_id(),
                            current_workspace: monitor.workspaces.current_index(),
                        }
                    })
                    .collect();
                IpcResponse::Monitors { data: monitors }
            }
            IpcCommand::GetCurrentMonitor => {
                let monitor = self.monitors.focused();
                IpcResponse::Monitor {
                    name: monitor.name.clone(),
                    is_primary: monitor.primary,
                }
            }
            IpcCommand::FocusMonitor { target } => {
                match target.to_lowercase().as_str() {
                    "left" => {
                        match self.focus_monitor_direction(Direction::Left) {
                            Ok(()) => IpcResponse::Ok,
                            Err(e) => IpcResponse::Error {
                                code: "focus_monitor_failed".to_string(),
                                message: e.to_string(),
                            },
                        }
                    }
                    "right" => {
                        match self.focus_monitor_direction(Direction::Right) {
                            Ok(()) => IpcResponse::Ok,
                            Err(e) => IpcResponse::Error {
                                code: "focus_monitor_failed".to_string(),
                                message: e.to_string(),
                            },
                        }
                    }
                    name => {
                        // Try to find monitor by name
                        if let Some(monitor_id) = self.monitors.find_by_name(name) {
                            match self.focus_monitor(monitor_id) {
                                Ok(()) => IpcResponse::Ok,
                                Err(e) => IpcResponse::Error {
                                    code: "focus_monitor_failed".to_string(),
                                    message: e.to_string(),
                                },
                            }
                        } else {
                            IpcResponse::Error {
                                code: "monitor_not_found".to_string(),
                                message: format!("Monitor '{}' not found", name),
                            }
                        }
                    }
                }
            }
            IpcCommand::SetFrameName { name } => {
                let focused_frame = self.workspaces().current().layout.focused;

                // If setting a name (not clearing), check for uniqueness
                if let Some(ref n) = name {
                    if !n.is_empty() {
                        // Check if name is taken by another frame
                        if let Some((_, _, existing_id)) = self.find_frame_by_name_global(n) {
                            if existing_id != focused_frame {
                                return IpcResponse::Error {
                                    code: "name_taken".to_string(),
                                    message: format!("Frame name '{}' is already in use", n),
                                };
                            }
                        }
                    }
                }

                // Set the name
                if self.workspaces_mut().current_mut().layout.set_frame_name(focused_frame, name) {
                    IpcResponse::Ok
                } else {
                    IpcResponse::Error {
                        code: "set_frame_name_failed".to_string(),
                        message: "Failed to set frame name".to_string(),
                    }
                }
            }
            IpcCommand::GetFrameByName { name } => {
                if let Some((monitor_id, ws_idx, node_id)) = self.find_frame_by_name_global(&name) {
                    let monitor = self.monitors.get(monitor_id).unwrap();
                    let ws = &monitor.workspaces.workspaces[ws_idx];
                    let window_count = if let Some(frame) = ws.layout.get(node_id).and_then(|n| n.as_frame()) {
                        frame.windows.len()
                    } else {
                        0
                    };
                    let frame_name = ws.layout.get_frame_name(node_id).map(|s| s.to_string());

                    IpcResponse::Frame {
                        id: format!("{:?}", node_id),
                        name: frame_name,
                        monitor: monitor.name.clone(),
                        workspace: ws_idx + 1, // 1-indexed for user display
                        window_count,
                    }
                } else {
                    IpcResponse::Error {
                        code: "frame_not_found".to_string(),
                        message: format!("No frame found with name '{}'", name),
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
        let geometries = self.workspaces().current().layout.calculate_geometries(
            self.usable_screen(),
            self.config.gap,
        );
        let tiled_count = self.workspaces().current().layout.all_windows().len();
        let floating_count = self.workspaces().current().floating_windows.len();
        WmStateSnapshot {
            focused_window: self.focused_window,
            focused_frame: self.workspaces().current().layout.focused_frame_id(),
            window_count: tiled_count + floating_count,
            frame_count: self.workspaces().current().layout.all_frames().len(),
            layout: self.workspaces().current().layout.snapshot(Some(&geometries)),
            windows: self.get_window_info_list(),
        }
    }

    /// Get information about all managed windows
    fn get_window_info_list(&self) -> Vec<WindowInfo> {
        let mut windows = Vec::new();
        let all_frames = self.workspaces().current().layout.all_frames();

        // Add tiled windows
        for frame_id in all_frames {
            if let Some(frame) = self.workspaces().current().layout.get(frame_id).and_then(|n| n.as_frame()) {
                let is_focused_frame = frame_id == self.workspaces().current().layout.focused;
                for (tab_index, &window) in frame.windows.iter().enumerate() {
                    let is_focused_tab = tab_index == frame.focused;
                    windows.push(WindowInfo {
                        id: window,
                        title: window_query::get_window_title(&self.conn, &self.atoms, window),
                        frame: format!("{:?}", frame_id),
                        tab_index,
                        is_focused: is_focused_frame && is_focused_tab && self.focused_window == Some(window),
                        is_visible: is_focused_tab, // Only the focused tab is visible
                        is_tagged: self.tagged_windows.contains(&window),
                        is_floating: false,
                        is_urgent: self.urgent_windows.contains(&window),
                    });
                }
            }
        }

        // Add floating windows
        for fw in &self.workspaces().current().floating_windows {
            windows.push(WindowInfo {
                id: fw.window,
                title: window_query::get_window_title(&self.conn, &self.atoms, fw.window),
                frame: "floating".to_string(),
                tab_index: 0,
                is_focused: self.focused_window == Some(fw.window),
                is_visible: true, // Floating windows are always visible
                is_tagged: self.tagged_windows.contains(&fw.window),
                is_floating: true,
                is_urgent: self.urgent_windows.contains(&fw.window),
            });
        }

        windows
    }

    /// Validate WM state invariants
    fn validate_state(&self) -> Vec<String> {
        let mut violations = Vec::new();

        // Check: focused window should be in layout or floating
        if let Some(w) = self.focused_window {
            let in_layout = self.workspaces().current().layout.find_window(w).is_some();
            let is_floating = self.workspaces().current().is_floating(w);
            if !in_layout && !is_floating {
                violations.push(format!("Focused window 0x{:x} is not in layout or floating", w));
            }
        }

        // Check: focused frame should exist
        if self.workspaces().current().layout.get(self.workspaces().current().layout.focused).is_none() {
            violations.push(format!("Focused frame {:?} does not exist", self.workspaces().current().layout.focused));
        }

        // Check: all hidden windows should be in layout
        for &w in &self.hidden_windows {
            if self.workspaces().current().layout.find_window(w).is_none() {
                violations.push(format!("Hidden window 0x{:x} is not in layout", w));
            }
        }

        // Check: tab bar windows should correspond to existing frames
        let mon_id = self.monitors.focused_id();
        let ws_idx = self.workspaces().current_index();
        for &(m, idx, frame_id) in self.tab_bar_windows.keys() {
            if m == mon_id && idx == ws_idx && self.workspaces().current().layout.get(frame_id).is_none() {
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

    /// Apply startup configuration to all monitors
    fn apply_startup_config(&mut self) -> Result<()> {
        if self.user_config.startup.workspace.is_empty() {
            log::info!("No startup layout configuration found");
            return Ok(());
        }

        log::info!("Applying startup layout configuration");

        // Apply to each monitor's workspaces
        for (_monitor_id, monitor) in self.monitors.iter_mut() {
            let spawns = self.startup_manager.apply_config(
                &self.user_config.startup,
                &mut monitor.workspaces.workspaces,
            );

            // Log what we're going to spawn
            for spawn in &spawns {
                let frame_info = spawn
                    .frame_name
                    .as_ref()
                    .map(|n| format!(" in frame '{}'", n))
                    .unwrap_or_default();
                log::info!(
                    "Startup: will spawn '{}' on workspace {}{}",
                    spawn.command,
                    spawn.workspace_idx + 1,
                    frame_info
                );
            }
        }

        // Spawn all apps at once
        self.startup_manager.spawn_all();

        // Apply layout to show the configured frames
        self.apply_layout()?;

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

    // Apply startup layout configuration
    wm.apply_startup_config()?;

    // Manage any existing windows
    wm.scan_existing_windows()?;

    // Run the event loop
    wm.run()?;

    Ok(())
}
