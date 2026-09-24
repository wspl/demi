//! The quota of a Claude Code account (`usage-and-quota.md` § Claude Code):
//! a free probe of the OAuth usage endpoint, and the `rate_limits` the CLI's
//! stream-json lines report, which carry the same windows.

use std::sync::Arc;

use demi_core::{QuotaScope, QuotaSeverity, QuotaUnit, QuotaWindow, Timestamp};
use demi_provider::quota::{
    Observation, ProbeCost, ProbeReading, QuotaError, QuotaSource, clamp_used_percent, rfc3339,
    severity,
};
use demi_provider::wire::{NonEmpty, Reported};
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

    /// `GET /api/oauth/usage` with the account's token. A setup token does
    /// not say its plan, so the reading names none.
    fn probe(&self) -> BoxFuture<'_, Result<ProbeReading, QuotaError>> {
        Box::pin(async move {
            let secret = self
                .auth
                .stored()
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

    /// The `rate_limits` object on a line of the CLI's output, at the top
    /// level or inside `message`.
    fn observe(&self, observation: Observation<'_>) -> Option<Vec<QuotaWindow>> {
        let Observation::CliLine(line) = observation else {
            return None;
        };
        let limits = line
            .get("rate_limits")
            .or_else(|| line.get("message")?.get("rate_limits"))?;
        let windows = usage(limits)?.windows();
        (!windows.is_empty()).then_some(windows)
    }
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
    used_percentage: Option<f64>,
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
            ("five_hour", "5h session", self.five_hour),
            ("seven_day", "7d all models", self.seven_day),
            ("seven_day_sonnet", "7d Sonnet", self.seven_day_sonnet),
            ("seven_day_opus", "7d Opus", self.seven_day_opus),
        ];
        let mut windows: Vec<QuotaWindow> = named
            .into_iter()
            .filter_map(|(id, label, window)| {
                let window = window.into_inner()?;
                let used_percent = window
                    .utilization
                    .or(window.used_percentage)
                    .and_then(clamp_used_percent);
                Some(QuotaWindow {
                    id: id.into(),
                    label: label.into(),
                    used_percent,
                    used: None,
                    limit: None,
                    unit: Some(QuotaUnit::Percent),
                    resets_at: reset_time(window.resets_at),
                    severity: severity(used_percent),
                    scope: None,
                })
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
