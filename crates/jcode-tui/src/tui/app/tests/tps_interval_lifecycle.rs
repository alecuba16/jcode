// Regression tests for the TPS display interval lifecycle (total mode) and
// rollback semantics. These drive the real remote event acceptance path
// (`App::handle_server_event` -> `remote::handle_server_event`) so the full
// turn/stream state machine runs, exactly as it does for a live session.
//
// Bug A: in "total" interval mode the wall-clock denominator must anchor at
// the first API-call begin (KvCacheRequest), not at the first TokenUsage
// snapshot (which only arrives at/after call completion), so single-call
// turns get a meaningful t/s instead of a degenerate near-zero denominator.
//
// Bug B: a `RetryRollback` discards the aborted attempt entirely and the
// retry is a fresh sample, so all TPS accumulators, timers, and the held
// `last_displayed_tps` must be cleared together with the stream buffers.

/// Scope `JCODE_HOME` to a fresh temp home whose config.toml pins
/// `display.tps_interval = "total"`, run `f`, then restore the previous env
/// and invalidate the config cache so nothing leaks between tests.
fn with_total_tps_home<T>(f: impl FnOnce() -> T) -> T {
    let _guard = crate::storage::lock_test_env();
    let temp_home = tempfile::TempDir::new().expect("temp home");
    let prev_home = std::env::var_os("JCODE_HOME");
    crate::env::set_var("JCODE_HOME", temp_home.path());
    std::fs::write(
        temp_home.path().join("config.toml"),
        "[display]\ntps_interval = \"total\"\n",
    )
    .expect("write config");
    // The config cache is keyed by content, not by home; drop any cached
    // config from the ambient/previous home before reading this one.
    crate::config::invalidate_config_cache();

    let result = f();

    if let Some(prev_home) = prev_home {
        crate::env::set_var("JCODE_HOME", prev_home);
    } else {
        crate::env::remove_var("JCODE_HOME");
    }
    crate::config::invalidate_config_cache();
    result
}

#[test]
fn test_total_tps_clock_anchors_at_first_kv_cache_request_not_first_token_usage() {
    with_total_tps_home(|| {
        let mut app = create_test_app();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _enter = rt.enter();
        let mut remote = crate::tui::backend::RemoteConnection::dummy();

        // Sanity: the temp config forces total mode.
        assert!(crate::config::config().display.tps_interval
            == crate::config::TpsIntervalMode::Total);

        app.is_processing = true;
        app.status = ProcessingStatus::Sending;
        app.current_message_id = Some(1);

        // The very first API call of the turn begins: the server announces
        // the kv cache request before any tokens exist.
        app.handle_server_event(
            crate::protocol::ServerEvent::KvCacheRequest {
                system_static_hash: 1,
                tools_hash: 2,
                messages_hash: 3,
                message_hashes: vec![11],
                message_count: 1,
                tool_count: 0,
                system_static_chars: 10,
                tools_json_chars: 0,
                messages_json_chars: 30,
                ephemeral_hash: None,
                ephemeral_chars: 0,
                ephemeral_message_count: 0,
            },
            &mut remote,
        );

        // Bug A: the total wall-clock clock must already be running here,
        // before any TokenUsage snapshot arrives.
        assert!(
            app.streaming.streaming_total_tps_start.is_some(),
            "total-mode TPS clock must anchor at the first API-call begin (KvCacheRequest), before any TokenUsage"
        );
    });
}

#[test]
fn test_total_tps_clock_persists_across_kv_cache_calls_within_a_turn() {
    with_total_tps_home(|| {
        let mut app = create_test_app();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _enter = rt.enter();
        let mut remote = crate::tui::backend::RemoteConnection::dummy();

        app.is_processing = true;
        app.status = ProcessingStatus::Sending;
        app.current_message_id = Some(1);

        let kv_request = || crate::protocol::ServerEvent::KvCacheRequest {
            system_static_hash: 1,
            tools_hash: 2,
            messages_hash: 3,
            message_hashes: vec![11],
            message_count: 1,
            tool_count: 0,
            system_static_chars: 10,
            tools_json_chars: 0,
            messages_json_chars: 30,
            ephemeral_hash: None,
            ephemeral_chars: 0,
            ephemeral_message_count: 0,
        };

        app.handle_server_event(kv_request(), &mut remote);
        let first_start = app
            .streaming
            .streaming_total_tps_start
            .expect("first call anchors the clock");
        assert!(
            first_start.elapsed() < Duration::from_millis(500),
            "clock must be live, not backdated"
        );

        // Second API call within the same turn must not re-anchor the clock:
        // the total interval spans the whole turn.
        app.handle_server_event(kv_request(), &mut remote);
        assert_eq!(
            app.streaming.streaming_total_tps_start,
            Some(first_start),
            "total clock must persist across calls within one turn"
        );
    });
}

#[test]
fn test_retry_rollback_clears_total_tps_state_and_held_tps() {
    with_total_tps_home(|| {
        let mut app = create_test_app();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _enter = rt.enter();
        let mut remote = crate::tui::backend::RemoteConnection::dummy();

        app.is_processing = true;
        app.status = ProcessingStatus::Streaming;
        app.current_message_id = Some(1);

        // A partial attempt: tokens were already streamed and accounted.
        app.streaming.streaming_tps_collect_output = true;
        app.handle_server_event(
            crate::protocol::ServerEvent::TokenUsage {
                input: 100,
                output: 40,
                cache_read_input: None,
                cache_creation_input: None,
            },
            &mut remote,
        );
        assert!(app.streaming.streaming_total_output_tokens > 0);
        assert!(app.streaming.streaming_total_tps_start.is_some());

        // The transport faults mid-stream and the server replays the request:
        // the aborted attempt's partial output and its TPS sample are stale.
        app.handle_server_event(
            crate::protocol::ServerEvent::RetryRollback { attempt: 1, max: 3 },
            &mut remote,
        );

        // Bug B: rollback must clear the whole TPS sample, not just buffers.
        assert_eq!(
            app.streaming.streaming_total_output_tokens, 0,
            "rollback must discard the aborted attempt's token count"
        );
        assert!(
            app.streaming.streaming_total_tps_start.is_none(),
            "rollback must clear the total-mode clock so the retry restarts the interval"
        );
        assert_eq!(
            app.streaming.last_displayed_tps, None,
            "rollback must drop the held t/s so the stale attempt's rate cannot display"
        );
    });
}

/// Improvement C: a config-backed switch to hide every tokens-per-second
/// display. The switch must gate ALL surfaces that render a t/s value:
/// the Overview cost-line suffix (avg), the UsageInfo `output_tps` (streaming
/// rate), the widget `tokens_per_second` (streaming rate), and the status
/// bar's live `output_tps()`.
///
/// Scope `JCODE_HOME` to a fresh temp home whose config.toml sets
/// `display.show_tps = <value>`, run `f`, then restore the previous env and
/// invalidate the config cache so nothing leaks between tests.
fn with_show_tps_home<T>(value: bool, f: impl FnOnce() -> T) -> T {
    let _guard = crate::storage::lock_test_env();
    let temp_home = tempfile::TempDir::new().expect("temp home");
    let prev_home = std::env::var_os("JCODE_HOME");
    crate::env::set_var("JCODE_HOME", temp_home.path());
    std::fs::write(
        temp_home.path().join("config.toml"),
        format!("[display]\nshow_tps = {}\n", value),
    )
    .expect("write config");
    // The config cache is keyed by content, not by home; drop any cached
    // config from the ambient/previous home before reading this one.
    crate::config::invalidate_config_cache();

    let result = f();

    if let Some(prev_home) = prev_home {
        crate::env::set_var("JCODE_HOME", prev_home);
    } else {
        crate::env::remove_var("JCODE_HOME");
    }
    crate::config::invalidate_config_cache();
    result
}

/// Drive one streaming tick through the real remote event path so the
/// app has a live streaming sample (tokens + elapsed + held t/s), then
/// return the widget data built by the real `info_widget_data()` builder.
fn streaming_widget_data(app: &mut App) -> crate::tui::info_widget::InfoWidgetData {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _enter = rt.enter();
    let mut remote = crate::tui::backend::RemoteConnection::dummy();

    app.is_processing = true;
    app.current_message_id = Some(1);
    app.status = ProcessingStatus::Sending;
    app.handle_server_event(
        crate::protocol::ServerEvent::KvCacheRequest {
            system_static_hash: 1,
            tools_hash: 2,
            messages_hash: 3,
            message_hashes: vec![11],
            message_count: 1,
            tool_count: 0,
            system_static_chars: 10,
            tools_json_chars: 0,
            messages_json_chars: 30,
            ephemeral_hash: None,
            ephemeral_chars: 0,
            ephemeral_message_count: 0,
        },
        &mut remote,
    );
    app.status = ProcessingStatus::Streaming;
    app.streaming.streaming_tps_collect_output = true;
    // A real streaming sample: 240 tokens over an exact 400ms generation
    // window (no live `Instant` so the numbers are deterministic: 240 / 0.4
    // = 600 t/s). `snapshot_streaming_tps` folds `streaming_tps_elapsed`
    // into the observed window and refreshes `last_displayed_tps`, so this
    // is the same path a live tick takes.
    app.streaming.streaming_total_output_tokens = 240;
    app.streaming.streaming_tps_observed_output_tokens = 240;
    app.streaming.streaming_tps_elapsed = std::time::Duration::from_millis(400);
    app.streaming.streaming_tps_start = None;
    // A completed past turn so the rolling average (avg_tps over
    // tps_history) is also populated and its hiding is observable.
    app.streaming.tps_history.push_back(42.0);
    app.snapshot_streaming_tps();

    use crate::tui::TuiState;
    app.info_widget_data()
}

#[test]
fn test_show_tps_false_hides_tps_from_all_widget_surfaces() {
    with_show_tps_home(false, || {
        let mut app = create_test_app();
        let data = streaming_widget_data(&mut app);

        // Sanity: with the switch off, the app still tracks the real sample
        // internally (the switch hides display, it must not corrupt state).
        assert!(app.streaming.last_displayed_tps.is_some());
        // And with the switch off the live t/s surfaces must all be None.
        assert_eq!(
            data.tokens_per_second, None,
            "streaming t/s must be hidden from the widget when show_tps is off"
        );
        assert_eq!(
            data.avg_tokens_per_second, None,
            "rolling-average t/s must be hidden from the widget when show_tps is off"
        );
        assert_eq!(
            data.usage_info.as_ref().and_then(|u| u.output_tps),
            None,
            "usage output_tps must be hidden when show_tps is off"
        );

        // Status bar surface (draw_overscroll_status / ui_input streaming
        // label) reads TuiState::output_tps() directly.
        use crate::tui::TuiState;
        assert_eq!(
            app.output_tps(),
            None,
            "status-bar t/s must be hidden when show_tps is off"
        );

        // And the renderer itself must not emit the suffix.
        let line = crate::tui::info_widget::render_cost_tokens_line_for_test(&data);
        let text = line.spans.iter().map(|s| s.content.clone()).collect::<String>();
        assert!(
            !text.contains("t/s"),
            "cost line must not render any t/s when show_tps is off, got: {text}"
        );
    });
}

#[test]
fn test_show_tps_true_or_absent_keeps_tps_displayed() {
    with_show_tps_home(true, || {
        let mut app = create_test_app();
        let data = streaming_widget_data(&mut app);
        assert!(
            data.tokens_per_second.is_some(),
            "streaming t/s must stay visible when show_tps is on"
        );
        use crate::tui::TuiState;
        let tps = app
            .output_tps()
            .expect("status-bar t/s stays visible when show_tps is on");
        assert!(
            (tps - 600.0).abs() < 1.0,
            "live sample must be ~600 t/s (240 tokens / 0.4s), got {tps}"
        );
    });
}
