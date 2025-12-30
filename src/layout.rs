//! Layout tree for tiling window management.
//!
//! The layout is represented as a binary tree where:
//! - Leaf nodes are Frames (contain windows)
//! - Internal nodes are Splits (horizontal or vertical)

use slotmap::{new_key_type, SlotMap};
use x11rb::protocol::xproto::Window;

// Generate unique key types for our arena
new_key_type! {
    /// Unique identifier for a node in the layout tree
    pub struct NodeId;
}

/// Direction of a split
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDirection {
    /// Children arranged left-to-right
    Horizontal,
    /// Children arranged top-to-bottom
    Vertical,
}

/// A rectangle representing geometry
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

/// A frame is a leaf node that contains windows
#[derive(Debug, Clone)]
pub struct Frame {
    /// Windows in this frame (will be tabs in later milestones)
    pub windows: Vec<Window>,
    /// Currently focused window index
    pub focused: usize,
}

impl Frame {
    pub fn new() -> Self {
        Self {
            windows: Vec::new(),
            focused: 0,
        }
    }

    pub fn with_window(window: Window) -> Self {
        Self {
            windows: vec![window],
            focused: 0,
        }
    }

    pub fn focused_window(&self) -> Option<Window> {
        self.windows.get(self.focused).copied()
    }

    pub fn add_window(&mut self, window: Window) {
        self.windows.push(window);
        self.focused = self.windows.len() - 1;
    }

    pub fn remove_window(&mut self, window: Window) -> bool {
        if let Some(idx) = self.windows.iter().position(|&w| w == window) {
            self.windows.remove(idx);
            if self.focused >= self.windows.len() && !self.windows.is_empty() {
                self.focused = self.windows.len() - 1;
            }
            true
        } else {
            false
        }
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }
}

/// A split node divides space between two children
#[derive(Debug, Clone)]
pub struct Split {
    pub direction: SplitDirection,
    /// First child (left or top)
    pub first: NodeId,
    /// Second child (right or bottom)
    pub second: NodeId,
    /// Ratio of space given to first child (0.0 to 1.0)
    pub ratio: f32,
}

/// A node in the layout tree
#[derive(Debug, Clone)]
pub enum Node {
    Frame(Frame),
    Split(Split),
}

impl Node {
    pub fn as_frame(&self) -> Option<&Frame> {
        match self {
            Node::Frame(f) => Some(f),
            _ => None,
        }
    }

    pub fn as_frame_mut(&mut self) -> Option<&mut Frame> {
        match self {
            Node::Frame(f) => Some(f),
            _ => None,
        }
    }

    pub fn as_split(&self) -> Option<&Split> {
        match self {
            Node::Split(s) => Some(s),
            _ => None,
        }
    }
}

/// The layout tree manages the tiling structure
#[derive(Debug)]
pub struct LayoutTree {
    /// Arena storage for all nodes
    nodes: SlotMap<NodeId, Node>,
    /// Parent pointers for navigation
    parents: SlotMap<NodeId, Option<NodeId>>,
    /// Root node of the tree
    pub root: NodeId,
    /// Currently focused frame
    pub focused: NodeId,
}

impl LayoutTree {
    /// Create a new layout tree with a single empty frame
    pub fn new() -> Self {
        let mut nodes = SlotMap::with_key();
        let mut parents = SlotMap::with_key();

        let root = nodes.insert(Node::Frame(Frame::new()));
        parents.insert(None);

        Self {
            nodes,
            parents,
            root,
            focused: root,
        }
    }

    /// Get a node by ID
    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    /// Get a mutable node by ID
    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id)
    }

    /// Get parent of a node
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.parents.get(id).copied().flatten()
    }

    /// Get the focused frame
    pub fn focused_frame(&self) -> Option<&Frame> {
        self.get(self.focused).and_then(|n| n.as_frame())
    }

    /// Get the focused frame mutably
    pub fn focused_frame_mut(&mut self) -> Option<&mut Frame> {
        self.get_mut(self.focused).and_then(|n| n.as_frame_mut())
    }

    /// Add a window to the focused frame
    pub fn add_window(&mut self, window: Window) {
        if let Some(frame) = self.focused_frame_mut() {
            frame.add_window(window);
        }
    }

    /// Remove a window from any frame that contains it
    pub fn remove_window(&mut self, window: Window) -> Option<NodeId> {
        // Find and remove from whichever frame contains it
        let mut found_frame = None;
        for (id, node) in &mut self.nodes {
            if let Node::Frame(frame) = node {
                if frame.remove_window(window) {
                    found_frame = Some(id);
                    break;
                }
            }
        }
        found_frame
    }

    /// Find which frame contains a window
    pub fn find_window(&self, window: Window) -> Option<NodeId> {
        for (id, node) in &self.nodes {
            if let Node::Frame(frame) = node {
                if frame.windows.contains(&window) {
                    return Some(id);
                }
            }
        }
        None
    }

    /// Split the focused frame
    pub fn split_focused(&mut self, direction: SplitDirection) -> NodeId {
        let old_focused = self.focused;

        // Create new empty frame
        let new_frame_id = self.nodes.insert(Node::Frame(Frame::new()));
        self.parents.insert(None); // Will be set below

        // Create split node
        let split = Split {
            direction,
            first: old_focused,
            second: new_frame_id,
            ratio: 0.5,
        };
        let split_id = self.nodes.insert(Node::Split(split));
        self.parents.insert(self.parent(old_focused));

        // Update parent pointers
        if let Some(parent_id) = self.parent(old_focused) {
            // Update parent's child reference
            if let Some(Node::Split(parent_split)) = self.nodes.get_mut(parent_id) {
                if parent_split.first == old_focused {
                    parent_split.first = split_id;
                } else {
                    parent_split.second = split_id;
                }
            }
        } else {
            // old_focused was root
            self.root = split_id;
        }

        // Set parent of old focused and new frame to the split
        if let Some(p) = self.parents.get_mut(old_focused) {
            *p = Some(split_id);
        }
        if let Some(p) = self.parents.get_mut(new_frame_id) {
            *p = Some(split_id);
        }

        // Focus the new frame
        self.focused = new_frame_id;

        new_frame_id
    }

    /// Get all frame IDs in the tree (in-order traversal)
    pub fn all_frames(&self) -> Vec<NodeId> {
        let mut frames = Vec::new();
        self.collect_frames(self.root, &mut frames);
        frames
    }

    fn collect_frames(&self, node_id: NodeId, frames: &mut Vec<NodeId>) {
        match self.get(node_id) {
            Some(Node::Frame(_)) => frames.push(node_id),
            Some(Node::Split(split)) => {
                self.collect_frames(split.first, frames);
                self.collect_frames(split.second, frames);
            }
            None => {}
        }
    }

    /// Focus the next frame in the given direction
    pub fn focus_direction(&mut self, direction: SplitDirection, forward: bool) -> bool {
        let frames = self.all_frames();
        if frames.len() <= 1 {
            return false;
        }

        let current_idx = frames.iter().position(|&f| f == self.focused).unwrap_or(0);

        // For now, simple linear navigation
        // TODO: Implement proper spatial navigation
        let next_idx = if forward {
            (current_idx + 1) % frames.len()
        } else {
            if current_idx == 0 { frames.len() - 1 } else { current_idx - 1 }
        };

        self.focused = frames[next_idx];
        true
    }

    /// Calculate geometries for all frames
    pub fn calculate_geometries(&self, screen: Rect, gap: u32) -> Vec<(NodeId, Rect)> {
        let mut result = Vec::new();
        self.calc_node_geometry(self.root, screen, gap, &mut result);
        result
    }

    fn calc_node_geometry(
        &self,
        node_id: NodeId,
        available: Rect,
        gap: u32,
        result: &mut Vec<(NodeId, Rect)>,
    ) {
        match self.get(node_id) {
            Some(Node::Frame(_)) => {
                result.push((node_id, available));
            }
            Some(Node::Split(split)) => {
                let (first_rect, second_rect) = Self::split_rect(
                    available,
                    split.direction,
                    split.ratio,
                    gap,
                );
                self.calc_node_geometry(split.first, first_rect, gap, result);
                self.calc_node_geometry(split.second, second_rect, gap, result);
            }
            None => {}
        }
    }

    fn split_rect(rect: Rect, direction: SplitDirection, ratio: f32, gap: u32) -> (Rect, Rect) {
        let half_gap = (gap / 2) as i32;

        match direction {
            SplitDirection::Horizontal => {
                let first_width = ((rect.width as f32 * ratio) as u32).saturating_sub(gap / 2);
                let second_width = rect.width.saturating_sub(first_width + gap);

                let first = Rect {
                    x: rect.x,
                    y: rect.y,
                    width: first_width,
                    height: rect.height,
                };
                let second = Rect {
                    x: rect.x + first_width as i32 + gap as i32,
                    y: rect.y,
                    width: second_width,
                    height: rect.height,
                };
                (first, second)
            }
            SplitDirection::Vertical => {
                let first_height = ((rect.height as f32 * ratio) as u32).saturating_sub(gap / 2);
                let second_height = rect.height.saturating_sub(first_height + gap);

                let first = Rect {
                    x: rect.x,
                    y: rect.y,
                    width: rect.width,
                    height: first_height,
                };
                let second = Rect {
                    x: rect.x,
                    y: rect.y + first_height as i32 + gap as i32,
                    width: rect.width,
                    height: second_height,
                };
                (first, second)
            }
        }
    }

    /// Get all windows in all frames
    pub fn all_windows(&self) -> Vec<Window> {
        let mut windows = Vec::new();
        for (_id, node) in &self.nodes {
            if let Node::Frame(frame) = node {
                windows.extend(&frame.windows);
            }
        }
        windows
    }
}
