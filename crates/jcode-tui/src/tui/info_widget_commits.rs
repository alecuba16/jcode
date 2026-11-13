//! Commits widget: what has landed on the current branch recently.
//!
//! The Changes widget answers "what is dirty". This one answers "what did we
//! just commit, and has it been pushed". Border layout: `Commits` top-left,
//! overflow bottom-left, the unpushed count bottom-right. Fully shown pushed
//! history confirms its push state on the bottom border instead.
//!
//! ```text
//! ╭ Commits ────────────────────────────────╮
//! │● e4b4d3a swarm dock lists each agen 2m  │
//! │● e205d0f info widgets put headers o 14m │
//! │  4f6bf8e replay reasoning before th 1h  │
//! ╰ +5 more ─────────────────── ↑2 unpushed ╯
//! ```
//!
//! Subjects are cut hard at the row edge, with no ellipsis, so every cell
//! carries commit text.

use super::frame::{self, Framed};
use super::text::clip;
use super::{GitInfo, InfoWidgetData, RecentCommit};
use crate::tui::color_support::rgb;
use ratatui::prelude::*;
use unicode_width::UnicodeWidthStr;

/// Hash cells kept once the subject no longer fits: still unambiguous enough
/// to paste, and every cell saved goes to the subject.
const SHORT_HASH: usize = 7;

/// Most commits listed before the rest collapse into the footer.
pub(super) const COMMITS_MAX_ROWS: usize = 5;

/// Hide the widget once the newest commit is older than this: stale history
/// is not worth margin space.
const COMMITS_FRESH_SECS: i64 = 24 * 3600;

pub(crate) fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub(crate) fn commits_has_data(info: &GitInfo) -> bool {
    commits_has_data_at(info, now_secs())
}

/// Overview Commits section: one counts row instead of a commit listing, so
/// the panel spends its rows on sections rather than history. Same freshness
/// gate as the widget: stale pushed history is not worth margin space.
pub(crate) fn render_commits_counts_at(
    info: &GitInfo,
    width: usize,
    now: i64,
) -> Vec<Line<'static>> {
    if !commits_has_data_at(info, now) {
        return Vec::new();
    }

    let total = info.recent_commits.len();
    let unpushed = info
        .recent_commits
        .iter()
        .filter(|c| c.unpushed)
        .count()
        .max(info.ahead);
    let label = if total == 1 { "commit" } else { "commits" };
    let mut spans: Vec<Span<'static>> = vec![
        Span::styled("Commits", Style::default().fg(rgb(180, 180, 190)).bold()),
        Span::raw(format!(" {total} {label}")),
    ];
    if unpushed > 0 {
        spans.push(Span::raw(" · "));
        spans.push(Span::styled(
            format!("↑{unpushed} unpushed"),
            Style::default().fg(rgb(140, 180, 255)),
        ));
    }
    let added: usize = info
        .recent_commits
        .iter()
        .map(|c| c.added.unwrap_or(0))
        .sum();
    let removed: usize = info
        .recent_commits
        .iter()
        .map(|c| c.removed.unwrap_or(0))
        .sum();
    if added + removed > 0 {
        spans.push(Span::raw(" · "));
        spans.push(Span::styled(
            format!("+{added}"),
            Style::default().fg(rgb(90, 170, 100)),
        ));
        spans.push(Span::styled(
            format!(" −{removed}"),
            Style::default().fg(rgb(200, 100, 95)),
        ));
    }
    vec![frame::fit(&Line::from(spans), width).unwrap_or_default()]
}

/// Same-frame callers that measure the section's height must pass their `now`
/// here so freshness cannot flip between measurement and draw. The counts row
/// never wraps: chrome truncates at the row edge instead.
pub(crate) fn render_commits_compact_at(
    info: &GitInfo,
    width: usize,
    now: i64,
) -> Vec<Line<'static>> {
    render_commits_counts_at(info, width, now)
}

pub(crate) fn commits_has_data_at(info: &GitInfo, now: i64) -> bool {
    info.recent_commits.first().is_some_and(|newest| {
        newest.unpushed || now.saturating_sub(newest.timestamp) < COMMITS_FRESH_SECS
    })
}

fn short_age(secs: i64) -> String {
    let secs = secs.max(0);
    if secs < 60 {
        "now".to_string()
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

pub(super) fn render_commits_widget(data: &InfoWidgetData, inner: Rect) -> Framed {
    render_commits_at(data, inner, now_secs())
}

fn render_commits_at(data: &InfoWidgetData, inner: Rect, now: i64) -> Framed {
    let Some(info) = &data.git_info else {
        return Framed::default();
    };
    if !commits_has_data_at(info, now) {
        return Framed::default();
    }
    let rows = (inner.height as usize)
        .clamp(1, COMMITS_MAX_ROWS)
        .min(info.recent_commits.len());
    let width = inner.width as usize;
    let lines: Vec<Line<'static>> = info.recent_commits[..rows]
        .iter()
        .map(|c| commit_line(c, width, now))
        .collect();

    let mut framed = Framed::body(lines).title(frame::label("Commits"));
    let unpushed = info.recent_commits.iter().filter(|c| c.unpushed).count();
    // `ahead` counts past the fetched window; prefer it when larger.
    let unpushed = unpushed.max(info.ahead);
    if unpushed > 0 {
        framed = framed.footer_right(Span::styled(
            format!("↑{unpushed} unpushed"),
            Style::default().fg(rgb(140, 180, 255)),
        ));
    }
    let hidden = info.recent_commits.len() - rows;
    if hidden > 0 {
        framed = framed.footer(frame::more(hidden));
    } else if unpushed == 0 {
        framed = framed.footer(frame::dim("all pushed"));
    }
    framed
}

fn commit_line(commit: &RecentCommit, width: usize, now: i64) -> Line<'static> {
    commit_line_spans(commit, width, now).0
}

fn commit_line_spans(
    commit: &RecentCommit,
    width: usize,
    now: i64,
) -> (Line<'static>, usize) {
    let marker = if commit.unpushed {
        Span::styled("● ", Style::default().fg(rgb(140, 180, 255)))
    } else {
        Span::styled("  ", Style::default())
    };
    let marker_indent = marker.width();
    let subject_cells = UnicodeWidthStr::width(commit.subject.as_str());
    let full_hash = UnicodeWidthStr::width(commit.hash.as_str());
    let hash = if width < 2 + full_hash + 1 + subject_cells {
        format!("{} ", clip(&commit.hash, SHORT_HASH))
    } else {
        format!("{} ", commit.hash)
    };
    let age = short_age(now - commit.timestamp);
    let stat = match (commit.added, commit.removed) {
        (Some(a), Some(r)) => Some((format!("+{a}"), format!(" −{r}"))),
        _ => None,
    };
    let stat_w = stat
        .as_ref()
        .map(|(a, r)| UnicodeWidthStr::width(a.as_str()) + UnicodeWidthStr::width(r.as_str()) + 1)
        .unwrap_or(0);
    let age_w = UnicodeWidthStr::width(age.as_str()) + 1;
    let fixed = marker_indent + UnicodeWidthStr::width(hash.as_str());

    // The subject is the point of the row: line stats only appear when it
    // keeps a comfortable width, and the age goes next.
    let min_subject = 12;
    let roomy_subject = 26;
    let (show_stat, show_age) = if width >= fixed + roomy_subject + stat_w + age_w {
        (stat.is_some(), true)
    } else if width >= fixed + min_subject + age_w {
        (false, true)
    } else {
        (false, false)
    };
    let right_w = if show_stat { stat_w } else { 0 } + if show_age { age_w } else { 0 };
    let subject_w = width.saturating_sub(fixed + right_w);
    let subject = clip(&commit.subject, subject_w);
    let pad = subject_w.saturating_sub(UnicodeWidthStr::width(subject.as_str()));

    let mut spans = vec![
        marker,
        Span::styled(hash, Style::default().fg(rgb(200, 170, 90))),
        Span::styled(
            subject,
            Style::default().fg(if commit.unpushed {
                rgb(210, 210, 220)
            } else {
                rgb(160, 160, 170)
            }),
        ),
        Span::raw(" ".repeat(pad)),
    ];
    if show_stat && let Some((a, r)) = stat {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(a, Style::default().fg(rgb(90, 170, 100))));
        spans.push(Span::styled(r, Style::default().fg(rgb(200, 100, 95))));
    }
    if show_age {
        spans.push(Span::styled(
            format!(" {age}"),
            Style::default().fg(rgb(120, 120, 130)),
        ));
    }
    let line = Line::from(spans);
    if line.width() <= width {
        (line, marker_indent)
    } else {
        (frame::fit(&line, width).unwrap_or_default(), marker_indent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn commit(hash: &str, subject: &str, ago: i64, unpushed: bool) -> RecentCommit {
        RecentCommit {
            hash: hash.to_string(),
            subject: subject.to_string(),
            timestamp: NOW - ago,
            unpushed,
            added: Some(120),
            removed: Some(46),
        }
    }

    fn data(commits: Vec<RecentCommit>, ahead: usize) -> InfoWidgetData {
        InfoWidgetData {
            git_info: Some(GitInfo {
                ahead,
                recent_commits: commits,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn lists_commits_with_unpushed_marker_and_border_count() {
        let d = data(
            vec![
                commit("e4b4d3a", "swarm dock lists each agent", 120, true),
                commit("e205d0f", "info widgets put headers on borders", 840, true),
                commit("4f6bf8e", "replay reasoning", 4000, false),
            ],
            2,
        );
        let f = render_commits_at(&d, Rect::new(0, 0, 60, 8), NOW);
        assert_eq!(f.lines.len(), 3);
        let first = text(&f.lines[0]);
        assert!(first.starts_with("● e4b4d3a swarm dock"), "{first}");
        assert!(first.ends_with(" 2m"), "{first}");
        assert!(first.contains("+120 −46"), "{first}");
        assert!(text(&f.lines[2]).starts_with("  4f6bf8e"));
        assert!(text(f.footer_right.as_ref().unwrap()).contains("↑2 unpushed"));
        for line in &f.lines {
            assert!(line.width() <= 60);
        }
    }

    #[test]
    fn overview_counts_row_lists_counts_not_commits() {
        let d = data(
            vec![
                commit("e4b4d3a", "swarm dock lists each agent", 120, true),
                commit("e205d0f", "info widgets put headers on borders", 840, false),
                commit("4f6bf8e", "replay reasoning before its text", 4000, false),
                commit("a526dd5", "tui: drop dot separators", 9000, false),
            ],
            0,
        );
        let lines = render_commits_counts_at(d.git_info.as_ref().unwrap(), 44, NOW);
        assert_eq!(lines.len(), 1, "the section is one counts row");
        let row = text(&lines[0]);
        assert_eq!(row, "Commits 4 commits · ↑1 unpushed · +480 −184");
        assert_eq!(lines[0].width(), 43, "counts row stays one visual row");
    }

    #[test]
    fn overview_counts_row_drops_zero_line_stats_and_singular() {
        let zero_stats = RecentCommit {
            hash: "aaa".to_string(),
            subject: "only commit".to_string(),
            timestamp: NOW - 120,
            unpushed: false,
            added: None,
            removed: None,
        };
        let d = data(vec![zero_stats], 0);
        let lines = render_commits_counts_at(d.git_info.as_ref().unwrap(), 40, NOW);
        let row = text(&lines[0]);
        assert_eq!(row, "Commits 1 commit");
    }

    #[test]
    fn overview_counts_row_prefers_larger_ahead_count() {
        let d = data(vec![commit("aaa", "only fetched commit", 120, false)], 3);
        let lines = render_commits_counts_at(d.git_info.as_ref().unwrap(), 40, NOW);
        let row = text(&lines[0]);
        assert!(row.contains("↑3 unpushed"), "ahead outranks the unpushed flags: {row}");
    }

    #[test]
    fn overview_counts_row_never_overflows_its_row() {
        let d = data(
            vec![commit("aaa", "subject", 120, true), commit("bbb", "s", 120, true)],
            2,
        );
        for w in [0usize, 4, 12, 30, 60] {
            let lines = render_commits_counts_at(d.git_info.as_ref().unwrap(), w, NOW);
            assert_eq!(lines.len(), 1);
            assert!(lines[0].width() <= w, "w={w}: {}", text(&lines[0]));
        }
    }

    #[test]
    fn overflow_goes_to_footer() {
        let commits = (0..8)
            .map(|i| commit(&format!("abc{i}"), "work", 60 * i, false))
            .collect();
        let f = render_commits_at(&data(commits, 0), Rect::new(0, 0, 40, 20), NOW);
        assert_eq!(f.lines.len(), COMMITS_MAX_ROWS);
        assert!(text(f.footer.as_ref().unwrap()).contains("+3 more"));
    }

    #[test]
    fn pushed_history_footer_reports_push_state_not_duplicate_age() {
        let d = data(
            vec![
                commit("aaa", "first", 60, false),
                commit("bbb", "second", 180, false),
            ],
            0,
        );
        let card = render_commits_at(&d, Rect::new(0, 0, 40, 5), NOW);
        assert_eq!(text(card.footer.as_ref().unwrap()), "all pushed");
        assert!(card.title_right.is_none());
    }

    #[test]
    fn stale_pushed_history_hides_the_widget() {
        let d = data(vec![commit("aaa", "old", 3 * 86_400, false)], 0);
        assert!(render_commits_at(&d, Rect::new(0, 0, 40, 5), NOW).is_empty());
        // Unpushed work always shows, however old.
        let d = data(vec![commit("aaa", "old", 3 * 86_400, true)], 1);
        assert!(!render_commits_at(&d, Rect::new(0, 0, 40, 5), NOW).is_empty());
    }

    #[test]
    fn narrow_widths_drop_stats_then_age_and_never_overflow() {
        let d = data(
            vec![commit("e4b4d3a", "a long subject line here", 120, true)],
            1,
        );
        for w in [0u16, 1, 8, 16, 24, 30, 60] {
            let f = render_commits_at(&d, Rect::new(0, 0, w, 3), NOW);
            for line in &f.lines {
                assert!(line.width() <= usize::from(w), "w={w}: {}", text(line));
            }
        }
        // Narrow margin: no ellipsis, short hash, subject fills the row.
        let f = render_commits_at(&d, Rect::new(0, 0, 20, 3), NOW);
        let row = text(&f.lines[0]);
        assert_eq!(row, "● e4b4d3a a long sub");
        let f = render_commits_at(&d, Rect::new(0, 0, 30, 3), NOW);
        let row = text(&f.lines[0]);
        assert!(!row.contains("...") && !row.contains('…'), "{row}");
        assert!(!row.contains('+'), "stats dropped first: {row}");
        assert!(row.ends_with(" 2m"), "age kept: {row}");
    }
}

#[cfg(test)]
mod parse_tests {
    use crate::tui::app::helpers::parse_recent_commits;

    #[test]
    fn parses_git_log_shortstat_records() {
        let text = "\x1ee4b4d3ad6\x1f1790663328\x1ftui: swarm dock\n\n 3 files changed, 363 insertions(+), 35 deletions(-)\n\
                    \x1efdad4643d\x1f1790662972\x1fadd gallery\n\n 1 file changed, 209 insertions(+)\n\
                    \x1eaaaaaaa\x1f1790660000\x1fempty commit\n";
        let commits = parse_recent_commits(text, 2);
        assert_eq!(commits.len(), 3);
        assert_eq!(commits[0].hash, "e4b4d3ad6");
        assert_eq!(commits[0].subject, "tui: swarm dock");
        assert_eq!(commits[0].timestamp, 1_790_663_328);
        assert_eq!(
            (commits[0].added, commits[0].removed),
            (Some(363), Some(35))
        );
        assert_eq!((commits[1].added, commits[1].removed), (Some(209), Some(0)));
        assert_eq!((commits[2].added, commits[2].removed), (None, None));
        assert!(commits[0].unpushed && commits[1].unpushed && !commits[2].unpushed);
    }
}
