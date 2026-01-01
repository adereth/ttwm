# ttwm Developer Guide

This guide covers the architecture, design decisions, and implementation details of ttwm for developers who want to understand, modify, or contribute to the codebase.

## Table of Contents

1. [Architecture Overview](#architecture-overview)
2. [Source Code Structure](#source-code-structure)
3. [Key Data Structures](#key-data-structures)
4. [Event Handling](#event-handling)
5. [Layout System](#layout-system)
6. [Tab Bar Rendering](#tab-bar-rendering)
7. [IPC Protocol](#ipc-protocol)
8. [EWMH Support](#ewmh-support)
9. [Testing](#testing)
10. [Contributing](#contributing)

---

## Architecture Overview

ttwm is a minimal X11 tiling window manager written in Rust. The architecture follows these principles:

1. **Event-driven**: The main loop processes X11 events and IPC commands
2. **Tree-based layout**: Windows are organized in a binary tree of frames and splits
3. **Tabbed containers**: Each frame can hold multiple windows as tabs
4. **Virtual desktops**: 9 independent workspaces, each with its own layout tree

### High-Level Flow

```
┌─────────────────────────────────────────────────────────────────┐
│                         X11 Server                               │
└─────────────────────────────────────────────────────────────────┘
                              ▲
                              │ X11 Events
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                          Wm (main.rs)                            │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────────┐  │
│  │ Event Loop  │──│ X11 Conn    │  │ FontRenderer (FreeType) │  │
│  └─────────────┘  └─────────────┘  └─────────────────────────┘  │
│         │                                                        │
│         ▼                                                        │
│  ┌─────────────────────────────────────────────────────────────┐│
│  │              WorkspaceManager (9 workspaces)                 ││
│  │  ┌────────────┐  ┌────────────┐       ┌────────────┐        ││
│  │  │ Workspace 1│  │ Workspace 2│  ...  │ Workspace 9│        ││
│  │  │ LayoutTree │  │ LayoutTree │       │ LayoutTree │        ││
│  │  └────────────┘  └────────────┘       └────────────┘        ││
│  └─────────────────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────────────────┘
                              ▲
                              │ IPC (Unix Socket)
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                     ttwmctl / External Tools                     │
└─────────────────────────────────────────────────────────────────┘
```

---

## Source Code Structure

```
src/
├── main.rs        # Core WM logic, event handling, rendering
├── layout.rs      # Layout tree data structure, geometry calculations
├── config.rs      # Configuration parsing, keybinding management
├── ipc.rs         # IPC server, command/response types
├── state.rs       # State machine types, validation
├── tracing.rs     # Event logging for debugging
└── bin/
    └── ttwmctl.rs # CLI control tool
```

### main.rs (~2800 lines)

The main window manager implementation containing:

- **`Wm` struct**: Core state machine holding X11 connection, configuration, workspaces, tab bar windows, and font renderer
- **Event handlers**: `handle_map_request()`, `handle_destroy_notify()`, `handle_key_press()`, `handle_button_press()`, `handle_motion_notify()`, `handle_enter_notify()`
- **Layout application**: `apply_layout()` configures window geometries and visibility
- **Tab bar rendering**: `draw_tab_bar()` renders tabs using FreeType
- **EWMH setup**: `setup_ewmh()` registers window manager atoms
- **IPC handling**: `handle_ipc_command()` processes commands from ttwmctl

### layout.rs (~1600 lines)

The layout tree and geometry system:

- **`LayoutTree`**: Binary tree using SlotMap for arena allocation
- **`Node` enum**: `Frame` (leaf with windows) or `Split` (internal with two children)
- **Tree operations**: `add_window()`, `remove_window()`, `split_focused()`, `cycle_tab()`
- **Geometry**: `calculate_geometries()` computes frame positions recursively
- **Spatial navigation**: `find_frame_in_direction()` for arrow-key focus

### config.rs (~420 lines)

Configuration parsing and keybinding management:

- **`Config`**: Top-level TOML structure
- **`parse_keybindings()`**: Converts string bindings to X11 keysyms
- **`WmAction` enum**: All possible window manager actions
- **Default values**: Fallbacks when config options are missing

### ipc.rs (~370 lines)

IPC server and protocol:

- **`IpcCommand`**: Query and action commands
- **`IpcResponse`**: Response types with serde serialization
- **`IpcServer`**: Non-blocking Unix socket listener
- **Snapshot types**: `WmStateSnapshot`, `LayoutSnapshot`, `WindowInfo`

### state.rs (~240 lines)

State machine definitions for validation:

- **`WindowState`**: Pending, Visible, Hidden, Destroying
- **`StateTransition`**: Events like WindowManaged, FocusChanged
- **`ViolationKind`**: State invariant violations

### tracing.rs (~50 lines)

Event logging:

- **`EventTracer`**: Collects recent events with timestamps
- Used for debugging via IPC `get_event_log` command

### bin/ttwmctl.rs (~200 lines)

CLI tool using Clap:

- Subcommands for all IPC operations
- Pretty JSON output formatting
- Connects to socket and sends/receives JSON

---

## Key Data Structures

### LayoutTree

The layout tree is a binary tree stored in a SlotMap arena:

```rust
pub struct LayoutTree {
    nodes: SlotMap<NodeId, Node>,  // Arena storage
    root: NodeId,                   // Root of the tree
    pub focused: NodeId,            // Currently focused frame
}
```

### Node

Each node is either a Frame (leaf) or Split (internal):

```rust
pub enum Node {
    Frame(Frame),
    Split(Split),
}

pub struct Frame {
    pub windows: Vec<Window>,      // Windows as tabs
    pub focused_tab: usize,        // Index of visible tab
    parent: Option<NodeId>,        // Parent split
}

pub struct Split {
    pub direction: SplitDirection, // Horizontal or Vertical
    pub ratio: f32,                // 0.0 to 1.0 split ratio
    pub first: NodeId,             // Left/top child
    pub second: NodeId,            // Right/bottom child
    parent: Option<NodeId>,
}
```

### Wm

The main window manager state:

```rust
pub struct Wm<'a> {
    conn: &'a RustConnection,      // X11 connection
    root: Window,                  // Root window
    screen_num: usize,
    config: LayoutConfig,          // Loaded configuration
    workspaces: WorkspaceManager,  // 9 workspaces
    windows: HashMap<Window, WindowInfo>,
    tab_bar_windows: HashMap<NodeId, Window>,
    font_renderer: FontRenderer,
    // ...
}
```

### WorkspaceManager

Manages virtual desktops:

```rust
pub struct WorkspaceManager {
    workspaces: [Workspace; 9],
    current: usize,
}

pub struct Workspace {
    pub layout: LayoutTree,
}
```

---

## Event Handling

### Event Loop

The main loop in `Wm::run()`:

```rust
loop {
    // Poll for IPC commands (non-blocking)
    if let Some((cmd, client)) = self.ipc_server.poll() {
        self.handle_ipc_command(cmd, client)?;
    }

    // Wait for X11 event
    let event = self.conn.wait_for_event()?;

    match event {
        Event::MapRequest(e) => self.handle_map_request(e)?,
        Event::DestroyNotify(e) => self.handle_destroy_notify(e)?,
        Event::KeyPress(e) => self.handle_key_press(e)?,
        Event::ButtonPress(e) => self.handle_button_press(e)?,
        Event::MotionNotify(e) => self.handle_motion_notify(e)?,
        Event::EnterNotify(e) => self.handle_enter_notify(e)?,
        Event::ConfigureRequest(e) => self.handle_configure_request(e)?,
        Event::ClientMessage(e) => self.handle_client_message(e)?,
        // ...
    }
}
```

### Event Flow: New Window

```
MapRequest event
       │
       ▼
handle_map_request()
       │
       ├─► Check if already managed
       │
       ├─► Get window attributes and hints
       │
       ├─► layout.add_window(window) ──► Adds to focused frame
       │
       ├─► Configure window geometry
       │
       ├─► Set focus if appropriate
       │
       └─► apply_layout() ──► Render all tab bars
```

### Event Flow: Key Press

```
KeyPress event
       │
       ▼
handle_key_press()
       │
       ├─► Look up keysym from keycode
       │
       ├─► Match against configured bindings
       │
       ▼
Dispatch to action:
  ├─► SplitHorizontal ──► layout.split_focused()
  ├─► FocusFrameLeft ──► focus_spatial(Direction::Left)
  ├─► CycleTabForward ──► layout.cycle_tab(true)
  ├─► CloseWindow ──► Send WM_DELETE_WINDOW or XKillClient
  └─► ... etc
       │
       ▼
apply_layout() ──► Update window positions and tab bars
```

---

## Layout System

### Binary Tree Structure

The layout is a binary tree where:
- **Leaves** (Frames) contain windows
- **Internal nodes** (Splits) divide space between two children

Example layout:
```
         Split (H, 0.5)
        /              \
   Frame A         Split (V, 0.5)
   [win1, win2]   /            \
               Frame B       Frame C
               [win3]        [win4, win5]
```

### Geometry Calculation

`calculate_geometries()` recursively computes positions:

```rust
fn calculate_geometries_recursive(
    &self,
    node_id: NodeId,
    available: Rect,
    gap: u32,
    geometries: &mut Vec<(NodeId, Rect)>,
) {
    match self.get(node_id) {
        Some(Node::Frame(_)) => {
            geometries.push((node_id, available));
        }
        Some(Node::Split(split)) => {
            let (first_rect, second_rect) = split_rect(
                available,
                split.direction,
                split.ratio,
                gap,
            );
            self.calculate_geometries_recursive(split.first, first_rect, gap, geometries);
            self.calculate_geometries_recursive(split.second, second_rect, gap, geometries);
        }
    }
}
```

### Spatial Navigation

`find_frame_in_direction()` finds the closest frame in a direction:

```rust
fn find_frame_in_direction(&self, from: NodeId, direction: Direction) -> Option<NodeId> {
    let geometries = self.calculate_geometries(screen, gap);
    let from_rect = geometries.get(&from)?;
    let from_center = (from_rect.x + from_rect.width/2, from_rect.y + from_rect.height/2);

    // Find frames in the correct direction
    let candidates: Vec<_> = geometries.iter()
        .filter(|(id, rect)| {
            match direction {
                Direction::Left => rect.x + rect.width <= from_rect.x,
                Direction::Right => rect.x >= from_rect.x + from_rect.width,
                // ... Up, Down
            }
        })
        .collect();

    // Return closest by center-to-center distance
    candidates.into_iter()
        .min_by_key(|(_, rect)| distance(from_center, rect_center(rect)))
        .map(|(id, _)| *id)
}
```

---

## Tab Bar Rendering

### Font Rendering Pipeline

1. **FreeType initialization**: Load font face from system fonts
2. **Glyph rendering**: Render each character to bitmap with anti-aliasing
3. **Buffer composition**: Composite glyphs onto BGRA pixel buffer
4. **X11 upload**: Use `put_image()` with ZPixmap format

```rust
impl FontRenderer {
    fn render_text(&self, text: &str, x: i32, y: i32, color: u32, buffer: &mut [u8], width: u32) {
        let face = self.library.face()?;
        for ch in text.chars() {
            face.load_char(ch, LoadFlag::RENDER)?;
            let glyph = face.glyph();
            let bitmap = glyph.bitmap();
            // Copy bitmap pixels to buffer at (x, y)
            // Blend using alpha from bitmap
        }
    }
}
```

### Tab Bar Layout

Each tab bar is an offscreen X11 window:

```
┌─────────────────────────────────────────────────┐
│[Tab 1 ][Tab 2 (focused)][Tab 3 ]                │
└─────────────────────────────────────────────────┘
  ▲         ▲                  ▲                ▲
  │         │                  │                │
unfocused  focused           unfocused       empty space
  bg        bg + accent        bg
```

Tab widths are calculated based on text length with min/max constraints (80-200px).

---

## IPC Protocol

### Socket Location

Socket path: `/tmp/ttwm$DISPLAY.sock`

Example: For `DISPLAY=:0`, socket is `/tmp/ttwm_0.sock`

### Message Format

One JSON message per line, newline-terminated.

### Command Examples

```json
{"command": "get_state"}
{"command": "focus_window", "window": 12345678}
{"command": "split", "direction": "horizontal"}
{"command": "focus_frame", "direction": "left"}
{"command": "resize_split", "delta": 0.05}
```

### Response Examples

```json
{"status": "ok"}
{"status": "error", "code": "invalid_window", "message": "Window not found"}
{"status": "state", "data": {...}}
```

### Adding New Commands

1. Add variant to `IpcCommand` enum in `ipc.rs`:
```rust
#[derive(Deserialize)]
pub enum IpcCommand {
    // ...
    MyNewCommand { param: String },
}
```

2. Add variant to `IpcResponse` if needed

3. Handle in `handle_ipc_command()` in `main.rs`:
```rust
IpcCommand::MyNewCommand { param } => {
    // Implement logic
    client.respond(IpcResponse::Ok)?;
}
```

4. Add CLI subcommand in `ttwmctl.rs`

---

## EWMH Support

ttwm implements core EWMH (Extended Window Manager Hints) atoms:

### Supported Atoms

| Atom | Purpose |
|------|---------|
| `_NET_SUPPORTED` | Lists all supported atoms |
| `_NET_WM_NAME` | Window manager name ("ttwm") |
| `_NET_CLIENT_LIST` | List of managed window IDs |
| `_NET_ACTIVE_WINDOW` | Currently focused window |
| `_NET_NUMBER_OF_DESKTOPS` | Number of workspaces (9) |
| `_NET_CURRENT_DESKTOP` | Current workspace index |
| `_NET_DESKTOP_NAMES` | Workspace names |
| `_NET_WM_STATE` | Window state (hidden, etc.) |

### Atom Registration

Atoms are registered in `setup_ewmh()`:

```rust
fn setup_ewmh(&self) -> Result<()> {
    let atoms = &self.atoms;

    // Set supported atoms list
    self.conn.change_property32(
        PropMode::REPLACE,
        self.root,
        atoms._NET_SUPPORTED,
        AtomEnum::ATOM,
        &[
            atoms._NET_WM_NAME,
            atoms._NET_CLIENT_LIST,
            // ...
        ],
    )?;

    // Set WM name
    self.conn.change_property8(
        PropMode::REPLACE,
        self.root,
        atoms._NET_WM_NAME,
        AtomEnum::STRING,
        b"ttwm",
    )?;

    Ok(())
}
```

---

## Testing

### Running Tests

```bash
# Run all tests
cargo test

# Run with output
cargo test -- --nocapture

# Run specific test
cargo test test_split_horizontal
```

### Test Coverage

The codebase includes 80+ tests covering:

**layout.rs tests**:
- Tree creation and basic operations
- Splitting (horizontal/vertical)
- Geometry calculations
- Spatial navigation
- Window/tab management
- Empty frame cleanup
- Resize operations

**config.rs tests**:
- Keybinding parsing
- Color parsing
- Default configuration loading

**ipc.rs tests**:
- Command serialization/deserialization
- Response serialization

### Test Patterns

Layout tests typically:
1. Create a layout tree
2. Perform operations (add windows, split, etc.)
3. Assert tree structure and geometry

```rust
#[test]
fn test_split_horizontal() {
    let mut tree = LayoutTree::new();
    tree.add_window(1);
    tree.split_focused(SplitDirection::Horizontal);

    // Root should now be a split
    assert!(tree.get(tree.root).unwrap().as_split().is_some());

    // Should have two frames
    let frames = tree.collect_frames();
    assert_eq!(frames.len(), 2);
}
```

---

## Contributing

### Code Style

- Follow Rust idioms and standard formatting (`cargo fmt`)
- Use `clippy` for linting (`cargo clippy`)
- Add tests for new functionality
- Document public APIs with doc comments

### Pull Request Process

1. Fork the repository
2. Create a feature branch
3. Make changes with tests
4. Ensure `cargo test` passes
5. Ensure `cargo clippy` has no warnings
6. Submit PR with clear description

### Areas for Contribution

- Additional EWMH support (fullscreen, urgent hints)
- Floating window support
- Multi-monitor improvements
- Animation/transitions
- Additional layout algorithms
- Documentation improvements

### Debugging Tips

Enable logging with:
```bash
RUST_LOG=debug ttwm
```

Log levels:
- `error`: Critical errors
- `warn`: Warnings
- `info`: Key events (window managed, focus changed)
- `debug`: Detailed event flow
- `trace`: Very verbose (X11 protocol details)

Use IPC for debugging:
```bash
ttwmctl validate      # Check state invariants
ttwmctl event-log     # View recent events
ttwmctl state         # Dump full state
```
