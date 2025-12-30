# State Machines in ttwm

This document describes the state machines that govern window management behavior in ttwm. Understanding these state machines helps with debugging and ensures consistent behavior.

## Window Lifecycle State Machine

Each managed window goes through these states:

```
                MapRequest
                    │
                    ▼
               ┌─────────┐
               │ Pending │  (brief, during manage_window())
               └────┬────┘
                    │
                    ▼
               ┌─────────┐
           ┌───│ Visible │◄──┐
           │   └────┬────┘   │
           │        │        │
           │  tab switch     │ tab switch back
           │        │        │
           │        ▼        │
           │   ┌─────────┐   │
           │   │ Hidden  │───┘
           │   └────┬────┘
           │        │
           └────┬───┘
                │ unmanage/destroy
                ▼
           ┌───────────┐
           │ Destroyed │
           └───────────┘
```

### States

- **Pending**: Window has sent MapRequest, we're in the process of managing it
- **Visible**: Window is the active tab in its frame, rendered on screen
- **Hidden**: Window is a background tab, unmapped but tracked in `hidden_windows`
- **Destroyed**: Window no longer exists

### Transitions

| From | To | Trigger |
|------|-----|---------|
| (none) | Pending | MapRequest event |
| Pending | Visible | manage_window() completes |
| Visible | Hidden | User switches tabs |
| Hidden | Visible | User switches back to this tab |
| Visible | Destroyed | DestroyNotify or UnmapNotify |
| Hidden | Destroyed | DestroyNotify |

## Focus State Machine

Focus can be on a window or nowhere:

```
    ┌──────────────────────────────────────────┐
    │                                          │
    ▼                                          │
┌─────────┐    focus_window(w)    ┌────────────┴───┐
│  None   │ ─────────────────────►│ Focused { w }  │
└─────────┘                       └────────┬───────┘
    ▲                                      │
    │ last window closed                   │ focus_window(other)
    └──────────────────────────────────────┘
```

### Invariants

- If `focused_window` is `Some(w)`, then `w` must exist in the layout tree
- `focused_window` should always match the focused tab in the focused frame
- Focus can only be `None` when there are no managed windows

## Frame State Machine

Frames can be empty, have one window, or have multiple (tabbed):

```
    ┌─────────────────────────────────────────────┐
    │                                             │
    ▼                                             │
┌─────────┐   add_window()   ┌──────────────┐     │
│  Empty  │ ────────────────►│ SingleWindow │     │
└────┬────┘                  └───────┬──────┘     │
     │                               │            │
     │ cleanup                       │ add_window()
     ▼                               ▼            │
  (removed)                   ┌─────────────┐     │
                              │   Tabbed    │─────┘
                              │ (2+ windows)│
                              └─────────────┘
                                     │
                                     │ remove all windows
                                     ▼
                                  Empty
```

### Tab Bar Rules

- No tab bar shown when `windows.len() == 1`
- Tab bar shown when `windows.len() >= 2`
- Tab bar is positioned at top of frame geometry
- Client window is positioned below tab bar

## Layout Tree State Machine

The layout tree structure changes through these operations:

```
           Initial State
                │
                ▼
         ┌─────────────┐
         │ Single Root │
         │   Frame     │
         └──────┬──────┘
                │
    ┌───────────┼───────────┐
    │           │           │
    ▼           ▼           ▼
  split     add_window   remove_window
    │           │           │
    ▼           ▼           ▼
┌─────────┐ ┌─────────┐ ┌─────────┐
│  Split  │ │  Frame  │ │ Cleanup │
│  Node   │ │  +Tab   │ │ Empties │
└─────────┘ └─────────┘ └─────────┘
```

### Split Invariants

- Split nodes always have exactly 2 children
- Split ratio is always in `[0.1, 0.9]`
- Frames are always leaf nodes
- Root is always valid (either a Frame or Split)

## Event Processing Flow

```
X11 Event
    │
    ▼
┌────────────────┐
│  poll_for_event│
└────────┬───────┘
         │
         ▼
┌────────────────┐
│  handle_event  │──────────────┐
└────────┬───────┘              │
         │                      │
    ┌────┴────┬────────┐        │
    │         │        │        │
    ▼         ▼        ▼        │
MapRequest  KeyPress  Focus     │
    │         │        │        │
    ▼         ▼        ▼        │
manage    handle    focus       │
window    action    window      │
    │         │        │        │
    └────┬────┴────────┘        │
         │                      │
         ▼                      │
    apply_layout◄───────────────┘
         │
         ▼
    update EWMH
```

## IPC State Access

Agents can query state via IPC:

```
ttwmctl state          # Full WM state snapshot
ttwmctl layout         # Layout tree structure
ttwmctl windows        # List of managed windows
ttwmctl focused        # Currently focused window
ttwmctl validate       # Check state invariants
```

### State Validation Checks

The `validate_state()` function checks:

1. Focused window exists in layout
2. Focused frame exists
3. Hidden windows are tracked in layout
4. Tab bar windows correspond to existing frames
5. No orphaned state references

## Debugging Tips

1. **Use `ttwmctl validate`** after any operation to check invariants
2. **Use `ttwmctl state | jq .`** to get formatted state dump
3. **Check the event log** with `ttwmctl event-log --count 50`
4. **Capture screenshots** with `ttwmctl screenshot /tmp/debug.png`

## Common State Bugs

### Orphaned Window
Window exists in X11 but not tracked in layout. Usually caused by:
- Race condition during manage_window()
- Missed MapRequest event

### Ghost Window
Window tracked in layout but doesn't exist in X11. Usually caused by:
- Missed DestroyNotify event
- Race condition during unmanage_window()

### Invalid Focus
Focus on window that doesn't exist. Usually caused by:
- Not updating focus after window destruction
- Race between focus change and window close
