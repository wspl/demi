//! The quota of a Grok Build account (`usage-and-quota.md` § Grok Build): a
//! free probe of the account's subscription and credits billing, and the
//! short-window rate limits on every chat response.

use std::sync::Arc;

use demi_core::{QuotaPlan, QuotaUnit, QuotaWindow, Timestamp};
use demi_provider::{
    quota::{
        Observation, ProbeCost, ProbeReading, QuotaError, QuotaSource, clamp_used_percent,
        header_number, rfc3339, severity, used_percent_from_ratio,
    },
    wire::{Reported, ReportedString},
};
use futures_util::future::BoxFuture;
use http::{HeaderMap, HeaderValue, header::ACCEPT};
use reqwest::Url;
use serde::{Deserialize, Deserializer};

use crate::{auth::GrokAuth, request::identity_headers};

/// The billing period type of a plan whose credits reset weekly.
const WEEKLY_PERIOD: &str = "USAGE_PERIOD_TYPE_WEEKLY";

/// The quota source of one Grok Build account.
pub(crate) struct GrokQuota {
    pub(crate) auth: Arc<GrokAuth>,
    pub(crate) http: reqwest::Client,
    pub(crate) user_url: Url,
    pub(crate) billing_url: Url,
}

impl QuotaSource for GrokQuota {
    fn probe_cost(&self) -> Option<ProbeCost> {
        Some(ProbeCost::Free)
    }

    /// `GET /v1/user?include=subscription` and `GET
    /// /v1/billing?format=credits`, sent together.
    fn probe(&self) -> BoxFuture<'_, Result<ProbeReading, QuotaError>> {
        Box::pin(async move {
            let credentials = self
                .auth
                .credentials(&self.http, None)
                .await
                .map_err(|failure| failure.quota_error())?;
            let mut headers = identity_headers(&credentials);
            headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
            let (user, billing) = tokio::try_join!(
                self.fetch(&self.user_url, &headers),
                self.fetch(&self.billing_url, &headers),
            )?;
            Ok(reading(&user, &billing, credentials.email))
        })
    }

    fn observe(&self, observation: Observation<'_>) -> Option<Vec<QuotaWindow>> {
        let Observation::Response { headers, .. } = observation else {
            return None;
        };
        rate_limit_windows(headers)
    }
}

impl GrokQuota {
    async fn fetch(&self, url: &Url, headers: &HeaderMap) -> Result<String, QuotaError> {
        let failed = |error: reqwest::Error| {
            QuotaError::Unavailable(format!(
                "Grok quota request failed: {}",
                error.without_url()
            ))
        };
        let response = self
            .http
            .get(url.clone())
            .headers(headers.clone())
            .send()
            .await
            .map_err(failed)?;
        let status = response.status();
        let body = response.text().await.map_err(failed)?;
        if !status.is_success() {
            let body: String = body.chars().take(200).collect();
            return Err(QuotaError::Unavailable(format!(
                "Grok quota request failed ({}): {body}",
                status.as_u16()
            )));
        }
        Ok(body)
    }
}

/// The account's subscription, as far as Demi shows it. Quota is a display,
/// so a field in a shape Demi does not know is left out and the rest still
/// shows.
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct User {
    subscription_tier: ReportedString,
    email: ReportedString,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Billing {
    config: Reported<BillingConfig>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct BillingConfig {
    credit_usage_percent: Reported<f64>,
    current_period: Reported<Period>,
    billing_period_end: ReportedString,
    monthly_limit: Reported<Amount>,
    used: Reported<Amount>,
    on_demand_cap: Reported<Amount>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Period {
    #[serde(rename = "type")]
    kind: ReportedString,
    end: ReportedString,
}

/// An amount of credits: a number, or `{ "val": number }`.
struct Amount(f64);

impl<'de> Deserialize<'de> for Amount {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let number = value.as_f64().or_else(|| value.get("val")?.as_f64());
        number
            .filter(|number| number.is_finite())
            .map(Self)
            .ok_or_else(|| serde::de::Error::custom("an amount is a number or { val: number }"))
    }
}

/// The reading of a probe: the plan from the subscription tier, the account
/// by its email, a credits window when the plan meters credits, weekly or
/// monthly, and an on-demand cap above zero.
fn reading(user: &str, billing: &str, email: Option<String>) -> ProbeReading {
    let user: User = serde_json::from_str(user).unwrap_or_default();
    let billing: Billing = serde_json::from_str(billing).unwrap_or_default();
    let config = billing.config.into_inner().unwrap_or_default();
    let period = config.current_period.into_inner().unwrap_or_default();
    let weekly = period.kind.as_deref() == Some(WEEKLY_PERIOD);
    let resets_at: Option<Timestamp> = period
        .end
        .into_inner()
        .or(config.billing_period_end.into_inner())
        .and_then(|text| rfc3339(&text));
    let limit = config.monthly_limit.into_inner().map(|amount| amount.0);
    let used = config.used.into_inner().map(|amount| amount.0);
    let used_percent = config
        .credit_usage_percent
        .into_inner()
        .and_then(clamp_used_percent)
        .or_else(|| used_percent_from_ratio(used?, limit?));
    let mut windows = Vec::new();
    // A plan that meters no credits names none of these, and a window with
    // nothing to show is not one.
    if used_percent.is_some() || used.is_some() || limit.is_some() {
        let (id, label) = if weekly {
            ("weekly", "Weekly credits")
        } else {
            ("monthly", "Monthly credits")
        };
        windows.push(QuotaWindow {
            id: id.into(),
            label: label.into(),
            used_percent,
            used,
            limit,
            unit: Some(QuotaUnit::Credits),
            resets_at,
            severity: severity(used_percent),
            scope: None,
        });
    }
    if let Some(cap) = config.on_demand_cap.into_inner().filter(|cap| cap.0 > 0.0) {
        windows.push(QuotaWindow {
            id: "on_demand_cap".into(),
            label: "On-demand cap".into(),
            used_percent: None,
            used: None,
            limit: Some(cap.0),
            unit: Some(QuotaUnit::Credits),
            resets_at,
            severity: None,
            scope: None,
        });
    }
    let plan = user
        .subscription_tier
        .into_inner()
        .filter(|tier| !tier.is_empty())
        .map(|tier| QuotaPlan {
            label: tier.clone(),
            id: tier,
        });
    ProbeReading {
        plan,
        account_label: email.or(user.email.into_inner()),
        windows,
    }
}

/// The short-window rate limits a chat response reports; not the plan's
/// credits. The amount used is the limit minus what remains.
fn rate_limit_windows(headers: &HeaderMap) -> Option<Vec<QuotaWindow>> {
    let read = |name: &str| header_number(headers, name);
    let requests = (
        read("x-ratelimit-limit-requests"),
        read("x-ratelimit-remaining-requests"),
    );
    let tokens = (
        read("x-ratelimit-limit-tokens"),
        read("x-ratelimit-remaining-tokens"),
    );
    let windows: Vec<QuotaWindow> = [
        (
            "rpm",
            "Requests (short window)",
            QuotaUnit::Requests,
            requests,
        ),
        ("tpm", "Tokens (short window)", QuotaUnit::Tokens, tokens),
    ]
    .into_iter()
    .filter_map(|(id, label, unit, (limit, remaining))| {
        let limit = limit?;
        let used = remaining.map(|remaining| limit - remaining);
        let used_percent = used.and_then(|used| used_percent_from_ratio(used, limit));
        Some(QuotaWindow {
            id: id.into(),
            label: label.into(),
            used_percent,
            used,
            limit: Some(limit),
            unit: Some(unit),
            resets_at: None,
            severity: severity(used_percent),
            scope: None,
        })
    })
    .collect();
    (!windows.is_empty()).then_some(windows)
}
