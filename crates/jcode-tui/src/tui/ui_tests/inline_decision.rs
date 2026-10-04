use super::*;
use ratatui::backend::TestBackend;
use ratatui::{Terminal, layout::Rect};

/// Render the decision chooser via draw_inline_ui and return the buffer rows.
fn render_decision_chooser(state: &TestState, width: u16, height: u16) -> Vec<String> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("failed to create test terminal");
    terminal
        .draw(|frame| {
            let area = Rect::new(0, 0, width, height);
            crate::tui::ui::inline_ui::draw_inline_ui(frame, state, area);
        })
        .expect("failed to draw decision chooser");
    let buf = terminal.backend().buffer();
    let mut lines = Vec::with_capacity(height as usize);
    for y in 0..height {
        let mut line = String::with_capacity(width as usize);
        for x in 0..width {
            line.push_str(buf[(x, y)].symbol());
        }
        lines.push(line.trim_end().to_string());
    }
    lines
}

fn decision_state(selected: usize, answer_draft: &str, centered: bool) -> TestState {
    TestState {
        centered_mode: centered,
        pending_decision_state: Some(crate::tui::app::PendingDecision {
            request_id: "decision-1".to_string(),
            question: "Deploy now or stage first?".to_string(),
            options: vec![
                ("Deploy".to_string(), None),
                ("Stage".to_string(), Some("stage first".to_string())),
            ],
            selected,
            answer_draft: answer_draft.to_string(),
        }),
        ..TestState::default()
    }
}

/// decision_truncate_display: every branch. Fits, zero budget, single column,
/// loop truncation with plain chars, and wide chars (emoji) that overflow.
#[test]
fn decision_truncate_display_covers_all_branches() {
    // Fits: unchanged.
    assert_eq!(
        crate::tui::ui::inline_ui::decision_truncate_display_for_test("hello", 10),
        "hello"
    );
    // Zero budget: empty.
    assert_eq!(
        crate::tui::ui::inline_ui::decision_truncate_display_for_test("hello", 0),
        ""
    );
    // One column: just the ellipsis.
    assert_eq!(
        crate::tui::ui::inline_ui::decision_truncate_display_for_test("hello", 1),
        "…"
    );
    // Loop truncation: keep what fits, end with the ellipsis.
    let truncated = crate::tui::ui::inline_ui::decision_truncate_display_for_test("abcdef", 4);
    assert_eq!(truncated, "abc…");
    // Wide chars: a two-column char that does not fit is dropped whole.
    let emoji = crate::tui::ui::inline_ui::decision_truncate_display_for_test("a❓b", 3);
    assert_eq!(emoji, "a…");
}

/// inline_ui_height: the decision branch counts option rows, one row per
/// option detail, the answer row, and the hint row, capped at 17.
#[test]
fn inline_ui_height_decision_branch_counts_details_and_caps() {
    let two_options_one_detail = decision_state(0, "", false);
    let state = two_options_one_detail;
    assert_eq!(
        crate::tui::ui::inline_ui::inline_ui_height_for_test(&state),
        2 + 1 + 1 + 1 + 3,
        "2 options + 1 detail + answer + hint rows + header + border"
    );

    // Many options with details must cap at the 17-row maximum.
    let mut tall = TestState::default();
    tall.pending_decision_state = Some(crate::tui::app::PendingDecision {
        request_id: "decision-2".to_string(),
        question: "q".to_string(),
        options: (0..8)
            .map(|i| (format!("option {i}"), Some("detail".to_string())))
            .collect(),
        selected: 0,
        answer_draft: String::new(),
    });
    assert_eq!(
        crate::tui::ui::inline_ui::inline_ui_height_for_test(&tall),
        17,
        "the chooser height must cap at 17 rows"
    );
}

/// A tiny area (height or width <= 2) draws nothing and must not panic.
#[test]
fn chooser_in_tiny_area_draws_nothing_without_panicking() {
    let state = decision_state(0, "", false);
    let rows = render_decision_chooser(&state, 2, 2);
    assert!(
        rows.iter().all(|r| r.trim().is_empty()),
        "a 2x2 area must render nothing"
    );
    let rows = render_decision_chooser(&state, 30, 1);
    assert!(
        rows.iter().all(|r| r.trim().is_empty()),
        "a one-row area must render nothing"
    );
}

/// Centered mode offsets the chooser horizontally instead of pinning it left.
/// The chooser is sized to its content, so centered mode only shows an offset
/// in a terminal wider than the chooser itself.
#[test]
fn centered_mode_offsets_the_chooser() {
    // Narrow options keep the chooser well under the 120-col frame.
    let mut narrow = TestState::default();
    narrow.centered_mode = false;
    narrow.pending_decision_state = Some(crate::tui::app::PendingDecision {
        request_id: "decision-3".to_string(),
        question: "Go?".to_string(),
        options: vec![("Yes".to_string(), None), ("No".to_string(), None)],
        selected: 0,
        answer_draft: String::new(),
    });
    let mut narrow_centered = narrow.clone();
    narrow_centered.centered_mode = true;

    let left_rows = render_decision_chooser(&narrow, 120, 12);
    let centered_rows = render_decision_chooser(&narrow_centered, 120, 12);

    let border_x = |rows: &[String]| {
        rows.iter()
            .map(|r| r.find('╭').unwrap_or(usize::MAX))
            .filter(|x| *x != usize::MAX)
            .min()
            .expect("frame has a border")
    };
    let left_border_x = border_x(&left_rows);
    let centered_border_x = border_x(&centered_rows);
    assert_eq!(left_border_x, 0, "left-aligned chooser starts at column 0");
    assert!(
        centered_border_x > 0,
        "centered mode must offset the chooser right, got {}",
        centered_border_x
    );
}

/// Answer row selected with an empty draft shows the typing hint; with a
/// draft it shows the draft text instead.
#[test]
fn answer_row_hint_renders_only_with_empty_draft() {
    let empty = decision_state(crate::tui::app::PendingDecision::ANSWER_ROW, "", false);
    let rows = render_decision_chooser(&empty, 60, 14);
    let text = rows.join("\n");
    assert!(
        text.contains("Type your own answer"),
        "empty draft must show the typing hint, got: {}",
        text
    );

    let filled = decision_state(
        crate::tui::app::PendingDecision::ANSWER_ROW,
        "ship it friday",
        false,
    );
    let rows = render_decision_chooser(&filled, 60, 14);
    let text = rows.join("\n");
    assert!(
        text.contains("ship it friday"),
        "draft text must render, got: {}",
        text
    );
    assert!(
        !text.contains("Type your own answer"),
        "a filled draft replaces the hint"
    );
}

/// An option row selected (not the answer row) never shows the typing hint.
#[test]
fn option_row_selection_renders_marker_without_hint() {
    let state = decision_state(0, "", false);
    let rows = render_decision_chooser(&state, 60, 14);
    let text = rows.join("\n");
    assert!(text.contains("❯ 1. Deploy"), "selected row shows marker");
    assert!(
        !text.contains("Type your own answer"),
        "hint only renders for the answer row"
    );
}
/// A three-row-tall area fits the border and exactly one content row: the
/// chooser renders the first option and clips the rest, proving the inner
/// region is always non-empty once the height/width <= 2 early return is
/// passed (the border consumes exactly 2 rows/cols and content_width is
/// clamped to at least 1).
#[test]
fn chooser_with_one_inner_row_renders_first_option_without_panicking() {
    let state = decision_state(0, "", false);
    let rows = render_decision_chooser(&state, 40, 3);
    assert!(
        rows.iter().any(|r| r.contains('╭')),
        "the rounded top border must render"
    );
    assert!(
        rows.iter().any(|r| r.contains("1. Deploy")),
        "the single inner row shows the first option"
    );
    assert!(
        !rows.iter().any(|r| r.contains("Your answer")),
        "only one content row fits; the answer row must be clipped"
    );
}
