# Improving jcode Status Detection in herdr

## Problem

Herdr's `jcode-support` branch detects jcode's agent state using **only screen-scraping** (TOML manifest regex/contains rules). This is fragile because:

1. **Spinner frames change** — `⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏` are matched per-frame, but any visual change breaks detection
2. **Status text is version-dependent** — `"sending…"`, `"thinking…"`, `"running tool"` labels change between jcode releases
3. **False positives** — generic words like `"permission"`, `"allow"`, `"deny"` appear in conversation text
4. **No transcript viewer rule** — scrollback text can confuse state detection (other agents have `skip_state_update` rules)
5. **Stale text handling** — requires complex `bottom_non_empty_lines(N)` + `not` gate tuning

## How reliable agents do it

Every well-detected agent uses **OSC sequences** as the primary detection path:

| Agent | OSC title rules | OSC progress | Screen-scrape fallback |
|-------|----------------|--------------|----------------------|
| codex | priority 1100 (spinner + "Action Required") | no | yes (lower priority) |
| claude | priority 1100 (braille prefix) | yes (`4;0`) | yes (lower priority) |
| opencode | JS integration plugin | JS plugin | yes (lower priority) |
| kilo | JS integration plugin | JS plugin | yes (lower priority) |
| **jcode** | **none** | **none** | **only path (fragile)** |

Herdr captures two OSC channels from the terminal stream (`src/pane/osc.rs`):
- **OSC 0/2** (terminal title) → `osc_title` region in manifests
- **OSC 9** (progress) → `osc_progress` region in manifests

jcode already emits OSC 0 via `crossterm::terminal::SetTitle` for the window title, but it only contains the session name — **not the processing status**.

## Proposed solution: OSC 9 progress emission

### Phase 1: jcode emits structured OSC 9 sequences

When the TUI's `ProcessingStatus` changes, emit an OSC 9 progress sequence with a stable, version-independent payload:

```
Working:  ESC ] 9 ;jcode:working BEL
Idle:     ESC ] 9 ;jcode:idle BEL
Blocked:  ESC ] 9 ;jcode:blocked BEL
```

Hook point: `update_terminal_title()` in `tui_lifecycle_runtime.rs` already runs on status changes. Add a sibling `emit_agent_status_osc()` call alongside it.

The emission should happen:
- When `app.status` transitions (Sending/Connecting/Thinking/Streaming/RunningTool → working)
- When turn finishes (→ idle)
- When permission prompt opens (→ blocked)
- On every full redraw as a heartbeat (cheap, keeps herdr in sync)

### Phase 2: Update herdr jcode.toml with OSC rules at high priority

```toml
[[rules]]
id = "osc_progress_working"
state = "working"
priority = 1100
region = "osc_progress"
visible_working = true
contains = ["jcode:working"]

[[rules]]
id = "osc_progress_blocked"
state = "blocked"
priority = 1100
region = "osc_progress"
visible_blocker = true
contains = ["jcode:blocked"]

[[rules]]
id = "osc_progress_idle"
state = "idle"
priority = 1100
region = "osc_progress"
visible_idle = true
contains = ["jcode:idle"]
```

These OSC rules win over all screen-scrape rules (current max priority is 300). Screen-scrape rules remain as fallback for environments where OSC is stripped (some terminal multiplexers).

### Phase 3: Add transcript viewer skip rule

Add a `skip_state_update` rule to detect when the user is scrolling through transcript history (matching claude/codex patterns):

```toml
[[rules]]
id = "transcript_viewer"
state = "unknown"
priority = 1000
region = "bottom_non_empty_lines(5)"
skip_state_update = true
# Match jcode scrollback indicators if any exist
```

## Benefits

1. **Version-stable** — `jcode:working` string never changes even if UI text changes
2. **No false positives** — structured payload can't appear in conversation text
3. **No stale text issues** — OSC is a live signal, not screen-scraped
4. **Cheap** — one escape sequence per status change, negligible overhead
5. **Backward compatible** — screen-scrape rules remain as fallback
6. **Consistent with ecosystem** — matches codex/claude OSC-first approach

## Implementation files

### jcode side:
- `crates/jcode-tui/src/tui/app/tui_lifecycle_runtime.rs` — add `emit_agent_status_osc()` method
- `crates/jcode-tui/src/tui/app/run_shell.rs` — call it in the redraw loop
- `crates/jcode-tui/src/tui/app.rs` — call it on status transitions

### herdr side:
- `src/detect/manifests/jcode.toml` — add OSC progress rules at priority 1100
- `website/agent-detection/jcode.toml` — mirror the bundled manifest