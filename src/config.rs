//! Configuration file support for ttwm.
//!
//! Loads settings from ~/.config/ttwm/config.toml if it exists,
//! otherwise uses sensible defaults.
//!
//! Also provides `LayoutConfig` - the runtime configuration struct with
//! resolved color values and layout parameters.

use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

// =============================================================================
// Runtime Configuration (resolved values)
// =============================================================================

/// Runtime layout configuration with resolved color values.
///
/// This struct holds the actual u32 color values and layout parameters
/// used during rendering. It's constructed from the file-based config
/// types at startup.
#[derive(Debug, Clone)]
pub struct LayoutConfig {
    /// Gap between windows
    pub gap: u32,
    /// Outer gap (margin from screen edge)
    pub outer_gap: u32,
    /// Border width
    pub border_width: u32,
    /// Tab bar height (for horizontal tabs)
    pub tab_bar_height: u32,
    /// Vertical tab bar width (for vertical tabs)
    pub vertical_tab_width: u32,
    /// Tab bar background color
    pub tab_bar_bg: u32,
    /// Tab bar focused tab color
    pub tab_focused_bg: u32,
    /// Tab bar unfocused tab color
    pub tab_unfocused_bg: u32,
    /// Visible tab in unfocused frame color
    pub tab_visible_unfocused_bg: u32,
    /// Tagged tab background color
    pub tab_tagged_bg: u32,
    /// Urgent tab background color
    pub tab_urgent_bg: u32,
    /// Tab bar text color
    pub tab_text_color: u32,
    /// Tab bar text color for background tabs
    pub tab_text_unfocused: u32,
    /// Tab separator color
    pub tab_separator: u32,
    /// Border color for focused window
    pub border_focused: u32,
    /// Border color for unfocused window
    pub border_unfocused: u32,
    /// Show application icons in tabs
    pub show_tab_icons: bool,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            gap: 8,
            outer_gap: 8,
            border_width: 2,
            tab_bar_height: 28,
            vertical_tab_width: 28,
            tab_bar_bg: 0x000000,       // Black (fallback)
            tab_focused_bg: 0x5294e2,   // Blue (matching border)
            tab_unfocused_bg: 0x3a3a3a, // Darker gray
            tab_visible_unfocused_bg: 0x4a6a9a, // Muted blue
            tab_tagged_bg: 0xe06c75,    // Soft red
            tab_urgent_bg: 0xd19a66,    // Orange/amber
            tab_text_color: 0xffffff,   // White
            tab_text_unfocused: 0x888888, // Dim gray
            tab_separator: 0x4a4a4a,    // Subtle separator
            border_focused: 0x5294e2,   // Blue
            border_unfocused: 0x3a3a3a, // Gray
            show_tab_icons: true,
        }
    }
}

// =============================================================================
// File-based Configuration (TOML parsing)
// =============================================================================

/// Top-level configuration
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub general: GeneralConfig,
    pub appearance: AppearanceConfig,
    pub colors: ColorConfig,
    pub keybindings: KeybindingConfig,
    pub exec: ExecConfig,
    pub startup: StartupConfig,
}

/// Exec keybindings (key combo -> command to run)
#[derive(Debug, Deserialize, Clone)]
#[serde(default)]
pub struct ExecConfig {
    #[serde(flatten)]
    pub bindings: HashMap<String, String>,
}

/// Startup layout configuration
#[derive(Debug, Default, Deserialize, Clone)]
#[serde(default)]
pub struct StartupConfig {
    /// Per-workspace layout configurations, keyed by workspace number as string ("1"-"9")
    #[serde(default)]
    pub workspace: HashMap<String, WorkspaceStartup>,
}

/// Configuration for a single workspace's startup layout
#[derive(Debug, Deserialize, Clone)]
pub struct WorkspaceStartup {
    /// The layout tree definition
    pub layout: LayoutNodeConfig,
}

/// Recursive enum representing either a frame or a split in the layout tree
#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum LayoutNodeConfig {
    /// A leaf frame that can contain windows
    Frame(FrameConfig),
    /// A split node with two children
    Split(SplitConfig),
}

/// Configuration for a frame (leaf node)
#[derive(Debug, Default, Deserialize, Clone)]
#[serde(default)]
pub struct FrameConfig {
    /// Optional name for the frame (used for window placement rules)
    pub name: Option<String>,
    /// Whether tabs should be displayed vertically
    #[serde(default)]
    pub vertical_tabs: bool,
    /// Applications to spawn in this frame at startup
    #[serde(default)]
    pub apps: Vec<String>,
}

/// Configuration for a split node
#[derive(Debug, Deserialize, Clone)]
pub struct SplitConfig {
    /// Split direction: "horizontal" or "vertical"
    pub direction: SplitDirectionConfig,
    /// Ratio of space given to first child (0.0 to 1.0, default 0.5)
    #[serde(default = "default_ratio")]
    pub ratio: f32,
    /// First child (left or top)
    pub first: Box<LayoutNodeConfig>,
    /// Second child (right or bottom)
    pub second: Box<LayoutNodeConfig>,
}

/// Split direction for config parsing
#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum SplitDirectionConfig {
    Horizontal,
    Vertical,
}

fn default_ratio() -> f32 {
    0.5
}

/// General settings
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
pub struct GeneralConfig {
    // Reserved for future general settings
}

/// Appearance settings (gaps, borders, etc.)
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct AppearanceConfig {
    pub gap: u32,
    pub outer_gap: u32,
    pub border_width: u32,
    pub tab_bar_height: u32,
    pub vertical_tab_width: u32,
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
    pub tab_urgent_bg: String,
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
    pub toggle_float: Option<String>,
    pub toggle_fullscreen: Option<String>,
    pub toggle_vertical_tabs: Option<String>,
    pub focus_urgent: Option<String>,
    pub focus_monitor_left: Option<String>,
    pub focus_monitor_right: Option<String>,
}

/// Parsed keybinding (ready for X11 grab)
#[derive(Debug, Clone, Copy)]
pub struct ParsedBinding {
    pub keysym: u32,
    pub modifiers: u16,
}

/// Window manager action
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WmAction {
    Spawn(String),
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
    ToggleFloat,
    ToggleFullscreen,
    ToggleVerticalTabs,
    FocusUrgent,
    FocusMonitorLeft,
    FocusMonitorRight,
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
        insert(WmAction::ToggleFloat, &self.keybindings.toggle_float);
        insert(WmAction::ToggleFullscreen, &self.keybindings.toggle_fullscreen);
        insert(WmAction::ToggleVerticalTabs, &self.keybindings.toggle_vertical_tabs);
        insert(WmAction::FocusUrgent, &self.keybindings.focus_urgent);
        insert(WmAction::FocusMonitorLeft, &self.keybindings.focus_monitor_left);
        insert(WmAction::FocusMonitorRight, &self.keybindings.focus_monitor_right);

        // Parse exec bindings (key combo -> command)
        for (key_combo, command) in &self.exec.bindings {
            if let Some(parsed) = parse_key_binding(key_combo) {
                bindings.insert(WmAction::Spawn(command.clone()), parsed);
            } else {
                log::warn!("Failed to parse exec keybinding: {}", key_combo);
            }
        }

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
        // Function keys F1-F12
        "f1" => Some(0xffbe),
        "f2" => Some(0xffbf),
        "f3" => Some(0xffc0),
        "f4" => Some(0xffc1),
        "f5" => Some(0xffc2),
        "f6" => Some(0xffc3),
        "f7" => Some(0xffc4),
        "f8" => Some(0xffc5),
        "f9" => Some(0xffc6),
        "f10" => Some(0xffc7),
        "f11" => Some(0xffc8),
        "f12" => Some(0xffc9),
        // Bracket keys
        "[" | "bracketleft" => Some(0x5b),
        "]" | "bracketright" => Some(0x5d),
        // Slash key
        "/" | "slash" => Some(0x2f),
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

impl Default for ExecConfig {
    fn default() -> Self {
        let mut bindings = HashMap::new();
        bindings.insert("Mod4+x".to_string(), "alacritty".to_string());
        bindings.insert("Mod4+r".to_string(), "gmrun".to_string());
        Self { bindings }
    }
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            gap: 8,
            outer_gap: 8,
            border_width: 2,
            tab_bar_height: 28,
            vertical_tab_width: 28,
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
            tab_urgent_bg: "#d19a66".to_string(),
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
            quit: Some("Mod4+Control+F4".to_string()),
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
            toggle_float: Some("Mod4+f".to_string()),
            toggle_fullscreen: Some("Mod4+Return".to_string()),
            toggle_vertical_tabs: Some("Mod4+slash".to_string()),
            focus_urgent: Some("Mod4+space".to_string()),
            focus_monitor_left: Some("Mod4+Control+Left".to_string()),
            focus_monitor_right: Some("Mod4+Control+Right".to_string()),
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

        assert!(bindings.contains_key(&WmAction::Spawn("alacritty".to_string())));
        assert!(bindings.contains_key(&WmAction::Spawn("gmrun".to_string())));
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

    #[test]
    fn test_startup_config_simple_frame() {
        let toml = r#"
[startup.workspace.1]
layout = { type = "frame", name = "main", apps = ["alacritty"] }
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert!(config.startup.workspace.contains_key("1"));
        let ws = &config.startup.workspace["1"];
        match &ws.layout {
            LayoutNodeConfig::Frame(frame) => {
                assert_eq!(frame.name, Some("main".to_string()));
                assert_eq!(frame.apps, vec!["alacritty".to_string()]);
                assert!(!frame.vertical_tabs);
            }
            _ => panic!("Expected frame"),
        }
    }

    #[test]
    fn test_startup_config_nested_split() {
        let toml = r#"
[startup.workspace.1]
layout = { type = "split", direction = "horizontal", ratio = 0.6, first = { type = "frame", name = "editor" }, second = { type = "split", direction = "vertical", ratio = 0.5, first = { type = "frame", name = "terminal" }, second = { type = "frame", name = "browser", vertical_tabs = true } } }
"#;
        let config: Config = toml::from_str(toml).unwrap();
        let ws = &config.startup.workspace["1"];
        match &ws.layout {
            LayoutNodeConfig::Split(split) => {
                assert!(matches!(split.direction, SplitDirectionConfig::Horizontal));
                assert!((split.ratio - 0.6).abs() < 0.01);
                // Check first child is frame "editor"
                match split.first.as_ref() {
                    LayoutNodeConfig::Frame(f) => {
                        assert_eq!(f.name, Some("editor".to_string()));
                    }
                    _ => panic!("Expected frame"),
                }
                // Check second child is a vertical split
                match split.second.as_ref() {
                    LayoutNodeConfig::Split(s2) => {
                        assert!(matches!(s2.direction, SplitDirectionConfig::Vertical));
                        // Check browser frame has vertical_tabs
                        match s2.second.as_ref() {
                            LayoutNodeConfig::Frame(f) => {
                                assert_eq!(f.name, Some("browser".to_string()));
                                assert!(f.vertical_tabs);
                            }
                            _ => panic!("Expected frame"),
                        }
                    }
                    _ => panic!("Expected split"),
                }
            }
            _ => panic!("Expected split"),
        }
    }

    #[test]
    fn test_startup_config_default_ratio() {
        let toml = r#"
[startup.workspace.2]
layout = { type = "split", direction = "vertical", first = { type = "frame" }, second = { type = "frame" } }
"#;
        let config: Config = toml::from_str(toml).unwrap();
        let ws = &config.startup.workspace["2"];
        match &ws.layout {
            LayoutNodeConfig::Split(split) => {
                assert!((split.ratio - 0.5).abs() < 0.01); // Default ratio
            }
            _ => panic!("Expected split"),
        }
    }

    #[test]
    fn test_startup_config_empty() {
        let config = Config::default();
        assert!(config.startup.workspace.is_empty());
    }
}
