//! Automatic memory optimization turns.
//!
//! When the session is idle (or only background tasks run) and the feature is
//! enabled, queue one synthetic user turn that runs the memory optimization
//! prompt through the normal session loop with real memory tool access.

use super::*;

/// Session-local bookkeeping so the poll can stay cheap between ticks.
#[derive(Debug, Default)]
pub(super) struct MemoryOptimizeRuntime {
    last_poll: Option<Instant>,
}

impl App {
    pub(super) fn poll_memory_optimize(&mut self) {
        let mut cfg = crate::config::config().memory_optimize.clone();
        cfg.enabled = super::commands::memory_optimize_effective_enabled(self);
        if !cfg.enabled {
            return;
        }
        if self.is_remote || self.is_replay {
            return;
        }
        if self.is_processing() || self.pending_turn || self.pending_queued_dispatch {
            return;
        }
        if !self.memory_enabled {
            return;
        }
        // Do not race queued work: same guard the queued-command dispatcher uses.
        if !self.queued_messages.is_empty() || !self.queued_commands.is_empty() {
            return;
        }
        // One check per minute is plenty; ticks run at ~4fps.
        let now = Instant::now();
        if self
            .memory_optimize
            .last_poll
            .is_some_and(|last| now.duration_since(last) < Duration::from_secs(60))
        {
            return;
        }
        self.memory_optimize.last_poll = Some(now);

        let bg_running = self
            .background_task_rows
            .iter()
            .any(|task| task.status == crate::tui::BackgroundTaskRowStatus::Running);
        // A running background task is itself an optimization window; only
        // count it as activity when the bg-wait trigger is off.
        let last_activity = [
            self.last_user_interaction,
            self.last_stream_activity,
            Some(self.app_started),
        ]
        .into_iter()
        .flatten()
        .max();
        let idle_secs = last_activity.map_or(0, |last| now.duration_since(last).as_secs());
        let effective_idle_secs = if bg_running && !cfg.trigger_on_bg_wait {
            // Background work should not fire the idle path while disabled.
            0
        } else {
            idle_secs
        };

        let state = crate::memory_optimize::MemoryOptimizeState::load();
        if !crate::memory_optimize::should_auto_run(
            &cfg,
            &state,
            effective_idle_secs,
            bg_running,
            chrono::Utc::now(),
        ) {
            return;
        }
        if crate::memory_optimize::mark_started(chrono::Utc::now())
            .save()
            .is_err()
        {
            return;
        }
        super::commands::queue_memory_optimize_turn(
            self,
            "Memory optimize: queued automatic pass.".to_string(),
            "Session is idle: running automatic memory optimization.".to_string(),
        );
    }

    /// Test hook: same logic as `poll_memory_optimize` with injected clocks.
    #[cfg(test)]
    pub(super) fn poll_memory_optimize_at(
        &mut self,
        now: Instant,
        utc: chrono::DateTime<chrono::Utc>,
    ) -> bool {
        let mut cfg = crate::config::config().memory_optimize.clone();
        cfg.enabled = super::commands::memory_optimize_effective_enabled(self);
        if !cfg.enabled {
            return false;
        }
        if self.is_remote || self.is_replay {
            return false;
        }
        if self.is_processing() || self.pending_turn || self.pending_queued_dispatch {
            return false;
        }
        if !self.memory_enabled {
            return false;
        }
        if !self.queued_messages.is_empty() || !self.queued_commands.is_empty() {
            return false;
        }
        let bg_running = self
            .background_task_rows
            .iter()
            .any(|task| task.status == crate::tui::BackgroundTaskRowStatus::Running);
        let last_activity = [
            self.last_user_interaction,
            self.last_stream_activity,
            Some(self.app_started),
        ]
        .into_iter()
        .flatten()
        .max();
        let idle_secs = last_activity.map_or(0, |last| now.duration_since(last).as_secs());
        let effective_idle_secs = if bg_running && !cfg.trigger_on_bg_wait {
            0
        } else {
            idle_secs
        };
        let state = crate::memory_optimize::MemoryOptimizeState::load();
        let fire = crate::memory_optimize::should_auto_run(
            &cfg,
            &state,
            effective_idle_secs,
            bg_running,
            utc,
        );
        if fire {
            // Cooldown stamp first: a failed save still counts as attempted, so
            // a read-only home cannot loop-queue passes every tick.
            let _ = crate::memory_optimize::mark_started(utc).save();
            super::commands::queue_memory_optimize_turn(
                self,
                "Memory optimize: queued automatic pass.".to_string(),
                "Session is idle: running automatic memory optimization.".to_string(),
            );
        }
        fire
    }
}

#[cfg(test)]
mod tests {
    // Tests live in tests/memory_optimize.rs, included from tests.rs so they
    // can reuse the shared `create_test_app`/`with_temp_jcode_home` fixtures.
}
