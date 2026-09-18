//! Runtime image-input capability overrides.
//!
//! Static capability knowledge (`Provider::supports_image_input`) can be wrong
//! for direct OpenAI-compatible endpoints: the default there is optimistic
//! (any `/v1/chat/completions` server "might" accept `image_url` parts), and a
//! text-only model behind such an endpoint advertises image support it does not
//! have. When that happens the request fails with a modality error only after
//! the image is already in the request, and every later request repeats the
//! failure because the stale `true` answer keeps the images flowing.
//!
//! This module records per `(provider-key, model)` that the endpoint *actually*
//! rejected image input, with a TTL. `MultiProvider::supports_image_input`
//! consults the override first, so:
//!
//! - `complete_with_failover` replaces image blocks with text markers for the
//!   very next request (the existing #755 filter), and
//! - turn loops stop attaching new tool-result/generated images while the
//!   override is fresh.
//!
//! The override is scoped to the model: switching to a vision-capable model on
//! the same endpoint clears the penalty (and a manual `/model` switch clears
//! every entry for the runtime anyway via `clear_image_input_override`).
//!
//! Unlike `ACCOUNT_RUNTIME_UNAVAILABLE_PROVIDERS`, these entries never steer
//! provider failover: the endpoint is healthy, only the image modality is
//! unsupported, so we degrade the request instead of switching providers.

use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};
use std::time::{Duration, Instant, SystemTime};

/// How long a recorded "this model rejected images" observation stays
/// authoritative. Long enough to cover a long session, short enough that a
/// server-side upgrade (or a wrongly classified error) heals itself.
const IMAGE_INPUT_OVERRIDE_TTL: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone)]
struct ImageInputRejection {
    recorded_at: Instant,
    #[allow(dead_code)] // kept for parity with other runtime records; useful in future dumps
    observed_at: SystemTime,
    #[allow(dead_code)] // debug aid when inspecting why a request was filtered
    matched: String,
}

static IMAGE_INPUT_REJECTIONS: LazyLock<RwLock<HashMap<String, ImageInputRejection>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Error fragments that mean "this model/endpoint cannot accept image input".
///
/// Kept deliberately narrow: generic schema errors (e.g. a missing tool field)
/// must not be treated as a modality problem. All needles are matched
/// case-insensitively against the full error text.
const IMAGE_REJECTION_NEEDLES: &[&str] = &[
    // Anthropic-style
    "does not support image",
    "images are not supported",
    "image input is not supported",
    "unsupported content type: image",
    "does not accept image",
    "cannot process images",
    "no image support",
    // OpenAI / OpenAI-compatible style
    "does not support image input",
    "image input not supported",
    "not support image_url",
    "invalid content type: image_url",
    "unsupported content type: image_url",
    "image_url is not supported",
    "image content is not supported",
    "not multimodal",
    "is not multimodal",
    "does not support multimodal",
    "text-only model",
    "only supports text",
    "only supports text input",
    "image is not a supported modality",
    "modality not supported",
    "unsupported modality",
    "input modalities",
];

/// True when `error_text` looks like the endpoint rejected image input.
pub(crate) fn error_indicates_image_rejection(error_text: &str) -> bool {
    let lower = error_text.to_ascii_lowercase();
    IMAGE_REJECTION_NEEDLES
        .iter()
        .any(|needle| lower.contains(needle))
}

/// Record that `model` on `provider_key` rejected image input.
///
/// `provider_key` must already be runtime-scoped (see
/// `MultiProvider::provider_key`), because the same provider slot can serve
/// different accounts with different capabilities.
pub(crate) fn record_image_input_rejection(provider_key: &str, model: &str, error_text: &str) {
    let model = model.trim();
    if model.is_empty() || provider_key.trim().is_empty() {
        return;
    }
    let matched = IMAGE_REJECTION_NEEDLES
        .iter()
        .find(|needle| error_text.to_ascii_lowercase().contains(*needle))
        .map(|needle| needle.to_string())
        .unwrap_or_else(|| "unclassified".to_string());
    let entry_key = format!("{provider_key}::{model}");
    if let Ok(mut rejections) = IMAGE_INPUT_REJECTIONS.write() {
        rejections.insert(
            entry_key,
            ImageInputRejection {
                recorded_at: Instant::now(),
                observed_at: SystemTime::now(),
                matched: matched.clone(),
            },
        );
    }
    crate::logging::info(&format!(
        "Recorded image-input rejection for {provider_key}/{model} (matched: {}). \
         Outbound images will be filtered to text markers until the model changes.",
        matched
    ));
}

/// True when a fresh "rejected image input" record exists for `model` on
/// `provider_key`. Expired entries are dropped on read.
pub(crate) fn image_input_rejected(provider_key: &str, model: &str) -> bool {
    let model = model.trim();
    if model.is_empty() || provider_key.trim().is_empty() {
        return false;
    }
    let entry_key = format!("{provider_key}::{model}");
    let Ok(mut rejections) = IMAGE_INPUT_REJECTIONS.write() else {
        return false;
    };
    match rejections.get(&entry_key) {
        Some(entry) if entry.recorded_at.elapsed() <= IMAGE_INPUT_OVERRIDE_TTL => true,
        Some(_) => {
            rejections.remove(&entry_key);
            false
        }
        None => false,
    }
}

/// Drop every override for `provider_key` (called when the model is switched
/// on that runtime, so a fresh model gets a fresh capability answer).
pub(crate) fn clear_image_input_overrides_for_provider(provider_key: &str) {
    let prefix = format!("{provider_key}::");
    if let Ok(mut rejections) = IMAGE_INPUT_REJECTIONS.write() {
        rejections.retain(|key, _| !key.starts_with(&prefix));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_provider() -> String {
        format!(
            "test-provider-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )
    }

    #[test]
    fn modality_errors_are_classified() {
        assert!(error_indicates_image_rejection(
            "Error: model does not support image input"
        ));
        assert!(error_indicates_image_rejection(
            "400: this is a text-only model"
        ));
        assert!(error_indicates_image_rejection(
            "Request failed: image_url is not supported by this model"
        ));
        assert!(error_indicates_image_rejection(
            "Invalid request: unsupported modality: image"
        ));
        assert!(error_indicates_image_rejection(
            "model is not multimodal, cannot accept image parts"
        ));
        assert!(error_indicates_image_rejection(
            "model does not support multimodal requests"
        ));
    }
    #[test]
    fn unrelated_errors_are_not_classified() {
        assert!(!error_indicates_image_rejection("rate limit exceeded"));
        assert!(!error_indicates_image_rejection(
            "tool_use ids were found without tool_result"
        ));
        assert!(!error_indicates_image_rejection(
            "max_tokens must be greater than 0"
        ));
        // Positive-sounding multimodal text must not flip a request to the
        // text-only path.
        assert!(!error_indicates_image_rejection(
            "gpt-4o is multimodal and can see images"
        ));
        assert!(!error_indicates_image_rejection(""));
    }

    #[test]
    fn rejection_roundtrip_and_clear() {
        let provider = unique_provider();
        assert!(!image_input_rejected(&provider, "text-model"));

        record_image_input_rejection(&provider, "text-model", "does not support image input");
        assert!(image_input_rejected(&provider, "text-model"));
        // Scoped per model.
        assert!(!image_input_rejected(&provider, "other-model"));

        clear_image_input_overrides_for_provider(&provider);
        assert!(!image_input_rejected(&provider, "text-model"));
    }

    #[test]
    fn empty_inputs_are_ignored() {
        let provider = unique_provider();
        record_image_input_rejection(&provider, "  ", "does not support image input");
        record_image_input_rejection("", "model", "does not support image input");
        assert!(!image_input_rejected(&provider, "  "));
        assert!(!image_input_rejected("", "model"));
    }
}
