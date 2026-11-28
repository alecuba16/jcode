//! Memory store optimization policy: when the host may queue a model-driven
//! pass that compresses, merges, and prunes long-term memory entries.

use serde::{Deserialize, Serialize};

/// Default idle window before an automatic optimization pass may fire.
pub const DEFAULT_IDLE_SECS: u64 = 1800;
/// Default minimum spacing between automatic passes.
pub const DEFAULT_MIN_INTERVAL_SECS: u64 = 86400;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryOptimizeConfig {
    /// Queue automatic optimization turns (default: false).
    pub enabled: bool,
    /// Seconds without user interaction or model streaming before an
    /// automatic pass may fire. Zero disables the idle trigger (default: 1800).
    pub idle_secs: u64,
    /// Also treat a running background task as an optimization window, even
    /// when the session is otherwise active (default: true).
    pub trigger_on_bg_wait: bool,
    /// Minimum seconds between automatic passes (default: 86400).
    pub min_interval_secs: u64,
}

impl Default for MemoryOptimizeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            idle_secs: DEFAULT_IDLE_SECS,
            trigger_on_bg_wait: true,
            min_interval_secs: DEFAULT_MIN_INTERVAL_SECS,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_off_with_daily_cooldown() {
        let cfg = MemoryOptimizeConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.idle_secs, 1800);
        assert!(cfg.trigger_on_bg_wait);
        assert_eq!(cfg.min_interval_secs, 86400);
    }

    #[test]
    fn section_roundtrips_through_json() {
        let mut root = serde_json::json!({
            "memory_optimize": {
                "enabled": true,
                "idle_secs": 60,
                "min_interval_secs": 3600,
            }
        });
        let parsed: crate::MemoryOptimizeConfig =
            serde_json::from_value(root["memory_optimize"].clone()).expect("deserialize");
        assert!(parsed.enabled);
        assert_eq!(parsed.idle_secs, 60);
        assert_eq!(parsed.min_interval_secs, 3600);
        assert!(parsed.trigger_on_bg_wait);

        let serialized = serde_json::to_string(&parsed).expect("serialize");
        let restored: MemoryOptimizeConfig =
            serde_json::from_str(&serialized).expect("deserialize");
        assert_eq!(restored, parsed);
    }

    #[test]
    fn zero_disables_idle_trigger_and_roundtrips() {
        let parsed: crate::MemoryOptimizeConfig = serde_json::from_value(serde_json::json!({
            "enabled": true,
            "idle_secs": 0
        }))
        .expect("deserialize");
        assert!(parsed.enabled);
        assert_eq!(parsed.idle_secs, 0);
        let serialized = serde_json::to_string(&parsed).expect("serialize");
        assert!(serialized.contains("\"idle_secs\":0"));
    }
}
