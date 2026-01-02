//! IPC interface for programmatic control of ttwm.
//!
//! Provides a Unix socket server that accepts JSON commands and returns JSON responses.
//! This enables coding agents and external tools to:
//! - Query WM state
//! - Execute actions (focus, split, etc.)
//! - Capture screenshots
//! - Validate state invariants

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub use crate::types::LayoutSnapshot;

/// Get the socket path for this display
pub fn socket_path() -> PathBuf {
    let display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".to_string());
    let sanitized = display.replace([':', '.'], "_");
    PathBuf::from(format!("/tmp/ttwm{}.sock", sanitized))
}

/// Commands that can be sent to the WM via IPC
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum IpcCommand {
    // Queries
    /// Get full WM state snapshot
    GetState,
    /// Get layout tree as JSON
    GetLayout,
    /// Get list of all managed windows
    GetWindows,
    /// Get currently focused window
    GetFocused,
    /// Validate state invariants
    ValidateState,
    /// Get recent event log
    GetEventLog {
        #[serde(default)]
        count: Option<usize>,
    },

    // Actions
    /// Focus a specific window
    FocusWindow { window: u32 },
    /// Focus a specific tab by index (1-based)
    FocusTab { index: usize },
    /// Focus frame in direction (left, right, up, down)
    FocusFrame { direction: String },
    /// Split the focused frame
    Split { direction: String },
    /// Move window to adjacent frame
    MoveWindow { forward: bool },
    /// Resize the focused split
    ResizeSplit { delta: f32 },
    /// Close the focused window
    CloseWindow,
    /// Cycle tabs in focused frame
    CycleTab { forward: bool },

    // Tagging
    /// Tag a window (uses focused window if not specified)
    TagWindow { window: Option<u32> },
    /// Untag a window (uses focused window if not specified)
    UntagWindow { window: Option<u32> },
    /// Toggle tag on a window (uses focused window if not specified)
    ToggleTag { window: Option<u32> },
    /// Move all tagged windows to the focused frame and untag them
    MoveTagged,
    /// Untag all windows
    UntagAll,
    /// Get list of tagged window IDs
    GetTagged,

    // Floating
    /// Toggle floating state for a window (uses focused window if not specified)
    ToggleFloat { window: Option<u32> },
    /// Get list of floating window IDs
    GetFloating,

    // Urgent
    /// Get list of urgent window IDs (ordered oldest first)
    GetUrgent,
    /// Focus the oldest urgent window
    FocusUrgent,

    // Workspaces
    /// Switch to a specific workspace (0-8)
    SwitchWorkspace { index: usize },
    /// Switch to next workspace
    WorkspaceNext,
    /// Switch to previous workspace
    WorkspacePrev,
    /// Get current workspace index
    GetCurrentWorkspace,
    /// Move a window to a specific workspace
    MoveToWorkspace { window: Option<u32>, workspace: usize },

    // Monitors
    /// Get list of all monitors
    GetMonitors,
    /// Get currently focused monitor
    GetCurrentMonitor,
    /// Focus a specific monitor by name or direction (left/right)
    FocusMonitor { target: String },

    // Debug
    /// Capture screenshot to file
    Screenshot { path: String },

    // Control
    /// Quit the window manager
    Quit,
}

/// Responses from the WM
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum IpcResponse {
    /// Operation succeeded with no data
    Ok,
    /// Full state snapshot
    State { data: WmStateSnapshot },
    /// Layout tree
    Layout { data: LayoutSnapshot },
    /// List of windows
    Windows { data: Vec<WindowInfo> },
    /// Focused window
    Focused { window: Option<u32> },
    /// Validation result
    Validation {
        valid: bool,
        violations: Vec<String>,
    },
    /// Event log
    EventLog { entries: Vec<EventLogEntry> },
    /// Screenshot saved
    Screenshot { path: String },
    /// List of tagged window IDs
    Tagged { windows: Vec<u32> },
    /// List of floating window IDs
    Floating { windows: Vec<u32> },
    /// List of urgent window IDs (ordered oldest first)
    Urgent { windows: Vec<u32> },
    /// Current workspace info
    Workspace { index: usize, total: usize },
    /// List of monitors
    Monitors { data: Vec<MonitorInfo> },
    /// Current monitor info
    Monitor { name: String, is_primary: bool },
    /// Error response
    Error { code: String, message: String },
}

/// Snapshot of the full WM state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WmStateSnapshot {
    pub focused_window: Option<u32>,
    pub focused_frame: String,
    pub window_count: usize,
    pub frame_count: usize,
    pub layout: LayoutSnapshot,
    pub windows: Vec<WindowInfo>,
}

/// Information about a managed window
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowInfo {
    pub id: u32,
    pub title: String,
    pub frame: String,
    pub tab_index: usize,
    pub is_focused: bool,
    pub is_visible: bool,
    pub is_tagged: bool,
    pub is_floating: bool,
    pub is_urgent: bool,
}

/// Entry in the event log
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventLogEntry {
    pub sequence: u64,
    pub timestamp_ms: u64,
    pub event_type: String,
    pub window: Option<u32>,
    pub details: String,
}

/// Information about a monitor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorInfo {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub is_primary: bool,
    pub is_focused: bool,
    pub current_workspace: usize,
}

/// IPC server that listens on a Unix socket
pub struct IpcServer {
    listener: UnixListener,
    socket_path: PathBuf,
}

impl IpcServer {
    /// Create a new IPC server bound to the socket path
    pub fn bind() -> std::io::Result<Self> {
        let path = socket_path();

        // Remove existing socket if present
        if path.exists() {
            std::fs::remove_file(&path)?;
        }

        let listener = UnixListener::bind(&path)?;

        // Set non-blocking mode for polling
        listener.set_nonblocking(true)?;

        log::info!("IPC server listening on {:?}", path);

        Ok(Self {
            listener,
            socket_path: path,
        })
    }

    /// Poll for incoming commands (non-blocking)
    /// Returns None if no command is pending
    pub fn poll(&self) -> Option<(IpcCommand, IpcClient)> {
        match self.listener.accept() {
            Ok((stream, _)) => {
                // Set a read timeout for the stream
                stream
                    .set_read_timeout(Some(Duration::from_millis(100)))
                    .ok();
                stream
                    .set_write_timeout(Some(Duration::from_millis(100)))
                    .ok();

                let mut reader = BufReader::new(stream.try_clone().ok()?);
                let mut line = String::new();

                match reader.read_line(&mut line) {
                    Ok(0) => None, // EOF
                    Ok(_) => {
                        match serde_json::from_str::<IpcCommand>(&line) {
                            Ok(cmd) => {
                                log::debug!("IPC command received: {:?}", cmd);
                                Some((cmd, IpcClient { stream }))
                            }
                            Err(e) => {
                                log::warn!("Invalid IPC command: {}", e);
                                // Send error response
                                let mut client = IpcClient { stream };
                                if let Err(resp_err) = client.respond(IpcResponse::Error {
                                    code: "parse_error".to_string(),
                                    message: format!("Failed to parse command: {}", e),
                                }) {
                                    log::warn!("Failed to send error response: {}", resp_err);
                                }
                                None
                            }
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => None,
                    Err(e) => {
                        log::warn!("IPC read error: {}", e);
                        None
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => None,
            Err(e) => {
                log::warn!("IPC accept error: {}", e);
                None
            }
        }
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        // Clean up socket file
        if let Err(e) = std::fs::remove_file(&self.socket_path) {
            log::debug!("Failed to remove socket file {:?}: {}", self.socket_path, e);
        }
    }
}

/// Handle for responding to an IPC client
pub struct IpcClient {
    stream: UnixStream,
}

impl IpcClient {
    /// Send a response to the client
    pub fn respond(&mut self, response: IpcResponse) -> std::io::Result<()> {
        let json = serde_json::to_string(&response)?;
        writeln!(self.stream, "{}", json)?;
        self.stream.flush()?;
        Ok(())
    }
}

/// Client for connecting to the IPC server (used by ttwmctl)
#[allow(dead_code)]
pub struct IpcConnection {
    stream: UnixStream,
}

#[allow(dead_code)]
impl IpcConnection {
    /// Connect to the WM's IPC socket
    pub fn connect() -> std::io::Result<Self> {
        let path = socket_path();
        let stream = UnixStream::connect(&path)?;

        // Set timeouts
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;

        Ok(Self { stream })
    }

    /// Send a command and receive the response
    pub fn send(&mut self, command: &IpcCommand) -> std::io::Result<IpcResponse> {
        let json = serde_json::to_string(command)?;
        writeln!(self.stream, "{}", json)?;
        self.stream.flush()?;

        let mut reader = BufReader::new(&self.stream);
        let mut line = String::new();
        reader.read_line(&mut line)?;

        serde_json::from_str(&line).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_serialization() {
        let cmd = IpcCommand::GetState;
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("get_state"));

        let cmd = IpcCommand::FocusWindow { window: 12345 };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("focus_window"));
        assert!(json.contains("12345"));

        let cmd = IpcCommand::Split {
            direction: "horizontal".to_string(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("horizontal"));
    }

    #[test]
    fn test_response_serialization() {
        let resp = IpcResponse::Ok;
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("ok"));

        let resp = IpcResponse::Error {
            code: "test".to_string(),
            message: "test error".to_string(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("error"));
        assert!(json.contains("test error"));
    }

    #[test]
    fn test_command_deserialization() {
        let json = r#"{"command": "get_state"}"#;
        let cmd: IpcCommand = serde_json::from_str(json).unwrap();
        assert!(matches!(cmd, IpcCommand::GetState));

        let json = r#"{"command": "focus_window", "window": 42}"#;
        let cmd: IpcCommand = serde_json::from_str(json).unwrap();
        assert!(matches!(cmd, IpcCommand::FocusWindow { window: 42 }));

        let json = r#"{"command": "split", "direction": "vertical"}"#;
        let cmd: IpcCommand = serde_json::from_str(json).unwrap();
        if let IpcCommand::Split { direction } = cmd {
            assert_eq!(direction, "vertical");
        } else {
            panic!("Expected Split command");
        }
    }

    #[test]
    fn test_toggle_float_command_serialization() {
        // With window specified
        let cmd = IpcCommand::ToggleFloat { window: Some(12345) };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("toggle_float"));
        assert!(json.contains("12345"));

        // Without window (uses focused)
        let cmd = IpcCommand::ToggleFloat { window: None };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("toggle_float"));
        assert!(json.contains("null"));
    }

    #[test]
    fn test_toggle_float_command_deserialization() {
        let json = r#"{"command": "toggle_float", "window": 42}"#;
        let cmd: IpcCommand = serde_json::from_str(json).unwrap();
        assert!(matches!(cmd, IpcCommand::ToggleFloat { window: Some(42) }));

        let json = r#"{"command": "toggle_float", "window": null}"#;
        let cmd: IpcCommand = serde_json::from_str(json).unwrap();
        assert!(matches!(cmd, IpcCommand::ToggleFloat { window: None }));

        let json = r#"{"command": "toggle_float"}"#;
        let cmd: IpcCommand = serde_json::from_str(json).unwrap();
        assert!(matches!(cmd, IpcCommand::ToggleFloat { window: None }));
    }

    #[test]
    fn test_get_floating_command_serialization() {
        let cmd = IpcCommand::GetFloating;
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("get_floating"));
    }

    #[test]
    fn test_get_floating_command_deserialization() {
        let json = r#"{"command": "get_floating"}"#;
        let cmd: IpcCommand = serde_json::from_str(json).unwrap();
        assert!(matches!(cmd, IpcCommand::GetFloating));
    }

    #[test]
    fn test_floating_response_serialization() {
        let resp = IpcResponse::Floating {
            windows: vec![100, 200, 300],
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("floating"));
        assert!(json.contains("100"));
        assert!(json.contains("200"));
        assert!(json.contains("300"));
    }

    #[test]
    fn test_floating_response_empty() {
        let resp = IpcResponse::Floating { windows: vec![] };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("floating"));
        assert!(json.contains("[]"));
    }

    #[test]
    fn test_window_info_with_is_floating() {
        let info = WindowInfo {
            id: 12345,
            title: "Test Window".to_string(),
            frame: "floating".to_string(),
            tab_index: 0,
            is_focused: true,
            is_visible: true,
            is_tagged: false,
            is_floating: true,
            is_urgent: false,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"is_floating\":true"));

        let info = WindowInfo {
            id: 12345,
            title: "Test Window".to_string(),
            frame: "frame_1".to_string(),
            tab_index: 0,
            is_focused: false,
            is_visible: true,
            is_tagged: false,
            is_floating: false,
            is_urgent: false,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"is_floating\":false"));
    }

    #[test]
    fn test_get_urgent_command_serialization() {
        let cmd = IpcCommand::GetUrgent;
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("get_urgent"));
    }

    #[test]
    fn test_get_urgent_command_deserialization() {
        let json = r#"{"command": "get_urgent"}"#;
        let cmd: IpcCommand = serde_json::from_str(json).unwrap();
        assert!(matches!(cmd, IpcCommand::GetUrgent));
    }

    #[test]
    fn test_focus_urgent_command_serialization() {
        let cmd = IpcCommand::FocusUrgent;
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("focus_urgent"));
    }

    #[test]
    fn test_focus_urgent_command_deserialization() {
        let json = r#"{"command": "focus_urgent"}"#;
        let cmd: IpcCommand = serde_json::from_str(json).unwrap();
        assert!(matches!(cmd, IpcCommand::FocusUrgent));
    }

    #[test]
    fn test_urgent_response_serialization() {
        let resp = IpcResponse::Urgent {
            windows: vec![100, 200, 300],
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("urgent"));
        assert!(json.contains("100"));
        assert!(json.contains("200"));
        assert!(json.contains("300"));
    }

    #[test]
    fn test_urgent_response_empty() {
        let resp = IpcResponse::Urgent { windows: vec![] };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("urgent"));
        assert!(json.contains("[]"));
    }

    #[test]
    fn test_window_info_with_is_urgent() {
        let info = WindowInfo {
            id: 12345,
            title: "Urgent Window".to_string(),
            frame: "frame_1".to_string(),
            tab_index: 0,
            is_focused: false,
            is_visible: true,
            is_tagged: false,
            is_floating: false,
            is_urgent: true,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"is_urgent\":true"));

        let info = WindowInfo {
            id: 12345,
            title: "Normal Window".to_string(),
            frame: "frame_1".to_string(),
            tab_index: 0,
            is_focused: false,
            is_visible: true,
            is_tagged: false,
            is_floating: false,
            is_urgent: false,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"is_urgent\":false"));
    }
}
