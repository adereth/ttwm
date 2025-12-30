# Agentic Testing Guide for ttwm

This document describes the testing infrastructure designed for coding agents to autonomously debug and test ttwm without human intervention.

## Overview

ttwm provides several capabilities for programmatic testing:

1. **IPC Interface** - Unix socket with JSON protocol for state queries and commands
2. **CLI Tool (`ttwmctl`)** - Command-line interface for scripting
3. **State Validation** - Invariant checking to detect bugs
4. **Event Tracing** - Ring buffer of recent events for debugging
5. **Screenshot Capture** - Visual verification of layout state
6. **Integration Tests** - Xvfb-based test harness

## Quick Start

```bash
# Build ttwm and ttwmctl
cargo build --release

# Start ttwm (in an X session)
./target/release/ttwm &

# Query state
./target/release/ttwmctl state | jq .

# Validate invariants
./target/release/ttwmctl validate

# Capture screenshot
./target/release/ttwmctl screenshot /tmp/debug.png
```

## IPC Protocol

### Socket Location

The IPC socket is created at `/tmp/ttwm{display}.sock` where `{display}` is the sanitized `$DISPLAY` value (colons and dots replaced with underscores).

Examples:
- `DISPLAY=:0` → `/tmp/ttwm_0.sock`
- `DISPLAY=:99` → `/tmp/ttwm_99.sock`
- `DISPLAY=localhost:0.0` → `/tmp/ttwm_localhost_0_0.sock`

### Protocol Format

Commands and responses are JSON objects, one per line (newline-delimited JSON).

```bash
# Example: Send command via netcat
echo '{"command": "get_state"}' | nc -U /tmp/ttwm_0.sock
```

### Available Commands

#### Query Commands

| Command | Description | Response |
|---------|-------------|----------|
| `get_state` | Full WM state snapshot | `WmStateSnapshot` |
| `get_layout` | Layout tree structure | `LayoutSnapshot` |
| `get_windows` | List of managed windows | `WindowInfo[]` |
| `get_focused` | Currently focused window | `u32 \| null` |
| `validate_state` | Check state invariants | `ValidationResult` |
| `get_event_log` | Recent events | `EventLogEntry[]` |

#### Action Commands

| Command | Parameters | Description |
|---------|------------|-------------|
| `focus_window` | `window: u32` | Focus a specific window |
| `focus_tab` | `index: usize` | Focus tab by index in current frame |
| `split` | `direction: "horizontal" \| "vertical"` | Split focused frame |
| `close_window` | - | Close focused window |
| `screenshot` | `path: string` | Save screenshot to file |
| `quit` | - | Gracefully shutdown WM |

### Command Examples

```json
// Get full state
{"command": "get_state"}

// Split horizontally
{"command": "split", "direction": "horizontal"}

// Focus specific window
{"command": "focus_window", "window": 12345678}

// Get last 50 events
{"command": "get_event_log", "count": 50}

// Capture screenshot
{"command": "screenshot", "path": "/tmp/test.png"}

// Validate state
{"command": "validate_state"}
```

### Response Format

All responses include a `status` field:

```json
// Success
{"status": "ok"}

// State response
{"status": "state", "data": {...}}

// Error
{"status": "error", "code": "invalid_command", "message": "Unknown command: foo"}

// Validation result
{"status": "validation", "valid": true, "violations": []}
```

## ttwmctl CLI Reference

### Usage

```
ttwmctl [OPTIONS] <COMMAND>

Commands:
  state       Get full WM state as JSON
  layout      Get layout tree as JSON
  windows     List all managed windows
  focused     Get currently focused window
  validate    Validate state invariants
  focus       Focus a window by ID
  split       Split the focused frame
  screenshot  Capture screenshot
  event-log   Get recent event log
  quit        Quit the window manager
  help        Print help information
```

### Options

```
-s, --socket <PATH>  Override socket path
                     Default: /tmp/ttwm{DISPLAY}.sock
```

### Examples

```bash
# Get state formatted with jq
ttwmctl state | jq .

# Get window count
ttwmctl state | jq '.data.window_count'

# List windows with titles
ttwmctl windows | jq '.windows[] | {id, title}'

# Focus a specific window
ttwmctl focus 12345678

# Split horizontally
ttwmctl split horizontal

# Validate and check result
ttwmctl validate | jq '.valid'

# Get last 20 events
ttwmctl event-log --count 20

# Capture screenshot
ttwmctl screenshot /tmp/debug.png
```

## State Validation

The `validate_state` command checks invariants that should always hold:

### Invariants Checked

| Invariant | Description |
|-----------|-------------|
| Focused window exists | If `focused_window` is set, the window exists in layout |
| Focused frame exists | The `focused_frame` ID references a valid frame |
| No orphaned hidden windows | Hidden windows are tracked in layout |
| Valid tab indices | Tab `active_tab` is within bounds |
| No empty non-root frames | Empty frames are cleaned up |

### Validation Response

```json
{
  "status": "validation",
  "valid": true,
  "violations": []
}

// Or with violations:
{
  "status": "validation",
  "valid": false,
  "violations": [
    {
      "kind": "invalid_focus",
      "description": "Focused window 12345 not found in layout"
    }
  ]
}
```

### Violation Types

| Kind | Description |
|------|-------------|
| `orphaned_window` | Window exists in X but not tracked |
| `ghost_window` | Window tracked but doesn't exist in X |
| `invalid_focus` | Focus on non-existent window |
| `empty_frame_leaked` | Empty frame not cleaned up |
| `split_ratio_out_of_bounds` | Ratio outside [0.1, 0.9] |
| `tab_index_out_of_bounds` | Tab index exceeds window count |
| `focused_frame_missing` | Focused frame doesn't exist |
| `hidden_window_orphaned` | Hidden window not in layout |
| `orphaned_tab_bar` | Tab bar for non-existent frame |

## Event Tracing

The event tracer maintains a ring buffer of recent events for debugging.

### Event Log Entry

```json
{
  "sequence": 42,
  "timestamp_ms": 15234,
  "event_type": "MapRequest",
  "window": 12345678,
  "details": "class=XTerm"
}
```

### Event Types

| Type | Description |
|------|-------------|
| `MapRequest` | Window requested mapping |
| `DestroyNotify` | Window destroyed |
| `UnmapNotify` | Window unmapped |
| `ConfigureRequest` | Window requested resize |
| `KeyPress` | Key pressed |
| `ButtonPress` | Mouse button pressed |
| `EnterNotify` | Mouse entered window |
| `window_managed` | Window added to layout |
| `window_unmanaged` | Window removed from layout |
| `focus_changed` | Focus changed |
| `tab_switched` | Tab changed in frame |
| `frame_split` | Frame was split |
| `split_resized` | Split ratio changed |
| `window_moved` | Window moved between frames |
| `frame_removed` | Empty frame cleaned up |
| `ipc_command` | IPC command received |

### Querying Events

```bash
# Get last 100 events
ttwmctl event-log --count 100 | jq .

# Filter for focus events
ttwmctl event-log --count 50 | jq '.entries[] | select(.event_type | contains("focus"))'

# Find events for specific window
ttwmctl event-log | jq '.entries[] | select(.window == 12345678)'
```

## Screenshot Capture

Screenshots are captured via X11 GetImage and saved as PNG.

```bash
# Capture full root window
ttwmctl screenshot /tmp/full.png

# Use in test script
ttwmctl screenshot /tmp/before.png
ttwmctl split horizontal
ttwmctl screenshot /tmp/after.png
```

### Notes

- Screenshots capture the root window (full display)
- Format is always PNG
- Path must be writable by the WM process
- Useful for visual regression testing

## Integration Testing

### Requirements

- Xvfb (headless X server)
- Built ttwm and ttwmctl binaries

### Running Integration Tests

```bash
# Run all tests including integration
RUST_LOG=info cargo test --test integration

# Run specific integration test
cargo test --test integration test_split_creates_two_frames
```

### Test Harness

The `TestHarness` struct manages test lifecycle:

```rust
struct TestHarness {
    xvfb: Child,      // Xvfb process
    wm: Child,        // ttwm process
    display: String,  // Display (e.g., ":99")
    socket_path: PathBuf,
}

impl TestHarness {
    fn new() -> Option<Self>;                    // Start Xvfb + ttwm
    fn send_command(&self, cmd: &Value) -> Result<Value, String>;
    fn get_state(&self) -> Result<Value, String>;
    fn split(&self, direction: &str) -> Result<Value, String>;
    fn validate(&self) -> Result<Value, String>;
    fn quit(&self) -> Result<Value, String>;
}
```

### Writing Integration Tests

```rust
#[test]
fn test_my_scenario() {
    // Create harness (skips if Xvfb unavailable)
    let Some(harness) = TestHarness::new() else {
        eprintln!("Skipping: Xvfb not available");
        return;
    };

    // Test operations
    let state = harness.get_state().expect("Failed to get state");
    assert_eq!(state["data"]["frame_count"], 1);

    // Split
    harness.split("horizontal").expect("Failed to split");

    // Verify
    let state = harness.get_state().expect("Failed to get state");
    assert_eq!(state["data"]["frame_count"], 2);

    // Validate invariants
    let validation = harness.validate().expect("Failed to validate");
    assert_eq!(validation["valid"], true);
}
```

## Agent Debugging Workflow

### 1. Reproduce the Issue

```bash
# Get initial state
ttwmctl state > /tmp/before.json
ttwmctl screenshot /tmp/before.png

# Perform operations that trigger the bug
# ...

# Capture after state
ttwmctl state > /tmp/after.json
ttwmctl screenshot /tmp/after.png
```

### 2. Check Invariants

```bash
# Run validation
result=$(ttwmctl validate)
if echo "$result" | jq -e '.valid == false' > /dev/null; then
    echo "State violation detected:"
    echo "$result" | jq '.violations'
fi
```

### 3. Examine Event History

```bash
# Get recent events
ttwmctl event-log --count 100 | jq '.entries'

# Look for the sequence leading to the bug
ttwmctl event-log | jq '.entries[-20:]'
```

### 4. Analyze State

```bash
# Check layout structure
ttwmctl layout | jq .

# List windows with details
ttwmctl windows | jq '.windows[] | {id, title, frame}'

# Check focus
ttwmctl focused
```

### 5. Test Fix

```bash
# Run integration tests
cargo test --test integration

# Or test specific scenario manually
# ...

# Verify fix with validation
ttwmctl validate | jq '.valid'
```

## State Machine Reference

See [state-machines.md](state-machines.md) for detailed documentation of:

- Window lifecycle states (Pending → Visible ↔ Hidden → Destroyed)
- Focus state machine (None ↔ Focused)
- Frame state machine (Empty → SingleWindow → Tabbed)
- Event processing flow

## Troubleshooting

### IPC Connection Failed

```
Error: Failed to connect to socket
```

- Check ttwm is running: `pgrep ttwm`
- Check socket exists: `ls -la /tmp/ttwm*.sock`
- Check DISPLAY matches: `echo $DISPLAY`

### Validation Errors

If validation reports violations:

1. Check event log for recent changes
2. Capture screenshot for visual verification
3. Compare layout state with expected state
4. Look for race conditions in event handling

### Test Harness Fails

```
Skipping: Xvfb not available
```

Install Xvfb:
```bash
# Debian/Ubuntu
sudo apt install xvfb

# Fedora
sudo dnf install xorg-x11-server-Xvfb

# Arch
sudo pacman -S xorg-server-xvfb
```

### Screenshot Capture Fails

- Ensure path is absolute and directory exists
- Check write permissions
- Verify X connection is working

## API Reference

### WmStateSnapshot

```typescript
interface WmStateSnapshot {
  focused_window: number | null;
  focused_frame: string;
  window_count: number;
  frame_count: number;
  layout: LayoutSnapshot;
}
```

### LayoutSnapshot

```typescript
type LayoutSnapshot = FrameSnapshot | SplitSnapshot;

interface FrameSnapshot {
  type: "frame";
  id: string;
  windows: number[];
  focused_tab: number;
  geometry: Rect | null;
}

interface SplitSnapshot {
  type: "split";
  id: string;
  direction: "horizontal" | "vertical";
  ratio: number;
  first: LayoutSnapshot;
  second: LayoutSnapshot;
  geometry: Rect | null;
}

interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}
```

### WindowInfo

```typescript
interface WindowInfo {
  id: number;
  title: string;
  class: string;
  frame: string;
  visible: boolean;
}
```

### EventLogEntry

```typescript
interface EventLogEntry {
  sequence: number;
  timestamp_ms: number;
  event_type: string;
  window: number | null;
  details: string;
}
```
