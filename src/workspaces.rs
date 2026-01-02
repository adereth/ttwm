//! Workspace (virtual desktop) management.
//!
//! This module provides workspace management for ttwm, allowing users
//! to organize windows across multiple virtual desktops.

use x11rb::protocol::xproto::Window;

use crate::layout::LayoutTree;

/// Number of workspaces (virtual desktops)
pub const NUM_WORKSPACES: usize = 9;

/// A workspace (virtual desktop) containing an independent layout tree
#[derive(Debug)]
pub struct Workspace {
    /// Unique identifier (1-9)
    pub id: usize,
    /// The layout tree for this workspace
    pub layout: LayoutTree,
    /// The last focused window in this workspace (for focus restoration)
    pub last_focused_window: Option<Window>,
}

impl Workspace {
    /// Create a new workspace with the given id
    pub fn new(id: usize) -> Self {
        let layout = LayoutTree::new();
        Self {
            id,
            layout,
            last_focused_window: None,
        }
    }
}

/// Manages multiple workspaces (virtual desktops)
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
