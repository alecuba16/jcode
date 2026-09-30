//! Tests for the current-directory pinned group in the session picker: the
//! sessions whose working directory matches the folder `/resume` was opened
//! from float to the top of the All view under their own header.

use super::*;
use crate::tui::session_picker::filter::short_dir_label;
use chrono::{Duration as ChronoDuration, Utc};

#[test]
fn test_current_dir_sessions_pin_to_top_with_header() {
    let mut old_here = make_session("old_here", "old-here", false, SessionStatus::Closed);
    old_here.working_dir = Some("/home/jeremy/project".to_string());
    old_here.last_message_time = Utc::now() - ChronoDuration::hours(2);
    let mut new_here = make_session("new_here", "new-here", false, SessionStatus::Closed);
    new_here.working_dir = Some("/home/jeremy/project/".to_string());
    new_here.last_message_time = Utc::now() - ChronoDuration::minutes(1);
    let mut elsewhere = make_session("elsewhere", "elsewhere", false, SessionStatus::Closed);
    elsewhere.working_dir = Some("/home/jeremy/elsewhere".to_string());
    elsewhere.last_message_time = Utc::now() - ChronoDuration::minutes(30);

    let mut picker =
        SessionPicker::new(vec![elsewhere.clone(), old_here.clone(), new_here.clone()]);
    picker.set_current_dir(Some("/home/jeremy/project".to_string()));
    picker.rebuild_items();

    // The pinned group header is the first item, followed by the current-dir
    // sessions in newest-first order, then the rest.
    assert!(matches!(
        picker.items.first(),
        Some(PickerItem::CurrentDirHeader { label, session_count })
            if *label == "/home/jeremy/project" && *session_count == 2
    ));
    let visible_ids: Vec<&str> = picker
        .visible_session_iter()
        .map(|session| session.id.as_str())
        .collect();
    assert_eq!(visible_ids, vec!["new_here", "old_here", "elsewhere"]);

    // Every item maps to the same visible-session order, so the preview and
    // navigation agree with the pinned layout.
    let first_session_item = picker
        .item_to_session
        .iter()
        .position(|slot| slot.is_some())
        .expect("at least one selectable item");
    assert!(matches!(
        picker.items[first_session_item],
        PickerItem::Session
    ));
    assert_eq!(picker.list_state.selected(), Some(first_session_item));
}

#[test]
fn test_current_dir_group_absent_without_matches_or_current_dir() {
    // No current_dir set: identical to the classic flat list.
    let session = make_session("plain", "plain", false, SessionStatus::Closed);
    let mut picker = SessionPicker::new(vec![session]);
    picker.rebuild_items();
    assert_eq!(picker.items.len(), 1);
    assert!(matches!(picker.items[0], PickerItem::Session));

    // current_dir set but no session matches: no pinned group is added.
    let mut session = make_session("nomatch", "nomatch", false, SessionStatus::Closed);
    session.working_dir = Some("/home/jeremy/other".to_string());
    let mut picker = SessionPicker::new(vec![session]);
    picker.set_current_dir(Some("/home/jeremy/project".to_string()));
    picker.rebuild_items();
    assert_eq!(picker.items.len(), 1);
    assert!(matches!(picker.items[0], PickerItem::Session));
}

#[test]
fn test_current_dir_sessions_not_duplicated_in_saved_or_other_sections() {
    let mut saved_here = make_session("saved_here", "saved-here", false, SessionStatus::Closed);
    saved_here.working_dir = Some("/home/jeremy/project".to_string());
    saved_here.saved = true;
    let mut saved_elsewhere = make_session(
        "saved_elsewhere",
        "saved-elsewhere",
        false,
        SessionStatus::Closed,
    );
    saved_elsewhere.working_dir = Some("/home/jeremy/elsewhere".to_string());
    saved_elsewhere.saved = true;

    let mut picker = SessionPicker::new(vec![saved_here.clone(), saved_elsewhere.clone()]);
    picker.set_current_dir(Some("/home/jeremy/project".to_string()));
    picker.rebuild_items();

    // The saved session from the current dir stays in the pinned group (with
    // its pin badge on the row) and does not also appear under Saved.
    let visible_ids: Vec<&str> = picker
        .visible_session_iter()
        .map(|session| session.id.as_str())
        .collect();
    assert_eq!(visible_ids, vec!["saved_here", "saved_elsewhere"]);
    let headers: Vec<bool> = picker
        .items
        .iter()
        .map(|item| {
            matches!(item, PickerItem::CurrentDirHeader { .. })
                || matches!(item, PickerItem::SavedHeader { .. })
        })
        .collect();
    // header, pinned row, saved header, saved row
    assert_eq!(
        headers,
        vec![true, false, true, false],
        "expected the pinned-group and saved headers, in that order"
    );
    assert!(matches!(
        picker.items[0],
        PickerItem::CurrentDirHeader {
            session_count: 1,
            ..
        }
    ));
    assert!(matches!(
        picker.items[2],
        PickerItem::SavedHeader { session_count: 1 }
    ));
}

#[test]
fn test_current_dir_group_follows_search_and_skips_grouped_views() {
    let mut here_a = make_session("here_a", "here-a", false, SessionStatus::Closed);
    here_a.working_dir = Some("/home/jeremy/project".to_string());
    let mut here_b = make_session("here_b", "here-b", false, SessionStatus::Closed);
    here_b.working_dir = Some("/home/jeremy/project".to_string());

    let mut picker = SessionPicker::new(vec![here_a.clone(), here_b.clone()]);
    picker.set_current_dir(Some("/home/jeremy/project".to_string()));

    // Searching narrows the pinned group instead of bypassing it. The search
    // index is built at construction, so search by the session id.
    picker.start_search("here_a");
    assert_eq!(picker.visible_sessions.len(), 1);
    assert_eq!(
        picker.visible_session_iter().next().map(|s| s.id.as_str()),
        Some("here_a")
    );
    assert!(matches!(
        picker.items.first(),
        Some(PickerItem::CurrentDirHeader { .. })
    ));

    // Filter modes other than All are flat lists: no pinned header there.
    picker.search_active = false;
    picker.search_query.clear();
    picker.cycle_filter_mode();
    assert_eq!(picker.filter_mode, SessionFilterMode::CurrentDir);
    assert!(
        picker
            .items
            .iter()
            .all(|item| matches!(item, PickerItem::Session))
    );
}

#[test]
fn test_current_dir_group_pinned_across_server_groups_and_orphans() {
    let mut grouped_here =
        make_session("grouped_here", "grouped-here", false, SessionStatus::Closed);
    grouped_here.working_dir = Some("/home/jeremy/project".to_string());
    grouped_here.last_message_time = Utc::now() - ChronoDuration::minutes(1);
    let mut orphan_here = make_session("orphan_here", "orphan-here", false, SessionStatus::Closed);
    orphan_here.working_dir = Some("/home/jeremy/project/".to_string());
    orphan_here.last_message_time = Utc::now() - ChronoDuration::minutes(5);
    let mut orphan_elsewhere = make_session(
        "orphan_elsewhere",
        "orphan-elsewhere",
        false,
        SessionStatus::Closed,
    );
    orphan_elsewhere.working_dir = Some("/home/jeremy/elsewhere".to_string());
    orphan_elsewhere.last_message_time = Utc::now() - ChronoDuration::minutes(10);

    let groups = vec![ServerGroup {
        name: "main".to_string(),
        icon: "🛰".to_string(),
        version: "v0.1.0".to_string(),
        git_hash: "abc1234".to_string(),
        is_running: true,
        sessions: vec![grouped_here.clone()],
    }];

    let mut picker =
        SessionPicker::new_grouped(groups, vec![orphan_here.clone(), orphan_elsewhere.clone()]);
    picker.set_current_dir(Some("/home/jeremy/project".to_string()));
    picker.rebuild_items();

    // Pinned group first; the current-dir rows are pulled out of their
    // original sections (the server group becomes empty and is skipped), then
    // the remaining orphans under their header.
    let visible_ids: Vec<&str> = picker
        .visible_session_iter()
        .map(|session| session.id.as_str())
        .collect();
    assert_eq!(
        visible_ids,
        vec!["grouped_here", "orphan_here", "orphan_elsewhere"]
    );
    assert!(matches!(
        picker.items[0],
        PickerItem::CurrentDirHeader {
            session_count: 2,
            ..
        }
    ));
    assert!(matches!(
        picker.items[3],
        PickerItem::OrphanHeader { session_count: 1 }
    ));
}

#[test]
fn test_short_dir_label_collapses_long_paths_to_basename() {
    // Short paths render in full.
    assert_eq!(
        short_dir_label("/home/jeremy/project"),
        "/home/jeremy/project"
    );
    // Long home-style paths collapse to ~/basename.
    let long_home = "/Users/jeremy/Documents/deeply/nested/projects/jcode";
    assert_eq!(short_dir_label(long_home), "~/jcode");
    let long_home2 = "/home/jeremy/Documents/deeply/nested/projects/jcode";
    assert_eq!(short_dir_label(long_home2), "~/jcode");
    // Long non-home paths collapse to /…/basename.
    let long_other = "/srv/workspace/very/deeply/nested/project";
    assert_eq!(short_dir_label(long_other), "/…/project");
}

#[test]
fn test_reseed_grouped_keeps_pinned_group_and_selection() {
    // Simulate the async `/resume` refresh: the cached list renders first,
    // then the freshly loaded session data lands via `reseed_grouped`. The
    // pinned group must survive the swap and the user's selection with it.
    let mut here = make_session("here", "here", false, SessionStatus::Closed);
    here.working_dir = Some("/home/jeremy/project".to_string());
    let mut elsewhere = make_session("elsewhere", "elsewhere", false, SessionStatus::Closed);
    elsewhere.working_dir = Some("/home/jeremy/elsewhere".to_string());

    let mut picker = SessionPicker::new_grouped(Vec::new(), vec![here, elsewhere]);
    picker.set_current_dir(Some("/home/jeremy/project".to_string()));
    picker.rebuild_items();
    assert!(matches!(
        picker.items.first(),
        Some(PickerItem::CurrentDirHeader { .. })
    ));

    // Select the pinned session, then reseed with an extended list (the
    // refresh adds a second current-dir session and a new orphan).
    let pinned_idx = picker
        .item_to_session
        .iter()
        .position(|slot| slot.is_some())
        .expect("selectable item");
    picker.list_state.select(Some(pinned_idx));
    assert_eq!(
        picker.selected_session().map(|s| s.id.as_str()),
        Some("here")
    );

    let mut here2 = make_session("here2", "here2", false, SessionStatus::Closed);
    here2.working_dir = Some("/home/jeremy/project".to_string());
    let mut other = make_session("other", "other", false, SessionStatus::Closed);
    other.working_dir = Some("/home/jeremy/other".to_string());
    picker.reseed_grouped(Vec::new(), vec![here2, other]);

    // Pinned group still on top; the reseeded list has only one
    // current-dir session (here2), so the header count reflects that.
    assert!(matches!(
        picker.items.first(),
        Some(PickerItem::CurrentDirHeader {
            session_count: 1,
            ..
        })
    ));
    // The selected session survived by id ("here" is no longer in the list,
    // so selection falls back to the first selectable item).
    let selected_id = picker
        .selected_session()
        .map(|s| s.id.as_str())
        .expect("a selection exists after reseed");
    assert!(
        ["here", "here2"].contains(&selected_id),
        "selection should resolve to a current-dir session, got {selected_id}"
    );
}
