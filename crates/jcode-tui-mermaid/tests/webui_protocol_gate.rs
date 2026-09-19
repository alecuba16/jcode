//! Process-level regression test for the HERDR_WEBUI mermaid/math protocol
//! gate (branch `herdr_webui_halfblocks`).
//!
//! The global PICKER is a process-global OnceLock, so env-based protocol
//! inference can only be exercised faithfully by initializing it in a fresh
//! process carrying the exact environment herdr-webui's builtin backend
//! exports into panes: HERDR_WEBUI=1, TERM_PROGRAM=ghostty, the pane's TERM,
//! and KITTY_WINDOW_ID scrubbed (webui src/builtin_backend.rs). The parent
//! spawns itself as a child with that environment; the child calls
//! init_picker() and asserts the picker stays on Halfblocks (halfblock
//! text art the browser can actually draw) instead of Kitty, whose
//! unicode-placeholder placements leak through as U+10EEEE garbage.
//!
//! A control child with the identical environment minus HERDR_WEBUI proves
//! the same TERM_PROGRAM=ghostty hint selects Kitty without the gate, so the
//! assertion exercises the gate itself rather than an accident of the
//! environment. Both children pin JCODE_MERMAID_PICKER_PROBE off and scrub
//! KITTY_WINDOW_ID/LC_TERMINAL/HERDR_WEBUI so the outcome is deterministic
//! no matter where the suite runs (including inside a webui pane).

use std::process::{Command, Output};

const CHILD_MODE_VAR: &str = "JCODE_TEST_WEBUI_GATE_CHILD";
const TEST_NAME: &str = "herdr_webui_gate_picks_halfblocks_and_control_picks_kitty";

fn spawn_gate_child(mode: &'static str) -> Output {
    let exe = std::env::current_exe().expect("test binary path");
    let mut command = Command::new(exe);
    command
        .arg("--exact")
        .arg(TEST_NAME)
        .arg("--nocapture")
        .env(CHILD_MODE_VAR, mode)
        // Pin the exact pane environment the builtin webui backend exports.
        .env("TERM", "xterm-256color")
        .env("TERM_PROGRAM", "ghostty")
        .env_remove("KITTY_WINDOW_ID")
        .env_remove("LC_TERMINAL")
        // The probe is opt-in and queries stdio, which is piped here; pin the
        // default Fast init mode so the child is deterministic.
        .env_remove("JCODE_MERMAID_PICKER_PROBE");
    match mode {
        "webui" => {
            command.env("HERDR_WEBUI", "1");
        }
        "control" => {
            command.env_remove("HERDR_WEBUI");
        }
        other => panic!("unknown gate child mode: {other}"),
    }
    command.output().expect("spawn HERDR_WEBUI gate child")
}

fn require_gate_child(mode: &'static str, expected: &str) {
    let output = spawn_gate_child(mode);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "{mode} child failed with {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status.code()
    );
    let marker = format!("GATE_RESULT={expected}");
    assert!(
        stdout.contains(&marker),
        "{mode} child did not report {marker}\nstdout:\n{stdout}"
    );
}

#[test]
fn herdr_webui_gate_picks_halfblocks_and_control_picks_kitty() {
    // Child mode: initialize the picker under the inherited environment and
    // report the resulting protocol; the parent asserts on the marker.
    if let Ok(mode) = std::env::var(CHILD_MODE_VAR) {
        let expected = match mode.as_str() {
            "webui" => "Some(Halfblocks)",
            "control" => "Some(Kitty)",
            other => panic!("unknown gate child mode: {other}"),
        };
        jcode_tui_mermaid::init_picker();
        let actual = format!("{:?}", jcode_tui_mermaid::protocol_type());
        println!("GATE_RESULT={actual}");
        assert_eq!(actual, expected, "{mode} child protocol mismatch");
        return;
    }

    require_gate_child("webui", "Some(Halfblocks)");
    require_gate_child("control", "Some(Kitty)");
}
