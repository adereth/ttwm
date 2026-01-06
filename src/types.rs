//! Shared types used across multiple modules.
//!
//! This module contains common data structures to avoid circular dependencies
//! between layout and ipc modules.

use serde::{Deserialize, Serialize};

/// A rectangle representing geometry
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self { x, y, width, height }
    }

    /// Center X coordinate
    pub fn center_x(&self) -> i32 {
        self.x + (self.width as i32) / 2
    }

    /// Center Y coordinate
    pub fn center_y(&self) -> i32 {
        self.y + (self.height as i32) / 2
    }
}

/// Serializable rectangle for IPC snapshots
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RectSnapshot {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl From<Rect> for RectSnapshot {
    fn from(r: Rect) -> Self {
        Self {
            x: r.x,
            y: r.y,
            width: r.width,
            height: r.height,
        }
    }
}

/// EWMH strut partial - space reserved at screen edges by docks/panels
#[derive(Debug, Clone, Copy, Default)]
pub struct StrutPartial {
    pub left: u32,
    pub right: u32,
    pub top: u32,
    pub bottom: u32,
    // Extended fields for multi-monitor (start/end coords)
    pub left_start_y: u32,
    pub left_end_y: u32,
    pub right_start_y: u32,
    pub right_end_y: u32,
    pub top_start_x: u32,
    pub top_end_x: u32,
    pub bottom_start_x: u32,
    pub bottom_end_x: u32,
}

/// Snapshot of the layout tree for IPC serialization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutSnapshot {
    pub root: NodeSnapshot,
}

/// Snapshot of a single node in the layout tree
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NodeSnapshot {
    Frame {
        id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        windows: Vec<u32>,
        focused_tab: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        geometry: Option<RectSnapshot>,
    },
    Split {
        id: String,
        direction: String,
        ratio: f32,
        first: Box<NodeSnapshot>,
        second: Box<NodeSnapshot>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rect_center() {
        let rect = Rect::new(0, 0, 100, 100);
        assert_eq!(rect.center_x(), 50);
        assert_eq!(rect.center_y(), 50);

        let rect = Rect::new(10, 20, 100, 200);
        assert_eq!(rect.center_x(), 60);
        assert_eq!(rect.center_y(), 120);
    }

    #[test]
    fn test_rect_snapshot_from_rect() {
        let rect = Rect::new(10, 20, 100, 200);
        let snapshot: RectSnapshot = rect.into();
        assert_eq!(snapshot.x, 10);
        assert_eq!(snapshot.y, 20);
        assert_eq!(snapshot.width, 100);
        assert_eq!(snapshot.height, 200);
    }
}
