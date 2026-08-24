use crate::color;
use crate::color::rgb;
use ratatui::prelude::*;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicPtr, Ordering};

#[repr(usize)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ThemeColor {
    Background,
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
    InputText,
    InputBg,
    AiText,
    Bold,
    MarkdownText,
    HeaderIcon,
    HeaderName,
    HeaderSession,
    /// Success / additions.
    Success,
    /// Warnings.
    Warning,
    /// Errors / deletions.
    Error,
    /// Informational highlights.
    Info,
    /// Borders and rules.
    Border,
    /// Selected row background.
    SelectionBg,
}

const THEME_COLOR_COUNT: usize = ThemeColor::SelectionBg as usize + 1;

#[derive(Debug, Clone)]
pub struct Theme {
    name: String,
    colors: BTreeMap<ThemeColor, Color>,
}

impl Theme {
    fn new(name: impl Into<String>, colors: BTreeMap<ThemeColor, Color>) -> Self {
        Self {
            name: name.into(),
            colors,
        }
    }

    fn color(&self, key: ThemeColor) -> Color {
        self.colors.get(&key).copied().unwrap_or(Color::Reset)
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Debug, Deserialize)]
struct ThemeFile {
    colors: BTreeMap<String, String>,
}

const BUILTIN_THEMES: &[&str] = &["system", "light", "dark"];
static ACTIVE_THEME: OnceLock<AtomicPtr<ThemeSnapshot>> = OnceLock::new();

#[derive(Debug, Clone)]
struct ThemeSnapshot {
    name: String,
    colors: [Color; THEME_COLOR_COUNT],
    uses_terminal_adaptation: bool,
}

impl ThemeSnapshot {
    fn from_theme(theme: &Theme) -> Self {
        let name = theme.name().to_string();
        let uses_terminal_adaptation = theme_uses_terminal_adaptation(&name);
        Self {
            name,
            colors: [
                theme.color(ThemeColor::Background),
                theme.color(ThemeColor::User),
                theme.color(ThemeColor::Ai),
                theme.color(ThemeColor::Tool),
                theme.color(ThemeColor::FileLink),
                theme.color(ThemeColor::Dim),
                theme.color(ThemeColor::Accent),
                theme.color(ThemeColor::SystemMessage),
                theme.color(ThemeColor::Queued),
                theme.color(ThemeColor::Asap),
                theme.color(ThemeColor::Pending),
                theme.color(ThemeColor::UserText),
                theme.color(ThemeColor::UserBg),
                theme.color(ThemeColor::InputText),
                theme.color(ThemeColor::InputBg),
                theme.color(ThemeColor::AiText),
                theme.color(ThemeColor::Bold),
                theme.color(ThemeColor::MarkdownText),
                theme.color(ThemeColor::HeaderIcon),
                theme.color(ThemeColor::HeaderName),
                theme.color(ThemeColor::HeaderSession),
                theme.color(ThemeColor::Success),
                theme.color(ThemeColor::Warning),
                theme.color(ThemeColor::Error),
                theme.color(ThemeColor::Info),
                theme.color(ThemeColor::Border),
                theme.color(ThemeColor::SelectionBg),
            ],
            uses_terminal_adaptation,
        }
    }

    fn color(&self, key: ThemeColor) -> Color {
        self.colors[key as usize]
    }
}

fn active_theme() -> &'static AtomicPtr<ThemeSnapshot> {
    ACTIVE_THEME.get_or_init(|| AtomicPtr::new(std::ptr::null_mut()))
}

fn active_theme_snapshot() -> &'static ThemeSnapshot {
    let slot = active_theme();
    let current = slot.load(Ordering::Acquire);
    if !current.is_null() {
        // SAFETY: snapshots are intentionally leaked after publication so active
        // render readers can dereference them without a lock or epoch guard.
        return unsafe { &*current };
    }

    let snapshot = Box::into_raw(Box::new(ThemeSnapshot::from_theme(&system_theme())));
    match slot.compare_exchange(
        std::ptr::null_mut(),
        snapshot,
        Ordering::AcqRel,
        Ordering::Acquire,
    ) {
        Ok(_) => unsafe { &*snapshot },
        Err(existing) => {
            // Another thread won initialization. Drop our unpublished snapshot.
            unsafe {
                drop(Box::from_raw(snapshot));
                &*existing
            }
        }
    }
}

pub fn active_theme_name() -> String {
    active_theme_snapshot().name.clone()
}

/// Whether the active palette should be post-processed by the terminal
/// light/dark adapter from `theme_mode`.
///
/// `system` and `light` reuse jcode's native palette plus master's rendered
/// buffer adapter. Custom TOML themes are explicit palettes, so adapting them
/// again would surprise users and can invert hand-picked colors.
pub fn active_theme_uses_terminal_adaptation() -> bool {
    active_theme_snapshot().uses_terminal_adaptation
}

fn theme_uses_terminal_adaptation(name: &str) -> bool {
    name.eq_ignore_ascii_case("system") || name.eq_ignore_ascii_case("light")
}

pub fn set_theme(name: &str, themes_dir: Option<&Path>) -> anyhow::Result<()> {
    let theme = load_theme(name, themes_dir)?;
    let snapshot = Box::into_raw(Box::new(ThemeSnapshot::from_theme(&theme)));
    // Keep old snapshots alive to make lock-free render reads safe while a theme
    // change is racing. Theme changes are rare and each snapshot is tiny.
    let _old = active_theme().swap(snapshot, Ordering::AcqRel);
    Ok(())
}

pub fn load_theme(name: &str, themes_dir: Option<&Path>) -> anyhow::Result<Theme> {
    let name = name.trim();
    match name.to_ascii_lowercase().as_str() {
        "" | "auto" | "system" => Ok(system_theme()),
        "light" => Ok(system_palette_named("light")),
        "dark" => Ok(dark_theme()),
        _ => load_custom_theme(name, themes_dir),
    }
}

pub fn available_theme_names(themes_dir: Option<&Path>) -> Vec<String> {
    let mut names = BUILTIN_THEMES
        .iter()
        .map(|name| (*name).to_string())
        .collect::<Vec<_>>();
    if let Some(dir) = themes_dir
        && let Ok(entries) = std::fs::read_dir(dir)
    {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("toml") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if !is_safe_custom_theme_name(stem) {
                continue;
            }
            if !names.iter().any(|name| name == stem) {
                names.push(stem.to_string());
            }
        }
    }
    names
}

fn themed_color(key: ThemeColor) -> Color {
    active_theme_snapshot().color(key)
}

fn load_custom_theme(name: &str, themes_dir: Option<&Path>) -> anyhow::Result<Theme> {
    if !is_safe_custom_theme_name(name) {
        anyhow::bail!(
            "Invalid theme name '{}': use only ASCII letters, numbers, '-' or '_'",
            name
        );
    }
    let dir = themes_dir.ok_or_else(|| anyhow::anyhow!("No themes directory configured"))?;
    let path = dir.join(format!("{name}.toml"));
    let content = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("Failed to read theme {}: {}", path.display(), e))?;
    let file: ThemeFile = toml::from_str(&content)
        .map_err(|e| anyhow::anyhow!("Failed to parse theme {}: {}", path.display(), e))?;

    let mut theme = system_palette_named(name);
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

fn is_safe_custom_theme_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn parse_theme_color(raw: &str) -> Option<ThemeColor> {
    match raw.trim().replace('-', "_").to_ascii_lowercase().as_str() {
        "background" | "background_color" | "app_bg" | "app_background" => {
            Some(ThemeColor::Background)
        }
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
        "input_text" => Some(ThemeColor::InputText),
        "input_bg" => Some(ThemeColor::InputBg),
        "ai_text" => Some(ThemeColor::AiText),
        "bold" | "bold_color" => Some(ThemeColor::Bold),
        "markdown_text" | "md_text" => Some(ThemeColor::MarkdownText),
        "header_icon" | "header_icon_color" => Some(ThemeColor::HeaderIcon),
        "header_name" | "header_name_color" => Some(ThemeColor::HeaderName),
        "header_session" | "header_session_color" => Some(ThemeColor::HeaderSession),
        "success" | "success_color" => Some(ThemeColor::Success),
        "warning" | "warning_color" => Some(ThemeColor::Warning),
        "error" | "error_color" => Some(ThemeColor::Error),
        "info" | "info_color" => Some(ThemeColor::Info),
        "border" | "border_color" => Some(ThemeColor::Border),
        "selection_bg" | "selection_bg_color" => Some(ThemeColor::SelectionBg),
        _ => None,
    }
}

fn named_color(name: &str) -> Option<Color> {
    // ratatui named colors. Useful for users who want a terminal-quantized
    // color (256-color friendly) instead of an explicit RGB value.
    Some(match name {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "gray" | "grey" => Color::Gray,
        "darkgray" | "darkgrey" => Color::DarkGray,
        "lightred" => Color::LightRed,
        "lightgreen" => Color::LightGreen,
        "lightyellow" => Color::LightYellow,
        "lightblue" => Color::LightBlue,
        "lightmagenta" => Color::LightMagenta,
        "lightcyan" => Color::LightCyan,
        "white" => Color::White,
        _ => return None,
    })
}

fn parse_color(raw: &str) -> Option<Color> {
    let raw = raw.trim();
    if raw.eq_ignore_ascii_case("reset") || raw.eq_ignore_ascii_case("default") {
        return Some(Color::Reset);
    }
    // Named colors (red, blue, light-cyan, ...) for user convenience.
    if let Some(color) = named_color(&raw.to_ascii_lowercase().replace(['-', ' '], "")) {
        return Some(color);
    }
    let hex = raw.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}

fn system_theme() -> Theme {
    system_palette_named("system")
}

fn system_palette_named(name: &str) -> Theme {
    Theme::new(
        name,
        BTreeMap::from([
            (ThemeColor::Background, Color::Reset),
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
            (ThemeColor::InputText, Color::Reset),
            (ThemeColor::InputBg, Color::Reset),
            (ThemeColor::AiText, rgb(220, 220, 215)),
            (ThemeColor::Bold, rgb(240, 240, 235)),
            (ThemeColor::MarkdownText, rgb(200, 200, 195)),
            (ThemeColor::HeaderIcon, rgb(120, 210, 230)),
            (ThemeColor::HeaderName, rgb(190, 210, 235)),
            (ThemeColor::HeaderSession, rgb(255, 255, 255)),
            // Semantic accents. These mirror crate::palette::Role defaults so
            // the built-in theme is byte-identical to the historical palette,
            // but custom TOML themes can now override them independently.
            (ThemeColor::Success, rgb(100, 200, 100)),
            (ThemeColor::Warning, rgb(255, 200, 100)),
            (ThemeColor::Error, rgb(255, 100, 100)),
            (ThemeColor::Info, rgb(140, 180, 255)),
            (ThemeColor::Border, rgb(100, 100, 110)),
            (ThemeColor::SelectionBg, rgb(60, 60, 80)),
        ]),
    )
}

fn dark_theme() -> Theme {
    let mut theme = system_palette_named("dark");
    // Force an explicit dark background so the app is readable even on
    // terminals with a white/light default background. The system theme keeps
    // Color::Reset (terminal default) and relies on the buffer adapter for
    // light backgrounds; the explicit "dark" theme must guarantee a dark bg.
    theme.colors.insert(ThemeColor::Background, rgb(18, 18, 26));
    // Force input text to white so it is readable on the dark background.
    // Color::Reset inherits the terminal default fg, which may be dark or
    // low-contrast on some terminals, making typed text invisible.
    theme
        .colors
        .insert(ThemeColor::InputText, rgb(240, 240, 245));
    // Force an explicit dark input background too. The system theme keeps
    // Color::Reset and relies on the buffer adapter, but the explicit "dark"
    // theme must guarantee contrast: a light terminal default background would
    // make white input text invisible without an explicit dark InputBg.
    theme.colors.insert(ThemeColor::InputBg, rgb(18, 18, 26));
    theme
}

pub fn user_color() -> Color {
    themed_color(ThemeColor::User)
}
pub fn background_color() -> Color {
    themed_color(ThemeColor::Background)
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
pub fn input_text() -> Color {
    themed_color(ThemeColor::InputText)
}
pub fn input_bg() -> Color {
    themed_color(ThemeColor::InputBg)
}
pub fn ai_text() -> Color {
    themed_color(ThemeColor::AiText)
}
pub fn bold_color() -> Color {
    themed_color(ThemeColor::Bold)
}
pub fn markdown_text_color() -> Color {
    themed_color(ThemeColor::MarkdownText)
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

/// Semantic accents. These resolve through the theme system so custom TOML
/// themes can override them, falling back to the historical palette defaults
/// when unset. This is a deliberate design choice: the palette buffer-adapt pass
/// still re-expresses configured palette overrides, so these accessors return
/// the role default (not the configured palette color) to avoid double-substitution.
pub fn success_color() -> Color {
    themed_color(ThemeColor::Success)
}
pub fn warning_color() -> Color {
    themed_color(ThemeColor::Warning)
}
pub fn error_color() -> Color {
    themed_color(ThemeColor::Error)
}
pub fn info_color() -> Color {
    themed_color(ThemeColor::Info)
}
pub fn border_color() -> Color {
    themed_color(ThemeColor::Border)
}
pub fn selection_bg_color() -> Color {
    themed_color(ThemeColor::SelectionBg)
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

/// Whether `symbol` is one of the cells owned by the primary activity spinner.
///
/// The TUI's single-cell spinner redraw uses this to avoid patching a status-row
/// cell after a late overlay, such as the slash-command palette, has taken
/// ownership of it.
pub fn is_activity_indicator_frame(symbol: &str) -> bool {
    SPINNER_FRAMES.contains(&symbol)
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
    use std::sync::Mutex;

    /// Serializes tests that mutate the process-global `ACTIVE_THEME`. Without
    /// this, parallel test threads race on the singleton and flake (e.g. one
    /// test asserts `light` while another has just re-set it to `dark`). The
    /// lock is held until `ActiveThemeGuard` has restored the default theme on
    /// drop, so no mutating test observes another's transient state.
    static THEME_TEST_MUTEX: Mutex<()> = Mutex::new(());

    #[test]
    fn loads_custom_theme_from_toml() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            temp.path().join("solar.toml"),
            "[colors]\nuser = \"#010203\"\nmarkdown_text = \"#0A0B0C\"\ninput_bg = \"reset\"\n",
        )
        .expect("write theme");

        let theme = load_theme("solar", Some(temp.path())).expect("load custom theme");
        assert_eq!(theme.name(), "solar");
        assert_eq!(theme.color(ThemeColor::User), Color::Rgb(1, 2, 3));
        assert_eq!(
            theme.color(ThemeColor::MarkdownText),
            Color::Rgb(10, 11, 12)
        );
        assert_eq!(theme.color(ThemeColor::InputBg), Color::Reset);
    }

    #[test]
    fn rejects_unsafe_custom_theme_names() {
        assert!(load_theme("../bad", Some(Path::new("/tmp"))).is_err());
    }

    #[test]
    fn parses_named_colors_in_custom_theme() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            temp.path().join("named.toml"),
            "[colors]\nerror = \"red\"\ninfo = \"light-blue\"\nsuccess = \"#64C864\"\n",
        )
        .expect("write theme");

        let theme = load_theme("named", Some(temp.path())).expect("load named theme");
        assert_eq!(theme.color(ThemeColor::Error), Color::Red);
        assert_eq!(theme.color(ThemeColor::Info), Color::LightBlue);
        assert_eq!(theme.color(ThemeColor::Success), Color::Rgb(100, 200, 100));
    }

    #[test]
    fn semantic_colors_route_through_theme_system() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            temp.path().join("accented.toml"),
            "[colors]\nerror = \"#0A0B0C\"\nborder = \"#010203\"\nselection_bg = \"#FF00FF\"\n",
        )
        .expect("write theme");

        let theme = load_theme("accented", Some(temp.path())).expect("load accented theme");
        assert_eq!(theme.color(ThemeColor::Error), Color::Rgb(10, 11, 12));
        assert_eq!(theme.color(ThemeColor::Border), Color::Rgb(1, 2, 3));
        assert_eq!(
            theme.color(ThemeColor::SelectionBg),
            Color::Rgb(255, 0, 255)
        );
    }

    #[test]
    fn dark_theme_sets_explicit_input_background() {
        let theme = dark_theme();
        // Dark theme must set an explicit dark InputBg so white InputText stays
        // readable even on a light terminal default background.
        assert_ne!(theme.color(ThemeColor::InputBg), Color::Reset);
        assert_ne!(theme.color(ThemeColor::InputText), Color::Reset);
    }

    #[test]
    fn lists_builtin_and_safe_custom_theme_names() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("ocean.toml"), "[colors]\n").expect("write theme");
        std::fs::write(temp.path().join("bad.name.toml"), "[colors]\n")
            .expect("write invalid theme name");
        std::fs::write(temp.path().join("notes.txt"), "ignored").expect("write txt");

        let names = available_theme_names(Some(temp.path()));
        assert!(names.contains(&"system".to_string()));
        assert!(names.contains(&"light".to_string()));
        assert!(names.contains(&"dark".to_string()));
        assert!(names.contains(&"ocean".to_string()));
    }

    #[test]
    fn spinner_frames_are_circular_braille_sequence() {
        assert_eq!(
            SPINNER_FRAMES,
            &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]
        );
        assert!(is_activity_indicator_frame("⠋"));
        assert!(is_activity_indicator_frame("⠏"));
        assert!(!is_activity_indicator_frame("/"));
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
        const {
            assert!(
                LIVENESS_SPINNER_FPS >= 8.0,
                "liveness spinner should animate at a smooth, responsive rate"
            );
        }
    }

    // ---- parse_color ----

    #[test]
    fn parse_color_parses_six_digit_hex() {
        assert_eq!(parse_color("#ff0000"), Some(Color::Rgb(255, 0, 0)));
        assert_eq!(parse_color("#0a0b0c"), Some(Color::Rgb(10, 11, 12)));
        // Uppercase hex digits are accepted.
        assert_eq!(parse_color("#FF00FF"), Some(Color::Rgb(255, 0, 255)));
        // Surrounding whitespace is trimmed.
        assert_eq!(parse_color("  #00ff00  "), Some(Color::Rgb(0, 255, 0)));
    }

    #[test]
    fn parse_color_requires_hash_prefix() {
        // A bare hex string is not recognized: parse_color tries named colors
        // first, then requires an explicit `#` before the hex digits.
        assert_eq!(parse_color("ff0000"), None);
        assert_eq!(parse_color("000000"), None);
    }

    #[test]
    fn parse_color_rejects_malformed_hex() {
        // Short form (e.g. #fff) is not supported, only #rrggbb.
        assert_eq!(parse_color("#fff"), None);
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("#1234567"), None);
        // Non-hex digits fail parsing.
        assert_eq!(parse_color("#gg0000"), None);
        assert_eq!(parse_color("#00zz00"), None);
        // There is no rgb(...) parser.
        assert_eq!(parse_color("rgb(255,0,0)"), None);
        // Empty / garbage.
        assert_eq!(parse_color(""), None);
        assert_eq!(parse_color("xyz"), None);
        assert_eq!(parse_color("#"), None);
    }

    #[test]
    fn parse_color_recognizes_reset_and_default() {
        assert_eq!(parse_color("reset"), Some(Color::Reset));
        assert_eq!(parse_color("RESET"), Some(Color::Reset));
        assert_eq!(parse_color("default"), Some(Color::Reset));
        assert_eq!(parse_color("Default"), Some(Color::Reset));
    }

    #[test]
    fn parse_color_resolves_named_colors_with_normalization() {
        // Exact lowercase names.
        assert_eq!(parse_color("red"), Some(Color::Red));
        assert_eq!(parse_color("blue"), Some(Color::Blue));
        assert_eq!(parse_color("green"), Some(Color::Green));
        assert_eq!(parse_color("white"), Some(Color::White));
        assert_eq!(parse_color("black"), Some(Color::Black));
        // Case is normalized before lookup.
        assert_eq!(parse_color("Red"), Some(Color::Red));
        assert_eq!(parse_color("MAGENTA"), Some(Color::Magenta));
        // Dashes and spaces are stripped so "light-blue" maps to LightBlue.
        assert_eq!(parse_color("light-blue"), Some(Color::LightBlue));
        assert_eq!(parse_color("light blue"), Some(Color::LightBlue));
        assert_eq!(parse_color("dark-gray"), Some(Color::DarkGray));
        assert_eq!(parse_color("grey"), Some(Color::Gray));
    }

    // ---- named_color ----

    #[test]
    fn named_color_resolves_known_names() {
        assert_eq!(named_color("black"), Some(Color::Black));
        assert_eq!(named_color("red"), Some(Color::Red));
        assert_eq!(named_color("green"), Some(Color::Green));
        assert_eq!(named_color("yellow"), Some(Color::Yellow));
        assert_eq!(named_color("blue"), Some(Color::Blue));
        assert_eq!(named_color("magenta"), Some(Color::Magenta));
        assert_eq!(named_color("cyan"), Some(Color::Cyan));
        assert_eq!(named_color("gray"), Some(Color::Gray));
        assert_eq!(named_color("grey"), Some(Color::Gray));
        assert_eq!(named_color("darkgray"), Some(Color::DarkGray));
        assert_eq!(named_color("darkgrey"), Some(Color::DarkGray));
        assert_eq!(named_color("lightred"), Some(Color::LightRed));
        assert_eq!(named_color("lightgreen"), Some(Color::LightGreen));
        assert_eq!(named_color("lightyellow"), Some(Color::LightYellow));
        assert_eq!(named_color("lightblue"), Some(Color::LightBlue));
        assert_eq!(named_color("lightmagenta"), Some(Color::LightMagenta));
        assert_eq!(named_color("lightcyan"), Some(Color::LightCyan));
        assert_eq!(named_color("white"), Some(Color::White));
    }

    #[test]
    fn named_color_rejects_unknown_and_is_case_sensitive() {
        // named_color matches the exact (lowercase) string; it does not
        // lowercase its input. parse_color does that normalization first.
        assert_eq!(named_color("notacolor"), None);
        assert_eq!(named_color(""), None);
        assert_eq!(named_color("Red"), None);
        assert_eq!(named_color("RED"), None);
        assert_eq!(named_color("LightBlue"), None);
        // Dashes/spaces are not stripped here (parse_color does that).
        assert_eq!(named_color("light-blue"), None);
        assert_eq!(named_color("light blue"), None);
    }

    // ---- parse_theme_color ----

    #[test]
    fn parse_theme_color_maps_each_variant_canonical_name() {
        let cases: &[(&str, ThemeColor)] = &[
            ("background", ThemeColor::Background),
            ("user", ThemeColor::User),
            ("ai", ThemeColor::Ai),
            ("tool", ThemeColor::Tool),
            ("file_link", ThemeColor::FileLink),
            ("dim", ThemeColor::Dim),
            ("accent", ThemeColor::Accent),
            ("system_message", ThemeColor::SystemMessage),
            ("queued", ThemeColor::Queued),
            ("asap", ThemeColor::Asap),
            ("pending", ThemeColor::Pending),
            ("user_text", ThemeColor::UserText),
            ("user_bg", ThemeColor::UserBg),
            ("input_text", ThemeColor::InputText),
            ("input_bg", ThemeColor::InputBg),
            ("ai_text", ThemeColor::AiText),
            ("bold", ThemeColor::Bold),
            ("markdown_text", ThemeColor::MarkdownText),
            ("header_icon", ThemeColor::HeaderIcon),
            ("header_name", ThemeColor::HeaderName),
            ("header_session", ThemeColor::HeaderSession),
            ("success", ThemeColor::Success),
            ("warning", ThemeColor::Warning),
            ("error", ThemeColor::Error),
            ("info", ThemeColor::Info),
            ("border", ThemeColor::Border),
            ("selection_bg", ThemeColor::SelectionBg),
        ];
        for (raw, expected) in cases {
            assert_eq!(
                parse_theme_color(raw),
                Some(*expected),
                "canonical name {raw:?}"
            );
        }
    }

    #[test]
    fn parse_theme_color_accepts_aliases() {
        let cases: &[(&str, ThemeColor)] = &[
            ("background_color", ThemeColor::Background),
            ("app_bg", ThemeColor::Background),
            ("app_background", ThemeColor::Background),
            ("user_color", ThemeColor::User),
            ("ai_color", ThemeColor::Ai),
            ("tool_color", ThemeColor::Tool),
            ("file_link_color", ThemeColor::FileLink),
            ("dim_color", ThemeColor::Dim),
            ("accent_color", ThemeColor::Accent),
            ("system_message_color", ThemeColor::SystemMessage),
            ("queued_color", ThemeColor::Queued),
            ("asap_color", ThemeColor::Asap),
            ("pending_color", ThemeColor::Pending),
            ("bold_color", ThemeColor::Bold),
            ("md_text", ThemeColor::MarkdownText),
            ("header_icon_color", ThemeColor::HeaderIcon),
            ("header_name_color", ThemeColor::HeaderName),
            ("header_session_color", ThemeColor::HeaderSession),
            ("success_color", ThemeColor::Success),
            ("warning_color", ThemeColor::Warning),
            ("error_color", ThemeColor::Error),
            ("info_color", ThemeColor::Info),
            ("border_color", ThemeColor::Border),
            ("selection_bg_color", ThemeColor::SelectionBg),
        ];
        for (raw, expected) in cases {
            assert_eq!(parse_theme_color(raw), Some(*expected), "alias {raw:?}");
        }
    }

    #[test]
    fn parse_theme_color_normalizes_input() {
        // Trims surrounding whitespace.
        assert_eq!(
            parse_theme_color("  background  "),
            Some(ThemeColor::Background)
        );
        // Case-insensitive.
        assert_eq!(
            parse_theme_color("BACKGROUND"),
            Some(ThemeColor::Background)
        );
        assert_eq!(parse_theme_color("User_Text"), Some(ThemeColor::UserText));
        // Dashes are folded to underscores.
        assert_eq!(
            parse_theme_color("app-background"),
            Some(ThemeColor::Background)
        );
        assert_eq!(
            parse_theme_color("header-icon"),
            Some(ThemeColor::HeaderIcon)
        );
        assert_eq!(parse_theme_color("md-text"), Some(ThemeColor::MarkdownText));
        assert_eq!(
            parse_theme_color("selection-bg-color"),
            Some(ThemeColor::SelectionBg)
        );
        // Mixed case + dashes + spaces together.
        assert_eq!(
            parse_theme_color("  Error-Color  "),
            Some(ThemeColor::Error)
        );
    }

    #[test]
    fn parse_theme_color_rejects_unknown_keys() {
        assert_eq!(parse_theme_color("notacolor"), None);
        assert_eq!(parse_theme_color(""), None);
        assert_eq!(parse_theme_color("   "), None);
        // British spelling is not supported.
        assert_eq!(parse_theme_color("background_colour"), None);
        // Plain *_text / *_bg keys have no `_color` alias form.
        assert_eq!(parse_theme_color("user_text_color"), None);
        assert_eq!(parse_theme_color("input_bg_color"), None);
    }

    // ---- active_theme_uses_terminal_adaptation ----

    /// Restores the process-global active theme to the default `system` theme
    /// when dropped, so tests that mutate it do not leak state to other tests.
    /// Drop runs even when the test panics, so the global is always restored.
    struct ActiveThemeGuard;

    impl Drop for ActiveThemeGuard {
        fn drop(&mut self) {
            let _ = set_theme("system", None);
        }
    }

    #[test]
    fn active_theme_adaptation_follows_active_theme() {
        let _lock = THEME_TEST_MUTEX.lock().unwrap();
        let _guard = ActiveThemeGuard;

        // `system` (the default) delegates to the terminal light/dark adapter.
        set_theme("system", None).expect("set system theme");
        assert!(active_theme_uses_terminal_adaptation());

        // `light` also reuses the buffer adapter.
        set_theme("light", None).expect("set light theme");
        assert!(active_theme_uses_terminal_adaptation());

        // `dark` ships an explicit palette, so adaptation must be off.
        set_theme("dark", None).expect("set dark theme");
        assert!(!active_theme_uses_terminal_adaptation());

        // A custom TOML theme is an explicit palette too: no adaptation.
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            temp.path().join("ocean.toml"),
            "[colors]\nuser = \"#001122\"\n",
        )
        .expect("write theme");
        set_theme("ocean", Some(temp.path())).expect("load custom theme");
        assert_eq!(active_theme_name(), "ocean");
        assert!(!active_theme_uses_terminal_adaptation());
    }

    // ---- Theme::new ----

    #[test]
    fn theme_new_stores_name_and_color_map() {
        let mut colors = BTreeMap::new();
        colors.insert(ThemeColor::User, Color::Rgb(1, 2, 3));
        colors.insert(ThemeColor::Ai, Color::Rgb(4, 5, 6));
        let theme = Theme::new("test-palette", colors);
        assert_eq!(theme.name(), "test-palette");
        assert_eq!(theme.color(ThemeColor::User), Color::Rgb(1, 2, 3));
        assert_eq!(theme.color(ThemeColor::Ai), Color::Rgb(4, 5, 6));
    }

    #[test]
    fn theme_new_accepts_string_and_string_literal() {
        // `impl Into<String>` accepts both owned and borrowed strings.
        let theme_owned = Theme::new(String::from("owned"), BTreeMap::new());
        assert_eq!(theme_owned.name(), "owned");
        let theme_lit = Theme::new("literal", BTreeMap::new());
        assert_eq!(theme_lit.name(), "literal");
    }

    // ---- Theme::color ----

    #[test]
    fn theme_color_returns_value_for_existing_key() {
        let mut colors = BTreeMap::new();
        colors.insert(ThemeColor::Bold, Color::Rgb(10, 20, 30));
        let theme = Theme::new("t", colors);
        assert_eq!(theme.color(ThemeColor::Bold), Color::Rgb(10, 20, 30));
    }

    #[test]
    fn theme_color_returns_reset_for_missing_key() {
        // An empty palette yields Color::Reset for every key, matching the
        // documented fallback in Theme::color.
        let theme = Theme::new("empty", BTreeMap::new());
        assert_eq!(theme.color(ThemeColor::User), Color::Reset);
        assert_eq!(theme.color(ThemeColor::Background), Color::Reset);
        assert_eq!(theme.color(ThemeColor::Bold), Color::Reset);
    }

    // ---- active_theme_name ----

    #[test]
    fn active_theme_name_matches_last_set_theme() {
        let _lock = THEME_TEST_MUTEX.lock().unwrap();
        let _guard = ActiveThemeGuard;
        set_theme("dark", None).expect("set dark theme");
        assert_eq!(active_theme_name(), "dark");

        set_theme("light", None).expect("set light theme");
        assert_eq!(active_theme_name(), "light");
    }

    // ---- set_theme ----

    #[test]
    fn set_theme_switches_to_builtin_dark() {
        let _lock = THEME_TEST_MUTEX.lock().unwrap();
        let _guard = ActiveThemeGuard;
        set_theme("dark", None).expect("set dark theme");
        assert_eq!(active_theme_name(), "dark");
        // `dark` ships an explicit palette, so adaptation must be off.
        assert!(!active_theme_uses_terminal_adaptation());
    }

    #[test]
    fn set_theme_rejects_unknown_theme_without_themes_dir() {
        let _lock = THEME_TEST_MUTEX.lock().unwrap();
        let _guard = ActiveThemeGuard;
        // A safe-but-unknown name with no themes dir cannot be loaded: the
        // custom-theme loader requires a configured directory.
        let result = set_theme("nonexistent-theme-xyz", None);
        assert!(result.is_err(), "unknown theme should error without a dir");
        // The failed set must not mutate the active theme.
        assert_ne!(active_theme_name(), "nonexistent-theme-xyz");
    }

    // ---- system_palette_named ----

    #[test]
    fn system_palette_named_labels_each_builtin() {
        assert_eq!(system_palette_named("system").name(), "system");
        assert_eq!(system_palette_named("light").name(), "light");
        assert_eq!(system_palette_named("dark").name(), "dark");
    }

    #[test]
    fn system_palette_named_ships_default_palette() {
        // Every builtin palette keeps Color::Reset for the background (the
        // terminal adapter handles light/dark), plus a concrete Bold color.
        let theme = system_palette_named("system");
        assert_eq!(theme.color(ThemeColor::Background), Color::Reset);
        assert_eq!(theme.color(ThemeColor::InputText), Color::Reset);
        assert_eq!(theme.color(ThemeColor::InputBg), Color::Reset);
    }

    #[test]
    fn system_palette_named_unknown_name_still_builds_theme() {
        // system_palette_named is a low-level builder: it does not validate the
        // name against the builtin list, it just labels the palette. Unknown
        // names still produce a Theme with that name and the default palette.
        let theme = system_palette_named("ocean");
        assert_eq!(theme.name(), "ocean");
        assert_eq!(theme.color(ThemeColor::Background), Color::Reset);
    }

    // ---- background_color / input_text / input_bg / bold_color / markdown_text_color ----
    //
    // These read the active theme, so they need an ActiveThemeGuard and a
    // pinned truecolor capability so the builtin `rgb(...)` literals resolve to
    // exact Color::Rgb values regardless of the host terminal.

    #[test]
    fn background_color_returns_dark_theme_background() {
        let _lock = THEME_TEST_MUTEX.lock().unwrap();
        let _guard = ActiveThemeGuard;
        color::pin_truecolor_for_tests();
        set_theme("dark", None).expect("set dark theme");
        assert_eq!(background_color(), Color::Rgb(18, 18, 26));
    }

    #[test]
    fn input_text_returns_dark_theme_input_text() {
        let _lock = THEME_TEST_MUTEX.lock().unwrap();
        let _guard = ActiveThemeGuard;
        color::pin_truecolor_for_tests();
        set_theme("dark", None).expect("set dark theme");
        assert_eq!(input_text(), Color::Rgb(240, 240, 245));
    }

    #[test]
    fn input_bg_returns_dark_theme_input_bg() {
        let _lock = THEME_TEST_MUTEX.lock().unwrap();
        let _guard = ActiveThemeGuard;
        color::pin_truecolor_for_tests();
        set_theme("dark", None).expect("set dark theme");
        assert_eq!(input_bg(), Color::Rgb(18, 18, 26));
    }

    #[test]
    fn bold_color_returns_dark_theme_bold() {
        let _lock = THEME_TEST_MUTEX.lock().unwrap();
        let _guard = ActiveThemeGuard;
        color::pin_truecolor_for_tests();
        set_theme("dark", None).expect("set dark theme");
        // dark_theme inherits Bold from the system palette default.
        assert_eq!(bold_color(), Color::Rgb(240, 240, 235));
    }

    #[test]
    fn markdown_text_color_returns_dark_theme_markdown_text() {
        let _lock = THEME_TEST_MUTEX.lock().unwrap();
        let _guard = ActiveThemeGuard;
        color::pin_truecolor_for_tests();
        set_theme("dark", None).expect("set dark theme");
        // dark_theme inherits MarkdownText from the system palette default.
        assert_eq!(markdown_text_color(), Color::Rgb(200, 200, 195));
    }

    // --- Contract tests ---
    //
    // These pin the public theme API as a contract: set→read round-trip, the
    // global color accessors routing through the active theme, lossless custom
    // TOML loading, and full ThemeColor key coverage. They document the
    // behavior external callers depend on and guard against silent regressions.

    /// Contract: the public `set_theme` → `active_theme_name` round-trip must
    /// hold for every builtin theme. Installing a builtin theme by name and
    /// then reading the active theme name must echo back the exact name passed
    /// in.
    #[test]
    fn contract_set_theme_then_active_theme_name_round_trips() {
        let _lock = THEME_TEST_MUTEX.lock().unwrap();
        let _guard = ActiveThemeGuard;

        for &name in &["system", "light", "dark"] {
            set_theme(name, None).unwrap_or_else(|e| panic!("set_theme({name:?}) failed: {e}"));
            assert_eq!(
                active_theme_name(),
                name,
                "active_theme_name() must echo the name passed to set_theme"
            );
        }
    }

    /// Contract: the global color accessors (`background_color`, `input_text`,
    /// `input_bg`, `bold_color`, `markdown_text_color`) must route through the
    /// active theme, returning exactly what `Theme::color()` returns for the
    /// matching `ThemeColor` variant on the currently installed palette — not
    /// a stale default. Verified against both the `dark` and `light` builtin
    /// palettes, whose Background/InputText/InputBg values differ, so a stale
    /// accessor would be caught.
    #[test]
    fn contract_color_accessors_match_active_theme() {
        let _lock = THEME_TEST_MUTEX.lock().unwrap();
        let _guard = ActiveThemeGuard;
        color::pin_truecolor_for_tests();

        for &name in &["dark", "light"] {
            let theme = load_theme(name, None)
                .unwrap_or_else(|e| panic!("load_theme({name:?}) failed: {e}"));
            set_theme(name, None).unwrap_or_else(|e| panic!("set_theme({name:?}) failed: {e}"));

            assert_eq!(
                background_color(),
                theme.color(ThemeColor::Background),
                "{name}: background_color() must match active Theme::color(Background)"
            );
            assert_eq!(
                input_text(),
                theme.color(ThemeColor::InputText),
                "{name}: input_text() must match active Theme::color(InputText)"
            );
            assert_eq!(
                input_bg(),
                theme.color(ThemeColor::InputBg),
                "{name}: input_bg() must match active Theme::color(InputBg)"
            );
            assert_eq!(
                bold_color(),
                theme.color(ThemeColor::Bold),
                "{name}: bold_color() must match active Theme::color(Bold)"
            );
            assert_eq!(
                markdown_text_color(),
                theme.color(ThemeColor::MarkdownText),
                "{name}: markdown_text_color() must match active Theme::color(MarkdownText)"
            );
        }
    }

    /// Contract: loading a custom TOML theme must preserve the exact RGB hex
    /// values written in the file, end to end through parse → store → read. No
    /// quantization, no default bleeding in for the keys that were set.
    ///
    /// Uses `markdown_text` rather than a generic `text` key because the
    /// theme TOML schema has no `text` alias; only keys that
    /// `parse_theme_color` recognizes are accepted.
    #[test]
    fn contract_custom_theme_load_preserves_rgb_values() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            temp.path().join("ocean.toml"),
            "[colors]\n\
             background = \"#0a0b1a\"\n\
             markdown_text = \"#e0e0f0\"\n\
             input_text = \"#c0c0ff\"\n",
        )
        .expect("write ocean.toml");

        let theme = load_theme("ocean", Some(temp.path())).expect("load ocean theme");
        assert_eq!(theme.name(), "ocean");
        assert_eq!(
            theme.color(ThemeColor::Background),
            Color::Rgb(10, 11, 26),
            "background hex must round-trip losslessly"
        );
        assert_eq!(
            theme.color(ThemeColor::MarkdownText),
            Color::Rgb(224, 224, 240),
            "markdown_text hex must round-trip losslessly"
        );
        assert_eq!(
            theme.color(ThemeColor::InputText),
            Color::Rgb(192, 192, 255),
            "input_text hex must round-trip losslessly"
        );
    }

    /// Contract: every `ThemeColor` variant must be reachable through its
    /// canonical TOML key (no orphaned enum members that the loader cannot
    /// parse), and every variant must be resolvable through `Theme::color()`
    /// once it has been stored. This guards against adding an enum variant
    /// without a matching parser arm.
    #[test]
    fn contract_every_theme_color_variant_is_parseable_and_resolvable() {
        let variants: &[(&str, ThemeColor)] = &[
            ("background", ThemeColor::Background),
            ("user", ThemeColor::User),
            ("ai", ThemeColor::Ai),
            ("tool", ThemeColor::Tool),
            ("file_link", ThemeColor::FileLink),
            ("dim", ThemeColor::Dim),
            ("accent", ThemeColor::Accent),
            ("system_message", ThemeColor::SystemMessage),
            ("queued", ThemeColor::Queued),
            ("asap", ThemeColor::Asap),
            ("pending", ThemeColor::Pending),
            ("user_text", ThemeColor::UserText),
            ("user_bg", ThemeColor::UserBg),
            ("input_text", ThemeColor::InputText),
            ("input_bg", ThemeColor::InputBg),
            ("ai_text", ThemeColor::AiText),
            ("bold", ThemeColor::Bold),
            ("markdown_text", ThemeColor::MarkdownText),
            ("header_icon", ThemeColor::HeaderIcon),
            ("header_name", ThemeColor::HeaderName),
            ("header_session", ThemeColor::HeaderSession),
            ("success", ThemeColor::Success),
            ("warning", ThemeColor::Warning),
            ("error", ThemeColor::Error),
            ("info", ThemeColor::Info),
            ("border", ThemeColor::Border),
            ("selection_bg", ThemeColor::SelectionBg),
        ];

        const KNOWN: Color = Color::Rgb(1, 2, 3);
        for &(canonical, variant) in variants {
            assert_eq!(
                parse_theme_color(canonical),
                Some(variant),
                "canonical key {canonical:?} should parse to {variant:?}"
            );

            // A Theme that maps only this variant to a known color must hand
            // it back through Theme::color() losslessly.
            let mut colors = BTreeMap::new();
            colors.insert(variant, KNOWN);
            let theme = Theme::new("coverage", colors);
            assert_eq!(
                theme.color(variant),
                KNOWN,
                "Theme::color({variant:?}) should return the stored color"
            );
        }
    }
}
