//! State machine types and validation for ttwm.
//!
//! This module formalizes the state machines in the window manager:
//! - Window lifecycle states
//! - Focus states
//! - Frame states
//!
//! The state validator checks invariants that should always hold.

use serde::{Deserialize, Serialize};

/// Window lifecycle states
///
/// ```text
///                MapRequest
///                    │
///                    ▼
///               ┌─────────┐
///               │ Pending │
///               └────┬────┘
///                    │ manage_window()
///                    ▼
///               ┌─────────┐ ◄──────────────┐
///               │ Mapped  │                │
///               └────┬────┘                │
///                    │                     │
///          ┌─────────┴─────────┐           │
///          │ tab switch        │           │ tab switch back
///          ▼                   ▼           │
///     ┌─────────┐         ┌─────────┐ ─────┘
///     │ Hidden  │◄───────►│ Visible │
///     └────┬────┘         └────┬────┘
///          │                   │
///          └─────────┬─────────┘
///                    │ unmanage/destroy
///                    ▼
///              ┌───────────┐
///              │ Destroyed │
///              └───────────┘
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowState {
    /// Window has requested mapping but is not yet managed
    Pending,
    /// Window is managed and visible (focused tab in its frame)
    Visible,
    /// Window is managed but hidden (background tab)
    Hidden,
    /// Window is being destroyed/unmanaged
    Destroying,
}

/// Focus state machine
///
/// ```text
///     ┌──────────────────────────────────────┐
///     │                                      │
///     ▼                                      │
/// ┌─────────┐    focus_window(w)    ┌────────┴───────┐
/// │  None   │ ─────────────────────►│ Focused { w }  │
/// └─────────┘                       └────────┬───────┘
///     ▲                                      │
///     │ last window closes                   │ focus_window(other)
///     │                                      ▼
///     └──────────────────────────────────────┘
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FocusState {
    /// No window is focused
    None,
    /// A specific window is focused
    Focused { window: u32 },
}

/// Frame state machine
///
/// ```text
///     ┌─────────────────────────────────────────────┐
///     │                                             │
///     ▼                                             │
/// ┌─────────┐   add_window()   ┌──────────────┐     │
/// │  Empty  │ ────────────────►│ SingleWindow │     │
/// └────┬────┘                  └───────┬──────┘     │
///      │                               │            │
///      │ remove_empty_frames()         │ add_window()
///      ▼                               ▼            │
///   (removed)                   ┌─────────────┐     │
///                               │   Tabbed    │ ────┘
///                               │ (2+ windows)│ remove_window()
///                               └─────────────┘     to 0 windows
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameState {
    /// Frame has no windows (pending cleanup unless root)
    Empty,
    /// Frame has exactly one window (no tab bar shown)
    SingleWindow { window: u32 },
    /// Frame has multiple windows (tab bar shown)
    Tabbed {
        windows: Vec<u32>,
        active_tab: usize,
    },
}

/// State violations that can be detected
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StateViolation {
    pub kind: ViolationKind,
    pub description: String,
}

/// Types of state violations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViolationKind {
    /// Window exists in X but not tracked in layout
    OrphanedWindow,
    /// Window tracked in layout but doesn't exist in X
    GhostWindow,
    /// Focus is on a window that doesn't exist
    InvalidFocus,
    /// Empty frame that should have been cleaned up
    EmptyFrameLeaked,
    /// Split ratio outside valid bounds
    SplitRatioOutOfBounds,
    /// Tab index points to non-existent tab
    TabIndexOutOfBounds,
    /// Focused frame doesn't exist
    FocusedFrameMissing,
    /// Hidden window not tracked in layout
    HiddenWindowOrphaned,
    /// Tab bar exists for non-existent frame
    OrphanedTabBar,
}

/// State transition events that can be traced
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "transition", rename_all = "snake_case")]
pub enum StateTransition {
    /// Window was added to management
    WindowManaged {
        window: u32,
        frame: String,
    },
    /// Window was removed from management
    WindowUnmanaged {
        window: u32,
        reason: UnmanageReason,
    },
    /// Focus changed to a different window
    FocusChanged {
        from: Option<u32>,
        to: Option<u32>,
    },
    /// Tab switched within a frame
    TabSwitched {
        frame: String,
        from: usize,
        to: usize,
    },
    /// Frame was split
    FrameSplit {
        original_frame: String,
        new_frame: String,
        direction: String,
    },
    /// Split ratio was adjusted
    SplitResized {
        split: String,
        old_ratio: f32,
        new_ratio: f32,
    },
    /// Window moved between frames
    WindowMoved {
        window: u32,
        from_frame: String,
        to_frame: String,
    },
    /// Empty frame was removed
    FrameRemoved {
        frame: String,
    },
}

/// Reason a window was unmanaged
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnmanageReason {
    /// Client closed/destroyed the window
    ClientDestroyed,
    /// Client unmapped the window
    ClientUnmapped,
    /// WM closed the window (user action)
    WmClosed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_window_state_serialization() {
        let state = WindowState::Visible;
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(json, "\"visible\"");

        let parsed: WindowState = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, WindowState::Visible);
    }

    #[test]
    fn test_focus_state_serialization() {
        let state = FocusState::Focused { window: 12345 };
        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains("12345"));

        let parsed: FocusState = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, FocusState::Focused { window: 12345 });
    }

    #[test]
    fn test_state_transition_serialization() {
        let transition = StateTransition::WindowManaged {
            window: 42,
            frame: "NodeId(1)".to_string(),
        };
        let json = serde_json::to_string(&transition).unwrap();
        assert!(json.contains("window_managed"));
        assert!(json.contains("42"));
    }
}
