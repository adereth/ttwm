//! Configuration file support for ttwm.
//!
//! Loads settings from ~/.config/ttwm/config.toml if it exists,
//! otherwise uses sensible defaults.

use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

/// Top-level configuration
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub general: GeneralConfig,
    pub appearance: AppearanceConfig,
    pub colors: ColorConfig,
    pub keybindings: KeybindingConfig,
}

/// General settings
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    pub terminal: String,
}

/// Appearance settings (gaps, borders, etc.)
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct AppearanceConfig {
    pub gap: u32,
    pub outer_gap: u32,
    pub border_width: u32,
    pub tab_bar_height: u32,
    pub tab_font: String,
    pub tab_font_size: u32,
    pub show_tab_icons: bool,
}

/// Color settings (hex strings like "#5294e2")
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct ColorConfig {
    pub tab_bar_bg: String,
    pub tab_focused_bg: String,
    pub tab_unfocused_bg: String,
    pub tab_visible_unfocused_bg: String,
    pub tab_tagged_bg: String,
    pub tab_text: String,
    pub tab_text_unfocused: String,
    pub tab_separator: String,
    pub border_focused: String,
    pub border_unfocused: String,
}

/// Keybinding configuration (strings like "Mod4+Return")
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct KeybindingConfig {
    pub spawn_terminal: Option<String>,
    pub cycle_tab_forward: Option<String>,
    pub cycle_tab_backward: Option<String>,
    pub focus_next: Option<String>,
    pub focus_prev: Option<String>,
    pub focus_frame_left: Option<String>,
    pub focus_frame_right: Option<String>,
    pub focus_frame_up: Option<String>,
    pub focus_frame_down: Option<String>,
    pub move_window_left: Option<String>,
    pub move_window_right: Option<String>,
    pub resize_shrink: Option<String>,
    pub resize_grow: Option<String>,
    pub split_horizontal: Option<String>,
    pub split_vertical: Option<String>,
    pub close_window: Option<String>,
    pub quit: Option<String>,
    pub focus_tab_1: Option<String>,
    pub focus_tab_2: Option<String>,
    pub focus_tab_3: Option<String>,
    pub focus_tab_4: Option<String>,
    pub focus_tab_5: Option<String>,
    pub focus_tab_6: Option<String>,
    pub focus_tab_7: Option<String>,
    pub focus_tab_8: Option<String>,
    pub focus_tab_9: Option<String>,
    pub workspace_next: Option<String>,
    pub workspace_prev: Option<String>,
    pub tag_window: Option<String>,
    pub move_tagged_windows: Option<String>,
    pub untag_all: Option<String>,
}

/// Parsed keybinding (ready for X11 grab)
#[derive(Debug, Clone, Copy)]
pub struct ParsedBinding {
    pub keysym: u32,
    pub modifiers: u16,
}

/// Window manager action
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WmAction {
    SpawnTerminal,
    CycleTabForward,
    CycleTabBackward,
    FocusNext,
    FocusPrev,
    FocusFrameLeft,
    FocusFrameRight,
    FocusFrameUp,
    FocusFrameDown,
    MoveWindowLeft,
    MoveWindowRight,
    ResizeShrink,
    ResizeGrow,
    SplitHorizontal,
    SplitVertical,
    CloseWindow,
    Quit,
    FocusTab(usize),
    WorkspaceNext,
    WorkspacePrev,
    TagWindow,
    MoveTaggedToFrame,
    UntagAll,
}

impl Config {
    /// Load config from default path (~/.config/ttwm/config.toml)
    pub fn load() -> Self {
        Self::load_from_path(Self::default_path())
    }

    /// Default config file path
    pub fn default_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("ttwm")
            .join("config.toml")
    }

    /// Load config from a specific path
    pub fn load_from_path(path: PathBuf) -> Self {
        match std::fs::read_to_string(&path) {
            Ok(contents) => match toml::from_str(&contents) {
                Ok(config) => {
                    log::info!("Loaded config from {:?}", path);
                    config
                }
                Err(e) => {
                    log::warn!("Failed to parse config: {}", e);
                    Self::default()
                }
            },
            Err(_) => {
                log::info!("No config file found at {:?}, using defaults", path);
                Self::default()
            }
        }
    }

    /// Parse keybindings into action -> ParsedBinding map
    pub fn parse_keybindings(&self) -> HashMap<WmAction, ParsedBinding> {
        let mut bindings = HashMap::new();

        // Helper to parse and insert
        let mut insert = |action: WmAction, key_str: &Option<String>| {
            if let Some(s) = key_str {
                if let Some(parsed) = parse_key_binding(s) {
                    bindings.insert(action, parsed);
                } else {
                    log::warn!("Failed to parse keybinding: {}", s);
                }
            }
        };

        insert(WmAction::SpawnTerminal, &self.keybindings.spawn_terminal);
        insert(WmAction::CycleTabForward, &self.keybindings.cycle_tab_forward);
        insert(WmAction::CycleTabBackward, &self.keybindings.cycle_tab_backward);
        insert(WmAction::FocusNext, &self.keybindings.focus_next);
        insert(WmAction::FocusPrev, &self.keybindings.focus_prev);
        insert(WmAction::FocusFrameLeft, &self.keybindings.focus_frame_left);
        insert(WmAction::FocusFrameRight, &self.keybindings.focus_frame_right);
        insert(WmAction::FocusFrameUp, &self.keybindings.focus_frame_up);
        insert(WmAction::FocusFrameDown, &self.keybindings.focus_frame_down);
        insert(WmAction::MoveWindowLeft, &self.keybindings.move_window_left);
        insert(WmAction::MoveWindowRight, &self.keybindings.move_window_right);
        insert(WmAction::ResizeShrink, &self.keybindings.resize_shrink);
        insert(WmAction::ResizeGrow, &self.keybindings.resize_grow);
        insert(WmAction::SplitHorizontal, &self.keybindings.split_horizontal);
        insert(WmAction::SplitVertical, &self.keybindings.split_vertical);
        insert(WmAction::CloseWindow, &self.keybindings.close_window);
        insert(WmAction::Quit, &self.keybindings.quit);
        insert(WmAction::FocusTab(1), &self.keybindings.focus_tab_1);
        insert(WmAction::FocusTab(2), &self.keybindings.focus_tab_2);
        insert(WmAction::FocusTab(3), &self.keybindings.focus_tab_3);
        insert(WmAction::FocusTab(4), &self.keybindings.focus_tab_4);
        insert(WmAction::FocusTab(5), &self.keybindings.focus_tab_5);
        insert(WmAction::FocusTab(6), &self.keybindings.focus_tab_6);
        insert(WmAction::FocusTab(7), &self.keybindings.focus_tab_7);
        insert(WmAction::FocusTab(8), &self.keybindings.focus_tab_8);
        insert(WmAction::FocusTab(9), &self.keybindings.focus_tab_9);
        insert(WmAction::WorkspaceNext, &self.keybindings.workspace_next);
        insert(WmAction::WorkspacePrev, &self.keybindings.workspace_prev);
        insert(WmAction::TagWindow, &self.keybindings.tag_window);
        insert(WmAction::MoveTaggedToFrame, &self.keybindings.move_tagged_windows);
        insert(WmAction::UntagAll, &self.keybindings.untag_all);

        bindings
    }
}

/// Parse a key binding string like "Mod4+Shift+h" into keysym and modifiers
pub fn parse_key_binding(s: &str) -> Option<ParsedBinding> {
    let parts: Vec<&str> = s.split('+').collect();
    if parts.is_empty() {
        return None;
    }

    let mut modifiers: u16 = 0;
    let key_part = parts.last()?;

    // X11 modifier masks
    const SHIFT_MASK: u16 = 1;
    const CONTROL_MASK: u16 = 4;
    const MOD1_MASK: u16 = 8; // Alt
    const MOD4_MASK: u16 = 64; // Super/Win

    for part in &parts[..parts.len() - 1] {
        match part.to_lowercase().as_str() {
            "mod4" | "super" | "win" => modifiers |= MOD4_MASK,
            "shift" => modifiers |= SHIFT_MASK,
            "control" | "ctrl" => modifiers |= CONTROL_MASK,
            "mod1" | "alt" => modifiers |= MOD1_MASK,
            _ => {
                log::warn!("Unknown modifier: {}", part);
            }
        }
    }

    let keysym = key_to_keysym(key_part)?;
    Some(ParsedBinding { keysym, modifiers })
}

/// Convert key name to X11 keysym
fn key_to_keysym(key: &str) -> Option<u32> {
    match key.to_lowercase().as_str() {
        "return" | "enter" => Some(0xff0d),
        "tab" => Some(0xff09),
        "escape" | "esc" => Some(0xff1b),
        "space" => Some(0x20),
        "backspace" => Some(0xff08),
        "delete" => Some(0xffff),
        "a" => Some(0x61),
        "b" => Some(0x62),
        "c" => Some(0x63),
        "d" => Some(0x64),
        "e" => Some(0x65),
        "f" => Some(0x66),
        "g" => Some(0x67),
        "h" => Some(0x68),
        "i" => Some(0x69),
        "j" => Some(0x6a),
        "k" => Some(0x6b),
        "l" => Some(0x6c),
        "m" => Some(0x6d),
        "n" => Some(0x6e),
        "o" => Some(0x6f),
        "p" => Some(0x70),
        "q" => Some(0x71),
        "r" => Some(0x72),
        "s" => Some(0x73),
        "t" => Some(0x74),
        "u" => Some(0x75),
        "v" => Some(0x76),
        "w" => Some(0x77),
        "x" => Some(0x78),
        "y" => Some(0x79),
        "z" => Some(0x7a),
        "1" => Some(0x31),
        "2" => Some(0x32),
        "3" => Some(0x33),
        "4" => Some(0x34),
        "5" => Some(0x35),
        "6" => Some(0x36),
        "7" => Some(0x37),
        "8" => Some(0x38),
        "9" => Some(0x39),
        "0" => Some(0x30),
        // Function/navigation keys
        "page_up" | "pageup" | "pgup" | "prior" => Some(0xff55),
        "page_down" | "pagedown" | "pgdn" | "next" => Some(0xff56),
        "left" => Some(0xff51),
        "up" => Some(0xff52),
        "right" => Some(0xff53),
        "down" => Some(0xff54),
        "home" => Some(0xff50),
        "end" => Some(0xff57),
        // Bracket keys
        "[" | "bracketleft" => Some(0x5b),
        "]" | "bracketright" => Some(0x5d),
        _ => {
            log::warn!("Unknown key: {}", key);
            None
        }
    }
}

/// Parse hex color string (e.g., "#5294e2" or "5294e2") to u32
pub fn parse_color(s: &str) -> Option<u32> {
    let s = s.trim_start_matches('#');
    u32::from_str_radix(s, 16).ok()
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            terminal: "alacritty".to_string(),
        }
    }
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            gap: 8,
            outer_gap: 8,
            border_width: 2,
            tab_bar_height: 28,
            tab_font: "monospace".to_string(),
            tab_font_size: 11,
            show_tab_icons: true,
        }
    }
}

impl Default for ColorConfig {
    fn default() -> Self {
        Self {
            tab_bar_bg: "#000000".to_string(),
            tab_focused_bg: "#5294e2".to_string(),
            tab_unfocused_bg: "#3a3a3a".to_string(),
            tab_visible_unfocused_bg: "#4a6a9a".to_string(),
            tab_tagged_bg: "#e06c75".to_string(),
            tab_text: "#ffffff".to_string(),
            tab_text_unfocused: "#888888".to_string(),
            tab_separator: "#4a4a4a".to_string(),
            border_focused: "#5294e2".to_string(),
            border_unfocused: "#3a3a3a".to_string(),
        }
    }
}

impl Default for KeybindingConfig {
    fn default() -> Self {
        Self {
            spawn_terminal: Some("Mod4+x".to_string()),
            cycle_tab_forward: Some("Mod4+Page_Down".to_string()),
            cycle_tab_backward: Some("Mod4+Page_Up".to_string()),
            focus_next: Some("Mod4+j".to_string()),
            focus_prev: Some("Mod4+k".to_string()),
            focus_frame_left: Some("Mod4+Left".to_string()),
            focus_frame_right: Some("Mod4+Right".to_string()),
            focus_frame_up: Some("Mod4+Up".to_string()),
            focus_frame_down: Some("Mod4+Down".to_string()),
            move_window_left: Some("Mod4+Shift+Left".to_string()),
            move_window_right: Some("Mod4+Shift+Right".to_string()),
            resize_shrink: Some("Mod4+Control+Left".to_string()),
            resize_grow: Some("Mod4+Control+Right".to_string()),
            split_horizontal: Some("Mod4+s".to_string()),
            split_vertical: Some("Mod4+v".to_string()),
            close_window: Some("Mod4+q".to_string()),
            quit: Some("Mod4+Shift+q".to_string()),
            focus_tab_1: Some("Mod4+1".to_string()),
            focus_tab_2: Some("Mod4+2".to_string()),
            focus_tab_3: Some("Mod4+3".to_string()),
            focus_tab_4: Some("Mod4+4".to_string()),
            focus_tab_5: Some("Mod4+5".to_string()),
            focus_tab_6: Some("Mod4+6".to_string()),
            focus_tab_7: Some("Mod4+7".to_string()),
            focus_tab_8: Some("Mod4+8".to_string()),
            focus_tab_9: Some("Mod4+9".to_string()),
            workspace_next: Some("Mod4+]".to_string()),
            workspace_prev: Some("Mod4+[".to_string()),
            tag_window: Some("Mod4+t".to_string()),
            move_tagged_windows: Some("Mod4+a".to_string()),
            untag_all: Some("Mod4+Shift+t".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_key_binding() {
        let binding = parse_key_binding("Mod4+Return").unwrap();
        assert_eq!(binding.keysym, 0xff0d);
        assert_eq!(binding.modifiers, 64); // Mod4

        let binding = parse_key_binding("Mod4+Shift+q").unwrap();
        assert_eq!(binding.keysym, 0x71);
        assert_eq!(binding.modifiers, 64 | 1); // Mod4 + Shift

        let binding = parse_key_binding("Mod4+Control+h").unwrap();
        assert_eq!(binding.keysym, 0x68);
        assert_eq!(binding.modifiers, 64 | 4); // Mod4 + Control
    }

    #[test]
    fn test_parse_color() {
        assert_eq!(parse_color("#5294e2"), Some(0x5294e2));
        assert_eq!(parse_color("5294e2"), Some(0x5294e2));
        assert_eq!(parse_color("#2e2e2e"), Some(0x2e2e2e));
        assert_eq!(parse_color("ffffff"), Some(0xffffff));
    }

    #[test]
    fn test_default_keybindings() {
        let config = Config::default();
        let bindings = config.parse_keybindings();

        assert!(bindings.contains_key(&WmAction::SpawnTerminal));
        assert!(bindings.contains_key(&WmAction::Quit));
        assert!(bindings.contains_key(&WmAction::FocusTab(1)));
    }

    #[test]
    fn test_key_to_keysym() {
        assert_eq!(key_to_keysym("return"), Some(0xff0d));
        assert_eq!(key_to_keysym("Return"), Some(0xff0d));
        assert_eq!(key_to_keysym("tab"), Some(0xff09));
        assert_eq!(key_to_keysym("h"), Some(0x68));
        assert_eq!(key_to_keysym("1"), Some(0x31));
    }
}
