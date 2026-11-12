// Regression tests for issue #438: in remote sessions, the runtime model
// picker preview advertises Ctrl+N (toggle favorite) and Ctrl+O (set default),
// but the remote key path never routed those chords to
// `model_picker_preview_hotkey`. They fell through to the remote global
// Ctrl-key handling and were swallowed as unrecognized hotkeys.

fn remote_model_picker_preview_state() -> crate::tui::InlineInteractiveState {
    crate::tui::InlineInteractiveState {
        kind: crate::tui::PickerKind::Model,
        filtered: vec![0],
        entries: vec![crate::tui::PickerEntry {
            name: "gpt-5.5".to_string(),
            options: vec![crate::tui::PickerOption {
                provider: "OpenAI".to_string(),
                api_method: "openai-api".to_string(),
                available: true,
                detail: String::new(),
                estimated_reference_cost_micros: None,
            }],
            action: crate::tui::PickerAction::Model,
            selected_option: 0,
            is_current: false,
            is_default: false,
            is_favorite: false,
            is_memory_model: false,
            recommended: false,
            recommendation_rank: usize::MAX,
            usage_score: 0,
            old: false,
            created_date: None,
            effort: None,
        }],
        selected: 0,
        column: 0,
        filter: String::new(),
        preview: true,
    }
}

#[test]
fn test_remote_model_picker_preview_ctrl_n_toggles_favorite() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();
        let mut remote = crate::tui::backend::RemoteConnection::dummy();

        app.is_remote = true;
        app.inline_interactive_state = Some(remote_model_picker_preview_state());

        rt.block_on(app.handle_remote_key(KeyCode::Char('n'), KeyModifiers::CONTROL, &mut remote))
            .expect("Ctrl+N should be handled in the remote path");

        let picker = app
            .inline_interactive_state
            .as_ref()
            .expect("picker preview should stay open after Ctrl+N");
        assert!(picker.preview, "picker should remain in preview mode");
        assert!(
            picker.entries[0].is_favorite,
            "Ctrl+N must toggle the selected model as a favorite in remote sessions"
        );
        assert!(
            app.status_notice()
                .is_some_and(|notice| notice.contains("Favorited")),
            "favorite toggle should surface a status notice, got: {:?}",
            app.status_notice()
        );

        // Toggling again must unfavorite, proving the chord is consumed by the
        // picker on every press instead of falling through once.
        rt.block_on(app.handle_remote_key(KeyCode::Char('n'), KeyModifiers::CONTROL, &mut remote))
            .expect("second Ctrl+N should be handled in the remote path");
        let picker = app
            .inline_interactive_state
            .as_ref()
            .expect("picker preview should stay open");
        assert!(!picker.entries[0].is_favorite);
    });
}

#[test]
fn test_remote_model_picker_preview_ctrl_o_sets_default() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();
        let mut remote = crate::tui::backend::RemoteConnection::dummy();

        app.is_remote = true;
        app.inline_interactive_state = Some(remote_model_picker_preview_state());

        rt.block_on(app.handle_remote_key(KeyCode::Char('o'), KeyModifiers::CONTROL, &mut remote))
            .expect("Ctrl+O should be handled in the remote path");

        let picker = app
            .inline_interactive_state
            .as_ref()
            .expect("picker preview should stay open after Ctrl+O");
        assert!(picker.preview, "picker should remain in preview mode");
        assert!(
            picker.entries[0].is_default,
            "Ctrl+O must mark the selected model as the default in remote sessions"
        );
        assert!(
            app.display_messages()
                .iter()
                .any(|msg| msg.content.contains("Saved default model")),
            "setting the default should confirm with a system message"
        );
    });
}

#[test]
fn test_model_picker_chords_do_not_pollute_filter() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        app.is_remote = true;
        app.remote_available_entries = vec!["gpt-5.5".to_string()];
        app.remote_model_options = vec![crate::provider::ModelRoute {
            model: "gpt-5.5".to_string(),
            provider: "OpenAI".to_string(),
            api_method: "openai".to_string(),
            available: true,
            detail: String::new(),
            cheapness: None,
            usage: None,
        }];

        app.open_model_picker();
        assert!(
            app.inline_interactive_state.is_some(),
            "model picker should be open"
        );

        // Ctrl+S inside the picker must not leak a stray 's' into the filter:
        // it collides with the global input-stash chord, so it is a dead key
        // here (never opens the swarm sub-picker, never types).
        app.handle_key(KeyCode::Char('s'), KeyModifiers::CONTROL)
            .unwrap();
        let picker = app
            .inline_interactive_state
            .as_ref()
            .expect("picker must stay open after Ctrl+S");
        assert!(
            picker.filter.is_empty(),
            "Ctrl+S must not type into the filter"
        );
        assert!(
            !picker
                .entries
                .iter()
                .any(|e| matches!(e.action, crate::tui::PickerAction::AgentModelChoice { .. })),
            "Ctrl+S must not open the swarm sub-picker"
        );

        // Alt+S still opens the swarm sub-picker from the preview.
        app.handle_key(KeyCode::Char('s'), KeyModifiers::ALT)
            .unwrap();
        let picker = app
            .inline_interactive_state
            .as_ref()
            .expect("picker must stay open after Alt+S");
        assert!(
            picker.entries.iter().any(|e| matches!(
                e.action,
                crate::tui::PickerAction::AgentModelChoice {
                    target: crate::tui::AgentModelTarget::Swarm,
                    ..
                }
            )),
            "Alt+S must open the swarm sub-picker"
        );
    });
}

#[test]
fn test_model_picker_preview_keeps_ctrl_s_as_input_stash() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        app.is_remote = true;
        app.remote_available_entries = vec!["gpt-5.5".to_string()];
        app.remote_model_options = vec![crate::provider::ModelRoute {
            model: "gpt-5.5".to_string(),
            provider: "OpenAI".to_string(),
            api_method: "openai".to_string(),
            available: true,
            detail: String::new(),
            cheapness: None,
            usage: None,
        }];

        // Type `/model` so the preview is open with a draft behind it.
        for c in "/model".chars() {
            app.handle_key(KeyCode::Char(c), KeyModifiers::empty())
                .unwrap();
        }
        let picker = app
            .inline_interactive_state
            .as_ref()
            .expect("typing /model should open the picker preview");
        assert!(picker.preview);

        // Ctrl+S must keep its global meaning while the preview is open:
        // stash the draft, never open a sub-picker, never type into the
        // filter. This is the muscle-memory collision the rebind fixed.
        app.handle_key(KeyCode::Char('s'), KeyModifiers::CONTROL)
            .unwrap();
        let picker = app
            .inline_interactive_state
            .as_ref()
            .expect("preview must stay open after Ctrl+S");
        assert!(
            picker.filter.is_empty(),
            "Ctrl+S must not type into the filter"
        );
        assert!(
            !picker
                .entries
                .iter()
                .any(|e| matches!(e.action, crate::tui::PickerAction::AgentModelChoice { .. })),
            "Ctrl+S must not open an agent sub-picker"
        );
        assert!(
            app.stashed_input.is_some(),
            "Ctrl+S with the preview open must stash the draft"
        );
        assert_eq!(app.input(), "");
    });
}

#[test]
fn test_remote_model_picker_alt_chords_open_sub_pickers() {
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();
        let mut remote = crate::tui::backend::RemoteConnection::dummy();

        app.is_remote = true;
        app.remote_available_entries = vec!["gpt-5.5".to_string()];
        app.remote_model_options = vec![crate::provider::ModelRoute {
            model: "gpt-5.5".to_string(),
            provider: "OpenAI".to_string(),
            api_method: "openai".to_string(),
            available: true,
            detail: String::new(),
            cheapness: None,
            usage: None,
        }];

        for c in "/model".chars() {
            rt.block_on(app.handle_remote_key(
                KeyCode::Char(c),
                KeyModifiers::empty(),
                &mut remote,
            ))
            .unwrap();
        }
        assert!(
            app.inline_interactive_state
                .as_ref()
                .is_some_and(|p| p.preview),
            "typed /model should open the picker preview"
        );

        // The remote event loop must route the preview chords to the picker,
        // not drop them into the global ctrl/alt handling.
        rt.block_on(app.handle_remote_key(KeyCode::Char('s'), KeyModifiers::ALT, &mut remote))
            .expect("Alt+S should be handled in the remote path");
        let picker = app
            .inline_interactive_state
            .as_ref()
            .expect("swarm sub-picker should be open after Alt+S");
        assert!(picker.entries.iter().any(|e| matches!(
            e.action,
            crate::tui::PickerAction::AgentModelChoice {
                target: crate::tui::AgentModelTarget::Swarm,
                ..
            }
        )));

        // Back out, reopen the /model preview, then Alt+M for memory.
        rt.block_on(app.handle_remote_key(KeyCode::Esc, KeyModifiers::empty(), &mut remote))
            .unwrap();
        for c in "/model".chars() {
            rt.block_on(app.handle_remote_key(
                KeyCode::Char(c),
                KeyModifiers::empty(),
                &mut remote,
            ))
            .unwrap();
        }
        rt.block_on(app.handle_remote_key(KeyCode::Char('m'), KeyModifiers::ALT, &mut remote))
            .expect("Alt+M should be handled in the remote path");
        let picker = app
            .inline_interactive_state
            .as_ref()
            .expect("memory sub-picker should be open after Alt+M");
        assert!(picker.entries.iter().any(|e| matches!(
            e.action,
            crate::tui::PickerAction::AgentModelChoice {
                target: crate::tui::AgentModelTarget::Memory,
                ..
            }
        )));
    });
}

#[test]
fn test_agent_model_chord_shadows_user_remap_only_inside_model_picker() {
    // A user remapping model_switch_next to alt+s must not lose the chord's
    // picker meaning: inside the runtime /model picker the chord wins (the
    // picker dispatches before the config bindings), while anywhere else the
    // user's remap keeps working. Pin the precedence so a future reorder of
    // handle_key_core / handle_remote_key_internal cannot silently flip it.
    let _guard = crate::storage::lock_test_env();
    let temp = tempfile::tempdir().expect("tempdir");
    let prev_home = std::env::var_os("JCODE_HOME");
    crate::env::set_var("JCODE_HOME", temp.path());
    crate::config::Config::invalidate_cache();

    let config_path = crate::config::Config::path().expect("config path");
    std::fs::create_dir_all(config_path.parent().expect("config parent"))
        .expect("create config parent");
    std::fs::write(
        &config_path,
        "[keybindings]\nmodel_switch_next = \"alt+s\"\n",
    )
    .expect("write config");

    let mut app = create_test_app();
    app.is_remote = true;
    app.remote_available_entries = vec!["gpt-5.5".to_string()];
    app.remote_model_options = vec![crate::provider::ModelRoute {
        model: "gpt-5.5".to_string(),
        provider: "OpenAI".to_string(),
        api_method: "openai".to_string(),
        available: true,
        detail: String::new(),
        cheapness: None,
        usage: None,
    }];
    app.open_model_picker();

    // Inside the picker the hardcoded chord wins even though the user
    // remapped model_switch_next to the same chord.
    app.handle_key(KeyCode::Char('s'), KeyModifiers::ALT)
        .unwrap();
    assert!(
        app.inline_interactive_state
            .as_ref()
            .is_some_and(|p| p.entries.iter().any(|e| matches!(
                e.action,
                crate::tui::PickerAction::AgentModelChoice {
                    target: crate::tui::AgentModelTarget::Swarm,
                    ..
                }
            ))),
        "the agent-model chord must win inside the model picker even when a user remap collides"
    );

    // Close the picker; outside the picker the remap must still fire as
    // model switching, not open anything.
    app.handle_key(KeyCode::Esc, KeyModifiers::empty()).unwrap();
    assert!(app.inline_interactive_state.is_none());
    app.handle_key(KeyCode::Char('s'), KeyModifiers::ALT)
        .unwrap();
    assert!(
        app.inline_interactive_state.is_none(),
        "outside the picker the user's alt+s remap must not open a sub-picker"
    );

    if let Some(home) = prev_home {
        crate::env::set_var("JCODE_HOME", home);
    } else {
        crate::env::remove_var("JCODE_HOME");
    }
    crate::config::Config::invalidate_cache();
}

#[test]
fn test_local_model_picker_alt_m_marks_memory_model() {
    // Alt+M (not Ctrl+M) marks the highlighted model as the memory model in
    // the /model picker. In legacy terminal encoding Ctrl+M is delivered as
    // the same byte as Enter (\r), so it can never reach this handler; the
    // chord must be Alt+M for the marking to be reachable in practice.
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();

        app.inline_interactive_state = Some(remote_model_picker_preview_state());

        app.handle_key(KeyCode::Char('m'), KeyModifiers::ALT)
            .expect("Alt+M should be handled in the local path");

        let picker = app
            .inline_interactive_state
            .as_ref()
            .expect("picker preview should stay open after Alt+M");
        assert!(picker.preview, "picker should remain in preview mode");
        assert!(
            picker.entries[0].is_memory_model,
            "Alt+M must mark the selected model as the memory model"
        );
        assert!(
            app.status_notice()
                .is_some_and(|notice| notice.contains("Marked as memory model")),
            "memory marking should surface a status notice, got: {:?}",
            app.status_notice()
        );

        // The mark must persist to disk. Regression guard: the save used to
        // fail silently when the app config dir did not exist yet (fresh
        // installs), because the memory-model writer skipped
        // `create_dir_all` on the parent directory.
        {
            let store_path = crate::storage::app_config_dir()
                .expect("app config dir")
                .join("memory_model.json");
            let raw = std::fs::read_to_string(&store_path)
                .expect("memory model store must persist after Alt+M");
            assert!(
                raw.contains("gpt-5.5"),
                "persisted memory model store should name the marked model: {}",
                raw
            );
        }

        // Toggling again must unmark, proving the chord is consumed by the
        // picker on every press.
        app.handle_key(KeyCode::Char('m'), KeyModifiers::ALT)
            .expect("second Alt+M should be handled");

        let picker = app
            .inline_interactive_state
            .as_ref()
            .expect("picker should still be open");
        assert!(
            !picker.entries[0].is_memory_model,
            "second Alt+M must unmark the memory model"
        );

        // The old chord must be dead: Ctrl+M (delivered as Enter \r by most
        // terminals, but simulated directly here) must not toggle marking.
        // Enter confirms the selection instead, closing the preview state
        // route for memory marking. We assert the marking flag never flips
        // back on from the legacy chord.
        app.inline_interactive_state = Some(remote_model_picker_preview_state());
        app.handle_key(KeyCode::Char('m'), KeyModifiers::CONTROL)
            .expect("Ctrl+M key event should still be consumed without panic");
        let picker = app
            .inline_interactive_state
            .as_ref()
            .expect("picker state should still be inspectable");
        assert!(
            !picker.entries[0].is_memory_model,
            "Ctrl+M must no longer mark the memory model"
        );

        // macOS terminals with the default Option behavior insert Option+M as
        // `µ` with no ALT modifier. The picker must map it back to Alt+M so the
        // advertised chord works there too, mirroring the global side-panel
        // toggle's fallback. On non-macOS platforms this arm is a no-op.
        app.inline_interactive_state = Some(remote_model_picker_preview_state());
        if cfg!(target_os = "macos") {
            app.handle_key(KeyCode::Char('µ'), KeyModifiers::NONE)
                .expect("µ (macOS Option+M) should be consumed");
            let picker = app
                .inline_interactive_state
                .as_ref()
                .expect("picker should stay open after µ");
            assert!(
                picker.entries[0].is_memory_model,
                "macOS Option+M (µ) must mark the memory model"
            );
        }
    });
}

#[test]
fn test_alt_m_with_picker_open_does_not_touch_side_panel() {
    // While the model picker is open, Alt+M belongs to the picker (mark the
    // memory model). The side-panel toggle shares the same chord and sits
    // later in the dispatch chain; it must not fire from under the picker.
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        app.side_panel_user_hidden = false;
        app.inline_interactive_state = Some(remote_model_picker_preview_state());

        app.handle_key(KeyCode::Char('m'), KeyModifiers::ALT)
            .expect("Alt+M should be handled with the picker open");

        let picker = app
            .inline_interactive_state
            .as_ref()
            .expect("picker should stay open after Alt+M");
        assert!(
            picker.entries[0].is_memory_model,
            "Alt+M must mark the memory model while the picker is open"
        );
        assert!(
            !app.side_panel_user_hidden,
            "Alt+M inside the picker must not dismiss the side panel"
        );
    });
}

#[test]
fn test_alt_m_without_picker_still_toggles_side_panel() {
    // With no picker open, Alt+M keeps its global meaning: the side-panel
    // toggle. Guard the picker claim from leaking into the global path.
    with_temp_jcode_home(|| {
        let mut app = create_test_app();
        app.side_panel_user_hidden = false;
        assert!(app.inline_interactive_state.is_none());

        app.handle_key(KeyCode::Char('m'), KeyModifiers::ALT)
            .expect("Alt+M should be handled with no picker open");

        // The test app has no side-panel pages, so the toggle lands on its
        // "no pages" notice branch instead of flipping the hidden flag.
        // The notice still proves the chord reached the side-panel toggle
        // and was not swallowed by the picker's memory-marking claim.
        assert!(
            app.status_notice()
                .is_some_and(|notice| notice.contains("Side panel: no pages")),
            "Alt+M with no picker must fall through to the side-panel toggle, got: {:?}",
            app.status_notice()
        );
    });
}
