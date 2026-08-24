use super::{
    BackgroundInfo, CacheHitInfo, CacheMissAttribution, GraphEdge, GraphNode, InfoWidgetData,
    InjectedMemoryItem, Margins, MemoryActivity, MemoryEvent, MemoryEventKind, MemoryInfo,
    MemoryState, PipelineState, StepStatus, SwarmInfo, UsageInfo, UsageProvider, WidgetKind,
    calculate_placements, calculate_widget_height, dashed_separator, effective_prompt_tokens,
    format_age, format_memory_count, memory_active_summary, occasional_status_tip,
    render_cost_tokens_line, render_kv_cache_summary_line, render_mcp_servers_line,
    render_memory_compact, render_memory_widget, render_recovered_memories_widget,
    render_skills_line, render_todos_compact, render_todos_expanded, render_usage_compact,
    swarm_plan_todos, truncate_smart, wrap_text,
};
use crate::protocol::SwarmMemberStatus;
use ratatui::layout::Rect;
use std::time::{Duration, Instant};

#[test]
fn effective_prompt_tokens_handles_split_and_subset_accounting() {
    // Anthropic-style split accounting: `input` is only the uncached remainder,
    // so cache_read pushed beyond input means the true prompt is the sum.
    assert_eq!(effective_prompt_tokens(2449, 19499, 684), 22632);
    // OpenAI-style subset accounting: cached tokens are inside `input`.
    assert_eq!(effective_prompt_tokens(10000, 6000, 0), 10000);
    // No cache telemetry at all behaves like a plain input count.
    assert_eq!(effective_prompt_tokens(5000, 0, 0), 5000);
}

#[test]
fn cache_hit_ratio_uses_effective_prompt_for_split_providers() {
    // Mirrors a real Anthropic log line where read >> input and the old code
    // clamped the ratio to 100%.
    let cache = CacheHitInfo {
        reported_input_tokens: 2449,
        read_tokens: 19499,
        creation_tokens: 684,
        ..Default::default()
    };
    // 19499 / (2449 + 19499 + 684) = 0.8616...
    let ratio = cache.hit_ratio().expect("ratio");
    assert!((ratio - 0.8616).abs() < 0.01, "ratio was {ratio}");
}

#[test]
fn truncate_smart_handles_unicode() {
    let s = "eagle running - keep going";
    let out = truncate_smart(s, 15);
    assert_eq!(out, "eagle runnin...");
}

#[test]
fn occasional_status_tip_only_shows_during_part_of_cycle() {
    assert!(occasional_status_tip(60, 5).is_none());
    assert!(occasional_status_tip(60, 27).is_none());
    assert!(occasional_status_tip(60, 28).is_some());
    assert!(occasional_status_tip(60, 39).is_some());
    assert!(occasional_status_tip(60, 40).is_none());
    assert!(occasional_status_tip(60, 89).is_none());
}

#[test]
fn kv_cache_widget_shows_session_hit_ratio() {
    let data = InfoWidgetData {
        cache_hit_info: Some(CacheHitInfo {
            reported_input_tokens: 20_000,
            read_tokens: 15_000,
            creation_tokens: 3_000,
            optimal_input_tokens: 16_667,
            last_reported_input_tokens: Some(10_000),
            last_read_tokens: Some(9_400),
            last_creation_tokens: Some(0),
            last_optimal_input_tokens: Some(9_895),
            miss_attributions: vec![CacheMissAttribution {
                turn_number: 20,
                call_index: 1,
                missed_tokens: 69_000,
                reason: "provider switch".to_string(),
            }],
        }),
        ..Default::default()
    };

    assert!(data.has_data_for(WidgetKind::KvCache));
    let cache = data.cache_hit_info.as_ref().unwrap();
    let lines = render_kv_cache_summary_line(cache);
    let text = lines_text(&lines);

    // The summary line shows header + yield/warm + session stats.
    // Full miss-attribution detail is no longer rendered as a standalone widget.
    assert!(text.contains("KV cache:"));
    assert!(text.contains("yield "));
    assert!(text.contains("90%"));
    assert!(text.contains("session "));
    assert!(text.contains("39%"));
}

#[test]
fn todos_widgets_show_item_and_aggregate_confidence() {
    let data = InfoWidgetData {
        todos: vec![
            crate::todo::TodoItem {
                group: None,
                id: "todo-1".to_string(),
                content: "Validate confidence UI".to_string(),
                status: "in_progress".to_string(),
                priority: "high".to_string(),
                confidence: Some(crate::todo::ConfidenceState::from_legacy_score(80)),
                completion_confidence: None,
                confidence_history: Vec::new(),
                blocked_by: Vec::new(),
                assigned_to: None,
            },
            crate::todo::TodoItem {
                group: None,
                id: "todo-2".to_string(),
                content: "Ship completed item".to_string(),
                status: "completed".to_string(),
                priority: "medium".to_string(),
                confidence: Some(crate::todo::ConfidenceState::from_legacy_score(70)),
                completion_confidence: Some(crate::todo::ConfidenceState::from_legacy_score(95)),
                confidence_history: Vec::new(),
                blocked_by: Vec::new(),
                assigned_to: None,
            },
        ],
        ..Default::default()
    };

    let normal_text = lines_text(&render_todos_expanded(&data, Rect::new(0, 0, 80, 8)));
    assert!(normal_text.contains("plausible"));
    assert!(normal_text.contains("plausible"));
    assert!(normal_text.contains("plausible"));

    let expanded_text = lines_text(&render_todos_expanded(&data, Rect::new(0, 0, 80, 8)));
    assert!(expanded_text.contains("plausible"));
    assert!(expanded_text.contains("plausible"));
    assert!(expanded_text.contains("plausible"));

    let compact_text = lines_text(&render_todos_compact(&data, Rect::new(0, 0, 80, 2)));
    assert!(compact_text.contains("plausible"));
}

#[test]
fn todos_widgets_render_group_headers_when_groups_present() {
    let mk = |group: Option<&str>, id: &str, status: &str| crate::todo::TodoItem {
        group: group.map(|g| g.to_string()),
        id: id.to_string(),
        content: format!("task {id}"),
        status: status.to_string(),
        priority: "medium".to_string(),
        confidence: Some(crate::todo::ConfidenceState::from_legacy_score(80)),
        completion_confidence: None,
        confidence_history: Vec::new(),
        blocked_by: Vec::new(),
        assigned_to: None,
    };
    let data = InfoWidgetData {
        todos: vec![
            mk(Some("optimize rendering"), "a", "completed"),
            mk(Some("optimize rendering"), "b", "in_progress"),
            mk(Some("fix scrollback"), "c", "pending"),
            mk(None, "d", "pending"),
        ],
        ..Default::default()
    };

    let expanded = lines_text_concat(&render_todos_expanded(&data, Rect::new(0, 0, 80, 14)));
    // Group headers appear with per-group progress counters, first-seen order,
    // and the ungrouped bucket renders under "Other".
    assert!(expanded.contains("optimize rendering"), "{expanded}");
    assert!(expanded.contains("1/2"), "{expanded}");
    assert!(
        expanded.contains("1/2 · confidence plausible"),
        "group confidence missing: {expanded}"
    );
    assert!(expanded.contains("fix scrollback"), "{expanded}");
    assert!(expanded.contains("Other"), "{expanded}");
    let opt_idx = expanded.find("optimize rendering").unwrap();
    let fix_idx = expanded.find("fix scrollback").unwrap();
    let other_idx = expanded.find("Other").unwrap();
    assert!(opt_idx < fix_idx, "first-seen group order: {expanded}");
    assert!(fix_idx < other_idx, "ungrouped bucket last: {expanded}");
}

#[test]
fn task_group_headers_render_their_own_weighted_confidence() {
    let mk = |group: &str, id: &str, priority: &str, confidence: u8| crate::todo::TodoItem {
        group: Some(group.to_string()),
        id: id.to_string(),
        content: format!("task {id}"),
        status: "pending".to_string(),
        priority: priority.to_string(),
        confidence: Some(crate::todo::ConfidenceState::from_legacy_score(confidence)),
        completion_confidence: None,
        confidence_history: Vec::new(),
        blocked_by: Vec::new(),
        assigned_to: None,
    };
    let data = InfoWidgetData {
        todos: vec![
            mk("high confidence", "a", "high", 100),
            mk("high confidence", "b", "low", 40),
            mk("lower confidence", "c", "medium", 60),
        ],
        ..Default::default()
    };

    for text in [
        lines_text_concat(&render_todos_expanded(&data, Rect::new(0, 0, 90, 10))),
        lines_text_concat(&render_todos_expanded(&data, Rect::new(0, 0, 90, 14))),
    ] {
        assert!(
            text.contains("high confidence 0/2 · confidence plausible"),
            "weighted group confidence missing: {text}"
        );
        assert!(
            text.contains("lower confidence 0/1 · confidence plausible"),
            "group-scoped confidence missing: {text}"
        );
    }
}

#[test]
fn todos_widgets_stay_flat_without_groups() {
    let mk = |id: &str, status: &str| crate::todo::TodoItem {
        group: None,
        id: id.to_string(),
        content: format!("task {id}"),
        status: status.to_string(),
        priority: "medium".to_string(),
        confidence: Some(crate::todo::ConfidenceState::from_legacy_score(80)),
        completion_confidence: None,
        confidence_history: Vec::new(),
        blocked_by: Vec::new(),
        assigned_to: None,
    };
    let data = InfoWidgetData {
        todos: vec![mk("a", "completed"), mk("b", "pending")],
        ..Default::default()
    };
    let expanded = lines_text(&render_todos_expanded(&data, Rect::new(0, 0, 80, 14)));
    assert!(!expanded.contains("Other"), "no group bucket: {expanded}");
}

#[test]
fn todos_widget_renders_exact_pips_for_small_lists() {
    let mk = |status: &str| crate::todo::TodoItem {
        group: None,
        id: status.to_string(),
        content: format!("item {status}"),
        status: status.to_string(),
        priority: "medium".to_string(),
        confidence: Some(crate::todo::ConfidenceState::from_legacy_score(80)),
        completion_confidence: None,
        confidence_history: Vec::new(),
        blocked_by: Vec::new(),
        assigned_to: None,
    };
    let data = InfoWidgetData {
        todos: vec![
            mk("completed"),
            mk("completed"),
            mk("in_progress"),
            mk("pending"),
        ],
        ..Default::default()
    };

    let lines = render_todos_expanded(&data, Rect::new(0, 0, 80, 8));
    let header = lines_text(&lines[..1]);
    // Exact 1:1 pips on the header: 2 done + 1 active render as filled ●,
    // 1 open renders as hollow ○. (Active is full amber, not half.)
    assert_eq!(
        header.matches('●').count(),
        3,
        "expected 3 filled pips: {header}"
    );
    assert_eq!(
        header.matches('○').count(),
        1,
        "expected 1 open pip: {header}"
    );
    assert!(
        !header.contains('◐'),
        "active pip should be full, not half: {header}"
    );
    // The old block bar should be gone everywhere.
    let all = lines_text(&lines);
    assert!(!all.contains('█'), "old block bar should be gone: {all}");
    assert!(!all.contains('░'), "old empty bar should be gone: {all}");
}

fn plan_item(id: &str, status: &str) -> crate::plan::PlanItem {
    crate::plan::PlanItem {
        content: format!("task {id}"),
        status: status.to_string(),
        priority: "medium".to_string(),
        id: id.to_string(),
        subsystem: None,
        file_scope: Vec::new(),
        blocked_by: Vec::new(),
        assigned_to: None,
    }
}

#[test]
fn swarm_plan_todos_normalizes_scheduler_statuses() {
    let items = vec![
        plan_item("a", "running"),
        plan_item("b", "running_stale"),
        plan_item("c", "done"),
        plan_item("d", "completed"),
        plan_item("e", "failed"),
        plan_item("f", "stopped"),
        plan_item("g", "crashed"),
        plan_item("h", "queued"),
        plan_item("i", "ready"),
        plan_item("j", "blocked"),
        plan_item("k", "pending"),
        plan_item("l", "in_progress"),
        plan_item("m", "weird_custom_status"),
    ];
    let todos = swarm_plan_todos(&items);
    let status_of = |id: &str| {
        todos
            .iter()
            .find(|t| t.id == id)
            .map(|t| t.status.clone())
            .unwrap()
    };
    // Active scheduler states surface as in_progress (▶ amber, sorts first).
    assert_eq!(status_of("a"), "in_progress");
    assert_eq!(status_of("b"), "in_progress");
    // Terminal success maps onto completed (✓).
    assert_eq!(status_of("c"), "completed");
    assert_eq!(status_of("d"), "completed");
    // Terminal failure maps onto cancelled (✗) instead of an open circle.
    assert_eq!(status_of("e"), "cancelled");
    assert_eq!(status_of("f"), "cancelled");
    assert_eq!(status_of("g"), "cancelled");
    // Runnable / blocked states render as pending (○).
    assert_eq!(status_of("h"), "pending");
    assert_eq!(status_of("i"), "pending");
    assert_eq!(status_of("j"), "pending");
    // Statuses the todo renderer already understands pass through.
    assert_eq!(status_of("k"), "pending");
    assert_eq!(status_of("l"), "in_progress");
    // Arbitrary strings pass through unchanged (rendered as open ○).
    assert_eq!(status_of("m"), "weird_custom_status");
}

#[test]
fn swarm_plan_todos_preserve_blockers_and_assignee_and_flow_to_renderer() {
    let mut blocked = plan_item("audit-x", "queued");
    blocked.blocked_by = vec!["audit-y".to_string()];
    let mut running = plan_item("audit-y", "running");
    running.assigned_to = Some("worker-1".to_string());
    let items = vec![blocked, running];

    let todos = swarm_plan_todos(&items);
    assert_eq!(todos[0].blocked_by, vec!["audit-y".to_string()]);
    assert_eq!(todos[1].assigned_to.as_deref(), Some("worker-1"));

    let data = InfoWidgetData {
        todos,
        ..Default::default()
    };
    let text = lines_text(&render_todos_expanded(&data, Rect::new(0, 0, 80, 14)));
    // Blocked items get the dependency marker and suffix.
    assert!(text.contains("⊳"), "blocked glyph missing: {text}");
    assert!(text.contains("(blocked)"), "blocked suffix missing: {text}");
    // The running item sorts first as in_progress.
    let running_idx = text.find("task audit-y").unwrap();
    let blocked_idx = text.find("task audit-x").unwrap();
    assert!(running_idx < blocked_idx, "active-first order: {text}");
}

#[test]
fn swarm_plan_running_items_render_before_completed_in_large_plans() {
    // 120-item deep plan: 100 completed, 1 running near the end, rest queued.
    // The running item must be visible in the small line budget instead of
    // hiding behind the "+N more" footer.
    let mut items: Vec<crate::plan::PlanItem> = (0..100)
        .map(|i| plan_item(&format!("done-{i}"), "completed"))
        .collect();
    items.push(plan_item("hot-task", "running"));
    for i in 0..19 {
        items.push(plan_item(&format!("queued-{i}"), "queued"));
    }

    let data = InfoWidgetData {
        todos: swarm_plan_todos(&items),
        ..Default::default()
    };
    let text = lines_text(&render_todos_expanded(&data, Rect::new(0, 0, 60, 8)));
    assert!(
        text.contains("task hot-task"),
        "running plan item should be visible in the budgeted list: {text}"
    );
    assert!(text.contains("+"), "footer summarizes the rest: {text}");
}

#[test]
fn todo_widget_header_says_plan_when_showing_swarm_plan_projection() {
    let items = vec![plan_item("a", "running"), plan_item("b", "queued")];
    let plan_data = InfoWidgetData {
        todos: swarm_plan_todos(&items),
        todos_are_swarm_plan: true,
        ..Default::default()
    };
    for text in [
        lines_text(&render_todos_expanded(&plan_data, Rect::new(0, 0, 60, 8))),
        lines_text(&render_todos_expanded(&plan_data, Rect::new(0, 0, 60, 14))),
        lines_text(&render_todos_compact(&plan_data, Rect::new(0, 0, 60, 3))),
    ] {
        assert!(text.contains("Plan"), "plan header missing: {text}");
        assert!(!text.contains("Todos"), "plan must not claim Todos: {text}");
    }

    let todo_data = InfoWidgetData {
        todos: swarm_plan_todos(&items),
        todos_are_swarm_plan: false,
        ..Default::default()
    };
    let text = lines_text(&render_todos_expanded(&todo_data, Rect::new(0, 0, 60, 8)));
    assert!(text.contains("Todos"), "todos header missing: {text}");
}

fn todo_item(id: &str, content: &str, status: &str, group: Option<&str>) -> crate::todo::TodoItem {
    crate::todo::TodoItem {
        content: content.to_string(),
        status: status.to_string(),
        priority: "medium".to_string(),
        id: id.to_string(),
        group: group.map(|g| g.to_string()),
        blocked_by: Vec::new(),
        assigned_to: None,
        confidence: Some(crate::todo::ConfidenceState::from_legacy_score(80)),
        completion_confidence: None,
        confidence_history: Vec::new(),
    }
}

/// Join spans without separators so assertions can match text that spans
/// multiple styled segments (e.g. "loop " + "85%").
fn lines_text_concat(lines: &[ratatui::text::Line<'_>]) -> String {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn flat_todo_list_shows_feedback_loop_assessments_on_header_in_all_widget_sizes() {
    let data = InfoWidgetData {
        todos: vec![
            todo_item("a", "optimize grep", "in_progress", None),
            todo_item("b", "add bench", "pending", None),
        ],
        todo_goals: vec![crate::todo::TodoGoal {
            group: None,
            closed_feedback_loop: Some(crate::todo::FeedbackLoopState::from_legacy_score(85)),
            feedback_loop_relevance: Some(crate::todo::FeedbackLoopRelevance::Representative),
            feedback_loop_coverage: Some(crate::todo::FeedbackLoopCoverage::MainPaths),
            ..Default::default()
        }],
        ..Default::default()
    };
    for text in [
        lines_text_concat(&render_todos_expanded(&data, Rect::new(0, 0, 70, 8))),
        lines_text_concat(&render_todos_expanded(&data, Rect::new(0, 0, 70, 14))),
        lines_text_concat(&render_todos_compact(&data, Rect::new(0, 0, 70, 3))),
    ] {
        assert!(
            text.contains("loop strong/representative/main_paths"),
            "loop suffix missing: {text}"
        );
    }
}

#[test]
fn grouped_todos_show_closed_feedback_loop_on_their_group_headers() {
    let data = InfoWidgetData {
        todos: vec![
            todo_item("a", "speed up search", "in_progress", Some("optimize grep")),
            todo_item("b", "sketch layout", "pending", Some("onboarding design")),
        ],
        todo_goals: vec![
            crate::todo::TodoGoal {
                group: Some("optimize grep".to_string()),
                closed_feedback_loop: Some(crate::todo::FeedbackLoopState::from_legacy_score(90)),
                ..Default::default()
            },
            crate::todo::TodoGoal {
                group: Some("onboarding design".to_string()),
                closed_feedback_loop: Some(crate::todo::FeedbackLoopState::from_legacy_score(20)),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    for text in [
        lines_text_concat(&render_todos_expanded(&data, Rect::new(0, 0, 70, 10))),
        lines_text_concat(&render_todos_expanded(&data, Rect::new(0, 0, 70, 14))),
    ] {
        assert!(text.contains("loop strong"), "group loop missing: {text}");
        assert!(text.contains("loop weak"), "low group loop missing: {text}");
    }
}

#[test]
fn todos_without_goals_render_no_loop_suffix() {
    let data = InfoWidgetData {
        todos: vec![todo_item("a", "do a thing", "pending", None)],
        ..Default::default()
    };
    for text in [
        lines_text_concat(&render_todos_expanded(&data, Rect::new(0, 0, 70, 8))),
        lines_text_concat(&render_todos_expanded(&data, Rect::new(0, 0, 70, 14))),
        lines_text_concat(&render_todos_compact(&data, Rect::new(0, 0, 70, 3))),
    ] {
        assert!(!text.contains("loop "), "unexpected loop suffix: {text}");
    }
}

#[test]
fn loop_suffix_renders_safely_at_tiny_sizes() {
    let data = InfoWidgetData {
        todos: vec![todo_item(
            "a",
            "very long content that will need truncation 汉字 emoji 🚀",
            "in_progress",
            Some("a very long group name that must truncate"),
        )],
        todo_goals: vec![crate::todo::TodoGoal {
            group: Some("a very long group name that must truncate".to_string()),
            closed_feedback_loop: Some(crate::todo::FeedbackLoopState::from_legacy_score(100)),
            ..Default::default()
        }],
        ..Default::default()
    };
    for (w, h) in [(0, 0), (1, 1), (5, 2), (12, 4), (200, 50)] {
        let rect = Rect::new(0, 0, w, h);
        let _ = render_todos_expanded(&data, rect);
        let _ = render_todos_expanded(&data, rect);
        let _ = render_todos_compact(&data, rect);
    }
}

#[test]
fn swarm_plan_gate_items_render_like_normal_items() {
    // Deep-mode critique gates share the plan item shape; only the id differs.
    let mut gate = plan_item("explore-root::gate", "queued");
    gate.content = "Critique the work of 'explore-root' adversarially.".to_string();
    gate.blocked_by = vec!["explore-root".to_string()];
    let items = vec![plan_item("explore-root", "running"), gate];

    let data = InfoWidgetData {
        todos: swarm_plan_todos(&items),
        ..Default::default()
    };
    let text = lines_text(&render_todos_expanded(&data, Rect::new(0, 0, 80, 14)));
    assert!(text.contains("Critique the work"), "{text}");
    assert!(text.contains("(blocked)"), "gate blocked on parent: {text}");
}

#[test]
fn swarm_plan_todos_render_safely_at_extreme_sizes() {
    // Panic-safety sweep: long ids, wide glyphs, huge plans, tiny rects.
    let mut items: Vec<crate::plan::PlanItem> = (0..300)
        .map(|i| {
            let mut item = plan_item(
                &format!("very-long-node-id-{i}::gate::retry::{}", "x".repeat(80)),
                match i % 5 {
                    0 => "running",
                    1 => "completed",
                    2 => "failed",
                    3 => "queued",
                    _ => "blocked",
                },
            );
            item.content = format!("宽字符 emoji 🚀 test {} {}", i, "汉".repeat(40));
            item.blocked_by = vec!["dep".to_string()];
            item
        })
        .collect();
    items.push(plan_item("", ""));

    let data = InfoWidgetData {
        todos: swarm_plan_todos(&items),
        ..Default::default()
    };
    for (w, h) in [(0, 0), (1, 1), (2, 5), (7, 3), (20, 8), (200, 50)] {
        let rect = Rect::new(0, 0, w, h);
        let _ = render_todos_expanded(&data, rect);
        let _ = render_todos_expanded(&data, rect);
        let _ = render_todos_compact(&data, rect);
    }
}

#[test]
fn cost_based_usage_widgets_show_price_and_tokens() {
    let usage = UsageInfo {
        provider: UsageProvider::CostBased,
        total_cost: 0.01234,
        input_tokens: 12_345,
        output_tokens: 678,
        available: true,
        ..Default::default()
    };
    let data = InfoWidgetData {
        usage_info: Some(usage.clone()),
        ..Default::default()
    };

    assert!(data.has_data_for(WidgetKind::UsageLimits));

    let expanded_text = lines_text(&render_usage_compact(&usage, 40, false));
    assert!(expanded_text.contains("$0.0123"));
    assert!(expanded_text.contains("12.3K in + 678 out"));

    let compact_text = lines_text(&render_usage_compact(&usage, 40, false));
    assert!(compact_text.contains("$0.0123"));
    assert!(compact_text.contains("12.3K in + 678 out"));
}

fn node(kind: &str, label: &str, degree: usize) -> GraphNode {
    GraphNode {
        id: format!("{}:{}", kind, label.replace(' ', "_")),
        label: label.to_string(),
        kind: kind.to_string(),
        is_memory: kind != "tag" && kind != "cluster",
        is_active: true,
        confidence: 0.9,
        degree,
    }
}

fn edge(source: usize, target: usize, kind: &str) -> GraphEdge {
    GraphEdge {
        source,
        target,
        kind: kind.to_string(),
    }
}

fn lines_text(lines: &[ratatui::text::Line<'_>]) -> String {
    lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn memory_widget_hides_sidecar_model_when_idle() {
    let info = MemoryInfo {
        total_count: 3,
        project_count: 2,
        global_count: 1,
        sidecar_available: true,
        sidecar_model: Some("openai · gpt-5.3-codex-spark".to_string()),
        ..Default::default()
    };
    let data = InfoWidgetData {
        memory_info: Some(info),
        ..Default::default()
    };

    let text = render_memory_widget(&data, Rect::new(0, 0, 40, 5))
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();

    assert!(text.contains("3 memories"));
    assert!(!text.contains("model:"));
    assert!(!text.contains("gpt-5.3"));
    assert!(text.contains("3 memories"));
}

#[test]
fn memory_widget_renders_current_cycle_activity() {
    let now = Instant::now();
    let mut pipeline = PipelineState::new();
    pipeline.search = StepStatus::Done;
    pipeline.verify = StepStatus::Running;
    pipeline.verify_progress = Some((1, 3));

    let data = InfoWidgetData {
        memory_info: Some(MemoryInfo {
            total_count: 7,
            project_count: 4,
            global_count: 3,
            sidecar_model: Some("openai · gpt-5.3-codex-spark".to_string()),
            activity: Some(MemoryActivity {
                state: MemoryState::SidecarChecking { count: 3 },
                state_since: now - Duration::from_secs(12),
                pipeline: Some(pipeline),
                recent_events: vec![
                    MemoryEvent {
                        kind: MemoryEventKind::MemoryInjected {
                            count: 2,
                            prompt_chars: 318,
                            age_ms: 44,
                            preview: "prefers terse answers".to_string(),
                            items: Vec::new(),
                        },
                        timestamp: now - Duration::from_secs(11),
                        detail: None,
                    },
                    MemoryEvent {
                        kind: MemoryEventKind::EmbeddingComplete {
                            latency_ms: 71,
                            hits: 9,
                        },
                        timestamp: now - Duration::from_secs(12),
                        detail: None,
                    },
                ],
            }),
            graph_nodes: vec![node("fact", "release build", 2), node("tag", "rust", 1)],
            graph_edges: vec![edge(0, 1, "has_tag")],
            ..Default::default()
        }),
        ..Default::default()
    };

    let text = render_memory_widget(&data, Rect::new(0, 0, 40, 8))
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();

    assert!(text.contains("7 memories"));
    assert!(text.contains("find matches"));
    assert!(text.contains("check relevance"));
    assert!(text.contains("1/3"));
    assert!(text.contains("inject context"));
    assert!(text.contains("update memory"));
    assert!(text.contains("now:"));
    assert!(text.contains("checking 3 candidate"));
    assert!(!text.contains("model:"));
    assert!(!text.contains("gpt-5.3"));
    assert!(!text.contains("4 project"));
    assert!(!text.contains("3 global"));
}

#[test]
fn memory_widget_marks_completed_pipeline_even_when_state_is_idle() {
    let now = Instant::now();
    let mut pipeline = PipelineState::new();
    pipeline.search = StepStatus::Done;
    pipeline.verify = StepStatus::Done;
    pipeline.inject = StepStatus::Done;
    pipeline.maintain = StepStatus::Done;

    let data = InfoWidgetData {
        memory_info: Some(MemoryInfo {
            sidecar_model: Some("openai · gpt-5.3-codex-spark".to_string()),
            activity: Some(MemoryActivity {
                state: MemoryState::Idle,
                state_since: now - Duration::from_secs(4),
                pipeline: Some(pipeline),
                recent_events: vec![MemoryEvent {
                    kind: MemoryEventKind::MemoryInjected {
                        count: 1,
                        prompt_chars: 42,
                        age_ms: 12,
                        preview: "prefers terse answers".to_string(),
                        items: Vec::new(),
                    },
                    timestamp: now - Duration::from_secs(3),
                    detail: None,
                }],
            }),
            ..Default::default()
        }),
        ..Default::default()
    };

    let text = render_memory_widget(&data, Rect::new(0, 0, 40, 4))
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();

    assert!(text.contains("done"));
    assert!(text.contains("last:"));
}

#[test]
fn memory_widget_does_not_stay_done_after_idle_settles() {
    let now = Instant::now();
    let mut pipeline = PipelineState::new();
    pipeline.search = StepStatus::Done;
    pipeline.verify = StepStatus::Done;
    pipeline.inject = StepStatus::Done;
    pipeline.maintain = StepStatus::Done;

    let data = InfoWidgetData {
        memory_info: Some(MemoryInfo {
            total_count: 128,
            activity: Some(MemoryActivity {
                state: MemoryState::Idle,
                state_since: now - Duration::from_secs(12),
                pipeline: Some(pipeline),
                recent_events: vec![MemoryEvent {
                    kind: MemoryEventKind::MemoryInjected {
                        count: 1,
                        prompt_chars: 42,
                        age_ms: 12,
                        preview: "prefers terse answers".to_string(),
                        items: Vec::new(),
                    },
                    timestamp: now - Duration::from_secs(11),
                    detail: None,
                }],
            }),
            ..Default::default()
        }),
        ..Default::default()
    };

    let text = render_memory_widget(&data, Rect::new(0, 0, 50, 6))
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();

    assert!(text.contains("128 memories"), "{text}");
    assert!(!text.contains("done"), "{text}");
    assert!(!text.contains("idle"), "{text}");
    assert!(!text.contains("trace:"), "{text}");
}

#[test]
fn memory_widget_never_renders_uppercase_state_badges() {
    let data = InfoWidgetData {
        memory_info: Some(MemoryInfo {
            total_count: 128,
            activity: Some(MemoryActivity {
                state: MemoryState::SidecarChecking { count: 3 },
                state_since: Instant::now(),
                pipeline: None,
                recent_events: Vec::new(),
            }),
            ..Default::default()
        }),
        ..Default::default()
    };

    let text = render_memory_widget(&data, Rect::new(0, 0, 40, 8))
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert!(text.contains("128 memories"), "{text}");
    for badge in [
        "IDLE", "SEARCH", "VERIFY", "READY", "INJECT", "SAVE", "UPDATE", "TOOL", "DONE", "FAILED",
        "DISABLED",
    ] {
        assert!(!text.contains(badge), "unexpected badge {badge}: {text}");
    }
}

#[test]
fn memory_widget_uses_distinct_trace_label_when_idle() {
    let now = Instant::now();
    let mut pipeline = PipelineState::new();
    pipeline.search = StepStatus::Done;
    pipeline.verify = StepStatus::Done;
    pipeline.inject = StepStatus::Done;
    pipeline.maintain = StepStatus::Done;

    let data = InfoWidgetData {
        memory_info: Some(MemoryInfo {
            sidecar_model: Some("openai · gpt-5.3-codex-spark".to_string()),
            activity: Some(MemoryActivity {
                state: MemoryState::Idle,
                state_since: now - Duration::from_secs(4),
                pipeline: Some(pipeline),
                recent_events: vec![MemoryEvent {
                    kind: MemoryEventKind::MemoryInjected {
                        count: 1,
                        prompt_chars: 42,
                        age_ms: 12,
                        preview: "prefers terse answers".to_string(),
                        items: Vec::new(),
                    },
                    timestamp: now - Duration::from_secs(3),
                    detail: None,
                }],
            }),
            ..Default::default()
        }),
        ..Default::default()
    };

    let text = render_memory_widget(&data, Rect::new(0, 0, 60, 8))
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();

    // The pipeline completed and a memory was injected, so the widget shows
    // the "Last: " status line and the recovered-memory summary line instead
    // of the generic "trace:" line.
    assert_eq!(text.matches("last:").count(), 1, "{text}");
    assert!(text.contains("1 memory injected"), "{text}");
}

#[test]
fn memory_compact_does_not_show_model() {
    let lines = render_memory_compact(
        &MemoryInfo {
            sidecar_model: Some("openai · gpt-5.3-codex-spark".to_string()),
            ..Default::default()
        },
        30,
    );

    let text = lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();

    assert!(!text.contains("gpt-5.3"), "{text}");
    assert!(!text.contains("codex-spark"), "{text}");
}

#[test]
fn memory_compact_shows_memory_count_before_status() {
    let lines = render_memory_compact(
        &MemoryInfo {
            total_count: 128,
            activity: Some(MemoryActivity {
                state: MemoryState::Idle,
                state_since: Instant::now() - Duration::from_secs(8),
                pipeline: None,
                recent_events: Vec::new(),
            }),
            ..Default::default()
        },
        30,
    );

    let text = lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();

    assert!(text.contains("128 memories"), "{text}");
    assert!(!text.contains("idle"), "{text}");
    assert!(!text.contains(" · "), "{text}");
    assert!(!text.contains("memory ·"), "{text}");
}

#[test]
fn memory_widget_is_hidden_when_disabled() {
    // When disabled with 0 memories, activity, or sidecar, the widget is hidden.
    let data = InfoWidgetData {
        memory_info: Some(MemoryInfo {
            total_count: 0,
            project_count: 0,
            global_count: 0,
            disabled: true,
            ..Default::default()
        }),
        ..Default::default()
    };

    assert!(render_memory_widget(&data, Rect::new(0, 0, 40, 5)).is_empty());
    assert!(!data.has_data_for(WidgetKind::MemoryActivity));

    // When disabled but has memories, the widget shows "Memory disabled".
    let data_with_mem = InfoWidgetData {
        memory_info: Some(MemoryInfo {
            total_count: 12,
            project_count: 8,
            global_count: 4,
            disabled: true,
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(!render_memory_widget(&data_with_mem, Rect::new(0, 0, 40, 5)).is_empty());
    assert!(data_with_mem.has_data_for(WidgetKind::MemoryActivity));
}

#[test]
fn memory_widget_shows_option_a_steps_without_pipeline_object() {
    let data = InfoWidgetData {
        memory_info: Some(MemoryInfo {
            sidecar_model: Some("openai · gpt-5.3-codex-spark".to_string()),
            activity: Some(MemoryActivity {
                state: MemoryState::SidecarChecking { count: 3 },
                state_since: Instant::now(),
                pipeline: None,
                recent_events: Vec::new(),
            }),
            ..Default::default()
        }),
        ..Default::default()
    };

    let text = render_memory_widget(&data, Rect::new(0, 0, 40, 8))
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();

    assert!(text.contains("find matches"), "{text}");
    assert!(text.contains("check relevance"), "{text}");
    assert!(text.contains("inject context"), "{text}");
    assert!(text.contains("update memory"), "{text}");
    assert!(text.contains("checking 3 candidate"), "{text}");
}

#[test]
fn memory_activity_priority_is_elevated_while_processing() {
    let mut idle_data = InfoWidgetData {
        memory_info: Some(MemoryInfo {
            total_count: 2,
            activity: Some(MemoryActivity {
                state: MemoryState::Idle,
                state_since: Instant::now(),
                pipeline: None,
                recent_events: Vec::new(),
            }),
            ..Default::default()
        }),
        ..Default::default()
    };

    assert_eq!(
        idle_data.effective_priority(WidgetKind::MemoryActivity),
        WidgetKind::MemoryActivity.priority()
    );

    idle_data.memory_info.as_mut().unwrap().activity = Some(MemoryActivity {
        state: MemoryState::Embedding,
        state_since: Instant::now(),
        pipeline: None,
        recent_events: Vec::new(),
    });

    assert_eq!(idle_data.effective_priority(WidgetKind::MemoryActivity), 0);
}

#[test]
fn contextual_subgraph_prefers_memory_hub() {
    let mut nodes = vec![
        node("fact", "core build flow", 6),
        node("preference", "use cargo test", 4),
        node("tag", "rust", 5),
        node("tag", "testing", 3),
        node("fact", "docs in readme", 1),
    ];
    nodes[0].is_active = true;
    nodes[0].confidence = 0.95;

    let info = MemoryInfo {
        total_count: 5,
        graph_nodes: nodes,
        graph_edges: vec![
            edge(0, 1, "relates_to"),
            edge(0, 2, "has_tag"),
            edge(1, 3, "has_tag"),
            edge(4, 2, "has_tag"),
        ],
        ..Default::default()
    };

    let subgraph = super::select_contextual_subgraph(&info, 3, 6).expect("subgraph");
    assert_eq!(subgraph.nodes.len(), 3);
    assert!(
        subgraph
            .nodes
            .iter()
            .any(|n| n.label.contains("core build flow"))
    );
}

#[test]
fn overview_shows_for_any_renderable_content() {
    // The Overview is the single merged widget: it shows whenever the panel
    // renders any content at all. Model-only data must go somewhere, and the
    // standalone ModelInfo margin widget is gone (its content is merged into
    // the Overview panel), so the Overview owns it now.
    let one_section = InfoWidgetData {
        model: Some("gpt-test".to_string()),
        ..Default::default()
    };
    assert!(one_section.has_data_for(WidgetKind::Overview));

    let two_sections = InfoWidgetData {
        model: Some("gpt-test".to_string()),
        queue_mode: Some(true),
        ..Default::default()
    };
    assert!(two_sections.has_data_for(WidgetKind::Overview));
}

#[test]
fn overview_widget_is_placed_when_space_allows() {
    {
        let mut guard = super::get_or_init_state();
        if let Some(state) = guard.as_mut() {
            state.enabled = true;
            state.placements.clear();
            state.anchors.clear();
            state.widget_states.clear();
        }
    }

    let data = InfoWidgetData {
        model: Some("gpt-test".to_string()),
        queue_mode: Some(true),
        status_line_pinned: true,
        ..Default::default()
    };
    let margins = Margins {
        right_widths: vec![40; 20],
        left_widths: Vec::new(),
        centered: false,
        ..Default::default()
    };
    let placements = calculate_placements(Rect::new(0, 0, 80, 20), &margins, &data);
    assert!(
        placements.iter().any(|p| p.kind == WidgetKind::Overview),
        "expected overview widget placement"
    );
}

#[test]
fn workspace_widget_has_high_priority_when_enabled() {
    {
        let mut guard = super::get_or_init_state();
        if let Some(state) = guard.as_mut() {
            state.enabled = true;
            state.placements.clear();
            state.anchors.clear();
            state.widget_states.clear();
        }
    }

    let data = InfoWidgetData {
        workspace_rows: vec![crate::tui::workspace_map::VisibleWorkspaceRow {
            workspace: 0,
            is_current: true,
            focused_index: Some(0),
            sessions: vec![crate::tui::workspace_map::WorkspaceSessionTile::new("fox")],
        }],
        model: Some("gpt-test".to_string()),
        queue_mode: Some(true),
        ..Default::default()
    };

    let available = data.available_widgets();
    assert_eq!(available.first(), Some(&WidgetKind::WorkspaceMap));

    let margins = Margins {
        right_widths: vec![40; 20],
        left_widths: Vec::new(),
        centered: false,
        ..Default::default()
    };
    let placements = calculate_placements(Rect::new(0, 0, 80, 20), &margins, &data);
    assert_eq!(
        placements.first().map(|p| p.kind),
        Some(WidgetKind::WorkspaceMap)
    );
}

#[test]
fn model_info_renders_connection_type() {
    let data = InfoWidgetData {
        model: Some("gpt-5.3-codex".to_string()),
        provider_name: Some("openai".to_string()),
        connection_type: Some("websocket".to_string()),
        auth_method: super::AuthMethod::OpenAIOAuth,
        ..Default::default()
    };
    let lines = super::render_model_info(&data, Rect::new(0, 0, 40, 10));
    let text = lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
    assert!(text.contains("websocket"));
}

#[test]
fn usage_pill_renders_filled_and_empty_segments() {
    let line = super::render_usage_pill(200_000, 1_000_000, 26);
    let text: String = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();

    assert!(text.contains('▰'), "expected filled pill segments: {text}");
    assert!(text.contains('▱'), "expected empty pill segments: {text}");
}

#[test]
fn usage_pill_renders_when_narrow() {
    let line = super::render_usage_pill(200_000, 1_000_000, 10);
    let text: String = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();

    assert!(
        text.contains('▰') || text.contains('▱'),
        "narrow bar should still render pill segments: {text}"
    );
}

#[test]
fn context_usage_line_shows_numeric_label_inside_bar() {
    let line = super::render_context_usage_line("Context", 50_000, 200_000, 40);
    let text: String = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();

    assert!(text.contains("Context"), "expected context label: {text}");
    assert!(
        text.contains("50k/200k"),
        "expected inline token label: {text}"
    );
}

#[test]
fn render_context_compact_prefers_observed_token_usage_for_label() {
    let data = InfoWidgetData {
        context_info: Some(crate::prompt::ContextInfo {
            total_chars: 400_000,
            ..Default::default()
        }),
        context_limit: Some(200_000),
        observed_context_tokens: Some(50_000),
        ..Default::default()
    };

    let lines = super::render_context_compact(&data, Rect::new(0, 0, 40, 1));
    let text: String = lines[0]
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();

    assert!(
        text.contains("50k/200k"),
        "expected observed token count: {text}"
    );
    assert!(
        !text.contains("100k/200k"),
        "should not fall back to char estimate when observed tokens exist: {text}"
    );
}

#[test]
fn render_context_compact_reports_updating_when_snapshot_is_stale() {
    let data = InfoWidgetData {
        context_info_stale: true,
        context_info: Some(crate::prompt::ContextInfo {
            total_chars: 400_000,
            ..Default::default()
        }),
        context_limit: Some(200_000),
        ..Default::default()
    };

    let lines = super::render_context_compact(&data, Rect::new(0, 0, 40, 1));
    let text: String = lines[0]
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();

    assert!(
        text.contains("updating"),
        "expected updating marker: {text}"
    );
    assert!(
        !text.contains("100k/200k"),
        "stale snapshots must not render old usage as current: {text}"
    );
}

fn managed_member(id: &str, status: &str, role: Option<&str>) -> SwarmMemberStatus {
    SwarmMemberStatus {
        session_id: id.to_string(),
        friendly_name: Some(id.to_string()),
        status: status.to_string(),
        detail: None,
        task_label: None,
        role: role.map(str::to_string),
        is_headless: Some(true),
        live_attachments: None,
        status_age_secs: Some(3),
        output_tail: Some("streaming some work".to_string()),
        report_back_to_session_id: Some("parent".to_string()),
        todo_progress: Some((2, 5)),
        todo_items: Vec::new(),
        runtime: crate::protocol::SwarmMemberRuntime::default(),
    }
}

/// With managed members present, the SwarmStatus widget switches to compact
/// mode: agents tally + node progress bar, and has_data_for admits the
/// widget into layout.
#[test]
fn swarm_widget_dock_mode_lists_managed_agents() {
    let data = InfoWidgetData {
        swarm_info: Some(SwarmInfo {
            managed_members: vec![
                managed_member("researcher", "running", Some("coordinator")),
                managed_member("reviewer", "completed", None),
            ],
            plan_progress: Some((3, 2, 7)),
            selected: 0,
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(data.has_data_for(WidgetKind::SwarmStatus));
    // Managing agents bumps the dock's effective priority near the top.
    assert!(data.effective_priority(WidgetKind::SwarmStatus) < WidgetKind::SwarmStatus.priority());

    let lines = super::render_swarm_widget(&data, Rect::new(0, 0, 34, 10));
    let text = lines_text(&lines);
    assert!(text.contains("1/2 agents"), "got: {text}");
    assert!(text.contains("nodes 3/7"), "got: {text}");
    // First two lines: summary line + plan progress bar (low-profile underline cells).
    assert!(
        lines.len() >= 2,
        "compact widget has at least summary + bar"
    );
    let bar: String = lines[1].spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(
        bar.chars().all(|c| c == '▁') && !bar.is_empty(),
        "expected underline bar cells: {bar}"
    );
    // Remaining lines: individual agent status lines (one per managed member).
    assert_eq!(
        lines.len(),
        4,
        "summary + bar + 2 agent lines: {}",
        lines.len()
    );
    assert!(text.contains("researcher"), "agent name visible: {text}");
    assert!(text.contains("reviewer"), "agent name visible: {text}");
    // Height: summary + bar + 2 agents (+ borders).
    let h = calculate_widget_height(WidgetKind::SwarmStatus, &data, 34, 20);
    assert_eq!(h, 6, "compact height = 4 content + 2 border: {h}");
}

/// Without managed members the legacy session-list rendering is preserved and
/// the widget stays out of layout (has_data_for is false).
#[test]
fn swarm_widget_without_managed_agents_stays_hidden_from_layout() {
    let data = InfoWidgetData {
        swarm_info: Some(SwarmInfo {
            session_count: 4,
            session_names: vec!["alpha".to_string()],
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(!data.has_data_for(WidgetKind::SwarmStatus));
    assert_eq!(
        calculate_widget_height(WidgetKind::SwarmStatus, &data, 34, 20),
        0
    );
}

#[test]
fn swarm_widget_renders_member_roles_and_details() {
    let data = InfoWidgetData {
        swarm_info: Some(SwarmInfo {
            session_count: 3,
            client_count: Some(1),
            members: vec![
                SwarmMemberStatus {
                    session_id: "coord-12345678".to_string(),
                    friendly_name: Some("coord".to_string()),
                    status: "running".to_string(),
                    detail: Some("orchestrating patch".to_string()),
                    task_label: None,
                    role: Some("coordinator".to_string()),
                    is_headless: None,
                    live_attachments: None,
                    status_age_secs: None,
                    output_tail: None,
                    report_back_to_session_id: None,
                    todo_progress: None,
                    todo_items: Vec::new(),
                    runtime: crate::protocol::SwarmMemberRuntime::default(),
                },
                SwarmMemberStatus {
                    session_id: "tree-12345678".to_string(),
                    friendly_name: Some("trees".to_string()),
                    status: "ready".to_string(),
                    detail: Some("worktree synced".to_string()),
                    task_label: None,
                    role: Some("agent".to_string()),
                    is_headless: None,
                    live_attachments: None,
                    status_age_secs: None,
                    output_tail: None,
                    report_back_to_session_id: None,
                    todo_progress: None,
                    todo_items: Vec::new(),
                    runtime: crate::protocol::SwarmMemberRuntime::default(),
                },
            ],
            ..Default::default()
        }),
        ..Default::default()
    };

    let text = lines_text(&super::render_swarm_widget(&data, Rect::new(0, 0, 80, 4)));

    assert!(text.contains("3 sessions"), "got: {text}");
    assert!(text.contains("1 client"), "got: {text}");
    assert!(text.contains("★"), "got: {text}");
    assert!(
        text.contains("coord running - orchestrating patch"),
        "got: {text}"
    );
    assert!(
        text.contains("trees ready - worktree synced"),
        "got: {text}"
    );
}

#[test]
fn swarm_widget_handles_empty_swarm_and_zero_area_without_panic() {
    // No swarm info at all: renders nothing.
    let data = InfoWidgetData::default();
    assert!(super::render_swarm_widget(&data, Rect::new(0, 0, 40, 4)).is_empty());

    // Empty swarm (no members, no names, no subagent status): only the stats line.
    let data = InfoWidgetData {
        swarm_info: Some(SwarmInfo::default()),
        ..Default::default()
    };
    let lines = super::render_swarm_widget(&data, Rect::new(0, 0, 40, 4));
    assert_eq!(lines.len(), 1, "expected only the stats line");
    // session_count == 0 and client_count == None: stats line is just the bee icon.
    let text = lines_text(&lines);
    assert!(
        !text.contains("0s"),
        "zero sessions must not render: {text}"
    );

    // Zero-size rect must not panic or underflow.
    let _ = super::render_swarm_widget(&data, Rect::new(0, 0, 0, 0));
    let mut member_data = data.clone();
    member_data.swarm_info.as_mut().unwrap().members = vec![SwarmMemberStatus {
        session_id: "abc".to_string(),
        friendly_name: None,
        status: "running".to_string(),
        detail: None,
        task_label: None,
        role: None,
        is_headless: None,
        live_attachments: None,
        status_age_secs: None,
        output_tail: None,
        report_back_to_session_id: None,
        todo_progress: None,
        todo_items: Vec::new(),
        runtime: crate::protocol::SwarmMemberRuntime::default(),
    }];
    let _ = super::render_swarm_widget(&member_data, Rect::new(0, 0, 0, 0));
    let _ = super::render_swarm_widget(&member_data, Rect::new(0, 0, 3, 1));
}

#[test]
fn swarm_widget_caps_member_rows_for_large_swarms() {
    let members: Vec<SwarmMemberStatus> = (0..500)
        .map(|i| SwarmMemberStatus {
            session_id: format!("session-{i:04}"),
            friendly_name: Some(format!("worker-{i}")),
            status: "running".to_string(),
            detail: Some("very long detail text that should be truncated".repeat(4)),
            task_label: None,
            role: None,
            is_headless: None,
            live_attachments: None,
            status_age_secs: None,
            output_tail: None,
            report_back_to_session_id: None,
            todo_progress: None,
            todo_items: Vec::new(),
            runtime: crate::protocol::SwarmMemberRuntime::default(),
        })
        .collect();
    let data = InfoWidgetData {
        swarm_info: Some(SwarmInfo {
            session_count: 500,
            client_count: Some(3),
            members,
            session_names: (0..500).map(|i| format!("worker-{i}")).collect(),
            ..Default::default()
        }),
        ..Default::default()
    };

    let lines = super::render_swarm_widget(&data, Rect::new(0, 0, 30, 10));
    // Stats line + at most 8 member rows regardless of swarm size.
    assert_eq!(lines.len(), 9, "expected stats line + capped member rows");
    let text = lines_text(&lines);
    assert!(text.contains("500 sessions"), "got: {text}");
    assert!(text.contains("3 clients"), "got: {text}");
}

#[test]
fn background_compact_handles_empty_and_large_task_lists() {
    // running_count == 0: summary is suppressed even if stale task names linger.
    let info = BackgroundInfo {
        running_count: 0,
        running_tasks: vec!["stale".to_string()],
        ..Default::default()
    };
    assert!(super::render_background_compact(&info).is_empty());

    // Large task list: summary + 3 rows + overflow line.
    let info = BackgroundInfo {
        running_count: 200,
        running_tasks: (0..200).map(|i| format!("task-{i}")).collect(),
        progress_detail: Some("42% · working".to_string()),
        ..Default::default()
    };
    let lines = super::render_background_compact(&info);
    assert_eq!(lines.len(), 5, "summary + 3 tasks + overflow");
    let text = lines_text(&lines);
    assert!(text.contains("200 running"), "got: {text}");
    assert!(text.contains("+197 more"), "got: {text}");
}

#[test]
fn background_compact_renders_summary_format() {
    let info = BackgroundInfo {
        running_count: 4,
        running_tasks: vec![
            "selfdev build".to_string(),
            "train.py".to_string(),
            "cargo test".to_string(),
            "download".to_string(),
        ],
        progress_summary: Some("selfdev build".to_string()),
        progress_detail: Some("[#####-------] 42% · Building (parsed)".to_string()),
        memory_agent_active: false,
        memory_agent_turns: 0,
    };

    let compact_text = lines_text(&super::render_background_compact(&info));

    assert!(compact_text.contains("Background"), "got: {compact_text}");
    assert!(compact_text.contains("4"), "got: {compact_text}");
    assert!(!compact_text.contains("mem:"), "got: {compact_text}");
    assert!(
        compact_text.contains("selfdev build"),
        "got: {compact_text}"
    );
    assert!(compact_text.contains("train.py"), "got: {compact_text}");
    assert!(compact_text.contains("cargo test"), "got: {compact_text}");
    assert!(compact_text.contains("+1 more"), "got: {compact_text}");
    assert!(
        compact_text.contains("[#####-------]"),
        "got: {compact_text}"
    );
}

#[test]
fn sticky_placement_clamps_width_to_current_margin() {
    {
        let mut guard = super::get_or_init_state();
        if let Some(state) = guard.as_mut() {
            state.enabled = true;
            state.placements.clear();
            state.anchors.clear();
            state.widget_states.clear();
        }
    }

    let data = InfoWidgetData {
        model: Some("gpt-test".to_string()),
        queue_mode: Some(true),
        ..Default::default()
    };
    let area = Rect::new(0, 0, 100, 10);

    // First frame places a wide widget.
    let first = calculate_placements(
        area,
        &Margins {
            right_widths: vec![30; 10],
            left_widths: Vec::new(),
            centered: false,
            ..Default::default()
        },
        &data,
    );
    assert!(!first.is_empty(), "expected initial placement");
    assert_eq!(first[0].rect.width, 30);

    // Second frame shrinks margin by 4 columns (within sticky tolerance).
    let second_margins = vec![26; 10];
    let second = calculate_placements(
        area,
        &Margins {
            right_widths: second_margins.clone(),
            left_widths: Vec::new(),
            centered: false,
            ..Default::default()
        },
        &data,
    );
    assert!(!second.is_empty(), "expected sticky placement");

    let p = &second[0];
    let row_start = p.rect.y.saturating_sub(area.y) as usize;
    let row_end = row_start + p.rect.height as usize;
    let min_margin = second_margins[row_start..row_end]
        .iter()
        .copied()
        .min()
        .unwrap_or(0);
    assert!(
        p.rect.width <= min_margin,
        "sticky width {} exceeded current margin {}",
        p.rect.width,
        min_margin
    );
}

#[test]
fn placements_never_include_border_only_widgets() {
    {
        let mut guard = super::get_or_init_state();
        if let Some(state) = guard.as_mut() {
            state.enabled = true;
            state.placements.clear();
            state.anchors.clear();
            state.widget_states.clear();
        }
    }

    let data = InfoWidgetData {
        model: Some("gpt-test".to_string()),
        session_count: Some(2),
        context_info: Some(crate::prompt::ContextInfo {
            system_prompt_chars: 24_000,
            total_chars: 40_000,
            ..Default::default()
        }),
        todos: vec![crate::todo::TodoItem {
            group: None,
            content: "ship patch".to_string(),
            status: "in_progress".to_string(),
            priority: "high".to_string(),
            id: "todo-1".to_string(),
            blocked_by: Vec::new(),
            assigned_to: None,
            confidence: None,
            completion_confidence: None,
            confidence_history: Vec::new(),
        }],
        queue_mode: Some(true),
        memory_info: Some(MemoryInfo {
            total_count: 1,
            ..Default::default()
        }),
        swarm_info: Some(SwarmInfo {
            session_count: 2,
            ..Default::default()
        }),
        background_info: Some(BackgroundInfo {
            running_count: 1,
            running_tasks: vec!["bash".to_string()],
            ..Default::default()
        }),
        usage_info: Some(UsageInfo {
            provider: UsageProvider::Anthropic,
            primary_limit_label: Some("5-hour".to_string()),
            five_hour: 0.35,
            secondary_limit_label: Some("Weekly".to_string()),
            seven_day: 0.62,
            available: true,
            ..Default::default()
        }),
        ..Default::default()
    };

    let placements = calculate_placements(
        Rect::new(0, 0, 100, 10),
        &Margins {
            right_widths: vec![40; 10],
            left_widths: Vec::new(),
            centered: false,
            ..Default::default()
        },
        &data,
    );

    assert!(
        placements.iter().all(|p| p.rect.height > 2),
        "found border-only widget placement: {:?}",
        placements
    );
}

/// The compact overview page must render exactly as many lines as
/// `compute_page_layout` reserved for it. A mismatch either clips the last
/// sections (background tasks were the historical victim, since they render
/// last) or leaves blank reserved rows.
#[test]
fn compact_page_height_estimate_matches_rendered_lines() {
    use super::InfoPageKind;

    // No todos/memory so the only candidate page is CompactOnly, and the
    // background section (rendered last) is included.
    let data = InfoWidgetData {
        model: Some("claude-test-1".to_string()),
        provider_name: Some("anthropic".to_string()),
        session_count: Some(2),
        context_info: Some(crate::prompt::ContextInfo {
            system_prompt_chars: 10_000,
            total_chars: 30_000,
            ..Default::default()
        }),
        background_info: Some(BackgroundInfo {
            running_count: 2,
            running_tasks: vec!["bash".to_string(), "task".to_string()],
            ..Default::default()
        }),
        usage_info: Some(UsageInfo {
            provider: UsageProvider::Anthropic,
            primary_limit_label: Some("5-hour".to_string()),
            five_hour: 0.3,
            secondary_limit_label: Some("Weekly".to_string()),
            seven_day: 0.5,
            available: true,
            ..Default::default()
        }),
        cache_hit_info: Some(CacheHitInfo {
            reported_input_tokens: 1_000,
            read_tokens: 800,
            ..Default::default()
        }),
        ..Default::default()
    };

    let inner = Rect::new(0, 0, 38, 30);
    let layout = super::compute_page_layout(&data, inner.width as usize, inner.height);
    assert_eq!(layout.pages.len(), 1, "expected a single compact page");
    assert_eq!(layout.pages[0].kind, InfoPageKind::CompactOnly);

    let lines = super::render_page(InfoPageKind::CompactOnly, &data, inner);
    assert_eq!(
        lines.len() as u16,
        layout.pages[0].height,
        "compact page height estimate must match rendered line count \
         (background section is rendered last and gets clipped on mismatch)"
    );
}

/// Same consistency check for a cost-based (API key) provider, whose usage
/// section renders a single line.
#[test]
fn compact_page_height_matches_for_cost_based_usage() {
    use super::InfoPageKind;

    let data = InfoWidgetData {
        model: Some("gpt-test".to_string()),
        background_info: Some(BackgroundInfo {
            running_count: 1,
            running_tasks: vec!["bash".to_string()],
            ..Default::default()
        }),
        usage_info: Some(UsageInfo {
            provider: UsageProvider::CostBased,
            total_cost: 0.42,
            input_tokens: 10_000,
            output_tokens: 2_000,
            available: true,
            ..Default::default()
        }),
        ..Default::default()
    };

    let inner = Rect::new(0, 0, 38, 30);
    let layout = super::compute_page_layout(&data, inner.width as usize, inner.height);
    assert_eq!(layout.pages.len(), 1);
    assert_eq!(layout.pages[0].kind, InfoPageKind::CompactOnly);

    let lines = super::render_page(InfoPageKind::CompactOnly, &data, inner);
    assert_eq!(lines.len() as u16, layout.pages[0].height);
}

/// Build InfoWidgetData with a single MemoryInjected event carrying the given
/// items. Used by the recovered-memories widget tests.
fn data_with_injected(items: Vec<InjectedMemoryItem>) -> InfoWidgetData {
    let now = Instant::now();
    InfoWidgetData {
        memory_info: Some(MemoryInfo {
            total_count: items.len().max(1),
            activity: Some(MemoryActivity {
                state: MemoryState::Idle,
                state_since: now - Duration::from_secs(2),
                pipeline: None,
                recent_events: vec![MemoryEvent {
                    kind: MemoryEventKind::MemoryInjected {
                        count: items.len().max(1),
                        prompt_chars: 42,
                        age_ms: 12,
                        preview: String::new(),
                        items,
                    },
                    timestamp: now - Duration::from_secs(1),
                    detail: None,
                }],
            }),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[test]
fn recovered_memories_widget_is_empty_when_no_memory_info() {
    let data = InfoWidgetData::default();
    let lines = render_recovered_memories_widget(&data, Rect::new(0, 0, 40, 10));
    assert!(lines.is_empty());
}

#[test]
fn recovered_memories_widget_is_empty_without_injected_events() {
    let now = Instant::now();
    let data = InfoWidgetData {
        memory_info: Some(MemoryInfo {
            activity: Some(MemoryActivity {
                state: MemoryState::Idle,
                state_since: now,
                pipeline: None,
                recent_events: vec![MemoryEvent {
                    kind: MemoryEventKind::EmbeddingComplete {
                        latency_ms: 5,
                        hits: 3,
                    },
                    timestamp: now,
                    detail: None,
                }],
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let lines = render_recovered_memories_widget(&data, Rect::new(0, 0, 40, 10));
    assert!(lines.is_empty());
}

#[test]
fn recovered_memories_widget_lists_items_when_present() {
    let data = data_with_injected(vec![
        InjectedMemoryItem {
            section: "fact".to_string(),
            content: "prefers terse answers".to_string(),
        },
        InjectedMemoryItem {
            section: "preference".to_string(),
            content: "uses worktrees".to_string(),
        },
    ]);
    // Use a wide terminal so the 50% cap still fits each item on one line.
    let lines = render_recovered_memories_widget(&data, Rect::new(0, 0, 200, 10));
    let text = lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();

    assert!(text.contains("2 memories recovered:"), "{text}");
    assert!(text.contains("prefers terse answers"), "{text}");
    assert!(text.contains("uses worktrees"), "{text}");
}

#[test]
fn recovered_memories_widget_shows_summary_when_no_item_details() {
    let data = data_with_injected(Vec::new());
    let lines = render_recovered_memories_widget(&data, Rect::new(0, 0, 44, 10));
    let text = lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
    assert!(text.contains("1 memory injected"), "{text}");
}

#[test]
fn has_recovered_memories_reflects_injected_events() {
    let data = data_with_injected(vec![InjectedMemoryItem {
        section: "fact".to_string(),
        content: "x".to_string(),
    }]);
    assert!(data.has_recovered_memories());

    let empty = InfoWidgetData::default();
    assert!(!empty.has_recovered_memories());
}

#[test]
fn recovered_memories_height_matches_rendered_lines() {
    let items = vec![
        InjectedMemoryItem {
            section: "fact".to_string(),
            content: "one".to_string(),
        },
        InjectedMemoryItem {
            section: "fact".to_string(),
            content: "two".to_string(),
        },
    ];
    let data = data_with_injected(items);
    // Header + 2 items = 3 content lines.
    let rendered = render_recovered_memories_widget(&data, Rect::new(0, 0, 40, 30));
    assert_eq!(rendered.len(), 3);

    // calculate_widget_height should return the same content height (no border).
    let h = calculate_widget_height(WidgetKind::RecoveredMemories, &data, 40, 30);
    assert_eq!(h, 3 + 2, "height must include the 2-row border");
}

/// Visual snapshot test: render a fully-populated Overview panel and print
/// it as a bordered box so we can see the layout. Run with:
///   cargo test -p jcode-tui --lib -- info_widget_tests::visual_full_overview --nocapture
#[test]
fn visual_full_overview() {
    use crate::ambient::AmbientStatus;
    use crate::prompt::ContextInfo;
    use crate::tui::info_widget::{AmbientWidgetData, AuthMethod, CompactionInfo, GitInfo};

    let data = InfoWidgetData {
        // Session
        session_count: Some(3),
        session_name: Some("fix/info_fields".to_string()),
        working_dir: Some(
            "/Users/alejandro/Documents/projects/worktrees/jcode/fix-info-fields".to_string(),
        ),

        // Model + provider + auth
        model: Some("claude-sonnet-4-20250514".to_string()),
        provider_name: Some("anthropic".to_string()),
        reasoning_effort: Some("high".to_string()),
        service_tier: Some("priority".to_string()),
        auth_method: AuthMethod::AnthropicOAuth,
        upstream_provider: None,
        connection_type: Some("websocket/persistent-fresh".to_string()),

        // Context
        context_info: Some(ContextInfo {
            system_prompt_chars: 8_500,
            user_messages_chars: 12_000,
            assistant_messages_chars: 15_000,
            tool_calls_chars: 6_000,
            tool_results_chars: 20_000,
            total_chars: 61_500,
            user_messages_count: 8,
            assistant_messages_count: 7,
            tool_calls_count: 15,
            tool_results_count: 15,
            ..Default::default()
        }),
        context_limit: Some(200_000),

        // Usage (Anthropic OAuth: subscription bars with reset times)
        usage_info: Some(UsageInfo {
            provider: UsageProvider::Anthropic,
            primary_limit_label: Some("5h".to_string()),
            five_hour: 0.42,
            five_hour_resets_at: Some("2026-07-26T13:00:00Z".to_string()),
            secondary_limit_label: Some("weekly".to_string()),
            seven_day: 0.18,
            seven_day_resets_at: Some("2026-07-28T00:00:00Z".to_string()),
            total_cost: 0.0,
            input_tokens: 45_200,
            output_tokens: 8_100,
            output_tps: Some(42.5),
            available: true,
            ..Default::default()
        }),
        avg_tokens_per_second: Some(38.2),

        // KV cache
        cache_hit_info: Some(CacheHitInfo {
            reported_input_tokens: 45_200,
            read_tokens: 30_000,
            creation_tokens: 5_000,
            optimal_input_tokens: 40_000,
            ..Default::default()
        }),

        // Compaction
        compaction_info: Some(CompactionInfo {
            is_compacting: false,
            compacted_messages: 12,
            active_messages: 8,
            summary_chars: 3_500,
            mode: "auto".to_string(),
        }),

        // Background
        background_info: Some(BackgroundInfo {
            running_count: 2,
            running_tasks: vec!["bash: build".to_string(), "task: tests".to_string()],
            progress_summary: Some("Building... 80%".to_string()),
            ..Default::default()
        }),

        // Swarm
        swarm_info: Some(SwarmInfo {
            session_count: 3,
            session_names: vec![
                "coordinator".to_string(),
                "worker-1".to_string(),
                "worker-2".to_string(),
            ],
            plan_progress: Some((2, 1, 5)),
            ..Default::default()
        }),

        // MCP servers
        mcp_servers: vec![("filesystem".to_string(), 8), ("github".to_string(), 12)],

        // Skills
        available_skills: vec![
            "/codebase-memory".to_string(),
            "/bitbucket".to_string(),
            "/code-review-excellence".to_string(),
        ],

        // Git
        git_info: Some(GitInfo {
            branch: "fix/info_fields".to_string(),
            modified: 3,
            staged: 1,
            untracked: 2,
            ahead: 2,
            behind: 0,
            dirty_files: vec![
                "src/main.rs".to_string(),
                "src/info_widget.rs".to_string(),
                "src/model.rs".to_string(),
            ],
        }),

        // Memory
        memory_info: Some(MemoryInfo {
            total_count: 47,
            project_count: 32,
            global_count: 15,
            activity: Some(MemoryActivity {
                state: MemoryState::Idle,
                state_since: Instant::now(),
                pipeline: None,
                recent_events: vec![MemoryEvent {
                    kind: MemoryEventKind::MemoryInjected {
                        count: 3,
                        prompt_chars: 318,
                        age_ms: 44,
                        preview: "prefers worktrees".to_string(),
                        items: vec![
                            InjectedMemoryItem {
                                section: "preference".to_string(),
                                content: "User prefers worktrees for branch work".to_string(),
                            },
                            InjectedMemoryItem {
                                section: "correction".to_string(),
                                content: "rustfmt uses edition 2024 (let-chains)".to_string(),
                            },
                            InjectedMemoryItem {
                                section: "fact".to_string(),
                                content: "Build binary from scratch, no selfdev profile"
                                    .to_string(),
                            },
                        ],
                    },
                    timestamp: Instant::now(),
                    detail: None,
                }],
            }),
            ..Default::default()
        }),

        // Ambient
        ambient_info: Some(AmbientWidgetData {
            show_widget: true,
            status: AmbientStatus::Idle,
            queue_count: 2,
            next_queue_preview: Some("Review PR #42".to_string()),
            reminder_count: 1,
            next_reminder_preview: Some("Deploy at 5pm".to_string()),
            last_run_ago: Some("12m ago".to_string()),
            last_summary: None,
            next_wake: Some("in 45m".to_string()),
            next_reminder_wake: None,
            budget_percent: None,
        }),

        // Todos
        todos: vec![
            crate::todo::TodoItem {
                group: None,
                id: "t1".to_string(),
                content: "Add auth indicator to Overview".to_string(),
                status: "completed".to_string(),
                priority: "high".to_string(),
                confidence: Some(crate::todo::ConfidenceState::from_legacy_score(95)),
                completion_confidence: Some(crate::todo::ConfidenceState::from_legacy_score(100)),
                confidence_history: Vec::new(),
                blocked_by: Vec::new(),
                assigned_to: None,
            },
            crate::todo::TodoItem {
                group: None,
                id: "t2".to_string(),
                content: "Remove dead widget code".to_string(),
                status: "in_progress".to_string(),
                priority: "medium".to_string(),
                confidence: Some(crate::todo::ConfidenceState::from_legacy_score(70)),
                completion_confidence: None,
                confidence_history: Vec::new(),
                blocked_by: Vec::new(),
                assigned_to: None,
            },
        ],

        ..Default::default()
    };

    // Render the Overview content using render_sections (same function
    // the real Overview widget calls).
    let inner = Rect::new(0, 0, 44, 40);
    let lines = super::render_sections(&data, inner, None);

    // Build a visual bordered box from the rendered lines.
    let w = inner.width as usize;
    let mut output = String::new();

    // Top border
    output.push_str(&format!("┌{}┐\n", "─".repeat(w)));

    for line in &lines {
        // Flatten spans into a single string
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        let visible_len = text.chars().count();
        let padded = if visible_len < w {
            format!("{}{}", text, " ".repeat(w - visible_len))
        } else {
            text.chars().take(w).collect::<String>()
        };
        output.push_str(&format!("│{}│\n", padded));
    }

    // Fill remaining rows with empty lines (up to inner.height)
    let content_lines = lines.len();
    for _ in content_lines..inner.height as usize {
        output.push_str(&format!("│{}│\n", " ".repeat(w)));
    }

    // Bottom border
    output.push_str(&format!("└{}┘", "─".repeat(w)));

    println!("\n{output}\n");

    // Basic assertions to verify key content is present
    let all_text: String = lines
        .iter()
        .flat_map(|l| l.spans.iter())
        .map(|s| s.content.as_ref())
        .collect::<String>();

    assert!(
        all_text.contains("anthropic"),
        "provider name should be present"
    );
    assert!(
        all_text.contains("Sonnet 4"),
        "model name should be present"
    );
    assert!(
        all_text.contains("🔐"),
        "auth method icon should be present"
    );
    assert!(
        all_text.contains("OAuth"),
        "auth method label should be present"
    );
    assert!(
        all_text.contains("websocket"),
        "connection type should be present"
    );
    assert!(
        all_text.contains("Anthropic limits"),
        "usage provider label should be present"
    );
    assert!(
        all_text.contains("5h"),
        "primary usage bar label should be present"
    );
    assert!(
        all_text.contains("58% left"),
        "primary usage percent should be present (shows % left)"
    );
    assert!(
        all_text.contains("weekly"),
        "secondary usage bar label should be present"
    );
    assert!(
        all_text.contains("82% left"),
        "secondary usage percent should be present (shows % left)"
    );
    assert!(all_text.contains("⌀38.2"), "avg t/s should be present");
    assert!(all_text.contains("🧠"), "memory icon should be present");
    assert!(
        all_text.contains("47 memories"),
        "memory count should be present"
    );
    assert!(all_text.contains("🐝"), "swarm icon should be present");
}

// ---------------------------------------------------------------------------
// Unit tests for pure format/render helpers
// ---------------------------------------------------------------------------

/// Single helper to flatten a `Line` into its plain-text content.
fn line_text(line: &ratatui::text::Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

#[test]
fn format_age_handles_zero_and_sub_two_seconds() {
    assert_eq!(format_age(Duration::from_secs(0)), "now");
    assert_eq!(format_age(Duration::from_secs(1)), "now");
    assert_eq!(format_age(Duration::from_millis(1500)), "now");
}

#[test]
fn format_age_formats_seconds() {
    assert_eq!(format_age(Duration::from_secs(5)), "5s");
    assert_eq!(format_age(Duration::from_secs(59)), "59s");
}

#[test]
fn format_age_formats_minutes() {
    assert_eq!(format_age(Duration::from_secs(60)), "1m");
    assert_eq!(format_age(Duration::from_secs(120)), "2m");
    assert_eq!(format_age(Duration::from_secs(3599)), "59m");
}

#[test]
fn format_age_formats_hours() {
    assert_eq!(format_age(Duration::from_secs(3600)), "1h");
    assert_eq!(format_age(Duration::from_secs(7200)), "2h");
    assert_eq!(format_age(Duration::from_secs(86_400)), "24h");
}

#[test]
fn format_memory_count_singular_and_plural() {
    assert_eq!(format_memory_count(0), "0 memories");
    assert_eq!(format_memory_count(1), "1 memory");
    assert_eq!(format_memory_count(100), "100 memories");
    assert_eq!(format_memory_count(1000), "1000 memories");
}

#[test]
fn wrap_text_short_text_returns_single_line() {
    let lines = wrap_text("hello", 10);
    assert_eq!(lines, vec!["hello".to_string()]);
}

#[test]
fn wrap_text_long_text_wraps_into_multiple_lines() {
    let lines = wrap_text("hello world foo bar", 10);
    assert_eq!(
        lines,
        vec![
            "hello".to_string(),
            "world foo".to_string(),
            "bar".to_string()
        ]
    );
}

#[test]
fn wrap_text_empty_text_returns_single_empty_line() {
    let lines = wrap_text("", 10);
    assert_eq!(lines, vec!["".to_string()]);
}

#[test]
fn wrap_text_text_exactly_at_max_chars_stays_one_line() {
    let lines = wrap_text("hello", 5);
    assert_eq!(lines, vec!["hello".to_string()]);
}

#[test]
fn wrap_text_max_chars_zero_returns_original_text() {
    let lines = wrap_text("hello world", 0);
    assert_eq!(lines, vec!["hello world".to_string()]);
}

#[test]
fn wrap_text_handles_newlines_as_paragraphs() {
    let lines = wrap_text("hello\nworld", 10);
    assert_eq!(lines, vec!["hello".to_string(), "world".to_string()]);
}

#[test]
fn wrap_text_long_word_exceeds_max_chars_on_its_own_line() {
    let lines = wrap_text("short verylongword", 10);
    assert_eq!(lines, vec!["short".to_string(), "verylongword".to_string()]);
}

#[test]
fn dashed_separator_fills_width_with_dashes() {
    let line = dashed_separator(10);
    let text = line_text(&line);
    assert_eq!(text, "- - - - - ");
}

#[test]
fn dashed_separator_zero_width_is_empty() {
    let line = dashed_separator(0);
    let text = line_text(&line);
    assert_eq!(text, "");
}

#[test]
fn dashed_separator_odd_width() {
    let line = dashed_separator(5);
    let text = line_text(&line);
    assert_eq!(text, "- - -");
}

#[test]
fn render_mcp_servers_line_single_line_when_fits() {
    let servers = vec![("filesystem".to_string(), 8)];
    let lines = render_mcp_servers_line(&servers, 40);
    assert_eq!(lines.len(), 1);
    let text = line_text(&lines[0]);
    assert!(text.contains("mcp: filesystem (8 tools)"), "{text}");
}

#[test]
fn render_mcp_servers_line_shows_ellipsis_for_zero_tool_count() {
    let servers = vec![("fs".to_string(), 0)];
    let lines = render_mcp_servers_line(&servers, 40);
    let text = line_text(&lines[0]);
    assert!(text.contains("fs (...)"), "{text}");
}

#[test]
fn render_mcp_servers_line_multiple_servers_on_one_line_when_wide() {
    let servers = vec![("fs".to_string(), 8), ("gh".to_string(), 12)];
    let lines = render_mcp_servers_line(&servers, 80);
    assert_eq!(lines.len(), 1);
    let text = line_text(&lines[0]);
    assert!(text.contains("fs (8 tools)"), "{text}");
    assert!(text.contains("gh (12 tools)"), "{text}");
}

#[test]
fn render_mcp_servers_line_compact_format_when_full_too_long() {
    let servers = vec![("filesystem".to_string(), 8), ("github".to_string(), 12)];
    // Width that's too narrow for "filesystem (8 tools), github (12 tools)"
    // but wide enough for the compact "filesystem(8) github(12)".
    let lines = render_mcp_servers_line(&servers, 40);
    assert_eq!(lines.len(), 1);
    let text = line_text(&lines[0]);
    assert!(text.contains("filesystem(8)"), "{text}");
    assert!(text.contains("github(12)"), "{text}");
}

#[test]
fn render_mcp_servers_line_multi_line_when_narrow() {
    let servers = vec![("filesystem".to_string(), 8), ("github".to_string(), 12)];
    // Width that's too narrow for the compact single line (29 chars) but
    // wide enough to avoid truncating the individual server entries.
    let lines = render_mcp_servers_line(&servers, 25);
    // Header line + one per server.
    assert_eq!(lines.len(), 3);
    let header = line_text(&lines[0]);
    assert!(header.contains("mcp: 2 servers"), "{header}");
    let all_text = lines_text(&lines);
    assert!(all_text.contains("filesystem"), "{all_text}");
    assert!(all_text.contains("github"), "{all_text}");
}

#[test]
fn render_skills_line_counts_loaded_skills() {
    let skills: Vec<String> = Vec::new();
    let lines = render_skills_line(&skills, 40);
    assert_eq!(lines.len(), 1);
    let text = line_text(&lines[0]);
    assert!(text.contains("skills: 0 loaded"), "{text}");

    let skills = vec!["git".to_string()];
    let lines = render_skills_line(&skills, 40);
    let text = line_text(&lines[0]);
    assert!(text.contains("skills: 1 loaded"), "{text}");

    let skills = vec!["git".to_string(), "review".to_string(), "test".to_string()];
    let lines = render_skills_line(&skills, 40);
    let text = line_text(&lines[0]);
    assert!(text.contains("skills: 3 loaded"), "{text}");
}

#[test]
fn render_cost_tokens_line_shows_cost_and_tokens_for_cost_based() {
    let data = InfoWidgetData {
        usage_info: Some(UsageInfo {
            provider: UsageProvider::CostBased,
            total_cost: 0.01234,
            input_tokens: 12_345,
            output_tokens: 678,
            available: true,
            ..Default::default()
        }),
        ..Default::default()
    };
    let line = render_cost_tokens_line(&data, Rect::new(0, 0, 40, 1));
    let text = line_text(&line);
    assert!(text.contains("$0.0123"), "{text}");
    assert!(text.contains("12k"), "{text}");
    assert!(text.contains("678"), "{text}");
    assert!(text.contains("t/s"), "{text}");
}

#[test]
fn render_cost_tokens_line_shows_na_when_no_usage_info() {
    let data = InfoWidgetData::default();
    let line = render_cost_tokens_line(&data, Rect::new(0, 0, 40, 1));
    let text = line_text(&line);
    assert!(text.contains("$NA"), "{text}");
    assert!(text.contains("⌀0 t/s"), "{text}");
}

#[test]
fn render_cost_tokens_line_shows_avg_tps_when_available() {
    let data = InfoWidgetData {
        usage_info: Some(UsageInfo {
            provider: UsageProvider::CostBased,
            total_cost: 0.05,
            input_tokens: 5_000,
            output_tokens: 200,
            available: true,
            ..Default::default()
        }),
        avg_tokens_per_second: Some(38.2),
        ..Default::default()
    };
    let line = render_cost_tokens_line(&data, Rect::new(0, 0, 40, 1));
    let text = line_text(&line);
    assert!(text.contains("⌀38.2 t/s"), "{text}");
}

#[test]
fn render_cost_tokens_line_shows_zero_tps_when_avg_is_none() {
    let data = InfoWidgetData {
        usage_info: Some(UsageInfo {
            provider: UsageProvider::CostBased,
            total_cost: 0.05,
            input_tokens: 5_000,
            output_tokens: 200,
            available: true,
            ..Default::default()
        }),
        ..Default::default()
    };
    let line = render_cost_tokens_line(&data, Rect::new(0, 0, 40, 1));
    let text = line_text(&line);
    assert!(text.contains("⌀0 t/s"), "{text}");
}

#[test]
fn has_model_supplementary_info_true_when_compaction_mode_set() {
    let data = InfoWidgetData {
        native_compaction_mode: Some("auto".to_string()),
        ..Default::default()
    };
    assert!(data.has_model_supplementary_info());
}

#[test]
fn has_model_supplementary_info_false_without_compaction_mode() {
    let data = InfoWidgetData::default();
    assert!(!data.has_model_supplementary_info());
}

#[test]
fn memory_active_summary_returns_none_for_idle() {
    assert_eq!(memory_active_summary(&MemoryState::Idle), None);
}

#[test]
fn memory_active_summary_returns_labels_for_active_states() {
    assert_eq!(
        memory_active_summary(&MemoryState::Embedding),
        Some("embedding".to_string())
    );
    assert_eq!(
        memory_active_summary(&MemoryState::SidecarChecking { count: 3 }),
        Some("checking".to_string())
    );
    assert_eq!(
        memory_active_summary(&MemoryState::FoundRelevant { count: 2 }),
        Some("found".to_string())
    );
    assert_eq!(
        memory_active_summary(&MemoryState::Extracting {
            reason: "conversation end".to_string()
        }),
        Some("extracting".to_string())
    );
    assert_eq!(
        memory_active_summary(&MemoryState::Maintaining {
            phase: "pruning".to_string()
        }),
        Some("maintaining".to_string())
    );
    assert_eq!(
        memory_active_summary(&MemoryState::ToolAction {
            action: "read_file".to_string(),
            detail: "src/main.rs".to_string()
        }),
        Some("tool".to_string())
    );
}

#[test]
fn render_kv_cache_summary_line_empty_when_no_cache_telemetry() {
    // All-zero telemetry yields no hit ratio, so nothing renders.
    let cache = CacheHitInfo::default();
    assert!(render_kv_cache_summary_line(&cache).is_empty());
}

#[test]
fn render_kv_cache_summary_line_renders_session_ratio_only_without_last_stats() {
    // Subset accounting (no creation, read <= input): denominator = input.
    let cache = CacheHitInfo {
        reported_input_tokens: 1_000,
        read_tokens: 800,
        ..Default::default()
    };
    let lines = render_kv_cache_summary_line(&cache);
    // No last-request stats -> a single header line.
    assert_eq!(lines.len(), 1, "{lines:?}");
    let text = lines_text(&lines);
    assert!(text.contains("KV cache:"), "{text}");
    assert!(text.contains("session"), "{text}");
    // 800 / 1000 = 80%
    assert!(text.contains("80%"), "{text}");
}

#[test]
fn render_compaction_compact_shows_status_mode_and_detail_stats() {
    let info = super::CompactionInfo {
        is_compacting: true,
        compacted_messages: 12,
        active_messages: 8,
        summary_chars: 3_500,
        mode: "auto".to_string(),
    };
    let lines = super::render_compaction_compact(&info, 40);
    assert_eq!(lines.len(), 2);
    let l0: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
    let l1: String = lines[1].spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(l0.contains("compacting"), "status line: {l0}");
    assert!(l0.contains("auto"), "mode on line 1: {l0}");
    assert!(l1.contains("12 old"), "compacted count: {l1}");
    assert!(l1.contains("8 active"), "active count: {l1}");
    assert!(l1.contains("tok"), "summary tokens: {l1}");
}

#[test]
fn render_compaction_compact_says_compacted_when_idle() {
    let info = super::CompactionInfo {
        is_compacting: false,
        compacted_messages: 5,
        active_messages: 3,
        summary_chars: 1_000,
        mode: "manual".to_string(),
    };
    let lines = super::render_compaction_compact(&info, 60);
    let l0: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(l0.contains("compacted"), "{l0}");
    assert!(l0.contains("manual"), "{l0}");
}

#[test]
fn render_compaction_compact_renders_safely_at_tiny_width() {
    let info = super::CompactionInfo {
        is_compacting: false,
        compacted_messages: 1_000_000,
        active_messages: 2_000_000,
        summary_chars: 500_000,
        mode: "auto".to_string(),
    };
    // Must not panic on a width smaller than the detail text.
    let lines = super::render_compaction_compact(&info, 8);
    assert_eq!(lines.len(), 2);
}

#[test]
fn render_model_info_supplementary_shows_native_compaction_with_threshold() {
    let data = InfoWidgetData {
        native_compaction_mode: Some("on".to_string()),
        native_compaction_threshold_tokens: Some(200_000),
        ..Default::default()
    };
    let lines = super::render_model_info_supplementary(&data, Rect::new(0, 0, 44, 10));
    assert_eq!(lines.len(), 1);
    let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(text.contains("native"), "{text}");
    assert!(text.contains("on"), "{text}");
    assert!(text.contains("200k"), "{text}");
}

#[test]
fn render_model_info_supplementary_omits_threshold_when_unset() {
    let data = InfoWidgetData {
        native_compaction_mode: Some("off".to_string()),
        ..Default::default()
    };
    let lines = super::render_model_info_supplementary(&data, Rect::new(0, 0, 44, 10));
    assert_eq!(lines.len(), 1);
    let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(text.contains("native off"), "{text}");
    assert!(!text.contains("@"), "no threshold suffix: {text}");
}

#[test]
fn render_model_info_supplementary_empty_without_compaction_mode() {
    // Service tier and connection type live in the status line / main model
    // widget, so the supplementary view is empty when no native compaction.
    let data = InfoWidgetData {
        model: Some("claude-sonnet-4".to_string()),
        service_tier: Some("priority".to_string()),
        connection_type: Some("websocket".to_string()),
        ..Default::default()
    };
    let lines = super::render_model_info_supplementary(&data, Rect::new(0, 0, 44, 10));
    assert!(
        lines.is_empty(),
        "expected no supplementary lines: {lines:?}"
    );
}

#[test]
fn calculate_fixed_overview_placement_anchors_top_right() {
    let data = InfoWidgetData {
        model: Some("gpt-test".to_string()),
        queue_mode: Some(true),
        ..Default::default()
    };
    let area = Rect::new(0, 0, 80, 20);
    let placements = super::calculate_fixed_overview_placement(area, &data);
    assert_eq!(placements.len(), 1);
    let p = &placements[0];
    assert_eq!(p.kind, WidgetKind::Overview);
    assert_eq!(p.side, super::Side::Right);
    // Width caps at FIXED_WIDTH (44) for an 80-col terminal.
    assert_eq!(p.rect.width, 44, "width: {}", p.rect.width);
    assert_eq!(
        p.rect.x, 36,
        "x = area.x + area.width - width: {}",
        p.rect.x
    );
    assert_eq!(p.rect.y, 0);
    assert!(
        p.rect.height > 0 && p.rect.height <= 20,
        "content fits within area: {}",
        p.rect.height
    );
}

#[test]
fn overview_panel_engaged_reflects_overview_placement() {
    super::clear_widget_placements_for_tests();
    // No overview placed yet.
    assert!(!super::overview_panel_engaged());

    // Drive the pinned status-line path which places a fixed Overview widget.
    let data = InfoWidgetData {
        model: Some("gpt-test".to_string()),
        queue_mode: Some(true),
        status_line_pinned: true,
        ..Default::default()
    };
    let margins = Margins {
        right_widths: vec![40; 20],
        left_widths: Vec::new(),
        centered: false,
        ..Default::default()
    };
    let placements = calculate_placements(Rect::new(0, 0, 80, 20), &margins, &data);
    assert!(
        placements.iter().any(|p| p.kind == WidgetKind::Overview),
        "expected overview placement"
    );
    assert!(super::overview_panel_engaged());

    // Leave global state clean for subsequent tests.
    super::clear_widget_placements_for_tests();
}

#[test]
fn render_widget_content_dispatches_swarm_to_swarm_widget() {
    let data = InfoWidgetData {
        swarm_info: Some(SwarmInfo {
            managed_members: vec![managed_member("researcher", "running", None)],
            ..Default::default()
        }),
        ..Default::default()
    };
    let lines =
        super::render_widget_content(WidgetKind::SwarmStatus, &data, Rect::new(0, 0, 34, 10));
    let text = lines_text(&lines);
    assert!(!lines.is_empty(), "swarm content should render");
    assert!(text.contains("researcher"), "agent visible: {text}");
}

#[test]
fn render_widget_content_returns_empty_for_merged_overview_kinds() {
    let data = InfoWidgetData {
        model: Some("gpt-test".to_string()),
        cache_hit_info: Some(CacheHitInfo {
            reported_input_tokens: 1_000,
            read_tokens: 800,
            ..Default::default()
        }),
        ..Default::default()
    };
    // These kinds are merged into the Overview panel, so standalone rendering
    // intentionally yields no content.
    for kind in [
        WidgetKind::Overview,
        WidgetKind::Todos,
        WidgetKind::KvCache,
        WidgetKind::Compaction,
    ] {
        assert!(
            super::render_widget_content(kind, &data, Rect::new(0, 0, 40, 10)).is_empty(),
            "{kind:?} should be empty (merged into Overview)"
        );
    }
}
