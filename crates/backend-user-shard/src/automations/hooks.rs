//! A URL trigger's calls.
//!
//! # Who may start a run
//!
//! A `POST` starts a run when it carries the secret in one of three ways:
//!
//! - as a bearer token;
//! - signed with the secret as GitHub signs;
//! - signed as Stripe signs, with the signing secret the user pasted.
//!
//! Any other call is refused, and the caller cannot tell an unknown address
//! from a wrong secret.
//!
//! # Limits and bursts
//!
//! - A delivery ID seen in the last hour is answered as the first and starts
//!   nothing.
//! - Once the day's limit is reached, a call is answered and counted in the
//!   day's skipped run.
//! - Calls in a burst make one run: each restarts a 30-second wait, up to 2
//!   minutes from the first.
//! - A call that comes while a run of the automation works waits for its end
//!   and joins the next one.
//!
//! Each call is kept in the database as it is taken, with its delivery ID,
//! until a run takes it, so a restart loses none and starts the wait again. The
//! edge holds the size and rate limits.

use std::rc::Rc;
use std::time::Duration;

use bytes::Bytes;
use demi_backend_database::StorageError;
use demi_backend_database::automations::{NewCall, PendingCall, RunCause};
use demi_backend_providers::vault::seal::{Row, VaultKey};
use demi_host_interface::{FileContents, Host, WhenExists, WriteOptions};
use demi_shared_types::{AutomationId, AutomationRunId, Timestamp};
use demi_web_api_protocol::automations::{RunState, Trigger};
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
/// How old a Stripe signature may be, as Stripe advises.
pub(crate) const STRIPE_TOLERANCE_SECONDS: i64 = 300;

/// The headers a run's message leaves out: the secret and what proves it, and
/// those Demi's own server and the proxies and CDNs in front of it add, which
/// describe Demi, not the call.
const STRIPPED: [&str; 14] = [
    "authorization",
    "cookie",
    "x-hub-signature",
    "x-hub-signature-256",
    "stripe-signature",
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

/// A call of an automation's URL as the edge received it.
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

/// A burst of an automation's calls waiting out its quiet; its calls wait
/// in the database.
pub(crate) struct Burst {
    /// Woken when a call restarts the wait.
    restarted: Rc<Notify>,
    timer: AbortOnDropHandle<()>,
}

/// The id a call no automation takes is checked as, and the secrets its
/// check opens: the same work as for a wrong secret, so the time of an
/// answer does not tell an unknown address from a known one.
const UNKNOWN: &str = "00000000000000000000000000000000";

/// Does for a call no automation takes the work a call with a wrong secret
/// costs: a sealed secret and a Stripe secret opened, and the call checked
/// against them; it never passes.
pub fn check_unknown(key: &VaultKey, call: &HookCall, now: Timestamp) {
    static SEALED: std::sync::OnceLock<(Vec<u8>, Vec<u8>)> = std::sync::OnceLock::new();
    let (secret, stripe) = SEALED.get_or_init(|| {
        let mut random = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::rng(), &mut random);
        let text = hex::encode(random);
        (
            key.seal(Row::AutomationSecret(UNKNOWN), text.as_bytes()),
            key.seal(Row::AutomationStripeSecret(UNKNOWN), text.as_bytes()),
        )
    });
    // Neither result matters: a secret no caller knows passes nothing.
    let secret = key.open(Row::AutomationSecret(UNKNOWN), secret).unwrap_or_default();
    let stripe = key.open(Row::AutomationStripeSecret(UNKNOWN), stripe).unwrap_or_default();
    let secret = String::from_utf8_lossy(&secret);
    let stripe = String::from_utf8_lossy(&stripe);
    std::hint::black_box(verified(call, &secret, Some(&stripe), now));
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

/// Whether the call proves it holds `secret` or was signed by Stripe with
/// `stripe`, at `now`.
pub(crate) fn verified(call: &HookCall, secret: &str, stripe: Option<&str>, now: Timestamp) -> bool {
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
    if let (Some(stripe), Some(value)) = (stripe, header(call, "stripe-signature")) {
        let mut time = None;
        let mut signatures = Vec::new();
        for part in value.split(',') {
            match part.trim().split_once('=') {
                Some(("t", value)) => time = value.parse::<i64>().ok(),
                Some(("v1", value)) => signatures.extend(hex::decode(value).ok()),
                _ => {}
            }
        }
        if let Some(time) = time {
            let age = now.as_millisecond() / 1000 - time;
            if age.abs() <= STRIPE_TOLERANCE_SECONDS {
                let stamp = format!("{time}.");
                return signatures
                    .iter()
                    .any(|signature| signs(stripe.as_bytes(), &[stamp.as_bytes(), &call.body], signature));
            }
        }
    }
    false
}

impl Shard {
    /// A call of the automation `id`'s URL, which the edge found to be the
    /// user's.
    pub async fn hook_call(&self, id: &AutomationId, call: HookCall) -> Result<HookAnswer, StorageError> {
        let control = &self.services().control;
        let automation = control
            .automation(self.user().clone(), id.clone())
            .await?
            .filter(|automation| matches!(automation.trigger, Trigger::Url {}));
        let now = control.now();
        let (secret, stripe) = match &automation {
            Some(_) => self.automation_secrets(id).await?,
            None => (None, None),
        };
        let (Some(automation), Some(secret)) = (automation, secret) else {
            check_unknown(self.services().vault.key(), &call, now);
            return Ok(HookAnswer::Refused);
        };
        if !verified(&call, &secret, stripe.as_deref(), now) {
            return Ok(HookAnswer::Refused);
        }
        let delivery = header(&call, "x-github-delivery")
            .or_else(|| header(&call, "idempotency-key"))
            .map(str::to_owned);
        // A paused automation starts nothing, as Run Now alone would; one
        // at its daily limit counts the call in the day's skipped run.
        let limited = !automation.paused && self.limit_reached(&automation, now).await?.is_some();
        let held = automation.paused || limited;
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
            self.count_held_back(&automation, 1).await?;
        }
        // A call while a run works waits for its end, which gathers it.
        if !held && !self.automation_working(id).await? {
            self.gather(id);
        }
        Ok(HookAnswer::Taken)
    }

    /// Whether a run of the automation works now.
    async fn automation_working(&self, id: &AutomationId) -> Result<bool, StorageError> {
        Ok(self
            .services()
            .control
            .automation_runs(id.clone(), demi_web_api_protocol::automations::RUNS_SHOWN)
            .await?
            .iter()
            .any(|run| run.state == RunState::Running))
    }

    /// Starts the automation's burst, or restarts its wait.
    fn gather(&self, id: &AutomationId) {
        let mut pending = self.automations().pending.borrow_mut();
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

    /// A run of the automation ended: the calls that came while it worked
    /// start the next burst.
    pub(crate) async fn run_ended(&self, id: &AutomationId) {
        match self.services().control.has_pending_calls(id.clone()).await {
            Ok(true) => self.gather(id),
            Ok(false) => {}
            Err(error) => tracing::warn!(automation = %id, "the calls waiting for a run were not read: {error}"),
        }
    }

    /// As the shard starts: the calls a run of the backend before kept
    /// waiting start their bursts again, unless a run left working takes
    /// them at its end.
    pub(crate) async fn resume_pending_calls(&self) {
        let ids = match self.services().control.automations_with_pending_calls(self.user().clone()).await {
            Ok(ids) => ids,
            Err(error) => {
                tracing::warn!(user = %self.user(), "the calls waiting for runs were not read: {error}");
                return;
            }
        };
        for id in ids {
            match self.automation_working(&id).await {
                Ok(true) => {}
                Ok(false) => self.gather(&id),
                Err(error) => tracing::warn!(automation = %id, "the calls waiting for a run stay: {error}"),
            }
        }
    }

    /// Writes each call's body to the conversation's scratch directory on
    /// its Host, never the project (`crates/runner-jobs/src/scratch.rs`), and
    /// answers each file's path, none for an empty body.
    pub(crate) async fn write_bodies(
        &self,
        conversation: &ConversationId,
        run: &AutomationRunId,
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
                    let file = format!("automation-{short}-call-{}.{}", index + 1, if json { "json" } else { "txt" });
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

/// Waits until the burst of the automation `id` is quiet for 30 seconds,
/// or 2 minutes passed since its first call, and fires one run with every
/// call waiting, unless the automation was paused, its trigger changed or it
/// was deleted since `generation`.
async fn gather_until_quiet(
    shard: std::rc::Weak<Shard>,
    id: AutomationId,
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
        let mut pending = shard.automations().pending.borrow_mut();
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
    let automation = match control.automation(shard.user().clone(), id.clone()).await {
        Ok(Some(automation)) if !automation.paused && matches!(automation.trigger, Trigger::Url {}) => automation,
        Ok(_) => return,
        Err(error) => {
            tracing::warn!(automation = %id, "a URL's calls started no run yet: {error}");
            return;
        }
    };
    let calls = match control.take_pending_calls(id.clone()).await {
        Ok(calls) => calls,
        Err(error) => {
            tracing::warn!(automation = %id, "a URL's calls started no run yet: {error}");
            return;
        }
    };
    // A pause or an edit while the calls were read drops them.
    if calls.is_empty() || !shard.automation_current(&id, generation) {
        return;
    }
    let at = control.now();
    shard
        .fire(
            &automation,
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
pub const CALLS_PER_MINUTE: u32 = 30;

const MINUTE: Duration = Duration::from_secs(60);

/// The client addresses whose calls of unknown URLs are counted at once;
/// past it, the address counted longest ago is forgotten.
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

/// The calls the URLs took this minute: each automation's in a window of
/// its own, which its deletion drops; a URL no automation has, by the
/// address it was called from, of a bounded number of addresses.
#[derive(Default)]
pub struct HookLimits(std::sync::Mutex<Limits>);

#[derive(Default)]
struct Limits {
    known: std::collections::HashMap<AutomationId, Window>,
    unknown: std::collections::HashMap<Option<std::net::IpAddr>, Window>,
    /// The addresses of `unknown` in the order they were first counted.
    order: std::collections::VecDeque<Option<std::net::IpAddr>>,
}

impl HookLimits {
    /// Counts a call of the automation `id`; false when it is over the limit.
    pub fn admit_known(&self, id: &AutomationId, now: std::time::Instant) -> bool {
        let mut limits = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        limits
            .known
            .entry(id.clone())
            .or_insert(Window { start: now, calls: 0 })
            .admit(now)
    }

    /// Drops the window of the deleted automation `id`.
    pub(crate) fn forget(&self, id: &AutomationId) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .known
            .remove(id);
    }

    /// Counts a call of a URL no automation has from `client`; false when
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

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "s3cret";
    const STRIPE: &str = "whsec_stripe";

    fn call(headers: &[(&str, &str)], body: &str) -> HookCall {
        HookCall {
            method: "POST".into(),
            headers: headers.iter().map(|(name, value)| ((*name).to_owned(), (*value).to_owned())).collect(),
            body: Bytes::from(body.to_owned()),
        }
    }

    fn hmac_hex(key: &str, data: &str) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes()).unwrap();
        mac.update(data.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    /// Each way a caller proves the secret: a bearer token, GitHub's
    /// signature of the body, and Stripe's of its time and the body with
    /// the secret the user pasted, no older than five minutes; anything
    /// else is refused.
    #[test]
    fn a_call_passes_only_with_the_secret_or_its_signature() {
        let now = Timestamp::from_millisecond(1_800_000_000_000).unwrap();
        let body = r#"{"action":"opened"}"#;
        let github = format!("sha256={}", hmac_hex(SECRET, body));
        let stamp = now.as_millisecond() / 1000;
        let stripe = |time: i64| format!("t={time},v1={}", hmac_hex(STRIPE, &format!("{time}.{body}")));
        let cases: Vec<(&str, Vec<(&str, String)>, bool)> = vec![
            ("bearer", vec![("authorization", format!("Bearer {SECRET}"))], true),
            ("wrong bearer", vec![("authorization", "Bearer s3cres".into())], false),
            ("no proof", vec![], false),
            ("github", vec![("x-hub-signature-256", github.clone())], true),
            ("github of another body", vec![("x-hub-signature-256", format!("sha256={}", hmac_hex(SECRET, "{}")))], false),
            ("github with the stripe secret", vec![("x-hub-signature-256", format!("sha256={}", hmac_hex(STRIPE, body)))], false),
            ("stripe", vec![("stripe-signature", stripe(stamp))], true),
            ("stripe four minutes old", vec![("stripe-signature", stripe(stamp - 240))], true),
            ("stripe six minutes old", vec![("stripe-signature", stripe(stamp - 360))], false),
            ("stripe signed with the url secret", vec![("stripe-signature", format!("t={stamp},v1={}", hmac_hex(SECRET, &format!("{stamp}.{body}"))))], false),
        ];
        for (name, headers, passes) in cases {
            let headers: Vec<(&str, &str)> = headers.iter().map(|(name, value)| (*name, value.as_str())).collect();
            assert_eq!(verified(&call(&headers, body), SECRET, Some(STRIPE), now), passes, "{name}");
        }
        // Without a pasted Stripe secret, Stripe's signature proves nothing.
        assert!(!verified(&call(&[("stripe-signature", &stripe(stamp))], body), SECRET, None, now));
    }

    /// Calls of unknown URLs from ever new addresses keep a bounded count
    /// of addresses, and each automation's window refuses its 31st call of
    /// a minute and takes calls again the next minute. Microseconds.
    #[test]
    fn the_limits_keep_bounded_state() {
        let limits = HookLimits::default();
        let now = std::time::Instant::now();
        for number in 0..(UNKNOWN_CALLERS as u32 + 1000) {
            assert!(limits.admit_unknown(Some(std::net::IpAddr::from(number.to_be_bytes())), now));
        }
        let held = limits.0.lock().unwrap();
        assert_eq!((held.unknown.len(), held.order.len()), (UNKNOWN_CALLERS, UNKNOWN_CALLERS));
        drop(held);
        let id = AutomationId::try_from("0123456789abcdef0123456789abcdef".to_owned()).unwrap();
        let admitted = (0..31).filter(|_| limits.admit_known(&id, now)).count();
        assert_eq!(admitted, CALLS_PER_MINUTE as usize);
        assert!(limits.admit_known(&id, now + MINUTE));
        // A deleted automation's window goes with it.
        limits.forget(&id);
        assert!(limits.0.lock().unwrap().known.is_empty());
    }
}
