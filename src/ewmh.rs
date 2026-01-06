//! EWMH (Extended Window Manager Hints) atom management.
//!
//! This module provides the X11 atoms required for EWMH compliance,
//! enabling proper integration with desktop environments and pagers.

use anyhow::Result;
use x11rb::protocol::xproto::{Atom, ConnectionExt};
use x11rb::rust_connection::RustConnection;

/// EWMH and ICCCM atoms used by the window manager
#[allow(dead_code)]
pub struct Atoms {
    // ICCCM atoms
    pub wm_protocols: Atom,
    pub wm_delete_window: Atom,

    // Core EWMH atoms
    pub net_supported: Atom,
    pub net_client_list: Atom,
    pub net_active_window: Atom,
    pub net_wm_name: Atom,
    pub net_supporting_wm_check: Atom,
    pub utf8_string: Atom,

    // Workspace-related atoms
    pub net_current_desktop: Atom,
    pub net_number_of_desktops: Atom,
    pub net_desktop_names: Atom,
    pub net_wm_desktop: Atom,

    // Icon atom
    pub net_wm_icon: Atom,

    // Close window request
    pub net_close_window: Atom,

    // Window state atoms (for urgent hints and fullscreen)
    pub net_wm_state: Atom,
    pub net_wm_state_demands_attention: Atom,
    pub net_wm_state_fullscreen: Atom,

    // Window type atoms (for auto-float detection)
    pub net_wm_window_type: Atom,
    pub net_wm_window_type_dialog: Atom,
    pub net_wm_window_type_splash: Atom,
    pub net_wm_window_type_toolbar: Atom,
    pub net_wm_window_type_utility: Atom,
    pub net_wm_window_type_menu: Atom,
    pub net_wm_window_type_popup_menu: Atom,
    pub net_wm_window_type_dropdown_menu: Atom,
    pub net_wm_window_type_tooltip: Atom,
    pub net_wm_window_type_notification: Atom,
    pub net_wm_window_type_dock: Atom,

    // Strut atoms (for dock/panel space reservation)
    pub net_wm_strut: Atom,
    pub net_wm_strut_partial: Atom,
}

impl Atoms {
    /// Create and intern all required atoms
    pub fn new(conn: &RustConnection) -> Result<Self> {
        Ok(Self {
            wm_protocols: Self::intern(conn, b"WM_PROTOCOLS")?,
            wm_delete_window: Self::intern(conn, b"WM_DELETE_WINDOW")?,
            net_supported: Self::intern(conn, b"_NET_SUPPORTED")?,
            net_client_list: Self::intern(conn, b"_NET_CLIENT_LIST")?,
            net_active_window: Self::intern(conn, b"_NET_ACTIVE_WINDOW")?,
            net_wm_name: Self::intern(conn, b"_NET_WM_NAME")?,
            net_supporting_wm_check: Self::intern(conn, b"_NET_SUPPORTING_WM_CHECK")?,
            utf8_string: Self::intern(conn, b"UTF8_STRING")?,
            net_current_desktop: Self::intern(conn, b"_NET_CURRENT_DESKTOP")?,
            net_number_of_desktops: Self::intern(conn, b"_NET_NUMBER_OF_DESKTOPS")?,
            net_desktop_names: Self::intern(conn, b"_NET_DESKTOP_NAMES")?,
            net_wm_desktop: Self::intern(conn, b"_NET_WM_DESKTOP")?,
            net_wm_icon: Self::intern(conn, b"_NET_WM_ICON")?,
            net_close_window: Self::intern(conn, b"_NET_CLOSE_WINDOW")?,
            net_wm_state: Self::intern(conn, b"_NET_WM_STATE")?,
            net_wm_state_demands_attention: Self::intern(conn, b"_NET_WM_STATE_DEMANDS_ATTENTION")?,
            net_wm_state_fullscreen: Self::intern(conn, b"_NET_WM_STATE_FULLSCREEN")?,
            net_wm_window_type: Self::intern(conn, b"_NET_WM_WINDOW_TYPE")?,
            net_wm_window_type_dialog: Self::intern(conn, b"_NET_WM_WINDOW_TYPE_DIALOG")?,
            net_wm_window_type_splash: Self::intern(conn, b"_NET_WM_WINDOW_TYPE_SPLASH")?,
            net_wm_window_type_toolbar: Self::intern(conn, b"_NET_WM_WINDOW_TYPE_TOOLBAR")?,
            net_wm_window_type_utility: Self::intern(conn, b"_NET_WM_WINDOW_TYPE_UTILITY")?,
            net_wm_window_type_menu: Self::intern(conn, b"_NET_WM_WINDOW_TYPE_MENU")?,
            net_wm_window_type_popup_menu: Self::intern(conn, b"_NET_WM_WINDOW_TYPE_POPUP_MENU")?,
            net_wm_window_type_dropdown_menu: Self::intern(conn, b"_NET_WM_WINDOW_TYPE_DROPDOWN_MENU")?,
            net_wm_window_type_tooltip: Self::intern(conn, b"_NET_WM_WINDOW_TYPE_TOOLTIP")?,
            net_wm_window_type_notification: Self::intern(conn, b"_NET_WM_WINDOW_TYPE_NOTIFICATION")?,
            net_wm_window_type_dock: Self::intern(conn, b"_NET_WM_WINDOW_TYPE_DOCK")?,
            net_wm_strut: Self::intern(conn, b"_NET_WM_STRUT")?,
            net_wm_strut_partial: Self::intern(conn, b"_NET_WM_STRUT_PARTIAL")?,
        })
    }

    /// Intern an atom name
    fn intern(conn: &RustConnection, name: &[u8]) -> Result<Atom> {
        Ok(conn.intern_atom(false, name)?.reply()?.atom)
    }
}
