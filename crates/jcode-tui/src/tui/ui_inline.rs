use super::*;
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use unicode_width::UnicodeWidthStr;

fn inline_view_display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// Test-only wrappers: the helpers are private, but coverage tests verify
/// every truncation and height branch directly.
#[cfg(test)]
pub(super) fn decision_truncate_display_for_test(text: &str, max_width: usize) -> String {
    decision_truncate_display(text, max_width)
}

#[cfg(test)]
pub(super) fn inline_ui_height_for_test(app: &dyn TuiState) -> u16 {
    inline_ui_height(app)
}

fn decision_truncate_display(text: &str, max_width: usize) -> String {
    if inline_view_display_width(text) <= max_width {
        return text.to_string();
    }
    if max_width == 0 {
        return String::new();
    }
    if max_width == 1 {
        return "…".to_string();
    }
    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let ch_width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + ch_width + 1 > max_width {
            break;
        }
        out.push(ch);
        used += ch_width;
    }
    out.push('…');
    out
}

pub(super) fn inline_ui_height(app: &dyn TuiState) -> u16 {
    match app.inline_ui_state() {
        Some(crate::tui::InlineUiStateRef::Interactive(picker)) => {
            // Model choices replace the command suggestions, without reflowing chat.
            if picker.kind == crate::tui::PickerKind::Model {
                return 0;
            }
            let visible_rows = picker.filtered.len() as u16;
            let rows_needed = visible_rows + 1 + 2; // header + rounded border
            rows_needed.min(20)
        }
        Some(crate::tui::InlineUiStateRef::View(view)) => {
            let visible_rows = view.lines.len().max(1) as u16;
            let rows_needed = visible_rows + 1 + 2; // header + rounded border
            rows_needed.min(10)
        }
        Some(crate::tui::InlineUiStateRef::Decision(decision)) => {
            // question header + option rows (+ one row per option detail) +
            // the free-form "Your answer" row + hint row + rounded border.
            // All options must stay visible even when details push the
            // chooser tall; the hint row is the first to be clipped in a very
            // short terminal.
            let detail_rows = decision
                .options
                .iter()
                .filter(|(_, detail)| detail.is_some())
                .count() as u16;
            let visible_rows = decision.options.len() as u16 + detail_rows + 1 + 1;
            let rows_needed = visible_rows + 1 + 2;
            rows_needed.min(17)
        }
        None => 0,
    }
}

pub(super) fn draw_inline_ui(frame: &mut Frame, app: &dyn TuiState, area: Rect) {
    match app.inline_ui_state() {
        Some(crate::tui::InlineUiStateRef::Interactive(picker))
            if picker.kind != crate::tui::PickerKind::Model =>
        {
            super::inline_interactive_ui::draw_inline_interactive(frame, app, area)
        }
        Some(crate::tui::InlineUiStateRef::View(view)) => draw_inline_view(frame, app, view, area),
        Some(crate::tui::InlineUiStateRef::Decision(decision)) => {
            draw_inline_decision(frame, app, decision, area)
        }
        _ => {}
    }
}

/// Render the blocking ask_user decision chooser: question header, numbered
/// options (selected row highlighted, detail on its own dim line), a free-form
/// "Your answer" entry row, and a key-hint footer. The main input box is
/// disabled while the chooser is visible; all typing goes to the answer row.
fn draw_inline_decision(
    frame: &mut Frame,
    app: &dyn TuiState,
    decision: &crate::tui::app::PendingDecision,
    area: Rect,
) {
    let height = area.height as usize;
    let width = area.width as usize;
    if height <= 2 || width <= 2 {
        return;
    }

    let title_line = format!("❓ {}", decision.question);
    let mut content_width = inline_view_display_width(title_line.as_str());
    for (label, detail) in &decision.options {
        let row_width = inline_view_display_width(label)
            + detail
                .as_ref()
                .map(|d| inline_view_display_width(d) + 2)
                .unwrap_or(0)
            + 6;
        content_width = content_width.max(row_width);
    }
    content_width = content_width.max(inline_view_display_width(
        "1-9 or ↑↓ + Enter to choose, type your answer below, Esc to dismiss",
    ));
    let content_width = content_width.min(width.saturating_sub(2)).max(1);
    let outer_width = content_width.saturating_add(2).min(width);
    let horizontal_offset = if app.centered_mode() {
        area.width.saturating_sub(outer_width as u16) / 2
    } else {
        0
    };
    let render_area = Rect {
        x: area.x + horizontal_offset,
        y: area.y,
        width: outer_width as u16,
        height: area.height,
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(rgb(170, 140, 60)))
        .style(Style::default().bg(rgb(18, 18, 26)))
        .title(Line::from(Span::styled(
            decision_truncate_display(title_line.as_str(), content_width),
            Style::default().fg(rgb(230, 190, 90)).bold(),
        )));
    frame.render_widget(block.clone(), render_area);

    let inner = block.inner(render_area);
    let mut lines: Vec<Line> = Vec::new();
    for (index, (label, detail)) in decision.options.iter().enumerate() {
        let is_selected = index == decision.selected;
        let marker = if is_selected { "❯" } else { " " };
        let number = index + 1;
        let row = format!("{} {}. {}", marker, number, label);
        let style = if is_selected {
            Style::default().fg(accent_color()).bold()
        } else {
            Style::default().fg(dim_color())
        };
        lines.push(Line::from(Span::styled(
            decision_truncate_display(row.as_str(), inner.width as usize),
            style,
        )));
        if let Some(detail) = detail {
            let detail_row = format!("     {}", detail);
            lines.push(Line::from(Span::styled(
                decision_truncate_display(detail_row.as_str(), inner.width as usize),
                Style::default().fg(dim_color()),
            )));
        }
    }

    // Free-form "Your answer" row: an extra option, like ask_user's own
    // free-form escape hatch, with its own inline edit buffer. Typing lands
    // here (the main input box is disabled while the chooser is visible).
    let answer_selected = decision.selected_is_answer_row();
    let number = decision.options.len() + 1;
    let answer_visible = if answer_selected {
        format!("❯ {}. Your answer: {}", number, decision.answer_draft)
    } else {
        format!(" {}. Your answer", number)
    };
    let answer_style = if answer_selected {
        Style::default().fg(accent_color()).bold()
    } else {
        Style::default().fg(dim_color())
    };
    lines.push(Line::from(Span::styled(
        decision_truncate_display(answer_visible.as_str(), inner.width as usize),
        answer_style,
    )));
    if answer_selected && decision.answer_draft.is_empty() {
        lines.push(Line::from(Span::styled(
            "     Type your own answer, then Enter",
            Style::default().fg(dim_color()),
        )));
    }

    lines.push(Line::from(Span::styled(
        "1-9 or ↑↓ + Enter to choose · type on Your answer · Esc dismisses",
        Style::default().fg(rgb(120, 120, 150)).italic(),
    )));
    let visible: Vec<Line> = lines.into_iter().take(inner.height as usize).collect();
    frame.render_widget(Paragraph::new(visible), inner);
}

fn draw_inline_view(
    frame: &mut Frame,
    app: &dyn TuiState,
    view: &crate::tui::InlineViewState,
    area: Rect,
) {
    let height = area.height as usize;
    let width = area.width as usize;
    if height <= 2 || width <= 2 {
        return;
    }

    let mut content_width = inline_view_display_width(view.title.as_str());
    if let Some(status) = view.status.as_ref() {
        content_width = content_width.max(inline_view_display_width(status.as_str()) + 2);
    }
    for line in &view.lines {
        content_width = content_width.max(inline_view_display_width(line.as_str()));
    }
    let content_width = content_width.min(width.saturating_sub(2)).max(1);
    let outer_width = content_width.saturating_add(2).min(width);
    let horizontal_offset = if app.centered_mode() {
        area.width.saturating_sub(outer_width as u16) / 2
    } else {
        0
    };
    let render_area = Rect {
        x: area.x + horizontal_offset,
        y: area.y,
        width: outer_width as u16,
        height: area.height,
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(rgb(85, 85, 110)))
        .style(Style::default().bg(rgb(18, 18, 26)));
    frame.render_widget(block.clone(), render_area);

    let inner = block.inner(render_area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    let mut header_spans = vec![Span::styled(
        view.title.clone(),
        Style::default().fg(Color::White).bold(),
    )];
    if let Some(status) = view.status.as_ref() {
        header_spans.push(Span::styled(
            format!("  {}", status),
            Style::default().fg(dim_color()).italic(),
        ));
    }
    lines.push(Line::from(header_spans));

    for line in &view.lines {
        lines.push(Line::from(Span::styled(
            line.clone(),
            Style::default().fg(rgb(200, 200, 220)),
        )));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}
