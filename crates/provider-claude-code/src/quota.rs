//! The quota of a Claude Code account (`usage-and-quota.md` § Claude Code):
//! a free probe of the OAuth usage endpoint, and the `rate_limit_event`
//! lines the CLI prints when the vendor's rate-limit headers change.

use std::collections::BTreeMap;
use std::sync::Arc;

use demi_provider_common::quota::{
    Observation, ProbeCost, ProbeReading, QuotaError, QuotaSource, clamp_used_percent, rfc3339,
    severity,
};
use demi_provider_common::wire::{NonEmpty, Reported};
use demi_shared_types::{QuotaScope, QuotaSeverity, QuotaUnit, QuotaWindow, Timestamp};
use futures_util::future::BoxFuture;
use http::HeaderValue;
use http::header::{ACCEPT, AUTHORIZATION};
use reqwest::Url;
use serde::Deserialize;

use crate::account::ClaudeAuth;

/// The beta the OAuth usage endpoint answers under.
const OAUTH_BETA: &str = "oauth-2025-04-20";

/// The quota source of one Claude Code account.
pub(crate) struct ClaudeQuota {
    pub(crate) auth: Arc<ClaudeAuth>,
    pub(crate) http: reqwest::Client,
    pub(crate) usage_url: Url,
}

impl QuotaSource for ClaudeQuota {
    fn probe_cost(&self) -> Option<ProbeCost> {
        Some(ProbeCost::Free)
    }

    /// `GET /api/oauth/usage` with the account's access token, refreshed
    /// first when it expires within five minutes. The endpoint does not say
    /// the plan, so the reading names none.
    fn probe(&self) -> BoxFuture<'_, Result<ProbeReading, QuotaError>> {
        Box::pin(async move {
            let secret = self
                .auth
                .credentials(&self.http, None)
                .await
                .map_err(|failure| failure.quota_error())?;
            let failed = |error: reqwest::Error| {
                QuotaError::Unavailable(format!(
                    "Claude usage request failed: {}",
                    error.without_url()
                ))
            };
            let response = self
                .http
                .get(self.usage_url.clone())
                .header(AUTHORIZATION, secret.access_token.bearer())
                .header("anthropic-beta", HeaderValue::from_static(OAUTH_BETA))
                .header(ACCEPT, HeaderValue::from_static("application/json"))
                .send()
                .await
                .map_err(failed)?;
            let status = response.status();
            let body = response.text().await.map_err(failed)?;
            if !status.is_success() {
                let body: String = body.chars().take(200).collect();
                return Err(QuotaError::Unavailable(format!(
                    "Claude usage request failed ({}): {body}",
                    status.as_u16()
                )));
            }
            let invalid = |reason: String| {
                QuotaError::Invalid(format!("Claude usage answer cannot be read: {reason}"))
            };
            let answer: serde_json::Value =
                serde_json::from_str(&body).map_err(|error| invalid(error.to_string()))?;
            let usage = usage(&answer).ok_or_else(|| invalid("it is not an object".into()))?;
            Ok(ProbeReading {
                plan: None,
                account_label: None,
                windows: usage.windows(),
            })
        })
    }

    /// The windows a `rate_limit_event` line of the CLI's output reports.
    fn observe(&self, observation: Observation<'_>) -> Option<Vec<QuotaWindow>> {
        let Observation::CliLine(line) = observation else {
            return None;
        };
        if line.get("type")?.as_str()? != "rate_limit_event" {
            return None;
        }
        // serde's derive would read an array's items as the fields in order.
        let info = line
            .get("rate_limit_info")
            .filter(|info| info.is_object())?;
        let info = RateLimitInfo::deserialize(info).ok()?;
        let windows = info.windows();
        (!windows.is_empty()).then_some(windows)
    }
}

/// The windows Demi names, by the vendor's id.
const NAMED_WINDOWS: [(&str, &str); 4] = [
    ("five_hour", "5h session"),
    ("seven_day", "7d all models"),
    ("seven_day_sonnet", "7d Sonnet"),
    ("seven_day_opus", "7d Opus"),
];

/// A named window with the share used and its reset time.
fn named_window(
    id: &str,
    used_percent: Option<f64>,
    resets_at: Option<Timestamp>,
) -> Option<QuotaWindow> {
    let (id, label) = NAMED_WINDOWS.into_iter().find(|(named, _)| *named == id)?;
    Some(QuotaWindow {
        id: id.into(),
        label: label.into(),
        used_percent,
        used: None,
        limit: None,
        unit: Some(QuotaUnit::Percent),
        resets_at,
        severity: severity(used_percent),
        scope: None,
    })
}

/// The `rate_limit_info` of a `rate_limit_event` line: the window the
/// vendor's headers name as binding, and every window they report. The CLI
/// passes the headers through: a utilization is a fraction of one, and a
/// reset time is Unix seconds (Claude Code 2.1.286).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RateLimitInfo {
    #[serde(default)]
    rate_limit_type: Reported<String>,
    #[serde(default)]
    utilization: Reported<f64>,
    #[serde(default)]
    resets_at: Reported<i64>,
    #[serde(default)]
    unified_windows: Reported<BTreeMap<String, Reported<UnifiedWindow>>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UnifiedWindow {
    #[serde(default)]
    utilization: Reported<f64>,
    #[serde(default)]
    resets_at: Reported<i64>,
}

impl RateLimitInfo {
    /// Each named window of `unifiedWindows`, then the binding window when
    /// those do not hold it. A window Demi does not name is left out.
    fn windows(self) -> Vec<QuotaWindow> {
        let mut windows: Vec<QuotaWindow> = self
            .unified_windows
            .into_inner()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(id, window)| {
                let window = window.into_inner()?;
                named_window(
                    &id,
                    fraction_percent(window.utilization),
                    unix_seconds(window.resets_at),
                )
            })
            .collect();
        if let Some(id) = self.rate_limit_type.into_inner()
            && !windows.iter().any(|window| window.id == id)
            && let Some(window) = named_window(
                &id,
                fraction_percent(self.utilization),
                unix_seconds(self.resets_at),
            )
        {
            windows.push(window);
        }
        windows
    }
}

/// A utilization the headers state as a fraction of one, as a percentage.
fn fraction_percent(fraction: Reported<f64>) -> Option<f64> {
    clamp_used_percent(fraction.into_inner()? * 100.0)
}

/// A reset time the headers state in Unix seconds.
fn unix_seconds(seconds: Reported<i64>) -> Option<Timestamp> {
    Timestamp::from_millisecond(seconds.into_inner()?.checked_mul(1000)?).ok()
}

/// The usage an account reports. Quota is a display, so a window or a limit
/// Demi cannot read is left out and the rest still shows.
#[derive(Default, Deserialize)]
#[serde(default)]
struct Usage {
    five_hour: Reported<Window>,
    seven_day: Reported<Window>,
    seven_day_sonnet: Reported<Window>,
    seven_day_opus: Reported<Window>,
    limits: Reported<Vec<Reported<Limit>>>,
}

/// The usage `value` reports, when it is an object: serde's derive would
/// read an array's items as the fields in order.
fn usage(value: &serde_json::Value) -> Option<Usage> {
    if !value.is_object() {
        return None;
    }
    Usage::deserialize(value).ok()
}

/// One of the named windows.
#[derive(Deserialize)]
struct Window {
    #[serde(default)]
    utilization: Option<f64>,
    #[serde(default)]
    resets_at: Reported<String>,
}

/// One entry of `limits`.
#[derive(Deserialize)]
struct Limit {
    kind: NonEmpty,
    #[serde(default)]
    percent: Option<f64>,
    #[serde(default)]
    severity: Reported<QuotaSeverity>,
    #[serde(default)]
    resets_at: Reported<String>,
    #[serde(default)]
    scope: Reported<LimitScope>,
}

#[derive(Deserialize)]
struct LimitScope {
    model: Option<ScopedModel>,
}

#[derive(Deserialize)]
struct ScopedModel {
    display_name: Option<String>,
}

impl Usage {
    /// The named windows, then every other limit. The `session` and
    /// `weekly_all` limits are the five-hour and seven-day windows, so they
    /// are skipped.
    fn windows(self) -> Vec<QuotaWindow> {
        let named = [
            ("five_hour", self.five_hour),
            ("seven_day", self.seven_day),
            ("seven_day_sonnet", self.seven_day_sonnet),
            ("seven_day_opus", self.seven_day_opus),
        ];
        let mut windows: Vec<QuotaWindow> = named
            .into_iter()
            .filter_map(|(id, window)| {
                let window = window.into_inner()?;
                let used_percent = window.utilization.and_then(clamp_used_percent);
                named_window(id, used_percent, reset_time(window.resets_at))
            })
            .collect();
        let limits = self.limits.into_inner().unwrap_or_default();
        for limit in limits.into_iter().filter_map(Reported::into_inner) {
            let kind = limit.kind.0;
            if kind == "session" || kind == "weekly_all" {
                continue;
            }
            let model = limit
                .scope
                .into_inner()
                .and_then(|scope| scope.model)
                .and_then(|model| model.display_name);
            let used_percent = limit.percent.and_then(clamp_used_percent);
            let (id, label, scope) = match model {
                Some(model) => (
                    format!("limit:{kind}:{model}"),
                    format!("{kind} ({model})"),
                    QuotaScope {
                        kind: "model".into(),
                        label: Some(model),
                    },
                ),
                None => (
                    format!("limit:{kind}"),
                    kind.clone(),
                    QuotaScope { kind, label: None },
                ),
            };
            windows.push(QuotaWindow {
                id,
                label,
                used_percent,
                used: None,
                limit: None,
                unit: Some(QuotaUnit::Percent),
                resets_at: reset_time(limit.resets_at),
                severity: limit
                    .severity
                    .into_inner()
                    .or_else(|| severity(used_percent)),
                scope: Some(scope),
            });
        }
        windows
    }
}

/// A reset time the usage answers state as an RFC 3339 time; any other value
/// is an unknown reset time.
fn reset_time(text: Reported<String>) -> Option<Timestamp> {
    rfc3339(&text.into_inner()?)
}
