# ttwm User Guide

ttwm (Tabbed Tiling Window Manager) is a minimal X11 tiling window manager inspired by [Notion](https://notionwm.net/). It combines the efficiency of tiling layouts with the organization of tabbed windows, allowing multiple windows to share the same screen space as tabs within frames.

## Table of Contents

1. [Core Concepts](#core-concepts)
2. [Installation](#installation)
3. [Quick Start](#quick-start)
4. [Keyboard Shortcuts](#keyboard-shortcuts)
5. [Mouse Interactions](#mouse-interactions)
6. [Configuration](#configuration)
7. [IPC and ttwmctl](#ipc-and-ttwmctl)
8. [Troubleshooting](#troubleshooting)

---

## Core Concepts

### Frames

A **frame** is a rectangular area on the screen that can hold one or more windows. When a frame contains multiple windows, they are displayed as tabs at the top of the frame. Only one window (the focused tab) is visible at a time; the others are hidden but remain managed.

**Empty frames** (created by splitting) display a bordered placeholder in the content area. The border color indicates focus state: blue when focused, gray when unfocused. Click on an empty frame to focus it, then open a new window to place it there.

### Tabs

**Tabs** are windows stacked within a single frame. Each tab shows a title in the tab bar at the top of the frame. Click a tab or use keyboard shortcuts to switch between tabs. The focused tab has a highlighted background color.

**Vertical tabs** can be enabled per-frame with `Mod4+/`. In vertical mode, tabs appear on the left side of the frame and display only the application icon (no text). This is useful for frames with many windows where you want to maximize horizontal space. Toggle back to horizontal tabs with the same shortcut.

### Splits

**Splits** divide a frame into two smaller frames, either horizontally (side-by-side) or vertically (stacked). You can create complex layouts by splitting frames repeatedly. The gap between frames can be dragged to resize the split.

### Workspaces

ttwm provides **9 virtual workspaces** (desktops). Each workspace maintains its own independent layout tree. Cycle through workspaces to organize windows by task or project.

### Floating Windows

**Floating windows** are exempt from the tiling layout. They render above tiled windows and can be freely moved and resized with the mouse. Some window types automatically float:

- Dialog boxes
- Splash screens
- Toolbars and utility windows
- Menus and tooltips

You can manually toggle any window between tiled and floating mode with `Mod4+f`. Floating windows are per-workspace (hidden when you switch workspaces).

### Urgent Windows

**Urgent windows** are windows that request attention using the `_NET_WM_STATE_DEMANDS_ATTENTION` hint. This is typically triggered by:

- Terminal bell (`echo -e '\a'`)
- Chat applications receiving messages
- Download completion notifications
- Any application requesting user attention

When a window becomes urgent:
- Its tab turns **orange/amber** (configurable via `tab_urgent_bg`)
- If the urgent window is on another workspace, a small **orange indicator** appears in the upper-right corner of the screen

**Clearing urgent state:**
- Focus the urgent window (the orange highlight clears automatically)
- Use `Mod4+Space` to jump to the oldest urgent window

Urgent windows are handled in FIFO order (first-in, first-out), so `Mod4+Space` always focuses the window that has been waiting longest for attention.

### Multi-Monitor Support

ttwm supports **multiple monitors** with per-monitor workspaces (similar to i3/bspwm). Each monitor maintains its own independent set of 9 workspaces.

**Key features:**
- Each monitor has its own 9 workspaces
- Workspace switching only affects the currently focused monitor
- Use `Mod4+Control+Left/Right` to move focus between monitors
- Drag tabs to frames on other monitors to move windows
- Use tagging (`Mod4+t`) to batch-move windows between monitors

**Monitor detection:**
- Monitors are detected via RandR at startup
- RandR hotplug events are supported (connect/disconnect monitors)

---

## Installation

### Requirements

- Rust toolchain (1.70 or later)
- X11 development libraries
- FreeType development libraries

On Debian/Ubuntu:
```bash
sudo apt install build-essential libx11-dev libxcb1-dev libfreetype6-dev
```

On Arch Linux:
```bash
sudo pacman -S base-devel libx11 libxcb freetype2
```

### Building

```bash
git clone https://github.com/yourusername/ttwm.git
cd ttwm
cargo build --release
```

The binaries will be in `target/release/`:
- `ttwm` - The window manager
- `ttwmctl` - The control CLI tool

### Starting ttwm

**From a display manager**: Add a desktop entry or select ttwm from your display manager's session menu.

**From .xinitrc**:
```bash
exec /path/to/ttwm
```

**Configuration**: Copy the example config to your home directory:
```bash
mkdir -p ~/.config/ttwm
cp config.toml.example ~/.config/ttwm/config.toml
```

---

## Quick Start

1. **Start ttwm** and you'll see an empty screen
2. **Open a terminal**: Press `Mod4+x` (Super+x) to spawn alacritty (configurable via `[exec]` section)
3. **Split the screen**: Press `Mod4+s` for horizontal split or `Mod4+v` for vertical split
4. **Open another terminal**: The new window appears in the newly focused frame
5. **Navigate between frames**: Use `Mod4+Left/Right/Up/Down` to move focus
6. **Create tabs**: Open multiple windows in the same frame to create tabs
7. **Switch tabs**: Use `Mod4+Page_Down` / `Mod4+Page_Up` or `Mod4+1-9`

---

## Keyboard Shortcuts

All keyboard shortcuts use `Mod4` (the Super/Windows key) as the primary modifier. These can be customized in your config file.

### Exec Bindings (default)

| Shortcut | Action |
|----------|--------|
| `Mod4+x` | Spawn alacritty |
| `Mod4+r` | Run gmrun |

### Window Control

| Shortcut | Action |
|----------|--------|
| `Mod4+q` | Close focused window |
| `Mod4+f` | Toggle floating mode for focused window |
| `Mod4+/` | Toggle vertical tabs for focused frame |
| `Mod4+Control+F4` | Quit ttwm |

### Tab Navigation

| Shortcut | Action |
|----------|--------|
| `Mod4+Page_Down` | Focus next tab |
| `Mod4+Page_Up` | Focus previous tab |
| `Mod4+1` through `Mod4+9` | Focus tab by number |

### Window Focus

| Shortcut | Action |
|----------|--------|
| `Mod4+j` | Focus next window (linear) |
| `Mod4+k` | Focus previous window (linear) |

### Frame Navigation (Spatial)

| Shortcut | Action |
|----------|--------|
| `Mod4+Left` | Focus frame to the left |
| `Mod4+Right` | Focus frame to the right |
| `Mod4+Up` | Focus frame above |
| `Mod4+Down` | Focus frame below |

### Splitting

| Shortcut | Action |
|----------|--------|
| `Mod4+s` | Split current frame horizontally |
| `Mod4+v` | Split current frame vertically |

### Moving Windows

| Shortcut | Action |
|----------|--------|
| `Mod4+Shift+Left` | Move window to frame on left |
| `Mod4+Shift+Right` | Move window to frame on right |

### Resizing Splits

| Shortcut | Action |
|----------|--------|
| `Mod4+Control+Left` | Shrink focused split |
| `Mod4+Control+Right` | Grow focused split |

### Workspaces

| Shortcut | Action |
|----------|--------|
| `Mod4+]` | Switch to next workspace |
| `Mod4+[` | Switch to previous workspace |

### Tagging (Batch Operations)

| Shortcut | Action |
|----------|--------|
| `Mod4+t` | Toggle tag on focused window |
| `Mod4+a` | Move all tagged windows to focused frame |
| `Mod4+Shift+t` | Untag all windows |

### Urgent Windows

| Shortcut | Action |
|----------|--------|
| `Mod4+Space` | Focus oldest urgent window (jumps to other workspace if needed) |

### Monitor Navigation

| Shortcut | Action |
|----------|--------|
| `Mod4+Control+Left` | Focus monitor to the left |
| `Mod4+Control+Right` | Focus monitor to the right |

---

## Mouse Interactions

### Tab Bar

- **Left-click on a tab**: Focus that window
- **Left-click on empty frame's tab bar**: Focus the empty frame

### Frame Area

- **Left-click in empty frame**: Focus the empty frame

### Gap Between Frames

- **Left-click and drag**: Resize the split by dragging the gap between frames

### Floating Windows

- **Left-click inside a floating window**: Focus the window
- **Left-click and drag inside a floating window**: Move the window
- **Left-click and drag on the edge/corner of a floating window**: Resize the window

Floating windows have an 8-pixel resize zone around their edges. The cursor will change to indicate resize direction.

---

## Configuration

ttwm is configured through a TOML file located at `~/.config/ttwm/config.toml`.

### Appearance Settings

```toml
[appearance]
# Gap between windows (pixels)
gap = 8

# Gap from screen edges (pixels)
outer_gap = 8

# Window border width (pixels)
border_width = 2

# Tab bar height (pixels)
tab_bar_height = 26

# Tab bar font (fontconfig name)
tab_font = "Segoe UI"

# Tab bar font size in points
tab_font_size = 12

# Show application icons in tabs (20x20 pixels)
show_tab_icons = true

# Vertical tab bar width (pixels) - icons only, no text
vertical_tab_width = 28
```

### Color Settings

All colors are specified in hex format (`#RRGGBB`):

```toml
[colors]
# Tab bar background (uses pseudo-transparency from desktop wallpaper)
tab_bar_bg = "#000000"

# Focused tab background
tab_focused_bg = "#5294e2"

# Unfocused tab background (same frame)
tab_unfocused_bg = "#3a3a3a"

# Visible tab in an unfocused frame
tab_visible_unfocused_bg = "#4a6a9a"

# Tagged window tab background
tab_tagged_bg = "#e06c75"

# Text color for focused tabs
tab_text = "#ffffff"

# Text color for unfocused tabs
tab_text_unfocused = "#888888"

# Separator line between inactive tabs
tab_separator = "#4a4a4a"

# Border color for focused window
border_focused = "#5294e2"

# Border color for unfocused windows
border_unfocused = "#3a3a3a"
```

### Keybinding Settings

Override default keybindings in the `[keybindings]` section. Format: `"Modifier+Key"`

Available modifiers: `Mod4` (Super), `Shift`, `Control`, `Alt`

```toml
[keybindings]
# Examples
close_window = "Mod4+Shift+c"      # Change to Mod4+Shift+c
split_horizontal = "Mod4+h"        # Change to Mod4+h
split_vertical = "Mod4+v"          # Keep as Mod4+v
```

All available keybinding options:
- `cycle_tab_forward`, `cycle_tab_backward`
- `focus_tab_1` through `focus_tab_9`
- `focus_next`, `focus_prev`
- `focus_frame_left`, `focus_frame_right`, `focus_frame_up`, `focus_frame_down`
- `move_window_left`, `move_window_right`
- `resize_shrink`, `resize_grow`
- `split_horizontal`, `split_vertical`
- `close_window`, `toggle_float`, `toggle_vertical_tabs`, `quit`
- `workspace_next`, `workspace_prev`
- `tag_window`, `move_tagged_windows`, `untag_all`
- `focus_monitor_left`, `focus_monitor_right`

### Exec Settings

Run programs with keybindings using the `[exec]` section. Format: `"Modifier+Key" = "command [args...]"`

```toml
[exec]
# Run alacritty terminal with Mod4+x
"Mod4+x" = "alacritty"
# Run gmrun launcher with Mod4+r
"Mod4+r" = "gmrun"
# Run htop in alacritty with Mod4+Shift+x
"Mod4+Shift+x" = "alacritty -e htop"
```

---

## IPC and ttwmctl

ttwm provides a Unix socket IPC interface for external control and scripting. The `ttwmctl` command-line tool communicates with the running window manager.

### Socket Location

The socket is created at `/tmp/ttwm$DISPLAY.sock` (e.g., `/tmp/ttwm_0.sock` for display `:0`).

### Common Commands

```bash
# Get full WM state as JSON
ttwmctl state

# Get layout tree
ttwmctl layout

# List all windows
ttwmctl windows

# Get focused window ID
ttwmctl focused

# Focus a specific window by ID
ttwmctl focus 0x1c00004

# Focus tab by index (1-9)
ttwmctl focus-tab 2

# Focus frame in a direction
ttwmctl focus-frame left

# Split the focused frame
ttwmctl split horizontal
ttwmctl split vertical

# Move window to adjacent frame
ttwmctl move-window forward
ttwmctl move-window backward

# Resize the focused split
ttwmctl resize grow
ttwmctl resize shrink

# Close focused window
ttwmctl close

# Cycle tabs
ttwmctl cycle-tab forward
ttwmctl cycle-tab backward

# Tagging commands
ttwmctl tag                    # Tag focused window
ttwmctl tag 0x1c00004          # Tag specific window
ttwmctl untag                  # Untag focused window
ttwmctl toggle-tag             # Toggle tag on focused window
ttwmctl move-tagged            # Move all tagged to focused frame
ttwmctl untag-all              # Untag all windows
ttwmctl tagged                 # List tagged window IDs

# Floating window commands
ttwmctl toggle-float           # Toggle floating for focused window
ttwmctl toggle-float 0x1c00004 # Toggle floating for specific window
ttwmctl floating               # List floating window IDs

# Urgent window commands
ttwmctl urgent                 # List urgent window IDs (oldest first)
ttwmctl focus-urgent           # Focus oldest urgent window

# Workspace commands
ttwmctl workspace 3            # Switch to workspace 3
ttwmctl workspace next         # Switch to next workspace
ttwmctl workspace prev         # Switch to previous workspace
ttwmctl current-workspace      # Get current workspace number
ttwmctl move-to-workspace 2    # Move focused window to workspace 2
ttwmctl move-to-workspace 2 --window 0x1c00004  # Move specific window

# Monitor commands
ttwmctl monitors               # List all monitors with geometry and state
ttwmctl current-monitor        # Get currently focused monitor name
ttwmctl focus-monitor DP-1     # Focus a specific monitor by name
ttwmctl focus-monitor left     # Focus monitor to the left
ttwmctl focus-monitor right    # Focus monitor to the right

# Validate WM state (for debugging)
ttwmctl validate

# Get recent event log
ttwmctl event-log

# Quit the window manager
ttwmctl quit
```

### Scripting Examples

**Focus a window by title pattern**:
```bash
# Find and focus Firefox
window_id=$(ttwmctl windows | jq -r '.data[] | select(.title | contains("Firefox")) | .id')
ttwmctl focus "$window_id"
```

**Create a three-column layout**:
```bash
ttwmctl split horizontal
ttwmctl focus-frame right
ttwmctl split horizontal
```

---

## Troubleshooting

### Window manager doesn't start

- Check that no other window manager is running
- Verify X11 is running and DISPLAY is set correctly
- Check logs: `RUST_LOG=info ttwm 2>&1 | tee ttwm.log`

### Keybindings don't work

- Ensure no other application is grabbing the same key combinations
- Check config file syntax with a TOML validator
- Verify the config file location: `~/.config/ttwm/config.toml`

### Font rendering issues

- Install the specified font or use a system font like "monospace"
- Try different font names: "DejaVu Sans", "Liberation Sans", "FreeSans"

### IPC commands fail

- Verify ttwm is running
- Check the socket exists: `ls -la /tmp/ttwm*.sock`
- Ensure DISPLAY environment variable is set correctly

### Windows not tiling

- Some windows request specific sizes; ttwm respects minimum size hints
- Dialog windows may float or overlap
- Check window class/type with `xprop` to understand window behavior

---

## Tips

1. **Use workspaces**: Keep related windows on the same workspace (e.g., browser + docs on workspace 1, terminal + editor on workspace 2)

2. **Tab heavy workflows**: Put related windows as tabs in the same frame rather than splitting many times

3. **Quick splits**: After splitting, the new empty frame is focused, so just open your next application

4. **Resize with mouse**: For fine-grained control, drag the gaps between frames

5. **Script common layouts**: Use ttwmctl to create scripts for your preferred layouts
