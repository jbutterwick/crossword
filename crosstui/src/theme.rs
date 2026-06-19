//! Color themes.
//!
//! A theme is a small palette of base colors (loaded from `themes.toml`); the
//! UI maps those onto roles via the accessor methods below. The currently
//! active theme lives in a global so every widget can read it during render
//! without threading it through call sites.
// ponytail: one global theme for the whole UI — fine for a single-window TUI;
// revisit only if rendering ever goes multi-threaded.

use std::fs;
use std::path::Path;
use std::sync::RwLock;

use ratatui::style::Color;
use serde::Deserialize;

/// A theme's base palette. `Copy` so it can live in the global cheaply.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub bg: Color,
    pub fg: Color,
    pub surface: Color,
    pub muted: Color,
    pub red: Color,
    pub green: Color,
    pub yellow: Color,
    pub blue: Color,
    pub magenta: Color,
}

impl Theme {
    /// Neutral dark fallback, used until a theme is loaded/selected.
    pub const CLASSIC: Theme = Theme {
        bg: Color::Rgb(0x1e, 0x1e, 0x1e),
        fg: Color::Rgb(0xe0, 0xe0, 0xe0),
        surface: Color::Rgb(0x3a, 0x3a, 0x3a),
        muted: Color::Rgb(0x80, 0x80, 0x80),
        red: Color::Rgb(0xff, 0x6b, 0x6b),
        green: Color::Rgb(0x5f, 0xd7, 0x5f),
        yellow: Color::Rgb(0xe5, 0xc0, 0x7b),
        blue: Color::Rgb(0x5f, 0xaf, 0xff),
        magenta: Color::Rgb(0xd7, 0x5f, 0xd7),
    };

    // --- role accessors: where each base color is used in the UI ---

    /// White-square background.
    pub fn square(&self) -> Color {
        self.fg
    }
    /// Letter color on a square (and on cursor/word cells).
    pub fn square_fg(&self) -> Color {
        self.bg
    }
    /// Black-square fill.
    pub fn block(&self) -> Color {
        self.surface
    }
    /// Cursor cell background.
    pub fn cursor(&self) -> Color {
        self.red
    }
    /// Current-word cell background.
    pub fn word(&self) -> Color {
        self.yellow
    }
    /// Titles, brand, borders.
    pub fn accent(&self) -> Color {
        self.blue
    }
    /// Clue numbers.
    pub fn clue_number(&self) -> Color {
        self.magenta
    }
    /// Selected-row highlight background.
    pub fn sel_bg(&self) -> Color {
        self.yellow
    }
    /// Selected-row highlight foreground.
    pub fn sel_fg(&self) -> Color {
        self.bg
    }
}

/// A theme plus its display name (the name isn't needed during render, so it's
/// kept out of [`Theme`] to keep that `Copy`).
pub struct ThemeEntry {
    pub name: String,
    pub theme: Theme,
}

static CURRENT: RwLock<Theme> = RwLock::new(Theme::CLASSIC);

/// The active theme. Read this in render code.
pub fn current() -> Theme {
    *CURRENT.read().unwrap()
}

/// Sets the active theme (used for selection and live preview).
pub fn set(theme: Theme) {
    *CURRENT.write().unwrap() = theme;
}

const THEMES_FILE: &str = "themes.toml";
const SELECTED_FILE: &str = "theme.txt";

/// Shipped default themes, written to the data dir on first run so the user can edit them.
pub const BOOTSTRAP: &str = include_str!("../themes.toml");

#[derive(Deserialize)]
struct ThemesFile {
    theme: Vec<RawTheme>,
}

#[derive(Deserialize)]
struct RawTheme {
    name: String,
    bg: String,
    fg: String,
    surface: String,
    muted: String,
    red: String,
    green: String,
    yellow: String,
    blue: String,
    magenta: String,
}

/// Parses a `#rrggbb` hex string, falling back to a loud magenta on malformed
/// input so mistakes in `themes.toml` are visible rather than silent.
fn parse_color(s: &str) -> Color {
    let s = s.trim().trim_start_matches('#');
    if s.len() == 6 {
        if let (Ok(r), Ok(g), Ok(b)) = (
            u8::from_str_radix(&s[0..2], 16),
            u8::from_str_radix(&s[2..4], 16),
            u8::from_str_radix(&s[4..6], 16),
        ) {
            return Color::Rgb(r, g, b);
        }
    }
    Color::Magenta
}

impl From<RawTheme> for ThemeEntry {
    fn from(r: RawTheme) -> Self {
        ThemeEntry {
            name: r.name,
            theme: Theme {
                bg: parse_color(&r.bg),
                fg: parse_color(&r.fg),
                surface: parse_color(&r.surface),
                muted: parse_color(&r.muted),
                red: parse_color(&r.red),
                green: parse_color(&r.green),
                yellow: parse_color(&r.yellow),
                blue: parse_color(&r.blue),
                magenta: parse_color(&r.magenta),
            },
        }
    }
}

/// Loads themes from `dir/themes.toml`, writing the bootstrap copy first if it's
/// missing. Always returns at least one theme.
pub fn load(dir: &Path) -> Vec<ThemeEntry> {
    let path = dir.join(THEMES_FILE);
    if !path.exists() {
        let _ = fs::write(&path, BOOTSTRAP);
    }
    let text = fs::read_to_string(&path).unwrap_or_else(|_| BOOTSTRAP.to_string());
    let parsed: ThemesFile = toml::from_str(&text).unwrap_or(ThemesFile { theme: vec![] });

    let mut entries: Vec<ThemeEntry> = parsed.theme.into_iter().map(Into::into).collect();
    if entries.is_empty() {
        entries.push(ThemeEntry {
            name: "Classic".to_string(),
            theme: Theme::CLASSIC,
        });
    }
    entries
}

/// The name of the previously selected theme, if any.
pub fn load_selected(dir: &Path) -> Option<String> {
    fs::read_to_string(dir.join(SELECTED_FILE))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Persists the selected theme name.
pub fn save_selected(dir: &Path, name: &str) {
    let _ = fs::write(dir.join(SELECTED_FILE), name);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex() {
        assert_eq!(parse_color("#1e1e2e"), Color::Rgb(0x1e, 0x1e, 0x2e));
        assert_eq!(parse_color("1e1e2e"), Color::Rgb(0x1e, 0x1e, 0x2e));
        assert_eq!(parse_color("nope"), Color::Magenta);
    }

    #[test]
    fn bootstrap_is_valid() {
        let parsed: ThemesFile = toml::from_str(BOOTSTRAP).expect("themes.toml parses");
        assert!(!parsed.theme.is_empty());
        // Every color must be valid hex (never the magenta fallback).
        for t in &parsed.theme {
            for (field, value) in [
                ("bg", &t.bg),
                ("fg", &t.fg),
                ("surface", &t.surface),
                ("muted", &t.muted),
                ("red", &t.red),
                ("green", &t.green),
                ("yellow", &t.yellow),
                ("blue", &t.blue),
                ("magenta", &t.magenta),
            ] {
                assert_ne!(
                    parse_color(value),
                    Color::Magenta,
                    "theme {:?} field {} = {:?} is not valid hex",
                    t.name,
                    field,
                    value
                );
            }
        }
    }
}
