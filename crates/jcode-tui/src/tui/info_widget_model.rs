use super::text::truncate_chars;
use super::{AuthMethod, InfoWidgetData};
use crate::tui::color_support::rgb;
use ratatui::prelude::*;

pub(super) fn render_model_info(data: &InfoWidgetData, inner: Rect) -> Vec<Line<'static>> {
    let Some(model) = &data.model else {
        return Vec::new();
    };

    let short_name = crate::tui::session_facts::pretty_model(model);
    let max_len = inner.width.saturating_sub(2) as usize;
    let mut lines: Vec<Line> = Vec::new();

    // Line 1: Provider (bold white) + ⚡[tier] + upstream provider if present
    if let Some(provider) = data
        .provider_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let mut provider_spans = vec![Span::styled(
            provider.to_lowercase(),
            Style::default().fg(rgb(255, 255, 255)).bold(),
        )];
        // Service tier right after provider — OpenAI-only badge; other
        // providers do not report OpenAI service tiers and must not show it.
        let is_openai = provider.to_ascii_lowercase().starts_with("openai");
        if is_openai && let Some(tier) = data.service_tier.as_deref().and_then(short_service_tier) {
            provider_spans.push(Span::styled(
                format!(" ⚡[{tier}]"),
                Style::default().fg(rgb(200, 140, 255)).bold(),
            ));
        }
        if let Some(upstream) = data.upstream_provider.as_deref().map(str::trim)
            && !upstream.is_empty()
        {
            provider_spans.push(Span::styled(
                " -> ",
                Style::default().fg(rgb(100, 100, 110)),
            ));
            provider_spans.push(Span::styled(
                upstream.to_string(),
                Style::default().fg(rgb(220, 190, 120)),
            ));
        }
        lines.push(Line::from(provider_spans));
    }

    // Line 2: (effort) + model name (blue) + native compaction
    let mut spans: Vec<Span> = Vec::new();

    // Effort indicator before model name
    if let Some(effort) = data
        .reasoning_effort
        .as_deref()
        .and_then(short_reasoning_effort)
    {
        spans.push(Span::styled(
            format!("({effort}) "),
            Style::default().fg(rgb(255, 200, 100)),
        ));
    }

    let model_text = if short_name.chars().count() > max_len {
        format!(
            "{}...",
            truncate_chars(&short_name, max_len.saturating_sub(3))
        )
    } else {
        short_name
    };
    spans.push(Span::styled(
        model_text,
        Style::default().fg(rgb(140, 180, 255)),
    ));

    if let Some(mode) = &data.native_compaction_mode {
        let label = if let Some(tokens) = data.native_compaction_threshold_tokens {
            format!("native {} @ {}k", mode, tokens / 1000)
        } else {
            format!("native {}", mode)
        };
        spans.push(Span::styled(" ", Style::default()));
        spans.push(Span::styled(label, Style::default().fg(rgb(120, 210, 230))));
    }

    lines.push(Line::from(spans));

    // Auth method on a separate line if present, with connection type
    // appended in brackets (light gray) when available.
    if data.auth_method != AuthMethod::Unknown {
        let (icon, label, color) = match data.auth_method {
            AuthMethod::ApiKey => ("🔑", "API Key", rgb(180, 180, 190)),
            AuthMethod::AnthropicOAuth => ("🔐", "OAuth", rgb(255, 160, 100)),
            AuthMethod::AnthropicApiKey => ("🔑", "API Key", rgb(180, 180, 190)),
            AuthMethod::OpenAIOAuth => ("🔐", "OAuth", rgb(100, 200, 180)),
            AuthMethod::OpenAIApiKey => ("🔑", "API Key", rgb(180, 180, 190)),
            AuthMethod::OpenRouterApiKey => ("🔑", "API Key", rgb(140, 180, 255)),
            AuthMethod::OpenCodeApiKey => ("🔑", "API Key", rgb(140, 180, 255)),
            AuthMethod::CopilotOAuth => ("🔐", "OAuth", rgb(110, 200, 140)),
            AuthMethod::GeminiOAuth => ("🔐", "OAuth", rgb(120, 190, 255)),
            AuthMethod::Unknown => unreachable!(),
        };
        let mut auth_spans = vec![
            Span::styled(format!("{} ", icon), Style::default().fg(color)),
            Span::styled(label, Style::default().fg(rgb(140, 140, 150))),
        ];
        if let Some(ref upstream) = data.upstream_provider {
            auth_spans.push(Span::styled(
                " via ",
                Style::default().fg(rgb(100, 100, 110)),
            ));
            auth_spans.push(Span::styled(
                upstream.clone(),
                Style::default().fg(rgb(200, 180, 100)),
            ));
        }
        if let Some(connection) = data
            .connection_type
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            auth_spans.push(Span::styled(
                format!(" [{}]", connection.to_lowercase()),
                Style::default().fg(rgb(100, 100, 110)),
            ));
        }
        lines.push(Line::from(auth_spans));
    }

    // Note: session count/name is now shown in line 1 of the info panel
    // (render_sections), not here, to avoid duplication.

    lines
}

/// Render only the supplementary model info not shown in the status line:
/// native compaction mode. Service tier is already shown inline on the
/// model name line. Used when `status_line_active` suppresses the full
/// `render_model_info`.
pub(super) fn render_model_info_supplementary(
    data: &InfoWidgetData,
    _inner: Rect,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Native compaction mode.
    if let Some(mode) = &data.native_compaction_mode {
        let label = if let Some(tokens) = data.native_compaction_threshold_tokens {
            format!("native {} @ {}k", mode, tokens / 1000)
        } else {
            format!("native {}", mode)
        };
        lines.push(Line::from(vec![
            Span::styled("📦 ", Style::default().fg(rgb(120, 210, 230))),
            Span::styled(label, Style::default().fg(rgb(120, 210, 230))),
        ]));
    }

    lines
}

#[allow(dead_code)] // Retained for status-bar model rendering; currently unused after a layout change.
pub(crate) fn shorten_model_name(model: &str) -> String {
    if model.contains("claude") {
        if model.contains("opus-4-5") || model.contains("opus-4.5") {
            return "opus-4.5".to_string();
        }
        if model.contains("sonnet-4") {
            return "sonnet-4".to_string();
        }
        if model.contains("sonnet-3-5") || model.contains("sonnet-3.5") {
            return "sonnet-3.5".to_string();
        }
        if model.contains("haiku") {
            return "haiku".to_string();
        }
        if let Some(idx) = model.find("claude-") {
            let rest = &model[idx + 7..];
            if let Some(end) = rest.find('-') {
                return rest[..end].to_string();
            }
        }
    }

    if model.contains("gpt")
        && let Some(start) = model.find("gpt-")
    {
        let rest = &model[start..];
        let parts: Vec<&str> = rest.splitn(3, '-').collect();
        if parts.len() >= 2 {
            return format!("{}-{}", parts[0], parts[1]);
        }
    }

    if model.len() > 15 {
        format!("{}…", crate::util::truncate_str(model, 14))
    } else {
        model.to_string()
    }
}

fn short_reasoning_effort(effort: &str) -> Option<&str> {
    let effort = effort.trim();
    if effort.is_empty() {
        return None;
    }
    Some(match effort {
        "max" => "max",
        "xhigh" => "xhi",
        "high" => "hi",
        "medium" => "med",
        "low" => "lo",
        "none" => "∅",
        "swarm" => "swarm",
        "swarm-deep" => "swarm+",
        other => other,
    })
}

fn short_service_tier(service_tier: &str) -> Option<&str> {
    let service_tier = service_tier.trim();
    if service_tier.is_empty() || service_tier == "off" || service_tier == "default" {
        return None;
    }
    Some(match service_tier {
        "priority" => "fast",
        "flex" => "flex",
        other => other,
    })
}

/// Render a directory path home-relative (e.g. `/home/me/x` -> `~/x`).
fn home_relative_dir(path: &str) -> String {
    crate::tui::session_facts::dir_label(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::info_widget::InfoWidgetData;

    fn data() -> InfoWidgetData {
        InfoWidgetData {
            todos: Vec::new(),
            todo_goals: Vec::new(),
            todos_are_swarm_plan: false,
            context_info: None,
            context_info_stale: false,
            queue_mode: None,
            context_limit: None,
            model: Some("gpt-5-codex".to_string()),
            reasoning_effort: Some("high".to_string()),
            service_tier: Some("priority".to_string()),
            native_compaction_mode: None,
            native_compaction_threshold_tokens: None,
            session_count: None,
            session_name: None,
            working_dir: None,
            client_count: None,
            memory_info: None,
            swarm_info: None,
            background_info: None,
            usage_info: None,
            usage_display_used: false,
            tokens_per_second: None,
            avg_tokens_per_second: None,
            provider_name: None,
            auth_method: crate::tui::info_widget::AuthMethod::Unknown,
            upstream_provider: None,
            connection_type: None,
            diagrams: Vec::new(),
            workspace_rows: Vec::new(),
            workspace_animation_tick: 0,
            ambient_info: None,
            observed_context_tokens: None,
            cache_hit_info: None,
            compaction_info: None,
            is_compacting: false,
            git_info: None,
            status_line_active: false,
            status_line_pinned: false,
            mcp_servers: Vec::new(),
            available_skills: Vec::new(),
        }
    }

    fn first_line_text(lines: Vec<Line<'static>>) -> String {
        lines
            .into_iter()
            .next()
            .expect("first model line")
            .spans
            .into_iter()
            .map(|span| span.content.into_owned())
            .collect::<String>()
    }

    #[test]
    fn overview_shows_runtime_metadata() {
        let rect = Rect::new(0, 0, 40, 8);
        let mut data = data();
        data.provider_name = Some("openai".to_string());

        let lines = render_model_info(&data, rect);
        let overview = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join(" | ");

        assert!(overview.contains("(hi)"));
        assert!(overview.contains("[fast]"));
    }

    #[test]
    fn openai_fast_badge_follows_service_tier_not_model_name() {
        let rect = Rect::new(0, 0, 40, 8);
        let mut data = data();
        data.provider_name = Some("OpenAI".to_string());
        data.model = Some("gpt-future-model".to_string());

        for (tier, badge) in [(Some("priority"), "[fast]"), (Some("flex"), "[flex]")] {
            data.service_tier = tier.map(str::to_string);
            assert!(first_line_text(render_model_info(&data, rect)).contains(badge));
            assert!(first_line_text(render_model_info(&data, rect)).contains(badge));
        }
        for tier in [None, Some("off"), Some("default")] {
            data.service_tier = tier.map(str::to_string);
            assert!(!first_line_text(render_model_info(&data, rect)).contains("[fast]"));
            assert!(!first_line_text(render_model_info(&data, rect)).contains("[fast]"));
        }
    }

    #[test]
    fn non_openai_provider_hides_openai_service_tier() {
        let rect = Rect::new(0, 0, 40, 8);
        let mut data = data();
        data.model = Some("deepseek-v4-flash".to_string());
        data.provider_name = Some("deepseek".to_string());

        assert!(!first_line_text(render_model_info(&data, rect)).contains("[fast]"));
        assert!(!first_line_text(render_model_info(&data, rect)).contains("[fast]"));
    }
}
