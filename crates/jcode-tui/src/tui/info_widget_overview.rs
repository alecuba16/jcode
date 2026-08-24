use super::info_widget::{
    AuthMethod, InfoWidgetData, MemoryEventKind, UsageProvider, is_traceworthy_memory_event,
};

pub(crate) const MAX_TODO_LINES: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InfoPageKind {
    CompactOnly,
    TodosExpanded,
    MemoryExpanded,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct InfoPage {
    pub kind: InfoPageKind,
    pub height: u16,
}

#[derive(Clone, Debug)]
pub(crate) struct PageLayout {
    pub pages: Vec<InfoPage>,
    pub max_page_height: u16,
    pub show_dots: bool,
}

pub(crate) fn compute_page_layout(
    data: &InfoWidgetData,
    _inner_width: usize,
    inner_height: u16,
) -> PageLayout {
    let compact_height = compact_overview_height(data);
    if compact_height == 0 {
        return PageLayout {
            pages: Vec::new(),
            max_page_height: 0,
            show_dots: false,
        };
    }

    let mut pages: Vec<InfoPage> = Vec::with_capacity(2);
    let todos_compact = compact_todos_height(data);

    let todos_expanded = expanded_todos_height(data);
    if todos_expanded > 0 {
        let page = InfoPage {
            kind: InfoPageKind::TodosExpanded,
            height: compact_height - todos_compact + todos_expanded,
        };
        if page.height <= inner_height {
            pages.push(page);
        }
    }

    let memory_compact = compact_memory_height(data);
    let memory_expanded = expanded_memory_height(data);
    if memory_expanded > 0 {
        let page = InfoPage {
            kind: InfoPageKind::MemoryExpanded,
            height: compact_height - memory_compact + memory_expanded,
        };
        if page.height <= inner_height {
            pages.push(page);
        }
    }

    if pages.is_empty() {
        if compact_height <= inner_height {
            pages.push(InfoPage {
                kind: InfoPageKind::CompactOnly,
                height: compact_height,
            });
        } else {
            return PageLayout {
                pages,
                max_page_height: 0,
                show_dots: false,
            };
        }
    }

    let mut show_dots = false;
    if pages.len() > 1 {
        let filtered_len = pages
            .iter()
            .filter(|page| page.height < inner_height)
            .count();
        if filtered_len > 1 {
            pages.retain(|page| page.height < inner_height);
            show_dots = true;
        } else if filtered_len == 1 {
            pages.retain(|page| page.height < inner_height);
        }
    }

    let max_page_height = pages
        .iter()
        .map(|page| page.height + u16::from(show_dots))
        .max()
        .unwrap_or(0);

    PageLayout {
        pages,
        max_page_height,
        show_dots,
    }
}

fn compact_context_height(data: &InfoWidgetData) -> u16 {
    if data.status_line_active {
        // Only the "updating..." stale indicator is shown.
        return u16::from(data.context_info_stale);
    }
    if let Some(info) = &data.context_info
        && info.total_chars > 0
    {
        return 1;
    }
    0
}

fn compact_todos_height(data: &InfoWidgetData) -> u16 {
    if data.todos.is_empty() { 0 } else { 2 }
}

fn compact_memory_height(data: &InfoWidgetData) -> u16 {
    // Show 1 line: "0 memories" when empty (but not disabled),
    // "Memory disabled" when off, or expanded view when has data.
    if data.memory_info.is_none() {
        return 0;
    }
    let mut h = 1u16;

    // Add recovered memories lines (rendered inline after memory count).
    // This is only a layout estimate, so keep it allocation-free and avoid
    // truncating, formatting, and wrapping every recovered memory on each pass.
    if let Some(activity) = data
        .memory_info
        .as_ref()
        .and_then(|info| info.activity.as_ref())
    {
        let mut item_count: usize = 0;
        let mut total_injected: usize = 0;
        for event in &activity.recent_events {
            if let MemoryEventKind::MemoryInjected {
                count,
                items: ev_items,
                ..
            } = &event.kind
                && *count > 0
            {
                total_injected = total_injected.saturating_add(*count);
                item_count = item_count.saturating_add(ev_items.len());
            }
        }
        if total_injected > 0 {
            // One summary/header line, plus one estimated line per item.
            let recovered_lines = 1usize.saturating_add(item_count);
            h = h.saturating_add(u16::try_from(recovered_lines).unwrap_or(u16::MAX));
        }
    }
    h
}

fn compact_model_height(data: &InfoWidgetData) -> u16 {
    if data.status_line_active {
        // Count only supplementary lines (service tier, compaction).
        // Session info is now in line 1 of render_sections.
        if !data.has_model_supplementary_info() {
            return 0;
        }
        let mut lines = 0u16;
        if data
            .service_tier
            .as_deref()
            .map(|s| !s.trim().is_empty() && s != "off" && s != "default")
            .unwrap_or(false)
        {
            lines += 1;
        }
        if data.native_compaction_mode.is_some() {
            lines += 1;
        }
        return lines;
    }
    if data.model.is_some() {
        // Line 1: provider (when present)
        // Line 2: (effort) + model + [tier] + compaction
        let mut lines = 1u16;
        let has_provider = data
            .provider_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .is_some();
        if has_provider {
            lines += 1;
        }
        // Auth method renders on a separate line when present.
        if data.auth_method != AuthMethod::Unknown {
            lines += 1;
        }
        // Session info is now in line 1 of render_sections, not in model info.
        lines
    } else {
        0
    }
}

fn compact_background_height(data: &InfoWidgetData) -> u16 {
    if let Some(info) = &data.background_info
        && info.running_count > 0
    {
        let task_lines = info.running_tasks.len().min(3) as u16;
        let overflow_line = u16::from(info.running_tasks.len() > 3);
        return 1 + task_lines + overflow_line;
    }
    0
}

fn compact_usage_height(data: &InfoWidgetData) -> u16 {
    if let Some(info) = &data.usage_info
        && info.available
        && !matches!(
            info.provider,
            UsageProvider::CostBased | UsageProvider::Copilot
        )
    {
        // Subscription-style providers render an optional provider label plus
        // whichever primary, secondary, and Spark windows are actually present.
        let label = info.provider.label();
        let label_line = u16::from(!label.is_empty());
        let primary_line = u16::from(info.primary_limit_label.is_some());
        let secondary_line = u16::from(info.secondary_limit_label.is_some());
        let spark_line = u16::from(info.spark.is_some());
        return label_line + primary_line + secondary_line + spark_line;
    }
    0
}

fn compact_kv_cache_height(data: &InfoWidgetData) -> u16 {
    // KV cache is never in the status line, always render when present.
    // Base line (yield + session) = 1, plus 1 if last request stats exist.
    if let Some(cache) = &data.cache_hit_info {
        let has_last =
            cache.last_read_tokens.is_some() || cache.last_reported_input_tokens.is_some();
        1 + u16::from(has_last)
    } else {
        0
    }
}

fn compact_compaction_height(data: &InfoWidgetData) -> u16 {
    if data.compaction_info.is_some() { 2 } else { 0 }
}

#[allow(dead_code)] // Replaced by compact_path_git_height
fn compact_git_height(data: &InfoWidgetData) -> u16 {
    if let Some(info) = &data.git_info
        && info.is_interesting()
    {
        return 1;
    }
    0
}

fn compact_mcp_height(data: &InfoWidgetData) -> u16 {
    if data.mcp_servers.is_empty() {
        return 0;
    }
    // Count-based estimate only. Avoid building display strings and joining
    // them during layout, which can run multiple times in a render cycle.
    if data.mcp_servers.len() <= 2 {
        return 1;
    }
    1 + data.mcp_servers.len() as u16
}

fn compact_skills_height(data: &InfoWidgetData) -> u16 {
    if data.available_skills.is_empty() {
        return 0;
    }
    1
}

fn compact_swarm_height(data: &InfoWidgetData) -> u16 {
    if let Some(info) = &data.swarm_info {
        // Stats line + subagent status or member lines (approximate).
        let extra = if !info.managed_members.is_empty() {
            1 // dock compact lines
        } else if !info.members.is_empty() {
            info.members.len().min(3) as u16
        } else if info.subagent_status.is_some() {
            1
        } else {
            0
        };
        1 + extra
    } else {
        // "0 sessions" line when no swarm active.
        1
    }
}

fn compact_ambient_height(data: &InfoWidgetData) -> u16 {
    if let Some(info) = &data.ambient_info
        && info.show_widget
    {
        // Status line + optional next run line.
        2
    } else {
        0
    }
}

fn compact_tps_height(data: &InfoWidgetData) -> u16 {
    if let Some(tps) = data.tokens_per_second
        && tps.is_finite()
        && tps > 0.1
    {
        1
    } else {
        0
    }
}

fn compact_overview_height(data: &InfoWidgetData) -> u16 {
    compact_session_height(data)
        + compact_path_git_height(data)
        + compact_model_height(data)
        + compact_context_height(data)
        + compact_cost_line_height(data)
        + compact_tps_height(data)
        + expanded_todos_height(data)
        + compact_memory_height(data)
        + compact_background_height(data)
        + compact_usage_height(data)
        + compact_kv_cache_height(data)
        + compact_compaction_height(data)
        + compact_mcp_height(data)
        + compact_skills_height(data)
        + compact_swarm_height(data)
        + compact_ambient_height(data)
        + compact_separator_height(data)
}

/// Height for dashed separator lines between sections.
/// Separators after: path/git (before provider), usage limits, compaction, KV cache, skills.
fn compact_separator_height(data: &InfoWidgetData) -> u16 {
    let mut count = 0u16;
    // Before provider/model section (after path/git line)
    if data.model.is_some() {
        count += 1;
    }
    // After usage limits
    if let Some(info) = &data.usage_info
        && info.available
        && !matches!(
            info.provider,
            UsageProvider::CostBased | UsageProvider::Copilot
        )
    {
        count += 1;
    }
    // After compaction
    if data.compaction_info.is_some() {
        count += 1;
    }
    // After KV cache
    if data.cache_hit_info.is_some() {
        count += 1;
    }
    // After skills (before background)
    if !data.available_skills.is_empty() {
        count += 1;
    }
    count
}

/// Height for line 1: session name + count (always rendered when either exists).
fn compact_session_height(data: &InfoWidgetData) -> u16 {
    let has_session = data.session_count.is_some()
        || data
            .session_name
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty());
    u16::from(has_session)
}

/// Height for line 2: merged working dir + git branch (1 line when either exists).
fn compact_path_git_height(data: &InfoWidgetData) -> u16 {
    let has_dir = data
        .working_dir
        .as_deref()
        .map(|d| !d.trim().is_empty())
        .unwrap_or(false);
    let has_git = data
        .git_info
        .as_ref()
        .map(|g| !g.branch.is_empty())
        .unwrap_or(false);
    u16::from(has_dir || has_git)
}

/// Height for line 5: cost + tokens + avg t/s (always 1 line).
/// For OAuth the first line of render_usage_compact is shown inline.
fn compact_cost_line_height(data: &InfoWidgetData) -> u16 {
    let _ = data;
    // Always 1 line regardless of provider type.
    1
}

fn expanded_todos_height(data: &InfoWidgetData) -> u16 {
    if data.todos.is_empty() {
        return 0;
    }

    let available_lines = MAX_TODO_LINES.saturating_sub(1);
    let todo_lines = data.todos.len().min(available_lines);
    let mut height = 1 + u16::try_from(todo_lines).unwrap_or(u16::MAX);
    if data.todos.len() > available_lines {
        height += 1;
    }
    height
}

fn expanded_memory_height(data: &InfoWidgetData) -> u16 {
    if let Some(info) = &data.memory_info
        && info.should_render()
    {
        let mut height = 1u16;
        if info.should_show_activity() {
            height += 1 + 4;
            if let Some(activity) = &info.activity
                && activity
                    .recent_events
                    .iter()
                    .any(is_traceworthy_memory_event)
            {
                height += 1;
            }
        }
        return height;
    }
    0
}

#[cfg(test)]
mod tests {
    use super::{InfoPageKind, compute_page_layout};
    use crate::todo::TodoItem;
    use crate::tui::info_widget::{InfoWidgetData, MemoryInfo};
    use std::collections::HashMap;

    use super::{
        compact_ambient_height, compact_compaction_height, compact_cost_line_height,
        compact_mcp_height, compact_path_git_height, compact_separator_height,
        compact_session_height, compact_skills_height, compact_swarm_height, compact_tps_height,
    };
    use crate::ambient::AmbientStatus;
    use crate::protocol::SwarmMemberStatus;
    use crate::tui::info_widget::{
        AmbientWidgetData, CacheHitInfo, CompactionInfo, GitInfo, SwarmInfo, UsageInfo,
        UsageProvider,
    };

    fn mk_member(id: &str, status: &str) -> SwarmMemberStatus {
        SwarmMemberStatus {
            session_id: id.to_string(),
            friendly_name: Some(id.to_string()),
            status: status.to_string(),
            detail: None,
            task_label: None,
            role: None,
            is_headless: Some(true),
            live_attachments: None,
            status_age_secs: None,
            output_tail: None,
            report_back_to_session_id: None,
            todo_progress: None,
            todo_items: Vec::new(),
            runtime: crate::protocol::SwarmMemberRuntime::default(),
        }
    }

    #[test]
    fn compute_page_layout_falls_back_to_compact_page() {
        let data = InfoWidgetData {
            model: Some("gpt-test".to_string()),
            queue_mode: Some(true),
            ..Default::default()
        };

        let layout = compute_page_layout(&data, 40, 8);

        assert_eq!(layout.pages.len(), 1);
        assert_eq!(layout.pages[0].kind, InfoPageKind::CompactOnly);
        assert!(!layout.show_dots);
    }

    #[test]
    fn compute_page_layout_keeps_multiple_expanded_pages_when_height_allows() {
        let data = InfoWidgetData {
            todos: vec![TodoItem {
                group: None,
                content: "ship refactor".to_string(),
                status: "pending".to_string(),
                priority: "high".to_string(),
                id: "todo-1".to_string(),
                blocked_by: Vec::new(),
                assigned_to: None,
                confidence: None,
                completion_confidence: None,
                confidence_history: Vec::new(),
            }],
            memory_info: Some(MemoryInfo {
                total_count: 3,
                project_count: 2,
                global_count: 1,
                by_category: HashMap::from([("fact".to_string(), 3usize)]),
                sidecar_model: Some("openai · gpt-5.3-codex-spark".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let layout = compute_page_layout(&data, 40, 8);

        assert!(layout.pages.len() >= 2);
        assert!(layout.show_dots);
        assert!(
            layout
                .pages
                .iter()
                .any(|page| page.kind == InfoPageKind::TodosExpanded)
        );
        assert!(
            layout
                .pages
                .iter()
                .any(|page| page.kind == InfoPageKind::MemoryExpanded)
        );
    }

    #[test]
    fn compact_compaction_height_reports_two_lines_when_present() {
        assert_eq!(compact_compaction_height(&InfoWidgetData::default()), 0);
        let data = InfoWidgetData {
            compaction_info: Some(CompactionInfo {
                is_compacting: true,
                compacted_messages: 1,
                active_messages: 2,
                summary_chars: 10,
                mode: "auto".to_string(),
            }),
            ..Default::default()
        };
        assert_eq!(compact_compaction_height(&data), 2);
    }

    #[test]
    fn compact_mcp_height_collapses_short_lists_and_expands_long_ones() {
        assert_eq!(compact_mcp_height(&InfoWidgetData::default()), 0);
        let one = InfoWidgetData {
            mcp_servers: vec![("fs".to_string(), 5)],
            ..Default::default()
        };
        assert_eq!(
            compact_mcp_height(&one),
            1,
            "single short server fits one line"
        );
        // Many long-named servers overflow both the full and compact 38-char
        // budgets, so each server gets its own line below the header.
        let many = InfoWidgetData {
            mcp_servers: (0..5).map(|i| (format!("abcdefghij{i}"), 9)).collect(),
            ..Default::default()
        };
        assert_eq!(compact_mcp_height(&many), 1 + 5);
    }

    #[test]
    fn compact_skills_height_is_one_when_skills_present() {
        assert_eq!(compact_skills_height(&InfoWidgetData::default()), 0);
        let data = InfoWidgetData {
            available_skills: vec!["/bitbucket".to_string()],
            ..Default::default()
        };
        assert_eq!(compact_skills_height(&data), 1);
    }

    #[test]
    fn compact_swarm_height_always_renders_and_grows_with_members() {
        // Even with no swarm_info, the "0 sessions" summary line still renders.
        assert_eq!(compact_swarm_height(&InfoWidgetData::default()), 1);
        // An empty swarm_info renders the summary line only.
        assert_eq!(
            compact_swarm_height(&InfoWidgetData {
                swarm_info: Some(SwarmInfo::default()),
                ..Default::default()
            }),
            1
        );
        // A live subagent status adds one detail line.
        assert_eq!(
            compact_swarm_height(&InfoWidgetData {
                swarm_info: Some(SwarmInfo {
                    subagent_status: Some("running grep".to_string()),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            2
        );
        // Managed members use the single-line dock compact rendering.
        assert_eq!(
            compact_swarm_height(&InfoWidgetData {
                swarm_info: Some(SwarmInfo {
                    managed_members: vec![mk_member("agent-1", "running")],
                    ..Default::default()
                }),
                ..Default::default()
            }),
            2
        );
        // Legacy members render one line each (capped at 3); two here.
        assert_eq!(
            compact_swarm_height(&InfoWidgetData {
                swarm_info: Some(SwarmInfo {
                    members: vec![mk_member("a", "ready"), mk_member("b", "running")],
                    ..Default::default()
                }),
                ..Default::default()
            }),
            3
        );
    }

    #[test]
    fn compact_ambient_height_only_when_widget_shown() {
        assert_eq!(compact_ambient_height(&InfoWidgetData::default()), 0);
        let mk_ambient = |show_widget: bool| InfoWidgetData {
            ambient_info: Some(AmbientWidgetData {
                show_widget,
                status: AmbientStatus::Idle,
                queue_count: 0,
                next_queue_preview: None,
                reminder_count: 0,
                next_reminder_preview: None,
                last_run_ago: None,
                last_summary: None,
                next_wake: None,
                next_reminder_wake: None,
                budget_percent: None,
            }),
            ..Default::default()
        };
        assert_eq!(compact_ambient_height(&mk_ambient(false)), 0);
        assert_eq!(compact_ambient_height(&mk_ambient(true)), 2);
    }

    #[test]
    fn compact_tps_height_requires_finite_positive_throughput() {
        assert_eq!(compact_tps_height(&InfoWidgetData::default()), 0);
        assert_eq!(
            compact_tps_height(&InfoWidgetData {
                tokens_per_second: Some(0.05),
                ..Default::default()
            }),
            0,
            "below the 0.1 threshold"
        );
        assert_eq!(
            compact_tps_height(&InfoWidgetData {
                tokens_per_second: Some(f32::NAN),
                ..Default::default()
            }),
            0,
            "NaN is not finite"
        );
        assert_eq!(
            compact_tps_height(&InfoWidgetData {
                tokens_per_second: Some(1.5),
                ..Default::default()
            }),
            1
        );
    }

    #[test]
    fn compact_separator_height_counts_section_dividers() {
        assert_eq!(compact_separator_height(&InfoWidgetData::default()), 0);
        // A model alone draws one separator before the provider section.
        assert_eq!(
            compact_separator_height(&InfoWidgetData {
                model: Some("gpt-test".to_string()),
                ..Default::default()
            }),
            1
        );
        // Cost-based usage does NOT add a separator (excluded from the count).
        assert_eq!(
            compact_separator_height(&InfoWidgetData {
                model: Some("gpt-test".to_string()),
                usage_info: Some(UsageInfo {
                    provider: UsageProvider::CostBased,
                    available: true,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            1
        );
        // Full set: model + OAuth usage + compaction + kv cache + skills = 5.
        let full = InfoWidgetData {
            model: Some("gpt-test".to_string()),
            usage_info: Some(UsageInfo {
                provider: UsageProvider::Anthropic,
                available: true,
                ..Default::default()
            }),
            compaction_info: Some(CompactionInfo {
                is_compacting: false,
                compacted_messages: 0,
                active_messages: 0,
                summary_chars: 0,
                mode: "auto".to_string(),
            }),
            cache_hit_info: Some(CacheHitInfo::default()),
            available_skills: vec!["/bitbucket".to_string()],
            ..Default::default()
        };
        assert_eq!(compact_separator_height(&full), 5);
    }

    #[test]
    fn compact_session_height_requires_non_blank_session_info() {
        assert_eq!(compact_session_height(&InfoWidgetData::default()), 0);
        assert_eq!(
            compact_session_height(&InfoWidgetData {
                session_count: Some(3),
                ..Default::default()
            }),
            1
        );
        assert_eq!(
            compact_session_height(&InfoWidgetData {
                session_name: Some("fix/info_fields".to_string()),
                ..Default::default()
            }),
            1
        );
        // Whitespace-only session name does not count.
        assert_eq!(
            compact_session_height(&InfoWidgetData {
                session_name: Some("   ".to_string()),
                ..Default::default()
            }),
            0
        );
    }

    #[test]
    fn compact_path_git_height_requires_dir_or_branch() {
        assert_eq!(compact_path_git_height(&InfoWidgetData::default()), 0);
        assert_eq!(
            compact_path_git_height(&InfoWidgetData {
                working_dir: Some("/repo".to_string()),
                ..Default::default()
            }),
            1
        );
        assert_eq!(
            compact_path_git_height(&InfoWidgetData {
                git_info: Some(GitInfo {
                    branch: "main".to_string(),
                    modified: 0,
                    staged: 0,
                    untracked: 0,
                    ahead: 0,
                    behind: 0,
                    dirty_files: Vec::new(),
                }),
                ..Default::default()
            }),
            1
        );
        // Whitespace-only working dir does not count.
        assert_eq!(
            compact_path_git_height(&InfoWidgetData {
                working_dir: Some("  ".to_string()),
                ..Default::default()
            }),
            0
        );
    }

    #[test]
    fn compact_cost_line_height_is_always_one() {
        // The cost line is always rendered regardless of provider type.
        assert_eq!(compact_cost_line_height(&InfoWidgetData::default()), 1);
        let with_usage = InfoWidgetData {
            usage_info: Some(UsageInfo {
                provider: UsageProvider::CostBased,
                total_cost: 0.42,
                available: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(compact_cost_line_height(&with_usage), 1);
    }
}
