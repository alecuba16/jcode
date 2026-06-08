//! `jcode menubar` - a lightweight live indicator of how many jcode sessions
//! are running and how many are actively streaming a model response.
//!
//! On macOS this renders a native menu bar (`NSStatusItem`) item that updates
//! roughly once a second by reading the on-disk active-pid / streaming
//! registries (see `crate::session::session_counts`). On other platforms (and
//! with `--once` / `--json`) it just prints the current counts.

use anyhow::Result;
use serde::Serialize;

use crate::session::{self, SessionCounts};

#[derive(Debug, Serialize)]
struct CountsReport {
    total: usize,
    streaming: usize,
}

impl From<SessionCounts> for CountsReport {
    fn from(counts: SessionCounts) -> Self {
        Self {
            total: counts.total,
            streaming: counts.streaming,
        }
    }
}

/// Format the compact title shown in the menu bar, e.g.
/// "2 streaming 5 sessions".
pub(crate) fn format_menubar_title(counts: SessionCounts) -> String {
    format!("{} streaming {} sessions", counts.streaming, counts.total)
}

/// Human-readable one-line summary used for `--once` and the menu header.
pub(crate) fn format_menubar_summary(counts: SessionCounts) -> String {
    format!(
        "{} streaming · {} session{} running",
        counts.streaming,
        counts.total,
        if counts.total == 1 { "" } else { "s" }
    )
}

pub fn run_menubar_command(once: bool, json: bool) -> Result<()> {
    if json {
        let report = CountsReport::from(session::session_counts());
        println!("{}", serde_json::to_string(&report)?);
        return Ok(());
    }

    if once {
        println!("{}", format_menubar_summary(session::session_counts()));
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        macos::run_status_item_app();
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        eprintln!(
            "The live menu bar indicator is only available on macOS. \
             Showing current counts instead (use --once or --json for scripting):"
        );
        println!("{}", format_menubar_summary(session::session_counts()));
        Ok(())
    }
}

/// Ensure a single background `jcode menubar` helper is running on macOS so the
/// session-count indicator shows up automatically for every macOS user without
/// them needing to run `jcode menubar` by hand.
///
/// This is a best-effort, fire-and-forget singleton: it records the helper's
/// PID in `~/.jcode/menubar.pid` and only spawns a new detached process when no
/// live helper is already running. Failures are silently ignored so they never
/// disrupt normal session startup.
#[cfg(target_os = "macos")]
pub fn ensure_menubar_helper_running() {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    // Allow users to opt out entirely.
    if std::env::var_os("JCODE_NO_MENUBAR").is_some() {
        return;
    }

    let Ok(dir) = crate::storage::jcode_dir() else {
        return;
    };
    let pid_path = dir.join("menubar.pid");

    // If a recorded helper PID is still alive, do nothing.
    if let Ok(raw) = std::fs::read_to_string(&pid_path) {
        if let Ok(pid) = raw.trim().parse::<u32>() {
            if crate::platform::is_process_running(pid) {
                return;
            }
        }
    }

    let Ok(exe) = std::env::current_exe() else {
        return;
    };

    let mut command = Command::new(exe);
    command
        .arg("menubar")
        .env("JCODE_NO_MENUBAR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // Detach from the parent's process group so the helper outlives this session.
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }

    if let Ok(child) = command.spawn() {
        let _ = std::fs::write(&pid_path, child.id().to_string());
    }
}

#[cfg(not(target_os = "macos"))]
pub fn ensure_menubar_helper_running() {}

#[cfg(target_os = "macos")]
mod macos {
    use super::{format_menubar_summary, format_menubar_title};
    use crate::session;

    use objc2::MainThreadMarker;
    use objc2::MainThreadOnly;
    use objc2::rc::Retained;
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSMenu, NSMenuItem, NSStatusBar,
        NSStatusItem, NSVariableStatusItemLength,
    };
    use objc2_foundation::{NSString, ns_string};

    /// Poll interval for refreshing the counts (milliseconds).
    const REFRESH_INTERVAL_MS: u64 = 1000;

    pub(super) fn run_status_item_app() {
        let mtm = MainThreadMarker::new()
            .expect("jcode menubar must run on the main thread (the process entry point)");

        let app = NSApplication::sharedApplication(mtm);
        // Accessory: no Dock icon, no main menu, just a menu bar item.
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

        let status_bar = NSStatusBar::systemStatusBar();
        let status_item: Retained<NSStatusItem> =
            status_bar.statusItemWithLength(NSVariableStatusItemLength);

        // Build the dropdown menu (header summary + quit).
        let menu = NSMenu::new(mtm);
        let summary_item = NSMenuItem::new(mtm);
        summary_item.setEnabled(false);
        menu.addItem(&summary_item);
        menu.addItem(&NSMenuItem::separatorItem(mtm));

        let quit_item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                ns_string!("Quit jcode menu bar"),
                Some(objc2::sel!(terminate:)),
                ns_string!("q"),
            )
        };
        menu.addItem(&quit_item);
        status_item.setMenu(Some(&menu));

        let refresh = move || {
            let counts = session::session_counts();
            let title = format_menubar_title(counts);
            let summary = format_menubar_summary(counts);
            if let Some(button) = status_item.button(mtm) {
                button.setTitle(&NSString::from_str(&title));
            }
            summary_item.setTitle(&NSString::from_str(&summary));
        };

        // Initial render before the run loop starts spinning.
        refresh();

        spawn_refresh_timer(refresh);

        // Run the Cocoa event loop. `terminate:` (the Quit item) exits the process.
        app.run();
    }

    /// Schedule a repeating timer on the main run loop that re-renders the item.
    fn spawn_refresh_timer<F>(refresh: F)
    where
        F: Fn() + 'static,
    {
        use std::ptr::NonNull;

        use objc2_foundation::{NSDefaultRunLoopMode, NSRunLoop, NSTimer};

        let interval = REFRESH_INTERVAL_MS as f64 / 1000.0;
        let block = block2::RcBlock::new(move |_timer: NonNull<NSTimer>| {
            refresh();
        });

        unsafe {
            let timer = NSTimer::timerWithTimeInterval_repeats_block(interval, true, &block);
            let run_loop = NSRunLoop::currentRunLoop();
            run_loop.addTimer_forMode(&timer, NSDefaultRunLoopMode);
            // The run loop retains the timer; keep our reference alive too so the
            // owned closure (and its captured `status_item`) lives for the whole
            // process lifetime.
            std::mem::forget(timer);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::SessionCounts;

    #[test]
    fn title_idle_shows_streaming_and_total() {
        let title = format_menubar_title(SessionCounts {
            total: 5,
            streaming: 0,
        });
        assert_eq!(title, "0 streaming 5 sessions");
    }

    #[test]
    fn title_streaming_shows_counts() {
        let title = format_menubar_title(SessionCounts {
            total: 5,
            streaming: 2,
        });
        assert_eq!(title, "2 streaming 5 sessions");
    }

    #[test]
    fn summary_pluralizes_sessions() {
        assert_eq!(
            format_menubar_summary(SessionCounts {
                total: 1,
                streaming: 0,
            }),
            "0 streaming · 1 session running"
        );
        assert_eq!(
            format_menubar_summary(SessionCounts {
                total: 3,
                streaming: 1,
            }),
            "1 streaming · 3 sessions running"
        );
    }

    #[test]
    fn counts_report_serializes_to_json() {
        let report = CountsReport::from(SessionCounts {
            total: 4,
            streaming: 2,
        });
        let json = serde_json::to_string(&report).unwrap();
        assert_eq!(json, r#"{"total":4,"streaming":2}"#);
    }
}
