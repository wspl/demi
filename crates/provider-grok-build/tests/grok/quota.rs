//! A Grok Build account's quota: the billing and subscription probe and the
//! rate-limit headers of every chat response (`crates/provider-grok-build/src/quota.rs`).

use std::sync::Arc;

use demi_provider_common::{
    Provider,
    testing::{MockResponse, MockVendor, inference_request},
};
use demi_shared_types::{QuotaKind, QuotaSeverity, QuotaSnapshot, QuotaUnit};
use serde_json::{Value, json};

use crate::{ACCOUNT, CHAT, chat, json_answer, pool_with, provider, run, runtime_of, secret};

/// The snapshot of a probe whose user and billing endpoints answer `user`
/// and `billing`, for an account whose sign-in names no email.
async fn probe(user: Value, billing: Value) -> (Arc<QuotaSnapshot>, MockVendor) {
    let vendor = MockVendor::start().await;
    vendor.respond_at("/v1/user", json_answer(200, user));
    vendor.respond_at("/v1/billing", json_answer(200, billing));
    let pool = pool_with(secret(&vendor, json!({ "email": null }))).await;
    let provider = provider(&vendor, &pool, Some(ACCOUNT));
    let snapshot = provider.quota().unwrap().probe().await.unwrap();
    (snapshot, vendor)
}

#[tokio::test]
async fn weekly_credits_take_their_share_and_period_end_and_a_cap_adds_its_window() {
    let (reading, _vendor) = probe(
        json!({ "subscriptionTier": "XPremiumPlus" }),
        json!({ "config": {
            "creditUsagePercent": 2,
            "currentPeriod": { "type": "USAGE_PERIOD_TYPE_WEEKLY", "start": "2026-08-14T10:45:24.951512+00:00", "end": "2026-08-21T10:45:24.951512+00:00" },
            "onDemandCap": 50,
            "billingPeriodEnd": "2026-08-30T00:00:00+00:00",
        } }),
    )
    .await;
    // Each window is of the kind and the period the billing states, from
    // the period's start to its end (`crates/provider-grok-build/src/quota.rs`).
    let ids: Vec<(&str, QuotaKind, Option<u64>)> = reading
        .windows
        .iter()
        .map(|window| (window.id.as_str(), window.kind, window.period_seconds))
        .collect();
    let week = Some(7 * 24 * 3600);
    assert_eq!(
        ids,
        [
            ("weekly", QuotaKind::Credits, week),
            ("on_demand_cap", QuotaKind::Cap, week)
        ]
    );
    let weekly = &reading.windows[0];
    assert_eq!(
        (weekly.used_percent, weekly.used, weekly.limit),
        (Some(2.0), None, None)
    );
    assert_eq!(
        weekly.resets_at,
        Some("2026-08-21T10:45:24.951Z".parse().unwrap())
    );
    assert_eq!(
        (reading.windows[1].limit, reading.windows[1].used_percent),
        (Some(50.0), None)
    );
}

#[tokio::test]
async fn every_chat_answer_a_refusal_included_reports_its_short_window_limits() {
    let vendor = MockVendor::start().await;
    let limits = |response: MockResponse, remaining: &str| {
        response
            .header("x-ratelimit-limit-requests", "120")
            .header("x-ratelimit-remaining-requests", remaining)
            .header("x-ratelimit-limit-tokens", "5000")
            .header("x-ratelimit-remaining-tokens", "4000")
    };
    vendor.respond_at(CHAT, limits(chat(&[]), "100"));
    vendor.respond_at(CHAT, limits(MockResponse::status(429), "0"));
    let pool = pool_with(secret(&vendor, json!({}))).await;
    let provider = provider(&vendor, &pool, Some(ACCOUNT));
    let mut runtime = runtime_of(&provider);
    run(runtime.as_mut(), inference_request()).await;
    let snapshot = provider.quota().unwrap().latest().unwrap();
    let windows: Vec<_> = snapshot
        .windows
        .iter()
        .map(|window| (window.id.as_str(), window.used, window.limit, window.unit))
        .collect();
    assert_eq!(
        windows,
        [
            ("rpm", Some(20.0), Some(120.0), Some(QuotaUnit::Requests)),
            ("tpm", Some(1_000.0), Some(5_000.0), Some(QuotaUnit::Tokens))
        ]
    );
    run(runtime.as_mut(), inference_request()).await;
    let rpm = &provider.quota().unwrap().latest().unwrap().windows[0];
    assert_eq!(
        (rpm.used_percent, rpm.severity),
        (Some(100.0), Some(QuotaSeverity::Critical))
    );
}
