//! Multi-monitor support using RandR.
//!
//! This module provides monitor detection and management, with per-monitor workspaces.

use std::collections::HashMap;

use anyhow::{Context, Result};
use slotmap::{new_key_type, SlotMap};
use x11rb::connection::Connection;
use x11rb::protocol::randr::{self, Output};
use x11rb::protocol::xproto::{ConnectionExt as XprotoExt, Window};
use x11rb::rust_connection::RustConnection;

use crate::layout::Direction;
use crate::types::Rect;
use crate::workspaces::WorkspaceManager;

new_key_type! {
    /// Unique identifier for a monitor
    pub struct MonitorId;
}

/// A physical monitor with its own set of workspaces
#[derive(Debug)]
pub struct Monitor {
    /// RandR output name (e.g., "DP-1", "HDMI-0")
    pub name: String,
    /// Whether this is the primary monitor
    pub primary: bool,
    /// Position and size on the root window
    pub geometry: Rect,
    /// Per-monitor workspace manager (9 workspaces)
    pub workspaces: WorkspaceManager,
    /// RandR outputs associated with this monitor
    pub outputs: Vec<Output>,
}

impl Monitor {
    /// Create a new monitor with the given properties
    pub(crate) fn new(name: String, primary: bool, geometry: Rect, outputs: Vec<Output>) -> Self {
        Self {
            name,
            primary,
            geometry,
            workspaces: WorkspaceManager::new(),
            outputs,
        }
    }
}

/// Manages all monitors and their workspaces
#[derive(Debug)]
pub struct MonitorManager {
    /// All monitors, keyed by MonitorId
    monitors: SlotMap<MonitorId, Monitor>,
    /// Currently focused monitor
    focused: MonitorId,
    /// Maps RandR output ID to MonitorId for quick lookup
    output_to_monitor: HashMap<Output, MonitorId>,
}

impl MonitorManager {
    /// Create a new empty monitor manager
    pub fn new() -> Self {
        Self {
            monitors: SlotMap::with_key(),
            focused: MonitorId::default(),
            output_to_monitor: HashMap::new(),
        }
    }

    /// Query monitors via RandR and populate the manager
    /// Returns the primary monitor ID
    pub fn refresh(&mut self, conn: &RustConnection, root: Window) -> Result<MonitorId> {
        // Clear existing monitors
        self.monitors.clear();
        self.output_to_monitor.clear();

        // Get monitors using RandR 1.5 GetMonitors (preferred)
        let monitors_reply = randr::get_monitors(conn, root, true)?
            .reply()
            .context("Failed to get monitors from RandR")?;

        log::info!(
            "RandR reports {} monitor(s)",
            monitors_reply.monitors.len()
        );

        let mut primary_id: Option<MonitorId> = None;

        for mon_info in monitors_reply.monitors {
            let name = get_atom_name(conn, mon_info.name)?;
            let geometry = Rect::new(
                mon_info.x as i32,
                mon_info.y as i32,
                mon_info.width as u32,
                mon_info.height as u32,
            );
            let is_primary = mon_info.primary;

            log::info!(
                "Monitor '{}': {}x{}+{}+{} {}",
                name,
                geometry.width,
                geometry.height,
                geometry.x,
                geometry.y,
                if is_primary { "(primary)" } else { "" }
            );

            let outputs: Vec<Output> = mon_info.outputs.clone();
            let monitor = Monitor::new(name.clone(), is_primary, geometry, outputs.clone());
            let monitor_id = self.monitors.insert(monitor);

            // Map outputs to this monitor
            for output in outputs {
                self.output_to_monitor.insert(output, monitor_id);
            }

            if is_primary {
                primary_id = Some(monitor_id);
            }
        }

        // If no monitors found, create a fallback using screen dimensions
        if self.monitors.is_empty() {
            log::warn!("No monitors detected, creating fallback from screen dimensions");
            let screen = &conn.setup().roots[0];
            let geometry = Rect::new(
                0,
                0,
                screen.width_in_pixels as u32,
                screen.height_in_pixels as u32,
            );
            let monitor = Monitor::new("default".to_string(), true, geometry, vec![]);
            primary_id = Some(self.monitors.insert(monitor));
        }

        // Set focused to primary, or first monitor if no primary
        self.focused = primary_id.unwrap_or_else(|| {
            self.monitors.keys().next().expect("At least one monitor must exist")
        });

        Ok(self.focused)
    }

    /// Get monitor by ID
    pub fn get(&self, id: MonitorId) -> Option<&Monitor> {
        self.monitors.get(id)
    }

    /// Get monitor by ID (mutable)
    pub fn get_mut(&mut self, id: MonitorId) -> Option<&mut Monitor> {
        self.monitors.get_mut(id)
    }

    /// Get the currently focused monitor
    pub fn focused(&self) -> &Monitor {
        self.monitors.get(self.focused).expect("Focused monitor must exist")
    }

    /// Get the currently focused monitor (mutable)
    pub fn focused_mut(&mut self) -> &mut Monitor {
        self.monitors.get_mut(self.focused).expect("Focused monitor must exist")
    }

    /// Get the focused monitor's ID
    pub fn focused_id(&self) -> MonitorId {
        self.focused
    }

    /// Set the focused monitor
    pub fn set_focused(&mut self, id: MonitorId) -> bool {
        if self.monitors.contains_key(id) {
            self.focused = id;
            true
        } else {
            false
        }
    }

    /// Find the monitor containing a point (for focus-follows-mouse)
    pub fn monitor_at(&self, x: i32, y: i32) -> Option<MonitorId> {
        for (id, monitor) in &self.monitors {
            let g = &monitor.geometry;
            if x >= g.x
                && x < g.x + g.width as i32
                && y >= g.y
                && y < g.y + g.height as i32
            {
                return Some(id);
            }
        }
        None
    }

    /// Find monitor in a direction relative to the focused monitor
    pub fn monitor_in_direction(&self, direction: Direction) -> Option<MonitorId> {
        let focused = self.focused();
        let focused_cx = focused.geometry.center_x();
        let focused_cy = focused.geometry.center_y();

        let mut best: Option<(MonitorId, i32)> = None;

        for (id, monitor) in &self.monitors {
            if id == self.focused {
                continue;
            }

            let cx = monitor.geometry.center_x();
            let cy = monitor.geometry.center_y();

            // Check if monitor is in the right direction
            let in_direction = match direction {
                Direction::Left => cx < focused_cx,
                Direction::Right => cx > focused_cx,
                Direction::Up => cy < focused_cy,
                Direction::Down => cy > focused_cy,
            };

            if !in_direction {
                continue;
            }

            // Calculate distance, prioritizing same-axis alignment
            let (primary_dist, secondary_dist) = match direction {
                Direction::Left | Direction::Right => {
                    ((focused_cx - cx).abs(), (focused_cy - cy).abs())
                }
                Direction::Up | Direction::Down => {
                    ((focused_cy - cy).abs(), (focused_cx - cx).abs())
                }
            };

            let distance = primary_dist + secondary_dist / 2;

            if best.is_none() || distance < best.unwrap().1 {
                best = Some((id, distance));
            }
        }

        best.map(|(id, _)| id)
    }

    /// Get all monitor IDs
    pub fn all_monitors(&self) -> Vec<MonitorId> {
        self.monitors.keys().collect()
    }

    /// Get the number of monitors
    pub fn count(&self) -> usize {
        self.monitors.len()
    }

    /// Find the primary monitor
    pub fn primary(&self) -> Option<MonitorId> {
        self.monitors
            .iter()
            .find(|(_, m)| m.primary)
            .map(|(id, _)| id)
    }

    /// Find a monitor by name
    pub fn find_by_name(&self, name: &str) -> Option<MonitorId> {
        self.monitors
            .iter()
            .find(|(_, m)| m.name == name)
            .map(|(id, _)| id)
    }

    /// Iterate over all monitors
    pub fn iter(&self) -> impl Iterator<Item = (MonitorId, &Monitor)> {
        self.monitors.iter()
    }

    /// Iterate over all monitors (mutable)
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (MonitorId, &mut Monitor)> {
        self.monitors.iter_mut()
    }

    /// Add a mock monitor for testing (bypasses RandR)
    /// Returns the MonitorId of the newly added monitor
    pub fn add_mock_monitor(&mut self, name: &str, geometry: Rect, primary: bool) -> MonitorId {
        let monitor = Monitor::new(name.to_string(), primary, geometry, vec![]);
        let id = self.monitors.insert(monitor);

        // Set as focused if it's the first monitor or if it's primary
        if self.monitors.len() == 1 || primary {
            self.focused = id;
        }

        id
    }

    /// Create a MonitorManager with mock monitors (for testing)
    /// Each config tuple is (name, geometry, is_primary)
    pub fn with_mock_monitors(configs: &[(&str, Rect, bool)]) -> Self {
        let mut manager = Self::new();
        let mut primary_id: Option<MonitorId> = None;

        for (name, geometry, is_primary) in configs {
            let monitor = Monitor::new(name.to_string(), *is_primary, geometry.clone(), vec![]);
            let id = manager.monitors.insert(monitor);
            if *is_primary {
                primary_id = Some(id);
            }
        }

        // Set focused to primary, or first monitor if no primary
        manager.focused = primary_id.unwrap_or_else(|| {
            manager.monitors.keys().next().unwrap_or_default()
        });

        manager
    }
}

impl Default for MonitorManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Get the string name of an X11 atom
fn get_atom_name(conn: &RustConnection, atom: x11rb::protocol::xproto::Atom) -> Result<String> {
    let reply = conn.get_atom_name(atom)?.reply()?;
    Ok(String::from_utf8_lossy(&reply.name).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_monitor_manager_new() {
        let manager = MonitorManager::new();
        assert_eq!(manager.count(), 0);
    }

    #[test]
    fn test_rect_center() {
        let rect = Rect::new(0, 0, 1920, 1080);
        assert_eq!(rect.center_x(), 960);
        assert_eq!(rect.center_y(), 540);
    }

    #[test]
    fn test_rect_center_with_offset() {
        let rect = Rect::new(1920, 0, 1920, 1080);
        assert_eq!(rect.center_x(), 2880);
        assert_eq!(rect.center_y(), 540);
    }

    // Mock monitor tests

    #[test]
    fn test_add_mock_monitor_single() {
        let mut manager = MonitorManager::new();
        let id = manager.add_mock_monitor("DP-1", Rect::new(0, 0, 1920, 1080), true);

        assert_eq!(manager.count(), 1);
        assert_eq!(manager.focused_id(), id);

        let monitor = manager.get(id).unwrap();
        assert_eq!(monitor.name, "DP-1");
        assert!(monitor.primary);
        assert_eq!(monitor.geometry.width, 1920);
        assert_eq!(monitor.geometry.height, 1080);
    }

    #[test]
    fn test_add_mock_monitor_multiple() {
        let mut manager = MonitorManager::new();
        let id1 = manager.add_mock_monitor("DP-1", Rect::new(0, 0, 1920, 1080), true);
        let id2 = manager.add_mock_monitor("HDMI-1", Rect::new(1920, 0, 1920, 1080), false);

        assert_eq!(manager.count(), 2);
        // Primary should be focused
        assert_eq!(manager.focused_id(), id1);

        let m1 = manager.get(id1).unwrap();
        let m2 = manager.get(id2).unwrap();
        assert_eq!(m1.name, "DP-1");
        assert_eq!(m2.name, "HDMI-1");
        assert!(m1.primary);
        assert!(!m2.primary);
    }

    #[test]
    fn test_with_mock_monitors() {
        let manager = MonitorManager::with_mock_monitors(&[
            ("DP-1", Rect::new(0, 0, 1920, 1080), true),
            ("HDMI-1", Rect::new(1920, 0, 1920, 1080), false),
        ]);

        assert_eq!(manager.count(), 2);
        assert!(manager.primary().is_some());

        let primary = manager.get(manager.primary().unwrap()).unwrap();
        assert_eq!(primary.name, "DP-1");
    }

    #[test]
    fn test_with_mock_monitors_no_primary() {
        let manager = MonitorManager::with_mock_monitors(&[
            ("DP-1", Rect::new(0, 0, 1920, 1080), false),
            ("HDMI-1", Rect::new(1920, 0, 1920, 1080), false),
        ]);

        assert_eq!(manager.count(), 2);
        // Should still have a focused monitor (first one)
        assert!(manager.monitors.contains_key(manager.focused_id()));
    }

    #[test]
    fn test_monitor_navigation_left_right() {
        let mut manager = MonitorManager::with_mock_monitors(&[
            ("DP-1", Rect::new(0, 0, 1920, 1080), true),
            ("HDMI-1", Rect::new(1920, 0, 1920, 1080), false),
        ]);

        // Start focused on DP-1 (left monitor)
        let dp1 = manager.find_by_name("DP-1").unwrap();
        let hdmi1 = manager.find_by_name("HDMI-1").unwrap();
        manager.set_focused(dp1);

        // Navigate right -> should find HDMI-1
        let right = manager.monitor_in_direction(Direction::Right);
        assert_eq!(right, Some(hdmi1));

        // Navigate left from DP-1 -> should find nothing
        let left = manager.monitor_in_direction(Direction::Left);
        assert_eq!(left, None);

        // Focus HDMI-1 and navigate left -> should find DP-1
        manager.set_focused(hdmi1);
        let left = manager.monitor_in_direction(Direction::Left);
        assert_eq!(left, Some(dp1));

        // Navigate right from HDMI-1 -> should find nothing
        let right = manager.monitor_in_direction(Direction::Right);
        assert_eq!(right, None);
    }

    #[test]
    fn test_monitor_navigation_up_down() {
        let mut manager = MonitorManager::with_mock_monitors(&[
            ("TOP", Rect::new(0, 0, 1920, 1080), true),
            ("BOTTOM", Rect::new(0, 1080, 1920, 1080), false),
        ]);

        let top = manager.find_by_name("TOP").unwrap();
        let bottom = manager.find_by_name("BOTTOM").unwrap();
        manager.set_focused(top);

        // Navigate down -> should find BOTTOM
        let down = manager.monitor_in_direction(Direction::Down);
        assert_eq!(down, Some(bottom));

        // Navigate up from TOP -> should find nothing
        let up = manager.monitor_in_direction(Direction::Up);
        assert_eq!(up, None);

        // Focus BOTTOM and navigate up -> should find TOP
        manager.set_focused(bottom);
        let up = manager.monitor_in_direction(Direction::Up);
        assert_eq!(up, Some(top));
    }

    #[test]
    fn test_monitor_at_point() {
        let manager = MonitorManager::with_mock_monitors(&[
            ("DP-1", Rect::new(0, 0, 1920, 1080), true),
            ("HDMI-1", Rect::new(1920, 0, 1920, 1080), false),
        ]);

        let dp1 = manager.find_by_name("DP-1").unwrap();
        let hdmi1 = manager.find_by_name("HDMI-1").unwrap();

        // Point in DP-1
        assert_eq!(manager.monitor_at(100, 100), Some(dp1));
        assert_eq!(manager.monitor_at(1919, 1079), Some(dp1));

        // Point in HDMI-1
        assert_eq!(manager.monitor_at(1920, 0), Some(hdmi1));
        assert_eq!(manager.monitor_at(2500, 500), Some(hdmi1));

        // Point outside all monitors
        assert_eq!(manager.monitor_at(-100, 100), None);
        assert_eq!(manager.monitor_at(5000, 100), None);
    }

    #[test]
    fn test_per_monitor_workspaces() {
        let manager = MonitorManager::with_mock_monitors(&[
            ("DP-1", Rect::new(0, 0, 1920, 1080), true),
            ("HDMI-1", Rect::new(1920, 0, 1920, 1080), false),
        ]);

        let dp1 = manager.find_by_name("DP-1").unwrap();
        let hdmi1 = manager.find_by_name("HDMI-1").unwrap();

        // Each monitor should have its own workspace manager
        let m1 = manager.get(dp1).unwrap();
        let m2 = manager.get(hdmi1).unwrap();

        // Both should start on workspace 0
        assert_eq!(m1.workspaces.current_index(), 0);
        assert_eq!(m2.workspaces.current_index(), 0);
    }

    #[test]
    fn test_focus_switching() {
        let mut manager = MonitorManager::with_mock_monitors(&[
            ("DP-1", Rect::new(0, 0, 1920, 1080), true),
            ("HDMI-1", Rect::new(1920, 0, 1920, 1080), false),
        ]);

        let dp1 = manager.find_by_name("DP-1").unwrap();
        let hdmi1 = manager.find_by_name("HDMI-1").unwrap();

        // Should start focused on primary (DP-1)
        assert_eq!(manager.focused_id(), dp1);
        assert_eq!(manager.focused().name, "DP-1");

        // Switch to HDMI-1
        assert!(manager.set_focused(hdmi1));
        assert_eq!(manager.focused_id(), hdmi1);
        assert_eq!(manager.focused().name, "HDMI-1");

        // Switch back to DP-1
        assert!(manager.set_focused(dp1));
        assert_eq!(manager.focused_id(), dp1);

        // Try to focus non-existent monitor
        let fake_id = MonitorId::default();
        assert!(!manager.set_focused(fake_id));
        // Should still be on DP-1
        assert_eq!(manager.focused_id(), dp1);
    }

    #[test]
    fn test_three_monitor_setup() {
        let manager = MonitorManager::with_mock_monitors(&[
            ("LEFT", Rect::new(0, 0, 1920, 1080), false),
            ("CENTER", Rect::new(1920, 0, 2560, 1440), true),
            ("RIGHT", Rect::new(4480, 0, 1920, 1080), false),
        ]);

        assert_eq!(manager.count(), 3);

        let left = manager.find_by_name("LEFT").unwrap();
        let center = manager.find_by_name("CENTER").unwrap();
        let right = manager.find_by_name("RIGHT").unwrap();

        // Primary (center) should be focused
        assert_eq!(manager.focused_id(), center);

        // Test navigation from center
        let mut manager = manager;
        manager.set_focused(center);

        let nav_left = manager.monitor_in_direction(Direction::Left);
        let nav_right = manager.monitor_in_direction(Direction::Right);

        assert_eq!(nav_left, Some(left));
        assert_eq!(nav_right, Some(right));
    }

    #[test]
    fn test_workspace_independence() {
        let mut manager = MonitorManager::with_mock_monitors(&[
            ("DP-1", Rect::new(0, 0, 1920, 1080), true),
            ("HDMI-1", Rect::new(1920, 0, 1920, 1080), false),
        ]);

        let dp1 = manager.find_by_name("DP-1").unwrap();
        let hdmi1 = manager.find_by_name("HDMI-1").unwrap();

        // Switch DP-1 to workspace 3
        manager.get_mut(dp1).unwrap().workspaces.switch_to(3);

        // Switch HDMI-1 to workspace 5
        manager.get_mut(hdmi1).unwrap().workspaces.switch_to(5);

        // Verify they're independent
        assert_eq!(manager.get(dp1).unwrap().workspaces.current_index(), 3);
        assert_eq!(manager.get(hdmi1).unwrap().workspaces.current_index(), 5);
    }
}
