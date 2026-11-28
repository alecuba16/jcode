//! Host-side helpers for the /memory_optimize feature.
//!
//! Keeps the cooldown state (last automatic run) and builds the prompt for
//! the queued optimization turn. The turn itself runs as a normal model turn
//! in the user's session using the existing memory tools.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::MemoryOptimizeConfig;

/// Cooldown state for automatic memory optimization turns.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryOptimizeState {
    /// Wall-clock time of the last queued automatic pass.
    pub last_run_at: Option<DateTime<Utc>>,
}

impl MemoryOptimizeState {
    pub fn load() -> Self {
        crate::storage::read_json(&state_path().unwrap_or_default())
            .unwrap_or_else(|_| Self::default())
    }

    pub fn save(&self) -> Result<()> {
        let path = state_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        crate::storage::write_json(&path, self)
    }
}

fn state_path() -> Result<std::path::PathBuf> {
    Ok(crate::storage::jcode_dir()?
        .join("memory")
        .join("optimize_state.json"))
}

/// Whether an automatic optimization pass may be queued now.
///
/// `idle_secs` is measured from the last user interaction or model streaming
/// (monotonic, session-local). The cooldown uses wall-clock time and ignores
/// suspend, so a slept machine can still fire once on wake.
pub fn should_auto_run(
    cfg: &MemoryOptimizeConfig,
    state: &MemoryOptimizeState,
    idle_secs: u64,
    bg_running: bool,
    now: DateTime<Utc>,
) -> bool {
    if !cfg.enabled {
        return false;
    }
    if state.last_run_at.is_some_and(|last| {
        now.signed_duration_since(last).num_seconds() < cfg.min_interval_secs as i64
    }) {
        return false;
    }
    let idle_ok = cfg.idle_secs > 0 && idle_secs >= cfg.idle_secs;
    let bg_ok = bg_running && cfg.trigger_on_bg_wait;
    idle_ok || bg_ok
}

/// Record that a pass was queued, persisting the new cooldown stamp.
pub fn mark_started(now: DateTime<Utc>) -> MemoryOptimizeState {
    MemoryOptimizeState {
        last_run_at: Some(now),
    }
}

/// The prompt for the optimization turn. Runs with real tool access, so the
/// forget rules stay conservative: preferences and corrections are never
/// dropped, and superseded or duplicated factual entries go first.
pub fn optimize_prompt() -> String {
    "Optimize the long-term memory store now. Use the memory tool.\n\
     \n\
     1. memory list, scope global. Read every entry before changing anything.\n\
     2. Compress verbose entries: remember a terse replacement first (same category and tags, every id, name, path, number and command copied verbatim), then forget the original.\n\
     3. Merge near-duplicate or overlapping entries into one: remember the merged entry, then forget the originals.\n\
     4. Forget entries that are stale, superseded, or about things that no longer exist. Never forget preferences, corrections, or feedback about the user.\n\
     5. memory list again, scope global, and verify: no facts lost, no duplicates left, no invented content.\n\
     6. End with one line: entries before, after, merged, forgotten.\n\
     \n\
     No other work in this turn. Do not touch files or run commands other than the memory tool."
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn cfg(enabled: bool, idle_secs: u64, bg: bool, cooldown: u64) -> MemoryOptimizeConfig {
        MemoryOptimizeConfig {
            enabled,
            idle_secs,
            trigger_on_bg_wait: bg,
            min_interval_secs: cooldown,
        }
    }

    fn at(hour: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 28, 12, 0, 0)
            .unwrap()
            .checked_add_signed(chrono::Duration::hours(hour))
            .unwrap()
    }

    #[test]
    fn disabled_never_fires() {
        let state = MemoryOptimizeState::default();
        assert!(!should_auto_run(
            &cfg(false, 0, true, 0),
            &state,
            0,
            true,
            at(0)
        ));
    }

    #[test]
    fn idle_window_fires() {
        let state = MemoryOptimizeState::default();
        assert!(should_auto_run(
            &cfg(true, 600, false, 0),
            &state,
            600,
            false,
            at(0)
        ));
    }

    #[test]
    fn zero_idle_secs_disables_idle_path() {
        let state = MemoryOptimizeState::default();
        assert!(!should_auto_run(
            &cfg(true, 0, false, 0),
            &state,
            9999,
            false,
            at(0)
        ));
    }

    #[test]
    fn bg_wait_fires_when_enabled() {
        let state = MemoryOptimizeState::default();
        assert!(should_auto_run(
            &cfg(true, 600, true, 0),
            &state,
            0,
            true,
            at(0)
        ));
    }

    #[test]
    fn bg_wait_ignored_when_disabled() {
        let state = MemoryOptimizeState::default();
        assert!(!should_auto_run(
            &cfg(true, 600, false, 0),
            &state,
            0,
            true,
            at(0)
        ));
    }

    #[test]
    fn cooldown_blocks_repeat() {
        let state = MemoryOptimizeState {
            last_run_at: Some(at(0)),
        };
        assert!(!should_auto_run(
            &cfg(true, 0, true, 7200),
            &state,
            0,
            true,
            at(1)
        ));
        assert!(should_auto_run(
            &cfg(true, 0, true, 7200),
            &state,
            0,
            true,
            at(2)
        ));
    }

    #[test]
    fn mark_started_stamps_cooldown() {
        let state = mark_started(at(0));
        assert_eq!(state.last_run_at, Some(at(0)));
    }

    #[test]
    fn prompt_keeps_guardrails() {
        let prompt = optimize_prompt();
        assert!(prompt.contains("scope global"));
        assert!(prompt.contains("Never forget preferences"));
        assert!(prompt.contains("verbatim"));
        assert!(prompt.contains("no invented content"));
    }
}
