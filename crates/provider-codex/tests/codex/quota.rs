//! A Codex account's quota: the `x-codex-*` headers on every answer of the
//! service and the free usage probe (`usage-and-quota.md` § Codex).

use demi_core::{QuotaSeverity, SnapshotSource, Timestamp};
use demi_provider::{
    Provider,
    quota::{ProbeCost, QuotaError},
    testing::{MockResponse, MockVendor, inference_request},
};
use demi_provider_codex::TransportMode;
use serde_json::json;

use crate::{NOW, RESPONSES, completed, fresh_token, pool_with, provider, run, runtime_of, secret};

fn windows(
    provider: &demi_provider_codex::CodexProvider,
) -> Vec<(String, String, Option<f64>, Option<Timestamp>)> {
    let snapshot = provider.quota().unwrap().latest().unwrap();
    snapshot
        .windows
        .iter()
        .map(|window| {
            (
                window.id.clone(),
                window.label.clone(),
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
            ("primary".into(), "5-hour".into(), Some(35.5), Some(reset)),
            ("secondary".into(), "Weekly".into(), Some(10.0), None),
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

#[tokio::test]
async fn the_probe_reads_the_usage_status_for_free_with_the_plan_and_windows_named_by_their_length()
{
    let vendor = MockVendor::start().await;
    let usage = json!({
        "plan_type": "self_serve_business_usage_based",
        "rate_limit": {
            "allowed": false,
            "limit_reached": true,
            "primary_window": { "used_percent": 100, "limit_window_seconds": 18_000, "reset_after_seconds": 60, "reset_at": 1_700_000_000 },
            "secondary_window": { "used_percent": 41, "limit_window_seconds": 172_800, "reset_after_seconds": 900, "reset_at": 1_700_500_000 },
        },
    });
    vendor.respond_at(
        "/backend-api/wham/usage",
        MockResponse::status(200).chunk(usage.to_string()),
    );
    let pool = pool_with(secret(&fresh_token(), "refresh-1", NOW)).await;
    let provider = provider(&vendor, &pool, TransportMode::Sse);
    let quota = provider.quota().unwrap();
    assert_eq!(quota.probe_cost(), Some(ProbeCost::Free));
    let snapshot = quota.probe().await.unwrap();
    let request = &vendor.requests()[0];
    assert_eq!(
        (request.method.as_str(), request.uri.path()),
        ("GET", "/backend-api/wham/usage")
    );
    assert_eq!(request.header("chatgpt-account-id"), Some("acct-1"));
    assert_eq!(
        request.header("authorization"),
        Some(format!("Bearer {}", fresh_token()).as_str())
    );
    let plan = snapshot.plan.as_ref().unwrap();
    assert_eq!(
        (plan.id.as_str(), plan.label.as_str()),
        (
            "self_serve_business_usage_based",
            "Self serve business usage based"
        )
    );
    assert_eq!(snapshot.account_label.as_deref(), Some("dev@example.com"));
    assert_eq!(
        windows(&provider),
        [
            (
                "primary".into(),
                "5-hour".into(),
                Some(100.0),
                Some("2023-11-14T22:13:20.000Z".parse().unwrap())
            ),
            (
                "secondary".into(),
                "2-day".into(),
                Some(41.0),
                Some("2023-11-20T17:06:40.000Z".parse().unwrap())
            ),
        ]
    );
}

#[tokio::test]
async fn a_usage_status_the_backend_refuses_fails_the_probe() {
    let vendor = MockVendor::start().await;
    vendor.respond_at(
        "/backend-api/wham/usage",
        MockResponse::status(403).chunk("nope"),
    );
    let pool = pool_with(secret(&fresh_token(), "refresh-1", NOW)).await;
    let provider = provider(&vendor, &pool, TransportMode::Sse);
    let error = provider.quota().unwrap().probe().await.unwrap_err();
    assert_eq!(
        error,
        QuotaError::Unavailable("Codex usage request failed with HTTP 403".into())
    );
    assert!(provider.quota().unwrap().latest().is_none());
}
