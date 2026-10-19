//! The quota of a Codex account (`usage-and-quota.md` § Codex): a free probe
//! of the account's usage status, which the Codex CLI reads for its own
//! status and which answers while the limit is reached, and the
//! `x-codex-*` headers on every response of the service.

use std::sync::Arc;

use demi_provider_common::quota::{
    Observation, ObservedWindows, ProbeCost, ProbeReading, QuotaError, QuotaSource,
    clamp_used_percent, header_number, plan_label, severity, unix_seconds,
};
use demi_shared_types::{QuotaKind, QuotaPlan, QuotaUnit, QuotaWindow};
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
}

/// The quota source of one Codex account.
pub(crate) struct CodexQuota {
    pub(crate) auth: Arc<CodexAuth>,
    pub(crate) http: demi_provider_common::ProviderHttp,
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
                demi_provider_common::wire::decode_untagged(&text).map_err(|error| {
                    QuotaError::Invalid(format!("Codex usage status cannot be read: {error}"))
                })?;
            let ObservedWindows { windows, absent } = usage_windows(usage.rate_limit);
            Ok(ProbeReading {
                plan: usage
                    .plan_type
                    .filter(|plan| !plan.is_empty())
                    .map(|plan| QuotaPlan {
                        label: plan_label(&plan),
                        id: plan,
                    }),
                account_label: Some(stored.label().label),
                windows,
                absent,
            })
        })
    }

    fn observe(&self, observation: Observation<'_>) -> Option<ObservedWindows> {
        let Observation::Response { headers, .. } = observation else {
            return None;
        };
        let mut report = ObservedWindows::default();
        for kind in WINDOWS {
            match header_window(headers, kind) {
                None => {}
                Some(Reported::Window(window)) => report.windows.push(window),
                Some(Reported::Absent) => report.absent.push(kind.id().into()),
            }
        }
        (!report.windows.is_empty() || !report.absent.is_empty()).then_some(report)
    }
}

/// What Codex says about one of its windows.
enum Reported {
    Window(QuotaWindow),
    /// The account has no such window.
    Absent,
}

/// `GET /wham/usage`, as far as Demi reads it.
#[derive(Deserialize)]
struct UsageStatus {
    #[serde(default)]
    plan_type: Option<String>,
    #[serde(default)]
    rate_limit: Option<RateLimit>,
}

/// A window's key: missing (`None`), `null` (`Some(None)`), which says the
/// account has no such window, or a window.
#[derive(Deserialize)]
struct RateLimit {
    #[serde(default, with = "serde_with::rust::double_option")]
    primary_window: Option<Option<UsageWindow>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    secondary_window: Option<Option<UsageWindow>>,
}

/// One window of the usage status: `limit_window_seconds` in seconds and
/// `reset_at` in Unix seconds.
#[derive(Deserialize)]
struct UsageWindow {
    used_percent: f64,
    limit_window_seconds: f64,
    reset_at: f64,
}

/// The windows of the usage status; a window it states as `null` is absent.
fn usage_windows(rate_limit: Option<RateLimit>) -> ObservedWindows {
    let Some(rate_limit) = rate_limit else {
        return ObservedWindows::default();
    };
    let windows = [
        (Kind::Primary, rate_limit.primary_window),
        (Kind::Secondary, rate_limit.secondary_window),
    ];
    let mut report = ObservedWindows::default();
    for (kind, window) in windows {
        match window {
            None => {}
            Some(None) => report.absent.push(kind.id().into()),
            Some(Some(window)) => report.windows.push(window_of(
                kind,
                clamp_used_percent(window.used_percent),
                Some(window.limit_window_seconds / 60.0),
                unix_seconds(window.reset_at),
            )),
        }
    }
    report
}

/// What the `x-codex-<kind>-*` headers say about a window: nothing without
/// its `used-percent` header; otherwise a window, even with a value Demi
/// cannot read, unless the headers are the placeholder Codex sends for a
/// window the account does not have: nothing used, a length of zero or none,
/// and no reset time. The Codex CLI reads its headers by the same rule.
fn header_window(headers: &HeaderMap, kind: Kind) -> Option<Reported> {
    let prefix = format!("x-codex-{}", kind.id());
    let used_name = format!("{prefix}-used-percent");
    headers.get(&used_name)?;
    let used = header_number(headers, &used_name);
    let minutes = header_number(headers, &format!("{prefix}-window-minutes"));
    let reset = header_number(headers, &format!("{prefix}-reset-at"));
    if used == Some(0.0) && minutes.is_none_or(|minutes| minutes == 0.0) && reset.is_none() {
        return Some(Reported::Absent);
    }
    Some(Reported::Window(window_of(
        kind,
        used.and_then(clamp_used_percent),
        minutes,
        reset.and_then(unix_seconds),
    )))
}

fn window_of(
    kind: Kind,
    used_percent: Option<f64>,
    minutes: Option<f64>,
    resets_at: Option<demi_shared_types::Timestamp>,
) -> QuotaWindow {
    QuotaWindow {
        id: kind.id().into(),
        kind: QuotaKind::Limit,
        used_percent,
        used: None,
        limit: None,
        unit: Some(QuotaUnit::Percent),
        resets_at,
        // A length the vendor states in fractions of a second is none.
        period_seconds: minutes
            .map(|minutes| minutes * 60.0)
            .filter(|seconds| *seconds >= 1.0 && seconds.fract() == 0.0)
            .map(|seconds| seconds as u64),
        severity: severity(used_percent),
        scope: None,
    }
}

