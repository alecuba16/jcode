# TUI themes

jcode ships with `light`, `dark`, and `system` themes. The default is `light`.

Switch themes in the TUI with:

```text
/theme light
/theme dark
/theme system
/theme <custom-name>
```

The choice is saved as `display.theme` in `~/.jcode/config.toml`.

Custom themes live in `~/.jcode/themes/<name>.toml`. Omitted colors inherit from the built-in light theme.

```toml
[colors]
user = "#1c58a0"
ai = "#267848"
tool = "#5c5c5c"
file_link = "#1c5cb0"
dim = "#696969"
accent = "#7048aa"
system_message = "#b03a7e"
queued = "#9e6a00"
asap = "#007096"
pending = "#707070"
user_text = "#1e1e22"
user_bg = "#f4f1ec"
ai_text = "#242628"
header_icon = "#007a96"
header_name = "#2c507a"
header_session = "#222226"
```

Use `"reset"` or `"default"` for terminal-provided colors.
