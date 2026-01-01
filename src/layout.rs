//! Layout tree for tiling window management.
//!
//! The layout is represented as a binary tree where:
//! - Leaf nodes are Frames (contain windows)
//! - Internal nodes are Splits (horizontal or vertical)

use serde::{Deserialize, Serialize};
use slotmap::{new_key_type, SlotMap};
use x11rb::protocol::xproto::Window;

// Generate unique key types for our arena
new_key_type! {
    /// Unique identifier for a node in the layout tree
    pub struct NodeId;
}

/// Direction of a split
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SplitDirection {
    /// Children arranged left-to-right
    Horizontal,
    /// Children arranged top-to-bottom
    Vertical,
}

/// Direction for spatial navigation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

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

    #[allow(dead_code)]
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

    #[allow(dead_code)]
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

    /// Find the closest frame in the given direction from the focused frame
    pub fn find_frame_in_direction(
        &self,
        direction: Direction,
        geometries: &[(NodeId, Rect)],
    ) -> Option<NodeId> {
        // Get focused frame's geometry
        let focused_rect = geometries.iter()
            .find(|(id, _)| *id == self.focused)
            .map(|(_, rect)| rect)?;

        let focused_cx = focused_rect.center_x();
        let focused_cy = focused_rect.center_y();

        // Filter frames in the given direction and find the closest one
        let mut best: Option<(NodeId, i32)> = None;

        for (frame_id, rect) in geometries {
            if *frame_id == self.focused {
                continue;
            }

            let cx = rect.center_x();
            let cy = rect.center_y();

            // Check if frame is in the right direction
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
            // For left/right: prefer frames at similar Y positions
            // For up/down: prefer frames at similar X positions
            let (primary_dist, secondary_dist) = match direction {
                Direction::Left | Direction::Right => {
                    ((focused_cx - cx).abs(), (focused_cy - cy).abs())
                }
                Direction::Up | Direction::Down => {
                    ((focused_cy - cy).abs(), (focused_cx - cx).abs())
                }
            };

            // Weighted distance: secondary distance matters less
            let distance = primary_dist + secondary_dist / 2;

            if best.is_none() || distance < best.unwrap().1 {
                best = Some((*frame_id, distance));
            }
        }

        best.map(|(id, _)| id)
    }

    /// Focus the frame in the given spatial direction
    pub fn focus_spatial(&mut self, direction: Direction, geometries: &[(NodeId, Rect)]) -> bool {
        if let Some(target) = self.find_frame_in_direction(direction, geometries) {
            self.focused = target;
            return true;
        }
        false
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

    /// Resize the split containing the focused frame
    /// delta > 0 grows the focused frame, delta < 0 shrinks it
    pub fn resize_focused_split(&mut self, delta: f32) -> bool {
        let parent_id = match self.parent(self.focused) {
            Some(id) => id,
            None => return false, // No parent split to resize
        };

        if let Some(Node::Split(split)) = self.nodes.get_mut(parent_id) {
            // Determine if focused is the first or second child
            let is_first = split.first == self.focused;

            // Adjust ratio (first child's share)
            let adjustment = if is_first { delta } else { -delta };
            split.ratio = (split.ratio + adjustment).clamp(0.1, 0.9);
            true
        } else {
            false
        }
    }

    /// Set the ratio of a specific split node directly
    /// Returns true if the split was found and updated
    pub fn set_split_ratio(&mut self, split_id: NodeId, ratio: f32) -> bool {
        if let Some(Node::Split(split)) = self.nodes.get_mut(split_id) {
            split.ratio = ratio.clamp(0.1, 0.9);
            true
        } else {
            false
        }
    }

    /// Find a split whose gap contains the given mouse coordinates
    /// Returns (split_id, direction, gap_start_position, total_size_in_split_direction)
    pub fn find_split_at_gap(
        &self,
        screen: Rect,
        gap: u32,
        mouse_x: i32,
        mouse_y: i32,
    ) -> Option<(NodeId, SplitDirection, i32, u32)> {
        self.find_gap_recursive(self.root, screen, gap, mouse_x, mouse_y)
    }

    fn find_gap_recursive(
        &self,
        node_id: NodeId,
        available: Rect,
        gap: u32,
        mouse_x: i32,
        mouse_y: i32,
    ) -> Option<(NodeId, SplitDirection, i32, u32)> {
        match self.get(node_id) {
            Some(Node::Frame(_)) => None, // Frames don't have gaps
            Some(Node::Split(split)) => {
                let (first_rect, second_rect) = Self::split_rect(
                    available,
                    split.direction,
                    split.ratio,
                    gap,
                );

                // Calculate the gap region
                let (gap_start, gap_end, perpendicular_start, perpendicular_end) = match split.direction {
                    SplitDirection::Horizontal => {
                        // Gap is between first_rect.x + first_rect.width and second_rect.x
                        let gap_x_start = first_rect.x + first_rect.width as i32;
                        let gap_x_end = second_rect.x;
                        (gap_x_start, gap_x_end, available.y, available.y + available.height as i32)
                    }
                    SplitDirection::Vertical => {
                        // Gap is between first_rect.y + first_rect.height and second_rect.y
                        let gap_y_start = first_rect.y + first_rect.height as i32;
                        let gap_y_end = second_rect.y;
                        (gap_y_start, gap_y_end, available.x, available.x + available.width as i32)
                    }
                };

                // Check if mouse is in this gap
                let (mouse_parallel, mouse_perpendicular) = match split.direction {
                    SplitDirection::Horizontal => (mouse_x, mouse_y),
                    SplitDirection::Vertical => (mouse_y, mouse_x),
                };

                if mouse_parallel >= gap_start
                    && mouse_parallel < gap_end
                    && mouse_perpendicular >= perpendicular_start
                    && mouse_perpendicular < perpendicular_end
                {
                    // Mouse is in this gap
                    let (split_start, total_size) = match split.direction {
                        SplitDirection::Horizontal => (available.x, available.width),
                        SplitDirection::Vertical => (available.y, available.height),
                    };
                    return Some((node_id, split.direction, split_start, total_size));
                }

                // Check children recursively
                if let Some(result) = self.find_gap_recursive(split.first, first_rect, gap, mouse_x, mouse_y) {
                    return Some(result);
                }
                if let Some(result) = self.find_gap_recursive(split.second, second_rect, gap, mouse_x, mouse_y) {
                    return Some(result);
                }

                None
            }
            None => None,
        }
    }

    /// Remove empty frames from the tree
    /// Returns true if any cleanup was performed
    pub fn remove_empty_frames(&mut self) -> bool {
        let mut changed = false;

        loop {
            // Find an empty frame that isn't the only node
            let empty_frame = self.nodes.iter()
                .find(|(id, node)| {
                    if let Node::Frame(frame) = node {
                        frame.is_empty() && *id != self.root
                    } else {
                        false
                    }
                })
                .map(|(id, _)| id);

            let frame_id = match empty_frame {
                Some(id) => id,
                None => break, // No more empty frames to remove
            };

            // Get the parent split
            let parent_id = match self.parent(frame_id) {
                Some(id) => id,
                None => break, // This shouldn't happen for non-root frames
            };

            // Get the sibling
            let sibling_id = if let Some(Node::Split(split)) = self.nodes.get(parent_id) {
                if split.first == frame_id {
                    split.second
                } else {
                    split.first
                }
            } else {
                break;
            };

            // Get grandparent
            let grandparent_id = self.parent(parent_id);

            // Replace parent split with sibling
            if let Some(gp_id) = grandparent_id {
                // Update grandparent's child reference
                if let Some(Node::Split(gp_split)) = self.nodes.get_mut(gp_id) {
                    if gp_split.first == parent_id {
                        gp_split.first = sibling_id;
                    } else {
                        gp_split.second = sibling_id;
                    }
                }
                // Update sibling's parent
                if let Some(p) = self.parents.get_mut(sibling_id) {
                    *p = Some(gp_id);
                }
            } else {
                // Parent was root, sibling becomes new root
                self.root = sibling_id;
                if let Some(p) = self.parents.get_mut(sibling_id) {
                    *p = None;
                }
            }

            // Remove the empty frame and the parent split
            self.nodes.remove(frame_id);
            self.parents.remove(frame_id);
            self.nodes.remove(parent_id);
            self.parents.remove(parent_id);

            // Update focused if needed
            if self.focused == frame_id {
                // Focus the first frame we can find
                self.focused = self.all_frames().first().copied().unwrap_or(self.root);
            }

            changed = true;
        }

        changed
    }

    /// Cycle to the next/previous tab in the focused frame
    /// Returns the newly focused window (if any)
    pub fn cycle_tab(&mut self, forward: bool) -> Option<Window> {
        let frame = self.focused_frame_mut()?;
        if frame.windows.is_empty() {
            return None;
        }

        let len = frame.windows.len();
        frame.focused = if forward {
            (frame.focused + 1) % len
        } else {
            if frame.focused == 0 { len - 1 } else { frame.focused - 1 }
        };

        frame.focused_window()
    }

    /// Focus a specific tab by index (0-based) in the focused frame
    /// Returns the newly focused window (if any)
    pub fn focus_tab(&mut self, index: usize) -> Option<Window> {
        let frame = self.focused_frame_mut()?;
        if index < frame.windows.len() {
            frame.focused = index;
            frame.focused_window()
        } else {
            None
        }
    }

    /// Reorder a tab within a frame (move from_index to to_index)
    pub fn reorder_tab(&mut self, frame_id: NodeId, from_index: usize, to_index: usize) -> bool {
        if let Some(Node::Frame(frame)) = self.nodes.get_mut(frame_id) {
            if from_index >= frame.windows.len() || to_index >= frame.windows.len() {
                return false;
            }
            if from_index == to_index {
                return false;
            }

            let window = frame.windows.remove(from_index);
            frame.windows.insert(to_index, window);

            // Adjust focused index if needed
            if frame.focused == from_index {
                frame.focused = to_index;
            } else if from_index < frame.focused && to_index >= frame.focused {
                frame.focused -= 1;
            } else if from_index > frame.focused && to_index <= frame.focused {
                frame.focused += 1;
            }

            true
        } else {
            false
        }
    }

    /// Move a window from source frame to target frame
    pub fn move_window_to_frame(
        &mut self,
        window: Window,
        source_frame: NodeId,
        target_frame: NodeId,
    ) -> bool {
        // Remove from source
        if let Some(Node::Frame(frame)) = self.nodes.get_mut(source_frame) {
            if !frame.remove_window(window) {
                return false;
            }
        } else {
            return false;
        }

        // Add to target
        if let Some(Node::Frame(frame)) = self.nodes.get_mut(target_frame) {
            frame.add_window(window);
        } else {
            return false;
        }

        // Update focused frame
        self.focused = target_frame;
        true
    }

    /// Get number of tabs in the focused frame
    #[allow(dead_code)]
    pub fn tab_count(&self) -> usize {
        self.focused_frame().map(|f| f.windows.len()).unwrap_or(0)
    }

    /// Move the focused window to an adjacent frame
    /// Returns the window that was moved (if any)
    pub fn move_window_to_adjacent(&mut self, forward: bool) -> Option<Window> {
        let frames = self.all_frames();
        if frames.len() <= 1 {
            return None;
        }

        // Get the focused window
        let window = self.focused_frame()?.focused_window()?;

        // Find current frame index
        let current_idx = frames.iter().position(|&f| f == self.focused)?;

        // Find adjacent frame
        let adjacent_idx = if forward {
            (current_idx + 1) % frames.len()
        } else {
            if current_idx == 0 { frames.len() - 1 } else { current_idx - 1 }
        };

        let adjacent_frame_id = frames[adjacent_idx];

        // Remove window from current frame
        if let Some(Node::Frame(frame)) = self.nodes.get_mut(self.focused) {
            frame.remove_window(window);
        }

        // Add window to adjacent frame
        if let Some(Node::Frame(frame)) = self.nodes.get_mut(adjacent_frame_id) {
            frame.add_window(window);
        }

        // Focus the adjacent frame
        self.focused = adjacent_frame_id;

        Some(window)
    }

    /// Create a snapshot of the layout tree for IPC serialization
    /// The geometries parameter should be pre-calculated if you want geometry info
    pub fn snapshot(&self, geometries: Option<&[(NodeId, Rect)]>) -> crate::ipc::LayoutSnapshot {
        use crate::ipc::{LayoutSnapshot, NodeSnapshot, RectSnapshot};

        fn snapshot_node(
            tree: &LayoutTree,
            node_id: NodeId,
            geometries: Option<&[(NodeId, Rect)]>,
        ) -> NodeSnapshot {
            match tree.get(node_id) {
                Some(Node::Frame(frame)) => {
                    let geometry = geometries.and_then(|g| {
                        g.iter()
                            .find(|(id, _)| *id == node_id)
                            .map(|(_, r)| RectSnapshot::from(*r))
                    });
                    NodeSnapshot::Frame {
                        id: format!("{:?}", node_id),
                        windows: frame.windows.clone(),
                        focused_tab: frame.focused,
                        geometry,
                    }
                }
                Some(Node::Split(split)) => {
                    let direction = match split.direction {
                        SplitDirection::Horizontal => "horizontal",
                        SplitDirection::Vertical => "vertical",
                    };
                    NodeSnapshot::Split {
                        id: format!("{:?}", node_id),
                        direction: direction.to_string(),
                        ratio: split.ratio,
                        first: Box::new(snapshot_node(tree, split.first, geometries)),
                        second: Box::new(snapshot_node(tree, split.second, geometries)),
                    }
                }
                None => NodeSnapshot::Frame {
                    id: "invalid".to_string(),
                    windows: vec![],
                    focused_tab: 0,
                    geometry: None,
                },
            }
        }

        LayoutSnapshot {
            root: snapshot_node(self, self.root, geometries),
        }
    }

    /// Get the focused frame's NodeId as a string (for IPC)
    pub fn focused_frame_id(&self) -> String {
        format!("{:?}", self.focused)
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    // ==================== Tree Creation Tests ====================

    #[test]
    fn test_new_tree_has_single_frame() {
        let tree = LayoutTree::new();

        // Should have exactly one frame
        let frames = tree.all_frames();
        assert_eq!(frames.len(), 1);

        // Root should be the focused frame
        assert_eq!(tree.root, tree.focused);

        // Frame should be empty
        let frame = tree.focused_frame().unwrap();
        assert!(frame.is_empty());
    }

    #[test]
    fn test_root_has_no_parent() {
        let tree = LayoutTree::new();
        assert!(tree.parent(tree.root).is_none());
    }

    // ==================== Split Tests ====================

    #[test]
    fn test_split_horizontal_creates_two_frames() {
        let mut tree = LayoutTree::new();
        let original_root = tree.root;

        tree.split_focused(SplitDirection::Horizontal);

        // Should now have 2 frames
        let frames = tree.all_frames();
        assert_eq!(frames.len(), 2);

        // Root should now be a split, not the original frame
        assert_ne!(tree.root, original_root);
        assert!(tree.get(tree.root).unwrap().as_split().is_some());
    }

    #[test]
    fn test_split_vertical_creates_two_frames() {
        let mut tree = LayoutTree::new();

        tree.split_focused(SplitDirection::Vertical);

        let frames = tree.all_frames();
        assert_eq!(frames.len(), 2);

        // Check the split direction
        let split = tree.get(tree.root).unwrap().as_split().unwrap();
        assert_eq!(split.direction, SplitDirection::Vertical);
    }

    #[test]
    fn test_split_focuses_new_frame() {
        let mut tree = LayoutTree::new();
        let original_focused = tree.focused;

        let new_frame = tree.split_focused(SplitDirection::Horizontal);

        // Focus should move to the new frame
        assert_eq!(tree.focused, new_frame);
        assert_ne!(tree.focused, original_focused);
    }

    #[test]
    fn test_nested_splits() {
        let mut tree = LayoutTree::new();

        // Split horizontally, then vertically
        tree.split_focused(SplitDirection::Horizontal);
        tree.split_focused(SplitDirection::Vertical);

        // Should have 3 frames now
        let frames = tree.all_frames();
        assert_eq!(frames.len(), 3);
    }

    #[test]
    fn test_split_ratio_is_half() {
        let mut tree = LayoutTree::new();
        tree.split_focused(SplitDirection::Horizontal);

        let split = tree.get(tree.root).unwrap().as_split().unwrap();
        assert_eq!(split.ratio, 0.5);
    }

    // ==================== Geometry Tests ====================

    #[test]
    fn test_single_frame_fills_screen() {
        let tree = LayoutTree::new();
        let screen = Rect::new(0, 0, 1920, 1080);

        let geometries = tree.calculate_geometries(screen, 0);

        assert_eq!(geometries.len(), 1);
        let (_, rect) = &geometries[0];
        assert_eq!(rect.x, 0);
        assert_eq!(rect.y, 0);
        assert_eq!(rect.width, 1920);
        assert_eq!(rect.height, 1080);
    }

    #[test]
    fn test_horizontal_split_divides_width() {
        let mut tree = LayoutTree::new();
        tree.split_focused(SplitDirection::Horizontal);

        let screen = Rect::new(0, 0, 1000, 500);
        let geometries = tree.calculate_geometries(screen, 0);

        assert_eq!(geometries.len(), 2);

        // Both should have same height
        assert_eq!(geometries[0].1.height, 500);
        assert_eq!(geometries[1].1.height, 500);

        // Widths should roughly add up (allowing for gap calculations)
        let total_width = geometries[0].1.width + geometries[1].1.width;
        assert!(total_width <= 1000);
    }

    #[test]
    fn test_vertical_split_divides_height() {
        let mut tree = LayoutTree::new();
        tree.split_focused(SplitDirection::Vertical);

        let screen = Rect::new(0, 0, 1000, 500);
        let geometries = tree.calculate_geometries(screen, 0);

        assert_eq!(geometries.len(), 2);

        // Both should have same width
        assert_eq!(geometries[0].1.width, 1000);
        assert_eq!(geometries[1].1.width, 1000);

        // Heights should roughly add up
        let total_height = geometries[0].1.height + geometries[1].1.height;
        assert!(total_height <= 500);
    }

    #[test]
    fn test_gaps_reduce_available_space() {
        let mut tree = LayoutTree::new();
        tree.split_focused(SplitDirection::Horizontal);

        let screen = Rect::new(0, 0, 1000, 500);
        let gap = 20;
        let geometries = tree.calculate_geometries(screen, gap);

        // With a gap, total width should be less
        let total_width = geometries[0].1.width + geometries[1].1.width;
        assert!(total_width < 1000);

        // Second rect should start after first + gap
        let expected_x = geometries[0].1.x + geometries[0].1.width as i32 + gap as i32;
        assert_eq!(geometries[1].1.x, expected_x);
    }

    // ==================== Spatial Navigation Tests ====================

    #[test]
    fn test_spatial_focus_left() {
        let mut tree = LayoutTree::new();
        tree.split_focused(SplitDirection::Horizontal);
        // Now focused on right frame

        let screen = Rect::new(0, 0, 1000, 500);
        let geometries = tree.calculate_geometries(screen, 0);
        let right_frame = tree.focused;

        // Focus left should move to left frame
        assert!(tree.focus_spatial(Direction::Left, &geometries));
        assert_ne!(tree.focused, right_frame);
    }

    #[test]
    fn test_spatial_focus_right() {
        let mut tree = LayoutTree::new();
        tree.split_focused(SplitDirection::Horizontal);

        let screen = Rect::new(0, 0, 1000, 500);
        let geometries = tree.calculate_geometries(screen, 0);

        // Focus left first
        tree.focus_spatial(Direction::Left, &geometries);
        let left_frame = tree.focused;

        // Focus right should move to right frame
        assert!(tree.focus_spatial(Direction::Right, &geometries));
        assert_ne!(tree.focused, left_frame);
    }

    #[test]
    fn test_spatial_focus_up() {
        let mut tree = LayoutTree::new();
        tree.split_focused(SplitDirection::Vertical);
        // Now focused on bottom frame

        let screen = Rect::new(0, 0, 1000, 500);
        let geometries = tree.calculate_geometries(screen, 0);
        let bottom_frame = tree.focused;

        // Focus up should move to top frame
        assert!(tree.focus_spatial(Direction::Up, &geometries));
        assert_ne!(tree.focused, bottom_frame);
    }

    #[test]
    fn test_spatial_focus_down() {
        let mut tree = LayoutTree::new();
        tree.split_focused(SplitDirection::Vertical);

        let screen = Rect::new(0, 0, 1000, 500);
        let geometries = tree.calculate_geometries(screen, 0);

        // Focus up first
        tree.focus_spatial(Direction::Up, &geometries);
        let top_frame = tree.focused;

        // Focus down should move to bottom frame
        assert!(tree.focus_spatial(Direction::Down, &geometries));
        assert_ne!(tree.focused, top_frame);
    }

    #[test]
    fn test_spatial_focus_no_frame_in_direction() {
        let mut tree = LayoutTree::new();
        tree.split_focused(SplitDirection::Horizontal);

        let screen = Rect::new(0, 0, 1000, 500);
        let geometries = tree.calculate_geometries(screen, 0);

        // Focus left (to leftmost frame)
        tree.focus_spatial(Direction::Left, &geometries);

        // Focus left again should fail (no frame to the left)
        assert!(!tree.focus_spatial(Direction::Left, &geometries));
    }

    #[test]
    fn test_spatial_focus_single_frame() {
        let tree = LayoutTree::new();
        let screen = Rect::new(0, 0, 1000, 500);
        let geometries = tree.calculate_geometries(screen, 0);

        // With single frame, can't focus in any direction
        assert!(tree.find_frame_in_direction(Direction::Left, &geometries).is_none());
        assert!(tree.find_frame_in_direction(Direction::Right, &geometries).is_none());
        assert!(tree.find_frame_in_direction(Direction::Up, &geometries).is_none());
        assert!(tree.find_frame_in_direction(Direction::Down, &geometries).is_none());
    }

    // ==================== Frame/Window Tests ====================

    #[test]
    fn test_add_window_to_frame() {
        let mut tree = LayoutTree::new();

        tree.add_window(1001);

        let frame = tree.focused_frame().unwrap();
        assert_eq!(frame.windows.len(), 1);
        assert_eq!(frame.windows[0], 1001);
    }

    #[test]
    fn test_add_multiple_windows() {
        let mut tree = LayoutTree::new();

        tree.add_window(1001);
        tree.add_window(1002);
        tree.add_window(1003);

        let frame = tree.focused_frame().unwrap();
        assert_eq!(frame.windows.len(), 3);
    }

    #[test]
    fn test_add_window_focuses_it() {
        let mut tree = LayoutTree::new();

        tree.add_window(1001);
        tree.add_window(1002);

        let frame = tree.focused_frame().unwrap();
        assert_eq!(frame.focused, 1); // Second window (index 1)
        assert_eq!(frame.focused_window(), Some(1002));
    }

    #[test]
    fn test_remove_window() {
        let mut tree = LayoutTree::new();

        tree.add_window(1001);
        tree.add_window(1002);

        let removed = tree.remove_window(1001);
        assert!(removed.is_some());

        let frame = tree.focused_frame().unwrap();
        assert_eq!(frame.windows.len(), 1);
        assert_eq!(frame.windows[0], 1002);
    }

    #[test]
    fn test_remove_nonexistent_window() {
        let mut tree = LayoutTree::new();
        tree.add_window(1001);

        let removed = tree.remove_window(9999);
        assert!(removed.is_none());
    }

    #[test]
    fn test_find_window() {
        let mut tree = LayoutTree::new();
        tree.add_window(1001);

        let found = tree.find_window(1001);
        assert!(found.is_some());
        assert_eq!(found.unwrap(), tree.focused);
    }

    #[test]
    fn test_find_window_not_found() {
        let tree = LayoutTree::new();
        assert!(tree.find_window(9999).is_none());
    }

    // ==================== Tab Cycling Tests ====================

    #[test]
    fn test_cycle_tab_forward() {
        let mut tree = LayoutTree::new();
        tree.add_window(1001);
        tree.add_window(1002);
        tree.add_window(1003);

        // Currently focused on 1003 (last added)
        let next = tree.cycle_tab(true);
        assert_eq!(next, Some(1001)); // Wraps around

        let next = tree.cycle_tab(true);
        assert_eq!(next, Some(1002));
    }

    #[test]
    fn test_cycle_tab_backward() {
        let mut tree = LayoutTree::new();
        tree.add_window(1001);
        tree.add_window(1002);
        tree.add_window(1003);

        // Currently focused on 1003 (index 2)
        let prev = tree.cycle_tab(false);
        assert_eq!(prev, Some(1002));

        let prev = tree.cycle_tab(false);
        assert_eq!(prev, Some(1001));

        let prev = tree.cycle_tab(false);
        assert_eq!(prev, Some(1003)); // Wraps around
    }

    #[test]
    fn test_cycle_tab_empty_frame() {
        let mut tree = LayoutTree::new();

        let result = tree.cycle_tab(true);
        assert!(result.is_none());
    }

    #[test]
    fn test_cycle_tab_single_window() {
        let mut tree = LayoutTree::new();
        tree.add_window(1001);

        let result = tree.cycle_tab(true);
        assert_eq!(result, Some(1001)); // Stays on same window
    }

    #[test]
    fn test_focus_tab_by_index() {
        let mut tree = LayoutTree::new();
        tree.add_window(1001);
        tree.add_window(1002);
        tree.add_window(1003);

        let result = tree.focus_tab(0);
        assert_eq!(result, Some(1001));

        let result = tree.focus_tab(2);
        assert_eq!(result, Some(1003));
    }

    #[test]
    fn test_focus_tab_out_of_bounds() {
        let mut tree = LayoutTree::new();
        tree.add_window(1001);

        let result = tree.focus_tab(5);
        assert!(result.is_none());
    }

    // ==================== Empty Frame Cleanup Tests ====================

    #[test]
    fn test_remove_empty_frames_single_frame() {
        let mut tree = LayoutTree::new();

        // Single empty frame at root should NOT be removed
        let changed = tree.remove_empty_frames();
        assert!(!changed);
        assert_eq!(tree.all_frames().len(), 1);
    }

    #[test]
    fn test_remove_empty_frames_after_split() {
        let mut tree = LayoutTree::new();
        tree.add_window(1001);
        tree.split_focused(SplitDirection::Horizontal);

        // Now we have: [frame with window] | [empty frame (focused)]
        // The empty frame should be removed
        let changed = tree.remove_empty_frames();
        assert!(changed);

        // Should be back to 1 frame
        assert_eq!(tree.all_frames().len(), 1);
    }

    #[test]
    fn test_remove_empty_frames_preserves_windows() {
        let mut tree = LayoutTree::new();
        tree.add_window(1001);
        tree.split_focused(SplitDirection::Horizontal);
        tree.add_window(1002);

        // Remove 1002, making the second frame empty
        tree.remove_window(1002);
        tree.remove_empty_frames();

        // Window 1001 should still exist
        assert!(tree.find_window(1001).is_some());
        assert_eq!(tree.all_windows().len(), 1);
    }

    // ==================== Move Window Tests ====================

    #[test]
    fn test_move_window_to_adjacent_forward() {
        let mut tree = LayoutTree::new();
        tree.add_window(1001);
        tree.split_focused(SplitDirection::Horizontal);

        // Focus back to first frame using spatial navigation
        let screen = Rect::new(0, 0, 1000, 500);
        let geometries = tree.calculate_geometries(screen, 0);
        tree.focus_spatial(Direction::Left, &geometries);

        // Move window forward
        let moved = tree.move_window_to_adjacent(true);
        assert_eq!(moved, Some(1001));

        // Window should now be in the second frame
        assert_eq!(tree.focused_frame().unwrap().windows.len(), 1);
    }

    #[test]
    fn test_move_window_single_frame() {
        let mut tree = LayoutTree::new();
        tree.add_window(1001);

        // Can't move with only one frame
        let moved = tree.move_window_to_adjacent(true);
        assert!(moved.is_none());
    }

    // ==================== Resize Tests ====================

    #[test]
    fn test_resize_focused_split() {
        let mut tree = LayoutTree::new();
        tree.split_focused(SplitDirection::Horizontal);

        // Focus first frame using spatial navigation
        let screen = Rect::new(0, 0, 1000, 500);
        let geometries = tree.calculate_geometries(screen, 0);
        tree.focus_spatial(Direction::Left, &geometries);

        let original_ratio = tree.get(tree.root).unwrap().as_split().unwrap().ratio;

        tree.resize_focused_split(0.1);

        let new_ratio = tree.get(tree.root).unwrap().as_split().unwrap().ratio;
        assert!(new_ratio > original_ratio);
    }

    #[test]
    fn test_resize_clamps_ratio() {
        let mut tree = LayoutTree::new();
        tree.split_focused(SplitDirection::Horizontal);

        // Try to resize way past the limit
        for _ in 0..20 {
            tree.resize_focused_split(0.1);
        }

        let ratio = tree.get(tree.root).unwrap().as_split().unwrap().ratio;
        assert!(ratio <= 0.9);
        assert!(ratio >= 0.1);
    }

    #[test]
    fn test_resize_no_parent() {
        let mut tree = LayoutTree::new();

        // Single frame has no parent split
        let resized = tree.resize_focused_split(0.1);
        assert!(!resized);
    }

    // ==================== Frame Operations Tests ====================

    #[test]
    fn test_frame_remove_adjusts_focus() {
        let mut frame = Frame::new();
        frame.add_window(1001);
        frame.add_window(1002);
        frame.add_window(1003);

        // Focus is on 1003 (index 2)
        assert_eq!(frame.focused, 2);

        // Remove focused window
        frame.remove_window(1003);

        // Focus should move to last remaining
        assert_eq!(frame.focused, 1);
        assert_eq!(frame.focused_window(), Some(1002));
    }

    #[test]
    fn test_frame_remove_middle() {
        let mut frame = Frame::new();
        frame.add_window(1001);
        frame.add_window(1002);
        frame.add_window(1003);

        frame.focused = 0; // Focus first
        frame.remove_window(1002); // Remove middle

        // Focus should stay at 0
        assert_eq!(frame.focused, 0);
        assert_eq!(frame.focused_window(), Some(1001));
    }

    // ==================== All Windows Tests ====================

    #[test]
    fn test_all_windows_multiple_frames() {
        let mut tree = LayoutTree::new();
        tree.add_window(1001);
        tree.split_focused(SplitDirection::Horizontal);
        tree.add_window(1002);
        tree.split_focused(SplitDirection::Vertical);
        tree.add_window(1003);

        let all = tree.all_windows();
        assert_eq!(all.len(), 3);
        assert!(all.contains(&1001));
        assert!(all.contains(&1002));
        assert!(all.contains(&1003));
    }

    // ==================== Tab Reorder Tests ====================

    #[test]
    fn test_reorder_tab_forward() {
        let mut tree = LayoutTree::new();
        tree.add_window(1001);
        tree.add_window(1002);
        tree.add_window(1003);

        let frame_id = tree.focused;

        // Move tab 0 to position 2
        assert!(tree.reorder_tab(frame_id, 0, 2));

        let frame = tree.focused_frame().unwrap();
        assert_eq!(frame.windows, vec![1002, 1003, 1001]);
    }

    #[test]
    fn test_reorder_tab_backward() {
        let mut tree = LayoutTree::new();
        tree.add_window(1001);
        tree.add_window(1002);
        tree.add_window(1003);

        let frame_id = tree.focused;

        // Move tab 2 to position 0
        assert!(tree.reorder_tab(frame_id, 2, 0));

        let frame = tree.focused_frame().unwrap();
        assert_eq!(frame.windows, vec![1003, 1001, 1002]);
    }

    #[test]
    fn test_reorder_tab_same_position() {
        let mut tree = LayoutTree::new();
        tree.add_window(1001);
        tree.add_window(1002);

        let frame_id = tree.focused;

        // Reorder to same position should return false
        assert!(!tree.reorder_tab(frame_id, 0, 0));
    }

    #[test]
    fn test_reorder_tab_out_of_bounds() {
        let mut tree = LayoutTree::new();
        tree.add_window(1001);
        tree.add_window(1002);

        let frame_id = tree.focused;

        // Out of bounds should return false
        assert!(!tree.reorder_tab(frame_id, 0, 5));
        assert!(!tree.reorder_tab(frame_id, 5, 0));
    }

    #[test]
    fn test_reorder_tab_updates_focus() {
        let mut tree = LayoutTree::new();
        tree.add_window(1001);
        tree.add_window(1002);
        tree.add_window(1003);

        let frame_id = tree.focused;

        // Focus is on 1003 (index 2), move it to position 0
        assert!(tree.reorder_tab(frame_id, 2, 0));

        let frame = tree.focused_frame().unwrap();
        // Focus should follow the moved window
        assert_eq!(frame.focused, 0);
        assert_eq!(frame.focused_window(), Some(1003));
    }

    // ==================== Move Window to Frame Tests ====================

    #[test]
    fn test_move_window_to_frame() {
        let mut tree = LayoutTree::new();
        tree.add_window(1001);
        tree.add_window(1002);

        let source_frame = tree.focused;
        tree.split_focused(SplitDirection::Horizontal);
        let target_frame = tree.focused;

        // Move window 1001 from source to target
        assert!(tree.move_window_to_frame(1001, source_frame, target_frame));

        // Source frame should have only 1002
        if let Some(Node::Frame(frame)) = tree.get(source_frame) {
            assert_eq!(frame.windows, vec![1002]);
        }

        // Target frame should have 1001
        if let Some(Node::Frame(frame)) = tree.get(target_frame) {
            assert!(frame.windows.contains(&1001));
        }
    }

    #[test]
    fn test_move_window_to_frame_updates_focus() {
        let mut tree = LayoutTree::new();
        tree.add_window(1001);

        let source_frame = tree.focused;
        tree.split_focused(SplitDirection::Horizontal);
        let target_frame = tree.focused;

        // Focus should change to target frame after move
        tree.focused = source_frame;
        assert!(tree.move_window_to_frame(1001, source_frame, target_frame));
        assert_eq!(tree.focused, target_frame);
    }

    #[test]
    fn test_move_window_nonexistent() {
        let mut tree = LayoutTree::new();
        tree.add_window(1001);

        let source_frame = tree.focused;
        tree.split_focused(SplitDirection::Horizontal);
        let target_frame = tree.focused;

        // Moving nonexistent window should fail
        assert!(!tree.move_window_to_frame(9999, source_frame, target_frame));
    }
}
