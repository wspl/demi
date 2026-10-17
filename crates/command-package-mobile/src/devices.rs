//! The Host's open devices (`mobile.md` § Commands, § Lifetime): which
//! conversation has each simulator open and under which handle, at most
//! [`MAX_DEVICES`] a conversation, never one device in two conversations;
//! and when each ends. A device Demi booted shuts down when its
//! conversation closes it or is released, or an hour after nobody watched
//! it and no command used it; one the user booted is only let go. The
//! program's end shuts nothing down: the simulators Demi booted are kept
//! ([`Booted`]), and the next program gives each still booted the hour.
//!
//! One owner changes the registry: opening and closing take turns on one
//! lock, so a device is never opened twice nor closed while it opens. The
//! plugin hears of each change of a conversation's devices, when it
//! happens.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{Arc, Mutex, OnceLock},
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
    booted::{Booted, Entry},
    driver::{Device, Driver, DriverError},
    error::MobileError,
    refs::Refs,
};

/// How long a device Demi booted stays open while nobody watches it and no
/// command uses it.
pub const IDLE: Duration = Duration::from_secs(60 * 60);
/// How long the view still says the agent uses a device after its last
/// command on it ended (`mobile.md` § The live view).
pub const AGENT_LINGER: Duration = Duration::from_secs(3);

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
    /// The commands on the device take turns.
    pub turn: tokio::sync::Mutex<()>,
    /// The files the panel's Screenshot and Record wrote into the
    /// conversation's scratch directory, removed when the device closes
    /// (`mobile.md` § The live view).
    pub saved: Mutex<Vec<std::path::PathBuf>>,
    used: watch::Sender<Instant>,
    viewers: watch::Sender<usize>,
    /// Cancelled once the device is closed, whatever closed it.
    pub closed: CancellationToken,
}

impl OpenDevice {
    /// A command used it now.
    pub fn used(&self) {
        self.used.send_replace(Instant::now());
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

    /// A viewer watches it until the guard drops.
    pub fn watched(self: &Arc<Self>) -> Watching {
        self.viewers.send_modify(|count| *count += 1);
        Watching(self.clone())
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

/// A viewer watching a device.
pub struct Watching(Arc<OpenDevice>);

impl Drop for Watching {
    fn drop(&mut self) {
        self.0.viewers.send_modify(|count| *count -= 1);
        // Leaving is the device's last use: the hour starts now.
        self.0.used();
    }
}

/// Why a device closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Closing {
    /// `close`, or its panel tab closed.
    Asked,
    /// The conversation was released.
    Released,
    /// Nobody watched or used it for [`IDLE`].
    Idle,
    /// The device itself went away.
    Lost,
    /// The program ends: the device is let go and stays as it is.
    Ending,
}

#[derive(Default)]
struct State {
    /// By the simulator's native id.
    open: HashMap<String, Arc<OpenDevice>>,
}

/// The Host's open devices.
pub struct Devices {
    drivers: Vec<Arc<dyn Driver>>,
    state: Mutex<State>,
    /// Opening and closing take turns.
    turns: tokio::sync::Mutex<()>,
    numbers: OnceLock<Numbers>,
    announcer: Announcer,
    /// Counts the changes of the open devices, which the live views follow.
    revision: watch::Sender<u64>,
    tasks: TaskTracker,
    /// Cancelled once the service closes: its tasks end.
    stopping: CancellationToken,
    /// The simulators Demi booted, across the program's ends.
    booted: Booted,
}

impl Devices {
    /// The registry over `drivers`. Each simulator `booted` holds that is
    /// still booted from Demi's boot gets the hour from now, unless a
    /// conversation opens it first; the others leave the record.
    pub fn new(drivers: Vec<Arc<dyn Driver>>, booted: Booted) -> Arc<Self> {
        let tasks = TaskTracker::new();
        let stopping = CancellationToken::new();
        let devices = Arc::new(Self {
            drivers,
            state: Mutex::default(),
            turns: tokio::sync::Mutex::new(()),
            numbers: OnceLock::new(),
            announcer: Announcer::start(&tasks, stopping.clone()),
            revision: watch::Sender::new(0),
            tasks,
            stopping,
            booted,
        });
        let left = devices.booted.entries();
        if !left.is_empty() {
            let stopping = devices.stopping.clone();
            devices.tasks.spawn(stopping.run_until_cancelled_owned(devices.clone().adopt(left)));
        }
        devices
    }

    /// What the last program booted and left: each simulator still in that
    /// boot shuts down after [`IDLE`] unless a conversation has it open
    /// then; one shut down or booted again since is no longer Demi's.
    async fn adopt(self: Arc<Self>, left: Vec<Entry>) {
        let since = Instant::now();
        let mut kept = Vec::new();
        for entry in left {
            match self.current_boot(&entry).await {
                Some(boot) if boot == entry.boot => kept.push(entry),
                _ => self.booted.remove(entry.platform, &entry.native_id),
            }
        }
        if kept.is_empty() {
            return;
        }
        tokio::time::sleep_until(since + IDLE).await;
        let _turn = self.turns.lock().await;
        for entry in kept {
            let open = self
                .state
                .lock()
                .expect("the registry is intact")
                .open
                .contains_key(&entry.native_id);
            // An open device's lifetime is its conversation's; one that
            // left the record meanwhile was shut down or is the user's.
            if open || !self.booted.holds(entry.platform, &entry.native_id, Some(entry.boot)) {
                continue;
            }
            if self.current_boot(&entry).await != Some(entry.boot) {
                self.booted.remove(entry.platform, &entry.native_id);
                continue;
            }
            tracing::info!("{} shuts down after an hour unused since Demi restarted", entry.native_id);
            let driver = self.driver(entry.platform);
            if let Err(error) = driver.shutdown(&entry.native_id).await {
                tracing::warn!("{} did not shut down: {error}", entry.native_id);
            }
            self.booted.remove(entry.platform, &entry.native_id);
        }
    }

    /// The mark of `entry`'s simulator's current boot; none while it is
    /// shut down or when its driver cannot list it.
    async fn current_boot(&self, entry: &Entry) -> Option<u64> {
        let driver = self.driver(entry.platform);
        match driver.list().await {
            Ok(listed) => listed
                .into_iter()
                .find(|simulator| simulator.native_id == entry.native_id)
                // One still booting, or hung there, is in its boot as well.
                .filter(|simulator| matches!(simulator.state, PowerState::Booting | PowerState::Booted))
                .and_then(|simulator| simulator.boot),
            Err(error) => {
                tracing::warn!("{} could not be listed: {error}", entry.native_id);
                None
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
    pub fn revisions(&self) -> watch::Receiver<u64> {
        self.revision.subscribe()
    }

    /// The conversations that hold devices, for the runner's `status`.
    pub fn conversations(&self) -> Vec<String> {
        let state = self.state.lock().expect("the registry is intact");
        let names: HashSet<&String> = state.open.values().map(|open| &open.conversation).collect();
        let mut names: Vec<String> = names.into_iter().cloned().collect();
        names.sort();
        names
    }

    /// `conversation`'s devices on this Host, by handle.
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

    /// `conversation`'s device `handle`.
    pub fn get(&self, conversation: &str, handle: &DeviceId) -> Result<Arc<OpenDevice>, MobileError> {
        self.of(conversation)
            .into_iter()
            .find(|open| &open.handle == handle)
            .ok_or_else(|| MobileError::NotOpen(handle.clone()))
    }

    /// Every simulator of the Host, as `conversation` sees them.
    pub async fn list(&self, conversation: &str) -> Result<ListResult, MobileError> {
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
                let open = state.open.get(&simulator.native_id);
                Simulator {
                    name: simulator.name,
                    platform: driver.platform(),
                    model: simulator.model,
                    os: simulator.os,
                    state: simulator.state,
                    native_id: simulator.native_id,
                    open: open
                        .filter(|open| open.conversation == conversation)
                        .map(|open| open.handle.clone()),
                    elsewhere: open.is_some_and(|open| open.conversation != conversation),
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
    /// lifetime the conversation's, or shut down again (`mobile.md`
    /// § Lifetime).
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
        let known = self
            .state
            .lock()
            .expect("the registry is intact")
            .open
            .get(&simulator.native_id)
            .cloned();
        if let Some(open) = known {
            if open.conversation == conversation {
                open.used();
                return Ok(open);
            }
            return Err(MobileError::Busy(simulator.name));
        }
        if self.of(conversation).len() >= MAX_DEVICES {
            return Err(MobileError::TooMany);
        }
        // Demi booted it now, or the program before this one did.
        let booting = simulator.state != PowerState::Booted;
        let booted = booting || self.booted.holds(driver.platform(), &simulator.native_id, simulator.boot);
        if booting {
            let recorded = Recorded {
                booted: &self.booted,
                platform: driver.platform(),
                native_id: &simulator.native_id,
            };
            if let Err(error) = driver.boot(&simulator.native_id, &recorded).await {
                // A boot that began and was not undone leaves the simulator
                // Demi's, for the hour from now.
                self.later(driver.platform(), &simulator.native_id);
                return Err(MobileError::Driver(error));
            }
        }
        let attached = driver.attach(conversation, &simulator.native_id).await;
        let device = match attached {
            Ok(device) => device,
            Err(error) => {
                if booted {
                    // The device it booted for nothing goes down again; one
                    // that does not stays Demi's, for later, since the open's
                    // own failure is what the caller needs.
                    match driver.shutdown(&simulator.native_id).await {
                        Ok(()) => self.booted.remove(driver.platform(), &simulator.native_id),
                        Err(failure) => {
                            tracing::warn!("{} stayed booted after a failed open: {failure}", simulator.name);
                            self.later(driver.platform(), &simulator.native_id);
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
                self.settle(driver.platform(), &simulator.native_id, booted, failure.is_none());
                return Err(MobileError::Failed(error.to_string()));
            }
        };
        let open = Arc::new(OpenDevice {
            handle: DeviceId::numbered(number),
            conversation: conversation.to_owned(),
            native_id: simulator.native_id.clone(),
            platform: driver.platform(),
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
            saved: Mutex::default(),
            turn: tokio::sync::Mutex::new(()),
            used: watch::Sender::new(Instant::now()),
            viewers: watch::Sender::new(0),
            closed: CancellationToken::new(),
        });
        self.state
            .lock()
            .expect("the registry is intact")
            .open
            .insert(open.native_id.clone(), open.clone());
        self.changed(conversation);
        self.tasks.spawn(self.clone().lifetime(driver, open.clone()));
        Ok(open)
    }

    /// Closes `conversation`'s device `handle`; answers whether it shut down.
    pub async fn close(self: &Arc<Self>, conversation: &str, handle: &DeviceId) -> Result<Ended, MobileError> {
        let open = self.get(conversation, handle)?;
        Ok(self.end(&open, Closing::Asked).await)
    }

    /// Closes every device of `conversation`: its release.
    pub async fn release(self: &Arc<Self>, conversation: &str) {
        for open in self.of(conversation) {
            self.end(&open, Closing::Released).await;
        }
    }

    /// Lets every device go as the service ends, shutting none down: the
    /// next program gives the ones Demi booted the hour.
    pub async fn close_all(self: &Arc<Self>) {
        let open: Vec<Arc<OpenDevice>> = self
            .state
            .lock()
            .expect("the registry is intact")
            .open
            .values()
            .cloned()
            .collect();
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
        open.closed.cancel();
        let mut saved = std::mem::take(&mut *open.saved.lock().expect("the saved files are intact"));
        // A recording nobody stopped goes with the device.
        if let Some((file, _)) = open.recording.lock().expect("the recording is intact").take() {
            saved.push(file);
        }
        for path in saved {
            // A file the user already moved or removed is gone as asked.
            if let Err(error) = tokio::fs::remove_file(&path).await
                && error.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!("{} could not be removed: {error}", path.display());
            }
        }
        let driver = self.driver(open.platform);
        let shut_down = open.booted && !matches!(why, Closing::Lost | Closing::Ending);
        if why == Closing::Lost {
            tracing::info!("{} ({}) went away", open.handle, open.name);
        }
        // A device that went away is let go as any other, so what attaching
        // started inside it ends; it is never shut down for going away.
        let failure = self.let_go(&driver, &open.device, &open.native_id, shut_down, &open.name).await;
        if why == Closing::Lost {
            // A device that went away may still be booted, as when its
            // helper ended: it stays Demi's until it is shut down.
            if open.booted {
                self.later(open.platform, &open.native_id);
            }
        } else {
            self.settle(open.platform, &open.native_id, shut_down, failure.is_none());
        }
        self.changed(&open.conversation);
        Ended {
            shut_down: shut_down && failure.is_none(),
            failure,
        }
    }

    /// After a device Demi booted was let go: a shutdown that was asked for
    /// and happened ends Demi's ownership; one that failed keeps it, for
    /// the hour from now.
    fn settle(self: &Arc<Self>, platform: MobilePlatform, native_id: &str, shut_down: bool, done: bool) {
        if !shut_down {
            return;
        }
        if done {
            self.booted.remove(platform, native_id);
        } else {
            self.later(platform, native_id);
        }
    }

    /// The simulator stays Demi's: shut down after [`IDLE`] unless a
    /// conversation opens it first, as one the last program left is.
    fn later(self: &Arc<Self>, platform: MobilePlatform, native_id: &str) {
        let Some(entry) = self
            .booted
            .entries()
            .into_iter()
            .find(|entry| entry.platform == platform && entry.native_id == native_id)
        else {
            return;
        };
        let stopping = self.stopping.clone();
        self.tasks
            .spawn(stopping.run_until_cancelled_owned(self.clone().adopt(vec![entry])));
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

    /// Ends `open` when it goes away, or, for a device Demi booted, once
    /// nobody watched it and no command used it for [`IDLE`].
    async fn lifetime(self: Arc<Self>, _driver: Arc<dyn Driver>, open: Arc<OpenDevice>) {
        let mut used = open.used.subscribe();
        let mut viewers = open.viewers.subscribe();
        let lost = open.device.ended();
        loop {
            let watched = *viewers.borrow_and_update() > 0;
            let idle_until = *used.borrow_and_update() + IDLE;
            tokio::select! {
                _ = open.closed.cancelled() => return,
                _ = self.stopping.cancelled() => return,
                _ = lost.cancelled() => {
                    self.ending(&open, Closing::Lost).await;
                    return;
                }
                _ = tokio::time::sleep_until(idle_until), if open.booted && !watched => {
                    tracing::info!("{} ({}) shuts down after an hour unused", open.handle, open.name);
                    self.ending(&open, Closing::Idle).await;
                    return;
                }
                changed = used.changed() => if changed.is_err() { return },
                changed = viewers.changed() => if changed.is_err() { return },
            }
        }
    }

    /// Stops every task and lets nothing go, as a program that is killed:
    /// for a test of what the next program does.
    #[cfg(feature = "testing")]
    pub fn killed(&self) {
        self.stopping.cancel();
    }

    /// A recording started or stopped: the views say so on every page,
    /// whoever started it.
    pub fn recorded(&self) {
        self.revision.send_modify(|revision| *revision += 1);
    }

    fn changed(&self, conversation: &str) {
        self.revision.send_modify(|revision| *revision += 1);
        self.announcer.announce(conversation);
    }
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
/// (`native-runtime.md` § Messages to the plugin).
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
    booted: &'a Booted,
    platform: MobilePlatform,
    native_id: &'a str,
}

impl crate::driver::Booting for Recorded<'_> {
    fn began(&self, boot: u64) {
        self.booted.add(Entry {
            platform: self.platform,
            native_id: self.native_id.to_owned(),
            boot,
        });
    }

    fn undone(&self) {
        self.booted.remove(self.platform, self.native_id);
    }
}
