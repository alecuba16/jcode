// Pure parse/round-trip tests for provider-selection helpers, split
// from provider_init_tests.rs to stay under the test-size budget.
use super::*;
use crate::external_auth::parse_external_auth_review_selection;

#[test]
fn parse_external_auth_review_selection_supports_all_and_deduped_indices() {
    assert_eq!(
        parse_external_auth_review_selection("", 3).unwrap(),
        Vec::<usize>::new()
    );
    assert_eq!(
        parse_external_auth_review_selection("a", 3).unwrap(),
        vec![0, 1, 2]
    );
    assert_eq!(
        parse_external_auth_review_selection("2,1,2", 3).unwrap(),
        vec![1, 0]
    );
    assert!(parse_external_auth_review_selection("4", 3).is_err());
    assert!(parse_external_auth_review_selection("nope", 3).is_err());
}

#[test]
fn parse_login_provider_selection_supports_skip_and_names() {
    let providers = provider_catalog::cli_login_providers();

    assert!(
        parse_login_provider_selection_input("", &providers)
            .unwrap()
            .is_none()
    );
    assert!(
        parse_login_provider_selection_input("skip", &providers)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        parse_login_provider_selection_input("claude", &providers)
            .unwrap()
            .map(|provider| provider.id),
        Some("claude")
    );
    let first_provider = providers[0].id;
    assert_eq!(
        parse_login_provider_selection_input("1", &providers)
            .unwrap()
            .map(|provider| provider.id),
        Some(first_provider)
    );
    assert!(parse_login_provider_selection_input("not-a-provider", &providers).is_err());
}

#[test]
fn choice_for_login_provider_round_trips_core_targets() {
    assert_eq!(
        choice_for_login_provider(provider_catalog::JCODE_LOGIN_PROVIDER),
        Some(ProviderChoice::Jcode)
    );
    assert_eq!(
        choice_for_login_provider(provider_catalog::OPENROUTER_LOGIN_PROVIDER),
        Some(ProviderChoice::Openrouter)
    );
    assert_eq!(
        choice_for_login_provider(provider_catalog::ANTHROPIC_API_LOGIN_PROVIDER),
        Some(ProviderChoice::AnthropicApi)
    );
    assert_eq!(
        choice_for_login_provider(provider_catalog::AZURE_LOGIN_PROVIDER),
        Some(ProviderChoice::Azure)
    );
    assert_eq!(
        choice_for_login_provider(provider_catalog::CURSOR_LOGIN_PROVIDER),
        Some(ProviderChoice::Cursor)
    );
    assert_eq!(
        choice_for_login_provider(provider_catalog::AUTO_IMPORT_LOGIN_PROVIDER),
        None
    );
}

#[test]
fn choice_for_login_provider_round_trips_openai_compatible_profiles() {
    assert_eq!(
        choice_for_login_provider(provider_catalog::OPENCODE_LOGIN_PROVIDER),
        Some(ProviderChoice::Opencode)
    );
    assert_eq!(
        choice_for_login_provider(provider_catalog::LMSTUDIO_LOGIN_PROVIDER),
        Some(ProviderChoice::Lmstudio)
    );
    assert_eq!(
        choice_for_login_provider(provider_catalog::OPENAI_COMPAT_LOGIN_PROVIDER),
        Some(ProviderChoice::OpenaiCompatible)
    );
}
