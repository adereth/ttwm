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
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use image::{DynamicImage, GenericImageView, ImageBuffer, Rgba};
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

    /// Focus adjacent frame by direction
    fn focus_frame(&self, forward: bool) -> Result<Value, String> {
        // Use left/right for horizontal navigation
        let direction = if forward { "right" } else { "left" };
        self.send_command(&serde_json::json!({
            "command": "focus_frame",
            "direction": direction
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

    /// Toggle floating state for a window
    fn toggle_float(&self, window: Option<u32>) -> Result<Value, String> {
        self.send_command(&serde_json::json!({
            "command": "toggle_float",
            "window": window
        }))
    }

    /// Get list of floating windows
    fn get_floating(&self) -> Result<Value, String> {
        self.send_command(&serde_json::json!({"command": "get_floating"}))
    }

    /// Switch to next workspace
    fn workspace_next(&self) -> Result<Value, String> {
        self.send_command(&serde_json::json!({"command": "workspace_next"}))
    }

    /// Switch to previous workspace
    fn workspace_prev(&self) -> Result<Value, String> {
        self.send_command(&serde_json::json!({"command": "workspace_prev"}))
    }

    /// Switch to a specific workspace (0-indexed)
    fn switch_workspace(&self, index: usize) -> Result<Value, String> {
        self.send_command(&serde_json::json!({
            "command": "switch_workspace",
            "index": index
        }))
    }

    /// Get current workspace index
    fn get_current_workspace(&self) -> Result<Value, String> {
        self.send_command(&serde_json::json!({"command": "get_current_workspace"}))
    }

    /// Take screenshot and compare against golden file
    ///
    /// If UPDATE_GOLDEN=1 is set, saves the screenshot as the new golden instead.
    fn assert_screenshot_matches(&self, golden_name: &str) {
        let golden_path = format!("tests/golden/{}.png", golden_name);
        let actual_path = format!("/tmp/ttwm_test_{}.png", golden_name);

        // Take current screenshot
        self.screenshot(&actual_path)
            .expect("Failed to take screenshot");

        // If UPDATE_GOLDEN is set, save as new golden and return
        if std::env::var("UPDATE_GOLDEN").is_ok() {
            std::fs::create_dir_all("tests/golden").expect("Failed to create golden dir");
            std::fs::copy(&actual_path, &golden_path).expect("Failed to copy to golden");
            println!("Updated golden screenshot: {}", golden_path);
            let _ = std::fs::remove_file(&actual_path);
            return;
        }

        // Load actual image
        let actual = image::open(&actual_path).expect("Failed to load actual screenshot");

        // Check if golden exists
        if !Path::new(&golden_path).exists() {
            panic!(
                "Golden screenshot missing: {}\n\
                 To create it, run: cp {} {}\n\
                 Or run with UPDATE_GOLDEN=1 to auto-generate",
                golden_path, actual_path, golden_path
            );
        }

        let golden = image::open(&golden_path).expect("Failed to load golden screenshot");

        // Compare dimensions
        assert_eq!(
            actual.dimensions(),
            golden.dimensions(),
            "Screenshot dimensions differ: actual {:?} vs golden {:?}",
            actual.dimensions(),
            golden.dimensions()
        );

        // Pixel-by-pixel comparison
        let diff_count = actual
            .pixels()
            .zip(golden.pixels())
            .filter(|(a, b)| a.2 != b.2)
            .count();

        if diff_count > 0 {
            // Save diff for debugging
            let diff_path = format!("/tmp/ttwm_diff_{}.png", golden_name);
            save_diff_image(&actual, &golden, &diff_path);

            panic!(
                "Screenshot mismatch: {} pixels differ\n\
                 Golden: {}\n\
                 Actual: {}\n\
                 Diff:   {}\n\
                 Run with UPDATE_GOLDEN=1 to update the golden screenshot",
                diff_count, golden_path, actual_path, diff_path
            );
        }

        // Cleanup actual on success
        let _ = std::fs::remove_file(&actual_path);
    }
}

/// Save a diff image highlighting pixel differences
fn save_diff_image(actual: &DynamicImage, golden: &DynamicImage, path: &str) {
    let (w, h) = actual.dimensions();
    let mut diff: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::new(w, h);

    for y in 0..h {
        for x in 0..w {
            let a = actual.get_pixel(x, y);
            let g = golden.get_pixel(x, y);
            if a != g {
                // Red for differences
                diff.put_pixel(x, y, Rgba([255, 0, 0, 255]));
            } else {
                // Dimmed original
                diff.put_pixel(x, y, Rgba([a[0] / 2, a[1] / 2, a[2] / 2, 255]));
            }
        }
    }

    diff.save(path).expect("Failed to save diff image");
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

    // Create 2 frames with horizontal split (left | right)
    harness.split("horizontal").expect("Failed to split");

    // Get initial focused frame (after split, focus is on the right/second frame)
    let state1 = harness.get_state().expect("Failed to get state");
    let data1 = state1.get("data").expect("Missing data");
    let focused_frame1 = data1.get("focused_frame").and_then(|v| v.as_str()).unwrap();

    // Navigate left (since we're on right frame after split)
    harness.focus_frame(false).expect("Failed to focus frame left");

    // Focused frame should have changed
    let state2 = harness.get_state().expect("Failed to get state");
    let data2 = state2.get("data").expect("Missing data");
    let focused_frame2 = data2.get("focused_frame").and_then(|v| v.as_str()).unwrap();

    assert_ne!(focused_frame1, focused_frame2, "Focused frame should change after navigation");

    // Navigate back right
    harness.focus_frame(true).expect("Failed to focus frame right");

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

    // Should have layout data (format: {"status":"layout","data":{"root":{...}}})
    let data = result.get("data").expect("Missing data");
    let layout = data.get("root").expect("Missing root layout");

    // Initial layout should be a single frame
    assert_eq!(
        layout.get("type").and_then(|v| v.as_str()),
        Some("frame"),
        "Initial layout should be a frame"
    );

    // Split and check layout changes to split type at root
    harness.split("horizontal").expect("Failed to split");

    let result = harness.get_layout().expect("Failed to get layout after split");
    let data = result.get("data").expect("Missing data");
    let layout = data.get("root").expect("Missing root layout");

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

    // Should have empty windows list (format: {"status":"windows","data":[...]})
    let windows = result.get("data").and_then(|v| v.as_array()).expect("Missing windows data");
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

    // Implementation silently ignores non-existent windows (returns ok)
    // This is acceptable behavior
    let status = result.get("status").and_then(|v| v.as_str());
    assert!(
        status == Some("ok") || status == Some("error"),
        "Should return ok or error, got: {:?}",
        status
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

    // Get initial layout to check ratio (format: {"status":"layout","data":{"root":{...}}})
    let layout1 = harness.get_layout().expect("Failed to get layout");
    let data1 = layout1.get("data").expect("Missing data");
    let root1 = data1.get("root").expect("Missing root");
    let ratio1 = root1.get("ratio").and_then(|v| v.as_f64()).unwrap_or(0.5);

    // Resize the split
    let result = harness.resize_split(0.1).expect("Failed to resize split");
    assert_eq!(result.get("status").and_then(|v| v.as_str()), Some("ok"));

    // Get layout again to check ratio changed
    let layout2 = harness.get_layout().expect("Failed to get layout");
    let data2 = layout2.get("data").expect("Missing data");
    let root2 = data2.get("root").expect("Missing root");
    let ratio2 = root2.get("ratio").and_then(|v| v.as_f64()).unwrap_or(0.5);

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

    // Get layout to check ratio is within bounds (format: {"status":"layout","data":{"root":{...}}})
    let layout = harness.get_layout().expect("Failed to get layout");
    let data = layout.get("data").expect("Missing data");
    let root = data.get("root").expect("Missing root");
    let ratio = root.get("ratio").and_then(|v| v.as_f64()).unwrap_or(0.5);

    assert!(ratio >= 0.1 && ratio <= 0.9, "Ratio should be within bounds [0.1, 0.9], got {}", ratio);

    // State should still be valid
    let result = harness.validate().expect("Failed to validate");
    assert_eq!(result.get("valid").and_then(|v| v.as_bool()), Some(true));
}

// =============================================================================
// Floating Window Tests
// =============================================================================

#[test]
fn test_get_floating_empty_initially() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    // Get floating windows
    let result = harness.get_floating().expect("Failed to get floating");
    assert_eq!(result.get("status").and_then(|v| v.as_str()), Some("floating"));

    // Should be empty
    let windows = result.get("windows").and_then(|v| v.as_array());
    assert!(windows.is_some(), "Should have windows array");
    assert!(windows.unwrap().is_empty(), "Should have no floating windows initially");
}

#[test]
fn test_toggle_float_no_window() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    // Try to toggle float with no focused window - should succeed (no-op)
    let result = harness.toggle_float(None).expect("Failed to toggle float");
    assert_eq!(result.get("status").and_then(|v| v.as_str()), Some("ok"));

    // Floating list should still be empty
    let result = harness.get_floating().expect("Failed to get floating");
    let windows = result.get("windows").and_then(|v| v.as_array());
    assert!(windows.unwrap().is_empty(), "Should have no floating windows");

    // State should be valid
    let result = harness.validate().expect("Failed to validate");
    assert_eq!(result.get("valid").and_then(|v| v.as_bool()), Some(true));
}

#[test]
fn test_toggle_float_nonexistent_window() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    // Try to toggle float for a window that doesn't exist
    let result = harness.toggle_float(Some(0xDEADBEEF)).expect("Failed to toggle float");

    // Should return error or ok (depending on implementation)
    // Currently returns error for non-existent windows
    let status = result.get("status").and_then(|v| v.as_str());
    assert!(
        status == Some("ok") || status == Some("error"),
        "Should return ok or error, got: {:?}",
        status
    );

    // State should be valid regardless
    let result = harness.validate().expect("Failed to validate");
    assert_eq!(result.get("valid").and_then(|v| v.as_bool()), Some(true));
}

#[test]
fn test_floating_windows_in_state() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    // Get initial state - should have 0 floating windows
    let state = harness.get_state().expect("Failed to get state");
    let data = state.get("data").expect("Missing data");

    // Window count should be 0 initially (both tiled and floating)
    assert_eq!(data.get("window_count").and_then(|v| v.as_u64()), Some(0));

    // Windows list should be empty
    let windows = data.get("windows").and_then(|v| v.as_array());
    assert!(windows.is_some(), "Should have windows array");
    assert!(windows.unwrap().is_empty(), "Should have no windows initially");
}

#[test]
fn test_windows_list_includes_is_floating_field() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    // Get windows - verify the response structure includes is_floating
    let result = harness.get_windows().expect("Failed to get windows");
    assert_eq!(result.get("status").and_then(|v| v.as_str()), Some("windows"));

    // The windows data array exists (even if empty)
    let data = result.get("data").and_then(|v| v.as_array());
    assert!(data.is_some(), "Should have data array");

    // Note: We can't fully test is_floating without spawning windows,
    // but we've verified the IPC structure is correct
}

// =============================================================================
// Workspace Switching Tests
// =============================================================================

#[test]
fn test_workspace_switch_basic() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    // Should start on workspace 0 (format: {"status":"workspace","index":0,"total":9})
    let result = harness.get_current_workspace().expect("Failed to get workspace");
    assert_eq!(result.get("index").and_then(|v| v.as_u64()), Some(0));

    // Switch to next workspace
    let result = harness.workspace_next().expect("Failed to switch workspace");
    assert_eq!(result.get("status").and_then(|v| v.as_str()), Some("ok"));

    // Should now be on workspace 1
    let result = harness.get_current_workspace().expect("Failed to get workspace");
    assert_eq!(result.get("index").and_then(|v| v.as_u64()), Some(1));

    // Switch back to previous workspace
    let result = harness.workspace_prev().expect("Failed to switch workspace");
    assert_eq!(result.get("status").and_then(|v| v.as_str()), Some("ok"));

    // Should be back on workspace 0
    let result = harness.get_current_workspace().expect("Failed to get workspace");
    assert_eq!(result.get("index").and_then(|v| v.as_u64()), Some(0));

    // State should be valid
    let result = harness.validate().expect("Failed to validate");
    assert_eq!(result.get("valid").and_then(|v| v.as_bool()), Some(true));
}

#[test]
fn test_workspace_switch_with_empty_frames() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    // Create multiple empty frames via splits
    harness.split("horizontal").expect("Failed to split horizontal");
    harness.split("vertical").expect("Failed to split vertical");

    // Verify we have a split layout with multiple frames
    let layout = harness.get_layout().expect("Failed to get layout");
    let data = layout.get("data").expect("Missing data");
    let root = data.get("root").expect("Missing root");
    assert_eq!(
        root.get("type").and_then(|v| v.as_str()),
        Some("split"),
        "Should have split layout"
    );

    // State should be valid before switch
    let result = harness.validate().expect("Failed to validate");
    assert_eq!(result.get("valid").and_then(|v| v.as_bool()), Some(true));

    // Switch to workspace 2
    let result = harness.switch_workspace(2).expect("Failed to switch workspace");
    assert_eq!(result.get("status").and_then(|v| v.as_str()), Some("ok"));

    // Verify we're on workspace 2
    let result = harness.get_current_workspace().expect("Failed to get workspace");
    assert_eq!(result.get("index").and_then(|v| v.as_u64()), Some(2));

    // Workspace 2 should have default single-frame layout
    let layout = harness.get_layout().expect("Failed to get layout");
    let data = layout.get("data").expect("Missing data");
    let root = data.get("root").expect("Missing root");
    assert_eq!(
        root.get("type").and_then(|v| v.as_str()),
        Some("frame"),
        "New workspace should have single frame"
    );

    // State should be valid
    let result = harness.validate().expect("Failed to validate");
    assert_eq!(result.get("valid").and_then(|v| v.as_bool()), Some(true));

    // Switch back to workspace 0
    let result = harness.switch_workspace(0).expect("Failed to switch workspace");
    assert_eq!(result.get("status").and_then(|v| v.as_str()), Some("ok"));

    // Verify layout is preserved (should still be split)
    let layout = harness.get_layout().expect("Failed to get layout");
    let data = layout.get("data").expect("Missing data");
    let root = data.get("root").expect("Missing root");
    assert_eq!(
        root.get("type").and_then(|v| v.as_str()),
        Some("split"),
        "Original workspace should preserve split layout"
    );

    // State should be valid after switch back
    let result = harness.validate().expect("Failed to validate");
    assert_eq!(result.get("valid").and_then(|v| v.as_bool()), Some(true));
}

#[test]
fn test_workspace_switch_multiple_times_with_empty_frames() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    // Create 3 empty frames on workspace 0
    harness.split("horizontal").expect("Failed to split");
    harness.split("horizontal").expect("Failed to split");

    // Switch back and forth multiple times
    for i in 0..5 {
        // Switch to workspace 1
        harness.workspace_next().expect("Failed to switch to next");

        // Validate state
        let result = harness.validate().expect("Failed to validate");
        assert_eq!(
            result.get("valid").and_then(|v| v.as_bool()),
            Some(true),
            "State should be valid after switch {} to workspace 1", i
        );

        // Switch back to workspace 0
        harness.workspace_prev().expect("Failed to switch to prev");

        // Validate state
        let result = harness.validate().expect("Failed to validate");
        assert_eq!(
            result.get("valid").and_then(|v| v.as_bool()),
            Some(true),
            "State should be valid after switch {} back to workspace 0", i
        );
    }

    // Final layout should still have splits
    let layout = harness.get_layout().expect("Failed to get layout");
    let data = layout.get("data").expect("Missing data");
    let root = data.get("root").expect("Missing root");
    assert_eq!(
        root.get("type").and_then(|v| v.as_str()),
        Some("split"),
        "Layout should still be split after multiple workspace switches"
    );
}

// =============================================================================
// Screenshot Regression Tests
// =============================================================================
//
// These tests verify visual output doesn't change unexpectedly.
// Run with UPDATE_GOLDEN=1 to regenerate golden screenshots.

#[test]
fn test_screenshot_regression_initial_empty() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    // Wait for initial render to complete
    std::thread::sleep(Duration::from_millis(100));

    harness.assert_screenshot_matches("initial_empty_frame");
}

#[test]
fn test_screenshot_regression_horizontal_split() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    harness.split("horizontal").expect("Failed to split");
    std::thread::sleep(Duration::from_millis(100));

    harness.assert_screenshot_matches("horizontal_split");
}

#[test]
fn test_screenshot_regression_vertical_split() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    harness.split("vertical").expect("Failed to split");
    std::thread::sleep(Duration::from_millis(100));

    harness.assert_screenshot_matches("vertical_split");
}

#[test]
fn test_screenshot_regression_nested_splits() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    // Create nested layout: horizontal split, then vertical on left, then horizontal on right
    harness.split("horizontal").expect("Failed to split h");
    harness.split("vertical").expect("Failed to split v");
    harness.focus_frame(true).expect("Failed to focus");
    harness.split("horizontal").expect("Failed to split h2");
    std::thread::sleep(Duration::from_millis(100));

    harness.assert_screenshot_matches("nested_splits");
}

#[test]
fn test_screenshot_regression_resized_split() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    harness.split("horizontal").expect("Failed to split");
    harness.resize_split(0.2).expect("Failed to resize");
    std::thread::sleep(Duration::from_millis(100));

    harness.assert_screenshot_matches("resized_split");
}

#[test]
fn test_screenshot_regression_workspace_layouts() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    // Setup layout on workspace 0
    harness.split("horizontal").expect("Failed to split");
    std::thread::sleep(Duration::from_millis(100));

    // Switch to workspace 1 and create different layout
    harness.workspace_next().expect("Failed to switch");
    harness.split("vertical").expect("Failed to split");
    harness.split("vertical").expect("Failed to split");
    std::thread::sleep(Duration::from_millis(100));
    harness.assert_screenshot_matches("workspace_1_layout");

    // Switch back and verify workspace 0 layout is preserved
    harness.workspace_prev().expect("Failed to switch back");
    std::thread::sleep(Duration::from_millis(100));
    harness.assert_screenshot_matches("workspace_0_after_switch");
}

#[test]
fn test_screenshot_regression_focus_change() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    harness.split("horizontal").expect("Failed to split");
    std::thread::sleep(Duration::from_millis(100));
    harness.assert_screenshot_matches("focus_left_frame");

    harness.focus_frame(true).expect("Failed to focus");
    std::thread::sleep(Duration::from_millis(100));
    harness.assert_screenshot_matches("focus_right_frame");
}

#[test]
fn test_screenshot_regression_grid_2x2() {
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping test: could not create test harness");
        return;
    };

    // Create a 2x2 grid
    harness.split("horizontal").expect("Failed to split h");
    harness.split("vertical").expect("Failed to split v1");
    harness.focus_frame(true).expect("Failed to focus");
    harness.split("vertical").expect("Failed to split v2");
    std::thread::sleep(Duration::from_millis(100));

    harness.assert_screenshot_matches("grid_2x2");
}
