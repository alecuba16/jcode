// Unit tests for `App::refresh_mcp_server_names`.
//
// NOTE: the task brief listed this function under `crates/jcode-base/src/mcp/
// manager.rs`, but `refresh_mcp_server_names` actually lives on `App` in
// `crates/jcode-tui/src/tui/app/tui_lifecycle_runtime.rs`. It reads the live
// MCP manager and caches the connected-server set with per-server tool counts.
// These tests live in `jcode-tui` because that is where the function is
// defined; `cargo test --package jcode-tui --lib -- refresh_mcp_server_names`
// covers them.

/// With no MCP servers connected (the empty state on a fresh app),
/// `refresh_mcp_server_names` must not panic and must leave the cache empty.
#[test]
fn refresh_mcp_server_names_empty_state_is_noop() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        assert!(
            app.mcp_server_names.is_empty(),
            "a fresh app should have no cached MCP server names"
        );

        // The method is async; drive it on a dedicated runtime like the other
        // jcode-tui tests that exercise async App methods.
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(app.refresh_mcp_server_names());

        assert!(
            app.mcp_server_names.is_empty(),
            "no connected servers -> mcp_server_names should stay empty"
        );
    });
}

// ---------------------------------------------------------------------------
// `/mcp` slash-command dispatch tests.
//
// These verify that the TUI command handler claims `/mcp`, produces sensible
// output for `/mcp status`, and rejects unknown servers with an error message.
// ---------------------------------------------------------------------------

/// Text of the last message the app pushed, whatever its role.
fn last_message(app: &crate::tui::app::App) -> String {
    app.display_messages
        .last()
        .map(|message| message.content.clone())
        .unwrap_or_default()
}

#[test]
fn mcp_status_with_no_servers_reports_none_configured() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        assert!(
            crate::tui::app::commands_dispatch::dispatch_local_command(&mut app, "/mcp"),
            "/mcp should be claimed by the dispatch table"
        );
        let output = last_message(&app);
        assert!(
            output.contains("none configured"),
            "with no servers configured, status should say so: {output}"
        );
    });
}

#[test]
fn mcp_status_alias_works() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        assert!(
            crate::tui::app::commands_dispatch::dispatch_local_command(&mut app, "/mcp status"),
            "/mcp status should be claimed by the dispatch table"
        );
        let output = last_message(&app);
        assert!(
            output.contains("none configured"),
            "/mcp status alias should produce the same output as /mcp: {output}"
        );
    });
}

#[test]
fn mcp_enable_unknown_server_reports_error() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        assert!(
            crate::tui::app::commands_dispatch::dispatch_local_command(
                &mut app,
                "/mcp enable nonexistent"
            ),
            "/mcp enable should be claimed by the dispatch table"
        );
        let output = last_message(&app);
        assert!(
            output.contains("not found") || output.contains("not_found"),
            "enabling a nonexistent server should report not found: {output}"
        );
    });
}

#[test]
fn mcp_disable_unknown_server_reports_error() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        assert!(
            crate::tui::app::commands_dispatch::dispatch_local_command(
                &mut app,
                "/mcp disable ghost"
            ),
            "/mcp disable should be claimed by the dispatch table"
        );
        let output = last_message(&app);
        assert!(
            output.contains("not found") || output.contains("not_found"),
            "disabling a nonexistent server should report not found: {output}"
        );
    });
}