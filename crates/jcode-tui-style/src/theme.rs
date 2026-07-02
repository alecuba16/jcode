use crate::color;
use crate::color::rgb;
use ratatui::prelude::*;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{OnceLock, RwLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ThemeColor {
    User,
    Ai,
    Tool,
    FileLink,
    Dim,
    Accent,
    SystemMessage,
    Queued,
    Asap,
    Pending,
    UserText,
    UserBg,
    AiText,
    HeaderIcon,
    HeaderName,
    HeaderSession,
}

#[derive(Debug, Clone)]
pub struct Theme {
    colors: BTreeMap<ThemeColor, Color>,
}

impl Theme {
    fn color(&self, key: ThemeColor) -> Color {
        self.colors.get(&key).copied().unwrap_or(Color::Reset)
    }
}

#[derive(Debug, Deserialize)]
struct ThemeFile {
    colors: BTreeMap<String, String>,
}

static ACTIVE_THEME: OnceLock<RwLock<Theme>> = OnceLock::new();

fn active_theme() -> &'static RwLock<Theme> {
    ACTIVE_THEME.get_or_init(|| RwLock::new(light_theme()))
}

pub fn set_theme(name: &str, themes_dir: Option<&Path>) -> anyhow::Result<()> {
    let theme = load_theme(name, themes_dir)?;
    *active_theme()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = theme;
    Ok(())
}

pub fn load_theme(name: &str, themes_dir: Option<&Path>) -> anyhow::Result<Theme> {
    let name = name.trim();
    match name.to_ascii_lowercase().as_str() {
        "" | "light" => Ok(light_theme()),
        "dark" => Ok(dark_theme()),
        "system" => Ok(system_theme()),
        _ => load_custom_theme(name, themes_dir),
    }
}

pub fn available_theme_names(themes_dir: Option<&Path>) -> Vec<String> {
    let mut names = vec![
        "light".to_string(),
        "dark".to_string(),
        "system".to_string(),
    ];
    if let Some(dir) = themes_dir {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) == Some("toml") {
                    if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) {
                        let stem = stem.to_string();
                        if !names.iter().any(|name| name == &stem) {
                            names.push(stem);
                        }
                    }
                }
            }
        }
    }
    names
}

fn themed_color(key: ThemeColor) -> Color {
    active_theme()
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .color(key)
}

fn load_custom_theme(name: &str, themes_dir: Option<&Path>) -> anyhow::Result<Theme> {
    if name.contains('/') || name.contains('\\') || name == "." || name == ".." {
        anyhow::bail!(
            "Invalid theme name '{}': use a file stem from the themes directory",
            name
        );
    }
    let dir = themes_dir.ok_or_else(|| anyhow::anyhow!("No themes directory configured"))?;
    let path = dir.join(format!("{name}.toml"));
    let content = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("Failed to read theme {}: {}", path.display(), e))?;
    let file: ThemeFile = toml::from_str(&content)
        .map_err(|e| anyhow::anyhow!("Failed to parse theme {}: {}", path.display(), e))?;

    let mut theme = light_theme();
    for (raw_key, raw_value) in file.colors {
        let key = parse_theme_color(&raw_key).ok_or_else(|| {
            anyhow::anyhow!("Unknown theme color '{}': {}", raw_key, path.display())
        })?;
        let value = parse_color(&raw_value).ok_or_else(|| {
            anyhow::anyhow!("Invalid theme color '{}': {}", raw_value, path.display())
        })?;
        theme.colors.insert(key, value);
    }
    Ok(theme)
}

fn parse_theme_color(raw: &str) -> Option<ThemeColor> {
    match raw.trim().replace('-', "_").to_ascii_lowercase().as_str() {
        "user" | "user_color" => Some(ThemeColor::User),
        "ai" | "ai_color" => Some(ThemeColor::Ai),
        "tool" | "tool_color" => Some(ThemeColor::Tool),
        "file_link" | "file_link_color" => Some(ThemeColor::FileLink),
        "dim" | "dim_color" => Some(ThemeColor::Dim),
        "accent" | "accent_color" => Some(ThemeColor::Accent),
        "system_message" | "system_message_color" => Some(ThemeColor::SystemMessage),
        "queued" | "queued_color" => Some(ThemeColor::Queued),
        "asap" | "asap_color" => Some(ThemeColor::Asap),
        "pending" | "pending_color" => Some(ThemeColor::Pending),
        "user_text" => Some(ThemeColor::UserText),
        "user_bg" => Some(ThemeColor::UserBg),
        "ai_text" => Some(ThemeColor::AiText),
        "header_icon" | "header_icon_color" => Some(ThemeColor::HeaderIcon),
        "header_name" | "header_name_color" => Some(ThemeColor::HeaderName),
        "header_session" | "header_session_color" => Some(ThemeColor::HeaderSession),
        _ => None,
    }
}

fn parse_color(raw: &str) -> Option<Color> {
    let raw = raw.trim();
    if raw.eq_ignore_ascii_case("reset") || raw.eq_ignore_ascii_case("default") {
        return Some(Color::Reset);
    }
    let hex = raw.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(rgb(r, g, b))
}

fn light_theme() -> Theme {
    Theme {
        colors: BTreeMap::from([
            (ThemeColor::User, rgb(28, 88, 160)),
            (ThemeColor::Ai, rgb(38, 120, 72)),
            (ThemeColor::Tool, rgb(92, 92, 92)),
            (ThemeColor::FileLink, rgb(28, 92, 176)),
            (ThemeColor::Dim, rgb(105, 105, 105)),
            (ThemeColor::Accent, rgb(112, 72, 170)),
            (ThemeColor::SystemMessage, rgb(176, 58, 126)),
            (ThemeColor::Queued, rgb(158, 106, 0)),
            (ThemeColor::Asap, rgb(0, 112, 150)),
            (ThemeColor::Pending, rgb(112, 112, 112)),
            (ThemeColor::UserText, rgb(30, 30, 34)),
            (ThemeColor::UserBg, rgb(244, 241, 236)),
            (ThemeColor::AiText, rgb(36, 38, 40)),
            (ThemeColor::HeaderIcon, rgb(0, 122, 150)),
            (ThemeColor::HeaderName, rgb(44, 80, 122)),
            (ThemeColor::HeaderSession, rgb(34, 34, 38)),
        ]),
    }
}

fn dark_theme() -> Theme {
    Theme {
        colors: BTreeMap::from([
            (ThemeColor::User, rgb(138, 180, 248)),
            (ThemeColor::Ai, rgb(129, 199, 132)),
            (ThemeColor::Tool, rgb(120, 120, 120)),
            (ThemeColor::FileLink, rgb(180, 200, 255)),
            (ThemeColor::Dim, rgb(80, 80, 80)),
            (ThemeColor::Accent, rgb(186, 139, 255)),
            (ThemeColor::SystemMessage, rgb(255, 170, 220)),
            (ThemeColor::Queued, rgb(255, 193, 7)),
            (ThemeColor::Asap, rgb(110, 210, 255)),
            (ThemeColor::Pending, rgb(140, 140, 140)),
            (ThemeColor::UserText, rgb(245, 245, 255)),
            (ThemeColor::UserBg, rgb(35, 40, 50)),
            (ThemeColor::AiText, rgb(220, 220, 215)),
            (ThemeColor::HeaderIcon, rgb(120, 210, 230)),
            (ThemeColor::HeaderName, rgb(190, 210, 235)),
            (ThemeColor::HeaderSession, rgb(255, 255, 255)),
        ]),
    }
}

fn system_theme() -> Theme {
    Theme {
        colors: BTreeMap::from([
            (ThemeColor::User, Color::Reset),
            (ThemeColor::Ai, Color::Reset),
            (ThemeColor::Tool, Color::Reset),
            (ThemeColor::FileLink, Color::Reset),
            (ThemeColor::Dim, Color::Reset),
            (ThemeColor::Accent, Color::Reset),
            (ThemeColor::SystemMessage, Color::Reset),
            (ThemeColor::Queued, Color::Reset),
            (ThemeColor::Asap, Color::Reset),
            (ThemeColor::Pending, Color::Reset),
            (ThemeColor::UserText, Color::Reset),
            (ThemeColor::UserBg, Color::Reset),
            (ThemeColor::AiText, Color::Reset),
            (ThemeColor::HeaderIcon, Color::Reset),
            (ThemeColor::HeaderName, Color::Reset),
            (ThemeColor::HeaderSession, Color::Reset),
        ]),
    }
}

pub fn user_color() -> Color {
    themed_color(ThemeColor::User)
}
pub fn ai_color() -> Color {
    themed_color(ThemeColor::Ai)
}
pub fn tool_color() -> Color {
    themed_color(ThemeColor::Tool)
}
pub fn file_link_color() -> Color {
    themed_color(ThemeColor::FileLink)
}
pub fn dim_color() -> Color {
    themed_color(ThemeColor::Dim)
}
pub fn accent_color() -> Color {
    themed_color(ThemeColor::Accent)
}
pub fn system_message_color() -> Color {
    themed_color(ThemeColor::SystemMessage)
}
pub fn queued_color() -> Color {
    themed_color(ThemeColor::Queued)
}
pub fn asap_color() -> Color {
    themed_color(ThemeColor::Asap)
}
pub fn pending_color() -> Color {
    themed_color(ThemeColor::Pending)
}
pub fn user_text() -> Color {
    themed_color(ThemeColor::UserText)
}
pub fn user_bg() -> Color {
    themed_color(ThemeColor::UserBg)
}
pub fn ai_text() -> Color {
    themed_color(ThemeColor::AiText)
}
pub fn header_icon_color() -> Color {
    themed_color(ThemeColor::HeaderIcon)
}
pub fn header_name_color() -> Color {
    themed_color(ThemeColor::HeaderName)
}
pub fn header_session_color() -> Color {
    themed_color(ThemeColor::HeaderSession)
}

// Spinner frames for animated status. Keep these single-cell because the fast
// spinner-only renderer patches one status cell between full TUI redraws. This
// sequence should read as a circular spin, not a grow/recede pulse.
const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Frame rate for slow, full-line "liveness" indicators that can only be
/// repainted by a full TUI redraw (e.g. the running-tool progress bar) when
/// decorative animations are disabled (Minimal tier, SSH, WSL, etc.). These
/// ride the ~1 Hz passive-liveness redraw, so advancing them faster would just
/// skip frames. Keep this slow so they read as alive without forcing more
/// expensive full-frame redraws.
pub const LIVENESS_INDICATOR_FPS: f32 = 1.5;

/// Frame rate for the low-cost single-cell circular spinner when decorative
/// animations are disabled. Unlike the full-line indicators above, this spinner
/// is patched by the cheap one-cell fast path between full redraws, so it can
/// animate at a smooth, responsive cadence (well above ~1 Hz) while still
/// staying very light on resources. Keep this in sync with the spinner-only
/// tick interval in the TUI run loop (`STATUS_SPINNER_ONLY_INTERVAL`, 80ms) so
/// each tick lands on exactly one new frame.
pub const LIVENESS_SPINNER_FPS: f32 = 12.5;

pub fn spinner_frame_index(elapsed: f32, fps: f32) -> usize {
    ((elapsed * fps) as usize) % SPINNER_FRAMES.len()
}

pub fn spinner_frame(elapsed: f32, fps: f32) -> &'static str {
    SPINNER_FRAMES[spinner_frame_index(elapsed, fps)]
}

pub fn activity_indicator_frame_index(
    elapsed: f32,
    fps: f32,
    enable_decorative_animations: bool,
) -> usize {
    if enable_decorative_animations {
        spinner_frame_index(elapsed, fps)
    } else {
        // Keep ticking at the smooth liveness rate instead of freezing on a
        // single frame. The single-cell fast path repaints this cheaply, so it
        // can animate well above ~1 Hz without a full-frame redraw.
        spinner_frame_index(elapsed, LIVENESS_SPINNER_FPS)
    }
}

pub fn activity_indicator(
    elapsed: f32,
    fps: f32,
    enable_decorative_animations: bool,
) -> &'static str {
    SPINNER_FRAMES[activity_indicator_frame_index(elapsed, fps, enable_decorative_animations)]
}

/// Convert HSL to RGB (h in 0-360, s and l in 0-1)
/// Chroma color based on position and time - creates flowing rainbow wave
/// Calculate chroma color with fade-in from dim during startup
/// Calculate smooth animated color for the header (single color, no position)
pub fn color_to_floats(c: Color, fallback: (f32, f32, f32)) -> (f32, f32, f32) {
    match c {
        Color::Rgb(r, g, b) => (r as f32, g as f32, b as f32),
        Color::Indexed(n) => {
            let (r, g, b) = color::indexed_to_rgb(n);
            (r as f32, g as f32, b as f32)
        }
        _ => fallback,
    }
}

pub fn blend_color(from: Color, to: Color, t: f32) -> Color {
    let (fr, fg, fb) = color_to_floats(from, (80.0, 80.0, 80.0));
    let (tr, tg, tb) = color_to_floats(to, (200.0, 200.0, 200.0));
    let r = fr + (tr - fr) * t;
    let g = fg + (tg - fg) * t;
    let b = fb + (tb - fb) * t;
    rgb(
        r.clamp(0.0, 255.0) as u8,
        g.clamp(0.0, 255.0) as u8,
        b.clamp(0.0, 255.0) as u8,
    )
}

pub fn rainbow_prompt_color(distance: usize) -> Color {
    // Rainbow colors (hue progression): red -> orange -> yellow -> green -> cyan -> blue -> violet
    const RAINBOW: [(u8, u8, u8); 7] = [
        (255, 80, 80),   // Red (softened)
        (255, 160, 80),  // Orange
        (255, 230, 80),  // Yellow
        (80, 220, 100),  // Green
        (80, 200, 220),  // Cyan
        (100, 140, 255), // Blue
        (180, 100, 255), // Violet
    ];

    // Gray target (dim_color())
    const GRAY: (u8, u8, u8) = (80, 80, 80);

    // Exponential decay factor - how quickly we fade to gray
    // decay = e^(-distance * rate), rate of ~0.4 gives nice falloff
    let decay = (-0.4 * distance as f32).exp();

    // Select rainbow color based on distance (cycle through)
    let rainbow_idx = distance.min(RAINBOW.len() - 1);
    let (r, g, b) = RAINBOW[rainbow_idx];

    // Blend rainbow color with gray based on decay
    // At distance 0: 100% rainbow, as distance increases: approaches gray
    let blend = |rainbow: u8, gray: u8| -> u8 {
        (rainbow as f32 * decay + gray as f32 * (1.0 - decay)) as u8
    };

    rgb(blend(r, GRAY.0), blend(g, GRAY.1), blend(b, GRAY.2))
}

pub fn prompt_entry_color(base: Color, t: f32) -> Color {
    let peak = rgb(255, 230, 120);
    // Quick pulse in/out over the animation window.
    let phase = if t < 0.5 { t * 2.0 } else { (1.0 - t) * 2.0 };
    blend_color(base, peak, phase.clamp(0.0, 1.0) * 0.7)
}

pub fn prompt_entry_bg_color(base: Color, t: f32) -> Color {
    let spotlight = rgb(58, 66, 82);
    let ease_in = 1.0 - (1.0 - t).powi(3);
    let ease_out = (1.0 - t).powi(2);
    let phase = (ease_in * ease_out * 1.65).clamp(0.0, 1.0);
    blend_color(base, spotlight, phase * 0.85)
}

pub fn prompt_entry_shimmer_color(base: Color, pos: f32, t: f32) -> Color {
    let travel = (t * 1.15).clamp(0.0, 1.0);
    let width = 0.18;
    let dist = (pos - travel).abs();
    let shimmer = (1.0 - (dist / width).clamp(0.0, 1.0)).powf(2.2);
    let pulse = (1.0 - t).powf(0.55);
    let highlight = rgb(255, 248, 210);
    blend_color(base, highlight, shimmer * pulse * 0.7)
}

/// Generate an animated color that pulses between two colors
pub fn animated_tool_color(elapsed: f32, enable_decorative_animations: bool) -> Color {
    if !enable_decorative_animations {
        return tool_color();
    }

    // Cycle period of ~1.5 seconds
    let t = (elapsed * 2.0).sin() * 0.5 + 0.5; // 0.0 to 1.0

    // Interpolate between cyan and purple
    let r = (80.0 + t * 106.0) as u8; // 80 -> 186
    let g = (200.0 - t * 61.0) as u8; // 200 -> 139
    let b = (220.0 + t * 35.0) as u8; // 220 -> 255

    rgb(r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_theme_dir() -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "jcode-tui-style-theme-test-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create theme dir");
        dir
    }

    #[test]
    fn spinner_frames_are_circular_braille_sequence() {
        assert_eq!(
            SPINNER_FRAMES,
            &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]
        );
    }

    #[test]
    fn spinner_frame_wraps_at_sequence_length() {
        let fps = 10.0;
        assert_eq!(spinner_frame(0.0, fps), "⠋");
        assert_eq!(spinner_frame(0.9, fps), "⠏");
        assert_eq!(spinner_frame(1.0, fps), "⠋");
    }

    #[test]
    fn activity_indicator_still_advances_without_decorative_animations() {
        // With decorative animations disabled the single-cell spinner must keep
        // ticking instead of freezing on one frame.
        let first = activity_indicator(0.0, 12.5, false);
        let later = activity_indicator(1.0, 12.5, false);
        assert!(SPINNER_FRAMES.contains(&first));
        assert_ne!(
            first, later,
            "liveness spinner should advance within one second"
        );
    }

    #[test]
    fn liveness_spinner_advances_smoothly_within_a_few_frames() {
        // The single-cell fast path patches one status cell per 80ms tick, so the
        // non-decorative liveness spinner should advance well faster than ~1 Hz
        // (it should not still read as frozen between consecutive fast-path ticks).
        let frame_at = |elapsed: f32| activity_indicator(elapsed, 12.5, false);
        // One 80ms fast-path tick should already move to the next frame.
        assert_ne!(
            frame_at(0.0),
            frame_at(0.08),
            "liveness spinner should advance every fast-path tick (80ms)"
        );
        // It must be meaningfully faster than the old ~1.5 Hz cadence.
        assert!(
            LIVENESS_SPINNER_FPS >= 8.0,
            "liveness spinner should animate at a smooth, responsive rate"
        );
    }

    #[test]
    fn light_theme_uses_off_white_user_background() {
        let theme = load_theme("light", None).expect("light theme");
        assert_eq!(theme.color(ThemeColor::UserBg), rgb(244, 241, 236));
        assert_eq!(theme.color(ThemeColor::SystemMessage), rgb(176, 58, 126));
    }

    #[test]
    fn system_theme_uses_terminal_defaults() {
        let theme = load_theme("system", None).expect("system theme");
        assert_eq!(theme.color(ThemeColor::User), Color::Reset);
        assert_eq!(theme.color(ThemeColor::UserBg), Color::Reset);
    }

    #[test]
    fn custom_theme_load_preserves_file_stem_case() {
        let dir = temp_theme_dir();
        std::fs::write(
            dir.join("Solarized.toml"),
            r##"
[colors]
user = "#123456"
user_bg = "#abcdef"
system_message = "default"
"##,
        )
        .expect("write theme");

        let names = available_theme_names(Some(&dir));
        assert!(names.iter().any(|name| name == "Solarized"));

        let theme = load_theme("Solarized", Some(&dir)).expect("custom theme");
        assert_eq!(theme.color(ThemeColor::User), rgb(0x12, 0x34, 0x56));
        assert_eq!(theme.color(ThemeColor::UserBg), rgb(0xab, 0xcd, 0xef));
        assert_eq!(theme.color(ThemeColor::SystemMessage), Color::Reset);

        std::fs::remove_dir_all(dir).expect("remove theme dir");
    }

    #[test]
    fn set_theme_applies_custom_theme() {
        let dir = temp_theme_dir();
        std::fs::write(
            dir.join("Custom.toml"),
            r##"
[colors]
user = "#010203"
"##,
        )
        .expect("write theme");

        set_theme("Custom", Some(&dir)).expect("set custom theme");
        assert_eq!(user_color(), rgb(1, 2, 3));
        set_theme("light", None).expect("reset theme");

        std::fs::remove_dir_all(dir).expect("remove theme dir");
    }

    #[test]
    fn custom_theme_rejects_traversal_names() {
        let dir = temp_theme_dir();
        for name in ["../x", "nested/theme", "nested\\theme"] {
            let error = load_theme(name, Some(&dir)).expect_err("theme name should be rejected");
            assert!(
                error.to_string().contains("Invalid theme name"),
                "unexpected error for {name}: {error}"
            );
        }
        std::fs::remove_dir_all(dir).expect("remove theme dir");
    }
}
