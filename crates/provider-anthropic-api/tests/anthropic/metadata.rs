//! What the provider says about itself: its entry, its needs, its state, its
//! model directory and how it reads its failure records.

use demi_core::{AuthState, FailureSource, ProviderErrorDiagnostics, RuntimeState, Timestamp};
use demi_provider::{
    HttpFailureRecord, Provider, RuntimeEnv,
    testing::{MockVendor, inference_request},
};

use crate::{provider_at, run, stop};

#[tokio::test]
async fn the_provider_names_its_entry_needs_no_process_host_and_is_ready_with_its_key() {
    let vendor = MockVendor::start().await;
    let provider = provider_at(&vendor, "/v1");
    assert_eq!((provider.id(), provider.display_name()), ("anthropic-work", "Work"));
    assert!(!provider.capabilities().process_host);
    assert_eq!(provider.auth_status().await, AuthState::Authenticated { account_label: None });
    assert!(matches!(provider.runtime_state(), RuntimeState::Ready { .. }));
    assert!(provider.quota().is_none() && provider.accounts().is_none());
}

#[tokio::test]
async fn the_directory_is_built_in_with_the_newest_opus_first() {
    let vendor = MockVendor::start().await;
    let list = provider_at(&vendor, "/v1").list_models().await.unwrap();
    let ids: Vec<&str> = list.models.iter().map(|model| model.id.as_str()).collect();
    assert_eq!(ids, ["claude-opus-4-8", "claude-opus-4-7", "claude-opus-4-6", "claude-sonnet-4-6", "claude-fable-5"]);
    assert_eq!(list.default_model_id.as_deref(), Some("claude-opus-4-8"));
    assert_eq!((list.source_fetched_at, list.stale), (Timestamp::UNIX_EPOCH, false));
    let opus = &list.models[0];
    assert_eq!(opus.display_name, "Claude Opus 4.8");
    assert_eq!((opus.context_window, opus.output_limit), (Some(1_000_000), Some(128_000)));
    assert_eq!(opus.supported_thinking_efforts.as_deref(), Some(&["low", "medium", "high", "xhigh", "max"].map(String::from)[..]));
    assert_eq!((opus.can_disable_thinking, opus.supports_video), (Some(false), Some(false)));
    assert!(vendor.requests().is_empty(), "reading the directory makes no request");
}

#[tokio::test]
async fn a_failure_record_is_read_by_the_standard_reading() {
    let vendor = MockVendor::start().await;
    let record = HttpFailureRecord {
        status: 429,
        headers: vec![("retry-after".into(), "90".into())],
        body: String::new(),
    };
    let diagnostics = ProviderErrorDiagnostics {
        source: FailureSource::Http,
        client_request_id: None,
        provider_request_id: None,
        provider_response_id: None,
        provider_code: None,
        http_status: Some(429),
        upstream: Some(record.to_json()),
    };
    let facts = provider_at(&vendor, "/v1").read_failure(&diagnostics, "2026-09-18T14:00:00.000Z".parse().unwrap());
    assert_eq!(facts.retry_at, Some("2026-09-18T14:01:30.000Z".parse().unwrap()));
}

#[tokio::test]
async fn a_fresh_runtime_sends_with_the_same_configuration() {
    let vendor = MockVendor::start().await;
    vendor.respond(stop());
    let runtime = provider_at(&vendor, "/v1")
        .runtime(RuntimeEnv { http: reqwest::Client::new() })
        .unwrap();
    let mut copy = runtime.fresh();
    run(copy.as_mut(), inference_request()).await;
    let sent = &vendor.requests()[0];
    assert_eq!((sent.uri.path(), sent.header("x-api-key")), ("/v1/messages", Some("sk-ant-test")));
}
