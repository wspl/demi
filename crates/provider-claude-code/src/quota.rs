//! The quota of a Claude Code account (`usage-and-quota.md` § Claude Code):
//! a probe that asks the CLI, in a process of the account's directory that
//! sends no message, for its usage (`get_usage`), and the
//! `rate_limit_event` lines the CLI prints when the vendor's rate-limit
//! headers change.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use demi_host_interface::{Process, ProcessOutput};
use demi_provider_common::quota::{
    Observation, ObservedWindows, ProbeCost, ProbeReading, QuotaError, QuotaSource,
    clamp_used_percent, plan_label, rfc3339, severity,
};
use demi_provider_common::wire::{NonEmpty, Reported, Tagged};
use demi_shared_types::{QuotaKind, QuotaPlan, QuotaScope, QuotaSeverity, QuotaUnit, QuotaWindow, StreamKind, Timestamp};
use futures_util::future::BoxFuture;
use futures_util::stream::LocalBoxStream;
use futures_util::{FutureExt as _, StreamExt as _};
use serde::Deserialize;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::account::ClaudeAuth;
use crate::cli;
use crate::input::{self, ControlRequest, Input};
use crate::live::{Handle, Tail, exit_message};
use crate::output::{ControlResponseBody, Line};
use crate::placement::{AccountMachine, AccountWork, CliWork, Placed, Placement};

/// How long a probe waits for the CLI's answer, such as one held behind a
/// lock a killed process left (`claude-code.md` § Accounts and sign-in).
const PROBE_LIMIT: Duration = Duration::from_secs(30);

/// The id of the probe's `get_usage` request.
const USAGE_REQUEST: &str = "usage";

/// The quota source of one Claude Code account.
pub(crate) struct ClaudeQuota {
    pub(crate) auth: Arc<ClaudeAuth>,
    /// The machine the probe's process runs on; none for a provider built
    /// without one, which cannot probe.
    pub(crate) machine: Option<Arc<dyn AccountMachine>>,
    /// The provider entry the probe is work of.
    pub(crate) entry: String,
}

impl QuotaSource for ClaudeQuota {
    fn probe_cost(&self) -> Option<ProbeCost> {
        Some(ProbeCost::Free)
    }

    /// The CLI's `get_usage` answer, in a process of the account's
    /// directory on the Claude Code machine: its `rate_limits`, the vendor's
    /// usage object, and its `subscription_type`, the plan. A sign-in an
    /// earlier release kept in the account's record is handed over first.
    fn probe(&self) -> BoxFuture<'_, Result<ProbeReading, QuotaError>> {
        Box::pin(async move {
            let machine = self.machine.clone().ok_or_else(|| {
                QuotaError::Unavailable("Claude Code's machine is not available".into())
            })?;
            let account = self
                .auth
                .work()
                .await
                .map_err(|failure| failure.quota_error())?;
            let (answer, answered) = oneshot::channel();
            let work = account.work;
            let probe: AccountWork = Box::new(move |placement, abandoned| {
                Box::pin(async move {
                    let probed = ask_usage(&*placement, work, abandoned).await;
                    // A probe nobody waits for any more reports to nobody.
                    let _ = answer.send(probed);
                })
            });
            machine
                .run(self.entry.clone(), probe)
                .await
                .map_err(|error| QuotaError::Unavailable(error.0))?;
            let usage = answered.await.unwrap_or_else(|_| {
                Err(QuotaError::Unavailable(
                    "Claude Code's usage probe ended without an answer".into(),
                ))
            })?;
            if let Some(version) = account.handed_over {
                self.auth.handed_over(version).await;
            }
            let invalid = |reason: &str| {
                QuotaError::Invalid(format!("Claude Code's usage answer cannot be read: {reason}"))
            };
            let answer = UsageAnswer::deserialize(&usage).map_err(|error| invalid(&error.to_string()))?;
            let rate_limits = answer
                .rate_limits
                .ok_or_else(|| invalid("it holds no rate limits"))?;
            let usage = usage_of(&rate_limits).ok_or_else(|| invalid("its rate limits are not an object"))?;
            Ok(ProbeReading {
                plan: answer
                    .subscription_type
                    .filter(|plan| !plan.is_empty())
                    .map(|plan| QuotaPlan {
                        label: plan_label(&plan),
                        id: plan,
                    }),
                account_label: None,
                windows: usage.windows(),
                absent: Vec::new(),
            })
        })
    }

    /// The windows a `rate_limit_event` line of the CLI's output reports.
    fn observe(&self, observation: Observation<'_>) -> Option<ObservedWindows> {
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
        (!windows.is_empty()).then(|| windows.into())
    }
}

/// What Demi reads of the CLI's `get_usage` answer (Claude Code 2.1.301).
#[derive(Deserialize)]
struct UsageAnswer {
    /// The vendor's usage object, null when the CLI did not read it.
    #[serde(default)]
    rate_limits: Option<serde_json::Value>,
    /// Such as `max`.
    #[serde(default)]
    subscription_type: Option<String>,
}

/// Starts the probe's process for `work` through `placement`, asks it for
/// its usage and closes it, within thirty seconds: the answer of its
/// `get_usage` request. It gives up when nobody waits for it any more
/// (`abandoned`). Its process is held outside what gives up, and closed,
/// SIGTERM first, however the probe ends: on its answer, its timeout or
/// its abandonment alike (`claude-code.md` § Process lifetime).
async fn ask_usage(
    placement: &dyn Placement,
    work: CliWork,
    abandoned: CancellationToken,
) -> Result<serde_json::Value, QuotaError> {
    let unavailable = QuotaError::Unavailable;
    let timed_out = || {
        unavailable(format!(
            "Claude Code did not answer its usage request within {} seconds",
            PROBE_LIMIT.as_secs()
        ))
    };
    let given_up = || unavailable("The usage probe was given up".into());
    let _work = placement.work();
    let deadline = tokio::time::Instant::now() + PROBE_LIMIT;
    // A start given up removes what it made.
    let started = tokio::select! {
        started = tokio::time::timeout_at(deadline, placement.start(work, Arc::new(cli::probe), &|_| {})) => started,
        () = abandoned.cancelled() => return Err(given_up()),
    };
    let Placed {
        process, config, ..
    } = started
        .map_err(|_| timed_out())?
        .map_err(|error| unavailable(error.0))?;
    let Process {
        output,
        control,
        exit,
    } = process;
    let exit = exit.shared();
    let mut handle = Handle::of(control, exit.clone(), Some(config));
    let asked = async {
        let lines = [
            input::line(&Input::ControlRequest {
                request_id: "initialize",
                request: ControlRequest::InitializeBare {},
            }),
            input::line(&Input::ControlRequest {
                request_id: USAGE_REQUEST,
                request: ControlRequest::GetUsage {
                    skip_behaviors: true,
                },
            }),
        ]
        .concat();
        match handle.control().write_stdin(lines.into()).await {
            Ok(()) => answer(output).await,
            Err(error) => Answer::Failed(format!(
                "Claude Code's usage request could not be written: {error}"
            )),
        }
    };
    let answered = tokio::select! {
        answered = tokio::time::timeout_at(deadline, asked) => answered.map_err(|_| timed_out()),
        () = abandoned.cancelled() => Err(given_up()),
    };
    handle.close().await;
    match answered? {
        Answer::Usage(usage) => Ok(usage),
        Answer::Failed(message) => Err(unavailable(message)),
        Answer::Ended(stderr) if !stderr.is_empty() => Err(unavailable(stderr)),
        Answer::Ended(_) => Err(unavailable(exit_message(&exit.await))),
    }
}

/// What the probe's process answered.
enum Answer {
    /// What its `get_usage` request answered.
    Usage(serde_json::Value),
    /// It refused the request, or could not be asked.
    Failed(String),
    /// Its output ended first, with the tail of its standard error.
    Ended(String),
}

/// Reads the CLI's output until the answer to its `get_usage` request.
async fn answer(mut output: LocalBoxStream<'static, ProcessOutput>) -> Answer {
    let mut stdout = Vec::new();
    let mut stderr = Tail::default();
    while let Some(chunk) = output.next().await {
        if chunk.stream == StreamKind::Stderr {
            stderr.push(&chunk.bytes);
            continue;
        }
        stdout.extend_from_slice(&chunk.bytes);
        while let Some(end) = stdout.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = stdout.drain(..=end).collect();
            // Any other line, the `system` line among them, says nothing of
            // the usage.
            let Ok(Tagged(Some(Line::ControlResponse(line)))) = serde_json::from_slice(&line) else {
                continue;
            };
            let Some(ControlResponseBody {
                subtype,
                request_id,
                error,
                response,
            }) = line.response
            else {
                continue;
            };
            if request_id.as_deref() != Some(USAGE_REQUEST) {
                continue;
            }
            if subtype.as_deref() != Some("success") {
                let reason = error.into_inner().unwrap_or_else(|| "no reason given".into());
                return Answer::Failed(format!("Claude Code refused its usage request: {reason}"));
            }
            return Answer::Usage(response.unwrap_or_default());
        }
    }
    Answer::Ended(stderr.text())
}

/// The windows Demi names, by the vendor's id, with their length and the
/// model a window on one model applies to.
const NAMED_WINDOWS: [(&str, u64, Option<&str>); 4] = [
    ("five_hour", FIVE_HOURS, None),
    ("seven_day", SEVEN_DAYS, None),
    ("seven_day_sonnet", SEVEN_DAYS, Some("Sonnet")),
    ("seven_day_opus", SEVEN_DAYS, Some("Opus")),
];

const FIVE_HOURS: u64 = 5 * 60 * 60;
const SEVEN_DAYS: u64 = 7 * 24 * 60 * 60;

/// A named window with the share used and its reset time.
fn named_window(
    id: &str,
    used_percent: Option<f64>,
    resets_at: Option<Timestamp>,
) -> Option<QuotaWindow> {
    let (id, period, model) = NAMED_WINDOWS
        .into_iter()
        .find(|(named, ..)| *named == id)?;
    Some(QuotaWindow {
        id: id.into(),
        kind: QuotaKind::Limit,
        used_percent,
        used: None,
        limit: None,
        unit: Some(QuotaUnit::Percent),
        resets_at,
        period_seconds: Some(period),
        requested_at: None,
        severity: severity(used_percent),
        scope: model.map(|model| QuotaScope {
            kind: "model".into(),
            label: Some(model.into()),
        }),
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
fn usage_of(value: &serde_json::Value) -> Option<Usage> {
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
            let period_seconds = limit_period(&kind);
            let (id, scope) = match model {
                Some(model) => (
                    format!("limit:{kind}:{model}"),
                    QuotaScope {
                        kind: "model".into(),
                        label: Some(model),
                    },
                ),
                None => (format!("limit:{kind}"), QuotaScope { kind, label: None }),
            };
            windows.push(QuotaWindow {
                id,
                kind: QuotaKind::Limit,
                used_percent,
                used: None,
                limit: None,
                unit: Some(QuotaUnit::Percent),
                resets_at: reset_time(limit.resets_at),
                period_seconds,
                requested_at: None,
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

/// The period of a limit of the probe, by its kind, never by the vendor's
/// identifier: a `weekly_*` kind has a week and a session kind five hours;
/// another kind's period Demi cannot tell (`usage-and-quota.md` § Claude
/// Code).
fn limit_period(kind: &str) -> Option<u64> {
    if kind.starts_with("weekly") {
        Some(SEVEN_DAYS)
    } else if kind.starts_with("session") {
        Some(FIVE_HOURS)
    } else {
        None
    }
}

/// A reset time the usage answers state as an RFC 3339 time; any other value
/// is an unknown reset time.
fn reset_time(text: Reported<String>) -> Option<Timestamp> {
    rfc3339(&text.into_inner()?)
}
