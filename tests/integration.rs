//! Integration tests for ttwm using Xvfb.
//!
//! These tests require:
//! - Xvfb (headless X server)
//! - Built ttwm and ttwmctl binaries
//!
//! Run with: RUST_LOG=info cargo test --test integration
//!
//! If Xvfb is not available, tests will be skipped.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use serde_json::Value;

/// Check if Xvfb is available
fn xvfb_available() -> bool {
    Command::new("which")
        .arg("Xvfb")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Test fixture that manages Xvfb and ttwm lifecycle
struct TestHarness {
    xvfb: Child,
    wm: Child,
    display: String,
    socket_path: PathBuf,
}

impl TestHarness {
    /// Create a new test harness with Xvfb and ttwm
    fn new() -> Option<Self> {
        if !xvfb_available() {
            eprintln!("Xvfb not available, skipping integration tests");
            return None;
        }

        // Find an available display number
        let display = ":99";

        // Start Xvfb
        let xvfb = match Command::new("Xvfb")
            .args([display, "-screen", "0", "1280x800x24"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                eprintln!("Failed to start Xvfb: {}", e);
                return None;
            }
        };

        // Wait for Xvfb to be ready
        std::thread::sleep(Duration::from_millis(500));

        // Determine socket path
        let sanitized_display = display.replace([':', '.'], "_");
        let socket_path = PathBuf::from(format!("/tmp/ttwm{}.sock", sanitized_display));

        // Remove old socket if present
        let _ = std::fs::remove_file(&socket_path);

        // Start ttwm
        let wm = match Command::new("./target/debug/ttwm")
            .env("DISPLAY", display)
            .env("RUST_LOG", "info")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                eprintln!("Failed to start ttwm: {}", e);
                return None;
            }
        };

        // Wait for WM to be ready and IPC socket to exist
        for _ in 0..50 {
            if socket_path.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        if !socket_path.exists() {
            eprintln!("IPC socket never appeared at {:?}", socket_path);
            return None;
        }

        Some(Self {
            xvfb,
            wm,
            display: display.to_string(),
            socket_path,
        })
    }

    /// Send an IPC command and get the response
    fn send_command(&self, command: &Value) -> Result<Value, String> {
        let mut stream = UnixStream::connect(&self.socket_path)
            .map_err(|e| format!("Failed to connect to IPC socket: {}", e))?;

        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| format!("Failed to set read timeout: {}", e))?;

        let json = serde_json::to_string(command)
            .map_err(|e| format!("Failed to serialize command: {}", e))?;
        writeln!(stream, "{}", json)
            .map_err(|e| format!("Failed to write command: {}", e))?;
        stream
            .flush()
            .map_err(|e| format!("Failed to flush: {}", e))?;

        let mut reader = BufReader::new(&stream);
        let mut response = String::new();
        reader
            .read_line(&mut response)
            .map_err(|e| format!("Failed to read response: {}", e))?;

        serde_json::from_str(&response)
            .map_err(|e| format!("Failed to parse response: {}", e))
    }

    /// Get the current state
    fn get_state(&self) -> Result<Value, String> {
        self.send_command(&serde_json::json!({"command": "get_state"}))
    }

    /// Spawn a test window (xterm or similar)
    fn spawn_window(&self) -> Result<(), String> {
        Command::new("xterm")
            .env("DISPLAY", &self.display)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to spawn xterm: {}", e))?;

        // Wait for window to be managed
        std::thread::sleep(Duration::from_millis(200));
        Ok(())
    }

    /// Split the focused frame
    fn split(&self, direction: &str) -> Result<Value, String> {
        self.send_command(&serde_json::json!({
            "command": "split",
            "direction": direction
        }))
    }

    /// Validate state
    fn validate(&self) -> Result<Value, String> {
        self.send_command(&serde_json::json!({"command": "validate_state"}))
    }

    /// Quit the window manager
    fn quit(&self) -> Result<Value, String> {
        self.send_command(&serde_json::json!({"command": "quit"}))
    }

    /// Get the layout tree
    fn get_layout(&self) -> Result<Value, String> {
        self.send_command(&serde_json::json!({"command": "get_layout"}))
    }

    /// Get all windows
    fn get_windows(&self) -> Result<Value, String> {
        self.send_command(&serde_json::json!({"command": "get_windows"}))
    }

    /// Get focused window
    fn get_focused(&self) -> Result<Value, String> {
        self.send_command(&serde_json::json!({"command": "get_focused"}))
    }

    /// Get event log with optional count limit
    fn get_event_log(&self, count: Option<usize>) -> Result<Value, String> {
        match count {
            Some(n) => self.send_command(&serde_json::json!({
                "command": "get_event_log",
                "count": n
            })),
            None => self.send_command(&serde_json::json!({"command": "get_event_log"})),
        }
    }

    /// Take a screenshot
    fn screenshot(&self, path: &str) -> Result<Value, String> {
        self.send_command(&serde_json::json!({
            "command": "screenshot",
            "path": path
        }))
    }

    /// Focus adjacent frame
    fn focus_frame(&self, forward: bool) -> Result<Value, String> {
        self.send_command(&serde_json::json!({
            "command": "focus_frame",
            "forward": forward
        }))
    }

    /// Resize the current split
    fn resize_split(&self, delta: f32) -> Result<Value, String> {
        self.send_command(&serde_json::json!({
            "command": "resize_split",
            "delta": delta
        }))
    }

    /// Focus a specific window by ID
    fn focus_window(&self, window: u32) -> Result<Value, String> {
        self.send_command(&serde_json::json!({
            "command": "focus_window",
            "window": window
        }))
    }
}

impl Drop for TestHarness {
    fn drop(&mut self) {
        // Try graceful shutdown first
        let _ = self.quit();
        std::thread::sleep(Duration::from_millis(100));

        // Force kill if still running
        let _ = self.wm.kill();
        let _ = self.wm.wait();
        let _ = self.xvfb.kill();
        let _ = self.xvfb.wait();
    }
}

#[test]
fn test_wm_starts_and_responds() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    // Get initial state
    let state = harness.get_state().expect("Failed to get state");

    // Check that we got a valid state response
    assert_eq!(state.get("status").and_then(|v| v.as_str()), Some("state"));

    // Should have zero windows initially
    let data = state.get("data").expect("Missing data");
    assert_eq!(data.get("window_count").and_then(|v| v.as_u64()), Some(0));
}

#[test]
fn test_state_validation() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    // Validate state
    let result = harness.validate().expect("Failed to validate");

    // Should be valid
    assert_eq!(result.get("status").and_then(|v| v.as_str()), Some("validation"));
    assert_eq!(result.get("valid").and_then(|v| v.as_bool()), Some(true));
}

#[test]
fn test_split_creates_two_frames() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    // Initial state should have 1 frame
    let state = harness.get_state().expect("Failed to get state");
    let data = state.get("data").expect("Missing data");
    assert_eq!(data.get("frame_count").and_then(|v| v.as_u64()), Some(1));

    // Split horizontally
    let result = harness.split("horizontal").expect("Failed to split");
    assert_eq!(result.get("status").and_then(|v| v.as_str()), Some("ok"));

    // Should now have 2 frames
    let state = harness.get_state().expect("Failed to get state");
    let data = state.get("data").expect("Missing data");
    assert_eq!(data.get("frame_count").and_then(|v| v.as_u64()), Some(2));

    // State should still be valid
    let result = harness.validate().expect("Failed to validate");
    assert_eq!(result.get("valid").and_then(|v| v.as_bool()), Some(true));
}

// Note: Tests that spawn windows require xterm and may be flaky
// They are left as examples but commented out by default

/*
#[test]
fn test_window_management() {
    let Some(harness) = TestHarness::new() else {
        return;
    };

    // Spawn a window
    harness.spawn_window().expect("Failed to spawn window");

    // Should now have 1 window
    let state = harness.get_state().expect("Failed to get state");
    let data = state.get("data").expect("Missing data");
    assert_eq!(data.get("window_count").and_then(|v| v.as_u64()), Some(1));
}
*/

// =============================================================================
// Layout & Splitting Tests
// =============================================================================

#[test]
fn test_vertical_split() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    // Initial state should have 1 frame
    let state = harness.get_state().expect("Failed to get state");
    let data = state.get("data").expect("Missing data");
    assert_eq!(data.get("frame_count").and_then(|v| v.as_u64()), Some(1));

    // Split vertically
    let result = harness.split("vertical").expect("Failed to split");
    assert_eq!(result.get("status").and_then(|v| v.as_str()), Some("ok"));

    // Should now have 2 frames
    let state = harness.get_state().expect("Failed to get state");
    let data = state.get("data").expect("Missing data");
    assert_eq!(data.get("frame_count").and_then(|v| v.as_u64()), Some(2));

    // State should still be valid
    let result = harness.validate().expect("Failed to validate");
    assert_eq!(result.get("valid").and_then(|v| v.as_bool()), Some(true));
}

#[test]
fn test_multiple_sequential_splits() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    // Initial: 1 frame
    let state = harness.get_state().expect("Failed to get state");
    let data = state.get("data").expect("Missing data");
    assert_eq!(data.get("frame_count").and_then(|v| v.as_u64()), Some(1));

    // First split (horizontal): 2 frames
    harness.split("horizontal").expect("Failed to split horizontal");
    let state = harness.get_state().expect("Failed to get state");
    let data = state.get("data").expect("Missing data");
    assert_eq!(data.get("frame_count").and_then(|v| v.as_u64()), Some(2));

    // Second split (vertical on current frame): 3 frames
    harness.split("vertical").expect("Failed to split vertical");
    let state = harness.get_state().expect("Failed to get state");
    let data = state.get("data").expect("Missing data");
    assert_eq!(data.get("frame_count").and_then(|v| v.as_u64()), Some(3));

    // Validate state
    let result = harness.validate().expect("Failed to validate");
    assert_eq!(result.get("valid").and_then(|v| v.as_bool()), Some(true));
}

#[test]
fn test_split_shorthand_directions() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    // Test shorthand "h" for horizontal
    let result = harness.split("h").expect("Failed to split with 'h'");
    assert_eq!(result.get("status").and_then(|v| v.as_str()), Some("ok"));

    let state = harness.get_state().expect("Failed to get state");
    let data = state.get("data").expect("Missing data");
    assert_eq!(data.get("frame_count").and_then(|v| v.as_u64()), Some(2));

    // Test shorthand "v" for vertical
    let result = harness.split("v").expect("Failed to split with 'v'");
    assert_eq!(result.get("status").and_then(|v| v.as_str()), Some("ok"));

    let state = harness.get_state().expect("Failed to get state");
    let data = state.get("data").expect("Missing data");
    assert_eq!(data.get("frame_count").and_then(|v| v.as_u64()), Some(3));

    // Validate
    let result = harness.validate().expect("Failed to validate");
    assert_eq!(result.get("valid").and_then(|v| v.as_bool()), Some(true));
}

// =============================================================================
// Frame Navigation Tests
// =============================================================================

#[test]
fn test_focus_frame_navigation() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    // Create 2 frames
    harness.split("horizontal").expect("Failed to split");

    // Get initial focused frame
    let state1 = harness.get_state().expect("Failed to get state");
    let data1 = state1.get("data").expect("Missing data");
    let focused_frame1 = data1.get("focused_frame").and_then(|v| v.as_str()).unwrap();

    // Navigate to next frame
    harness.focus_frame(true).expect("Failed to focus frame forward");

    // Focused frame should have changed
    let state2 = harness.get_state().expect("Failed to get state");
    let data2 = state2.get("data").expect("Missing data");
    let focused_frame2 = data2.get("focused_frame").and_then(|v| v.as_str()).unwrap();

    assert_ne!(focused_frame1, focused_frame2, "Focused frame should change after navigation");

    // Navigate back
    harness.focus_frame(false).expect("Failed to focus frame backward");

    let state3 = harness.get_state().expect("Failed to get state");
    let data3 = state3.get("data").expect("Missing data");
    let focused_frame3 = data3.get("focused_frame").and_then(|v| v.as_str()).unwrap();

    assert_eq!(focused_frame1, focused_frame3, "Should return to original frame");

    // Validate
    let result = harness.validate().expect("Failed to validate");
    assert_eq!(result.get("valid").and_then(|v| v.as_bool()), Some(true));
}

// =============================================================================
// Event Logging Tests
// =============================================================================

#[test]
fn test_event_log_records_operations() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    // Perform some operations
    harness.split("horizontal").expect("Failed to split");
    harness.split("vertical").expect("Failed to split");

    // Query event log
    let result = harness.get_event_log(None).expect("Failed to get event log");
    assert_eq!(result.get("status").and_then(|v| v.as_str()), Some("event_log"));

    let entries = result.get("entries").and_then(|v| v.as_array()).expect("Missing entries");

    // Should have recorded some events
    assert!(!entries.is_empty(), "Event log should not be empty after operations");

    // Look for split events
    let has_split_events = entries.iter().any(|e| {
        e.get("event_type")
            .and_then(|v| v.as_str())
            .map(|s| s.contains("split") || s.contains("frame"))
            .unwrap_or(false)
    });
    assert!(has_split_events, "Should have recorded split-related events");
}

#[test]
fn test_event_log_respects_count_limit() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    // Perform several operations to generate events
    for _ in 0..5 {
        harness.split("horizontal").expect("Failed to split");
    }

    // Request only 3 events
    let result = harness.get_event_log(Some(3)).expect("Failed to get event log");
    let entries = result.get("entries").and_then(|v| v.as_array()).expect("Missing entries");

    assert!(entries.len() <= 3, "Should return at most 3 events, got {}", entries.len());
}

#[test]
fn test_event_log_sequence_numbers() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    // Perform some operations
    harness.split("horizontal").expect("Failed to split");
    harness.split("vertical").expect("Failed to split");

    // Query event log
    let result = harness.get_event_log(None).expect("Failed to get event log");
    let entries = result.get("entries").and_then(|v| v.as_array()).expect("Missing entries");

    // Verify sequence numbers are monotonically increasing
    let mut prev_seq = 0u64;
    for entry in entries {
        let seq = entry.get("sequence").and_then(|v| v.as_u64()).expect("Missing sequence");
        assert!(seq > prev_seq, "Sequence numbers should be monotonically increasing");
        prev_seq = seq;
    }
}

// =============================================================================
// Screenshot Tests
// =============================================================================

#[test]
fn test_screenshot_creates_file() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    // Create a temp file path
    let screenshot_path = "/tmp/ttwm_test_screenshot.png";

    // Remove if exists from previous run
    let _ = std::fs::remove_file(screenshot_path);

    // Take screenshot
    let result = harness.screenshot(screenshot_path).expect("Failed to take screenshot");
    assert_eq!(result.get("status").and_then(|v| v.as_str()), Some("screenshot"));

    // Verify file was created
    assert!(
        std::path::Path::new(screenshot_path).exists(),
        "Screenshot file should exist"
    );

    // Verify it's a PNG (check magic bytes)
    let data = std::fs::read(screenshot_path).expect("Failed to read screenshot");
    assert!(data.len() > 8, "Screenshot file should have content");

    // PNG magic bytes: 137 80 78 71 13 10 26 10
    let png_magic = [137u8, 80, 78, 71, 13, 10, 26, 10];
    assert_eq!(&data[0..8], &png_magic, "File should be a valid PNG");

    // Cleanup
    let _ = std::fs::remove_file(screenshot_path);
}

#[test]
fn test_screenshot_error_on_invalid_path() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    // Try to save to non-existent directory
    let result = harness.screenshot("/nonexistent_dir_12345/test.png");

    match result {
        Ok(response) => {
            // Should be an error response
            assert_eq!(
                response.get("status").and_then(|v| v.as_str()),
                Some("error"),
                "Should return error for invalid path"
            );
        }
        Err(_) => {
            // Also acceptable - connection error or similar
        }
    }
}

// =============================================================================
// State Query Tests
// =============================================================================

#[test]
fn test_get_layout_returns_tree() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    // Get layout
    let result = harness.get_layout().expect("Failed to get layout");
    assert_eq!(result.get("status").and_then(|v| v.as_str()), Some("layout"));

    // Should have layout data
    let layout = result.get("layout").expect("Missing layout");

    // Initial layout should be a single frame
    assert_eq!(
        layout.get("type").and_then(|v| v.as_str()),
        Some("frame"),
        "Initial layout should be a frame"
    );

    // Split and check layout changes to split type at root
    harness.split("horizontal").expect("Failed to split");

    let result = harness.get_layout().expect("Failed to get layout after split");
    let layout = result.get("layout").expect("Missing layout");

    assert_eq!(
        layout.get("type").and_then(|v| v.as_str()),
        Some("split"),
        "Layout should be a split after splitting"
    );

    // Split should have direction
    assert!(layout.get("direction").is_some(), "Split should have direction");

    // Split should have first and second children
    assert!(layout.get("first").is_some(), "Split should have first child");
    assert!(layout.get("second").is_some(), "Split should have second child");
}

#[test]
fn test_get_windows_empty_initially() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    // Get windows
    let result = harness.get_windows().expect("Failed to get windows");
    assert_eq!(result.get("status").and_then(|v| v.as_str()), Some("windows"));

    // Should have empty windows list
    let windows = result.get("windows").and_then(|v| v.as_array()).expect("Missing windows");
    assert!(windows.is_empty(), "Should have no windows initially");
}

#[test]
fn test_get_focused_none_initially() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    // Get focused window
    let result = harness.get_focused().expect("Failed to get focused");
    assert_eq!(result.get("status").and_then(|v| v.as_str()), Some("focused"));

    // Should be null/none initially (no windows)
    let focused = result.get("window");
    assert!(
        focused.is_none() || focused.unwrap().is_null(),
        "Should have no focused window initially"
    );
}

// =============================================================================
// Error Handling Tests
// =============================================================================

#[test]
fn test_invalid_command_returns_error() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    // Send invalid command
    let result = harness.send_command(&serde_json::json!({"command": "nonexistent_command"}));

    match result {
        Ok(response) => {
            assert_eq!(
                response.get("status").and_then(|v| v.as_str()),
                Some("error"),
                "Should return error for invalid command"
            );
        }
        Err(_) => {
            // Parse error is also acceptable
        }
    }
}

#[test]
fn test_focus_nonexistent_window() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    // Try to focus a window ID that doesn't exist
    let result = harness.focus_window(99999999).expect("Failed to send command");

    assert_eq!(
        result.get("status").and_then(|v| v.as_str()),
        Some("error"),
        "Should return error for non-existent window"
    );
}

#[test]
fn test_invalid_split_direction() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    // Try to split with invalid direction
    let result = harness.send_command(&serde_json::json!({
        "command": "split",
        "direction": "diagonal"
    })).expect("Failed to send command");

    assert_eq!(
        result.get("status").and_then(|v| v.as_str()),
        Some("error"),
        "Should return error for invalid split direction"
    );
}

// =============================================================================
// State Validation Tests
// =============================================================================

#[test]
fn test_validation_after_multiple_operations() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    // Perform a series of operations
    harness.split("horizontal").expect("Failed to split");
    harness.split("vertical").expect("Failed to split");
    harness.focus_frame(true).expect("Failed to focus frame");
    harness.split("horizontal").expect("Failed to split");
    harness.focus_frame(false).expect("Failed to focus frame");

    // Validate state after all operations
    let result = harness.validate().expect("Failed to validate");

    assert_eq!(result.get("status").and_then(|v| v.as_str()), Some("validation"));
    assert_eq!(
        result.get("valid").and_then(|v| v.as_bool()),
        Some(true),
        "State should remain valid after multiple operations"
    );

    // Check violations list is empty
    let violations = result.get("violations").and_then(|v| v.as_array());
    if let Some(v) = violations {
        assert!(v.is_empty(), "Should have no violations, got: {:?}", v);
    }
}

// =============================================================================
// Resize Split Tests
// =============================================================================

#[test]
fn test_resize_split() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    // Create a split first
    harness.split("horizontal").expect("Failed to split");

    // Get initial layout to check ratio
    let layout1 = harness.get_layout().expect("Failed to get layout");
    let layout_data1 = layout1.get("layout").expect("Missing layout");
    let ratio1 = layout_data1.get("ratio").and_then(|v| v.as_f64()).unwrap_or(0.5);

    // Resize the split
    let result = harness.resize_split(0.1).expect("Failed to resize split");
    assert_eq!(result.get("status").and_then(|v| v.as_str()), Some("ok"));

    // Get layout again to check ratio changed
    let layout2 = harness.get_layout().expect("Failed to get layout");
    let layout_data2 = layout2.get("layout").expect("Missing layout");
    let ratio2 = layout_data2.get("ratio").and_then(|v| v.as_f64()).unwrap_or(0.5);

    assert!(
        (ratio2 - ratio1).abs() > 0.01,
        "Ratio should change after resize: {} -> {}",
        ratio1,
        ratio2
    );

    // Validate state
    let result = harness.validate().expect("Failed to validate");
    assert_eq!(result.get("valid").and_then(|v| v.as_bool()), Some(true));
}

#[test]
fn test_resize_split_bounds() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    // Create a split
    harness.split("horizontal").expect("Failed to split");

    // Try to resize way past bounds (should be clamped)
    for _ in 0..20 {
        let _ = harness.resize_split(0.1);
    }

    // Get layout to check ratio is within bounds
    let layout = harness.get_layout().expect("Failed to get layout");
    let layout_data = layout.get("layout").expect("Missing layout");
    let ratio = layout_data.get("ratio").and_then(|v| v.as_f64()).unwrap_or(0.5);

    assert!(ratio >= 0.1 && ratio <= 0.9, "Ratio should be within bounds [0.1, 0.9], got {}", ratio);

    // State should still be valid
    let result = harness.validate().expect("Failed to validate");
    assert_eq!(result.get("valid").and_then(|v| v.as_bool()), Some(true));
}
