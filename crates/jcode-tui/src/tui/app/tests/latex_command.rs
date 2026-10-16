// `/latex` status, mode switching, and persistence. The command must show the
// current mode, accept aliases (raw/off for none, text/terminal for unicode),
// reject junk with usage, and persist the choice to config.toml so the next
// session keeps it. JCODE_LATEX_RENDERING overrides the file value, so every
// test clears it (or pins it) to stay deterministic.

#[test]
fn latex_command_status_shows_current_mode_and_usage_hint() {
    let _guard = crate::storage::lock_test_env();
    let temp = tempfile::tempdir().expect("tempdir");
    let prev_home = std::env::var_os("JCODE_HOME");
    crate::env::set_var("JCODE_HOME", temp.path());
    crate::env::remove_var("JCODE_LATEX_RENDERING");
    crate::config::Config::invalidate_cache();

    let mut app = create_test_app();
    app.input = "/latex".to_string();
    app.submit_input();

    let msg = app
        .display_messages()
        .last()
        .expect("missing latex status message");
    assert_eq!(msg.role, "system");
    assert!(
        msg.content.contains("LaTeX rendering: image"),
        "default mode is image, got: {}",
        msg.content
    );
    assert!(
        msg.content.contains("/latex [none|unicode|image]"),
        "status output must show the usage form, got: {}",
        msg.content
    );

    if let Some(prev_home) = prev_home {
        crate::env::set_var("JCODE_HOME", prev_home);
    } else {
        crate::env::remove_var("JCODE_HOME");
    }
    crate::config::Config::invalidate_cache();
}

#[test]
fn latex_command_switches_mode_and_persists_it() {
    let _guard = crate::storage::lock_test_env();
    let temp = tempfile::tempdir().expect("tempdir");
    let prev_home = std::env::var_os("JCODE_HOME");
    let prev_latex_env = std::env::var_os("JCODE_LATEX_RENDERING");
    crate::env::set_var("JCODE_HOME", temp.path());
    crate::env::remove_var("JCODE_LATEX_RENDERING");
    crate::config::Config::invalidate_cache();

    let mut app = create_test_app();
    app.input = "/latex none".to_string();
    app.submit_input();

    assert_eq!(
        app.status_notice(),
        Some("LaTeX rendering: none".to_string())
    );
    assert_eq!(
        crate::config::config().display.latex_rendering,
        crate::config::LatexRenderingMode::None,
        "switching the mode must update the live config immediately"
    );

    let config_path = crate::config::Config::path().expect("config path");
    let raw = std::fs::read_to_string(&config_path).expect("read config back");
    assert!(
        raw.contains("latex_rendering = \"none\""),
        "mode must be persisted to config.toml, got: {}",
        raw
    );

    // Aliases must land on the same enum variants.
    app.input = "/latex unicode".to_string();
    app.submit_input();
    assert_eq!(
        crate::config::config().display.latex_rendering,
        crate::config::LatexRenderingMode::Unicode
    );
    app.input = "/latex off".to_string();
    app.submit_input();
    assert_eq!(
        crate::config::config().display.latex_rendering,
        crate::config::LatexRenderingMode::None,
        "\"off\" alias must select none"
    );

    if let Some(prev_home) = prev_home {
        crate::env::set_var("JCODE_HOME", prev_home);
    } else {
        crate::env::remove_var("JCODE_HOME");
    }
    if let Some(prev_latex) = prev_latex_env {
        crate::env::set_var("JCODE_LATEX_RENDERING", prev_latex);
    } else {
        crate::env::remove_var("JCODE_LATEX_RENDERING");
    }
    crate::config::Config::invalidate_cache();
}

#[test]
fn latex_command_rejects_unknown_mode_with_usage() {
    let _guard = crate::storage::lock_test_env();
    let temp = tempfile::tempdir().expect("tempdir");
    let prev_home = std::env::var_os("JCODE_HOME");
    let prev_latex_env = std::env::var_os("JCODE_LATEX_RENDERING");
    crate::env::set_var("JCODE_HOME", temp.path());
    crate::env::remove_var("JCODE_LATEX_RENDERING");
    crate::config::Config::invalidate_cache();

    let mut app = create_test_app();
    app.input = "/latex prettypng".to_string();
    app.submit_input();

    let msg = app
        .display_messages()
        .last()
        .expect("missing error message");
    assert_eq!(msg.role, "error");
    assert!(
        msg.content.contains("Usage: /latex [none|unicode|image]"),
        "bad mode must print usage, got: {}",
        msg.content
    );
    assert_eq!(
        crate::config::config().display.latex_rendering,
        crate::config::LatexRenderingMode::Image,
        "a rejected mode must not change the live config"
    );

    if let Some(prev_home) = prev_home {
        crate::env::set_var("JCODE_HOME", prev_home);
    } else {
        crate::env::remove_var("JCODE_HOME");
    }
    if let Some(prev_latex) = prev_latex_env {
        crate::env::set_var("JCODE_LATEX_RENDERING", prev_latex);
    } else {
        crate::env::remove_var("JCODE_LATEX_RENDERING");
    }
    crate::config::Config::invalidate_cache();
}
/// The env override wins over the persisted file value: the save still writes
/// the file, but the live config keeps serving the env mode.
#[test]
fn latex_command_save_is_shadowed_by_env_override() {
    let _guard = crate::storage::lock_test_env();
    let temp = tempfile::tempdir().expect("tempdir");
    let prev_home = std::env::var_os("JCODE_HOME");
    let prev_latex_env = std::env::var_os("JCODE_LATEX_RENDERING");
    crate::env::set_var("JCODE_HOME", temp.path());
    crate::env::set_var("JCODE_LATEX_RENDERING", "unicode");
    crate::config::Config::invalidate_cache();

    let mut app = create_test_app();
    app.input = "/latex none".to_string();
    app.submit_input();

    // The file must record what the user asked for, even though the env var
    // keeps shadowing the live config.
    let config_path = crate::config::Config::path().expect("config path");
    let raw = std::fs::read_to_string(&config_path).expect("read config back");
    assert!(
        raw.contains("latex_rendering = \"none\""),
        "save must write the requested mode to config.toml, got: {}",
        raw
    );
    // The live config still serves the env override, not the saved value.
    assert_eq!(
        crate::config::config().display.latex_rendering,
        crate::config::LatexRenderingMode::Unicode,
        "JCODE_LATEX_RENDERING must keep winning over the saved file value"
    );

    if let Some(prev_home) = prev_home {
        crate::env::set_var("JCODE_HOME", prev_home);
    } else {
        crate::env::remove_var("JCODE_HOME");
    }
    if let Some(prev_latex) = prev_latex_env {
        crate::env::set_var("JCODE_LATEX_RENDERING", prev_latex);
    } else {
        crate::env::remove_var("JCODE_LATEX_RENDERING");
    }
    crate::config::Config::invalidate_cache();
}
