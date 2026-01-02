//! Workspace (virtual desktop) management.
//!
//! This module provides workspace management for ttwm, allowing users
//! to organize windows across multiple virtual desktops.

use x11rb::protocol::xproto::Window;

use crate::layout::LayoutTree;

/// Number of workspaces (virtual desktops)
pub const NUM_WORKSPACES: usize = 9;

/// A floating window with its geometry
#[derive(Debug, Clone, Copy)]
pub struct FloatingWindow {
    pub window: Window,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// A workspace (virtual desktop) containing an independent layout tree
#[derive(Debug)]
pub struct Workspace {
    /// Unique identifier (1-9)
    pub id: usize,
    /// The layout tree for this workspace
    pub layout: LayoutTree,
    /// The last focused window in this workspace (for focus restoration)
    pub last_focused_window: Option<Window>,
    /// Floating windows in this workspace
    pub floating_windows: Vec<FloatingWindow>,
}

impl Workspace {
    /// Create a new workspace with the given id
    pub fn new(id: usize) -> Self {
        let layout = LayoutTree::new();
        Self {
            id,
            layout,
            last_focused_window: None,
            floating_windows: Vec::new(),
        }
    }

    /// Add a floating window to this workspace
    pub fn add_floating(&mut self, window: Window, x: i32, y: i32, width: u32, height: u32) {
        self.floating_windows.push(FloatingWindow {
            window,
            x,
            y,
            width,
            height,
        });
    }

    /// Remove a floating window from this workspace, returning its geometry if found
    pub fn remove_floating(&mut self, window: Window) -> Option<FloatingWindow> {
        if let Some(pos) = self.floating_windows.iter().position(|f| f.window == window) {
            Some(self.floating_windows.remove(pos))
        } else {
            None
        }
    }

    /// Find a floating window by its X11 window ID
    pub fn find_floating(&self, window: Window) -> Option<&FloatingWindow> {
        self.floating_windows.iter().find(|f| f.window == window)
    }

    /// Find a floating window by its X11 window ID (mutable)
    pub fn find_floating_mut(&mut self, window: Window) -> Option<&mut FloatingWindow> {
        self.floating_windows.iter_mut().find(|f| f.window == window)
    }

    /// Check if a window is floating in this workspace
    pub fn is_floating(&self, window: Window) -> bool {
        self.floating_windows.iter().any(|f| f.window == window)
    }

    /// Get all floating window IDs
    pub fn floating_window_ids(&self) -> Vec<Window> {
        self.floating_windows.iter().map(|f| f.window).collect()
    }
}

/// Manages multiple workspaces (virtual desktops)
#[derive(Debug)]
pub struct WorkspaceManager {
    /// All workspaces (fixed array of 9)
    pub workspaces: [Workspace; NUM_WORKSPACES],
    /// Index of the current workspace (0-8)
    current: usize,
}

impl WorkspaceManager {
    /// Create a new workspace manager with 9 workspaces
    pub fn new() -> Self {
        Self {
            workspaces: std::array::from_fn(|i| Workspace::new(i + 1)),
            current: 0,
        }
    }

    /// Get a reference to the current workspace
    pub fn current(&self) -> &Workspace {
        &self.workspaces[self.current]
    }

    /// Get a mutable reference to the current workspace
    pub fn current_mut(&mut self) -> &mut Workspace {
        &mut self.workspaces[self.current]
    }

    /// Get the index of the current workspace (0-based)
    pub fn current_index(&self) -> usize {
        self.current
    }

    /// Switch to a specific workspace (0-indexed)
    /// Returns the old workspace index if switch was successful
    pub fn switch_to(&mut self, target: usize) -> Option<usize> {
        if target >= NUM_WORKSPACES || target == self.current {
            return None;
        }
        let old = self.current;
        self.current = target;
        Some(old)
    }

    /// Cycle to the next workspace (wrapping around)
    /// Returns the old workspace index
    pub fn next(&mut self) -> usize {
        let old = self.current;
        self.current = (self.current + 1) % NUM_WORKSPACES;
        old
    }

    /// Cycle to the previous workspace (wrapping around)
    /// Returns the old workspace index
    pub fn prev(&mut self) -> usize {
        let old = self.current;
        self.current = if self.current == 0 {
            NUM_WORKSPACES - 1
        } else {
            self.current - 1
        };
        old
    }
}

impl Default for WorkspaceManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workspace_new_has_empty_floating() {
        let ws = Workspace::new(1);
        assert_eq!(ws.id, 1);
        assert!(ws.floating_windows.is_empty());
    }

    #[test]
    fn test_add_floating_window() {
        let mut ws = Workspace::new(1);
        ws.add_floating(100, 10, 20, 300, 200);

        assert_eq!(ws.floating_windows.len(), 1);
        assert_eq!(ws.floating_windows[0].window, 100);
        assert_eq!(ws.floating_windows[0].x, 10);
        assert_eq!(ws.floating_windows[0].y, 20);
        assert_eq!(ws.floating_windows[0].width, 300);
        assert_eq!(ws.floating_windows[0].height, 200);
    }

    #[test]
    fn test_add_multiple_floating_windows() {
        let mut ws = Workspace::new(1);
        ws.add_floating(100, 0, 0, 100, 100);
        ws.add_floating(200, 50, 50, 150, 150);
        ws.add_floating(300, 100, 100, 200, 200);

        assert_eq!(ws.floating_windows.len(), 3);
        assert_eq!(ws.floating_window_ids(), vec![100, 200, 300]);
    }

    #[test]
    fn test_remove_floating_window() {
        let mut ws = Workspace::new(1);
        ws.add_floating(100, 10, 20, 300, 200);
        ws.add_floating(200, 50, 60, 400, 300);

        let removed = ws.remove_floating(100);
        assert!(removed.is_some());
        let fw = removed.unwrap();
        assert_eq!(fw.window, 100);
        assert_eq!(fw.x, 10);
        assert_eq!(fw.y, 20);

        assert_eq!(ws.floating_windows.len(), 1);
        assert_eq!(ws.floating_windows[0].window, 200);
    }

    #[test]
    fn test_remove_nonexistent_floating_window() {
        let mut ws = Workspace::new(1);
        ws.add_floating(100, 0, 0, 100, 100);

        let removed = ws.remove_floating(999);
        assert!(removed.is_none());
        assert_eq!(ws.floating_windows.len(), 1);
    }

    #[test]
    fn test_find_floating_window() {
        let mut ws = Workspace::new(1);
        ws.add_floating(100, 10, 20, 300, 200);
        ws.add_floating(200, 50, 60, 400, 300);

        let found = ws.find_floating(200);
        assert!(found.is_some());
        let fw = found.unwrap();
        assert_eq!(fw.window, 200);
        assert_eq!(fw.x, 50);
        assert_eq!(fw.width, 400);
    }

    #[test]
    fn test_find_floating_window_not_found() {
        let ws = Workspace::new(1);
        assert!(ws.find_floating(999).is_none());
    }

    #[test]
    fn test_find_floating_mut() {
        let mut ws = Workspace::new(1);
        ws.add_floating(100, 10, 20, 300, 200);

        if let Some(fw) = ws.find_floating_mut(100) {
            fw.x = 100;
            fw.y = 200;
            fw.width = 500;
            fw.height = 400;
        }

        let fw = ws.find_floating(100).unwrap();
        assert_eq!(fw.x, 100);
        assert_eq!(fw.y, 200);
        assert_eq!(fw.width, 500);
        assert_eq!(fw.height, 400);
    }

    #[test]
    fn test_is_floating() {
        let mut ws = Workspace::new(1);
        ws.add_floating(100, 0, 0, 100, 100);

        assert!(ws.is_floating(100));
        assert!(!ws.is_floating(200));
    }

    #[test]
    fn test_floating_window_ids() {
        let mut ws = Workspace::new(1);
        assert!(ws.floating_window_ids().is_empty());

        ws.add_floating(100, 0, 0, 100, 100);
        ws.add_floating(200, 0, 0, 100, 100);
        ws.add_floating(300, 0, 0, 100, 100);

        let ids = ws.floating_window_ids();
        assert_eq!(ids.len(), 3);
        assert!(ids.contains(&100));
        assert!(ids.contains(&200));
        assert!(ids.contains(&300));
    }

    #[test]
    fn test_floating_windows_per_workspace() {
        let mut manager = WorkspaceManager::new();

        // Add floating window to workspace 0
        manager.current_mut().add_floating(100, 0, 0, 100, 100);
        assert_eq!(manager.current().floating_windows.len(), 1);

        // Switch to workspace 1
        manager.switch_to(1);
        assert!(manager.current().floating_windows.is_empty());

        // Add floating window to workspace 1
        manager.current_mut().add_floating(200, 0, 0, 100, 100);
        assert_eq!(manager.current().floating_windows.len(), 1);

        // Switch back to workspace 0 - should still have its floating window
        manager.switch_to(0);
        assert_eq!(manager.current().floating_windows.len(), 1);
        assert_eq!(manager.current().floating_windows[0].window, 100);
    }

    #[test]
    fn test_remove_middle_floating_window() {
        let mut ws = Workspace::new(1);
        ws.add_floating(100, 0, 0, 100, 100);
        ws.add_floating(200, 0, 0, 100, 100);
        ws.add_floating(300, 0, 0, 100, 100);

        ws.remove_floating(200);

        let ids = ws.floating_window_ids();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&100));
        assert!(ids.contains(&300));
        assert!(!ids.contains(&200));
    }
}
