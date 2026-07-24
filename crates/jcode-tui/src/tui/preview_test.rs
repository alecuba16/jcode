//! Render a preview frame with all info widgets populated so the user can
//! see where each widget appears in the layout. Run with:
//!   scripts/dev_cargo.sh test --profile selfdev -p jcode-tui -- preview_all_widgets --nocapture

use crate::tui::info_widget::{
    AuthMethod, BackgroundInfo, CacheHitInfo, CacheMissAttribution, CompactionInfo,
    GitInfo, InfoWidgetData, MemoryInfo, SwarmInfo, UsageInfo, UsageProvider,
    WidgetKind,
};

#[test]
fn preview_all_widgets() {
    let data = InfoWidgetData {
        todos: vec![
            crate::todo::TodoItem {
                content: "Review PR".to_string(),
                status: "pending".to_string(),
                priority: "high".to_string(),
                id: "1".to_string(),
                ..Default::default()
            },
        ],
        todo_goals: vec![],
        todos_are_swarm_plan: false,
        context_info: Some(crate::prompt::ContextInfo {
            system_prompt_chars: 5000,
            user_messages_chars: 40_000,
            total_chars: 45_000,
            ..Default::default()
        }),
        context_info_stale: false,
        queue_mode: Some(false),
        context_limit: Some(200_000),
        model: Some("claude-sonnet-4".to_string()),
        reasoning_effort: Some("high".to_string()),
        service_tier: Some("priority".to_string()),
        native_compaction_mode: Some("rolling".to_string()),
        native_compaction_threshold_tokens: Some(32_000),
        session_count: Some(3),
        session_name: Some("fix/info_fields".to_string()),
        working_dir: Some("/Users/test/projects/jcode".to_string()),
        client_count: Some(2),
        memory_info: Some(MemoryInfo {
            total_count: 42,
            project_count: 15,
            global_count: 27,
            ..Default::default()
        }),
        swarm_info: Some(SwarmInfo {
            session_count: 3,
            subagent_status: None,
            client_count: Some(2),
            session_names: vec!["worker-1".to_string(), "worker-2".to_string()],
            members: vec![],
            selected: 0,
            focused: false,
            plan_progress: Some((2, 1, 5)),
            spinner_frame: 0,
            managed_members: vec![],
        }),
        background_info: Some(BackgroundInfo {
            running_count: 2,
            running_tasks: vec!["Running tests".to_string(), "Building jcode".to_string()],
            progress_summary: Some("2 tasks running".to_string()),
            progress_detail: None,
            memory_agent_active: false,
            memory_agent_turns: 0,
        }),
        usage_info: Some(UsageInfo {
            provider: UsageProvider::Anthropic,
            available: true,
            five_hour: 0.35,
            seven_day: 0.12,
            five_hour_resets_at: Some("2026-07-24T22:00:00Z".to_string()),
            seven_day_resets_at: Some("2026-07-26T00:00:00Z".to_string()),
            primary_limit_label: Some("5h".to_string()),
            secondary_limit_label: Some("7d".to_string()),
            ..Default::default()
        }),
        tokens_per_second: Some(42.5),
        provider_name: Some("anthropic".to_string()),
        auth_method: AuthMethod::AnthropicOAuth,
        upstream_provider: None,
        connection_type: Some("websocket".to_string()),
        diagrams: vec![],
        workspace_rows: vec![],
        workspace_animation_tick: 0,
        ambient_info: None,
        observed_context_tokens: Some(12_000),
        cache_hit_info: Some(CacheHitInfo {
            reported_input_tokens: 50_000,
            read_tokens: 35_000,
            creation_tokens: 2_000,
            optimal_input_tokens: 40_000,
            last_reported_input_tokens: Some(48_000),
            last_read_tokens: Some(33_000),
            last_creation_tokens: Some(0),
            last_optimal_input_tokens: Some(38_000),
            miss_attributions: vec![CacheMissAttribution {
                turn_number: 5,
                call_index: 1,
                missed_tokens: 12_000,
                reason: "provider switch".to_string(),
            }],
        }),
        compaction_info: Some(CompactionInfo {
            is_compacting: false,
            compacted_messages: 3,
            active_messages: 20,
            summary_chars: 1500,
            mode: "rolling".to_string(),
        }),
        is_compacting: false,
        git_info: Some(GitInfo {
            branch: "fix/info_fields".to_string(),
            modified: 2,
            staged: 0,
            untracked: 0,
            ahead: 3,
            behind: 0,
            dirty_files: vec![
                "info_widget.rs".to_string(),
                "ui_input.rs".to_string(),
            ],
        }),
        // status_line_active = false so ALL widgets show (no suppression)
        status_line_active: false,
        status_line_pinned: false,
        mcp_servers: vec![
            ("codebase-memory".to_string(), 8),
            ("github".to_string(), 12),
        ],
        available_skills: vec![
            "codebase-memory".to_string(),
            "code-review-excellence".to_string(),
        ],
    };

    // Render with status line OFF (all widgets visible)
    println!("\n=== ALL WIDGETS (status_line_active=false) ===\n");
    let available = data.available_widgets();
    println!("Available widgets: {available:?}");
    for kind in &available {
        let height = crate::tui::info_widget::calculate_widget_height(
            *kind,
            &data,
            40,
            30,
        );
        println!("  {kind:?}: height={height}");
    }

    // Now with status line ON (supplementary only)
    let mut data_active = data.clone();
    data_active.status_line_active = true;
    println!("\n=== SUPPLEMENTARY ONLY (status_line_active=true) ===\n");
    let available_active = data_active.available_widgets();
    println!("Available widgets: {available_active:?}");
    for kind in &available_active {
        let height = crate::tui::info_widget::calculate_widget_height(
            *kind,
            &data_active,
            40,
            30,
        );
        println!("  {kind:?}: height={height}");
    }

    // Render the actual widget content for BOTH modes
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    for (label, render_data) in [
        ("FULL (status_line=false)", &data),
        ("SUPPLEMENTARY (status_line=true)", &data_active),
    ] {
        let backend = TestBackend::new(80, 50);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let available = render_data.available_widgets();
                let mut y = 0u16;
                for kind in &available {
                    let lines = crate::tui::info_widget::render_widget_content(
                        *kind,
                        render_data,
                        ratatui::layout::Rect::new(1, y + 1, 78, 49 - y),
                    );
                    // Draw a border for the widget
                    let h = (lines.len() as u16 + 2).min(49 - y);
                    let area = ratatui::layout::Rect::new(0, y, 80, h);
                    frame.render_widget(
                        ratatui::widgets::Block::default()
                            .borders(ratatui::widgets::Borders::ALL)
                            .title(format!(" {kind:?} ")),
                        area,
                    );
                    for (i, line) in lines.iter().enumerate() {
                        frame.render_widget(
                            ratatui::widgets::Paragraph::new(line.clone()),
                            ratatui::layout::Rect::new(1, y + 1 + i as u16, 78, 1),
                        );
                    }
                    y += h + 1;
                    if y >= 49 {
                        break;
                    }
                }
            })
            .unwrap();

        let buf = terminal.backend().buffer().clone();
        println!("\n=== RENDERED PREVIEW: {label} ===\n");
        for y in 0..50u16 {
            let mut line = String::new();
            for x in 0..80u16 {
                let cell = buf.get(x, y);
                line.push_str(cell.symbol());
            }
            if !line.trim().is_empty() {
                println!("{y:2}: {line}");
            }
        }
    }
}