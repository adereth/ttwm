//! ttwm - Tabbed Tiling Window Manager
//!
//! A minimal X11 tiling window manager inspired by Notion.
//! Milestone 3: Basic tiling with horizontal/vertical splits.

mod layout;

use std::process::Command;

use anyhow::{Context, Result};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use layout::{LayoutTree, Rect, SplitDirection};

/// EWMH atoms we need
struct Atoms {
    wm_protocols: Atom,
    wm_delete_window: Atom,
    net_supported: Atom,
    net_client_list: Atom,
    net_active_window: Atom,
    net_wm_name: Atom,
    net_supporting_wm_check: Atom,
    utf8_string: Atom,
}

impl Atoms {
    fn new(conn: &RustConnection) -> Result<Self> {
        Ok(Self {
            wm_protocols: Self::intern(conn, b"WM_PROTOCOLS")?,
            wm_delete_window: Self::intern(conn, b"WM_DELETE_WINDOW")?,
            net_supported: Self::intern(conn, b"_NET_SUPPORTED")?,
            net_client_list: Self::intern(conn, b"_NET_CLIENT_LIST")?,
            net_active_window: Self::intern(conn, b"_NET_ACTIVE_WINDOW")?,
            net_wm_name: Self::intern(conn, b"_NET_WM_NAME")?,
            net_supporting_wm_check: Self::intern(conn, b"_NET_SUPPORTING_WM_CHECK")?,
            utf8_string: Self::intern(conn, b"UTF8_STRING")?,
        })
    }

    fn intern(conn: &RustConnection, name: &[u8]) -> Result<Atom> {
        Ok(conn.intern_atom(false, name)?.reply()?.atom)
    }
}

/// Layout configuration
struct LayoutConfig {
    /// Gap between windows
    gap: u32,
    /// Outer gap (margin from screen edge)
    outer_gap: u32,
    /// Border width
    border_width: u32,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            gap: 8,
            outer_gap: 8,
            border_width: 2,
        }
    }
}

/// The main window manager state
struct Wm {
    conn: RustConnection,
    screen_num: usize,
    root: Window,
    atoms: Atoms,
    /// Layout tree for tiling
    layout: LayoutTree,
    /// Currently focused window (if any)
    focused_window: Option<Window>,
    /// WM check window for EWMH
    check_window: Window,
    /// Layout configuration
    config: LayoutConfig,
    /// Whether we should keep running
    running: bool,
}

impl Wm {
    /// Connect to X11 and set up the window manager
    fn new() -> Result<Self> {
        // Connect to X11 server
        let (conn, screen_num) = RustConnection::connect(None)
            .context("Failed to connect to X11 server")?;

        let screen = &conn.setup().roots[screen_num];
        let root = screen.root;

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

        Ok(Self {
            conn,
            screen_num,
            root,
            atoms,
            layout: LayoutTree::new(),
            focused_window: None,
            check_window,
            config: LayoutConfig::default(),
            running: true,
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
            | EventMask::STRUCTURE_NOTIFY;

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
            self.atoms.net_wm_name,
            self.atoms.net_supporting_wm_check,
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

        self.conn.flush()?;
        log::info!("EWMH properties set up");
        Ok(())
    }

    /// Update _NET_CLIENT_LIST with current windows
    fn update_client_list(&self) -> Result<()> {
        let windows = self.layout.all_windows();
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

    /// Apply the current layout to all windows
    fn apply_layout(&self) -> Result<()> {
        let screen_rect = self.usable_screen();
        let geometries = self.layout.calculate_geometries(screen_rect, self.config.gap);

        for (frame_id, rect) in geometries {
            if let Some(frame) = self.layout.get(frame_id).and_then(|n| n.as_frame()) {
                // Apply geometry to all windows in this frame
                // For now, all windows in a frame get the same geometry (stacked)
                for &window in &frame.windows {
                    let border = self.config.border_width;
                    self.conn.configure_window(
                        window,
                        &ConfigureWindowAux::new()
                            .x(rect.x)
                            .y(rect.y)
                            .width(rect.width.saturating_sub(border * 2))
                            .height(rect.height.saturating_sub(border * 2))
                            .border_width(border),
                    )?;
                }
            }
        }

        self.conn.flush()?;
        Ok(())
    }

    /// Grab keys we want to handle
    fn grab_keys(&self) -> Result<()> {
        // We need to grab keys on the root window
        // Mod4 = Super/Windows key (modifier mask 64)
        let mod_key = ModMask::M4;

        // Get keyboard mapping to find keycodes
        let setup = self.conn.setup();
        let min_keycode = setup.min_keycode;
        let max_keycode = setup.max_keycode;

        let mapping = self.conn.get_keyboard_mapping(min_keycode, max_keycode - min_keycode + 1)?
            .reply()?;

        let keysyms_per_keycode = mapping.keysyms_per_keycode as usize;

        // Keysym constants
        const XK_RETURN: u32 = 0xff0d;
        const XK_TAB: u32 = 0xff09;
        const XK_Q: u32 = 0x71;
        const XK_J: u32 = 0x6a;
        const XK_K: u32 = 0x6b;
        const XK_H: u32 = 0x68;
        const XK_L: u32 = 0x6c;
        const XK_S: u32 = 0x73;
        const XK_V: u32 = 0x76;

        let mut return_keycode: Option<Keycode> = None;
        let mut tab_keycode: Option<Keycode> = None;
        let mut q_keycode: Option<Keycode> = None;
        let mut j_keycode: Option<Keycode> = None;
        let mut k_keycode: Option<Keycode> = None;
        let mut h_keycode: Option<Keycode> = None;
        let mut l_keycode: Option<Keycode> = None;
        let mut s_keycode: Option<Keycode> = None;
        let mut v_keycode: Option<Keycode> = None;

        for (i, chunk) in mapping.keysyms.chunks(keysyms_per_keycode).enumerate() {
            for keysym in chunk {
                match *keysym {
                    XK_RETURN => return_keycode = Some(min_keycode + i as u8),
                    XK_TAB => tab_keycode = Some(min_keycode + i as u8),
                    XK_Q => q_keycode = Some(min_keycode + i as u8),
                    XK_J => j_keycode = Some(min_keycode + i as u8),
                    XK_K => k_keycode = Some(min_keycode + i as u8),
                    XK_H => h_keycode = Some(min_keycode + i as u8),
                    XK_L => l_keycode = Some(min_keycode + i as u8),
                    XK_S => s_keycode = Some(min_keycode + i as u8),
                    XK_V => v_keycode = Some(min_keycode + i as u8),
                    _ => {}
                }
            }
        }

        // Grab Mod4+Return for spawning terminal
        if let Some(keycode) = return_keycode {
            self.grab_key(keycode, mod_key)?;
            log::info!("Grabbed Mod4+Return (keycode {})", keycode);
        }

        // Grab Mod4+Tab for cycling windows forward
        if let Some(keycode) = tab_keycode {
            self.grab_key(keycode, mod_key)?;
            log::info!("Grabbed Mod4+Tab (keycode {})", keycode);
            // Also Mod4+Shift+Tab for cycling backward
            self.grab_key(keycode, mod_key | ModMask::SHIFT)?;
            log::info!("Grabbed Mod4+Shift+Tab (keycode {})", keycode);
        }

        // Grab Mod4+J/K for next/prev window
        if let Some(keycode) = j_keycode {
            self.grab_key(keycode, mod_key)?;
            log::info!("Grabbed Mod4+J (keycode {})", keycode);
        }
        if let Some(keycode) = k_keycode {
            self.grab_key(keycode, mod_key)?;
            log::info!("Grabbed Mod4+K (keycode {})", keycode);
        }

        // Grab Mod4+Shift+Q for quitting
        if let Some(keycode) = q_keycode {
            self.grab_key(keycode, mod_key | ModMask::SHIFT)?;
            log::info!("Grabbed Mod4+Shift+Q (keycode {})", keycode);

            // Also grab Mod4+Q for closing windows
            self.grab_key(keycode, mod_key)?;
            log::info!("Grabbed Mod4+Q (keycode {})", keycode);
        }

        // Grab Mod4+H/L for focus left/right
        if let Some(keycode) = h_keycode {
            self.grab_key(keycode, mod_key)?;
            log::info!("Grabbed Mod4+H (keycode {})", keycode);
        }
        if let Some(keycode) = l_keycode {
            self.grab_key(keycode, mod_key)?;
            log::info!("Grabbed Mod4+L (keycode {})", keycode);
        }

        // Grab Mod4+S for horizontal split, Mod4+V for vertical split
        if let Some(keycode) = s_keycode {
            self.grab_key(keycode, mod_key)?;
            log::info!("Grabbed Mod4+S (keycode {})", keycode);
        }
        if let Some(keycode) = v_keycode {
            self.grab_key(keycode, mod_key)?;
            log::info!("Grabbed Mod4+V (keycode {})", keycode);
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
        if self.layout.find_window(window).is_some() {
            return Ok(());
        }

        log::info!("Managing window 0x{:x}", window);

        // Set border color
        self.conn.change_window_attributes(
            window,
            &ChangeWindowAttributesAux::new()
                .border_pixel(0x5294e2), // Blue border
        )?;

        // Subscribe to events on this window
        self.conn.change_window_attributes(
            window,
            &ChangeWindowAttributesAux::new()
                .event_mask(EventMask::ENTER_WINDOW | EventMask::FOCUS_CHANGE),
        )?;

        // Map the window (make it visible)
        self.conn.map_window(window)?;

        // Add to the focused frame in our layout
        self.layout.add_window(window);

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
    fn unmanage_window(&mut self, window: Window) {
        if let Some(_frame_id) = self.layout.remove_window(window) {
            log::info!("Unmanaging window 0x{:x}", window);

            // Update EWMH client list
            let _ = self.update_client_list();

            // If this was focused, focus another window
            if self.focused_window == Some(window) {
                self.focused_window = None;

                // Try to focus the window in the focused frame
                if let Some(frame) = self.layout.focused_frame() {
                    if let Some(w) = frame.focused_window() {
                        let _ = self.focus_window(w);
                    }
                }

                // If still no focus, try any window
                if self.focused_window.is_none() {
                    let windows = self.layout.all_windows();
                    if let Some(&w) = windows.first() {
                        let _ = self.focus_window(w);
                    } else {
                        let _ = self.update_active_window();
                    }
                }
            }

            // Re-apply layout
            let _ = self.apply_layout();
        }
    }

    /// Cycle focus to the next/previous window
    fn cycle_focus(&mut self, forward: bool) -> Result<()> {
        let windows = self.layout.all_windows();
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

    /// Split the focused frame
    fn split_focused(&mut self, direction: SplitDirection) -> Result<()> {
        self.layout.split_focused(direction);
        self.apply_layout()?;
        log::info!("Split {:?}", direction);
        Ok(())
    }

    /// Focus the next/previous frame
    fn focus_frame(&mut self, forward: bool) -> Result<()> {
        if self.layout.focus_direction(SplitDirection::Horizontal, forward) {
            // Focus the window in the new frame
            if let Some(frame) = self.layout.focused_frame() {
                if let Some(window) = frame.focused_window() {
                    self.focus_window(window)?;
                }
            }
        }
        Ok(())
    }

    /// Focus a window
    fn focus_window(&mut self, window: Window) -> Result<()> {
        // Unfocus the previously focused window
        if let Some(old) = self.focused_window {
            if old != window && self.layout.find_window(old).is_some() {
                self.conn.change_window_attributes(
                    old,
                    &ChangeWindowAttributesAux::new()
                        .border_pixel(0x3a3a3a), // Gray border for unfocused
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
                .border_pixel(0x5294e2), // Blue border for focused
        )?;

        self.focused_window = Some(window);

        // Also update the layout's focused frame to match
        if let Some(frame_id) = self.layout.find_window(window) {
            self.layout.focused = frame_id;
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

            // Try to use WM_DELETE_WINDOW protocol first
            // For now, just destroy the window
            self.conn.kill_client(window)?;
            self.conn.flush()?;
        }
        Ok(())
    }

    /// Spawn a terminal
    fn spawn_terminal(&self) {
        log::info!("Spawning terminal");

        if let Err(e) = Command::new("alacritty").spawn() {
            log::error!("Failed to spawn alacritty: {}", e);
            // Fallback to xterm
            if let Err(e) = Command::new("xterm").spawn() {
                log::error!("Failed to spawn xterm: {}", e);
            }
        }
    }

    /// Handle an X11 event
    fn handle_event(&mut self, event: Event) -> Result<()> {
        match event {
            Event::MapRequest(e) => {
                log::debug!("MapRequest for window 0x{:x}", e.window);
                self.manage_window(e.window)?;
            }

            Event::UnmapNotify(e) => {
                log::debug!("UnmapNotify for window 0x{:x}", e.window);
                // Only unmanage if the event is about a window we manage
                // and not from a reparent operation
                if e.event == self.root {
                    self.unmanage_window(e.window);
                }
            }

            Event::DestroyNotify(e) => {
                log::debug!("DestroyNotify for window 0x{:x}", e.window);
                self.unmanage_window(e.window);
            }

            Event::ConfigureRequest(e) => {
                // For now, allow all configure requests
                log::debug!("ConfigureRequest for window 0x{:x}", e.window);

                // If we're managing this window, re-apply layout (ignore client's request)
                if self.layout.find_window(e.window).is_some() {
                    self.apply_layout()?;
                } else {
                    // Unmanaged window - allow the configure
                    let aux = ConfigureWindowAux::from_configure_request(&e);
                    self.conn.configure_window(e.window, &aux)?;
                    self.conn.flush()?;
                }
            }

            Event::EnterNotify(e) => {
                // Focus follows mouse
                if self.layout.find_window(e.event).is_some() {
                    log::debug!("EnterNotify for window 0x{:x}", e.event);
                    self.focus_window(e.event)?;
                }
            }

            Event::KeyPress(e) => {
                self.handle_key_press(e)?;
            }

            _ => {
                // Ignore other events for now
            }
        }

        Ok(())
    }

    /// Handle a key press event
    fn handle_key_press(&mut self, event: KeyPressEvent) -> Result<()> {
        let mod_key = u16::from(ModMask::M4);
        let shift = u16::from(ModMask::SHIFT);

        // Convert state to u16 and mask out NumLock and CapsLock for comparison
        let state_u16 = u16::from(event.state);
        let clean_state = state_u16 & !(u16::from(ModMask::M2) | u16::from(ModMask::LOCK));

        // Get the keysym for this keycode
        let setup = self.conn.setup();
        let min_keycode = setup.min_keycode;
        let max_keycode = setup.max_keycode;

        let mapping = self.conn.get_keyboard_mapping(min_keycode, max_keycode - min_keycode + 1)?
            .reply()?;

        let keysyms_per_keycode = mapping.keysyms_per_keycode as usize;
        let idx = (event.detail - min_keycode) as usize * keysyms_per_keycode;
        let keysym = mapping.keysyms.get(idx).copied().unwrap_or(0);

        log::debug!(
            "KeyPress: keycode={}, keysym=0x{:x}, state=0x{:x}, clean_state=0x{:x}",
            event.detail, keysym, state_u16, clean_state
        );

        // Keysym constants
        const XK_RETURN: u32 = 0xff0d;
        const XK_TAB: u32 = 0xff09;
        const XK_Q: u32 = 0x71;
        const XK_J: u32 = 0x6a;
        const XK_K: u32 = 0x6b;
        const XK_H: u32 = 0x68;
        const XK_L: u32 = 0x6c;
        const XK_S: u32 = 0x73;
        const XK_V: u32 = 0x76;

        match (keysym, clean_state) {
            // Mod4+Return -> spawn terminal
            (XK_RETURN, s) if s == mod_key => {
                self.spawn_terminal();
            }

            // Mod4+Tab -> cycle windows forward
            (XK_TAB, s) if s == mod_key => {
                self.cycle_focus(true)?;
            }

            // Mod4+Shift+Tab -> cycle windows backward
            (XK_TAB, s) if s == mod_key | shift => {
                self.cycle_focus(false)?;
            }

            // Mod4+J -> next window
            (XK_J, s) if s == mod_key => {
                self.cycle_focus(true)?;
            }

            // Mod4+K -> previous window
            (XK_K, s) if s == mod_key => {
                self.cycle_focus(false)?;
            }

            // Mod4+H -> focus left frame
            (XK_H, s) if s == mod_key => {
                self.focus_frame(false)?;
            }

            // Mod4+L -> focus right frame
            (XK_L, s) if s == mod_key => {
                self.focus_frame(true)?;
            }

            // Mod4+S -> split horizontal
            (XK_S, s) if s == mod_key => {
                self.split_focused(SplitDirection::Horizontal)?;
            }

            // Mod4+V -> split vertical
            (XK_V, s) if s == mod_key => {
                self.split_focused(SplitDirection::Vertical)?;
            }

            // Mod4+Q -> close focused window
            (XK_Q, s) if s == mod_key => {
                self.close_focused_window()?;
            }

            // Mod4+Shift+Q -> quit WM
            (XK_Q, s) if s == mod_key | shift => {
                log::info!("Quitting window manager");
                self.running = false;
            }

            _ => {}
        }

        Ok(())
    }

    /// Main event loop
    fn run(&mut self) -> Result<()> {
        log::info!("Entering event loop");

        while self.running {
            // Wait for and process events
            let event = self.conn.wait_for_event()
                .context("Error waiting for X11 event")?;

            if let Err(e) = self.handle_event(event) {
                log::error!("Error handling event: {}", e);
            }
        }

        log::info!("Exiting window manager");
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
