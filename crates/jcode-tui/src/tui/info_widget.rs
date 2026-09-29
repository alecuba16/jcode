#![cfg_attr(test, allow(clippy::items_after_test_module))]

//!
//! Supports multiple widget types with priority ordering and side preferences.
//! In centered mode, widgets can appear on both left and right margins.
//! In left-aligned mode, widgets only appear on the right margin.

use super::color_support::rgb;
#[path = "info_widget_commits.rs"]
mod commits;
#[path = "info_widget_frame.rs"]
pub(crate) mod frame;

/// A dashed horizontal separator line that fills the panel width.
fn dashed_separator(width: u16) -> Line<'static> {
    let w = width as usize;
    let dash = "- "
        .repeat(w.div_ceil(2))
        .chars()
        .take(w)
        .collect::<String>();
    Line::from(Span::styled(dash, Style::default().fg(rgb(60, 60, 70))))
}
#[path = "info_widget_git.rs"]
mod git;
#[path = "info_widget_graph.rs"]
mod graph;
#[path = "info_widget_memory_render.rs"]
mod memory_render;
#[path = "info_widget_memory_utils.rs"]
mod memory_utils;
#[path = "info_widget_model.rs"]
pub(crate) mod model;
#[path = "info_widget_swarm_background.rs"]
mod swarm_background;
#[path = "info_widget_swarm_gallery.rs"]
pub(crate) mod swarm_gallery;
#[path = "info_widget_text.rs"]
pub(crate) mod text;
#[path = "info_widget_tips.rs"]
mod tips;
#[path = "info_widget_todos.rs"]
mod todos_render;
#[path = "info_widget_usage.rs"]
mod usage_render;
use super::info_widget_overview::{InfoPageKind, MAX_TODO_LINES, PageLayout, compute_page_layout};
use super::workspace_map::VisibleWorkspaceRow;
use crate::ambient::AmbientStatus;
pub use crate::memory_types::{
    InjectedMemoryItem, MemoryActivity, MemoryEvent, MemoryEventKind, MemoryState, PipelineState,
    StepResult, StepStatus,
};
use crate::prompt::ContextInfo;
use crate::protocol::SwarmMemberStatus;
use crate::provider::DEFAULT_CONTEXT_LIMIT;
use crate::todo::TodoItem;
#[cfg(test)]
use memory_render::{format_age, render_memory_compact, wrap_text};
use memory_render::{render_memory_widget, render_recovered_memories_widget};
use ratatui::{
    prelude::*,
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};
use std::collections::HashMap;
#[cfg(test)]
use std::collections::HashSet;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use unicode_width::UnicodeWidthStr;

use commits::{commits_has_data, render_commits_widget};
use frame::Framed;
use git::{
    changes_has_data, changes_height, changes_legend, render_changes_framed, render_git_widget,
};
pub(crate) use git::{edited_paths_from_tool_call, resolve_edited_path};
pub use graph::{GraphEdge, GraphNode, build_graph_topology, graph_node_score};
pub(crate) use memory_utils::is_traceworthy_memory_event;
#[cfg(test)]
pub(crate) use memory_utils::memory_active_summary;
use memory_utils::{memory_last_trace_summary, memory_state_detail};
use model::{
    model_info_supplementary_height, render_model_info, render_model_info_supplementary,
    render_model_widget, runtime_height,
};
use swarm_background::{render_background_compact, render_background_widget, render_swarm_widget};
use text::{truncate_chars, truncate_smart, truncate_with_ellipsis};
pub(crate) use tips::occasional_status_tip;
use tips::render_tips_widget;
#[cfg(test)]
use todos_render::render_todos_compact;
use todos_render::render_todos_expanded;
use todos_render::render_todos_widget;
pub(crate) use todos_render::swarm_plan_todos;
#[cfg(test)]
use usage_render::render_usage_pill;
use usage_render::render_usage_widget;
use usage_render::{render_context_usage_line, render_usage_compact};

/// Overview rows used by the Changes section, mirroring [`render_sections`].
pub(super) fn changes_section_height(data: &InfoWidgetData) -> u16 {
    data.git_info.as_ref().map(changes_height).unwrap_or(0)
}

/// Overview rows used by the Runtime section, mirroring [`render_sections`].
pub(super) fn model_info_height(data: &InfoWidgetData) -> u16 {
    runtime_height(data) + model_info_supplementary_height(data)
}

/// Types of info widgets that can be displayed
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WidgetKind {
    /// Combined overview to reduce scattered widgets
    Overview,
    /// Niri-style workspace map preview
    WorkspaceMap,
    /// Todo list with progress
    Todos,
    /// Token/context usage bar
    ContextUsage,
    /// Memory sidecar activity
    MemoryActivity,
    /// Recovered memories list, used only by non-pinned fallback layouts.
    /// Fixed Overview renders recovered memories inline.
    RecoveredMemories,
    /// Subagents/sessions status
    SwarmStatus,
    /// Background work indicator
    BackgroundTasks,
    /// Conversation context compaction status
    Compaction,
    /// Subscription quota bars
    UsageLimits,
    /// Session-level KV cache hit ratio
    KvCache,
    /// Current model name
    ModelInfo,
    /// Mermaid diagrams
    Diagrams,
    /// Ambient mode status
    AmbientMode,
    /// Rotating tips/shortcuts
    Tips,
    /// Git status
    GitStatus,
    /// Commits: recent history on the current branch
    Commits,
}

impl WidgetKind {
    /// Priority for display (lower = higher priority)
    pub fn priority(self) -> u8 {
        match self {
            WidgetKind::Diagrams => 0, // Highest priority - user explicitly wants to see it
            WidgetKind::WorkspaceMap => 1,
            WidgetKind::Overview => 2,
            WidgetKind::MemoryActivity => 3, // Right below Overview so recalls appear under main info
            WidgetKind::RecoveredMemories => 3, // Attached to Overview, same priority bucket
            WidgetKind::Todos => 4,
            WidgetKind::ContextUsage => 5,
            WidgetKind::UsageLimits => 6, // Bumped up - important when near limits
            WidgetKind::KvCache => 7,
            WidgetKind::ModelInfo => 8,
            WidgetKind::Compaction => 9,
            WidgetKind::BackgroundTasks => 10,
            WidgetKind::GitStatus => 11,
            WidgetKind::Commits => 12,
            WidgetKind::SwarmStatus => 12, // Session list - lower priority
            WidgetKind::AmbientMode => 13, // Scheduled agent - lower priority
            WidgetKind::Tips => 14,        // Did you know - lowest
        }
    }

    /// Preferred side for this widget
    pub fn preferred_side(self) -> Side {
        match self {
            WidgetKind::Diagrams => Side::Right, // Diagrams on right
            WidgetKind::WorkspaceMap => Side::Right,
            WidgetKind::Overview => Side::Right,
            WidgetKind::Todos => Side::Right,
            WidgetKind::ContextUsage => Side::Right,
            WidgetKind::MemoryActivity => Side::Right,
            WidgetKind::RecoveredMemories => Side::Right, // Attached under Overview
            WidgetKind::SwarmStatus => Side::Left,
            WidgetKind::Compaction => Side::Left,
            WidgetKind::BackgroundTasks => Side::Left,
            WidgetKind::AmbientMode => Side::Left,
            WidgetKind::UsageLimits => Side::Left,
            WidgetKind::KvCache => Side::Left,
            WidgetKind::ModelInfo => Side::Left,
            WidgetKind::Tips => Side::Left,
            WidgetKind::GitStatus => Side::Left,
            WidgetKind::Commits => Side::Left,
        }
    }

    /// Minimum height needed for this widget
    pub fn min_height(self) -> u16 {
        match self {
            WidgetKind::Diagrams => 10, // Diagrams need more space
            WidgetKind::WorkspaceMap => 1,
            WidgetKind::Overview => 8,
            WidgetKind::Todos => 3,
            WidgetKind::ContextUsage => 1, // Can be just "updating..." line
            WidgetKind::MemoryActivity => 3,
            WidgetKind::RecoveredMemories => 2, // Header line + at least one item
            WidgetKind::SwarmStatus => 3,
            WidgetKind::Compaction => 3,
            WidgetKind::BackgroundTasks => 2,
            WidgetKind::AmbientMode => 3,
            WidgetKind::UsageLimits => 3,
            WidgetKind::KvCache => 3,
            WidgetKind::ModelInfo => 1, // Can be just session/tier line
            WidgetKind::Tips => 3,
            WidgetKind::GitStatus => 1,
            WidgetKind::Commits => 1,
        }
    }

    /// All widget kinds in priority order
    pub fn all_by_priority() -> &'static [WidgetKind] {
        &[
            WidgetKind::Diagrams,
            WidgetKind::WorkspaceMap,
            WidgetKind::Overview,
            WidgetKind::MemoryActivity,
            WidgetKind::RecoveredMemories,
            WidgetKind::Todos,
            WidgetKind::ContextUsage,
            WidgetKind::UsageLimits,
            WidgetKind::KvCache,
            WidgetKind::ModelInfo,
            WidgetKind::Compaction,
            WidgetKind::BackgroundTasks,
            WidgetKind::GitStatus,
            WidgetKind::Commits,
            WidgetKind::SwarmStatus,
            WidgetKind::AmbientMode,
            WidgetKind::Tips,
        ]
    }

    pub fn as_str(self) -> &'static str {
        match self {
            WidgetKind::Diagrams => "diagrams",
            WidgetKind::WorkspaceMap => "workspace",
            WidgetKind::Overview => "overview",
            WidgetKind::Todos => "todos",
            WidgetKind::ContextUsage => "context",
            WidgetKind::MemoryActivity => "memory",
            WidgetKind::RecoveredMemories => "recalled",
            WidgetKind::SwarmStatus => "swarm",
            WidgetKind::BackgroundTasks => "background",
            WidgetKind::Compaction => "compaction",
            WidgetKind::AmbientMode => "ambient",
            WidgetKind::UsageLimits => "usage",
            WidgetKind::KvCache => "kv-cache",
            WidgetKind::ModelInfo => "model",
            WidgetKind::Tips => "tips",
            WidgetKind::GitStatus => "git",
            WidgetKind::Commits => "commits",
        }
    }
}

/// Which side of the screen a widget is on
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub fn as_str(self) -> &'static str {
        match self {
            Side::Left => "left",
            Side::Right => "right",
        }
    }
}

pub(crate) fn is_overview_mergeable(kind: WidgetKind) -> bool {
    matches!(
        kind,
        WidgetKind::Todos
            | WidgetKind::ContextUsage
            | WidgetKind::SwarmStatus
            | WidgetKind::BackgroundTasks
            | WidgetKind::Compaction
            | WidgetKind::ModelInfo
            | WidgetKind::UsageLimits
            | WidgetKind::KvCache
            | WidgetKind::GitStatus
    )
}

/// A placed widget with its location and type
#[derive(Debug, Clone)]
pub struct WidgetPlacement {
    pub kind: WidgetKind,
    pub rect: Rect,
    pub side: Side,
    pub(crate) overview_layout: Option<PageLayout>,
}

pub use super::info_widget_layout::Margins;

/// Swarm/subagent status for the info widget
#[derive(Debug, Default, Clone)]
pub struct SwarmInfo {
    /// Number of sessions in the same swarm (same working directory)
    pub session_count: usize,
    /// Current subagent status (from Task tool execution)
    pub subagent_status: Option<String>,
    /// Number of connected clients (server mode)
    pub client_count: Option<usize>,
    /// List of session names in the swarm
    pub session_names: Vec<String>,
    /// Swarm member lifecycle status updates
    pub members: Vec<SwarmMemberStatus>,
    /// Agents this session manages (spawn-subtree filtered), shown in the
    /// swarm dock widget. Empty = no dock.
    pub managed_members: Vec<SwarmMemberStatus>,
    /// Selected agent index in the dock (display order), mirrors the inline
    /// swarm panel selection so both surfaces agree.
    pub selected: usize,
    /// Whether the swarm panel/dock has keyboard focus.
    pub focused: bool,
    /// Swarm plan progress (completed, running, total), when a plan is active.
    pub plan_progress: Option<(u32, u32, u32)>,
    /// Spinner frame for animating active agents' status glyphs.
    pub spinner_frame: usize,
}

/// Background task status for the info widget
#[derive(Debug, Default, Clone)]
pub struct BackgroundInfo {
    /// Number of running background tasks
    pub running_count: usize,
    /// Names of running tasks (e.g., "bash", "task")
    pub running_tasks: Vec<String>,
    /// Compact summary of the most recent task progress
    pub progress_summary: Option<String>,
    /// Detailed display for the most recent task progress
    pub progress_detail: Option<String>,
    /// Memory agent status
    pub memory_agent_active: bool,
    /// Memory agent turn count
    pub memory_agent_turns: usize,
}

/// Which provider the usage info is for
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UsageProvider {
    #[default]
    None,
    /// Anthropic/Claude OAuth (shows subscription usage)
    Anthropic,
    /// OpenAI/Codex OAuth (shows subscription usage)
    OpenAI,
    /// OpenRouter/API-key providers (shows token costs)
    CostBased,
    /// GitHub Copilot (shows session token counts, no cost)
    Copilot,
}

impl UsageProvider {
    pub fn label(&self) -> &'static str {
        match self {
            UsageProvider::None => "",
            UsageProvider::Anthropic => "Anthropic",
            UsageProvider::OpenAI => "OpenAI",
            UsageProvider::CostBased => "",
            UsageProvider::Copilot => "Copilot",
        }
    }
}

/// Authentication method used to access the model
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuthMethod {
    #[default]
    Unknown,
    /// Generic API key auth for API-backed providers without a provider-specific auth widget variant
    ApiKey,
    /// Anthropic OAuth (Claude Code CLI style)
    AnthropicOAuth,
    /// Anthropic API key
    AnthropicApiKey,
    /// OpenAI OAuth (Codex style)
    OpenAIOAuth,
    /// OpenAI API key
    OpenAIApiKey,
    /// OpenRouter API key
    OpenRouterApiKey,
    /// OpenCode API key
    OpenCodeApiKey,
    /// GitHub Copilot OAuth
    CopilotOAuth,
    /// Google Gemini OAuth
    GeminiOAuth,
}

/// Subscription usage info for the info widget
#[derive(Debug, Default, Clone)]
pub struct UsageInfo {
    /// Which provider this usage is for
    pub provider: UsageProvider,
    /// Primary subscription window label. OpenAI reports this dynamically.
    pub primary_limit_label: Option<String>,
    /// Primary window utilization (0.0-1.0) - for OAuth providers
    pub five_hour: f32,
    /// Primary reset timestamp (RFC3339), if known
    pub five_hour_resets_at: Option<String>,
    /// Secondary subscription window label, when one exists.
    pub secondary_limit_label: Option<String>,
    /// Secondary window utilization (0.0-1.0) - for OAuth providers
    pub seven_day: f32,
    /// Secondary reset timestamp (RFC3339), if known
    pub seven_day_resets_at: Option<String>,
    /// Codex Spark window utilization (0.0-1.0), if available
    pub spark: Option<f32>,
    /// Codex Spark reset timestamp (RFC3339), if known
    pub spark_resets_at: Option<String>,
    /// Total cost in USD - for API-key providers (OpenRouter, direct API key)
    pub total_cost: f32,
    /// Input tokens used - for cost calculation
    pub input_tokens: u64,
    /// Output tokens used - for cost calculation
    pub output_tokens: u64,
    /// Cache read tokens (from cache, cheaper) - for API-key providers
    pub cache_read_tokens: Option<u64>,
    /// Cache write tokens (creating cache, more expensive) - for API-key providers
    pub cache_write_tokens: Option<u64>,
    /// Output tokens per second (live streaming)
    pub output_tps: Option<f32>,
    /// Whether data was successfully fetched / available to show
    pub available: bool,
}

/// Session-level KV cache telemetry for providers that report cache usage.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CacheHitInfo {
    /// Sum of per-request full prompt sizes, never inferred from aggregate counters.
    pub prompt_tokens: Option<u64>,
    pub last_prompt_tokens: Option<u64>,
    /// Input tokens from completed API requests that included explicit cache telemetry.
    pub reported_input_tokens: u64,
    /// Tokens read from provider KV/prefix cache across this session.
    pub read_tokens: u64,
    /// Tokens written/created in provider cache across this session, when reported.
    pub creation_tokens: u64,
    /// Approximate reusable prefix tokens expected to be cache-readable.
    pub optimal_input_tokens: u64,
    /// Input tokens from the latest completed request with cache telemetry.
    pub last_reported_input_tokens: Option<u64>,
    /// Cached input tokens read on the latest completed request with cache telemetry.
    pub last_read_tokens: Option<u64>,
    /// Tokens written/created in provider cache on the latest completed request.
    pub last_creation_tokens: Option<u64>,
    /// Approximate reusable prefix tokens expected on the latest completed request.
    pub last_optimal_input_tokens: Option<u64>,
    /// Recent attributed misses with estimated cacheable tokens not read.
    pub miss_attributions: Vec<CacheMissAttribution>,
}

/// Effective prompt size to use as the denominator for cache-hit ratios.
///
/// Providers report `input_tokens` differently:
/// - Anthropic/Claude (split accounting): `input` is the *uncached remainder*,
///   while cache-read and cache-creation tokens are reported separately, so the
///   true prompt size is `input + read + creation`.
/// - OpenAI-style (subset accounting): cached tokens are already counted inside
///   `input`, so the prompt size is just `input`.
///
/// Resolve the accounting while the request's provider identity is still known.
pub fn effective_prompt_tokens(provider: &str, input: u64, read: u64, creation: u64) -> u64 {
    crate::compaction::effective_context_tokens_from_usage(
        provider,
        input,
        Some(read),
        Some(creation),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheMissAttribution {
    pub turn_number: usize,
    pub call_index: u16,
    pub missed_tokens: u64,
    pub reason: String,
}

impl CacheHitInfo {
    /// Fraction of the session's prompt tokens that were served from cache.
    pub fn hit_ratio(&self) -> Option<f32> {
        let denominator = self.prompt_tokens?;
        if denominator == 0 {
            None
        } else {
            Some((self.read_tokens as f32 / denominator as f32).clamp(0.0, 1.0))
        }
    }

    /// Fraction of the previously-cacheable prompt that was actually reused
    /// (read_tokens vs. the prior request's full prompt).
    pub fn optimal_ratio(&self) -> Option<f32> {
        if self.optimal_input_tokens == 0 {
            None
        } else {
            Some((self.read_tokens as f32 / self.optimal_input_tokens as f32).clamp(0.0, 1.0))
        }
    }

    pub fn last_ratio(&self) -> Option<f32> {
        let denominator = self.last_prompt_tokens?;
        if denominator == 0 {
            None
        } else {
            Some((self.last_read_tokens? as f32 / denominator as f32).clamp(0.0, 1.0))
        }
    }

    pub fn last_optimal_ratio(&self) -> Option<f32> {
        let optimal = self.last_optimal_input_tokens?;
        if optimal == 0 {
            None
        } else {
            Some((self.last_read_tokens? as f32 / optimal as f32).clamp(0.0, 1.0))
        }
    }
}

impl UsageInfo {
    /// Return the highest usage percentage across all limit windows (0-100).
    pub fn max_usage_pct(&self) -> u8 {
        let five_hr = (self.five_hour * 100.0).round().clamp(0.0, 100.0) as u8;
        let seven_day = (self.seven_day * 100.0).round().clamp(0.0, 100.0) as u8;
        let spark = self
            .spark
            .map(|v| (v * 100.0).round().clamp(0.0, 100.0) as u8)
            .unwrap_or(0);
        five_hr.max(seven_day).max(spark)
    }
}

/// Memory statistics for the info widget
#[derive(Debug, Default, Clone)]
pub struct MemoryInfo {
    /// Total memory count (project + global)
    pub total_count: usize,
    /// Project-specific memory count
    pub project_count: usize,
    /// Global memory count
    pub global_count: usize,
    /// Count by category
    pub by_category: HashMap<String, usize>,
    /// Whether sidecar is available
    pub sidecar_available: bool,
    /// Whether the memory feature is disabled for this session.
    /// When true, stored counts are still shown but recall/extraction are off.
    pub disabled: bool,
    /// Selected sidecar model/backend label for memory work
    pub sidecar_model: Option<String>,
    /// Current memory activity
    pub activity: Option<MemoryActivity>,
    /// Graph topology for visualization (node positions + edges)
    pub graph_nodes: Vec<GraphNode>,
    /// Directed edges into graph_nodes
    pub graph_edges: Vec<GraphEdge>,
}

impl MemoryInfo {
    pub(crate) fn should_render(&self) -> bool {
        // Always show when memory is enabled (even with 0 memories)
        // to prevent flickering as counts update.
        // When disabled, only show if there are stored memories to report.
        !self.disabled || self.total_count > 0
    }

    pub(crate) fn should_show_activity(&self) -> bool {
        self.activity.as_ref().is_some_and(|activity| {
            activity.is_processing()
                || (matches!(activity.state, MemoryState::Idle)
                    && activity
                        .pipeline
                        .as_ref()
                        .map(PipelineState::is_complete)
                        .unwrap_or(false)
                    && activity.state_since.elapsed() <= Duration::from_secs(5))
        })
    }
}

pub use jcode_tui_mermaid::DiagramInfo;

/// Git repository status for the info widget
#[derive(Debug, Clone, Default)]
pub struct GitInfo {
    pub branch: String,
    pub modified: usize,
    pub staged: usize,
    pub untracked: usize,
    pub ahead: usize,
    pub behind: usize,
    /// First few dirty paths with their porcelain status (capped).
    pub dirty_files: Vec<DirtyFile>,
    /// Total number of dirty paths, including those beyond the cap.
    pub dirty_total: usize,
    /// Lines added across all dirty files (text files only).
    pub added_total: usize,
    /// Lines removed across all dirty files (text files only).
    pub removed_total: usize,
    /// Absolute repository root, used to match agent-edited paths.
    pub repo_root: Option<std::path::PathBuf>,
    /// Most recent commits on HEAD, newest first.
    pub recent_commits: Vec<RecentCommit>,
}

/// One commit for the Commits widget.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RecentCommit {
    /// Abbreviated hash.
    pub hash: String,
    pub subject: String,
    /// Committer time, seconds since the Unix epoch.
    pub timestamp: i64,
    /// Not yet on the upstream branch.
    pub unpushed: bool,
    pub added: Option<usize>,
    pub removed: Option<usize>,
}

/// One dirty path from `git status --porcelain`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DirtyFile {
    /// Single-letter status shown in the Changes widget: `M`, `A`, `D`, `R`,
    /// `U` (conflict), or `?` (untracked).
    pub status: char,
    pub path: String,
    /// Lines added, `None` for binary or unknown.
    pub added: Option<usize>,
    /// Lines removed, `None` for binary or unknown.
    pub removed: Option<usize>,
    /// Last modification time, used for newest-first ordering.
    pub modified_at: Option<std::time::SystemTime>,
}

impl DirtyFile {
    pub fn new(status: char, path: impl Into<String>) -> Self {
        Self {
            status,
            path: path.into(),
            ..Default::default()
        }
    }

    pub fn with_lines(mut self, added: usize, removed: usize) -> Self {
        self.added = Some(added);
        self.removed = Some(removed);
        self
    }
}

impl GitInfo {
    pub fn is_interesting(&self) -> bool {
        self.modified > 0
            || self.staged > 0
            || self.untracked > 0
            || self.ahead > 0
            || self.behind > 0
    }
}

/// Ambient mode status data for the info widget
#[derive(Debug, Clone)]
pub struct AmbientWidgetData {
    pub show_widget: bool,
    pub status: AmbientStatus,
    pub queue_count: usize,
    pub next_queue_preview: Option<String>,
    pub reminder_count: usize,
    pub next_reminder_preview: Option<String>,
    pub last_run_ago: Option<String>,
    pub last_summary: Option<String>,
    pub next_wake: Option<String>,
    pub next_reminder_wake: Option<String>,
    pub budget_percent: Option<f32>,
}

const PAGE_SWITCH_SECONDS: u64 = 30;

/// Data to display in the info widget
#[derive(Debug, Default, Clone)]
pub struct InfoWidgetData {
    pub todos: Vec<TodoItem>,
    /// Goal-level assessments (closed feedback loop and objective)
    /// keyed by todo group (`group: None` covers the ungrouped list). Empty
    /// when the session has no recorded goals or `todos` is a swarm-plan
    /// projection.
    pub todo_goals: Vec<crate::todo::TodoGoal>,
    /// True when `todos` is actually a projection of the shared swarm plan
    /// (task DAG) rather than this session's private todo list. The widget
    /// renders a "Plan" header instead of "Todos" so the two are not
    /// conflated.
    pub todos_are_swarm_plan: bool,
    pub context_info: Option<ContextInfo>,
    /// True when context state is being updated and no authoritative snapshot is available.
    pub context_info_stale: bool,
    pub queue_mode: Option<bool>,
    pub context_limit: Option<usize>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub service_tier: Option<String>,
    pub native_compaction_mode: Option<String>,
    pub native_compaction_threshold_tokens: Option<usize>,
    pub session_count: Option<usize>,
    pub session_name: Option<String>,
    /// Current working directory for this session.
    pub working_dir: Option<String>,
    pub client_count: Option<usize>,
    /// Memory system statistics
    pub memory_info: Option<MemoryInfo>,
    /// Swarm/subagent status
    pub swarm_info: Option<SwarmInfo>,
    /// Background tasks status
    pub background_info: Option<BackgroundInfo>,
    /// Connected MCP servers with their tool counts.
    pub mcp_servers: Vec<(String, usize)>,
    /// Subscription usage info
    pub usage_info: Option<UsageInfo>,
    /// Show consumed rather than remaining percentages in usage limits.
    pub usage_display_used: bool,
    /// Streaming output tokens per second (approximate)
    pub tokens_per_second: Option<f32>,
    /// Average t/s over the last up to 5 completed turns (rolling buffer).
    /// Always shown in the info panel so the user can see throughput at a
    /// glance, even when not actively streaming.
    pub avg_tokens_per_second: Option<f32>,
    /// Hide every t/s readout (config `display.show_tps = false`). The
    /// counters keep running; only the rendering is suppressed.
    pub hide_tps: bool,
    /// Active provider name (openrouter/openai/anthropic/...)
    pub provider_name: Option<String>,
    /// Authentication method used to access the model
    pub auth_method: AuthMethod,
    /// Upstream provider (e.g., which OpenRouter provider served the request: fireworks, etc.)
    pub upstream_provider: Option<String>,
    /// Active connection type (websocket/https/etc.)
    pub connection_type: Option<String>,
    /// Mermaid diagrams to display
    pub diagrams: Vec<DiagramInfo>,
    /// Visible Niri-style workspace rows
    pub workspace_rows: Vec<VisibleWorkspaceRow>,
    /// Lightweight animation tick for workspace map rendering
    pub workspace_animation_tick: u64,
    /// Ambient mode status
    pub ambient_info: Option<AmbientWidgetData>,
    /// Actual API-reported context tokens (from last streaming response)
    /// When available, this is more accurate than the char-based estimate in context_info
    pub observed_context_tokens: Option<u64>,
    /// Session-level cache read ratio, when the active provider reports cache telemetry.
    pub cache_hit_info: Option<CacheHitInfo>,
    /// Conversation compaction status, shown as a compact rounded status card.
    pub compaction_info: Option<CompactionInfo>,
    /// Whether background compaction is currently in progress
    pub is_compacting: bool,
    /// Git repository status
    pub git_info: Option<GitInfo>,
    /// Whether the fixed status line (below the input) is showing model/context
    /// info. When true, the duplicate margin widgets (ModelInfo, ContextUsage,
    /// UsageLimits, KvCache) are suppressed so the info appears in one place.
    /// True for both the always-on mode and the elastic overscroll reveal.
    pub status_line_active: bool,
    /// Whether the status line is pinned permanently on (config `on`), as
    /// opposed to the elastic overscroll reveal. The fixed Overview widget is
    /// only used when this is true, to avoid popping in during elastic reveals.
    pub status_line_pinned: bool,
    /// Available skill names (e.g. /codebase-memory, /bitbucket, ...).
    pub available_skills: Vec<String>,
    /// Absolute paths the agent edited this session (edit-style tool calls),
    /// used to mark agent changes in the Changes widget.
    pub agent_edited: std::sync::Arc<std::collections::HashSet<std::path::PathBuf>>,
}

#[derive(Clone, Debug)]
pub struct CompactionInfo {
    pub is_compacting: bool,
    pub compacted_messages: usize,
    pub active_messages: usize,
    pub summary_chars: usize,
    pub mode: String,
}

impl InfoWidgetData {
    fn widget_disabled(kind: WidgetKind) -> bool {
        matches!(kind, WidgetKind::AmbientMode | WidgetKind::Tips)
    }

    /// Whether the model widget has supplementary info not shown in the status
    /// line: service tier, native compaction mode, or session count/name.
    pub fn has_model_supplementary_info(&self) -> bool {
        model_info_supplementary_height(self) > 0
    }

    pub fn is_empty(&self) -> bool {
        self.todos.is_empty()
            && self.context_info.is_none()
            && self.queue_mode.is_none()
            && self.model.is_none()
            && self.memory_info.is_none()
            && self.swarm_info.is_none()
            && self.background_info.is_none()
            && self.mcp_servers.is_empty()
            && self.diagrams.is_empty()
            && self.workspace_rows.is_empty()
    }

    /// Check if a specific widget kind has data to display
    pub fn has_data_for(&self, kind: WidgetKind) -> bool {
        if Self::widget_disabled(kind) {
            return false;
        }

        match kind {
            WidgetKind::Diagrams => !self.diagrams.is_empty(),
            WidgetKind::WorkspaceMap => !self.workspace_rows.is_empty(),
            WidgetKind::Overview => {
                // Keep the Overview widget as the single merged widget even when
                // the status line is active. It combines todos, kv-cache, git,
                // and supplementary info (service tier, compaction, session)
                // into one box. Having one widget = one anchor = no jumping
                // between multiple widget placements.
                let mut sections = 0usize;
                // Model section: shown when there is supplementary info
                // (service tier, compaction, session) even if the status line
                // is active.
                if self.has_model_supplementary_info()
                    || (!self.status_line_active && self.model.is_some())
                {
                    sections += 1;
                }
                // Context section: shown when stale even if status line active.
                if !self.status_line_active
                    && self
                        .context_info
                        .as_ref()
                        .map(|c| c.total_chars > 0)
                        .unwrap_or(false)
                    || self.status_line_active && self.context_info_stale
                {
                    sections += 1;
                }
                if !self.todos.is_empty() {
                    sections += 1;
                }
                // Tokens per second while streaming.
                if let Some(tps) = self.tokens_per_second
                    && tps.is_finite()
                    && tps > 0.1
                {
                    sections += 1;
                }
                if self
                    .background_info
                    .as_ref()
                    .map(|b| b.running_count > 0)
                    .unwrap_or(false)
                {
                    sections += 1;
                }
                if !self.mcp_servers.is_empty() {
                    sections += 1;
                }
                if self.queue_mode.is_some() {
                    sections += 1;
                }
                // Usage section: always counted. CostBased/Copilot show
                // $cost+tokens (or $NA) in the Overview instead of using a
                // standalone margin widget.
                if self
                    .usage_info
                    .as_ref()
                    .map(|u| u.available)
                    .unwrap_or(false)
                {
                    sections += 1;
                } else if self.status_line_active {
                    // No usage data at all but status line active: $NA line.
                    sections += 1;
                }
                // KV cache is never in the status line, always counts.
                if self.cache_hit_info.is_some() {
                    sections += 1;
                }
                if self.compaction_info.is_some() {
                    sections += 1;
                }
                if self
                    .git_info
                    .as_ref()
                    .map(|g| g.is_interesting())
                    .unwrap_or(false)
                {
                    sections += 1;
                }
                if !self.mcp_servers.is_empty() {
                    sections += 1;
                }
                if !self.available_skills.is_empty() {
                    sections += 1;
                }
                // The Overview is the single merged widget: show it whenever
                // the panel renders any content at all. The compact page
                // always includes the swarm "0 sessions" baseline line, so
                // this is true whenever the panel is renderable — including
                // the model-only case, which must not fall through to the
                // (now content-merged) standalone ModelInfo widget that
                // renders nothing.
                sections >= 1
            }
            WidgetKind::Todos => !self.todos.is_empty(),
            WidgetKind::ContextUsage => {
                if self.status_line_active {
                    // Keep the widget only for the "updating..." stale state;
                    // the normal context bar is already in the status line.
                    return self.context_info_stale;
                }
                self.context_info_stale
                    || self
                        .context_info
                        .as_ref()
                        .map(|c| c.total_chars > 0)
                        .unwrap_or(false)
            }
            WidgetKind::MemoryActivity => self
                .memory_info
                .as_ref()
                .map(MemoryInfo::should_render)
                .unwrap_or(false),
            WidgetKind::RecoveredMemories => self.has_recovered_memories(),
            WidgetKind::SwarmStatus => self
                .swarm_info
                .as_ref()
                .map(|s| !s.managed_members.is_empty())
                .unwrap_or(false),
            WidgetKind::BackgroundTasks => self
                .background_info
                .as_ref()
                .map(|b| b.running_count > 0)
                .unwrap_or(false),
            WidgetKind::Compaction => self.compaction_info.is_some(),
            WidgetKind::AmbientMode => false,
            WidgetKind::UsageLimits => {
                if self.status_line_active {
                    // Suppress CostBased/Copilot (already in status line);
                    // keep OAuth subscription bars with reset times.
                    return self
                        .usage_info
                        .as_ref()
                        .map(|u| {
                            u.available
                                && !matches!(
                                    u.provider,
                                    UsageProvider::CostBased | UsageProvider::Copilot
                                )
                        })
                        .unwrap_or(false);
                }
                self.usage_info
                    .as_ref()
                    .map(|u| u.available)
                    .unwrap_or(false)
            }
            WidgetKind::KvCache => {
                // KV cache is never shown in the status line, so always
                // keep the widget when data is present.
                self.cache_hit_info.is_some()
            }
            WidgetKind::ModelInfo => {
                if self.status_line_active {
                    // Keep the widget when it has supplementary info not shown
                    // in the status line (service tier, native compaction,
                    // session count/name).
                    self.service_tier.is_some()
                        || self.native_compaction_mode.is_some()
                        || self.session_count.is_some()
                        || self
                            .session_name
                            .as_deref()
                            .is_some_and(|s| !s.trim().is_empty())
                } else {
                    self.model.is_some()
                        || self.has_model_supplementary_info()
                        || self.session_count.is_some()
                        || self
                            .session_name
                            .as_deref()
                            .is_some_and(|s| !s.trim().is_empty())
                }
            }
            WidgetKind::Tips => false,
            WidgetKind::GitStatus => self
                .git_info
                .as_ref()
                .map(changes_has_data)
                .unwrap_or(false),
            WidgetKind::Commits => self
                .git_info
                .as_ref()
                .map(commits_has_data)
                .unwrap_or(false),
        }
    }

    /// Whether any memory was recovered (injected) in the recent activity
    /// window. Drives the attached `RecoveredMemories` panel below the
    /// Overview: it only appears when there is something to show.
    pub fn has_recovered_memories(&self) -> bool {
        self.memory_info
            .as_ref()
            .and_then(|info| info.activity.as_ref())
            .map(|activity| {
                activity.recent_events.iter().any(|e| {
                    matches!(
                        &e.kind,
                        MemoryEventKind::MemoryInjected { count, .. } if *count > 0
                    )
                })
            })
            .unwrap_or(false)
    }

    /// Get list of widget kinds that have data, in priority order
    /// Get effective priority for a widget, accounting for dynamic state.
    /// UsageLimits gets bumped up when usage is high.
    /// MemoryActivity gets bumped up while memory work is actively processing.
    pub fn effective_priority(&self, kind: WidgetKind) -> u8 {
        match kind {
            WidgetKind::MemoryActivity => {
                if self
                    .memory_info
                    .as_ref()
                    .and_then(|info| info.activity.as_ref())
                    .map(MemoryActivity::is_processing)
                    .unwrap_or(false)
                {
                    0
                } else {
                    kind.priority()
                }
            }
            WidgetKind::RecoveredMemories => kind.priority(),
            WidgetKind::UsageLimits => {
                let max_pct = self
                    .usage_info
                    .as_ref()
                    .map(|u| u.max_usage_pct())
                    .unwrap_or(0);
                if max_pct >= 80 {
                    1 // Very high - right after diagrams
                } else if max_pct >= 50 {
                    3 // Elevated - after overview and todos
                } else {
                    kind.priority()
                }
            }
            WidgetKind::Compaction => {
                if self
                    .compaction_info
                    .as_ref()
                    .map(|info| info.is_compacting)
                    .unwrap_or(false)
                {
                    2
                } else {
                    kind.priority()
                }
            }
            WidgetKind::SwarmStatus => {
                // A session actively managing agents wants them visible: the
                // dock is the cockpit for the swarm, so rank it just under
                // todos while any managed agent is still live.
                let managing = self
                    .swarm_info
                    .as_ref()
                    .map(|s| !s.managed_members.is_empty())
                    .unwrap_or(false);
                if managing { 3 } else { kind.priority() }
            }
            _ => kind.priority(),
        }
    }

    pub fn available_widgets(&self) -> Vec<WidgetKind> {
        let mut widgets: Vec<WidgetKind> = WidgetKind::all_by_priority()
            .iter()
            .copied()
            .filter(|&kind| self.has_data_for(kind))
            .collect();
        widgets.sort_by_key(|&kind| self.effective_priority(kind));

        // Always return only the Overview plus any special widgets
        // (WorkspaceMap, Diagrams, and the non-pinned RecoveredMemories fallback)
        // that are not merged into Overview.
        if self.status_line_pinned && widgets.contains(&WidgetKind::Overview) {
            widgets.retain(|&k| {
                k == WidgetKind::Overview
                    || k == WidgetKind::WorkspaceMap
                    || k == WidgetKind::Diagrams
                    || k == WidgetKind::RecoveredMemories
            });
        }

        widgets
    }
}

/// State for a single widget instance
#[derive(Debug, Clone, Default)]
struct SingleWidgetState {
    /// Current page index (for widgets with multiple pages)
    page_index: usize,
    /// Last time the page advanced
    last_page_switch: Option<Instant>,
}

/// Global state for all widgets
#[derive(Debug, Clone)]
struct WidgetsState {
    /// Whether the user has disabled widgets
    enabled: bool,
    /// Per-widget state (keyed by WidgetKind)
    widget_states: HashMap<WidgetKind, SingleWidgetState>,
    /// Current placements (updated each frame)
    placements: Vec<WidgetPlacement>,
    /// Persistent widget anchors (HUD slot memory, including hidden-in-place ones)
    anchors: Vec<super::info_widget_layout::WidgetAnchor>,
    /// Settlement tracker: which transcript lines' negative space has stopped
    /// changing and may therefore host a *new* resident widget.
    settlement: super::info_widget_settle::SettlementTracker,
    /// Content width the current anchors were computed at. A width change means
    /// every transcript line re-wrapped, so anchors (keyed by absolute line) are
    /// meaningless and must be flushed for one clean global re-layout.
    anchors_area_width: u16,
    /// When the SwarmStatus dock was last engaged (placed or anchored). Lets the
    /// inline swarm strip keep standing down through brief dock dropouts instead
    /// of popping back for a few frames (which resizes the bottom chrome and
    /// bounces the transcript).
    swarm_dock_last_engaged: Option<Instant>,
}

impl Default for WidgetsState {
    fn default() -> Self {
        Self {
            enabled: true,
            widget_states: HashMap::new(),
            placements: Vec::new(),
            anchors: Vec::new(),
            settlement: super::info_widget_settle::SettlementTracker::default(),
            anchors_area_width: 0,
            swarm_dock_last_engaged: None,
        }
    }
}

/// Global widget state (for polling across frames)
static WIDGETS_STATE: Mutex<Option<WidgetsState>> = Mutex::new(None);

fn get_or_init_state() -> std::sync::MutexGuard<'static, Option<WidgetsState>> {
    let mut guard = WIDGETS_STATE.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        *guard = Some(WidgetsState::default());
    }
    guard
}

/// Toggle widget visibility (user preference)
pub fn toggle_enabled() {
    let mut guard = get_or_init_state();
    if let Some(state) = guard.as_mut() {
        state.enabled = !state.enabled;
    }
}

/// Check if widget is enabled by user
pub fn is_enabled() -> bool {
    get_or_init_state()
        .as_ref()
        .map(|s| s.enabled)
        .unwrap_or(true)
}

/// Intersect the dock-gating `reliable` margin profile with the settlement
/// profile, so a *new* resident widget can only be placed next to transcript
/// lines whose negative space has stopped changing (never the streaming tail or
/// a re-rendering region). Also flushes all anchors when the content width
/// changes: a re-wrap moves every transcript line, so line-keyed anchors are
/// meaningless and one clean global re-layout is better than widgets drawn over
/// reflowed text.
pub fn apply_settlement(margins: &mut Margins, area_width: u16) {
    let mut guard = get_or_init_state();
    let Some(state) = guard.as_mut() else {
        return;
    };
    if state.anchors_area_width != area_width {
        state.anchors.clear();
        state.settlement.reset();
        state.anchors_area_width = area_width;
    }
    let settled = state.settlement.observe(margins, area_width);
    intersect_widths(&mut margins.right_reliable, &settled.right);
    if margins.centered {
        intersect_widths(&mut margins.left_reliable, &settled.left);
    }
}

/// Per-row minimum of `dst` and `src`. If `dst` is empty (no look-ahead
/// profile), it becomes `src` so settlement still gates docking on its own.
fn intersect_widths(dst: &mut Vec<u16>, src: &[u16]) {
    if dst.is_empty() {
        *dst = src.to_vec();
        return;
    }
    for (row, d) in dst.iter_mut().enumerate() {
        *d = (*d).min(src.get(row).copied().unwrap_or(0));
    }
}

/// Calculate widget placements for multiple widgets
/// Returns a list of placements for widgets that fit
pub fn calculate_placements(
    messages_area: Rect,
    margins: &Margins,
    data: &InfoWidgetData,
) -> Vec<WidgetPlacement> {
    let mut guard = get_or_init_state();
    let state = match guard.as_mut() {
        Some(s) => s,
        None => return Vec::new(),
    };

    // When the status line is pinned on (default), place the Overview widget
    // at a fixed top-right position (always visible, drawn on top of content).
    // Skip on very small terminals where the widget would take too much space;
    // those fall through to the normal margin-based placement.
    const MIN_FIXED_WIDTH: u16 = 52;
    const MIN_FIXED_HEIGHT: u16 = 10;
    if state.enabled
        && data.status_line_pinned
        && data.has_data_for(WidgetKind::Overview)
        && messages_area.width >= MIN_FIXED_WIDTH
        && messages_area.height >= MIN_FIXED_HEIGHT
    {
        let mut placements = calculate_fixed_overview_placement(messages_area, data);

        // If memories were recovered this cycle, they are now rendered inline
        // within the Overview panel (after the memory count line), so no
        // separate placement is needed.

        // Reserve the Overview's rect so the anchored placement doesn't place
        // other widgets on top of it. We do this by running the anchored placer
        // with a modified data that suppresses Overview and mergeable widgets.
        if placements.iter().any(|p| p.kind == WidgetKind::Overview) {
            // Run anchored placement for remaining special widgets (WorkspaceMap,
            // Diagrams). Suppress Overview from the anchored data so it doesn't
            // try to also place it. Reserve the Overview's rect by zeroing the
            // free widths in those rows so other widgets won't overlap it.
            let mut anchored_data = data.clone();
            anchored_data.todos.clear();
            anchored_data.cache_hit_info = None;
            anchored_data.compaction_info = None;
            anchored_data.git_info = None;
            anchored_data.background_info = None;
            anchored_data.queue_mode = None;
            anchored_data.service_tier = None;
            anchored_data.native_compaction_mode = None;
            anchored_data.session_count = None;
            anchored_data.session_name = None;
            anchored_data.context_info_stale = false;
            anchored_data.mcp_servers.clear();
            anchored_data.available_skills.clear();
            // Also suppress usage_info and context_info so has_data_for(Overview)
            // returns false, preventing a duplicate Overview from anchored placement.
            anchored_data.usage_info = None;
            anchored_data.context_info = None;
            anchored_data.model = None;
            // Clear status_line_active so the "no usage but status line active"
            // fallback (which always counts 1 section) doesn't make Overview
            // appear again. Also clear remaining fields that count as sections.
            anchored_data.status_line_active = false;
            anchored_data.tokens_per_second = None;
            anchored_data.avg_tokens_per_second = None;
            anchored_data.swarm_info = None;
            // Suppress memory_info so the standalone MemoryActivity widget
            // is not placed — memory count and recovered memories are now
            // rendered inline within the Overview panel.
            anchored_data.memory_info = None;

            // Zero out the free width on the right side where the Overview sits,
            // so the anchored system won't place WorkspaceMap/Diagrams there.
            let mut reserved_margins = margins.clone();
            let reserved_rects: Vec<Rect> = placements
                .iter()
                .filter(|p| p.kind == WidgetKind::Overview)
                .map(|p| p.rect)
                .collect();
            for rect in reserved_rects {
                let row_start = (rect.y.saturating_sub(messages_area.y)) as usize;
                let row_end = row_start + rect.height as usize;
                for row in row_start..row_end.min(reserved_margins.right_widths.len()) {
                    let right_edge = messages_area.x + messages_area.width;
                    let rect_left = right_edge.saturating_sub(rect.width);
                    let free_left_of_rect = rect_left.saturating_sub(messages_area.x);
                    // Only zero the width that the rect occupies on the right side.
                    // Keep whatever free space is to the LEFT of the rect.
                    if reserved_margins.right_widths[row] > free_left_of_rect {
                        reserved_margins.right_widths[row] = free_left_of_rect;
                    }
                }
            }

            let outcome = super::info_widget_layout::calculate_placements_anchored(
                messages_area,
                &reserved_margins,
                &anchored_data,
                state.enabled,
                &state.anchors,
            );
            state.anchors = outcome.anchors;
            placements.extend(outcome.visible);
        }

        state.placements = placements.clone();
        if swarm_dock_engaged(state) {
            state.swarm_dock_last_engaged = Some(Instant::now());
        }
        return placements;
    }

    let outcome = super::info_widget_layout::calculate_placements_anchored(
        messages_area,
        margins,
        data,
        state.enabled,
        &state.anchors,
    );
    state.anchors = outcome.anchors;
    state.placements = outcome.visible.clone();
    if swarm_dock_engaged(state) {
        state.swarm_dock_last_engaged = Some(Instant::now());
    }
    outcome.visible
}

/// Place the Overview widget at a fixed top-right position in the messages
/// area, always visible. The widget is drawn on top of transcript content
/// (cells behind it are cleared), so it never hides or jumps.
fn calculate_fixed_overview_placement(
    messages_area: Rect,
    data: &InfoWidgetData,
) -> Vec<WidgetPlacement> {
    const FIXED_WIDTH: u16 = 44;
    const MAX_HEIGHT: u16 = 30;
    const MIN_WIDTH: u16 = 28;

    // Adapt width to terminal: use full width on narrow terminals, cap at FIXED_WIDTH on wider ones.
    let width = FIXED_WIDTH
        .min(messages_area.width.saturating_sub(2))
        .max(MIN_WIDTH);
    let width = if messages_area.width < MIN_WIDTH + 2 {
        messages_area.width.saturating_sub(2)
    } else {
        width
    };

    let inner_width = (width.saturating_sub(2)) as usize;
    let inner_h = MAX_HEIGHT.saturating_sub(2);
    let layout = compute_page_layout(data, inner_width, inner_h);
    let content_h = layout.max_page_height;
    if content_h == 0 {
        return Vec::new();
    }
    // +2 for the border, +1 for dots if shown
    let total_h = content_h + 2 + u16::from(layout.show_dots);
    let clamped_h = total_h.min(messages_area.height).min(MAX_HEIGHT);

    let x = messages_area.x + messages_area.width - width;
    let y = messages_area.y;

    vec![WidgetPlacement {
        kind: WidgetKind::Overview,
        rect: Rect::new(x, y, width, clamped_h),
        side: Side::Right,
        overview_layout: if layout.max_page_height <= clamped_h.saturating_sub(2) {
            Some(layout)
        } else {
            None
        },
    }]
}

/// How long the inline swarm strip keeps standing down after the SwarmStatus
/// dock disengages. The dock's placement naturally churns while content
/// streams past it (hidden-in-place blinks, anchor abandonment, re-homing a
/// few frames later). Each strip appearance adds a row to the bottom chrome
/// and shoves the whole transcript up, so reacting instantly turns that churn
/// into visible up/down flicker. Standing down through a short linger converts
/// the churn into "strip stays hidden"; a genuine dock removal only delays the
/// strip's return by this much, once.
const SWARM_STRIP_STAND_DOWN_LINGER: Duration = Duration::from_millis(2000);

/// Whether the SwarmStatus dock widget is engaged: either actually placed, or
/// hidden-in-place behind a live anchor (a wide transcript line is momentarily
/// covering its slot and it will pop back into the same spot).
fn swarm_dock_engaged(state: &WidgetsState) -> bool {
    state.enabled
        && (state
            .placements
            .iter()
            .any(|p| p.kind == WidgetKind::SwarmStatus)
            || state
                .anchors
                .iter()
                .any(|a| a.placement.kind == WidgetKind::SwarmStatus))
}

/// Whether the inline swarm strip (above the status line) should stand down
/// because the SwarmStatus dock widget (margin HUD) is showing - or was very
/// recently showing - the same agents.
///
/// The strip is built before widget placement runs each frame, so this checks
/// the previous frame's state, like [`widget_visible_facts`]. Engagement
/// includes hidden-in-place anchors, and disengagement is debounced by
/// [`SWARM_STRIP_STAND_DOWN_LINGER`]: both exist so the dock's frame-to-frame
/// placement churn cannot toggle the strip row on and off, which resizes the
/// bottom chrome and makes the whole transcript jump up and down (flicker).
/// One frame of overlap when the dock first appears is visually harmless.
pub(crate) fn swarm_strip_stands_down_for_dock() -> bool {
    let guard = get_or_init_state();
    let Some(state) = guard.as_ref() else {
        return false;
    };
    if swarm_dock_engaged(state) {
        return true;
    }
    state
        .swarm_dock_last_engaged
        .is_some_and(|at| at.elapsed() < SWARM_STRIP_STAND_DOWN_LINGER)
}

/// Forget the per-frame placement/anchor state because the widget render pass
/// was skipped this frame (idle donut takeover, or no widget data at all).
/// Without this, `state.placements` keeps reporting widgets from the last
/// widget-bearing frame: the swarm strip would stand down for a dock that is
/// no longer drawn, leaving the managed agents visible nowhere.
pub(crate) fn note_widget_pass_skipped() {
    let mut guard = get_or_init_state();
    if let Some(state) = guard.as_mut() {
        state.placements.clear();
        state.anchors.clear();
        state.swarm_dock_last_engaged = None;
    }
}

/// Clear the remembered per-frame widget placements (and anchors). Tests that
/// assert on placement-dependent behavior (e.g. the swarm strip standing down
/// while the dock is visible) call this so state from earlier tests in the
/// same process cannot leak into their frame.
#[cfg(test)]
pub(crate) fn clear_widget_placements_for_tests() {
    let mut guard = get_or_init_state();
    if let Some(state) = guard.as_mut() {
        state.placements.clear();
        state.anchors.clear();
        state.swarm_dock_last_engaged = None;
    }
}

/// Force the process-global widget switch off for the guard's lifetime and
/// restore the previous value on drop. Deterministic render fixtures that
/// must not show the Overview panel (small terminals where the fixed
/// top-right placement would truncate the rows under test) use this instead
/// of mutating config.
#[cfg(test)]
pub(crate) struct WidgetsDisabledGuard {
    previous: bool,
}

#[cfg(test)]
impl WidgetsDisabledGuard {
    pub(crate) fn new() -> Self {
        let mut guard = get_or_init_state();
        let previous = guard.as_ref().map(|s| s.enabled).unwrap_or(true);
        if let Some(state) = guard.as_mut() {
            state.enabled = false;
        }
        Self { previous }
    }
}

#[cfg(test)]
impl Drop for WidgetsDisabledGuard {
    fn drop(&mut self) {
        let mut guard = get_or_init_state();
        if let Some(state) = guard.as_mut() {
            state.enabled = self.previous;
            state.placements.clear();
            state.anchors.clear();
            state.swarm_dock_last_engaged = None;
        }
    }
}

/// Calculate the height needed for a specific widget type
pub(crate) fn calculate_widget_height(
    kind: WidgetKind,
    data: &InfoWidgetData,
    width: u16,
    max_height: u16,
) -> u16 {
    let inner_width = width.saturating_sub(2) as usize;
    let border_height = 2u16;

    let content_height = match kind {
        WidgetKind::WorkspaceMap => {
            if data.workspace_rows.is_empty() {
                return 0;
            }
            let (_preferred_w, preferred_h) =
                super::workspace_map_widget::preferred_size(&data.workspace_rows);
            preferred_h.min(max_height.saturating_sub(border_height))
        }
        WidgetKind::Overview => {
            let overview = data.clone();
            let inner_h = max_height.saturating_sub(border_height);
            let layout = compute_page_layout(&overview, inner_width, inner_h);
            if layout.max_page_height == 0 {
                return 0;
            }
            layout.max_page_height
        }
        WidgetKind::Diagrams => {
            if data.diagrams.is_empty() {
                return 0;
            }
            // Use the full available height so the image fills the panel
            max_height.saturating_sub(border_height)
        }
        // Text widgets: the height is whatever the body actually renders at
        // the full available height, so estimate and render can never drift.
        _ => {
            // Rendering is permissive (e.g. the swarm widget still draws a
            // session list), but layout only admits widgets with data.
            if !data.has_data_for(kind) {
                return 0;
            }
            let inner = Rect::new(
                0,
                0,
                width.saturating_sub(2),
                max_height.saturating_sub(border_height),
            );
            let framed = render_widget_content(kind, data, inner);
            if framed.is_empty() {
                return 0;
            }
            framed.lines.len() as u16
        }
    };

    let total = content_height + border_height;
    total.min(max_height)
}

/// Legacy API for backwards compatibility - will be removed
/// Calculate the widget layout based on available space
/// Returns the Rect where the widget should be drawn, or None if it shouldn't show
#[deprecated(note = "Use calculate_placements instead")]
pub fn calculate_layout(
    messages_area: Rect,
    free_widths: &[u16],
    data: &InfoWidgetData,
) -> Option<Rect> {
    let margins = Margins {
        right_widths: free_widths.to_vec(),
        left_widths: Vec::new(),
        centered: false,
        ..Default::default()
    };
    let placements = calculate_placements(messages_area, &margins, data);
    placements.first().map(|p| p.rect)
}

/// Render all placed widgets
pub fn render_all(frame: &mut Frame, placements: &[WidgetPlacement], data: &InfoWidgetData) {
    for placement in placements {
        render_single_widget(frame, placement, data);
    }
}

/// Render a single widget at its placement
fn render_single_widget(frame: &mut Frame, placement: &WidgetPlacement, data: &InfoWidgetData) {
    let rect = placement.rect;

    // Semi-transparent looking border (using dim colors). When the status line
    // is pinned, the Overview floats over transcript content. Give the whole
    // panel (including borders) a dark background so wide characters in the
    // transcript cannot bleed through or shift the border position.
    let dark_bg = rgb(30, 30, 35);
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(rgb(70, 70, 80)).dim())
        .style(Style::default().bg(dark_bg));

    if placement.kind == WidgetKind::WorkspaceMap {
        block = block.title(Span::styled(
            " Workspace ",
            Style::default().fg(rgb(120, 120, 130)).dim(),
        ));
    }

    let inner = block.inner(rect);

    // Diagrams need special handling - render image instead of text
    if placement.kind == WidgetKind::Diagrams {
        let mut chrome = Framed::default().title(frame::label("◇ Diagram"));
        if data.diagrams.len() > 1 {
            chrome = chrome.title_right(frame::dim(format!("1/{}", data.diagrams.len())));
        }
        frame.render_widget(Clear, rect);
        frame.render_widget(chrome.apply(block, rect.width), rect);
        render_diagrams_widget(frame, inner, data);
        return;
    }
    if placement.kind == WidgetKind::Overview {
        // Reuse the layout computed at placement time so the height check and
        // the render cannot disagree. Check the overview would actually render
        // content before drawing the border.
        let layout = placement
            .overview_layout
            .clone()
            .unwrap_or_else(|| compute_page_layout(data, inner.width as usize, inner.height));
        if layout.pages.is_empty() || layout.max_page_height == 0 {
            return;
        }
        let Some(framed) = render_overview_framed(data, inner, layout) else {
            return;
        };
        frame.render_widget(Clear, rect);
        frame.render_widget(framed.apply(block, rect.width), rect);
        frame.render_widget(Paragraph::new(framed.lines), inner);
        return;
    }
    if placement.kind == WidgetKind::WorkspaceMap {
        if data.workspace_rows.is_empty() || inner.width == 0 || inner.height == 0 {
            return;
        }
        let chrome = Framed::default().title(frame::label("Workspace"));
        frame.render_widget(Clear, rect);
        frame.render_widget(chrome.apply(block, rect.width), rect);
        super::workspace_map_widget::render_workspace_map(
            frame.buffer_mut(),
            inner,
            &data.workspace_rows,
            data.workspace_animation_tick,
        );
        return;
    }
    let framed = render_widget_content(placement.kind, data, inner);
    if framed.is_empty() {
        return;
    }
    // All floating info widgets can overlap transcript content when the
    // status line is active. The dark background on the block already wipes
    // the cells underneath, so no extra Clear widget is needed here.
    frame.render_widget(framed.apply(block, rect.width), rect);
    frame.render_widget(Paragraph::new(framed.lines), inner);
}

/// Render mermaid diagrams widget (renders images, not text)
fn render_diagrams_widget(frame: &mut Frame, inner: Rect, data: &InfoWidgetData) {
    if data.diagrams.is_empty() {
        return;
    }

    // For now, just render the first/most recent diagram
    // Could add pagination later for multiple diagrams
    let diagram = &data.diagrams[0];

    // Scale up as well as down so margin diagrams use the whole widget instead
    // of appearing as a small top-left crop in a large panel.
    super::mermaid::render_image_widget_scale(diagram.hash, inner, frame.buffer_mut(), false);
}

/// Overview: the rotating multi-section page. Border layout: `Overview` or
/// the focused page's name top-left, page dots bottom-right. Takes the layout
/// computed at placement time so the height check and the render agree.
fn render_overview_framed(
    data: &InfoWidgetData,
    inner: Rect,
    layout: PageLayout,
) -> Option<Framed> {
    if inner.width == 0 || inner.height == 0 {
        return None;
    }

    if layout.pages.is_empty() || layout.max_page_height == 0 {
        return None;
    }

    let mut guard = get_or_init_state();
    let state = guard.as_mut()?;
    let widget_state = state.widget_states.entry(WidgetKind::Overview).or_default();

    if layout.pages.len() > 1 {
        let now = Instant::now();
        let should_advance = widget_state
            .last_page_switch
            .map(|last| now.duration_since(last).as_secs() >= PAGE_SWITCH_SECONDS)
            .unwrap_or(true);
        if should_advance {
            widget_state.page_index = (widget_state.page_index + 1) % layout.pages.len();
            widget_state.last_page_switch = Some(now);
        }
    } else {
        widget_state.page_index = 0;
        widget_state.last_page_switch = None;
    }

    let page_index = widget_state.page_index.min(layout.pages.len() - 1);
    drop(guard);
    let page = layout.pages[page_index];
    let mut lines = render_page(page.kind, data, inner);

    // If the page rendered no content, bail out to avoid an empty box
    if lines.is_empty() {
        return None;
    }
    lines.truncate(inner.height as usize);

    let title = match page.kind {
        InfoPageKind::CompactOnly => "Overview",
        InfoPageKind::TodosExpanded => {
            if data.todos_are_swarm_plan {
                "Overview · plan"
            } else {
                "Overview · todos"
            }
        }
        InfoPageKind::MemoryExpanded => "Overview · memory",
    };
    let mut framed = Framed::body(lines).title(frame::label(title));
    // The Changes section marks agent-edited files; explain the dot here since
    // compact sections have no border of their own. Page dots keep the right.
    if let Some(legend) = changes_legend(data, inner.height) {
        framed = framed.footer(legend);
    }
    if layout.show_dots {
        let mut dots: Vec<Span<'static>> = Vec::new();
        for i in 0..layout.pages.len() {
            if i > 0 {
                dots.push(Span::raw(" "));
            }
            if i == page_index {
                dots.push(Span::styled("●", Style::default().fg(rgb(170, 170, 180))));
            } else {
                dots.push(Span::styled("○", Style::default().fg(rgb(100, 100, 110))));
            }
        }
        framed = framed.footer_right(Line::from(dots));
    }
    Some(framed)
}
#[cfg(test)]
#[derive(Debug, Clone)]
struct MemorySubgraph {
    nodes: Vec<GraphNode>,
    _edges: Vec<GraphEdge>,
}
#[cfg(test)]
fn select_contextual_subgraph(
    info: &MemoryInfo,
    max_nodes: usize,
    max_edges: usize,
) -> Option<MemorySubgraph> {
    if info.graph_nodes.is_empty() || max_nodes == 0 {
        return None;
    }
    let node_count = info.graph_nodes.len();
    let center_idx = pick_subgraph_center(info)?;
    let mut neighbors: Vec<Vec<(usize, usize)>> = vec![Vec::new(); node_count];
    for (edge_idx, edge) in info.graph_edges.iter().enumerate() {
        if edge.source >= node_count || edge.target >= node_count {
            continue;
        }
        neighbors[edge.source].push((edge.target, edge_idx));
        neighbors[edge.target].push((edge.source, edge_idx));
    }
    let mut selected = Vec::with_capacity(max_nodes.min(node_count));
    let mut selected_set: HashSet<usize> = HashSet::new();
    let mut queue = std::collections::VecDeque::new();
    selected.push(center_idx);
    selected_set.insert(center_idx);
    queue.push_back(center_idx);
    while let Some(current) = queue.pop_front() {
        if selected.len() >= max_nodes {
            break;
        }
        let mut ranked = neighbors[current].clone();
        ranked.sort_by(|(a_idx, a_edge), (b_idx, b_edge)| {
            edge_kind_priority(&info.graph_edges[*b_edge].kind)
                .cmp(&edge_kind_priority(&info.graph_edges[*a_edge].kind))
                .then_with(|| {
                    graph_node_score(&info.graph_nodes[*b_idx])
                        .partial_cmp(&graph_node_score(&info.graph_nodes[*a_idx]))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| a_idx.cmp(b_idx))
        });
        for (next_idx, _) in ranked {
            if selected.len() >= max_nodes {
                break;
            }
            if selected_set.insert(next_idx) {
                selected.push(next_idx);
                queue.push_back(next_idx);
            }
        }
    }

    if selected.len() < max_nodes {
        let mut remaining: Vec<usize> = (0..node_count)
            .filter(|idx| !selected_set.contains(idx))
            .collect();
        remaining.sort_by(|a, b| {
            graph_node_score(&info.graph_nodes[*b])
                .partial_cmp(&graph_node_score(&info.graph_nodes[*a]))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.cmp(b))
        });
        for idx in remaining {
            if selected.len() >= max_nodes {
                break;
            }
            selected_set.insert(idx);
            selected.push(idx);
        }
    }

    let mut old_to_new = HashMap::new();
    let mut sub_nodes = Vec::with_capacity(selected.len());
    for (new_idx, old_idx) in selected.iter().copied().enumerate() {
        old_to_new.insert(old_idx, new_idx);
        sub_nodes.push(info.graph_nodes[old_idx].clone());
    }

    let center_new = old_to_new.get(&center_idx).copied().unwrap_or(0);
    let mut dedup: HashSet<(usize, usize, String)> = HashSet::new();
    let mut sub_edges: Vec<GraphEdge> = info
        .graph_edges
        .iter()
        .filter_map(|edge| {
            let source = *old_to_new.get(&edge.source)?;
            let target = *old_to_new.get(&edge.target)?;
            if source == target {
                return None;
            }
            if !dedup.insert((source, target, edge.kind.clone())) {
                return None;
            }
            Some(GraphEdge {
                source,
                target,
                kind: edge.kind.clone(),
            })
        })
        .collect();

    sub_edges.sort_by(|a, b| {
        let a_center = a.source == center_new || a.target == center_new;
        let b_center = b.source == center_new || b.target == center_new;
        b_center
            .cmp(&a_center)
            .then_with(|| edge_kind_priority(&b.kind).cmp(&edge_kind_priority(&a.kind)))
            .then_with(|| a.source.cmp(&b.source))
            .then_with(|| a.target.cmp(&b.target))
    });
    if sub_edges.len() > max_edges {
        sub_edges.truncate(max_edges);
    }

    Some(MemorySubgraph {
        nodes: sub_nodes,
        _edges: sub_edges,
    })
}

#[cfg(test)]
fn pick_subgraph_center(info: &MemoryInfo) -> Option<usize> {
    let mut best_idx: Option<usize> = None;
    let mut best_score: f32 = -1.0;

    for (idx, node) in info.graph_nodes.iter().enumerate() {
        let mut score = graph_node_score(node);
        if node.kind == "tag" || node.kind == "cluster" {
            score -= 0.75;
        }
        if !node.is_active {
            score -= 1.0;
        }
        if score > best_score {
            best_score = score;
            best_idx = Some(idx);
        }
    }

    best_idx
}

#[cfg(test)]
fn edge_kind_priority(kind: &str) -> u8 {
    match kind {
        "contradicts" => 6,
        "supersedes" => 5,
        "derived_from" => 4,
        "relates_to" => 3,
        "in_cluster" => 2,
        "has_tag" => 1,
        _ => 1,
    }
}

/// Render content for a specific widget type: body rows plus border text.
///
/// Merged-widget kinds (Todos, ContextUsage, UsageLimits, KvCache,
/// BackgroundTasks, GitStatus, Tips) still have Framed renderers here so the
/// non-pinned fallback layout keeps working; the pinned status-line layout
/// only ever places the Overview plus the specials.
fn render_widget_content(kind: WidgetKind, data: &InfoWidgetData, inner: Rect) -> Framed {
    let framed = match kind {
        // Image/map/paged widgets are handled specially in render_single_widget.
        WidgetKind::Diagrams | WidgetKind::WorkspaceMap | WidgetKind::Overview => {
            return Framed::default();
        }
        WidgetKind::Todos => render_todos_widget(data, inner),
        WidgetKind::MemoryActivity => render_memory_widget(data, inner),
        WidgetKind::RecoveredMemories => {
            Framed::body(render_recovered_memories_widget(data, inner))
        }
        WidgetKind::SwarmStatus | WidgetKind::ContextUsage | WidgetKind::BackgroundTasks => {
            // Swarm/background/context compact content joined into the Overview
            // sections; standalone framed rendering is handled via their own
            // widgets below for the fallback layout.

            match kind {
                WidgetKind::SwarmStatus => render_swarm_widget(data, inner),
                WidgetKind::ContextUsage => Framed::body(render_context_compact(data, inner)),
                _ => render_background_widget(data, inner),
            }
        }
        WidgetKind::AmbientMode => render_ambient_widget(data, inner),
        WidgetKind::Compaction => render_compaction_widget(data, inner),
        WidgetKind::UsageLimits => render_usage_widget(data, inner),
        WidgetKind::KvCache => render_kv_cache_widget(data, inner),
        // Runtime rows plus the supplementary lines the status line never
        // shows (native compaction mode, threshold).
        WidgetKind::ModelInfo => {
            let mut framed = render_model_widget(data, inner);
            framed
                .lines
                .extend(render_model_info_supplementary(data, inner));
            framed
        }
        WidgetKind::Tips => render_tips_widget(inner),
        WidgetKind::GitStatus => render_changes_framed(data, inner),
        WidgetKind::Commits => render_commits_widget(data, inner),
    };
    framed.settle()
}

fn render_compaction_widget(data: &InfoWidgetData, inner: Rect) -> Framed {
    let Some(info) = data.compaction_info.as_ref() else {
        return Framed::default();
    };
    let title_color = if info.is_compacting {
        rgb(255, 220, 140)
    } else {
        rgb(110, 210, 140)
    };
    let status = if info.is_compacting {
        "compacting"
    } else {
        "compacted"
    };
    let summary_tokens = (info.summary_chars / crate::compaction::CHARS_PER_TOKEN)
        .max(usize::from(info.summary_chars > 0));
    let detail = format!(
        "{} old · {} active · ~{} summary tok",
        info.compacted_messages, info.active_messages, summary_tokens
    );
    Framed::body(vec![Line::from(Span::styled(
        truncate_smart(&detail, inner.width as usize),
        Style::default().fg(rgb(180, 180, 190)),
    ))])
    .title(Line::from(vec![
        frame::label("Compaction "),
        Span::styled(status, Style::default().fg(title_color).bold()),
    ]))
    .title_right(frame::dim(info.mode.to_string()))
}

fn render_kv_cache_widget(data: &InfoWidgetData, inner: Rect) -> Framed {
    let Some(cache) = data.cache_hit_info.as_ref() else {
        return Framed::default();
    };
    // Top border: the headline yield. Body: the per-rate breakdown, then the
    // misses themselves. Bottom border: the miss total and overflow.
    let mut framed =
        Framed::body(vec![render_kv_cache_rates_line(cache)]).title(render_kv_cache_title(cache));

    if cache.miss_attributions.is_empty() {
        return framed.footer_right(Span::styled(
            "no misses",
            Style::default().fg(rgb(110, 210, 140)),
        ));
    }

    let total_missed: u64 = cache
        .miss_attributions
        .iter()
        .map(|sample| sample.missed_tokens)
        .sum();
    framed = framed.footer_right(Line::from(vec![
        Span::styled(
            compact_token_count(total_missed),
            Style::default().fg(rgb(255, 200, 100)),
        ),
        frame::dim(" missed"),
    ]));

    let max_w = inner.width as usize;
    for sample in cache.miss_attributions.iter().take(5) {
        let head = format!(
            "{} {} miss ",
            format_cache_turn_label(sample.turn_number, sample.call_index),
            compact_token_count(sample.missed_tokens)
        );
        let reason_w = max_w.saturating_sub(UnicodeWidthStr::width(head.as_str()) + 2);
        framed.lines.push(Line::from(vec![
            Span::styled(
                format_cache_turn_label(sample.turn_number, sample.call_index),
                Style::default().fg(rgb(140, 180, 255)).bold(),
            ),
            Span::styled(
                format!(" {} miss ", compact_token_count(sample.missed_tokens)),
                Style::default().fg(rgb(255, 200, 100)),
            ),
            Span::styled(
                format!("({})", truncate_smart(&sample.reason, reason_w.max(4))),
                Style::default().fg(rgb(140, 140, 150)),
            ),
        ]));
    }

    if cache.miss_attributions.len() > 5 {
        framed = framed.footer(frame::more(cache.miss_attributions.len() - 5));
    }

    framed
}

fn kv_cache_pcts(cache: &CacheHitInfo) -> Option<(u8, Option<u8>, Option<u8>, Color)> {
    let lifetime_ratio = cache.hit_ratio()?;
    let lifetime_pct = ratio_pct(lifetime_ratio);
    let warm_pct = cache.optimal_ratio().map(ratio_pct);
    let last_pct = cache.last_ratio().map(ratio_pct);
    let last_optimal_pct = cache.last_optimal_ratio().map(ratio_pct);
    let health_pct = last_optimal_pct
        .or(last_pct)
        .or(warm_pct)
        .unwrap_or(lifetime_pct);
    Some((
        lifetime_pct,
        warm_pct,
        last_pct,
        kv_cache_optimal_color(health_pct),
    ))
}

/// Border headline: `KV cache 90% yield` (or `priming`).
fn render_kv_cache_title(cache: &CacheHitInfo) -> Line<'static> {
    let mut spans = vec![frame::label("KV cache ")];
    match kv_cache_pcts(cache) {
        Some((_, Some(warm), _, color)) => {
            spans.push(Span::styled(
                format!("{warm}%"),
                Style::default().fg(color).bold(),
            ));
            spans.push(frame::dim(" yield"));
        }
        Some((_, None, _, color)) => {
            spans.push(Span::styled("priming", Style::default().fg(color).bold()));
        }
        None => {}
    }
    Line::from(spans)
}

/// Body row: `last 94% · session 39%`.
fn render_kv_cache_rates_line(cache: &CacheHitInfo) -> Line<'static> {
    let Some((lifetime_pct, _, last_pct, color)) = kv_cache_pcts(cache) else {
        return Line::default();
    };
    let mut spans = Vec::new();
    if let Some(last_pct) = last_pct {
        spans.push(Span::styled(
            "last ",
            Style::default().fg(rgb(140, 140, 150)),
        ));
        spans.push(Span::styled(
            format!("{}%", last_pct),
            Style::default().fg(color).bold(),
        ));
        spans.push(Span::styled(" · ", Style::default().fg(rgb(80, 80, 90))));
    }
    spans.push(Span::styled(
        "session ",
        Style::default().fg(rgb(140, 140, 150)),
    ));
    spans.push(Span::styled(
        format!("{}%", lifetime_pct),
        Style::default().fg(color).bold(),
    ));
    Line::from(spans)
}

fn render_kv_cache_summary_line(cache: &CacheHitInfo) -> Vec<Line<'static>> {
    let Some(lifetime_ratio) = cache.hit_ratio() else {
        return Vec::new();
    };

    let lifetime_pct = ratio_pct(lifetime_ratio);
    let warm_pct = cache.optimal_ratio().map(ratio_pct);
    let last_pct = cache.last_ratio().map(ratio_pct);
    let last_optimal_pct = cache.last_optimal_ratio().map(ratio_pct);
    let health_pct = last_optimal_pct
        .or(last_pct)
        .or(warm_pct)
        .unwrap_or(lifetime_pct);
    let color = kv_cache_optimal_color(health_pct);

    let mut lines = Vec::new();

    // Line 1: header + yield/warm + session
    let mut spans1 = vec![Span::styled(
        "KV cache: ",
        Style::default().fg(rgb(180, 180, 190)).bold(),
    )];

    if let Some(warm_pct) = warm_pct {
        spans1.push(Span::styled(
            "yield ",
            Style::default().fg(rgb(140, 140, 150)),
        ));
        spans1.push(Span::styled(
            format!("{}%", warm_pct),
            Style::default().fg(color).bold(),
        ));
    } else {
        spans1.push(Span::styled(
            "priming",
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
    }
    spans1.push(Span::styled(" · ", Style::default().fg(rgb(80, 80, 90))));
    spans1.push(Span::styled(
        "session ",
        Style::default().fg(rgb(140, 140, 150)),
    ));
    spans1.push(Span::styled(
        format!("{}%", lifetime_pct),
        Style::default().fg(color).bold(),
    ));
    lines.push(Line::from(spans1));

    // Line 2: last-turn stats when present.
    if let Some(last_pct) = last_pct {
        let mut spans2 = vec![Span::styled("  ", Style::default())];
        spans2.push(Span::styled(
            "last ",
            Style::default().fg(rgb(140, 140, 150)),
        ));
        spans2.push(Span::styled(
            format!("{}%", last_pct),
            Style::default().fg(color).bold(),
        ));
        if let Some(last_optimal_pct) = last_optimal_pct {
            spans2.push(Span::styled(
                format!(" (optimal {}%)", last_optimal_pct),
                Style::default().fg(rgb(140, 140, 150)),
            ));
        }
        lines.push(Line::from(spans2));
    }

    lines
}

fn format_cache_turn_label(turn_number: usize, call_index: u16) -> String {
    if call_index <= 1 {
        format!("{}>", turn_number)
    } else {
        format!("{}.{}>", turn_number, call_index)
    }
}

fn ratio_pct(ratio: f32) -> u8 {
    (ratio * 100.0).round().clamp(0.0, 100.0) as u8
}

fn kv_cache_optimal_color(pct: u8) -> Color {
    match pct {
        0..=24 => rgb(255, 110, 110),
        25..=59 => rgb(255, 200, 100),
        60..=84 => rgb(140, 180, 255),
        _ => rgb(110, 210, 140),
    }
}

fn compact_token_count(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f32 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.0}k", tokens as f32 / 1_000.0)
    } else {
        tokens.to_string()
    }
}

/// Render ambient mode status widget
fn render_ambient_widget(data: &InfoWidgetData, inner: Rect) -> Framed {
    let Some(info) = &data.ambient_info else {
        return Framed::default();
    };
    if !info.show_widget {
        return Framed::default();
    }

    let mut lines: Vec<Line> = Vec::new();
    let dim = rgb(100, 100, 110);
    let label_color = rgb(140, 140, 150);
    let max_w = inner.width.saturating_sub(2) as usize;

    // Status line with icon
    let (icon, status_text, status_color) = match &info.status {
        AmbientStatus::Idle => ("○", "Idle".to_string(), rgb(120, 120, 130)),
        AmbientStatus::Running { detail } => {
            ("●", format!("Running: {}", detail), rgb(100, 200, 100))
        }
        AmbientStatus::Scheduled { .. } => {
            ("◐", "Waiting for next run".to_string(), rgb(140, 180, 255))
        }
        AmbientStatus::Paused { reason } => (
            "⏸",
            format!(
                "Paused: {}",
                truncate_smart(reason, inner.width.saturating_sub(12) as usize)
            ),
            rgb(255, 200, 100),
        ),
        AmbientStatus::Disabled if info.reminder_count > 0 => (
            "⏰",
            "Scheduled tasks active".to_string(),
            rgb(140, 180, 255),
        ),
        AmbientStatus::Disabled => ("○", "Not running".to_string(), dim),
    };

    // Status is the headline: it lives on the top border.
    let title = Line::from(vec![
        Span::styled(format!("{} ", icon), Style::default().fg(status_color)),
        Span::styled(status_text, Style::default().fg(rgb(180, 180, 190)).bold()),
    ]);

    // Scheduled tasks count
    let queue_count = if matches!(info.status, AmbientStatus::Disabled) && info.reminder_count > 0 {
        info.reminder_count
    } else {
        info.queue_count
    };
    let queue_preview = if matches!(info.status, AmbientStatus::Disabled) && info.reminder_count > 0
    {
        info.next_reminder_preview.as_ref()
    } else {
        info.next_queue_preview.as_ref()
    };

    if queue_count > 0 {
        let count_text =
            if matches!(info.status, AmbientStatus::Disabled) && info.reminder_count > 0 {
                if queue_count == 1 {
                    "1 scheduled task".to_string()
                } else {
                    format!("{} scheduled tasks", queue_count)
                }
            } else if queue_count == 1 {
                "1 task queued".to_string()
            } else {
                format!("{} tasks queued", queue_count)
            };
        let mut spans = vec![Span::styled(count_text, Style::default().fg(label_color))];
        if let Some(preview) = queue_preview {
            spans.push(Span::styled(
                truncate_smart(&format!(" ({})", preview), max_w.saturating_sub(16)),
                Style::default().fg(dim),
            ));
        }
        lines.push(Line::from(spans));
    }

    // Last run
    if let Some(ref ago) = info.last_run_ago {
        let mut spans = vec![Span::styled(
            format!("Ran {}", ago),
            Style::default().fg(label_color),
        )];
        if let Some(ref summary) = info.last_summary {
            let remaining = max_w.saturating_sub(4 + ago.len());
            if remaining > 5 {
                spans.push(Span::styled(
                    truncate_smart(&format!(" - {}", summary), remaining),
                    Style::default().fg(dim),
                ));
            }
        }
        lines.push(Line::from(spans));
    }

    // Next scheduled run
    let next_due = if matches!(info.status, AmbientStatus::Disabled) && info.reminder_count > 0 {
        info.next_reminder_wake.as_ref()
    } else {
        info.next_wake.as_ref()
    };

    if let Some(next) = next_due {
        let prefix = if matches!(info.status, AmbientStatus::Disabled) && info.reminder_count > 0 {
            "Next scheduled task"
        } else {
            "Next run"
        };
        lines.push(Line::from(vec![Span::styled(
            format!("{} {}", prefix, next),
            Style::default().fg(label_color),
        )]));
    }

    let mut framed = Framed::body(lines).title(title);

    // Budget meter rides the bottom border.
    if let Some(budget) = info.budget_percent {
        let pct = (budget * 100.0).round().clamp(0.0, 100.0) as u8;
        let bar_width = inner.width.saturating_sub(16).clamp(4, 10) as usize;
        let filled = ((budget * bar_width as f32).round() as usize).min(bar_width);
        let empty = bar_width.saturating_sub(filled);

        let bar_color = if pct < 20 {
            rgb(255, 100, 100)
        } else if pct <= 50 {
            rgb(255, 200, 100)
        } else {
            rgb(100, 200, 100)
        };

        framed = framed.footer_right(Line::from(vec![
            frame::dim("budget "),
            Span::styled("▰".repeat(filled), Style::default().fg(bar_color)),
            Span::styled("▱".repeat(empty), Style::default().fg(rgb(60, 60, 70))),
            Span::styled(format!(" {}%", pct), Style::default().fg(bar_color)),
        ]));
    }

    framed
}

/// Legacy render function - kept for backwards compatibility
/// Renders the first available widget at the given rect
#[deprecated(note = "Use render_all instead")]
pub fn render(frame: &mut Frame, rect: Rect, data: &InfoWidgetData) {
    // Just render as the first available widget type
    let available = data.available_widgets();
    if available.is_empty() {
        return;
    }

    // Create a temporary placement for the first widget
    let placement = WidgetPlacement {
        kind: available[0],
        rect,
        side: Side::Right,
        overview_layout: None,
    };
    render_single_widget(frame, &placement, data);
}

fn render_page(kind: InfoPageKind, data: &InfoWidgetData, inner: Rect) -> Vec<Line<'static>> {
    match kind {
        InfoPageKind::CompactOnly => render_sections(data, inner, None),
        InfoPageKind::TodosExpanded => {
            render_sections(data, inner, Some(InfoPageKind::TodosExpanded))
        }
        InfoPageKind::MemoryExpanded => {
            render_sections(data, inner, Some(InfoPageKind::MemoryExpanded))
        }
    }
}

fn render_sections(
    data: &InfoWidgetData,
    inner: Rect,
    _focus: Option<InfoPageKind>,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let w = inner.width as usize;

    // Line 1: Session name + count
    if data.session_count.is_some() || data.session_name.is_some() {
        let mut parts: Vec<String> = Vec::new();
        if let Some(sessions) = data.session_count {
            parts.push(format!(
                "{} session{}",
                sessions,
                if sessions == 1 { "" } else { "s" }
            ));
        }
        if let Some(name) = data.session_name.as_deref()
            && !name.trim().is_empty()
        {
            parts.push(name.to_string());
        }
        if !parts.is_empty() {
            let detail = truncate_smart(&parts.join(" · "), w.saturating_sub(2));
            lines.push(Line::from(Span::styled(
                detail,
                Style::default().fg(rgb(140, 140, 150)),
            )));
        }
    }

    // Line 2: Working directory + git branch (merged into one line).
    // Identity facts when the status line is pinned: dir, branch, and the
    // git counts all live there, so the widget skips straight to the
    // dirty-file detail rows.
    if !data.status_line_active {
        let dir_label = data
            .working_dir
            .as_deref()
            .filter(|d| !d.trim().is_empty())
            .and_then(crate::tui::session_facts::dir_label_short)
            .map(|label| truncate_smart(&label, w.saturating_sub(2)));

        let git_info = data.git_info.as_ref().filter(|g| !g.branch.is_empty());

        if dir_label.is_some() || git_info.is_some() {
            let mut spans: Vec<Span<'static>> = Vec::new();
            if let Some(dir) = dir_label {
                spans.push(Span::styled(dir, Style::default().fg(rgb(140, 140, 150))));
            }
            if let Some(info) = git_info {
                if !spans.is_empty() {
                    spans.push(Span::styled("  ", Style::default().fg(rgb(240, 160, 60))));
                }
                spans.push(Span::styled(" ", Style::default().fg(rgb(240, 160, 60))));
                let branch_w = w.saturating_sub(
                    spans
                        .iter()
                        .map(|s| s.content.chars().count())
                        .sum::<usize>(),
                );
                let branch_display =
                    truncate_smart(&info.branch, branch_w.saturating_sub(12).max(6));
                spans.push(Span::styled(
                    branch_display,
                    Style::default().fg(rgb(160, 160, 170)),
                ));
                if info.ahead > 0 {
                    spans.push(Span::styled(
                        format!(" ↑{}", info.ahead),
                        Style::default().fg(rgb(100, 200, 100)),
                    ));
                }
                if info.behind > 0 {
                    spans.push(Span::styled(
                        format!(" ↓{}", info.behind),
                        Style::default().fg(rgb(255, 140, 100)),
                    ));
                }
                if info.modified > 0 {
                    spans.push(Span::styled(
                        format!(" ~{}", info.modified),
                        Style::default().fg(rgb(240, 200, 80)),
                    ));
                }
                if info.staged > 0 {
                    spans.push(Span::styled(
                        format!(" +{}", info.staged),
                        Style::default().fg(rgb(100, 200, 100)),
                    ));
                }
                if info.untracked > 0 {
                    spans.push(Span::styled(
                        format!(" ?{}", info.untracked),
                        Style::default().fg(rgb(140, 140, 150)),
                    ));
                }
            }
            if !spans.is_empty() {
                lines.push(Line::from(spans));
            }
        }
    }

    // Changed files: the detail behind the status line's git counts.
    if data.git_info.as_ref().is_some_and(changes_has_data) {
        lines.extend(render_git_widget(data, inner));
    }

    // Separator between path/git line and provider/model section
    if data.model.is_some() {
        lines.push(dashed_separator(inner.width));
    }

    // Line 3: Provider + model (may be 1 or 2 lines)
    if data.model.is_some() {
        lines.extend(render_model_info(data, inner));
    }

    // Usage info (subscription bars with reset times only).
    // Placed right after model info, above the context bar.
    // CostBased/Copilot cost+tokens are in the cost line below.
    if let Some(info) = &data.usage_info
        && info.available
        && !matches!(
            info.provider,
            UsageProvider::CostBased | UsageProvider::Copilot
        )
    {
        lines.extend(render_usage_compact(
            info,
            inner.width,
            data.usage_display_used,
        ));
        lines.push(dashed_separator(inner.width));
    }

    // Line 4: Context occupancy with bar
    if let Some(info) = &data.context_info
        && info.total_chars > 0
    {
        lines.extend(render_context_compact(data, inner));
    } else if data.context_info_stale {
        lines.extend(render_context_compact(data, inner));
    }

    // Compaction status — part of the context block
    if let Some(info) = &data.compaction_info {
        lines.extend(render_compaction_compact(info, inner.width));
        lines.push(dashed_separator(inner.width));
    }

    // Line 5: Cost + tokens IN/OUT + avg t/s (always visible)
    let cost_line = render_cost_tokens_line(data, inner);
    lines.push(cost_line);

    // KV cache — right after token counts
    if let Some(cache) = data.cache_hit_info.as_ref() {
        lines.extend(render_kv_cache_summary_line(cache));
        lines.push(dashed_separator(inner.width));
    }

    // Supplementary fields below the core 5 lines:

    // Service tier, native compaction, etc.
    if data.has_model_supplementary_info() {
        lines.extend(render_model_info_supplementary(data, inner));
    }

    // MCP servers + skills — grouped above background tasks
    if !data.mcp_servers.is_empty() {
        lines.extend(render_mcp_servers_line(&data.mcp_servers, inner.width));
    }
    if !data.available_skills.is_empty() {
        lines.extend(render_skills_line(&data.available_skills, inner.width));
        lines.push(dashed_separator(inner.width));
    }

    // Background tasks
    if let Some(info) = &data.background_info
        && info.running_count > 0
    {
        lines.extend(render_background_compact(info));
    }

    // Swarm status (subagents, managed members, plan progress)
    // Always show: full widget when swarm active, "0 sessions" when inactive.
    if data.swarm_info.is_some() {
        let framed = swarm_background::render_swarm_widget(data, inner);
        lines.extend(framed.lines);
    } else {
        lines.push(Line::from(vec![
            Span::styled("🐝 ", Style::default().fg(rgb(255, 200, 100))),
            Span::styled("0 sessions", Style::default().fg(rgb(120, 120, 130))),
        ]));
    }

    // Ambient mode status (scheduled agent)
    if let Some(info) = &data.ambient_info
        && info.show_widget
    {
        let framed = render_ambient_widget(data, inner);
        lines.extend(framed.lines);
    }

    // Memory info — just before todos, shows count and recalls.
    // Recovered memories are rendered inline below the count line,
    // using 2x panel width for the content area.
    if let Some(info) = &data.memory_info {
        let label = if info.disabled {
            "Memory disabled".to_string()
        } else {
            format_memory_count(info.total_count)
        };
        let mut style = Style::default();
        if info.disabled {
            style = style.fg(rgb(120, 120, 130)).add_modifier(Modifier::DIM);
        } else {
            style = style.fg(rgb(210, 210, 220));
        };
        let mut spans = vec![
            Span::styled("🧠 ", Style::default().fg(rgb(200, 150, 255))),
            Span::styled(label, style),
        ];
        // Append recalls count if available.
        if let Some(activity) = info.activity.as_ref() {
            let recalls = activity
                .recent_events
                .iter()
                .filter(|e| {
                    matches!(
                        &e.kind,
                        MemoryEventKind::MemoryInjected { .. }
                            | MemoryEventKind::MemorySurfaced { .. }
                    )
                })
                .count();
            if recalls > 0 {
                spans.push(Span::styled(
                    format!(
                        " · {} recall{}",
                        recalls,
                        if recalls == 1 { "" } else { "s" }
                    ),
                    Style::default().fg(rgb(140, 210, 255)),
                ));
            }
        }
        lines.push(Line::from(spans));

        // Render recovered memories inline, using 2x panel width.
        if data.has_recovered_memories() {
            let recovered = render_recovered_memories_widget(data, inner);
            lines.extend(recovered);
        }
    }

    // Todos
    if !data.todos.is_empty() {
        lines.extend(render_todos_expanded(data, inner));
    }

    lines
}

fn render_mcp_servers_line(servers: &[(String, usize)], width: u16) -> Vec<Line<'static>> {
    let w = width as usize;
    let full_parts: Vec<String> = servers
        .iter()
        .map(|(name, count)| {
            if *count > 0 {
                format!("{} ({} tools)", name, count)
            } else {
                format!("{} (...)", name)
            }
        })
        .collect();
    let full = format!("mcp: {}", full_parts.join(", "));
    if full.chars().count() <= w {
        return vec![Line::from(Span::styled(
            full,
            Style::default().fg(rgb(100, 180, 220)),
        ))];
    }
    // Try compact single line
    let short_parts: Vec<String> = servers
        .iter()
        .map(|(name, count)| {
            if *count > 0 {
                format!("{}({})", name, count)
            } else {
                format!("{}(…)", name)
            }
        })
        .collect();
    let short = format!("mcp: {}", short_parts.join(" "));
    if short.chars().count() <= w {
        return vec![Line::from(Span::styled(
            short,
            Style::default().fg(rgb(100, 180, 220)),
        ))];
    }
    // Multi-line: header + one server per line
    let mut lines = vec![Line::from(Span::styled(
        format!("mcp: {} servers", servers.len()),
        Style::default().fg(rgb(100, 180, 220)),
    ))];
    for (name, count) in servers {
        let entry = if *count > 0 {
            format!("  {} ({} tools)", name, count)
        } else {
            format!("  {} (...)", name)
        };
        let entry = truncate_smart(&entry, w);
        lines.push(Line::from(Span::styled(
            entry,
            Style::default().fg(rgb(100, 180, 220)),
        )));
    }
    lines
}

/// Format memory count label (e.g. "0 memories", "1 memory", "5 memories").
fn format_memory_count(total: usize) -> String {
    if total == 1 {
        "1 memory".to_string()
    } else {
        format!("{total} memories")
    }
}

/// Render the cost + tokens IN/OUT + t/s line (line 5, always visible).
/// Shows live t/s while streaming, or avg t/s when idle.
fn render_cost_tokens_line(data: &InfoWidgetData, inner: Rect) -> Line<'static> {
    let _w = inner.width as usize;
    let mut spans: Vec<Span<'static>> = Vec::new();

    // Cost + tokens with arrows: ↑ sent (output), ↓ received (input)
    if let Some(info) = &data.usage_info
        && info.available
    {
        match info.provider {
            UsageProvider::CostBased => {
                spans.push(Span::styled("💰 ", Style::default().fg(rgb(140, 180, 255))));
                spans.push(Span::styled(
                    format!("${:.4}", info.total_cost),
                    Style::default().fg(rgb(180, 180, 190)).bold(),
                ));
                spans.push(Span::styled(
                    format!(
                        "  ↓{} ↑{} tok",
                        compact_token_count(info.input_tokens),
                        compact_token_count(info.output_tokens),
                    ),
                    Style::default().fg(rgb(140, 140, 150)),
                ));
            }
            UsageProvider::Copilot => {
                spans.push(Span::styled("💰 ", Style::default().fg(rgb(140, 180, 255))));
                spans.push(Span::styled(
                    format!(
                        "$NA  ↓{} ↑{} tok",
                        compact_token_count(info.input_tokens),
                        compact_token_count(info.output_tokens),
                    ),
                    Style::default().fg(rgb(140, 140, 150)),
                ));
            }
            _ => {
                // OAuth subscription providers: show cost if available,
                // otherwise just token counts. Subscription bars are shown
                // separately by render_usage_compact below.
                if info.total_cost > 0.0 {
                    spans.push(Span::styled(
                        format!("${:.4}", info.total_cost),
                        Style::default().fg(rgb(180, 180, 190)).bold(),
                    ));
                    spans.push(Span::styled("  ", Style::default()));
                }
                spans.push(Span::styled(
                    format!(
                        "↓{} ↑{} tok",
                        compact_token_count(info.input_tokens),
                        compact_token_count(info.output_tokens),
                    ),
                    Style::default().fg(rgb(140, 140, 150)),
                ));
            }
        }
    } else {
        spans.push(Span::styled("$NA", Style::default().fg(rgb(140, 140, 150))));
    }

    // t/s: always show rolling average over last 5 turns (even if 0),
    // unless the user hid every t/s readout.
    if !data.hide_tps {
        if let Some(avg) = data.avg_tokens_per_second
            && avg.is_finite()
        {
            spans.push(Span::styled(
                format!("  ⌀{:.1} t/s", avg.max(0.0)),
                Style::default().fg(rgb(140, 180, 255)),
            ));
        } else {
            spans.push(Span::styled(
                "  ⌀0 t/s",
                Style::default().fg(rgb(140, 180, 255)),
            ));
        }
    }

    Line::from(spans)
}

/// Test-only access to the cost line renderer so app-level tests can assert
/// the rendered text without going through the whole widget draw path.
#[cfg(test)]
pub(crate) fn render_cost_tokens_line_for_test(data: &InfoWidgetData) -> Line<'static> {
    render_cost_tokens_line(data, Rect::new(0, 0, 80, 1))
}

// ---------------------------------------------------------------------------
// MCP servers & skills compact lines
// ---------------------------------------------------------------------------

fn render_compaction_compact(info: &CompactionInfo, width: u16) -> Vec<Line<'static>> {
    let status = if info.is_compacting {
        "compacting"
    } else {
        "compacted"
    };
    let summary_tokens = (info.summary_chars / crate::compaction::CHARS_PER_TOKEN)
        .max(usize::from(info.summary_chars > 0));
    let color = if info.is_compacting {
        rgb(255, 220, 140)
    } else {
        rgb(110, 210, 140)
    };
    let w = width as usize;

    // Line 1: status + mode
    let line1 = format!("CMP {} {}", status, info.mode);

    // Line 2: detail stats
    let detail = format!(
        "{} old · {} active · ~{} tok",
        info.compacted_messages, info.active_messages, summary_tokens
    );
    let detail = truncate_smart(&detail, w);

    vec![
        Line::from(Span::styled(line1, Style::default().fg(color))),
        Line::from(Span::styled(
            format!("  {}", detail),
            Style::default().fg(color),
        )),
    ]
}

fn render_skills_line(skills: &[String], _width: u16) -> Vec<Line<'static>> {
    vec![Line::from(Span::styled(
        format!("skills: {} loaded", skills.len()),
        Style::default().fg(rgb(100, 180, 220)),
    ))]
}

// ---------------------------------------------------------------------------
// Tips widget - rotating helpful tips and keyboard shortcuts
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "info_widget_tests.rs"]
mod tests;

fn format_event_for_expanded(
    event: &MemoryEvent,
    max_width: usize,
) -> (&'static str, String, Color) {
    match &event.kind {
        MemoryEventKind::EmbeddingComplete { latency_ms, hits } => (
            "→",
            truncate_with_ellipsis(&format!("{} hits ({}ms)", hits, latency_ms), max_width),
            rgb(140, 180, 255),
        ),
        MemoryEventKind::SidecarRelevant { memory_preview } => (
            "✓",
            truncate_with_ellipsis(memory_preview, max_width),
            rgb(100, 200, 100),
        ),
        MemoryEventKind::MemorySurfaced { memory_preview } => (
            "★",
            truncate_with_ellipsis(memory_preview, max_width),
            rgb(255, 220, 100),
        ),
        MemoryEventKind::MemoryInjected {
            count,
            prompt_chars,
            items,
            ..
        } => {
            let plural = if *count == 1 { "memory" } else { "memories" };
            let detail = items
                .first()
                .map(|item| format!(" [{}]", item.section))
                .unwrap_or_default();
            (
                "↳",
                truncate_with_ellipsis(
                    &format!("{} {} ({}c){}", count, plural, prompt_chars, detail),
                    max_width,
                ),
                rgb(140, 210, 255),
            )
        }
        MemoryEventKind::MaintenanceComplete { latency_ms } => (
            "🌿",
            truncate_with_ellipsis(&format!("maintained ({}ms)", latency_ms), max_width),
            rgb(120, 220, 180),
        ),
        MemoryEventKind::ExtractionStarted { reason } => (
            "🧠",
            truncate_with_ellipsis(&format!("extracting: {}", reason), max_width),
            rgb(200, 150, 255),
        ),
        MemoryEventKind::ExtractionComplete { count } => (
            "✓",
            truncate_with_ellipsis(&format!("saved {} memories", count), max_width),
            rgb(100, 200, 100),
        ),
        MemoryEventKind::Error { message } => (
            "!",
            truncate_with_ellipsis(message, max_width),
            rgb(255, 100, 100),
        ),
        MemoryEventKind::ToolRemembered {
            content, category, ..
        } => (
            "💾",
            truncate_with_ellipsis(&format!("[{}] {}", category, content), max_width),
            rgb(100, 200, 100),
        ),
        MemoryEventKind::ToolRecalled { query, count } => (
            "🔍",
            truncate_with_ellipsis(&format!("{} found for '{}'", count, query), max_width),
            rgb(140, 180, 255),
        ),
        MemoryEventKind::ToolForgot { id } => (
            "🗑\u{fe0f}",
            truncate_with_ellipsis(id, max_width),
            rgb(255, 170, 100),
        ),
        MemoryEventKind::ToolTagged { id, tags } => (
            "🏷\u{fe0f}",
            truncate_with_ellipsis(&format!("{} +{}", id, tags), max_width),
            rgb(140, 200, 255),
        ),
        MemoryEventKind::ToolLinked { from, to } => (
            "🔗",
            truncate_with_ellipsis(&format!("{} → {}", from, to), max_width),
            rgb(200, 180, 255),
        ),
        MemoryEventKind::ToolListed { count } => {
            ("📋", format!("{} memories", count), rgb(140, 140, 150))
        }
        _ => ("·", String::new(), rgb(100, 100, 110)),
    }
}

fn render_context_compact(data: &InfoWidgetData, inner: Rect) -> Vec<Line<'static>> {
    if data.context_info_stale {
        return vec![Line::from(vec![
            Span::styled("Context ", Style::default().fg(rgb(140, 140, 150))),
            Span::styled("updating...", Style::default().fg(rgb(220, 180, 80))),
        ])];
    }
    let Some(info) = &data.context_info else {
        return Vec::new();
    };
    if info.total_chars == 0 && data.observed_context_tokens.is_none() {
        return Vec::new();
    }

    let used_tokens = data
        .observed_context_tokens
        .map(|t| t as usize)
        .unwrap_or_else(|| info.estimated_tokens());
    let limit_tokens = data.context_limit.unwrap_or(DEFAULT_CONTEXT_LIMIT).max(1);
    let label = if data.is_compacting {
        "Context📦"
    } else {
        "Context"
    };

    vec![render_context_usage_line(
        label,
        used_tokens,
        limit_tokens,
        inner.width,
    )]
}
