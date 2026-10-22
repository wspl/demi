//! A Codex account's quota: the `x-codex-*` headers on every answer of the
//! service and the free usage probe (`crates/provider-codex/src/quota.rs`).

use demi_provider_codex::TransportMode;
use demi_provider_common::{
    Provider,
    testing::{MockResponse, MockVendor, inference_request},
};
use demi_shared_types::{QuotaKind, QuotaSeverity, SnapshotSource, Timestamp};

use crate::{NOW, RESPONSES, completed, fresh_token, pool_with, provider, run, runtime_of, secret};

fn windows(
    provider: &demi_provider_codex::CodexProvider,
) -> Vec<(String, QuotaKind, Option<u64>, Option<f64>, Option<Timestamp>)> {
    let snapshot = provider.quota().unwrap().latest().unwrap();
    snapshot
        .windows
        .iter()
        .map(|window| {
            (
                window.id.clone(),
                window.kind,
                window.period_seconds,
                window.used_percent,
                window.resets_at,
            )
        })
        .collect()
}

#[tokio::test]
async fn every_answer_of_the_service_a_refusal_included_updates_the_accounts_windows() {
    let vendor = MockVendor::start().await;
    let quota_headers = |response: MockResponse, used: &str| {
        response
            .header("x-codex-primary-used-percent", used)
            .header("x-codex-primary-window-minutes", "300")
            .header("x-codex-primary-reset-at", "1700000000")
            .header("x-codex-secondary-used-percent", "10")
            .header("x-codex-secondary-window-minutes", "10080")
    };
    vendor.respond_at(RESPONSES, quota_headers(completed(), "35.5"));
    vendor.respond_at(
        RESPONSES,
        quota_headers(MockResponse::status(429).chunk("{}"), "100"),
    );
    let pool = pool_with(secret(&fresh_token(), "refresh-1", NOW)).await;
    let provider = provider(&vendor, &pool, TransportMode::Sse);
    let mut runtime = runtime_of(&provider);
    run(runtime.as_mut(), inference_request()).await;
    let reset: Timestamp = "2023-11-14T22:13:20.000Z".parse().unwrap();
    assert_eq!(
        windows(&provider),
        [
            ("primary".into(), QuotaKind::Limit, Some(5 * 3600), Some(35.5), Some(reset)),
            ("secondary".into(), QuotaKind::Limit, Some(7 * 86_400), Some(10.0), None),
        ]
    );
    run(runtime.as_mut(), inference_request()).await;
    let snapshot = provider.quota().unwrap().latest().unwrap();
    assert_eq!(snapshot.source, SnapshotSource::Observation);
    assert_eq!(
        (
            snapshot.windows[0].used_percent,
            snapshot.windows[0].severity
        ),
        (Some(100.0), Some(QuotaSeverity::Critical))
    );
}

/// The headers Codex sends for a Pro account with a weekly window only, as
/// captured: a placeholder for the secondary window it does not have.
fn weekly_only(response: MockResponse) -> MockResponse {
    response
        .header("x-codex-primary-used-percent", "82")
        .header("x-codex-primary-window-minutes", "10080")
        .header("x-codex-primary-reset-at", "1792874449")
        .header("x-codex-secondary-used-percent", "0")
        .header("x-codex-secondary-window-minutes", "0")
        .header("x-codex-secondary-reset-at", "")
}

fn ids(provider: &demi_provider_codex::CodexProvider) -> Vec<(String, Option<u64>)> {
    windows(provider)
        .into_iter()
        .map(|(id, _, period, _, _)| (id, period))
        .collect()
}

#[tokio::test]
async fn the_placeholder_codex_sends_for_a_window_the_account_lacks_is_no_window_and_removes_one_kept()
{
    let vendor = MockVendor::start().await;
    vendor.respond_at(RESPONSES, weekly_only(completed()));
    // A real window may be unused yet: its length makes it one.
    vendor.respond_at(
        RESPONSES,
        completed()
            .header("x-codex-secondary-used-percent", "0")
            .header("x-codex-secondary-window-minutes", "300")
            .header("x-codex-secondary-reset-at", ""),
    );
    vendor.respond_at(RESPONSES, weekly_only(completed()));
    let pool = pool_with(secret(&fresh_token(), "refresh-1", NOW)).await;
    let provider = provider(&vendor, &pool, TransportMode::Sse);
    let mut runtime = runtime_of(&provider);
    let week = Some(7 * 86_400);

    run(runtime.as_mut(), inference_request()).await;
    assert_eq!(ids(&provider), [("primary".into(), week)]);

    run(runtime.as_mut(), inference_request()).await;
    assert_eq!(
        ids(&provider),
        [("primary".into(), week), ("secondary".into(), Some(5 * 3600))]
    );

    run(runtime.as_mut(), inference_request()).await;
    assert_eq!(ids(&provider), [("primary".into(), week)]);
}
