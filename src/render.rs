//! Rendering utilities for tab bar and text rendering.
//!
//! This module contains the font renderer and helper functions for
//! drawing tab bars with anti-aliased text.

use std::path::PathBuf;
use anyhow::{Context, Result};
use freetype::Library as FtLibrary;
use once_cell::sync::Lazy;

/// Static default icon for windows without _NET_WM_ICON
pub static DEFAULT_ICON: Lazy<CachedIcon> = Lazy::new(CachedIcon::default_icon);

/// Tab bar rendering constants
#[allow(dead_code)]
pub mod constants {
    /// Minimum tab width in pixels
    pub const TAB_MIN_WIDTH: u32 = 80;
    /// Maximum tab width in pixels
    pub const TAB_MAX_WIDTH: u32 = 200;
    /// Padding on each side of tab text
    pub const TAB_PADDING: u32 = 24;
    /// Vertical offset for tab text
    pub const TAB_TEXT_OFFSET: i16 = 4;
    /// Icon size (width and height)
    pub const ICON_SIZE: u32 = 20;
    /// Border radius for tab top corners
    pub const BEVEL_RADIUS: i16 = 6;
}

/// Cached window icon (20x20 BGRA pixels)
pub struct CachedIcon {
    /// BGRA pixel data (20 * 20 * 4 = 1600 bytes)
    pub pixels: Vec<u8>,
}

impl CachedIcon {
    /// Create a default icon for windows without _NET_WM_ICON
    pub fn default_icon() -> Self {
        CachedIcon { pixels: generate_default_icon() }
    }
}

/// Generate a default 20x20 window icon (BGRA format)
/// Design: Simple window outline with title bar
pub fn generate_default_icon() -> Vec<u8> {
    const SIZE: usize = 20;
    let mut pixels = vec![0u8; SIZE * SIZE * 4];

    // Colors (BGRA format)
    let border = [0x88, 0x88, 0x88, 0xFF];      // Gray border
    let title_bar = [0xAA, 0xAA, 0xAA, 0xFF];   // Lighter gray title bar
    let background = [0x3A, 0x3A, 0x3A, 0xFF];  // Dark background
    let transparent = [0x00, 0x00, 0x00, 0x00]; // Transparent

    for y in 0..SIZE {
        for x in 0..SIZE {
            let idx = (y * SIZE + x) * 4;
            let pixel = if x < 2 || x >= 18 || y < 2 || y >= 18 {
                // Outside main area - transparent with padding
                transparent
            } else if x == 2 || x == 17 || y == 2 || y == 17 {
                // Border
                border
            } else if y >= 3 && y <= 5 {
                // Title bar area (3 pixels tall)
                title_bar
            } else {
                // Window content area
                background
            };
            pixels[idx..idx + 4].copy_from_slice(&pixel);
        }
    }

    pixels
}

/// Font renderer using FreeType for anti-aliased text
pub struct FontRenderer {
    _library: FtLibrary,
    face: freetype::Face,
    _char_width: u32,
    char_height: u32,
    ascender: i32,
}

impl FontRenderer {
    /// Create a new font renderer with the specified font and size
    pub fn new(font_name: &str, font_size: u32) -> Result<Self> {
        // Initialize FreeType library
        let library = FtLibrary::init().context("Failed to initialize FreeType")?;

        // Use fontconfig to find the font file
        let font_path = Self::find_font(font_name)?;
        log::info!("Loading font: {:?}", font_path);

        // Load the font face
        let face = library
            .new_face(&font_path, 0)
            .context("Failed to load font face")?;

        // Set the font size (in 1/64th points, at 96 DPI)
        face.set_char_size(0, (font_size as isize) * 64, 96, 96)
            .context("Failed to set font size")?;

        // Get font metrics
        let metrics = face.size_metrics().context("Failed to get font metrics")?;
        let char_height = (metrics.height >> 6) as u32;
        let ascender = (metrics.ascender >> 6) as i32;

        // Calculate average character width (using 'M' as reference)
        let char_width = if face.load_char('M' as usize, freetype::face::LoadFlag::DEFAULT).is_ok() {
            let glyph = face.glyph();
            (glyph.advance().x >> 6) as u32
        } else {
            // Fallback: estimate based on size
            (font_size as f32 * 0.6) as u32
        };

        log::info!(
            "Font loaded: char_width={}, char_height={}, ascender={}",
            char_width,
            char_height,
            ascender
        );

        Ok(Self {
            _library: library,
            face,
            _char_width: char_width,
            char_height,
            ascender,
        })
    }

    /// Find font file path by searching common font directories
    fn find_font(font_name: &str) -> Result<PathBuf> {
        // Common font directories on Linux
        let font_dirs = [
            "/usr/share/fonts",
            "/usr/local/share/fonts",
            "/home",  // Will search ~/.local/share/fonts via home dir
        ];

        // Also check user font directory
        let home_fonts = dirs::home_dir()
            .map(|h| h.join(".local/share/fonts"))
            .filter(|p| p.exists());

        // Font file patterns to search for (ordered by preference)
        let font_patterns: Vec<String> = if font_name == "monospace" {
            // For "monospace", try common monospace fonts
            vec![
                "DejaVuSansMono".to_string(),
                "LiberationMono".to_string(),
                "UbuntuMono".to_string(),
                "DroidSansMono".to_string(),
                "FreeMono".to_string(),
                "NotoSansMono".to_string(),
            ]
        } else {
            // Convert font name to possible file name patterns
            let normalized = font_name.replace(' ', "");
            vec![
                normalized.clone(),
                font_name.replace(' ', "-"),
                font_name.to_string(),
            ]
        };

        // Search font directories
        let mut dirs_to_search: Vec<PathBuf> = font_dirs
            .iter()
            .map(PathBuf::from)
            .filter(|p| p.exists())
            .collect();

        if let Some(home_font_dir) = home_fonts {
            dirs_to_search.insert(0, home_font_dir);
        }

        for pattern in &font_patterns {
            for dir in &dirs_to_search {
                if let Some(font_path) = Self::search_font_in_dir(dir, pattern) {
                    log::info!("Found font: {:?}", font_path);
                    return Ok(font_path);
                }
            }
        }

        // Last resort: look for any .ttf or .otf file
        for dir in &dirs_to_search {
            if let Some(font_path) = Self::find_any_font_in_dir(dir) {
                log::warn!("Font '{}' not found, using fallback: {:?}", font_name, font_path);
                return Ok(font_path);
            }
        }

        anyhow::bail!("No suitable font found. Please install a TTF/OTF font.")
    }

    /// Search for a font file matching the pattern in a directory (recursive)
    fn search_font_in_dir(dir: &PathBuf, pattern: &str) -> Option<PathBuf> {
        let pattern_lower = pattern.to_lowercase();

        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();

                if path.is_dir() {
                    if let Some(found) = Self::search_font_in_dir(&path, pattern) {
                        return Some(found);
                    }
                } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    let name_lower = name.to_lowercase();
                    // Check if it's a font file and matches the pattern
                    if (name_lower.ends_with(".ttf") || name_lower.ends_with(".otf"))
                        && name_lower.contains(&pattern_lower)
                        && !name_lower.contains("bold")
                        && !name_lower.contains("italic")
                        && !name_lower.contains("oblique")
                    {
                        return Some(path);
                    }
                }
            }
        }
        None
    }

    /// Find any regular font file in a directory
    fn find_any_font_in_dir(dir: &PathBuf) -> Option<PathBuf> {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();

                if path.is_dir() {
                    // Check ttf subdirectory first (common on Linux)
                    if path.file_name().map_or(false, |n| n == "truetype" || n == "TTF") {
                        if let Some(found) = Self::find_any_font_in_dir(&path) {
                            return Some(found);
                        }
                    }
                } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    let name_lower = name.to_lowercase();
                    if (name_lower.ends_with(".ttf") || name_lower.ends_with(".otf"))
                        && !name_lower.contains("bold")
                        && !name_lower.contains("italic")
                    {
                        return Some(path);
                    }
                }
            }
        }
        None
    }

    /// Render text and return BGRA pixel data (for X11 ZPixmap format)
    pub fn render_text(&self, text: &str, fg_color: u32, bg_color: u32) -> (Vec<u8>, u32, u32) {
        if text.is_empty() {
            return (Vec::new(), 0, 0);
        }

        // Calculate text dimensions
        let width = self.measure_text(text);
        let height = self.char_height;

        if width == 0 || height == 0 {
            return (Vec::new(), 0, 0);
        }

        // Create BGRA buffer (X11 uses BGRX in 32-bit depth)
        let mut pixels = vec![0u8; (width * height * 4) as usize];

        // Fill with background color
        let bg_b = (bg_color & 0xFF) as u8;
        let bg_g = ((bg_color >> 8) & 0xFF) as u8;
        let bg_r = ((bg_color >> 16) & 0xFF) as u8;
        for i in 0..(width * height) as usize {
            pixels[i * 4] = bg_b;
            pixels[i * 4 + 1] = bg_g;
            pixels[i * 4 + 2] = bg_r;
            pixels[i * 4 + 3] = 0xFF;
        }

        // Extract foreground color components
        let fg_b = (fg_color & 0xFF) as u8;
        let fg_g = ((fg_color >> 8) & 0xFF) as u8;
        let fg_r = ((fg_color >> 16) & 0xFF) as u8;

        // Render each character
        let mut x_pos: i32 = 0;
        for ch in text.chars() {
            if self.face.load_char(ch as usize, freetype::face::LoadFlag::RENDER).is_ok() {
                let glyph = self.face.glyph();
                let bitmap = glyph.bitmap();
                let bitmap_left = glyph.bitmap_left();
                let bitmap_top = glyph.bitmap_top();

                let glyph_x = x_pos + bitmap_left;
                let glyph_y = self.ascender - bitmap_top;

                // Copy glyph bitmap to output (with alpha blending)
                for row in 0..bitmap.rows() {
                    for col in 0..bitmap.width() {
                        let px = glyph_x + col;
                        let py = glyph_y + row;

                        if px >= 0 && px < width as i32 && py >= 0 && py < height as i32 {
                            let src_idx = (row * bitmap.pitch() + col) as usize;
                            let alpha = bitmap.buffer()[src_idx] as u32;

                            if alpha > 0 {
                                let dst_idx = ((py as u32 * width + px as u32) * 4) as usize;
                                if alpha == 255 {
                                    pixels[dst_idx] = fg_b;
                                    pixels[dst_idx + 1] = fg_g;
                                    pixels[dst_idx + 2] = fg_r;
                                } else {
                                    // Alpha blend
                                    let inv_alpha = 255 - alpha;
                                    pixels[dst_idx] = ((fg_b as u32 * alpha + pixels[dst_idx] as u32 * inv_alpha) / 255) as u8;
                                    pixels[dst_idx + 1] = ((fg_g as u32 * alpha + pixels[dst_idx + 1] as u32 * inv_alpha) / 255) as u8;
                                    pixels[dst_idx + 2] = ((fg_r as u32 * alpha + pixels[dst_idx + 2] as u32 * inv_alpha) / 255) as u8;
                                }
                            }
                        }
                    }
                }

                x_pos += (glyph.advance().x >> 6) as i32;
            }
        }

        (pixels, width, height)
    }

    /// Measure text width in pixels
    pub fn measure_text(&self, text: &str) -> u32 {
        let mut width: i32 = 0;
        for ch in text.chars() {
            if self.face.load_char(ch as usize, freetype::face::LoadFlag::DEFAULT).is_ok() {
                width += (self.face.glyph().advance().x >> 6) as i32;
            }
        }
        width.max(0) as u32
    }

    /// Truncate text to fit within a given pixel width, adding "..." if needed
    pub fn truncate_text_to_width(&self, text: &str, max_width: u32) -> String {
        if text.is_empty() || max_width == 0 {
            return String::new();
        }

        let full_width = self.measure_text(text);
        if full_width <= max_width {
            return text.to_string();
        }

        // We need to truncate - find how many characters fit with "..."
        let ellipsis = "...";
        let ellipsis_width = self.measure_text(ellipsis);

        if ellipsis_width >= max_width {
            return String::new();
        }

        let available_for_text = max_width - ellipsis_width;
        let mut truncated = String::new();
        let mut current_width = 0u32;

        for ch in text.chars() {
            let ch_str = ch.to_string();
            let ch_width = self.measure_text(&ch_str);

            if current_width + ch_width > available_for_text {
                break;
            }

            truncated.push(ch);
            current_width += ch_width;
        }

        format!("{}{}", truncated, ellipsis)
    }
}

/// Blend BGRA icon pixels with a solid background color, returning BGRX (32-bit) data
pub fn blend_icon_with_background(icon_bgra: &[u8], bg_color: u32, size: u32) -> Vec<u8> {
    let bg_r = ((bg_color >> 16) & 0xFF) as f32;
    let bg_g = ((bg_color >> 8) & 0xFF) as f32;
    let bg_b = (bg_color & 0xFF) as f32;

    let pixel_count = (size * size) as usize;
    let mut result = vec![0u8; pixel_count * 4]; // 32-bit for put_image

    for i in 0..pixel_count {
        let src_idx = i * 4;
        let dst_idx = i * 4;

        if src_idx + 3 < icon_bgra.len() {
            let b = icon_bgra[src_idx] as f32;
            let g = icon_bgra[src_idx + 1] as f32;
            let r = icon_bgra[src_idx + 2] as f32;
            let a = icon_bgra[src_idx + 3] as f32 / 255.0;
            let inv_a = 1.0 - a;

            // Alpha blend with background
            result[dst_idx] = (b * a + bg_b * inv_a) as u8;
            result[dst_idx + 1] = (g * a + bg_g * inv_a) as u8;
            result[dst_idx + 2] = (r * a + bg_r * inv_a) as u8;
            result[dst_idx + 3] = 0; // Padding byte
        }
    }

    result
}

/// Lighten a color by adding to RGB components (for bevel highlight)
pub fn lighten_color(color: u32, amount: u8) -> u32 {
    let r = (((color >> 16) & 0xFF) as u16 + amount as u16).min(255) as u32;
    let g = (((color >> 8) & 0xFF) as u16 + amount as u16).min(255) as u32;
    let b = ((color & 0xFF) as u16 + amount as u16).min(255) as u32;
    (r << 16) | (g << 8) | b
}

/// Darken a color by multiplying RGB components (for bevel shadow and drop shadow)
pub fn darken_color(color: u32, factor: f32) -> u32 {
    let r = (((color >> 16) & 0xFF) as f32 * factor) as u32;
    let g = (((color >> 8) & 0xFF) as f32 * factor) as u32;
    let b = ((color & 0xFF) as f32 * factor) as u32;
    (r << 16) | (g << 8) | b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lighten_color() {
        // Test basic lightening
        assert_eq!(lighten_color(0x000000, 0x20), 0x202020);
        // Test clamping at 255
        assert_eq!(lighten_color(0xFFFFFF, 0x20), 0xFFFFFF);
        assert_eq!(lighten_color(0xF0F0F0, 0x20), 0xFFFFFF);
    }

    #[test]
    fn test_darken_color() {
        // Test basic darkening
        assert_eq!(darken_color(0xFFFFFF, 0.5), 0x7F7F7F);
        assert_eq!(darken_color(0x000000, 0.5), 0x000000);
    }

    #[test]
    fn test_blend_icon_with_background() {
        // Test fully opaque icon pixel
        let icon = vec![0x00, 0xFF, 0x00, 0xFF]; // Green, fully opaque
        let result = blend_icon_with_background(&icon, 0xFF0000, 1); // Red background
        assert_eq!(result[0], 0x00); // B
        assert_eq!(result[1], 0xFF); // G (from icon)
        assert_eq!(result[2], 0x00); // R

        // Test fully transparent icon pixel
        let icon = vec![0x00, 0xFF, 0x00, 0x00]; // Green, fully transparent
        let result = blend_icon_with_background(&icon, 0xFF0000, 1); // Red background
        assert_eq!(result[0], 0x00); // B (from bg)
        assert_eq!(result[1], 0x00); // G (from bg)
        assert_eq!(result[2], 0xFF); // R (from bg)
    }
}
