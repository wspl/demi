//! The quota of a Codex account (`usage-and-quota.md` § Codex): a free probe
//! of the account's usage status, which the Codex CLI reads for its own
//! status and which answers while the limit is reached, and the
//! `x-codex-*` headers on every response of the service.

use std::sync::Arc;

use demi_core::{QuotaPlan, QuotaUnit, QuotaWindow};
use demi_provider::quota::{
    Observation, ProbeCost, ProbeReading, QuotaError, QuotaSource, clamp_used_percent,
    header_number, severity, unix_seconds,
};
use futures_util::future::BoxFuture;
use http::{HeaderMap, HeaderValue, header::ACCEPT};
use reqwest::Url;
use serde::Deserialize;

use crate::{auth::CodexAuth, request::account_headers};

/// The two windows Codex reports.
const WINDOWS: [Kind; 2] = [Kind::Primary, Kind::Secondary];

#[derive(Clone, Copy)]
enum Kind {
    Primary,
    Secondary,
}

impl Kind {
    fn id(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Secondary => "secondary",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Primary => "Primary",
            Self::Secondary => "Secondary",
        }
    }
}

/// The quota source of one Codex account.
pub(crate) struct CodexQuota {
    pub(crate) auth: Arc<CodexAuth>,
    pub(crate) http: reqwest::Client,
    pub(crate) usage_url: Url,
    pub(crate) user_agent: HeaderValue,
}

impl QuotaSource for CodexQuota {
    fn probe_cost(&self) -> Option<ProbeCost> {
        Some(ProbeCost::Free)
    }

    fn probe(&self) -> BoxFuture<'_, Result<ProbeReading, QuotaError>> {
        Box::pin(async move {
            let credentials = self
                .auth
                .credentials(&self.http, None)
                .await
                .map_err(|failure| failure.quota_error())?;
            let stored = self
                .auth
                .stored()
                .await
                .map_err(|failure| failure.quota_error())?;
            let mut headers = account_headers(&credentials, &self.user_agent);
            headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
            let response = self
                .http
                .get(self.usage_url.clone())
                .headers(headers)
                .send()
                .await
                .map_err(|error| {
                    QuotaError::Unavailable(format!(
                        "Codex usage request failed: {}",
                        error.without_url()
                    ))
                })?;
            if !response.status().is_success() {
                let status = response.status().as_u16();
                return Err(QuotaError::Unavailable(format!(
                    "Codex usage request failed with HTTP {status}"
                )));
            }
            let text = response.text().await.map_err(|error| {
                QuotaError::Unavailable(format!(
                    "Codex usage request failed: {}",
                    error.without_url()
                ))
            })?;
            let usage: UsageStatus =
                demi_provider::wire::decode_untagged(&text).map_err(|error| {
                    QuotaError::Invalid(format!("Codex usage status cannot be read: {error}"))
                })?;
            Ok(ProbeReading {
                plan: usage
                    .plan_type
                    .filter(|plan| !plan.is_empty())
                    .map(|plan| QuotaPlan {
                        label: plan_label(&plan),
                        id: plan,
                    }),
                account_label: Some(stored.label().label),
                windows: usage_windows(usage.rate_limit),
            })
        })
    }

    fn observe(&self, observation: Observation<'_>) -> Option<Vec<QuotaWindow>> {
        let Observation::Response { headers, .. } = observation else {
            return None;
        };
        let windows: Vec<QuotaWindow> = WINDOWS
            .into_iter()
            .filter_map(|kind| header_window(headers, kind))
            .collect();
        (!windows.is_empty()).then_some(windows)
    }
}

/// `GET /wham/usage`, as far as Demi reads it.
#[derive(Deserialize)]
struct UsageStatus {
    #[serde(default)]
    plan_type: Option<String>,
    #[serde(default)]
    rate_limit: Option<RateLimit>,
}

#[derive(Deserialize)]
struct RateLimit {
    #[serde(default)]
    primary_window: Option<UsageWindow>,
    #[serde(default)]
    secondary_window: Option<UsageWindow>,
}

/// One window of the usage status: `limit_window_seconds` in seconds and
/// `reset_at` in Unix seconds.
#[derive(Deserialize)]
struct UsageWindow {
    used_percent: f64,
    limit_window_seconds: f64,
    reset_at: f64,
}

fn usage_windows(rate_limit: Option<RateLimit>) -> Vec<QuotaWindow> {
    let Some(rate_limit) = rate_limit else {
        return Vec::new();
    };
    let windows = [
        (Kind::Primary, rate_limit.primary_window),
        (Kind::Secondary, rate_limit.secondary_window),
    ];
    windows
        .into_iter()
        .filter_map(|(kind, window)| {
            let window = window?;
            Some(window_of(
                kind,
                clamp_used_percent(window.used_percent),
                Some(window.limit_window_seconds / 60.0),
                unix_seconds(window.reset_at),
            ))
        })
        .collect()
}

/// The window the `x-codex-<kind>-*` headers report: one is present when its
/// `used-percent` header is, even with a value Demi cannot read.
fn header_window(headers: &HeaderMap, kind: Kind) -> Option<QuotaWindow> {
    let prefix = format!("x-codex-{}", kind.id());
    let used_name = format!("{prefix}-used-percent");
    headers.get(&used_name)?;
    let used_percent = header_number(headers, &used_name).and_then(clamp_used_percent);
    let minutes = header_number(headers, &format!("{prefix}-window-minutes"));
    let resets_at = header_number(headers, &format!("{prefix}-reset-at")).and_then(unix_seconds);
    Some(window_of(kind, used_percent, minutes, resets_at))
}

fn window_of(
    kind: Kind,
    used_percent: Option<f64>,
    minutes: Option<f64>,
    resets_at: Option<demi_core::Timestamp>,
) -> QuotaWindow {
    QuotaWindow {
        id: kind.id().into(),
        label: window_label(kind, minutes),
        used_percent,
        used: None,
        limit: None,
        unit: Some(QuotaUnit::Percent),
        resets_at,
        severity: severity(used_percent),
        scope: None,
    }
}

/// A window is named by its length, which is what the user knows it by, the
/// same whichever path saw it: `Weekly` and `Daily`, else the length in the
/// largest unit that divides it, else the vendor's word for it.
fn window_label(kind: Kind, minutes: Option<f64>) -> String {
    const DAY: f64 = 24.0 * 60.0;
    const WEEK: f64 = 7.0 * DAY;
    let Some(minutes) = minutes.filter(|minutes| *minutes > 0.0) else {
        return kind.name().into();
    };
    if minutes % WEEK == 0.0 {
        return if minutes == WEEK {
            "Weekly".into()
        } else {
            format!("{}-week", minutes / WEEK)
        };
    }
    if minutes % DAY == 0.0 {
        return if minutes == DAY {
            "Daily".into()
        } else {
            format!("{}-day", minutes / DAY)
        };
    }
    if minutes % 60.0 == 0.0 {
        return format!("{}-hour", minutes / 60.0);
    }
    format!("{minutes}-minute")
}

/// `plus` reads `Plus`; `self_serve_business_usage_based` reads `Self serve
/// business usage based`.
fn plan_label(plan: &str) -> String {
    let words = plan.replace('_', " ");
    let mut characters = words.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => words,
    }
}
