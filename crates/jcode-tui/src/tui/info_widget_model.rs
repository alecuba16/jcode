//! Runtime widget: the detail layer behind the status line's model segment.
//!
//! The overscroll status line owns the identity facts: model, reasoning
//! effort, provider, auth method, directory, and branch. This widget never
//! repeats those. It shows only the runtime facts the line has no room for:
//! the OpenAI service tier, the upstream route, the transport, live
//! throughput, and which session/swarm this is.

use super::InfoWidgetData;
use super::frame::{self, Framed};
use super::text::truncate_smart;
use crate::tui::color_support::rgb;
use ratatui::prelude::*;

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

pub(super) fn model_info_supplementary_height(data: &InfoWidgetData) -> u16 {
    u16::from(data.native_compaction_mode.is_some())
}

/// Content rows the Runtime widget renders, mirroring [`render_model_widget`].
pub(super) fn runtime_height(data: &InfoWidgetData) -> u16 {
    runtime_rows(data).len() as u16
}

/// Border layout: ` Runtime ` top-left, live throughput bottom-right (it is
/// the one value that changes every turn), the rest as icon rows.
pub(super) fn render_model_widget(data: &InfoWidgetData, inner: Rect) -> Framed {
    let max_len = inner.width as usize;
    let tps = throughput(data);
    let lines: Vec<Line<'static>> = runtime_rows(data)
        .into_iter()
        .filter(|row| tps.is_none() || row.icon != TPS_ICON)
        .map(|row| row.into_line(max_len))
        .collect();
    let mut framed = Framed::body(lines).title(frame::label("Runtime"));
    if let Some(tps) = tps {
        framed = framed.footer_right(Line::from(vec![
            Span::styled(
                format!("{TPS_ICON} "),
                Style::default().fg(rgb(140, 180, 255)),
            ),
            frame::dim(format!("{tps:.0} tok/s")),
        ]));
    }
    framed
}

/// Overview section: every runtime fact as a row, no border.
pub(super) fn render_model_info(data: &InfoWidgetData, inner: Rect) -> Vec<Line<'static>> {
    let max_len = inner.width.saturating_sub(2) as usize;
    runtime_rows(data)
        .into_iter()
        .map(|row| row.into_line(max_len))
        .collect()
}

const TPS_ICON: &str = "⏱";

/// Throughput worth showing on the border, only when another row keeps the
/// body non-empty (a lone tok/s row stays in the body).
fn throughput(data: &InfoWidgetData) -> Option<f32> {
    let tps = data
        .tokens_per_second
        .filter(|t| t.is_finite() && *t > 0.1)?;
    (runtime_rows(data).len() > 1).then_some(tps)
}

struct RuntimeRow {
    icon: &'static str,
    icon_color: Color,
    text: String,
    text_color: Color,
}

impl RuntimeRow {
    fn into_line(self, max_len: usize) -> Line<'static> {
        Line::from(vec![
            Span::styled(
                format!("{} ", self.icon),
                Style::default().fg(self.icon_color),
            ),
            Span::styled(
                truncate_smart(&self.text, max_len.saturating_sub(2)),
                Style::default().fg(self.text_color),
            ),
        ])
    }
}

fn runtime_rows(data: &InfoWidgetData) -> Vec<RuntimeRow> {
    let accent = rgb(140, 180, 255);
    let muted = rgb(140, 140, 150);
    let mut rows = Vec::new();

    // Selected (default) model + effort first: the primary identity of the
    // session. The home icon marks the default model the session runs on;
    // the status line shows a shortened form, the panel carries the full name.
    if let Some(model) = non_empty(data.model.as_deref()) {
        let mut text = model.to_string();
        if let Some(effort) = non_empty(data.reasoning_effort.as_deref()) {
            text.push_str(&format!(" · {effort}"));
        }
        rows.push(RuntimeRow {
            icon: "🏠",
            icon_color: rgb(255, 135, 200),
            text,
            text_color: rgb(220, 220, 230),
        });
    }

    // Agent model overrides from config. Only shown when actually set; the
    // inherit default (None) is silent to keep the panel compact.
    if let Some(model) = non_empty(data.swarm_model_override.as_deref()) {
        let mut text = model.to_string();
        if let Some(effort) = non_empty(data.swarm_model_effort.as_deref()) {
            text.push_str(&format!(" · {effort}"));
        }
        rows.push(RuntimeRow {
            icon: "🐝",
            icon_color: rgb(255, 200, 100),
            text,
            text_color: muted,
        });
    }
    if let Some(model) = non_empty(data.memory_model_override.as_deref()) {
        let mut text = model.to_string();
        if let Some(effort) = non_empty(data.memory_model_effort.as_deref()) {
            text.push_str(&format!(" · {effort}"));
        }
        rows.push(RuntimeRow {
            icon: "🧠",
            icon_color: rgb(200, 150, 255),
            text,
            text_color: muted,
        });
    }

    let is_openai = data
        .provider_name
        .as_deref()
        .is_some_and(|provider| provider.trim().to_ascii_lowercase().starts_with("openai"));
    if is_openai && let Some(tier) = data.service_tier.as_deref().and_then(short_service_tier) {
        rows.push(RuntimeRow {
            icon: "⚡",
            icon_color: rgb(200, 140, 255),
            text: format!("{tier} tier"),
            text_color: rgb(200, 140, 255),
        });
    }

    if let Some(upstream) = non_empty(data.upstream_provider.as_deref()) {
        rows.push(RuntimeRow {
            icon: "☁",
            icon_color: accent,
            text: format!("via {upstream}"),
            text_color: rgb(220, 190, 120),
        });
    }

    if let Some(connection) = non_empty(data.connection_type.as_deref()) {
        rows.push(RuntimeRow {
            icon: "↔",
            icon_color: accent,
            text: connection.to_lowercase(),
            text_color: muted,
        });
    }

    if let Some(tps) = data.tokens_per_second
        && tps.is_finite()
        && tps > 0.1
    {
        rows.push(RuntimeRow {
            icon: TPS_ICON,
            icon_color: accent,
            text: format!("{tps:.0} tok/s"),
            text_color: muted,
        });
    }

    rows
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|s| !s.is_empty())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::info_widget::{AuthMethod, InfoWidgetData};

    fn data() -> InfoWidgetData {
        InfoWidgetData {
            model: Some("gpt-5-codex".to_string()),
            reasoning_effort: Some("high".to_string()),
            service_tier: Some("priority".to_string()),
            provider_name: Some("openai".to_string()),
            auth_method: AuthMethod::OpenAIOAuth,
            working_dir: Some("/home/me/jcode".to_string()),
            ..Default::default()
        }
    }

    fn text(lines: Vec<Line<'static>>) -> String {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
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
    fn runtime_widget_shows_model_and_effort_plus_runtime_detail() {
        let mut d = data();
        d.connection_type = Some("websocket".to_string());
        d.tokens_per_second = Some(61.7);
        let out = text(render_model_widget(&d, Rect::new(0, 0, 30, 8)).all_lines());
        // The panel leads with the selected model + effort (user-requested),
        // then the runtime detail. Identity facts that only the status line
        // owns (provider name, auth, dir) stay out.
        for owned in ["openai", "OAuth", "jcode"] {
            assert!(
                !out.contains(owned),
                "{owned:?} belongs to the status line: {out}"
            );
        }
        assert!(out.contains("gpt-5-codex · high"), "{out}");
        assert!(out.contains("fast tier"), "{out}");
        assert!(out.contains("websocket"), "{out}");
        assert!(out.contains("62 tok/s"), "{out}");
    }

    #[test]
    fn service_tier_only_for_openai() {
        let mut d = data();
        d.provider_name = Some("deepseek".to_string());
        assert!(
            !text(render_model_widget(&d, Rect::new(0, 0, 30, 8)).all_lines()).contains("tier")
        );
        for tier in [None, Some("off"), Some("default")] {
            let mut d = data();
            d.service_tier = tier.map(str::to_string);
            assert!(
                !text(render_model_widget(&d, Rect::new(0, 0, 30, 8)).all_lines()).contains("tier"),
                "tier {tier:?} must stay hidden"
            );
        }
    }

    #[test]
    fn identity_only_session_renders_just_the_model_row() {
        let mut d = data();
        d.service_tier = None;
        // With no runtime detail, the selected model + effort row still
        // renders: the panel always shows what model the session runs on.
        assert_eq!(runtime_height(&d), 1);
    }

    #[test]
    fn height_matches_rendered_rows() {
        let mut d = data();
        d.upstream_provider = Some("fireworks".to_string());
        let framed = render_model_widget(&d, Rect::new(0, 0, 30, 8));
        assert_eq!(framed.lines.len() as u16, runtime_height(&d));
        assert!(text(framed.all_lines()).contains("via fireworks"));
    }

    #[test]
    fn overview_shows_runtime_metadata() {
        let rect = Rect::new(0, 0, 40, 8);
        let mut data = data();
        data.provider_name = Some("openai".to_string());

        let out = text(render_model_info(&data, rect));
        // The status line owns the identity facts (model, effort, provider);
        // the Overview rows carry the runtime facts behind them.
        assert!(out.contains("fast tier"), "{out}");
    }

    #[test]
    fn openai_fast_badge_follows_service_tier_not_model_name() {
        let rect = Rect::new(0, 0, 40, 8);
        let mut data = data();
        data.provider_name = Some("OpenAI".to_string());
        data.model = Some("gpt-future-model".to_string());

        for (tier, badge) in [(Some("priority"), "fast tier"), (Some("flex"), "flex tier")] {
            data.service_tier = tier.map(str::to_string);
            assert!(text(render_model_info(&data, rect)).contains(badge));
        }
        for tier in [None, Some("off"), Some("default")] {
            data.service_tier = tier.map(str::to_string);
            assert!(!text(render_model_info(&data, rect)).contains("tier"));
        }
    }

    #[test]
    fn non_openai_provider_hides_openai_service_tier() {
        let rect = Rect::new(0, 0, 40, 8);
        let mut data = data();
        data.model = Some("deepseek-v4-flash".to_string());
        data.provider_name = Some("deepseek".to_string());

        assert!(!text(render_model_info(&data, rect)).contains("tier"));
    }
}
