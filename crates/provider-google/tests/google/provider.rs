//! What the provider says about itself: its entry, its needs, its state and
//! its model directory.

use demi_core::{AuthState, RuntimeState, Timestamp};
use demi_provider::{Provider, testing::MockVendor};

use crate::provider_at;

#[tokio::test]
async fn the_provider_names_its_entry_and_is_ready_with_its_key() {
    let vendor = MockVendor::start().await;
    let provider = provider_at(&vendor, "/v1beta");
    assert_eq!(
        (provider.id(), provider.display_name()),
        ("google-work", "Work")
    );
    assert!(!provider.capabilities().process_host);
    assert_eq!(
        provider.auth_status().await,
        AuthState::Authenticated {
            account_label: None
        }
    );
    assert_eq!(
        provider.runtime_state(),
        RuntimeState::Ready {
            message: Some("Uses the Gemini generateContent API".into())
        }
    );
    assert!(provider.quota().is_none() && provider.accounts().is_none());
}

#[tokio::test]
async fn the_directory_is_built_in_and_reads_video() {
    let vendor = MockVendor::start().await;
    let list = provider_at(&vendor, "/v1beta").list_models().await.unwrap();
    let ids: Vec<&str> = list.models.iter().map(|model| model.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "gemini-3.6-flash",
            "gemini-3.5-flash",
            "gemini-3.1-pro-preview",
            "gemini-2.5-flash"
        ]
    );
    assert_eq!(list.default_model_id.as_deref(), Some("gemini-3.6-flash"));
    assert_eq!(
        (list.source_fetched_at, list.stale),
        (Timestamp::UNIX_EPOCH, false)
    );
    let first = &list.models[0];
    assert_eq!(
        (first.context_window, first.output_limit),
        (Some(1_048_576), Some(65_536))
    );
    assert_eq!(
        (
            first.supports_video,
            first.default_thinking_effort.as_deref()
        ),
        (Some(true), Some("medium"))
    );
    assert!(
        vendor.requests().is_empty(),
        "reading the directory makes no request"
    );
}
