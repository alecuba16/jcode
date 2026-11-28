//! Automatic memory optimization turns.
//!
//! When the session is idle (or only background tasks run) and the feature is
//! enabled, queue one synthetic user turn that runs the memory optimization
//! prompt through the normal session loop with real memory tool access.

use super::*;

/// Session-local bookkeeping so the poll can stay cheap between ticks.
#[derive(Debug, Default)]
pub(super) struct MemoryOptimizeRuntime {
    pub(super) last_poll: Option<Instant>,
}

/// Common guards + trigger decision for automatic passes. Returns true when a
/// pass should fire. `now` drives the idle and throttle clocks, `utc` the
/// persisted cooldown. `throttle` applies the once-per-minute poll limit.
///
/// The cooldown state is a plain file without locking: two concurrent sessions
/// can both pass the cooldown check inside one tick and queue one pass each.
/// `min_interval_secs` bounds the damage to a single duplicate per window, so
/// the race is accepted rather than paying for cross-process locking.
fn memory_optimize_should_fire(
    app: &App,
    cfg: &crate::config::MemoryOptimizeConfig,
    now: Instant,
    utc: chrono::DateTime<chrono::Utc>,
    throttle: bool,
) -> bool {
    if app.is_remote || app.is_replay {
        return false;
    }
    if app.is_processing() || app.pending_turn || app.pending_queued_dispatch {
        return false;
    }
    if !app.memory_enabled {
        return false;
    }
    // Do not race queued work: same guard the queued-command dispatcher uses.
    if !app.queued_messages.is_empty() || !app.queued_commands.is_empty() {
        return false;
    }
    if throttle
        && app
            .memory_optimize
            .last_poll
            .is_some_and(|last| now.duration_since(last) < Duration::from_secs(60))
    {
        return false;
    }
    let bg_running = app
        .background_task_rows
        .iter()
        .any(|task| task.status == crate::tui::BackgroundTaskRowStatus::Running);
    let last_activity = [
        app.last_user_interaction,
        app.last_stream_activity,
        Some(app.app_started),
    ]
    .into_iter()
    .flatten()
    .max();
    let idle_secs = last_activity.map_or(0, |last| now.duration_since(last).as_secs());
    let state = crate::memory_optimize::MemoryOptimizeState::load();
    crate::memory_optimize::should_auto_run(cfg, &state, idle_secs, bg_running, utc)
}

impl App {
    pub(super) fn poll_memory_optimize(&mut self) {
        let mut cfg = crate::config::config().memory_optimize.clone();
        cfg.enabled = super::commands::memory_optimize_effective_enabled(self);
        if !cfg.enabled {
            return;
        }
        // One check per minute is plenty; ticks run at ~4fps.
        let now = Instant::now();
        let utc = chrono::Utc::now();
        let fire = memory_optimize_should_fire(self, &cfg, now, utc, true);
        // Stamp the throttle either way so a busy or failing pass cannot
        // re-enter on every tick.
        self.memory_optimize.last_poll = Some(now);
        if !fire {
            return;
        }
        // Cooldown stamp first: a failed save still counts as attempted, so a
        // read-only home cannot loop-queue passes every tick.
        let _ = crate::memory_optimize::mark_started(utc).save();
        super::commands::queue_memory_optimize_turn(
            self,
            "Memory optimize: queued automatic pass.".to_string(),
            "Session is idle: running automatic memory optimization.".to_string(),
        );
    }

    /// Test hook: same logic as `poll_memory_optimize` with injected clocks.
    /// `throttle: false` skips the once-per-minute poll limit so tests can
    /// drive the trigger decision directly; pass `true` to exercise it.
    #[cfg(test)]
    pub(super) fn poll_memory_optimize_at(
        &mut self,
        now: Instant,
        utc: chrono::DateTime<chrono::Utc>,
        throttle: bool,
    ) -> bool {
        let mut cfg = crate::config::config().memory_optimize.clone();
        cfg.enabled = super::commands::memory_optimize_effective_enabled(self);
        if !cfg.enabled {
            return false;
        }
        if !memory_optimize_should_fire(self, &cfg, now, utc, throttle) {
            return false;
        }
        let _ = crate::memory_optimize::mark_started(utc).save();
        super::commands::queue_memory_optimize_turn(
            self,
            "Memory optimize: queued automatic pass.".to_string(),
            "Session is idle: running automatic memory optimization.".to_string(),
        );
        true
    }
}

#[cfg(test)]
mod tests {
    // Tests live in tests/memory_optimize.rs, included from tests.rs so they
    // can reuse the shared `create_test_app`/`with_temp_jcode_home` fixtures.
}
