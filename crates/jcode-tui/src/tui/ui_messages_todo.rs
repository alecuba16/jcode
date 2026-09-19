//! Todo-card rendering for transcript tool messages.
//!
//! Split out of `ui_messages.rs` to keep that file within the code-size
//! budget; all helpers here render todo/plan/goal card rows.

use super::*;

pub(super) fn todo_card_line(
    spans: Vec<Span<'static>>,
    base_indent: &str,
    inner_width: usize,
) -> Line<'static> {
    let mut prefixed = vec![Span::raw(base_indent.to_string())];
    prefixed.extend(spans);
    super::truncate_line_with_ellipsis_to_width(
        &Line::from(prefixed),
        inner_width.saturating_add(base_indent.width()),
    )
}

pub(super) fn todo_card_goal_for_group<'a>(
    goals: &'a [crate::todo::TodoGoal],
    group: Option<&str>,
) -> Option<&'a crate::todo::TodoGoal> {
    let key = group.map(str::trim).filter(|value| !value.is_empty());
    goals.iter().find(|goal| {
        goal.group
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            == key
    })
}

fn todo_goal_score_spans(goal: &crate::todo::TodoGoal) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut states: Vec<(&str, String, Color)> = Vec::new();
    if !crate::todo::feedback_loop_passes(goal.closed_feedback_loop) {
        let (state, color) = goal.closed_feedback_loop.map_or_else(
            || ("missing".to_string(), todo_failure_color()),
            |state| {
                let color = if state <= crate::todo::FeedbackLoopState::Weak {
                    todo_failure_color()
                } else {
                    todo_warning_color()
                };
                (state.as_str().to_string(), color)
            },
        );
        states.push(("Closed feedback loop", state, color));
    }
    if !crate::todo::feedback_loop_relevance_passes(goal) {
        let (state, color) = goal.feedback_loop_relevance.map_or_else(
            || ("missing".to_string(), todo_failure_color()),
            |state| {
                let color = if state == crate::todo::FeedbackLoopRelevance::Indirect {
                    todo_failure_color()
                } else {
                    todo_warning_color()
                };
                (state.as_str().to_string(), color)
            },
        );
        states.push(("Relevance", state, color));
    }
    if !crate::todo::feedback_loop_coverage_passes(goal) {
        let (state, color) = goal.feedback_loop_coverage.map_or_else(
            || ("missing".to_string(), todo_failure_color()),
            |state| {
                let color = if state == crate::todo::FeedbackLoopCoverage::Narrow {
                    todo_failure_color()
                } else {
                    todo_warning_color()
                };
                (state.as_str().to_string(), color)
            },
        );
        states.push(("Coverage", state, color));
    }
    if !crate::todo::feedback_loop_traceability_passes(goal) {
        let (state, color) = goal.feedback_loop_traceability.map_or_else(
            || ("missing".to_string(), todo_failure_color()),
            |state| {
                let color = if state == crate::todo::FeedbackLoopTraceability::Unmapped {
                    todo_failure_color()
                } else {
                    todo_warning_color()
                };
                (state.as_str().to_string(), color)
            },
        );
        states.push(("Traceability", state, color));
    }

    if states.is_empty() {
        spans.push(Span::styled(
            "✓ All quality gates passing",
            Style::default().fg(todo_score_color()),
        ));
    }

    for (index, (label, state, color)) in states.into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" · ", Style::default().fg(dim_color())));
        }
        spans.push(Span::styled(
            format!("{} ", label),
            Style::default().fg(todo_label_color()),
        ));
        spans.push(Span::styled(state, Style::default().fg(color)));
    }

    // Delivery is progress toward the outcome, not a quality gate. Keep it
    // visible and visually separate from failures so it cannot read as one.
    if let Some(state) = goal.delivery_state {
        if !spans.is_empty() {
            spans.push(Span::styled(" · ", Style::default().fg(dim_color())));
        }
        spans.push(Span::styled(
            "Delivery ",
            Style::default().fg(todo_label_color()),
        ));
        let color = if state >= crate::todo::DeliveryState::WorkflowValidated {
            todo_score_color()
        } else if state == crate::todo::DeliveryState::Integrated {
            todo_warning_color()
        } else {
            todo_failure_color()
        };
        spans.push(Span::styled(
            state.as_str().to_string(),
            Style::default().fg(color),
        ));
    }
    spans
}

fn push_todo_status_pips<'a>(
    spans: &mut Vec<Span<'static>>,
    todos: impl IntoIterator<Item = &'a crate::todo::TodoItem>,
    max_pips: usize,
) {
    let (completed, in_progress, total) =
        todos
            .into_iter()
            .fold((0usize, 0usize, 0usize), |counts, todo| {
                (
                    counts.0 + usize::from(todo.status == "completed"),
                    counts.1 + usize::from(todo.status == "in_progress"),
                    counts.2 + 1,
                )
            });
    if total == 0 || max_pips == 0 {
        return;
    }

    let (done_pips, active_pips, open_pips) = if total <= max_pips.max(12) {
        (
            completed,
            in_progress,
            total.saturating_sub(completed + in_progress),
        )
    } else {
        let scale =
            |count: usize| ((count as f64 / total as f64) * max_pips as f64).round() as usize;
        let mut done = scale(completed);
        let mut active = scale(in_progress);
        if completed > 0 && done == 0 {
            done = 1;
        }
        if in_progress > 0 && active == 0 {
            active = 1;
        }
        done = done.min(max_pips);
        active = active.min(max_pips.saturating_sub(done));
        (done, active, max_pips.saturating_sub(done + active))
    };

    for _ in 0..done_pips {
        spans.push(Span::styled("●", Style::default().fg(rgb(100, 180, 100))));
    }
    for _ in 0..active_pips {
        spans.push(Span::styled("●", Style::default().fg(asap_color())));
    }
    for _ in 0..open_pips {
        spans.push(Span::styled("○", Style::default().fg(rgb(90, 90, 105))));
    }
}

pub(super) fn render_todo_status_header<'a>(
    todos: impl IntoIterator<Item = &'a crate::todo::TodoItem>,
    base_indent: &str,
    inner_width: usize,
) -> Line<'static> {
    let mut spans = Vec::new();
    push_todo_status_pips(&mut spans, todos, inner_width);
    todo_card_line(spans, base_indent, inner_width)
}

pub(super) fn render_todo_goal_header(
    label: &str,
    todos: &[&crate::todo::TodoItem],
    base_indent: &str,
    inner_width: usize,
) -> Line<'static> {
    let label_width = label.width();
    let mut spans = vec![Span::styled(
        label.to_string(),
        Style::default().fg(todo_group_color()).bold(),
    )];
    spans.push(Span::raw("  "));
    push_todo_status_pips(
        &mut spans,
        todos.iter().copied(),
        inner_width.saturating_sub(label_width + 2),
    );
    todo_card_line(spans, base_indent, inner_width)
}

fn wrap_todo_detail(value: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut chunks = Vec::new();
    let mut current = String::new();

    for word in value.split_whitespace() {
        let word_width = word.width();
        if !current.is_empty() && current.width() + 1 + word_width <= width {
            current.push(' ');
            current.push_str(word);
            continue;
        }
        if current.is_empty() && word_width <= width {
            current.push_str(word);
            continue;
        }
        if !current.is_empty() {
            chunks.push(std::mem::take(&mut current));
        }
        if word_width <= width {
            current.push_str(word);
            continue;
        }
        let mut word_chunks = split_by_display_width(word, width).into_iter().peekable();
        while let Some(chunk) = word_chunks.next() {
            if word_chunks.peek().is_some() {
                chunks.push(chunk);
            } else {
                current = chunk;
            }
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

/// Plan-level assessment lines shown once above the todo groups.
pub(super) fn push_todo_plan_details(
    lines: &mut Vec<Line<'static>>,
    plan: &crate::todo::TodoPlan,
    base_indent: &str,
    inner_width: usize,
    compact_details: bool,
) {
    let intention = plan
        .user_intention
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(state) = plan.understands_user_intent {
        let state_color = match state {
            crate::todo::IntentUnderstanding::Uncertain => todo_failure_color(),
            crate::todo::IntentUnderstanding::Partial => todo_warning_color(),
            crate::todo::IntentUnderstanding::Clear
            | crate::todo::IntentUnderstanding::Complete => todo_score_color(),
        };
        let intent_clear = matches!(
            state,
            crate::todo::IntentUnderstanding::Clear | crate::todo::IntentUnderstanding::Complete
        );
        if let Some(intention_text) = intention.filter(|_| intent_clear && !compact_details) {
            // A clear plan intention is worth reading in full, so wrap it
            // across card rows instead of clipping it to an ellipsis. Partial or
            // uncertain states stay on one ellipsized line: the state itself is
            // the signal, and the plan text is still subject to revision.
            // Keep the state in its own span so semantic colors stay testable.
            push_todo_wrapped_spans(
                lines,
                &[
                    Span::styled("Intent ", Style::default().fg(todo_label_color())),
                    Span::styled(state.as_str().to_string(), Style::default().fg(state_color)),
                    Span::styled(": ", Style::default().fg(todo_label_color())),
                ],
                intention_text,
                base_indent,
                inner_width,
            );
        } else {
            let mut spans = vec![
                Span::styled("Intent ", Style::default().fg(todo_label_color())),
                Span::styled(state.as_str().to_string(), Style::default().fg(state_color)),
                Span::styled(": ", Style::default().fg(todo_label_color())),
            ];
            if let Some(intention) = intention {
                spans.push(Span::styled(
                    intention.to_string(),
                    Style::default().fg(todo_meta_color()),
                ));
            }
            lines.push(todo_card_line(spans, base_indent, inner_width));
        }
    } else if let Some(intention) = intention {
        push_todo_detail(
            lines,
            "Intent",
            intention,
            base_indent,
            inner_width,
            compact_details,
        );
    }
}

fn push_todo_detail(
    lines: &mut Vec<Line<'static>>,
    label: &str,
    value: &str,
    base_indent: &str,
    inner_width: usize,
    compact: bool,
) {
    if !compact {
        push_todo_wrapped_detail(lines, label, value, base_indent, inner_width);
        return;
    }

    let prefix = format!("  {} · ", label);
    lines.push(todo_card_line(
        vec![
            Span::styled(prefix, Style::default().fg(todo_label_color())),
            Span::styled(value.to_string(), Style::default().fg(todo_meta_color())),
        ],
        base_indent,
        inner_width,
    ));
}

/// Wrap one labeled detail line to the card width.
fn push_todo_wrapped_detail(
    lines: &mut Vec<Line<'static>>,
    label: &str,
    value: &str,
    base_indent: &str,
    inner_width: usize,
) {
    let prefix = format!("  {} · ", label);
    push_todo_wrapped_spans(
        lines,
        &[Span::styled(
            prefix,
            Style::default().fg(todo_label_color()),
        )],
        value,
        base_indent,
        inner_width,
    );
}

/// Wrap `value` across card rows after a styled prefix; later rows are padded
/// so the wrapped text lines up under the first.
fn push_todo_wrapped_spans(
    lines: &mut Vec<Line<'static>>,
    prefix_spans: &[Span<'static>],
    value: &str,
    base_indent: &str,
    inner_width: usize,
) {
    let prefix_width: usize = prefix_spans.iter().map(|s| s.width()).sum();
    let available = inner_width.saturating_sub(prefix_width).max(1);
    for (index, chunk) in wrap_todo_detail(value, available).into_iter().enumerate() {
        let mut spans = if index == 0 {
            prefix_spans.to_vec()
        } else {
            vec![Span::raw(" ".repeat(prefix_width))]
        };
        spans.push(Span::styled(chunk, Style::default().fg(todo_meta_color())));
        lines.push(todo_card_line(spans, base_indent, inner_width));
    }
}

pub(super) fn push_todo_goal_details(
    lines: &mut Vec<Line<'static>>,
    goal: Option<&crate::todo::TodoGoal>,
    base_indent: &str,
    inner_width: usize,
    _compact_details: bool,
) {
    let Some(goal) = goal else {
        return;
    };
    let scores = todo_goal_score_spans(goal);
    if !scores.is_empty() {
        let score_width = Line::from(scores.clone()).width();
        let score_count = usize::from(!crate::todo::feedback_loop_passes(
            goal.closed_feedback_loop,
        )) + usize::from(!crate::todo::feedback_loop_relevance_passes(goal))
            + usize::from(!crate::todo::feedback_loop_coverage_passes(goal))
            + usize::from(!crate::todo::feedback_loop_traceability_passes(goal))
            + usize::from(goal.delivery_state.is_some());
        if score_width > inner_width.saturating_sub(2) && score_count > 1 {
            let mut states: Vec<(&str, String)> = Vec::new();
            if !crate::todo::feedback_loop_passes(goal.closed_feedback_loop) {
                states.push((
                    "Closed feedback loop",
                    goal.closed_feedback_loop
                        .map(|state| state.as_str())
                        .unwrap_or("missing")
                        .to_string(),
                ));
            }
            if !crate::todo::feedback_loop_relevance_passes(goal) {
                states.push((
                    "Relevance",
                    goal.feedback_loop_relevance
                        .map(|state| state.as_str())
                        .unwrap_or("missing")
                        .to_string(),
                ));
            }
            if !crate::todo::feedback_loop_coverage_passes(goal) {
                states.push((
                    "Coverage",
                    goal.feedback_loop_coverage
                        .map(|state| state.as_str())
                        .unwrap_or("missing")
                        .to_string(),
                ));
            }
            if !crate::todo::feedback_loop_traceability_passes(goal) {
                states.push((
                    "Traceability",
                    goal.feedback_loop_traceability
                        .map(|state| state.as_str())
                        .unwrap_or("missing")
                        .to_string(),
                ));
            }
            if let Some(state) = goal.delivery_state {
                states.push(("Delivery", state.as_str().to_string()));
            }
            for (label, state) in states {
                let mut spans = vec![Span::raw("  ")];
                spans.push(Span::styled(
                    format!("{} ", label),
                    Style::default().fg(todo_label_color()),
                ));
                let color = if label == "Delivery" {
                    match crate::todo::DeliveryState::parse(&state) {
                        Some(value) if value >= crate::todo::DeliveryState::WorkflowValidated => {
                            todo_score_color()
                        }
                        Some(crate::todo::DeliveryState::Integrated) => todo_warning_color(),
                        _ => todo_failure_color(),
                    }
                } else if matches!(
                    state.as_str(),
                    "missing" | "absent" | "weak" | "indirect" | "narrow" | "unmapped"
                ) {
                    todo_failure_color()
                } else {
                    todo_warning_color()
                };
                spans.push(Span::styled(state, Style::default().fg(color)));
                lines.push(todo_card_line(spans, base_indent, inner_width));
            }
        } else {
            let mut spans = vec![Span::raw("  ")];
            spans.extend(scores);
            lines.push(todo_card_line(spans, base_indent, inner_width));
        }
    }
}

/// Concise refinement card for assessment-only todo writes: the plan-level
/// intent change first, then any per-goal quality updates.
pub(super) fn render_todo_assessment_updates(
    plan_update: Option<&crate::todo::TodoPlanChange>,
    goal_updates: &[crate::todo::TodoGoalChange],
    width: u16,
) -> Vec<Line<'static>> {
    let mut lines = render_todo_plan_update(plan_update, width);
    lines.extend(render_todo_goal_updates(goal_updates, width));
    lines
}

fn render_todo_plan_update(
    plan_update: Option<&crate::todo::TodoPlanChange>,
    width: u16,
) -> Vec<Line<'static>> {
    let Some(update) = plan_update else {
        return Vec::new();
    };
    let intent_is_unclear = !crate::todo::intent_understanding_passes(
        update
            .after
            .as_ref()
            .and_then(|plan| plan.understands_user_intent),
    );
    if !(update
        .fields
        .contains(&crate::todo::TodoPlanField::UnderstandsUserIntent)
        || (intent_is_unclear
            && update
                .fields
                .contains(&crate::todo::TodoPlanField::UserIntention)))
    {
        return Vec::new();
    }
    let centered = markdown::center_code_blocks();
    let card_width = if centered {
        (width.saturating_sub(4) as usize).min(120)
    } else {
        (width.saturating_sub(2) as usize).min(100)
    }
    .max(1);
    let base_indent = if centered { "" } else { "  " };
    let inner_width = card_width.saturating_sub(base_indent.width()).max(1);
    let mut lines = vec![todo_card_line(
        vec![
            Span::styled("Plan", Style::default().fg(todo_group_color()).bold()),
            Span::styled("  updated", Style::default().fg(todo_meta_color())),
        ],
        base_indent,
        inner_width,
    )];

    for field in &update.fields {
        match field {
            crate::todo::TodoPlanField::UnderstandsUserIntent => push_todo_score_update(
                &mut lines,
                "Understands user intent",
                update
                    .before
                    .as_ref()
                    .and_then(|plan| plan.understands_user_intent)
                    .map(|state| state.as_str().to_string()),
                update
                    .after
                    .as_ref()
                    .and_then(|plan| plan.understands_user_intent)
                    .map(|state| state.as_str().to_string()),
                base_indent,
                inner_width,
            ),
            crate::todo::TodoPlanField::UserIntention if intent_is_unclear => {
                push_todo_text_update(
                    &mut lines,
                    "User intention",
                    update
                        .after
                        .as_ref()
                        .and_then(|plan| plan.user_intention.as_deref()),
                    base_indent,
                    inner_width,
                )
            }
            crate::todo::TodoPlanField::UserIntention => {}
        }
    }

    if centered {
        left_pad_lines_for_centered_mode(&mut lines, width);
    }
    lines
}

pub(crate) fn render_todo_goal_updates(
    updates: &[crate::todo::TodoGoalChange],
    width: u16,
) -> Vec<Line<'static>> {
    let centered = markdown::center_code_blocks();
    let card_width = if centered {
        (width.saturating_sub(4) as usize).min(120)
    } else {
        (width.saturating_sub(2) as usize).min(100)
    }
    .max(1);
    let base_indent = if centered { "" } else { "  " };
    let inner_width = card_width.saturating_sub(base_indent.width()).max(1);
    let mut lines = Vec::new();

    for update in updates {
        // Narrative assessment fields remain available in the dedicated todos
        // view. Inline tool cards only show the compact state transitions so a
        // long feedback loop or stopping rationale cannot dominate the chat.
        let visible_fields = update.fields.iter().filter(|field| {
            !matches!(
                field,
                crate::todo::TodoGoalField::FeedbackLoop
                    | crate::todo::TodoGoalField::StoppingEvidence
            )
        });
        if visible_fields.clone().next().is_none() {
            continue;
        }
        let goal = update.after.as_ref().or(update.before.as_ref());
        let label = goal
            .and_then(|goal| goal.group.as_deref())
            .map(str::trim)
            .filter(|group| !group.is_empty())
            .unwrap_or("Goal");
        lines.push(todo_card_line(
            vec![
                Span::styled(
                    label.to_string(),
                    Style::default().fg(todo_group_color()).bold(),
                ),
                Span::styled("  updated", Style::default().fg(todo_meta_color())),
            ],
            base_indent,
            inner_width,
        ));

        for field in visible_fields {
            match field {
                crate::todo::TodoGoalField::ClosedFeedbackLoop => push_todo_score_update(
                    &mut lines,
                    "Closed feedback loop",
                    update
                        .before
                        .as_ref()
                        .and_then(|goal| goal.closed_feedback_loop)
                        .map(|state| state.as_str().to_string()),
                    update
                        .after
                        .as_ref()
                        .and_then(|goal| goal.closed_feedback_loop)
                        .map(|state| state.as_str().to_string()),
                    base_indent,
                    inner_width,
                ),
                crate::todo::TodoGoalField::FeedbackLoopRelevance => push_todo_score_update(
                    &mut lines,
                    "Feedback-loop relevance",
                    update
                        .before
                        .as_ref()
                        .and_then(|goal| goal.feedback_loop_relevance)
                        .map(|state| state.as_str().to_string()),
                    update
                        .after
                        .as_ref()
                        .and_then(|goal| goal.feedback_loop_relevance)
                        .map(|state| state.as_str().to_string()),
                    base_indent,
                    inner_width,
                ),
                crate::todo::TodoGoalField::FeedbackLoopCoverage => push_todo_score_update(
                    &mut lines,
                    "Feedback-loop coverage",
                    update
                        .before
                        .as_ref()
                        .and_then(|goal| goal.feedback_loop_coverage)
                        .map(|state| state.as_str().to_string()),
                    update
                        .after
                        .as_ref()
                        .and_then(|goal| goal.feedback_loop_coverage)
                        .map(|state| state.as_str().to_string()),
                    base_indent,
                    inner_width,
                ),
                crate::todo::TodoGoalField::FeedbackLoopTraceability => push_todo_score_update(
                    &mut lines,
                    "Feedback-loop traceability",
                    update
                        .before
                        .as_ref()
                        .and_then(|goal| goal.feedback_loop_traceability)
                        .map(|state| state.as_str().to_string()),
                    update
                        .after
                        .as_ref()
                        .and_then(|goal| goal.feedback_loop_traceability)
                        .map(|state| state.as_str().to_string()),
                    base_indent,
                    inner_width,
                ),
                crate::todo::TodoGoalField::DeliveryState => push_todo_score_update(
                    &mut lines,
                    "Delivery",
                    update
                        .before
                        .as_ref()
                        .and_then(|goal| goal.delivery_state)
                        .map(|state| state.as_str().to_string()),
                    update
                        .after
                        .as_ref()
                        .and_then(|goal| goal.delivery_state)
                        .map(|state| state.as_str().to_string()),
                    base_indent,
                    inner_width,
                ),
                crate::todo::TodoGoalField::Autonomy => push_todo_score_update(
                    &mut lines,
                    "Autonomy",
                    update
                        .before
                        .as_ref()
                        .and_then(|goal| goal.autonomy)
                        .map(|state| state.as_str().to_string()),
                    update
                        .after
                        .as_ref()
                        .and_then(|goal| goal.autonomy)
                        .map(|state| state.as_str().to_string()),
                    base_indent,
                    inner_width,
                ),
                crate::todo::TodoGoalField::IterationMaturity => push_todo_score_update(
                    &mut lines,
                    "Iteration",
                    update
                        .before
                        .as_ref()
                        .and_then(|goal| goal.iteration_maturity)
                        .map(|state| state.as_str().to_string()),
                    update
                        .after
                        .as_ref()
                        .and_then(|goal| goal.iteration_maturity)
                        .map(|state| state.as_str().to_string()),
                    base_indent,
                    inner_width,
                ),
                crate::todo::TodoGoalField::FeedbackLoop
                | crate::todo::TodoGoalField::StoppingEvidence => unreachable!(),
            }
        }
    }

    if centered {
        left_pad_lines_for_centered_mode(&mut lines, width);
    }
    lines
}

fn push_todo_score_update(
    lines: &mut Vec<Line<'static>>,
    label: &str,
    before: Option<String>,
    after: Option<String>,
    base_indent: &str,
    inner_width: usize,
) {
    let mut spans = vec![
        Span::raw("  "),
        Span::styled(
            format!("{} ", label),
            Style::default().fg(todo_label_color()),
        ),
    ];
    match (before, after) {
        (Some(before), Some(after)) => {
            spans.push(Span::styled(before, Style::default().fg(todo_meta_color())));
            spans.push(Span::styled(" → ", Style::default().fg(todo_label_color())));
            spans.push(Span::styled(after, Style::default().fg(todo_score_color())));
        }
        (None, Some(after)) => {
            spans.push(Span::styled(after, Style::default().fg(todo_score_color())))
        }
        (_, None) => spans.push(Span::styled(
            "cleared",
            Style::default().fg(todo_meta_color()),
        )),
    }
    lines.push(todo_card_line(spans, base_indent, inner_width));
}

fn push_todo_text_update(
    lines: &mut Vec<Line<'static>>,
    label: &str,
    after: Option<&str>,
    base_indent: &str,
    inner_width: usize,
) {
    let value = after.map(str::trim).filter(|value| !value.is_empty());
    let prefix = format!("  {} · ", label);
    let prefix_width = prefix.width();
    let available = inner_width.saturating_sub(prefix_width).max(1);
    let chunks = value
        .map(|value| wrap_todo_detail(value, available))
        .filter(|chunks| !chunks.is_empty())
        .unwrap_or_else(|| vec!["cleared".to_string()]);
    for (index, chunk) in chunks.into_iter().enumerate() {
        lines.push(todo_card_line(
            vec![
                Span::styled(
                    if index == 0 {
                        prefix.clone()
                    } else {
                        " ".repeat(prefix_width)
                    },
                    Style::default().fg(todo_label_color()),
                ),
                Span::styled(chunk, Style::default().fg(todo_meta_color())),
            ],
            base_indent,
            inner_width,
        ));
    }
}

fn todo_card_confidence_label(todo: &crate::todo::TodoItem) -> Option<String> {
    if todo.status == "completed"
        && let (Some(planning), Some(completed)) = (todo.confidence, todo.completion_confidence)
        && planning != completed
    {
        return Some(format!("{}→{}", planning.as_str(), completed.as_str()));
    }
    let state = if todo.status == "completed" {
        todo.completion_confidence.or(todo.confidence)
    } else {
        todo.confidence
    };
    state.map(|state| state.as_str().to_string())
}

pub(super) fn render_todo_card_item_line(
    todo: &crate::todo::TodoItem,
    base_indent: &str,
    inner_width: usize,
) -> Line<'static> {
    let blocked = !todo.blocked_by.is_empty() && todo.status != "completed";
    let (glyph, glyph_color) = if blocked {
        ("⊳", rgb(225, 165, 90))
    } else {
        match todo.status.as_str() {
            "completed" => ("✓", rgb(105, 190, 125)),
            "in_progress" => ("●", asap_color()),
            "cancelled" => ("✗", rgb(190, 105, 115)),
            _ => ("○", rgb(135, 145, 160)),
        }
    };
    let text_color = match todo.status.as_str() {
        "completed" => rgb(135, 150, 145),
        "cancelled" => rgb(145, 130, 135),
        "in_progress" => rgb(225, 232, 240),
        _ => rgb(195, 202, 212),
    };
    let mut spans = vec![
        Span::raw("  "),
        Span::styled(format!("{} ", glyph), Style::default().fg(glyph_color)),
        Span::styled(todo.content.clone(), Style::default().fg(text_color)),
    ];
    if let Some(label) = todo_card_confidence_label(todo) {
        spans.push(Span::styled(
            format!(" · {}", label),
            Style::default().fg(todo_confidence_color()),
        ));
    }
    todo_card_line(spans, base_indent, inner_width)
}
