//! A URL trigger's calls.
//!
//! # Who may start a run
//!
//! A `POST` starts a run when it carries the secret in one of two ways:
//!
//! - as a bearer token;
//! - signed with the secret as GitHub signs.
//!
//! Any other call is refused with the answer an unknown address gets. The
//! answer is the same, but not in every case: an unknown address is refused
//! before its body is read, so only a call to a known address meets the
//! limit on the body's size and the wait for a body that does not finish.
//!
//! # Limits and bursts
//!
//! - A delivery ID seen in the last hour is answered as the first and starts
//!   nothing.
//! - Once the day's limit is reached, a call is answered and counted in the
//!   day's skipped run.
//! - Calls in a burst make one run: each restarts a 30-second wait, up to 2
//!   minutes from the first.
//! - A call that comes while a run of the schedule works waits for its end
//!   and joins the next one.
//!
//! Each call is kept in the database as it is taken, with its delivery ID,
//! until a run takes it, so a restart loses none and starts the wait again. The
//! edge holds the size and rate limits.

use std::rc::Rc;
use std::time::Duration;

use bytes::Bytes;
use demi_backend_database::StorageError;
use demi_backend_database::schedules::{NewCall, PendingCall, RunCause};
use demi_host_interface::{FileContents, Host, WhenExists, WriteOptions};
use demi_shared_types::{ScheduleId, ScheduleRunId, Timestamp};
use demi_web_api_protocol::schedules::{RunState, Trigger};
use demi_web_api_protocol::ids::ConversationId;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use tokio::sync::Notify;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tokio_util::task::AbortOnDropHandle;

use super::runs::{Fired, Payload};
use crate::shard::Shard;

/// The wait each call of a burst restarts.
pub(crate) const BURST_WAIT: Duration = Duration::from_secs(30);
/// The longest a burst gathers calls, from its first.
pub(crate) const BURST_MAX: Duration = Duration::from_secs(120);
/// How long a delivery ID is remembered.
pub(crate) const DELIVERY_MEMORY: Duration = Duration::from_secs(3600);

/// The headers a run's message leaves out: the secret and what proves it, and
/// those Demi's own server and the proxies and CDNs in front of it add, which
/// describe Demi, not the call.
const STRIPPED: [&str; 13] = [
    "authorization",
    "cookie",
    "x-hub-signature",
    "x-hub-signature-256",
    "host",
    "x-real-ip",
    "via",
    "forwarded",
    "cdn-loop",
    "true-client-ip",
    "x-client-ip",
    "x-cluster-client-ip",
    "x-request-start",
];
/// The prefixes of the headers proxies and CDNs add.
const STRIPPED_PREFIXES: [&str; 7] = ["x-forwarded-", "cf-", "x-amz-cf-", "fastly-", "x-vercel-", "x-azure-", "akamai-"];

/// Whether a run's message shows the header `name`, in lowercase.
fn shown_header(name: &str) -> bool {
    !STRIPPED.contains(&name) && !STRIPPED_PREFIXES.iter().any(|prefix| name.starts_with(prefix))
}

/// A call of a schedule's URL as the edge received it.
#[derive(Debug, Clone)]
pub struct HookCall {
    pub method: String,
    /// Each header, its name in lowercase, in the order they came.
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
}

/// How a call is answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookAnswer {
    /// 200: it was taken, whether or not it starts a run.
    Taken,
    /// 401: an unknown address, or a call without the secret.
    Refused,
}

/// A call as a run's message carries it: when, how, and its headers
/// without what proves the secret.
#[derive(Debug, Clone)]
pub(crate) struct Call {
    pub at: Timestamp,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
}

impl From<PendingCall> for Call {
    fn from(call: PendingCall) -> Self {
        Self {
            at: call.at,
            method: call.method,
            headers: call.headers,
            body: Bytes::from(call.body),
        }
    }
}

/// A burst of a schedule's calls waiting out its quiet; its calls wait
/// in the database.
pub(crate) struct Burst {
    /// Woken when a call restarts the wait.
    restarted: Rc<Notify>,
    timer: AbortOnDropHandle<()>,
}

/// The value of the header `name`, in lowercase, the first of them.
fn header<'a>(call: &'a HookCall, name: &str) -> Option<&'a str> {
    call.headers
        .iter()
        .find(|(each, _)| each == name)
        .map(|(_, value)| value.as_str())
}

fn signs(key: &[u8], parts: &[&[u8]], signature: &[u8]) -> bool {
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(key) else {
        return false;
    };
    for part in parts {
        mac.update(part);
    }
    // `verify_slice` compares in constant time.
    mac.verify_slice(signature).is_ok()
}

/// Whether the call proves it holds `secret`.
pub(crate) fn verified(call: &HookCall, secret: &str) -> bool {
    if let Some(token) = header(call, "authorization").and_then(|value| value.strip_prefix("Bearer "))
        && bool::from(token.trim().as_bytes().ct_eq(secret.as_bytes()))
    {
        return true;
    }
    if let Some(signature) = header(call, "x-hub-signature-256").and_then(|value| value.strip_prefix("sha256="))
        && let Ok(signature) = hex::decode(signature.trim())
        && signs(secret.as_bytes(), &[&call.body], &signature)
    {
        return true;
    }
    false
}

impl Shard {
    /// A call of the schedule `id`'s URL, which the edge found to be the
    /// user's.
    pub async fn hook_call(&self, id: &ScheduleId, call: HookCall) -> Result<HookAnswer, StorageError> {
        let control = &self.services().control;
        let schedule = control
            .schedule(id.clone())
            .await?
            .filter(|schedule| matches!(schedule.trigger, Trigger::Url {}));
        let now = control.now();
        let secret = match &schedule {
            Some(_) => self.schedule_secret(id).await?,
            None => None,
        };
        let (Some(schedule), Some(secret)) = (schedule, secret) else {
            return Ok(HookAnswer::Refused);
        };
        if !verified(&call, &secret) {
            return Ok(HookAnswer::Refused);
        }
        let delivery = header(&call, "x-github-delivery")
            .or_else(|| header(&call, "idempotency-key"))
            .map(str::to_owned);
        // A paused schedule, or one of an archived conversation, starts nothing, as
        // `demi schedule run` alone would; one at its daily limit counts the call in
        // the day's skipped run.
        let limited = schedule.on() && self.limit_reached(&schedule, now).await?.is_some();
        let held = !schedule.on() || limited;
        let since = now
            .to_jiff()
            .checked_sub(DELIVERY_MEMORY)
            .map(Timestamp::truncate)
            .unwrap_or(now);
        let kept = NewCall {
            at: now,
            delivery,
            held,
            method: call.method,
            headers: call.headers.into_iter().filter(|(name, _)| shown_header(name)).collect(),
            body: call.body.to_vec(),
        };
        // Kept with its delivery ID at once: a delivery sent again is
        // answered as the first, and a restart loses no call.
        if !control.accept_call(id.clone(), kept, since).await? {
            return Ok(HookAnswer::Taken);
        }
        if limited {
            self.count_held_back(&schedule, 1).await?;
        }
        // A call while a run works waits for its end, which gathers it.
        if !held && !self.schedule_working(id).await? {
            self.gather(id);
        }
        Ok(HookAnswer::Taken)
    }

    /// Whether a run of the schedule works now.
    async fn schedule_working(&self, id: &ScheduleId) -> Result<bool, StorageError> {
        Ok(self
            .services()
            .control
            .schedule_runs(id.clone(), demi_web_api_protocol::schedules::RUNS_SHOWN)
            .await?
            .iter()
            .any(|run| run.state == RunState::Running))
    }

    /// Starts the schedule's burst, or restarts its wait.
    fn gather(&self, id: &ScheduleId) {
        let mut pending = self.schedules().pending.borrow_mut();
        let entry = pending.entry(id.clone()).or_default();
        match &entry.burst {
            Some(burst) => burst.restarted.notify_one(),
            None => {
                let restarted = Rc::new(Notify::new());
                let task = self.tasks().spawn_local(gather_until_quiet(
                    Rc::downgrade(&self.this()),
                    id.clone(),
                    entry.generation,
                    Instant::now(),
                    restarted.clone(),
                ));
                entry.burst = Some(Burst {
                    restarted,
                    timer: AbortOnDropHandle::new(task),
                });
            }
        }
    }

    /// A run of the schedule ended: the calls that came while it worked
    /// start the next burst.
    pub(crate) async fn run_ended(&self, id: &ScheduleId) {
        match self.services().control.has_pending_calls(id.clone()).await {
            Ok(true) => self.gather(id),
            Ok(false) => {}
            Err(error) => tracing::warn!(event = "schedule_calls.failed", schedule = %id, step = "run_ended", %error, "the calls waiting for a run were not read"),
        }
    }

    /// As the shard starts: the calls a run of the backend before kept
    /// waiting start their bursts again, unless a run left working takes
    /// them at its end.
    pub(crate) async fn resume_pending_calls(&self) {
        let ids = match self.services().control.schedules_with_pending_calls(self.user().clone()).await {
            Ok(ids) => ids,
            Err(error) => {
                tracing::warn!(event = "schedule_calls.failed", user = %self.user(), step = "read_all", %error, "the calls waiting for runs were not read");
                return;
            }
        };
        for id in ids {
            match self.schedule_working(&id).await {
                Ok(true) => {}
                Ok(false) => self.gather(&id),
                Err(error) => tracing::warn!(event = "schedule_calls.failed", schedule = %id, step = "keep", %error, "the calls waiting for a run stay"),
            }
        }
    }

    /// Writes each call's body to the conversation's scratch directory on
    /// its Host, never the project (`crates/runner-jobs/src/scratch.rs`), and
    /// answers each file's path, none for an empty body.
    pub(crate) async fn write_bodies(
        &self,
        conversation: &ConversationId,
        run: &ScheduleRunId,
        calls: &[Call],
    ) -> Result<Vec<Option<String>>, String> {
        let cancel = CancellationToken::new();
        self.host_shard()
            .with_host(conversation, None, &cancel, async |host| {
                let identity = host.host.identity();
                if identity.scratch_root.is_empty() {
                    return Err("the Host has not named its scratch directory".to_owned());
                }
                let scratch = identity.paths.join(&identity.scratch_root, &demi_command_protocol::scratch_name(conversation.as_str()));
                let mut written = Vec::with_capacity(calls.len());
                let short: String = run.as_str().chars().take(8).collect();
                for (index, call) in calls.iter().enumerate() {
                    if call.body.is_empty() {
                        written.push(None);
                        continue;
                    }
                    let json = call.headers.iter().any(|(name, value)| {
                        name == "content-type" && value.to_ascii_lowercase().contains("json")
                    });
                    let file = format!("schedule-{short}-call-{}.{}", index + 1, if json { "json" } else { "txt" });
                    let path = identity.paths.join(&scratch, &file);
                    let name = host
                        .host
                        .fs()
                        .write_file(&path, FileContents::Bytes(call.body.clone()), WriteOptions {
                            exists: WhenExists::Rename,
                        })
                        .await
                        .map_err(|error| error.message)?;
                    written.push(Some(identity.paths.join(&scratch, &name)));
                }
                Ok(written)
            })
            .await
            .map_err(|error| error.to_string())?
    }
}

/// Waits until the burst of the schedule `id` is quiet for 30 seconds,
/// or 2 minutes passed since its first call, and fires one run with every
/// call waiting, unless the schedule was paused, its trigger changed or it
/// was deleted since `generation`.
async fn gather_until_quiet(
    shard: std::rc::Weak<Shard>,
    id: ScheduleId,
    generation: u64,
    first: Instant,
    restarted: Rc<Notify>,
) {
    loop {
        let deadline = (Instant::now() + BURST_WAIT).min(first + BURST_MAX);
        tokio::select! {
            () = tokio::time::sleep_until(deadline) => break,
            () = restarted.notified() => {}
        }
        if Instant::now() >= first + BURST_MAX {
            break;
        }
    }
    let Some(shard) = shard.upgrade() else {
        return;
    };
    {
        let mut pending = shard.schedules().pending.borrow_mut();
        let Some(entry) = pending.get_mut(&id).filter(|entry| entry.generation == generation) else {
            return;
        };
        // The timer is this task: dropping its handle would stop it at its
        // next wait, so it is let go of instead.
        if let Some(burst) = entry.burst.take() {
            drop(burst.timer.detach());
        }
    }
    let control = &shard.services().control;
    let schedule = match control.schedule(id.clone()).await {
        Ok(Some(schedule)) if schedule.on() && matches!(schedule.trigger, Trigger::Url {}) => schedule,
        Ok(_) => return,
        Err(error) => {
            tracing::warn!(event = "schedule_url_run.deferred", schedule = %id, step = "read_schedule", %error, "a URL's calls started no run yet");
            return;
        }
    };
    let calls = match control.take_pending_calls(id.clone()).await {
        Ok(calls) => calls,
        Err(error) => {
            tracing::warn!(event = "schedule_url_run.deferred", schedule = %id, step = "take_calls", %error, "a URL's calls started no run yet");
            return;
        }
    };
    // A pause or an edit while the calls were read drops them.
    if calls.is_empty() || !shard.schedule_current(&id, generation) {
        return;
    }
    let at = control.now();
    shard
        .fire(
            &schedule,
            Fired {
                cause: RunCause::Url,
                task: None,
                at,
                payload: Payload::Calls(calls.into_iter().map(Call::from).collect()),
            },
        )
        .await;
}

/// The calls a URL takes a minute, the refused ones included; more are
/// answered 429.
///
/// Bounded because the URL is public: anyone who knows it can call, and each
/// call may start a run.
pub const CALLS_PER_MINUTE: u32 = 30;

const MINUTE: Duration = Duration::from_secs(60);

/// The client addresses whose calls of unknown URLs are counted at once;
/// past it, the address counted longest ago is forgotten.
///
/// Bounded because the callers are held in memory and anyone can add one.
const UNKNOWN_CALLERS: usize = 4096;

/// A minute's count of calls, from its first.
#[derive(Clone, Copy)]
struct Window {
    start: std::time::Instant,
    calls: u32,
}

impl Window {
    /// Counts a call at `now`; false when it is over the limit.
    fn admit(&mut self, now: std::time::Instant) -> bool {
        if now.duration_since(self.start) >= MINUTE {
            *self = Self { start: now, calls: 0 };
        }
        self.calls = self.calls.saturating_add(1);
        self.calls <= CALLS_PER_MINUTE
    }
}

/// The calls the URLs took this minute: each schedule's in a window of
/// its own, which its deletion drops; a URL no schedule has, by the
/// address it was called from, of a bounded number of addresses.
#[derive(Default)]
pub struct HookLimits(std::sync::Mutex<Limits>);

#[derive(Default)]
struct Limits {
    known: std::collections::HashMap<ScheduleId, Window>,
    unknown: std::collections::HashMap<Option<std::net::IpAddr>, Window>,
    /// The addresses of `unknown` in the order they were first counted.
    order: std::collections::VecDeque<Option<std::net::IpAddr>>,
}

impl HookLimits {
    /// Counts a call of the schedule `id`; false when it is over the limit.
    pub fn admit_known(&self, id: &ScheduleId, now: std::time::Instant) -> bool {
        let mut limits = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        limits
            .known
            .entry(id.clone())
            .or_insert(Window { start: now, calls: 0 })
            .admit(now)
    }

    /// Drops the window of the deleted schedule `id`.
    pub(crate) fn forget(&self, id: &ScheduleId) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .known
            .remove(id);
    }

    /// Counts a call of a URL no schedule has from `client`; false when
    /// it is over the limit.
    pub fn admit_unknown(&self, client: Option<std::net::IpAddr>, now: std::time::Instant) -> bool {
        let mut limits = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !limits.unknown.contains_key(&client) {
            if limits.order.len() >= UNKNOWN_CALLERS
                && let Some(oldest) = limits.order.pop_front()
            {
                limits.unknown.remove(&oldest);
            }
            limits.order.push_back(client);
        }
        limits
            .unknown
            .entry(client)
            .or_insert(Window { start: now, calls: 0 })
            .admit(now)
    }

}
