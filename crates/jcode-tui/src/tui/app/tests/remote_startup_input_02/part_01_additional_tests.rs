use super::*;

#[test]
fn test_model_picker_copilot_selection_prefixes_model() {
    // Same ambient-profile hazard as
    // test_model_picker_copilot_models_have_copilot_route: the firmware
    // static catalog claims grok-code-fast-1 under an active named profile,
    // so selection would produce a firmware: prefix instead of copilot:.
    // Clearing the envs without the env lock races concurrent guard drops,
    // so hold the lock via with_temp_jcode_home for the whole body.
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        configure_test_remote_models_with_copilot(&mut app);

        app.open_model_picker();

        let picker = app
            .inline_interactive_state
            .as_ref()
            .expect("model picker should be open");

        // Find grok-code-fast-1 (which should only be a copilot route)
        let grok_idx = picker
            .entries
            .iter()
            .position(|m| m.name == "grok-code-fast-1")
            .expect("grok-code-fast-1 should be in picker");

        // Navigate to it and select
        let filtered_pos = picker
            .filtered
            .iter()
            .position(|&i| i == grok_idx)
            .expect("grok-code-fast-1 should be in filtered list");

        // Set the selected position to grok's position
        app.inline_interactive_state.as_mut().unwrap().selected = filtered_pos;

        // Press Enter to select
        app.handle_key(KeyCode::Enter, KeyModifiers::empty())
            .unwrap();

        // In remote mode, selection should produce a pending_model_switch with copilot: prefix
        if let Some(ref spec) = app.pending_model_switch {
            assert!(
                spec.starts_with("copilot:"),
                "copilot model should be prefixed with 'copilot:', got: {}",
                spec
            );
        }
        // Picker should be closed
        assert!(app.inline_interactive_state.is_none());
    });
}
