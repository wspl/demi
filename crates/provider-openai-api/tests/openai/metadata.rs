//! What the provider says about itself: its entry, its needs, its state and
//! its model directory.

use demi_core::{AuthState, RuntimeState, Timestamp};
use demi_provider::{Provider, testing::MockVendor};
use demi_provider_openai_api::{VendorPolicy, WireApi};

use crate::provider_at;

#[tokio::test]
async fn the_provider_names_its_entry_and_its_wire_and_is_ready_with_its_key() {
    let vendor = MockVendor::start().await;
    let responses = provider_at(&vendor, "/v1", WireApi::Responses, VendorPolicy::default());
    assert_eq!(
        (responses.id(), responses.display_name()),
        ("openai-work", "Work")
    );
    assert!(!responses.capabilities().process_host);
    assert_eq!(
        responses.auth_status().await,
        AuthState::Authenticated {
            account_label: None
        }
    );
    let message = |state: RuntimeState| match state {
        RuntimeState::Ready { message } => message,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        message(responses.runtime_state()).as_deref(),
        Some("Uses the OpenAI Responses API")
    );
    let chat = provider_at(
        &vendor,
        "/v1",
        WireApi::ChatCompletions,
        VendorPolicy::default(),
    );
    assert_eq!(
        message(chat.runtime_state()).as_deref(),
        Some("Uses the OpenAI Chat Completions API")
    );
    assert!(responses.quota().is_none() && responses.accounts().is_none());
}

#[tokio::test]
async fn the_directory_is_built_in_with_the_newest_gpt_first() {
    let vendor = MockVendor::start().await;
    let list = provider_at(&vendor, "/v1", WireApi::Responses, VendorPolicy::default())
        .list_models()
        .await
        .unwrap();
    let ids: Vec<&str> = list.models.iter().map(|model| model.id.as_str()).collect();
    assert_eq!(
        ids,
        ["gpt-5.5", "gpt-5.4", "gpt-5.4-mini", "gpt-5.3-codex-spark"]
    );
    assert_eq!(list.default_model_id.as_deref(), Some("gpt-5.5"));
    assert_eq!(
        (list.source_fetched_at, list.stale),
        (Timestamp::UNIX_EPOCH, false)
    );
    let first = &list.models[0];
    assert_eq!(
        (
            first.display_name.as_str(),
            first.context_window,
            first.output_limit
        ),
        ("GPT-5.5", Some(272_000), None)
    );
    assert_eq!(
        first.supported_thinking_efforts.as_deref(),
        Some(&["low", "medium", "high", "xhigh"].map(String::from)[..])
    );
    let tier = &first.service_tiers[0];
    assert_eq!(
        (tier.id.as_str(), tier.label.as_str(), tier.fast),
        ("priority", "Fast", true)
    );
    assert!(list.models[2].service_tiers.is_empty());
    assert_eq!(list.models[3].supports_attachments, Some(false));
    assert!(
        vendor.requests().is_empty(),
        "reading the directory makes no request"
    );
}
