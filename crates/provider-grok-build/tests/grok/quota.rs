//! A Grok Build account's quota: the billing and subscription probe and the
//! rate-limit headers of every chat response (`usage-and-quota.md` § Grok
//! Build).

use std::sync::Arc;

use demi_core::{QuotaSeverity, QuotaSnapshot, QuotaUnit, QuotaWindow};
use demi_provider::{
    Provider,
    testing::{MockResponse, MockVendor, inference_request},
};
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
async fn the_probe_asks_the_subscription_and_the_credits_billing_together() {
    let (reading, vendor) = probe(
        json!({ "subscriptionTier": "XPremiumPlus", "email": "a@b.com", "hasGrokCodeAccess": true }),
        json!({ "config": { "monthlyLimit": { "val": 20_000 }, "used": { "val": 5_000 }, "onDemandCap": { "val": 0 }, "billingPeriodEnd": "2026-08-01T00:00:00+00:00" } }),
    )
    .await;
    let mut urls: Vec<String> = vendor
        .requests()
        .iter()
        .map(|request| request.uri.to_string())
        .collect();
    urls.sort();
    assert_eq!(
        urls,
        [
            "/v1/billing?format=credits",
            "/v1/user?include=subscription"
        ]
    );
    let plan = reading.plan.clone().unwrap();
    assert_eq!(
        (plan.id.as_str(), plan.label.as_str()),
        ("XPremiumPlus", "XPremiumPlus")
    );
    assert_eq!(reading.account_label.as_deref(), Some("a@b.com"));
    assert_eq!(
        reading.windows,
        [QuotaWindow {
            id: "monthly".into(),
            label: "Monthly credits".into(),
            used_percent: Some(25.0),
            used: Some(5_000.0),
            limit: Some(20_000.0),
            unit: Some(QuotaUnit::Credits),
            resets_at: Some("2026-08-01T00:00:00.000Z".parse().unwrap()),
            severity: Some(QuotaSeverity::Normal),
            scope: None,
        }]
    );
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
    let ids: Vec<(&str, &str)> = reading
        .windows
        .iter()
        .map(|window| (window.id.as_str(), window.label.as_str()))
        .collect();
    assert_eq!(
        ids,
        [
            ("weekly", "Weekly credits"),
            ("on_demand_cap", "On-demand cap")
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
async fn a_plan_whose_billing_meters_no_credits_has_no_window_and_odd_fields_are_left_out() {
    let (reading, _vendor) = probe(
        json!({ "subscriptionTier": "XPremium", "email": "u@example.com" }),
        json!({ "config": {
            "currentPeriod": { "type": "USAGE_PERIOD_TYPE_WEEKLY", "start": "2026-09-18T00:00:00Z", "end": "2026-09-25T00:00:00Z" },
            "onDemandUsed": { "val": 0 },
            "prepaidBalance": { "val": 0 },
            "monthlyLimit": "lots",
            "billingPeriodEnd": "2026-10-01T00:00:00Z",
        } }),
    )
    .await;
    assert_eq!(reading.plan.as_ref().unwrap().label, "XPremium");
    assert!(reading.windows.is_empty(), "{:?}", reading.windows);
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
