//! Lenient serde helpers for string-valued configuration enums.
//!
//! A single unrecognized enum value used to abort parsing of the *entire*
//! `config.toml`, so `Config::load` silently fell back to `Config::default()`
//! and every unrelated setting looked like it was ignored (issue #689: one
//! `diagram_mode = "inline"` line also disabled `centered`, `idle_animation`,
//! `show_thinking`, and `reasoning_display`). Degrading the one field that is
//! actually wrong is far better than discarding the user's configuration.

use serde::Deserialize;

/// Deserialize a string-valued enum, falling back to its `Default` when the
/// value is not recognized.
pub(crate) fn lenient_enum<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    let raw = String::deserialize(deserializer)?;
    let value = serde::de::value::StrDeserializer::<serde::de::value::Error>::new(raw.as_str());
    Ok(T::deserialize(value).unwrap_or_default())
}

/// Deserialize a bool, accepting bare booleans, quoted boolean-like strings
/// ("on"/"off"/"true"/...), and integers (1/0). An unrecognized value falls
/// back to the *default* (`true`) so one bad line degrades only that field
/// instead of aborting the entire `[display]` parse (issue #689 pattern for
/// bool fields; struct-level `#[serde(default)]` only covers *missing*
/// fields, an erroring field still fails the whole struct).
pub(crate) fn lenient_bool_true<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct LenientBoolVisitor;

    impl<'de> serde::de::Visitor<'de> for LenientBoolVisitor {
        type Value = bool;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a boolean or a boolean-like string")
        }

        fn visit_bool<E>(self, value: bool) -> Result<bool, E> {
            Ok(value)
        }

        fn visit_i64<E>(self, value: i64) -> Result<bool, E> {
            Ok(value != 0)
        }

        fn visit_u64<E>(self, value: u64) -> Result<bool, E> {
            Ok(value != 0)
        }

        fn visit_str<E>(self, value: &str) -> Result<bool, E> {
            Ok(match value.trim().to_ascii_lowercase().as_str() {
                "1" | "true" | "yes" | "on" | "enable" | "enabled" => true,
                "0" | "false" | "no" | "off" | "disable" | "disabled" => false,
                // Unknown value: keep the default rather than failing the
                // whole config.toml parse.
                _ => true,
            })
        }
    }

    // `deserialize_any` so a quoted TOML string is handled by `visit_str`
    // rather than failing to type-check as a bool.
    deserializer.deserialize_any(LenientBoolVisitor)
}

/// `Option` variant of [`lenient_enum`]: an unrecognized value becomes `None`
/// ("not configured") rather than a config-wide parse failure.
pub(crate) fn lenient_optional_enum<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    let Some(raw) = Option::<String>::deserialize(deserializer)? else {
        return Ok(None);
    };
    let value = serde::de::value::StrDeserializer::<serde::de::value::Error>::new(raw.as_str());
    Ok(T::deserialize(value).ok())
}

/// Regression tests for issue #689: an unrecognized value in one `[display]`
/// enum field must not discard the rest of the user's configuration.
#[cfg(test)]
mod tests {
    use crate::{
        DiagramDisplayMode, DiffDisplayMode, DisplayConfig, LatexRenderingMode,
        MarkdownSpacingMode, OverscrollStatusMode, ReasoningDisplayMode,
    };

    fn parse(json: &str) -> DisplayConfig {
        serde_json::from_str(json).expect("display config must still parse")
    }

    #[test]
    fn lenient_bool_accepts_boolean_like_strings_and_keeps_default_on_unknown() {
        let accepts = |raw, expected: bool| {
            let cfg = parse(&format!("{{\"show_tps\": {raw}}}"));
            assert_eq!(cfg.show_tps, expected, "raw value: {raw}");
        };
        accepts("true", true);
        accepts("false", false);
        accepts("\"on\"", true);
        accepts("\"off\"", false);
        accepts("\"yes\"", true);
        accepts("\"no\"", false);
        accepts("1", true);
        accepts("0", false);
        // Unknown spellings degrade to the field default instead of
        // discarding the rest of the section.
        accepts("\"bogus\"", true);
    }

    #[test]
    fn unknown_diagram_mode_keeps_the_rest_of_the_display_config() {
        let display = parse(
            r#"{
                "centered": true,
                "idle_animation": true,
                "show_thinking": true,
                "reasoning_display": "current",
                "diagram_mode": "totally-bogus"
            }"#,
        );

        assert!(display.centered, "unrelated settings must survive");
        assert!(display.idle_animation);
        assert!(display.show_thinking);
        assert_eq!(display.reasoning_display(), ReasoningDisplayMode::Current);
        assert_eq!(display.diagram_mode, DiagramDisplayMode::default());
    }

    /// The exact config from the issue report: `diagram_mode = "inline"` is now
    /// accepted as the inline-only (default) mode.
    #[test]
    fn diagram_mode_inline_is_accepted_as_the_default_mode() {
        let display = parse(r#"{"centered": true, "diagram_mode": "inline"}"#);
        assert!(display.centered);
        assert_eq!(display.diagram_mode, DiagramDisplayMode::None);
    }

    #[test]
    fn unknown_reasoning_display_falls_back_without_losing_other_fields() {
        let display = parse(r#"{"centered": true, "reasoning_display": "nope"}"#);
        assert!(display.centered);
        assert!(
            !display.has_explicit_reasoning_display(),
            "an unparseable value means 'not configured'"
        );
    }

    #[test]
    fn unknown_diff_and_latex_modes_fall_back_to_defaults() {
        let display = parse(
            r#"{"centered": true, "diff_mode": "nope", "latex_rendering": "nope",
                "markdown_spacing": "nope", "overscroll_status": "nope"}"#,
        );
        assert!(display.centered);
        assert_eq!(display.diff_mode, DiffDisplayMode::default());
        assert_eq!(display.latex_rendering, LatexRenderingMode::default());
        assert_eq!(display.markdown_spacing, MarkdownSpacingMode::default());
        assert_eq!(display.overscroll_status, OverscrollStatusMode::default());
    }

    /// Valid values must still round-trip (leniency must not swallow them).
    #[test]
    fn valid_values_are_unaffected() {
        let display = parse(r#"{"diagram_mode": "pinned", "diff_mode": "full-inline"}"#);
        assert_eq!(display.diagram_mode, DiagramDisplayMode::Pinned);
        assert_eq!(display.diff_mode, DiffDisplayMode::FullInline);
    }
}
