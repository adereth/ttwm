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

### Tabs

**Tabs** are windows stacked within a single frame. Each tab shows a title in the tab bar at the top of the frame. Click a tab or use keyboard shortcuts to switch between tabs. The focused tab has a highlighted background color.

### Splits

**Splits** divide a frame into two smaller frames, either horizontally (side-by-side) or vertically (stacked). You can create complex layouts by splitting frames repeatedly. The gap between frames can be dragged to resize the split.

### Workspaces

ttwm provides **9 virtual workspaces** (desktops). Each workspace maintains its own independent layout tree. Cycle through workspaces to organize windows by task or project.

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
2. **Open a terminal**: Press `Mod4+x` (Super+x) to spawn your configured terminal
3. **Split the screen**: Press `Mod4+s` for horizontal split or `Mod4+v` for vertical split
4. **Open another terminal**: The new window appears in the newly focused frame
5. **Navigate between frames**: Use `Mod4+Left/Right/Up/Down` to move focus
6. **Create tabs**: Open multiple windows in the same frame to create tabs
7. **Switch tabs**: Use `Mod4+Page_Down` / `Mod4+Page_Up` or `Mod4+1-9`

---

## Keyboard Shortcuts

All keyboard shortcuts use `Mod4` (the Super/Windows key) as the primary modifier. These can be customized in your config file.

### Terminal and Control

| Shortcut | Action |
|----------|--------|
| `Mod4+x` | Spawn terminal |
| `Mod4+q` | Close focused window |
| `Mod4+Shift+q` | Quit ttwm |

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

---

## Mouse Interactions

### Tab Bar

- **Left-click on a tab**: Focus that window
- **Left-click on empty frame's tab bar**: Focus the empty frame

### Frame Area

- **Left-click in empty frame**: Focus the empty frame

### Gap Between Frames

- **Left-click and drag**: Resize the split by dragging the gap between frames

---

## Configuration

ttwm is configured through a TOML file located at `~/.config/ttwm/config.toml`.

### General Settings

```toml
[general]
# Terminal emulator to spawn with Mod4+x
terminal = "alacritty"
```

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
spawn_terminal = "Mod4+Return"     # Change to Mod4+Return
close_window = "Mod4+Shift+c"      # Change to Mod4+Shift+c
split_horizontal = "Mod4+h"        # Change to Mod4+h
split_vertical = "Mod4+v"          # Keep as Mod4+v
```

All available keybinding options:
- `spawn_terminal`
- `cycle_tab_forward`, `cycle_tab_backward`
- `focus_tab_1` through `focus_tab_9`
- `focus_next`, `focus_prev`
- `focus_frame_left`, `focus_frame_right`, `focus_frame_up`, `focus_frame_down`
- `move_window_left`, `move_window_right`
- `resize_shrink`, `resize_grow`
- `split_horizontal`, `split_vertical`
- `close_window`, `quit`
- `workspace_next`, `workspace_prev`

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
