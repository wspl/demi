//! The Host's open devices (`crates/plugin-mobile/command/src/operations.rs`):
//! which conversation has each simulator open and under which handle, and when
//! each ends.
//!
//! A conversation has at most [`MAX_DEVICES`] devices, and no device is in two
//! conversations.
//!
//! - A device Demi booted shuts down when its conversation closes it or is
//!   released; a device the user booted is only let go. Nothing closes a
//!   device for being unused: it lasts until someone closes it or it is gone
//!   (`docs/decisions/0031-a-device-tab-lasts-as-long-as-the-device.md`).
//! - The program's end shuts nothing down and forgets nothing: the record
//!   ([`Kept`]) holds the simulators Demi booted and the open devices with
//!   their conversations, handles and owner, and is shared with every other
//!   version of the service running now.
//! - A conversation's devices are taken back when it first reaches the
//!   service, each under its handle, while it is *resuming*: listed as open,
//!   its tab saying it reconnects, and a command on it waiting, as long as
//!   its caller does. The take-back waits for a simulator to finish booting,
//!   and tries again while the Host cannot say whether the simulator is there
//!   or the device cannot be attached. Only a simulator that is not there, or
//!   is shut down, closes the device as lost.
//! - A device whose helper or picture stream ends while its simulator runs
//!   is resuming again, under the same handle: the connection ended, not the
//!   device.
//! - A conversation whose devices the record holds but which is no longer on
//!   this Host, as one deleted while the Host was offline, is released as
//!   the service starts.
//!
//! One owner changes the registry: opening and closing take turns on one lock,
//! so a device is never opened twice nor closed while it opens. The plugin
//! hears of each change of a conversation's devices, when it happens.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use tokio::{
    sync::{Notify, watch},
    time::Instant,
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use demi_command_package_mobile_protocol::mobile::{
    DeviceId, DevicesChange, ListResult, MAX_DEVICES, MobileMessage, MobilePlatform, PowerState,
    Simulator, Unavailable,
};
use demi_command_protocol::ServiceSequence;
use demi_command_sdk::{Messages, Numbers};
use demi_live_view_protocol::{LiveActivity, LivePoint};

use crate::{
    driver::{Device, Driver, DriverError},
    error::MobileError,
    kept::{Entry, Kept, Opened},
    refs::Refs,
};

/// How long the view still says the agent uses a device after its last
/// command on it ended (`crates/plugin-mobile/command/src/live.rs`).
pub const AGENT_LINGER: Duration = Duration::from_secs(3);

/// How often a service looks whether another one took over its devices.
const TAKEOVER_POLL: Duration = Duration::from_secs(2);

/// How long a command on a device being taken back waits for it.
pub const RESUME_WAIT: Duration = Duration::from_secs(30);
/// How long a device whose connection ended waits before it is attached again.
const RECONNECT_DELAY: Duration = Duration::from_secs(1);
/// How often a simulator that is still booting is looked at.
const BOOT_POLL: Duration = Duration::from_secs(1);
/// The longest wait between two tries of a failed step.
const RETRY_LAST: Duration = Duration::from_secs(30);

/// One open device.
pub struct OpenDevice {
    pub handle: DeviceId,
    pub conversation: String,
    pub native_id: String,
    pub platform: MobilePlatform,
    pub name: String,
    /// The model, such as iPhone 17 or iPad Air 13-inch (M3).
    pub model: String,
    pub os: String,
    /// Whether Demi booted it, so it shuts down when it closes.
    pub booted: bool,
    pub device: Arc<dyn Device>,
    /// The elements the device's inspects listed.
    pub refs: Mutex<Refs>,
    /// Whether the agent uses the device, and where it last tapped.
    pub activity: watch::Sender<LiveActivity>,
    /// The agent's commands on the device that run, and which of its uses
    /// the last one ended.
    agent: Mutex<AgentUse>,
    /// The recording under way: its file, in the conversation's scratch
    /// directory, and since when.
    pub recording: Mutex<Option<(std::path::PathBuf, Instant)>>,
    /// Counts the changes of what the device's views say of it besides its
    /// screen, such as its recording, which only its own views follow.
    changes: watch::Sender<u64>,
    /// The commands that act on the device, the user's panel controls and
    /// the agent's taps, swipes, typing and keys, rotations and appearance,
    /// take turns in the order they arrive (`crates/plugin-mobile/command/src/live.rs`).
    pub inputs: tokio::sync::Mutex<()>,
    /// The commands that read or wait, `inspect`, `wait`, `screenshot` and
    /// `record`, take turns apart from the input, which never waits for
    /// them.
    pub reads: tokio::sync::Mutex<()>,
    /// The files the panel's Screenshot and Record wrote into the
    /// conversation's scratch directory, removed when the device closes
    /// (`crates/plugin-mobile/command/src/live.rs`).
    pub saved: Mutex<Vec<std::path::PathBuf>>,
    /// Cancelled once this connection to the device is over, whatever ended
    /// it: the device closed, or its connection was lost and is made again.
    pub closed: CancellationToken,
    /// Cancelled, before `closed`, when the device itself is over, which its
    /// views hear as the device being gone.
    pub gone: CancellationToken,
}

impl OpenDevice {
    fn new(
        handle: DeviceId,
        conversation: &str,
        platform: MobilePlatform,
        simulator: crate::driver::Simulator,
        booted: bool,
        device: Arc<dyn Device>,
    ) -> Self {
        Self {
            handle,
            conversation: conversation.to_owned(),
            native_id: simulator.native_id,
            platform,
            name: simulator.name,
            model: simulator.model,
            os: simulator.os,
            booted,
            device,
            refs: Mutex::default(),
            activity: watch::Sender::new(LiveActivity {
                active: false,
                point: None,
            }),
            agent: Mutex::default(),
            recording: Mutex::new(None),
            changes: watch::Sender::new(0),
            saved: Mutex::default(),
            inputs: tokio::sync::Mutex::new(()),
            reads: tokio::sync::Mutex::new(()),
            closed: CancellationToken::new(),
            gone: CancellationToken::new(),
        }
    }

    /// Follows the changes of what the device's views say of it besides its
    /// screen.
    pub fn changes(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    /// A recording started or stopped: the device's views say so on every
    /// page, whoever started it.
    pub fn recorded(&self) {
        self.changes.send_modify(|changes| *changes += 1);
    }

    /// The agent uses the device until the guard drops and for
    /// [`AGENT_LINGER`] after, unless it uses it again meanwhile.
    pub fn agent_uses(self: &Arc<Self>) -> AgentUsing {
        {
            let mut agent = self.agent.lock().expect("the agent's use is intact");
            agent.running += 1;
            agent.generation += 1;
        }
        self.activity.send_if_modified(|activity| !std::mem::replace(&mut activity.active, true));
        AgentUsing(self.clone())
    }

    /// The agent tapped at `point`, which the view marks.
    pub fn agent_tapped(&self, point: LivePoint) {
        self.activity.send_modify(|activity| activity.point = Some(point));
    }
}

#[derive(Default)]
struct AgentUse {
    running: usize,
    generation: u64,
}

/// The agent's command running on a device.
pub struct AgentUsing(Arc<OpenDevice>);

impl Drop for AgentUsing {
    fn drop(&mut self) {
        let generation = {
            let mut agent = self.0.agent.lock().expect("the agent's use is intact");
            agent.running -= 1;
            if agent.running > 0 {
                return;
            }
            agent.generation
        };
        // The note stays a moment, then goes unless the agent came back;
        // the timer ends with the device too.
        let device = Arc::downgrade(&self.0);
        let closed = self.0.closed.clone();
        tokio::spawn(async move {
            tokio::select! {
                () = tokio::time::sleep(AGENT_LINGER) => {}
                () = closed.cancelled() => return,
            }
            let Some(device) = device.upgrade() else { return };
            let agent = device.agent.lock().expect("the agent's use is intact");
            if agent.running == 0 && agent.generation == generation {
                device.activity.send_if_modified(|activity| std::mem::replace(&mut activity.active, false));
            }
        });
    }
}

/// Why a device closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Closing {
    /// `close`, or its panel tab closed.
    Asked,
    /// The conversation was released.
    Released,
    /// The device itself went away.
    Lost,
    /// Another service took the device over: this one lets it go and
    /// changes nothing the record holds.
    Taken,
    /// The program ends: the device is let go and stays as it is.
    Ending,
}

impl Closing {
    /// Why, as the Host's log says it.
    fn words(self) -> &'static str {
        match self {
            Self::Asked => "closed by the agent or its tab",
            Self::Released => "its conversation was released",
            Self::Lost => "it went away",
            Self::Taken => "another service took it over",
            Self::Ending => "the service ends",
        }
    }
}

/// A device of this service whose connection is being made, for the first
/// time or again: it is listed as open under its handle, and its tab says it
/// reconnects.
struct Resuming {
    /// What the record holds of it, which names this service as its owner.
    entry: Opened,
    /// The files the device's panel saved and its unfinished recording,
    /// which go to the connection that replaces the lost one.
    carried: Vec<std::path::PathBuf>,
}

#[derive(Default)]
struct State {
    /// By the simulator's native id.
    open: HashMap<String, Arc<OpenDevice>>,
    /// The devices this service holds without a connection, by native id.
    resuming: HashMap<String, Resuming>,
    /// The conversations that reached this service, whose recorded devices
    /// it took over then.
    served: HashSet<String>,
    /// Closing began: no device is added to `open` any more.
    closing: bool,
}

/// What the Host says of a device's simulator.
enum Probe {
    /// It is not there, or shut down: the device is lost.
    Gone(String),
    /// It is still booting: its boot ends before it is attached.
    Booting,
    Booted(crate::driver::Simulator),
    /// The Host could not say, which is no proof the device is gone.
    Unknown(String),
}

/// The Host's open devices.
pub struct Devices {
    drivers: Vec<Arc<dyn Driver>>,
    state: Mutex<State>,
    /// Opening and closing take turns.
    turns: tokio::sync::Mutex<()>,
    numbers: OnceLock<Numbers>,
    announcer: Announcer,
    /// Counts the changes of the open devices, which the conversations'
    /// pages hear of through the announcer and a test follows; a device's
    /// views follow that device's own changes.
    revision: watch::Sender<u64>,
    tasks: TaskTracker,
    /// Cancelled once the service closes: its tasks end.
    stopping: CancellationToken,
    /// The simulators Demi booted and the open devices, across the
    /// programs' ends and shared with the other versions running now.
    kept: Kept,
    /// Names this service in the record.
    owner: String,
    /// Whether a conversation is still on this Host.
    exists: Box<dyn Fn(&str) -> bool + Send + Sync>,
}

impl Devices {
    /// The registry over `drivers` and the record `kept`. `exists` says
    /// whether a conversation is still on this Host: the record's devices of
    /// one that is not are released as the registry starts. The devices of
    /// the conversations that are on it are taken back one conversation at a
    /// time, when it first reaches the registry ([`Self::reached`]).
    pub fn new(
        drivers: Vec<Arc<dyn Driver>>,
        kept: Kept,
        exists: Box<dyn Fn(&str) -> bool + Send + Sync>,
    ) -> Arc<Self> {
        static SERVICES: AtomicU64 = AtomicU64::new(0);
        let tasks = TaskTracker::new();
        let stopping = CancellationToken::new();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        let devices = Arc::new(Self {
            drivers,
            state: Mutex::default(),
            turns: tokio::sync::Mutex::new(()),
            numbers: OnceLock::new(),
            announcer: Announcer::start(&tasks, stopping.clone()),
            revision: watch::Sender::new(0),
            tasks,
            stopping,
            kept,
            owner: format!("{}-{nanos}-{}", std::process::id(), SERVICES.fetch_add(1, Ordering::Relaxed)),
            exists,
        });
        devices.tasks.spawn(devices.clone().sweep());
        devices.tasks.spawn(devices.clone().watch_takeover());
        devices
    }

    /// Lets go of a device another service took over soon after it did, so
    /// two services never hold one simulator for long: the service that lost
    /// it may be asked nothing more, as when a newer version took over.
    async fn watch_takeover(self: Arc<Self>) {
        loop {
            tokio::select! {
                () = self.stopping.cancelled() => return,
                () = tokio::time::sleep(TAKEOVER_POLL) => {}
            }
            self.let_go_taken().await;
        }
    }

    /// Releases what the record holds for the conversations that are no
    /// longer on this Host, which no release reached: the Host was offline
    /// when they ended, or no service ran. A simulator Demi booted for one
    /// shuts down, as its conversation's release shuts it down.
    async fn sweep(self: Arc<Self>) {
        for device in self.kept.forget_where_gone(|conversation| (self.exists)(conversation)) {
            tracing::info!(
                device = %device.handle,
                conversation = %device.conversation,
                "{} ({}) closes: its conversation is no longer on this Host",
                device.handle,
                device.native_id,
            );
            self.shut_down_if_demis(device.platform, &device.native_id).await;
        }
    }

    /// Shuts the simulator down when Demi booted it, in the boot it is in;
    /// answers whether it did, and why it could not.
    async fn shut_down_if_demis(&self, platform: MobilePlatform, native_id: &str) -> (bool, Option<String>) {
        let Probe::Booted(simulator) = self.probe(platform, native_id).await else {
            return (false, None);
        };
        if !self.kept.holds(platform, native_id, simulator.boot) {
            return (false, None);
        }
        match self.driver(platform).shutdown(native_id).await {
            Ok(()) => {
                self.kept.unbooted(platform, native_id);
                (true, None)
            }
            Err(error) => {
                tracing::warn!("{native_id} did not shut down: {error}");
                (false, Some(error.to_string()))
            }
        }
    }

    /// `conversation` reached this service: the devices the record holds for
    /// it become this service's, each listed as open at once and taken back
    /// under its handle by a task of its own. Only the first reach of a
    /// conversation does it, and no other conversation's entry is touched,
    /// so a service takes over no device of a conversation that is served
    /// elsewhere.
    pub fn reached(self: &Arc<Self>, conversation: &str) {
        for entry in self.claim(conversation) {
            self.tasks.spawn(self.clone().resume(entry, Duration::ZERO));
        }
    }

    /// The record's devices of `conversation` when it first reaches the
    /// service, as resuming ones.
    fn claim(&self, conversation: &str) -> Vec<Opened> {
        let mut state = self.state.lock().expect("the registry is intact");
        if state.closing || !state.served.insert(conversation.to_owned()) {
            return Vec::new();
        }
        let claimed = self.kept.claim(conversation, &self.owner);
        for entry in &claimed {
            state.resuming.insert(
                entry.native_id.clone(),
                Resuming {
                    entry: entry.clone(),
                    carried: Vec::new(),
                },
            );
        }
        claimed
    }

    /// What the Host says of the simulator `native_id` now.
    async fn probe(&self, platform: MobilePlatform, native_id: &str) -> Probe {
        let driver = self.driver(platform);
        if let Some(reason) = driver.unavailable() {
            return Probe::Unknown(reason);
        }
        match driver.list().await {
            Err(error) => Probe::Unknown(error.to_string()),
            Ok(listed) => match listed.into_iter().find(|simulator| simulator.native_id == native_id) {
                None => Probe::Gone("it no longer exists".into()),
                Some(simulator) => match simulator.state {
                    PowerState::Booted => Probe::Booted(simulator),
                    PowerState::Booting => Probe::Booting,
                    state @ (PowerState::ShuttingDown | PowerState::ShutDown) => {
                        Probe::Gone(format!("it is {}", power_word(state)))
                    }
                },
            },
        }
    }

    /// Takes `entry`'s device back, after `first`: waits for its simulator
    /// to finish booting, and tries again, with growing waits, while the
    /// Host cannot say whether the simulator is there or the device cannot
    /// be attached. Only a simulator that is not there, or is shut down,
    /// closes the device as lost, and a close, a release, another service's
    /// takeover or the service's end stops the task.
    ///
    /// Boxed, since taking a device back starts the task that takes it back
    /// again when its connection ends.
    fn resume(self: Arc<Self>, entry: Opened, first: Duration) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        Box::pin(async move { self.resuming(entry, first).await })
    }

    async fn resuming(self: Arc<Self>, entry: Opened, first: Duration) {
        if !self.pause(first).await {
            return;
        }
        let mut failures = 0u32;
        loop {
            if !self.pending(&entry) {
                return;
            }
            let wait = match self.probe(entry.platform, &entry.native_id).await {
                Probe::Gone(why) => {
                    self.lose(&entry, &why);
                    return;
                }
                Probe::Booting => BOOT_POLL,
                Probe::Unknown(why) => {
                    tracing::warn!("{} could not be found yet: {why}", entry.native_id);
                    failures += 1;
                    retry_after(failures)
                }
                Probe::Booted(simulator) => {
                    let driver = self.driver(entry.platform);
                    match driver.attach(&entry.conversation, &entry.native_id).await {
                        Ok(device) => {
                            self.install(&entry, simulator, device).await;
                            return;
                        }
                        Err(error) => {
                            tracing::warn!("{} could not be attached yet: {error}", entry.native_id);
                            failures += 1;
                            retry_after(failures)
                        }
                    }
                }
            };
            if !self.pause(wait).await {
                return;
            }
        }
    }

    /// Waits `wait`; answers false once the service ends.
    async fn pause(&self, wait: Duration) -> bool {
        tokio::select! {
            () = tokio::time::sleep(wait) => true,
            () = self.stopping.cancelled() => false,
        }
    }

    /// Whether `entry`'s device is still to be taken back by this service:
    /// it was not closed, released or taken over, and the service is not
    /// ending. A device another service took over is forgotten without a
    /// word to the record.
    fn pending(&self, entry: &Opened) -> bool {
        let mut state = self.state.lock().expect("the registry is intact");
        if state.closing || self.stopping.is_cancelled() {
            return false;
        }
        let ours = state
            .resuming
            .get(&entry.native_id)
            .is_some_and(|resuming| resuming.entry.handle == entry.handle);
        if !ours {
            return false;
        }
        if !self.kept.owns(&entry.native_id, &self.owner) {
            state.resuming.remove(&entry.native_id);
            drop(state);
            tracing::info!("{} ({}) was taken over by another service", entry.handle, entry.native_id);
            self.changed(&entry.conversation);
            return false;
        }
        true
    }

    /// Puts the attached `device` of `entry` among the open ones, unless
    /// the device is no longer to be taken back or closing began, when it is
    /// detached, which leaves the simulator and the record as they are.
    async fn install(self: &Arc<Self>, entry: &Opened, simulator: crate::driver::Simulator, device: Arc<dyn Device>) {
        let booted = self.kept.holds(entry.platform, &entry.native_id, simulator.boot);
        let installed = {
            let mut state = self.state.lock().expect("the registry is intact");
            let wanted = !state.closing
                && state
                    .resuming
                    .get(&entry.native_id)
                    .is_some_and(|resuming| resuming.entry.handle == entry.handle);
            if wanted {
                let resuming = state.resuming.remove(&entry.native_id).expect("it was found");
                let open = Arc::new(OpenDevice::new(
                    entry.handle.clone(),
                    &entry.conversation,
                    entry.platform,
                    simulator,
                    booted,
                    device.clone(),
                ));
                *open.saved.lock().expect("the saved files are intact") = resuming.carried;
                state.open.insert(entry.native_id.clone(), open.clone());
                Some(open)
            } else {
                None
            }
        };
        match installed {
            Some(open) => {
                self.tasks.spawn(self.clone().lifetime(open));
                self.changed(&entry.conversation);
            }
            None => {
                if let Err(error) = device.detach().await {
                    tracing::warn!("{} did not detach cleanly: {error}", entry.native_id);
                }
            }
        }
    }

    /// `entry`'s device is lost: its simulator is not there or is shut down.
    fn lose(&self, entry: &Opened, why: &str) {
        let removed = self
            .state
            .lock()
            .expect("the registry is intact")
            .resuming
            .remove(&entry.native_id)
            .is_some_and(|resuming| resuming.entry.handle == entry.handle);
        if !removed {
            return;
        }
        tracing::info!(
            device = %entry.handle,
            conversation = %entry.conversation,
            "{} ({}) closes: it was lost: {why}",
            entry.handle,
            entry.native_id,
        );
        self.kept.closed(&entry.native_id, &self.owner);
        self.changed(&entry.conversation);
    }

    /// Closes a device of this service that has no connection: removes it
    /// from the registry and the record, and shuts its simulator down when
    /// Demi booted it, as the close of a connected one does.
    async fn end_resuming(self: &Arc<Self>, entry: &Opened, why: Closing) -> Ended {
        let removed = {
            let mut state = self.state.lock().expect("the registry is intact");
            match state.resuming.get(&entry.native_id) {
                Some(resuming) if resuming.entry.handle == entry.handle => state.resuming.remove(&entry.native_id),
                _ => None,
            }
        };
        let Some(removed) = removed else {
            return Ended {
                shut_down: false,
                failure: None,
            };
        };
        let owned = self.kept.closed(&entry.native_id, &self.owner);
        for path in removed.carried {
            if let Err(error) = tokio::fs::remove_file(&path).await
                && error.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!("{} could not be removed: {error}", path.display());
            }
        }
        tracing::info!(
            device = %entry.handle,
            conversation = %entry.conversation,
            reason = ?why,
            "{} ({}) closes: {}",
            entry.handle,
            entry.native_id,
            why.words(),
        );
        let (shut_down, failure) = if owned {
            self.shut_down_if_demis(entry.platform, &entry.native_id).await
        } else {
            (false, None)
        };
        self.changed(&entry.conversation);
        Ended { shut_down, failure }
    }

    /// Waits until `settled` holds of the registry, or `cancel` or the
    /// service's end says stop, or [`RESUME_WAIT`] has passed.
    async fn wait_until<T>(
        &self,
        cancel: &CancellationToken,
        mut settled: impl FnMut(&State) -> Option<T>,
    ) -> Result<T, MobileError> {
        let mut seen = self.revision.subscribe();
        let deadline = Instant::now() + RESUME_WAIT;
        loop {
            if let Some(answer) = settled(&self.state.lock().expect("the registry is intact")) {
                return Ok(answer);
            }
            tokio::select! {
                changed = seen.changed() => if changed.is_err() {
                    return Err(MobileError::Cancelled);
                },
                () = cancel.cancelled() => return Err(MobileError::Cancelled),
                () = self.stopping.cancelled() => return Err(MobileError::Cancelled),
                () = tokio::time::sleep_until(deadline) => return Err(MobileError::Unavailable(
                    "the device is reconnecting; try again in a moment".into(),
                )),
            }
        }
    }

    fn driver(&self, platform: MobilePlatform) -> Arc<dyn Driver> {
        self.drivers
            .iter()
            .find(|driver| driver.platform() == platform)
            .expect("a recorded or open device's driver is registered")
            .clone()
    }

    pub fn attach_numbers(&self, numbers: Numbers) {
        // The SDK gives the source once per connection, before any call.
        let _first = self.numbers.set(numbers);
    }

    pub fn attach_messages(&self, messages: Messages) {
        self.announcer.attach(messages);
    }

    /// Follows the changes of the open devices.
    #[cfg(feature = "testing")]
    pub fn revisions(&self) -> watch::Receiver<u64> {
        self.revision.subscribe()
    }

    /// The conversations that hold devices, for the runner's `status`.
    pub async fn conversations(self: &Arc<Self>) -> Vec<String> {
        self.let_go_taken().await;
        let state = self.state.lock().expect("the registry is intact");
        let names: HashSet<&String> = state
            .open
            .values()
            .map(|open| &open.conversation)
            .chain(state.resuming.values().map(|resuming| &resuming.entry.conversation))
            .collect();
        let mut names: Vec<String> = names.into_iter().cloned().collect();
        names.sort();
        names
    }

    /// Lets go the open devices that another service took over, as a
    /// conversation that reached it asked: this service keeps nothing of
    /// them, and a service that holds nothing ends.
    async fn let_go_taken(self: &Arc<Self>) {
        let open: Vec<Arc<OpenDevice>> = self.state.lock().expect("the registry is intact").open.values().cloned().collect();
        for open in open {
            if !self.kept.owns(&open.native_id, &self.owner) {
                self.end(&open, Closing::Taken).await;
            }
        }
    }

    /// `conversation`'s connected devices on this Host, by handle.
    pub fn of(&self, conversation: &str) -> Vec<Arc<OpenDevice>> {
        let state = self.state.lock().expect("the registry is intact");
        let mut devices: Vec<Arc<OpenDevice>> = state
            .open
            .values()
            .filter(|open| open.conversation == conversation)
            .cloned()
            .collect();
        devices.sort_by_key(|open| open.handle.number());
        devices
    }

    /// `conversation`'s device `handle`. One this service is taking back
    /// is waited for, for [`RESUME_WAIT`] at most and not past `cancel`;
    /// the take-back goes on whatever becomes of the wait.
    pub async fn get(
        self: &Arc<Self>,
        conversation: &str,
        handle: &DeviceId,
        cancel: &CancellationToken,
    ) -> Result<Arc<OpenDevice>, MobileError> {
        loop {
            let found = self
                .wait_until(cancel, |state| {
                    let open = state
                        .open
                        .values()
                        .find(|open| open.conversation == conversation && &open.handle == handle);
                    match open {
                        Some(open) => Some(Some(open.clone())),
                        None => state
                            .resuming
                            .values()
                            .all(|resuming| !(resuming.entry.conversation == conversation && &resuming.entry.handle == handle))
                            .then_some(None),
                    }
                })
                .await?;
            let Some(open) = found else {
                return Err(MobileError::NotOpen(handle.clone()));
            };
            if self.kept.owns(&open.native_id, &self.owner) {
                return Ok(open);
            }
            self.end(&open, Closing::Taken).await;
        }
    }

    /// Every simulator of the Host, as `conversation` sees them.
    pub async fn list(self: &Arc<Self>, conversation: &str) -> Result<ListResult, MobileError> {
        self.let_go_taken().await;
        let mut devices = Vec::new();
        let mut unavailable = Vec::new();
        for driver in &self.drivers {
            if let Some(reason) = driver.unavailable() {
                unavailable.push(Unavailable {
                    platform: driver.platform(),
                    reason,
                });
                continue;
            }
            let listed = driver.list().await.map_err(MobileError::Driver)?;
            let state = self.state.lock().expect("the registry is intact");
            devices.extend(listed.into_iter().map(|simulator| {
                // A device being taken back is open as it was.
                let open = state
                    .open
                    .get(&simulator.native_id)
                    .map(|open| (&open.conversation, &open.handle))
                    .or_else(|| {
                        state
                            .resuming
                            .get(&simulator.native_id)
                            .map(|resuming| (&resuming.entry.conversation, &resuming.entry.handle))
                    });
                Simulator {
                    name: simulator.name,
                    platform: driver.platform(),
                    model: simulator.model,
                    os: simulator.os,
                    state: simulator.state,
                    native_id: simulator.native_id,
                    open: open
                        .filter(|(holder, _)| *holder == conversation)
                        .map(|(_, handle)| handle.clone()),
                    elsewhere: open.is_some_and(|(holder, _)| holder != conversation),
                }
            }));
        }
        Ok(ListResult {
            devices,
            unavailable,
        })
    }

    /// Opens the simulator `query` names, a name or a native id, for
    /// `conversation`, booting it when it is shut down. One the
    /// conversation has open already answers as it is.
    ///
    /// An open runs to its end on its own, whatever becomes of the request
    /// that asked for it: a simulator it booted is then either open, its
    /// lifetime the conversation's, or shut down again.
    pub async fn open(
        self: &Arc<Self>,
        conversation: &str,
        query: &str,
    ) -> Result<Arc<OpenDevice>, MobileError> {
        let devices = self.clone();
        let conversation = conversation.to_owned();
        let query = query.to_owned();
        self.tasks
            .spawn(async move { devices.opening(&conversation, &query).await })
            .await
            .map_err(|error| MobileError::Failed(format!("the open did not finish: {error}")))?
    }

    async fn opening(self: &Arc<Self>, conversation: &str, query: &str) -> Result<Arc<OpenDevice>, MobileError> {
        let _turn = self.turns.lock().await;
        let (driver, simulator) = self.find(query).await?;
        {
            let state = self.state.lock().expect("the registry is intact");
            if let Some(open) = state.open.get(&simulator.native_id) {
                if open.conversation == conversation {
                    return Ok(open.clone());
                }
                return Err(MobileError::Busy(simulator.name));
            }
            if let Some(resuming) = state.resuming.get(&simulator.native_id) {
                if resuming.entry.conversation == conversation {
                    return Err(MobileError::Unavailable(format!(
                        "{} is reconnecting; try again in a moment",
                        resuming.entry.handle
                    )));
                }
                return Err(MobileError::Busy(simulator.name));
            }
        }
        let count = {
            let state = self.state.lock().expect("the registry is intact");
            state.open.values().filter(|open| open.conversation == conversation).count()
                + state.resuming.values().filter(|resuming| resuming.entry.conversation == conversation).count()
        };
        if count >= MAX_DEVICES {
            return Err(MobileError::TooMany);
        }
        // Demi booted it now, or the program before this one did.
        let booting = simulator.state != PowerState::Booted;
        let booted = booting || self.kept.holds(driver.platform(), &simulator.native_id, simulator.boot);
        if booting {
            let recorded = Recorded {
                kept: &self.kept,
                platform: driver.platform(),
                native_id: &simulator.native_id,
            };
            // A boot that began and was not undone leaves the simulator
            // Demi's, to shut down with the device that opens it next.
            driver
                .boot(&simulator.native_id, &recorded)
                .await
                .map_err(MobileError::Driver)?;
        }
        let attached = driver.attach(conversation, &simulator.native_id).await;
        let device = match attached {
            Ok(device) => device,
            Err(error) => {
                if booted {
                    // The device it booted for nothing goes down again; one
                    // that does not stays Demi's, since the open's own
                    // failure is what the caller needs.
                    match driver.shutdown(&simulator.native_id).await {
                        Ok(()) => self.kept.unbooted(driver.platform(), &simulator.native_id),
                        Err(failure) => {
                            tracing::warn!("{} stayed booted after a failed open: {failure}", simulator.name);
                        }
                    }
                }
                return Err(MobileError::Driver(error));
            }
        };
        let numbers = self
            .numbers
            .get()
            .ok_or_else(|| MobileError::Failed("the numbers stream is not open".into()))?;
        let number = match numbers.draw(conversation, ServiceSequence::Device, 1).await {
            Ok(number) => number,
            Err(error) => {
                let failure = self.let_go(&driver, &device, &simulator.native_id, booted, &simulator.name).await;
                if booted && failure.is_none() {
                    self.kept.unbooted(driver.platform(), &simulator.native_id);
                }
                return Err(MobileError::Failed(error.to_string()));
            }
        };
        let open = Arc::new(OpenDevice::new(
            DeviceId::numbered(number),
            conversation,
            driver.platform(),
            simulator,
            booted,
            device,
        ));
        let added = {
            let mut state = self.state.lock().expect("the registry is intact");
            if state.closing {
                false
            } else {
                self.kept.opened(Opened {
                    handle: open.handle.clone(),
                    conversation: conversation.to_owned(),
                    platform: open.platform,
                    native_id: open.native_id.clone(),
                    owner: self.owner.clone(),
                });
                state.open.insert(open.native_id.clone(), open.clone());
                true
            }
        };
        if !added {
            // The service is closing: nothing is added now; the device is
            // let go as it is, for the next service to take back.
            if let Err(error) = open.device.detach().await {
                tracing::warn!("{} did not detach cleanly: {error}", open.name);
            }
            return Err(MobileError::Cancelled);
        }
        self.changed(conversation);
        self.tasks.spawn(self.clone().lifetime(open.clone()));
        Ok(open)
    }

    /// Closes `conversation`'s device `handle`; answers whether it shut down.
    /// A device being taken back closes at once, without waiting for it.
    pub async fn close(self: &Arc<Self>, conversation: &str, handle: &DeviceId) -> Result<Ended, MobileError> {
        let resuming = {
            let state = self.state.lock().expect("the registry is intact");
            state
                .resuming
                .values()
                .find(|resuming| resuming.entry.conversation == conversation && &resuming.entry.handle == handle)
                .map(|resuming| resuming.entry.clone())
        };
        if let Some(entry) = resuming {
            return Ok(self.end_resuming(&entry, Closing::Asked).await);
        }
        let open = self
            .of(conversation)
            .into_iter()
            .find(|open| &open.handle == handle)
            .ok_or_else(|| MobileError::NotOpen(handle.clone()))?;
        Ok(self.end(&open, Closing::Asked).await)
    }

    /// Closes every device of `conversation`: its release. The conversation
    /// need not have reached the service before: what the record holds for
    /// it is released too.
    pub async fn release(self: &Arc<Self>, conversation: &str) {
        // Not resumed: the devices it claims are only to be closed.
        self.claim(conversation);
        for open in self.of(conversation) {
            self.end(&open, Closing::Released).await;
        }
        let resuming: Vec<Opened> = self
            .state
            .lock()
            .expect("the registry is intact")
            .resuming
            .values()
            .filter(|resuming| resuming.entry.conversation == conversation)
            .map(|resuming| resuming.entry.clone())
            .collect();
        for entry in resuming {
            self.end_resuming(&entry, Closing::Released).await;
        }
    }

    /// Lets every device go as the service ends, shutting none down and
    /// forgetting none: the next program takes them back. Stopping and
    /// taking back share this boundary: once it began no device is added
    /// to the registry, and one a take-back attached meanwhile is detached.
    pub async fn close_all(self: &Arc<Self>) {
        let open: Vec<Arc<OpenDevice>> = {
            let mut state = self.state.lock().expect("the registry is intact");
            state.closing = true;
            state.open.values().cloned().collect()
        };
        futures_util::future::join_all(open.iter().map(|open| self.end(open, Closing::Ending)))
            .await;
        self.stopping.cancel();
        self.tasks.close();
        self.tasks.wait().await;
    }

    /// The simulator `query` names, and its driver: a native id, else a
    /// name, the booted one first, else the newest system's.
    async fn find(&self, query: &str) -> Result<(Arc<dyn Driver>, crate::driver::Simulator), MobileError> {
        let mut named = Vec::new();
        let mut unavailable = Vec::new();
        for driver in &self.drivers {
            if let Some(reason) = driver.unavailable() {
                unavailable.push(reason);
                continue;
            }
            for simulator in driver.list().await.map_err(MobileError::Driver)? {
                if simulator.native_id == query {
                    return Ok((driver.clone(), simulator));
                }
                if simulator.name == query {
                    named.push((driver.clone(), simulator));
                }
            }
        }
        match named.len() {
            0 => Err(MobileError::NoSimulator {
                query: query.to_owned(),
                unavailable,
            }),
            1 => Ok(named.remove(0)),
            _ => Err(MobileError::Ambiguous {
                name: query.to_owned(),
                candidates: named
                    .into_iter()
                    .map(|(_, simulator)| (simulator.native_id, simulator.os, power_word(simulator.state)))
                    .collect(),
            }),
        }
    }

    /// Closes `open` once, for `why`: detaches it, shuts it down when Demi
    /// booted it and it is still there, and tells the plugin. The closing
    /// runs to its end on its own, whatever becomes of the request that

    /// Closes `open` once, for `why`: detaches it, shuts it down when Demi
    /// booted it and it is still there, and tells the plugin. The closing
    /// runs to its end on its own, whatever becomes of the request that
    /// asked for it.
    async fn end(self: &Arc<Self>, open: &Arc<OpenDevice>, why: Closing) -> Ended {
        let devices = self.clone();
        let open = open.clone();
        match self.tasks.spawn(async move { devices.ending(&open, why).await }).await {
            Ok(ended) => ended,
            Err(error) => Ended {
                shut_down: false,
                failure: Some(format!("the close did not finish: {error}")),
            },
        }
    }

    async fn ending(self: &Arc<Self>, open: &Arc<OpenDevice>, why: Closing) -> Ended {
        let _turn = self.turns.lock().await;
        let removed = {
            let mut state = self.state.lock().expect("the registry is intact");
            match state.open.get(&open.native_id) {
                Some(current) if Arc::ptr_eq(current, open) => state.open.remove(&open.native_id),
                _ => None,
            }
        };
        if removed.is_none() {
            return Ended {
                shut_down: false,
                failure: None,
            };
        }
        // The device is gone for its views, unless it only leaves this
        // service to another, or the service ends, which they reconnect to.
        if !matches!(why, Closing::Taken | Closing::Ending) {
            open.gone.cancel();
        }
        open.closed.cancel();
        let mut saved = std::mem::take(&mut *open.saved.lock().expect("the saved files are intact"));
        // A recording nobody stopped goes with the device.
        if let Some((file, _)) = open.recording.lock().expect("the recording is intact").take() {
            saved.push(file);
        }
        if !matches!(why, Closing::Taken | Closing::Ending) {
            for path in saved {
                // A file the user already moved or removed is gone as asked.
                if let Err(error) = tokio::fs::remove_file(&path).await
                    && error.kind() != std::io::ErrorKind::NotFound
                {
                    tracing::warn!("{} could not be removed: {error}", path.display());
                }
            }
        }
        let driver = self.driver(open.platform);
        // What another service took over is not this one's to change.
        let owned = match why {
            Closing::Ending | Closing::Taken => false,
            _ => self.kept.closed(&open.native_id, &self.owner),
        };
        let shut_down = owned && open.booted && !matches!(why, Closing::Lost);
        // Every close says why, so a tab that went can be told apart from
        // another afterwards.
        tracing::info!(
            device = %open.handle,
            name = %open.name,
            conversation = %open.conversation,
            reason = ?why,
            shut_down,
            "{} ({}) closes: {}",
            open.handle,
            open.name,
            why.words(),
        );
        // A device that went away is let go as any other, so what attaching
        // started inside it ends; it is never shut down for going away.
        let failure = self.let_go(&driver, &open.device, &open.native_id, shut_down, &open.name).await;
        // A device that went away may still be booted, as when its helper
        // ended: it stays Demi's until it is shut down.
        if shut_down && failure.is_none() {
            self.kept.unbooted(open.platform, &open.native_id);
        }
        self.changed(&open.conversation);
        Ended {
            shut_down: shut_down && failure.is_none(),
            failure,
        }
    }

    /// Detaches a device, and shuts it down when `shut_down`; answers why
    /// the shutdown failed. A failed detach is the Host log's: the device
    /// leaves the registry either way.
    async fn let_go(
        &self,
        driver: &Arc<dyn Driver>,
        device: &Arc<dyn Device>,
        native_id: &str,
        shut_down: bool,
        name: &str,
    ) -> Option<String> {
        if let Err(error) = device.detach().await {
            tracing::warn!("{name} did not detach cleanly: {error}");
        }
        if !shut_down {
            return None;
        }
        match driver.shutdown(native_id).await {
            Ok(()) => None,
            Err(error) => {
                tracing::warn!("{name} did not shut down: {error}");
                Some(error.to_string())
            }
        }
    }


    /// Ends `open` when its simulator goes away, and keeps it when only its
    /// connection ends: an unused device stays open, and so does one whose
    /// helper or picture stream ended while the simulator runs.
    async fn lifetime(self: Arc<Self>, open: Arc<OpenDevice>) {
        let lost = open.device.ended();
        tokio::select! {
            biased;
            () = open.closed.cancelled() => {}
            () = self.stopping.cancelled() => {}
            () = lost.cancelled() => self.connection_ended(&open).await,
        }
    }

    /// `open`'s connection ended: the device is lost when its simulator is
    /// gone, else it is taken back under its handle, as the first take-back
    /// does, after a moment.
    async fn connection_ended(self: &Arc<Self>, open: &Arc<OpenDevice>) {
        if let Probe::Gone(_) = self.probe(open.platform, &open.native_id).await {
            self.ending(open, Closing::Lost).await;
            return;
        }
        let turn = self.turns.lock().await;
        let entry = Opened {
            handle: open.handle.clone(),
            conversation: open.conversation.clone(),
            platform: open.platform,
            native_id: open.native_id.clone(),
            owner: self.owner.clone(),
        };
        let removed = {
            let mut state = self.state.lock().expect("the registry is intact");
            match state.open.get(&open.native_id) {
                Some(current) if Arc::ptr_eq(current, open) && !state.closing => {
                    state.open.remove(&open.native_id);
                    let mut carried = std::mem::take(&mut *open.saved.lock().expect("the saved files are intact"));
                    if let Some((file, _)) = open.recording.lock().expect("the recording is intact").take() {
                        carried.push(file);
                    }
                    state.resuming.insert(
                        open.native_id.clone(),
                        Resuming {
                            entry: entry.clone(),
                            carried,
                        },
                    );
                    true
                }
                _ => false,
            }
        };
        if !removed {
            return;
        }
        tracing::info!(
            device = %open.handle,
            name = %open.name,
            "{} ({}) loses its connection and reconnects",
            open.handle,
            open.name,
        );
        open.closed.cancel();
        drop(turn);
        if let Err(error) = open.device.detach().await {
            tracing::warn!("{} did not detach cleanly: {error}", open.name);
        }
        self.changed(&open.conversation);
        self.tasks.spawn(self.clone().resume(entry, RECONNECT_DELAY));
    }

    /// Stops every task and lets nothing go, as a program that is killed:
    /// for a test of what the next program does.
    #[cfg(feature = "testing")]
    pub fn killed(&self) {
        self.stopping.cancel();
    }

    fn changed(&self, conversation: &str) {
        self.revision.send_modify(|revision| *revision += 1);
        self.announcer.announce(conversation);
    }
}

/// How long a take-back waits before it tries a failed step again: a second,
/// growing with each failure to half a minute.
fn retry_after(failures: u32) -> Duration {
    Duration::from_secs(1u64 << failures.saturating_sub(1).min(5)).min(RETRY_LAST)
}

/// A simulator's state in the words `list` uses.
fn power_word(state: PowerState) -> String {
    match state {
        PowerState::Booted => "booted",
        PowerState::Booting => "booting",
        PowerState::ShuttingDown => "shutting down",
        PowerState::ShutDown => "shut down",
    }
    .to_owned()
}

/// How a close ended: whether the device shut down, and why a shutdown
/// that was due did not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ended {
    pub shut_down: bool,
    pub failure: Option<String>,
}


/// Tells the plugin, once per burst, which conversations' devices changed
/// (`crates/command-protocol/src/messages.rs`).
struct Announcer {
    pending: Arc<Mutex<BTreeMap<String, ()>>>,
    wake: Arc<Notify>,
    messages: Arc<OnceLock<Messages>>,
    attached: Arc<Notify>,
}

impl Announcer {
    fn start(tasks: &TaskTracker, stopping: CancellationToken) -> Self {
        let announcer = Self {
            pending: Arc::default(),
            wake: Arc::default(),
            messages: Arc::default(),
            attached: Arc::default(),
        };
        let pending = announcer.pending.clone();
        let wake = announcer.wake.clone();
        let messages = announcer.messages.clone();
        let attached = announcer.attached.clone();
        let message = serde_json::to_value(MobileMessage {
            devices: DevicesChange::Changed,
        })
        .expect("the message serializes");
        tasks.spawn(async move {
            stopping.run_until_cancelled(async move {
            // A service whose runner never opens the messages stream, as in
            // a test without a plugin, has nobody to tell.
            let messages = loop {
                if let Some(messages) = messages.get() {
                    break messages.clone();
                }
                attached.notified().await;
            };
            loop {
                let next = pending
                    .lock()
                    .expect("the pending set is intact")
                    .pop_first();
                let Some((conversation, ())) = next else {
                    wake.notified().await;
                    continue;
                };
                if let Err(error) = messages.send(&conversation, message.clone()).await {
                    // The stream ends with the service; nobody is left to tell.
                    tracing::info!("the plugin was not told of {conversation}'s devices: {error}");
                    return;
                }
            }
            })
            .await;
        });
        announcer
    }

    fn attach(&self, messages: Messages) {
        let _first = self.messages.set(messages);
        self.attached.notify_one();
    }

    fn announce(&self, conversation: &str) {
        self.pending
            .lock()
            .expect("the pending set is intact")
            .insert(conversation.to_owned(), ());
        self.wake.notify_one();
    }
}

impl From<DriverError> for MobileError {
    fn from(error: DriverError) -> Self {
        Self::Driver(error)
    }
}

/// A boot `open` started, recorded in the simulators Demi booted as it
/// begins and taken out again once the driver undid a failed one.
struct Recorded<'a> {
    kept: &'a Kept,
    platform: MobilePlatform,
    native_id: &'a str,
}

impl crate::driver::Booting for Recorded<'_> {
    fn began(&self, boot: u64) {
        self.kept.booted(Entry {
            platform: self.platform,
            native_id: self.native_id.to_owned(),
            boot,
        });
    }

    fn undone(&self) {
        self.kept.unbooted(self.platform, self.native_id);
    }
}
