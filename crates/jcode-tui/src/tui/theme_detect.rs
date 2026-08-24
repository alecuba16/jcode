//! Terminal light/dark theme detection.
//!
//! Resolves the theme mode once per process, before the TUI enters raw mode:
//!
//! 1. `JCODE_THEME=dark|light` env override (also accepts `auto`).
//! 2. `display.theme` config: "dark", "light", "system", custom name, or "auto"/empty.
//! 3. Auto: query the terminal's background color (OSC 11 via
//!    `terminal-colorsaurus`) and classify by perceived lightness. Terminals
//!    known not to support OSC queries are rejected before the bounded query so
//!    they do not add hundreds of milliseconds to startup.
//! 4. Fallback: dark (jcode's native palette).
//!
//! The result is stored in `jcode_tui_style::theme_mode` where the renderer
//! adapts colors for light backgrounds at frame time.

use jcode_tui_style::ThemeMode;
use std::sync::{Mutex, OnceLock};

static DETECTED: OnceLock<ThemeMode> = OnceLock::new();

/// In-flight prewarm of the (blocking) terminal background query. See
/// [`prewarm_theme_mode`].
static PREWARM: Mutex<Option<std::thread::JoinHandle<ThemeMode>>> = Mutex::new(None);

/// Start resolving the theme mode on a background thread so the OSC 11 query
/// overlaps other startup work instead of extending the critical path.
pub fn prewarm_theme_mode() {
    if DETECTED.get().is_some() {
        return;
    }
    let Ok(mut slot) = PREWARM.lock() else {
        return;
    };
    if slot.is_some() {
        return;
    }
    if let Ok(handle) = std::thread::Builder::new()
        .name("jcode-theme-detect".to_string())
        .spawn(resolve_theme_mode)
    {
        *slot = Some(handle);
    }
}

/// Join a prewarm started by [`prewarm_theme_mode`], if any.
fn take_prewarmed_theme_mode() -> Option<ThemeMode> {
    let handle = PREWARM.lock().ok()?.take()?;
    handle.join().ok()
}

/// Resolve and install the global theme mode. Idempotent; the first call does
/// the (potentially blocking, sub-second) terminal query and later calls are
/// free. Must be called before entering raw mode / the alternate screen.
pub fn init_theme_mode() -> ThemeMode {
    apply_configured_palette();
    let mode = match take_prewarmed_theme_mode() {
        Some(prewarmed) => *DETECTED.get_or_init(|| prewarmed),
        None => *DETECTED.get_or_init(resolve_theme_mode),
    };
    jcode_tui_style::set_theme_mode(mode);
    mode
}

/// Resolve the theme while resuming an already-active TUI after an `exec` handoff.
///
/// The inherited terminal is already in raw mode and may already have a crossterm
/// event reader attached. Sending a fresh OSC 11 query in that state can leave the
/// terminal's color response in stdin, where it is decoded as ordinary composer
/// input. Prefer the theme captured by the previous process and otherwise resolve
/// configuration without querying the terminal.
pub fn init_theme_mode_for_resume(inherited_theme: Option<&str>) -> ThemeMode {
    apply_configured_palette();
    let inherited_theme = inherited_theme.and_then(|value| match value {
        "dark" => Some(ThemeMode::Dark),
        "light" => Some(ThemeMode::Light),
        _ => None,
    });
    let mode = *DETECTED
        .get_or_init(|| inherited_theme.unwrap_or_else(resolve_theme_mode_without_terminal_query));
    jcode_tui_style::set_theme_mode(mode);
    mode
}

pub fn current_theme_label() -> &'static str {
    match jcode_tui_style::theme_mode() {
        ThemeMode::Dark => "dark",
        ThemeMode::Light => "light",
    }
}

fn resolve_theme_mode() -> ThemeMode {
    resolve_configured_theme(true)
}

fn resolve_theme_mode_without_terminal_query() -> ThemeMode {
    resolve_configured_theme(false)
}

fn configured_theme_name() -> String {
    std::env::var("JCODE_THEME")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| crate::config::config().display.theme.clone())
}

fn theme_dir() -> Option<std::path::PathBuf> {
    crate::storage::app_config_dir()
        .ok()
        .map(|dir| dir.join("themes"))
}

fn palette_name(configured: &str) -> &str {
    match configured.trim() {
        "" | "auto" => "system",
        other => other,
    }
}

pub(crate) fn apply_configured_palette() {
    reinstall_palette();
    let configured = configured_theme_name();
    let dir = theme_dir();
    let name = palette_name(&configured);
    if let Err(err) = jcode_tui_style::theme::set_theme(name, dir.as_deref()) {
        crate::logging::info(&format!(
            "Failed to load theme '{name}' ({err:#}); using system theme"
        ));
        let _ = jcode_tui_style::theme::set_theme("system", None);
    }
}

/// Install the user's configured color palette from `[display.colors]`.
///
/// Invalid entries are logged and skipped rather than failing the palette, so
/// one typo can never leave the TUI unstyled. Safe to call repeatedly; the TUI
/// calls it again after `/colors` edits so changes apply without a restart.
fn reinstall_palette() {
    let configured = &crate::config::config().display.colors;
    let (palette, errors) = jcode_tui_style::Palette::from_pairs(
        configured
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str())),
    );
    for error in errors {
        crate::logging::warn(&format!("display.colors: {error}"));
    }
    jcode_tui_style::set_palette(palette);
}

fn resolve_configured_theme(query_terminal: bool) -> ThemeMode {
    let configured = configured_theme_name();

    match configured.trim().to_ascii_lowercase().as_str() {
        "dark" => return ThemeMode::Dark,
        "light" => return ThemeMode::Light,
        "" | "auto" | "system" => {}
        other => {
            crate::logging::info(&format!(
                "Custom theme '{other}' selected; using terminal background detection only when palette allows it"
            ));
        }
    }

    if query_terminal {
        detect_terminal_theme().unwrap_or(ThemeMode::Dark)
    } else {
        crate::logging::info(
            "Skipping terminal background query during reload handoff; preserving a safe theme",
        );
        ThemeMode::Dark
    }
}

/// Query the terminal background color and classify it as dark or light.
/// Returns None when the terminal does not support querying or the query
/// fails, in which case the caller falls back to dark.
fn detect_terminal_theme() -> Option<ThemeMode> {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return None;
    }
    if !terminal_background_query_supported(
        std::env::var("TERM").ok().as_deref(),
        std::env::var("TERM_PROGRAM").ok().as_deref(),
        std::env::var("LC_TERMINAL").ok().as_deref(),
    ) {
        crate::logging::info(
            "Skipping terminal background query for a terminal without OSC query support",
        );
        return None;
    }
    let mut options = terminal_colorsaurus::QueryOptions::default();
    // Keep startup snappy; supporting terminals answer in a few ms, and
    // colorsaurus detects non-supporting terminals before the timeout anyway.
    // This bound lands on the launch critical path whenever a terminal claims
    // OSC support but never replies (multiplexers, remote shells), so keep it
    // far below the perceptible-stall threshold.
    options.timeout = std::time::Duration::from_millis(120);
    // Ask for only OSC 11. `theme_mode` queries both OSC 10 (foreground) and
    // OSC 11 (background); on some terminals the two replies can be split far
    // enough apart that the query consumes one and crossterm later decodes the
    // other as ordinary keys. That produces composer garbage such as
    // `10;rgb:cdcd/d6d6/f4f4\\11;rgb:0000/0000/0000\\` at startup. Background
    // lightness is all our renderer needs, and a single request has no second
    // reply that can escape into the event reader.
    match terminal_colorsaurus::background_color(options) {
        Ok(background) if background.perceived_lightness() > 0.5 => {
            crate::logging::info("Detected light terminal background; adapting theme");
            Some(ThemeMode::Light)
        }
        Ok(_) => Some(ThemeMode::Dark),
        Err(e) => {
            crate::logging::info(&format!(
                "Terminal background detection unavailable ({e}); defaulting to dark theme"
            ));
            None
        }
    }
}

/// Reject terminal classes that cannot answer OSC 11 before entering the
/// colorsaurus timeout path. A concrete terminal-program hint wins because
/// launchers and multiplexers occasionally leave a conservative `TERM` value
/// in place even though the outer emulator supports OSC queries.
fn terminal_background_query_supported(
    term: Option<&str>,
    term_program: Option<&str>,
    lc_terminal: Option<&str>,
) -> bool {
    if term_program.is_some_and(|value| !value.trim().is_empty())
        || lc_terminal.is_some_and(|value| !value.trim().is_empty())
    {
        return true;
    }

    let term = term.unwrap_or("").trim().to_ascii_lowercase();
    !matches!(term.as_str(), "" | "dumb" | "linux" | "cons25" | "emacs")
}

#[cfg(test)]
mod tests {
    use super::terminal_background_query_supported;

    #[test]
    fn skips_terminals_without_osc_query_support() {
        for term in [None, Some(""), Some("dumb"), Some("linux"), Some("cons25")] {
            assert!(!terminal_background_query_supported(term, None, None));
        }
    }

    #[test]
    fn queries_terminal_emulators_and_honors_program_hints() {
        assert!(terminal_background_query_supported(
            Some("xterm-256color"),
            None,
            None
        ));
        assert!(terminal_background_query_supported(
            Some("linux"),
            Some("kitty"),
            None
        ));
        assert!(terminal_background_query_supported(
            Some("linux"),
            None,
            Some("iTerm2")
        ));
    }
}
