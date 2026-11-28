#[test]
fn test_memory_optimize_idle_poll_fires_then_cooldown_blocks() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        app.memory_enabled = true;
        app.memory_optimize_enabled = Some(true);
        // Simulate a session idle past the default 1800s window.
        let now = std::time::Instant::now();
        app.app_started = now - std::time::Duration::from_secs(3600);
        let utc = chrono::Utc::now();
        assert!(app.poll_memory_optimize_at(now, utc, false));
        // Same wall-clock moment: the cooldown must block a second pass.
        assert!(!app.poll_memory_optimize_at(now, utc, false));
    });
}

#[test]
fn test_memory_optimize_session_override_off_blocks_poll() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        app.memory_enabled = true;
        app.memory_optimize_enabled = Some(false);
        assert!(!app.poll_memory_optimize_at(std::time::Instant::now(), chrono::Utc::now(), false));
    });
}

#[test]
fn test_memory_optimize_memory_feature_off_blocks_poll() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        app.memory_enabled = false;
        app.memory_optimize_enabled = Some(true);
        assert!(!app.poll_memory_optimize_at(std::time::Instant::now(), chrono::Utc::now(), false));
    });
}

#[test]
fn test_memory_optimize_queued_work_blocks_poll() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        app.memory_enabled = true;
        app.memory_optimize_enabled = Some(true);
        app.queued_messages.push("pending".to_string());
        assert!(!app.poll_memory_optimize_at(std::time::Instant::now(), chrono::Utc::now(), false));
    });
}

#[test]
fn test_memory_optimize_processing_blocks_poll() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        app.memory_enabled = true;
        app.memory_optimize_enabled = Some(true);
        app.is_processing = true;
        assert!(!app.poll_memory_optimize_at(std::time::Instant::now(), chrono::Utc::now(), false));
    });
}

#[test]
fn test_memory_optimize_pending_turn_blocks_poll() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        app.memory_enabled = true;
        app.memory_optimize_enabled = Some(true);
        app.pending_turn = true;
        assert!(!app.poll_memory_optimize_at(std::time::Instant::now(), chrono::Utc::now(), false));
    });
}

#[test]
fn test_memory_optimize_remote_blocks_poll() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        app.memory_enabled = true;
        app.memory_optimize_enabled = Some(true);
        app.is_remote = true;
        assert!(!app.poll_memory_optimize_at(std::time::Instant::now(), chrono::Utc::now(), false));
    });
}

#[test]
fn test_memory_optimize_replay_blocks_poll() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        app.memory_enabled = true;
        app.memory_optimize_enabled = Some(true);
        app.is_replay = true;
        assert!(!app.poll_memory_optimize_at(std::time::Instant::now(), chrono::Utc::now(), false));
    });
}

#[test]
fn test_memory_optimize_throttle_blocks_within_window() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        app.memory_enabled = true;
        app.memory_optimize_enabled = Some(true);
        let now = std::time::Instant::now();
        app.app_started = now - std::time::Duration::from_secs(3600);
        // Poll stamped 30s ago: inside the 60s throttle window, so blocked
        // even though idle and cooldown would allow a pass.
        app.memory_optimize.last_poll = Some(now - std::time::Duration::from_secs(30));
        assert!(!app.poll_memory_optimize_at(now, chrono::Utc::now(), true));
        // Stamped 61s ago: throttle expired, pass fires.
        app.memory_optimize.last_poll = Some(now - std::time::Duration::from_secs(61));
        assert!(app.poll_memory_optimize_at(now, chrono::Utc::now(), true));
    });
}

#[test]
fn test_memory_optimize_command_dispatch() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        app.memory_enabled = true;
        app.memory_optimize_enabled = None;
        assert!(super::commands::parse_memory_optimize_command("/memory_optimize run").is_some());
        assert!(super::commands::parse_memory_optimize_command("/memory_optimize bogus").is_some());
        assert_eq!(
            super::commands::parse_memory_optimize_command("/memory other"),
            None
        );
        // No session override: effective state follows config (default off).
        assert!(!super::commands::memory_optimize_effective_enabled(&app));
        app.memory_optimize_enabled = Some(true);
        assert!(super::commands::memory_optimize_effective_enabled(&app));
    });
}
