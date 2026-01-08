//! IPC command handling for the window manager.
//!
//! Contains the handler for all IPC commands from ttwmctl and other clients.

use x11rb::protocol::xproto::Window;

use crate::ipc::{self, IpcCommand, IpcResponse, WmStateSnapshot, WindowInfo};
use crate::layout::{Direction, SplitDirection};
use crate::window_query;
use crate::Wm;

impl Wm {
    /// Handle an IPC command and return a response
    pub fn handle_ipc(&mut self, cmd: IpcCommand) -> IpcResponse {
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
                let urgent: Vec<u32> = self.urgent.windows().iter().map(|&w| w as u32).collect();
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
                        is_urgent: self.urgent.contains(window),
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
                is_urgent: self.urgent.contains(fw.window),
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
        for &(m, idx, frame_id) in self.tab_bars.windows.keys() {
            if m == mon_id && idx == ws_idx && self.workspaces().current().layout.get(frame_id).is_none() {
                violations.push(format!("Tab bar for non-existent frame {:?}", frame_id));
            }
        }

        violations
    }
}
