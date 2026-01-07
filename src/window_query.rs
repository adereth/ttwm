//! Window property query functions.
//!
//! Stateless functions for querying X11 window properties.

use anyhow::Result;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

use crate::ewmh::Atoms;
use crate::types::StrutPartial;

/// Get the window title from _NET_WM_NAME or WM_NAME.
pub fn get_window_title(conn: &impl Connection, atoms: &Atoms, window: Window) -> String {
    // Try _NET_WM_NAME first
    if let Ok(reply) = conn.get_property(
        false,
        window,
        atoms.net_wm_name,
        atoms.utf8_string,
        0,
        1024,
    ) {
        if let Ok(reply) = reply.reply() {
            if !reply.value.is_empty() {
                if let Ok(s) = String::from_utf8(reply.value) {
                    return s;
                }
            }
        }
    }

    // Fall back to WM_NAME
    if let Ok(reply) = conn.get_property(
        false,
        window,
        AtomEnum::WM_NAME,
        AtomEnum::STRING,
        0,
        1024,
    ) {
        if let Ok(reply) = reply.reply() {
            if !reply.value.is_empty() {
                if let Ok(s) = String::from_utf8(reply.value) {
                    return s;
                }
            }
        }
    }

    // Default title
    format!("0x{:x}", window)
}

/// Check if a window should float based on _NET_WM_WINDOW_TYPE.
/// Returns true for dialogs, splash screens, toolbars, utilities, menus, tooltips, notifications.
pub fn should_float(conn: &impl Connection, atoms: &Atoms, window: Window) -> bool {
    // Query the _NET_WM_WINDOW_TYPE property
    let reply = match conn.get_property(
        false,
        window,
        atoms.net_wm_window_type,
        AtomEnum::ATOM,
        0,
        1024,
    ) {
        Ok(cookie) => match cookie.reply() {
            Ok(reply) => reply,
            Err(_) => return false,
        },
        Err(_) => return false,
    };

    // Check if any window type indicates it should float
    if let Some(types) = reply.value32() {
        for window_type in types {
            if window_type == atoms.net_wm_window_type_dialog
                || window_type == atoms.net_wm_window_type_splash
                || window_type == atoms.net_wm_window_type_toolbar
                || window_type == atoms.net_wm_window_type_utility
                || window_type == atoms.net_wm_window_type_menu
                || window_type == atoms.net_wm_window_type_popup_menu
                || window_type == atoms.net_wm_window_type_dropdown_menu
                || window_type == atoms.net_wm_window_type_tooltip
                || window_type == atoms.net_wm_window_type_notification
            {
                log::info!("Window 0x{:x} should float (window type)", window);
                return true;
            }
        }
    }

    false
}

/// Check if a window is a dock (status bar like polybar).
pub fn is_dock_window(conn: &impl Connection, atoms: &Atoms, window: Window) -> bool {
    let reply = match conn.get_property(
        false,
        window,
        atoms.net_wm_window_type,
        AtomEnum::ATOM,
        0,
        1024,
    ) {
        Ok(cookie) => match cookie.reply() {
            Ok(reply) => reply,
            Err(_) => return false,
        },
        Err(_) => return false,
    };

    if let Some(types) = reply.value32() {
        for window_type in types {
            if window_type == atoms.net_wm_window_type_dock {
                return true;
            }
        }
    }
    false
}

/// Read strut partial from a window (returns Default if none set).
pub fn read_struts(conn: &impl Connection, atoms: &Atoms, window: Window) -> StrutPartial {
    // Try _NET_WM_STRUT_PARTIAL first (12 values)
    if let Ok(cookie) = conn.get_property(
        false,
        window,
        atoms.net_wm_strut_partial,
        AtomEnum::CARDINAL,
        0,
        12,
    ) {
        if let Ok(reply) = cookie.reply() {
            let values: Vec<u32> = reply.value32().map(|v| v.collect()).unwrap_or_default();
            if values.len() >= 12 {
                return StrutPartial {
                    left: values[0],
                    right: values[1],
                    top: values[2],
                    bottom: values[3],
                    left_start_y: values[4],
                    left_end_y: values[5],
                    right_start_y: values[6],
                    right_end_y: values[7],
                    top_start_x: values[8],
                    top_end_x: values[9],
                    bottom_start_x: values[10],
                    bottom_end_x: values[11],
                };
            }
        }
    }

    // Fallback to _NET_WM_STRUT (4 values)
    if let Ok(cookie) = conn.get_property(
        false,
        window,
        atoms.net_wm_strut,
        AtomEnum::CARDINAL,
        0,
        4,
    ) {
        if let Ok(reply) = cookie.reply() {
            let values: Vec<u32> = reply.value32().map(|v| v.collect()).unwrap_or_default();
            if values.len() >= 4 {
                return StrutPartial {
                    left: values[0],
                    right: values[1],
                    top: values[2],
                    bottom: values[3],
                    ..Default::default()
                };
            }
        }
    }

    StrutPartial::default()
}

/// Check if a window has the urgent hint set via _NET_WM_STATE or WM_HINTS.
pub fn is_window_urgent(conn: &impl Connection, atoms: &Atoms, window: Window) -> bool {
    // Check EWMH _NET_WM_STATE_DEMANDS_ATTENTION
    let reply = match conn.get_property(
        false,
        window,
        atoms.net_wm_state,
        AtomEnum::ATOM,
        0,
        1024,
    ) {
        Ok(cookie) => match cookie.reply() {
            Ok(reply) => Some(reply),
            Err(_) => None,
        },
        Err(_) => None,
    };

    if let Some(reply) = reply {
        if let Some(states) = reply.value32() {
            for state in states {
                if state == atoms.net_wm_state_demands_attention {
                    return true;
                }
            }
        }
    }

    // Check legacy WM_HINTS UrgencyHint flag (bit 8 = 256)
    const URGENCY_HINT: u32 = 256;
    let hints_reply = match conn.get_property(
        false,
        window,
        AtomEnum::WM_HINTS,
        AtomEnum::WM_HINTS,
        0,
        9, // WM_HINTS has 9 CARD32 values
    ) {
        Ok(cookie) => match cookie.reply() {
            Ok(reply) => Some(reply),
            Err(_) => None,
        },
        Err(_) => None,
    };

    if let Some(reply) = hints_reply {
        if let Some(values) = reply.value32() {
            let values: Vec<u32> = values.collect();
            if !values.is_empty() {
                let flags = values[0];
                if flags & URGENCY_HINT != 0 {
                    return true;
                }
            }
        }
    }

    false
}

/// Check if a window supports the WM_DELETE_WINDOW protocol.
pub fn supports_delete_protocol(conn: &impl Connection, atoms: &Atoms, window: Window) -> bool {
    // Get WM_PROTOCOLS property
    if let Ok(cookie) = conn.get_property(
        false,
        window,
        atoms.wm_protocols,
        AtomEnum::ATOM,
        0,
        32,
    ) {
        if let Ok(reply) = cookie.reply() {
            if let Some(protocol_atoms) = reply.value32() {
                return protocol_atoms.into_iter().any(|a| a == atoms.wm_delete_window);
            }
        }
    }
    false
}

/// Send WM_DELETE_WINDOW client message to request graceful close.
pub fn send_delete_window(conn: &impl Connection, atoms: &Atoms, window: Window) -> Result<()> {
    let data = ClientMessageData::from([atoms.wm_delete_window, 0u32, 0u32, 0u32, 0u32]);
    let event = ClientMessageEvent {
        response_type: CLIENT_MESSAGE_EVENT,
        format: 32,
        sequence: 0,
        window,
        type_: atoms.wm_protocols,
        data,
    };
    conn.send_event(
        false,
        window,
        EventMask::NO_EVENT,
        event,
    )?;
    conn.flush()?;
    Ok(())
}
