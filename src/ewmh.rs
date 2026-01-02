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
        })
    }

    /// Intern an atom name
    fn intern(conn: &RustConnection, name: &[u8]) -> Result<Atom> {
        Ok(conn.intern_atom(false, name)?.reply()?.atom)
    }
}
