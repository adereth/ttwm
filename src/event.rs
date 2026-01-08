//! Event handling for X11 events.
//!
//! Contains all event dispatch and handling logic for the window manager,
//! separated from main.rs for maintainability.

use anyhow::Result;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::protocol::Event;

use crate::layout::{NodeId, SplitDirection};
use crate::window_query;
use crate::Wm;

/// Edge or corner of a floating window for resizing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeEdge {
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
pub enum DragState {
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

impl Wm {
    /// Handle a client message event (EWMH requests)
    pub fn handle_client_message(&mut self, event: ClientMessageEvent) -> Result<()> {
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

    /// Handle an X11 event
    pub fn handle_event(&mut self, event: Event) -> Result<()> {
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
                    self.tab_bars.invalidate_icon(e.window);
                    // Redraw tab bars that might show this window
                    self.redraw_tabs_for_window(e.window)?;
                }
                // Redraw tab bar if title changed
                if e.atom == self.atoms.net_wm_name || e.atom == u32::from(AtomEnum::WM_NAME) {
                    self.redraw_tabs_for_window(e.window)?;
                }
                // Handle urgent state changes (EWMH _NET_WM_STATE or legacy WM_HINTS)
                if e.atom == self.atoms.net_wm_state || e.atom == u32::from(AtomEnum::WM_HINTS) {
                    let was_urgent = self.urgent.contains(e.window);
                    let is_urgent = window_query::is_window_urgent(&self.conn, &self.atoms, e.window);
                    if is_urgent && !was_urgent {
                        self.urgent.add(e.window); // Add to end (newest)
                        log::info!("Window 0x{:x} is now urgent", e.window);
                        self.redraw_tabs_for_window(e.window)?;
                        self.update_urgent_indicator()?;
                    } else if !is_urgent && was_urgent {
                        self.urgent.remove(e.window);
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
        for (&(m, idx, frame_id), &tab_window) in &self.tab_bars.windows {
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
                    if let Some(tab_window) = self.tab_bars.windows.remove(&(mon_id, ws_idx, frame_id)) {
                        if let Some(pixmap) = self.tab_bars.pixmaps.remove(&tab_window) {
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
        let clicked_empty_frame = self.tab_bars.empty_frame_windows.iter()
            .find(|(&(m, idx, _), &empty_window)| m == mon_id && idx == ws_idx && empty_window == event.event)
            .map(|(&(_, _, frame_id), _)| frame_id);

        if let Some(frame_id) = clicked_empty_frame {
            if event.detail == 2 {
                // Middle-click: remove empty frame
                if let Some(empty_window) = self.tab_bars.empty_frame_windows.remove(&(mon_id, ws_idx, frame_id)) {
                    self.conn.destroy_window(empty_window)?;
                }
                if let Some(tab_window) = self.tab_bars.windows.remove(&(mon_id, ws_idx, frame_id)) {
                    if let Some(pixmap) = self.tab_bars.pixmaps.remove(&tab_window) {
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
        let clicked_frame = self.tab_bars.windows.iter()
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
        for (&(m, idx, frame_id), &tab_window) in &self.tab_bars.windows {
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
}
