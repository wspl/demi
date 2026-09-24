//! The user's Cloud machine in the user's shard (`managed-hosts.md`
//! § Lifecycle and capacity): one phase, which carries what exists only in
//! it (the capacity permit while the machine is not stopped, the idle watch
//! and the maintenance loop while it runs, the reset while one holds it),
//! and the transitions that change it, each a task of the shard, so a
//! requester that goes away never cuts a boot or a save short.
//!
//! ```text
//!        admission                 idle stop, lifetime cap, close
//!   Off ----------> Booting -----> Running ----------------------> Saving --> Off
//!    ^                |  failed      |  |  death, or a sandbox the manager
//!    +----------------+              |  |  stopped (booted again)
//!    +-------------------------------+  |
//!    |                                  |  reset (from any phase)
//!    +------------ failed --------- Resetting -------- ready ----> Running
//! ```
//!
//! The Cloud's admission is what keeps it running: taking it wakes a stopped
//! Cloud, joins a boot under way, or waits for a running reset to finish,
//! holding nothing while it waits. Each operation of a Host handle an
//! admission made takes a lease of its own, refused while the Cloud is not
//! running.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::future::Future;
use std::rc::Rc;
use std::time::Duration;

use demi_gates::{ActivityGate, GateLease, Purpose};
use demi_host_remote::Admission;
use demi_machines_protocol::{HibernateParams, RuntimeState, RuntimeStateParams, WakeParams};
use demi_runner_protocol::boot::ManagedBoot;
use demi_shell::{HostError, HostErrorKind};
use demi_web_api::cloud::CloudState;
use demi_web_api::error::ErrorCode;
use demi_web_api::ids::{DeviceId, OperationId};
use futures_util::FutureExt as _;
use futures_util::future::{LocalBoxFuture, Shared};
use tokio::time::Instant;
use tokio_util::task::AbortOnDropHandle;

use super::capacity::CapacityPermit;
use super::client::MachinesError;
use crate::auth::sessions::TokenHash;
use crate::runner::codes::new_device_token;
use crate::shard::Shard;
use crate::storage::StorageError;
use crate::storage::devices::DeviceRecord;
use crate::storage::managed::ManagedOperation;

/// Why the Cloud admits no operation, or why one of its transitions failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum CloudError {
    #[error("Cloud is shutting down")]
    Closed,
    /// Its runtime was lost too often in a short time.
    #[error("Cloud repeatedly failed; reset the environment to recover")]
    CrashLoop,
    #[error("Cloud capacity is currently full; retry later")]
    Capacity,
    /// Another reset holds the Cloud.
    #[error("Another reset is in progress")]
    Resetting,
    /// A boot, a save or a reset failed, or the machine manager did.
    #[error("{0}")]
    Failed(String),
}

impl CloudError {
    /// The code and HTTP status an operation refused this way answers with
    /// (`web-api.md` § Cloud).
    pub(crate) fn code(&self) -> (ErrorCode, u16) {
        match self {
            Self::Closed => (ErrorCode::BackendClosing, 503),
            Self::CrashLoop => (ErrorCode::CloudCrashLoop, 503),
            Self::Capacity => (ErrorCode::CloudCapacity, 503),
            Self::Resetting => (ErrorCode::CloudResetting, 409),
            Self::Failed(_) => (ErrorCode::CloudUnavailable, 503),
        }
    }
}

impl From<MachinesError> for CloudError {
    fn from(error: MachinesError) -> Self {
        Self::Failed(error.to_string())
    }
}

impl From<StorageError> for CloudError {
    fn from(error: StorageError) -> Self {
        tracing::error!(error = &error as &dyn std::error::Error, "the Cloud's records failed");
        Self::Failed(format!("The Cloud's records could not be read or written: {error}"))
    }
}

/// A hold of the Cloud's admission: the Cloud keeps running while it is
/// held, and each operation of a Host made under it takes `per_operation`.
pub(crate) struct CloudAdmission {
    pub(crate) device: DeviceId,
    pub(crate) per_operation: Admission,
    _held: GateLease,
}

/// A transition joiners await; the task behind it runs to its end whoever
/// waits.
pub(super) type Transition = Shared<LocalBoxFuture<'static, Result<(), CloudError>>>;

/// The user's Cloud, in the user's shard: its one machine, made on the
/// first need, and whether the shard is closing it.
#[derive(Default)]
pub(crate) struct Cloud {
    machine: RefCell<Option<Rc<Machine>>>,
    closed: Cell<bool>,
}

impl Cloud {
    pub(super) fn machine(&self) -> Option<Rc<Machine>> {
        self.machine.borrow().clone()
    }

    pub(super) fn is_closed(&self) -> bool {
        self.closed.get()
    }

    /// Whether the machine of `device` runs, which a new expose on it needs
    /// (`expose.md` § Lifetime).
    pub(crate) fn runs(&self, device: &DeviceId) -> bool {
        self.machine()
            .is_some_and(|machine| machine.device.id == *device && machine.is_running())
    }

    /// Admits nothing more and ends the running machine's schedules, as the
    /// shard's close does first; a retirement already running finishes.
    pub(crate) fn stop(&self) {
        self.closed.set(true);
        if let Some(machine) = self.machine() {
            machine.stop_schedules();
        }
    }
}

/// The user's Cloud device and what the shard knows of its machine.
pub(super) struct Machine {
    pub(super) device: DeviceRecord,
    /// Every operation on the Cloud holds a lease of it; an idle stop and a
    /// reset reserve it; a checkpoint holds a maintenance lease.
    pub(super) gate: ActivityGate,
    phase: RefCell<Phase>,
    /// The boot, recovery or save under way, which admission and a reset
    /// join. Registered in the step that starts it and cleared in the step
    /// that ends it, so a phase that has one always shows it.
    transition: RefCell<Option<Transition>>,
    /// The latest reset, as storage holds it.
    pub(super) operation: RefCell<Option<ManagedOperation>>,
    /// When the manager reported the sandbox dead, the recent ones.
    deaths: RefCell<VecDeque<Instant>>,
    /// Why the last boot, save or reset failed, until a boot succeeds.
    pub(super) error: RefCell<Option<String>>,
}

pub(super) enum Phase {
    Off,
    Booting(CapacityPermit),
    Running(Running),
    Saving(CapacityPermit),
    Resetting {
        permit: CapacityPermit,
        operation: OperationId,
        task: Transition,
    },
}

/// A running machine: its permit, when it started and was last saved, and
/// the schedules that end with the phase.
pub(super) struct Running {
    pub(super) permit: CapacityPermit,
    pub(super) started_at: Instant,
    pub(super) checkpoint_at: Cell<Instant>,
    /// The idle watch and the maintenance loop.
    schedules: RefCell<Vec<AbortOnDropHandle<()>>>,
}

/// What admission found once no transition was under way.
enum Ready {
    Running,
    /// A reset holds the machine: the admission waits for it and tries again.
    Resetting,
}

impl Machine {
    fn new(device: DeviceRecord, operation: Option<ManagedOperation>) -> Self {
        Self {
            device,
            gate: ActivityGate::new(),
            phase: RefCell::new(Phase::Off),
            transition: RefCell::new(None),
            operation: RefCell::new(operation),
            deaths: RefCell::new(VecDeque::new()),
            error: RefCell::new(None),
        }
    }

    pub(super) fn phase(&self) -> std::cell::RefMut<'_, Phase> {
        self.phase.borrow_mut()
    }

    pub(super) fn state(&self) -> CloudState {
        match &*self.phase.borrow() {
            Phase::Off => CloudState::Off,
            Phase::Booting(_) => CloudState::Booting,
            Phase::Running(_) => CloudState::Running,
            Phase::Saving(_) => CloudState::Saving,
            Phase::Resetting { .. } => CloudState::Resetting,
        }
    }

    pub(super) fn is_running(&self) -> bool {
        matches!(&*self.phase.borrow(), Phase::Running(_))
    }

    /// When the running machine started and was last saved.
    pub(super) fn running_times(&self) -> Option<(Instant, Instant)> {
        match &*self.phase.borrow() {
            Phase::Running(running) => Some((running.started_at, running.checkpoint_at.get())),
            _ => None,
        }
    }

    /// Records that a checkpoint started at `at` published the machine's
    /// disks, while it still runs.
    pub(super) fn checkpointed(&self, at: Instant) {
        if let Phase::Running(running) = &*self.phase.borrow() {
            running.checkpoint_at.set(at);
        }
    }

    pub(super) fn transition(&self) -> Option<Transition> {
        self.transition.borrow().clone()
    }

    /// The reset that holds the machine.
    pub(super) fn reset_task(&self) -> Option<Transition> {
        match &*self.phase.borrow() {
            Phase::Resetting { task, .. } => Some(task.clone()),
            _ => None,
        }
    }

    /// The operation of the reset that holds the machine.
    pub(super) fn resetting(&self) -> Option<OperationId> {
        match &*self.phase.borrow() {
            Phase::Resetting { operation, .. } => Some(operation.clone()),
            _ => None,
        }
    }

    /// Ends the idle watch and the maintenance loop of the running machine,
    /// which stays running.
    pub(super) fn stop_schedules(&self) {
        if let Phase::Running(running) = &*self.phase.borrow() {
            running.schedules.borrow_mut().clear();
        }
    }

    /// A death the manager reported now, counted for the crash loop.
    fn record_death(&self, window: Duration) {
        let now = Instant::now();
        let mut deaths = self.deaths.borrow_mut();
        deaths.push_back(now);
        while deaths.front().is_some_and(|at| now.duration_since(*at) >= window) {
            deaths.pop_front();
        }
    }

    /// Forgets the recorded deaths, as a reset does.
    pub(super) fn clear_deaths(&self) {
        self.deaths.borrow_mut().clear();
    }

    /// Whether `limit` deaths fell within the last `window`.
    fn crash_looping(&self, limit: u32, window: Duration) -> bool {
        let now = Instant::now();
        let recent = self
            .deaths
            .borrow()
            .iter()
            .filter(|at| now.duration_since(**at) < window)
            .count();
        recent >= limit as usize
    }

    /// A lease for one operation of a Host the admission made, refused while
    /// the machine is not running or while a transition reserves it.
    fn admit(&self) -> Result<GateLease, HostError> {
        if !self.is_running() {
            return Err(HostError::new(HostErrorKind::Unavailable, "Cloud is not accepting operations"));
        }
        self.gate
            .try_enter(Purpose::Demand)
            .ok_or_else(|| HostError::new(HostErrorKind::Unavailable, "Cloud is changing state"))
    }
}

impl Shard {
    /// The user's Cloud machine, made on the first need with the latest
    /// reset its records hold.
    pub(super) async fn cloud_machine(&self, device: &DeviceRecord) -> Result<Rc<Machine>, StorageError> {
        if let Some(machine) = self.cloud().machine() {
            return Ok(machine);
        }
        let operation = self
            .services()
            .control
            .latest_managed_operation(device.id.clone())
            .await?;
        // Two first needs may both have read; the first to get here makes
        // the machine, and no await separates this check from the store.
        if let Some(machine) = self.cloud().machine() {
            return Ok(machine);
        }
        let machine = Rc::new(Machine::new(device.clone(), operation));
        self.cloud().machine.replace(Some(machine.clone()));
        Ok(machine)
    }

    /// Takes the Cloud's admission (`sessions-and-targets.md` § Host
    /// operations): it wakes a stopped Cloud, joins a boot under way, or
    /// waits for a running reset to finish, holding nothing while it waits,
    /// since a reset reserves what it would hold. An operation that waited
    /// for a reset that failed fails with it. Dropping the future at any
    /// await is safe: every transition it waits for is a task of its own.
    pub(crate) async fn admit_cloud(&self, device: &DeviceRecord) -> Result<CloudAdmission, CloudError> {
        let machine = self.cloud_machine(device).await?;
        loop {
            if self.cloud().is_closed() {
                return Err(CloudError::Closed);
            }
            if let Some(reset) = machine.reset_task() {
                reset.await?;
                continue;
            }
            let lease = machine.gate.enter(Purpose::Demand).await;
            match self.ensure_running(&machine).await? {
                Ready::Running => {
                    let per_operation = {
                        let machine = machine.clone();
                        Admission::Leased(Rc::new(move || machine.admit()))
                    };
                    return Ok(CloudAdmission {
                        device: device.id.clone(),
                        per_operation,
                        _held: lease,
                    });
                }
                Ready::Resetting => drop(lease),
            }
        }
    }

    /// Makes the machine run, for an admission that holds a lease: joins the
    /// transition under way; boots a stopped machine; and recovers a running
    /// one whose runner is gone, which a manager restart leaves without a
    /// death event.
    async fn ensure_running(&self, machine: &Rc<Machine>) -> Result<Ready, CloudError> {
        loop {
            if self.cloud().is_closed() {
                return Err(CloudError::Closed);
            }
            if let Some(transition) = machine.transition() {
                // Concurrent needs join one boot and share its failure.
                transition.await?;
                continue;
            }
            let recover = {
                let mut phase = machine.phase();
                match &*phase {
                    Phase::Resetting { .. } => return Ok(Ready::Resetting),
                    Phase::Running(_) if self.devices().online(&machine.device.id) => return Ok(Ready::Running),
                    Phase::Running(_) => true,
                    // A boot and a save register their transition in the step
                    // that sets the phase, which the check above found none of.
                    Phase::Booting(_) | Phase::Saving(_) => {
                        return Err(CloudError::Failed("Cloud is changing state".into()));
                    }
                    Phase::Off => {
                        // From the crash-loop check to the transition's
                        // registration nothing awaits: two first needs start
                        // one boot, and capacity is never overshot.
                        let tuning = &self.services().cloud.tuning;
                        if machine.crash_looping(tuning.crash_loop_deaths, tuning.crash_loop_window) {
                            return Err(CloudError::CrashLoop);
                        }
                        let permit = self.services().cloud.capacity.try_take().ok_or(CloudError::Capacity)?;
                        *phase = Phase::Booting(permit);
                        false
                    }
                }
            };
            let shard = self.this();
            let target = machine.clone();
            let transition = if recover {
                self.spawn_transition(machine, async move { shard.recover(&target).await })
            } else {
                self.spawn_transition(machine, async move {
                    let booted = shard.boot(&target).await;
                    shard.finish_boot(&target, &booted);
                    booted
                })
            };
            transition.await?;
        }
    }

    /// Runs `work` as the machine's transition, a task of the shard; the
    /// transition is cleared in the step that ends it.
    pub(super) fn spawn_transition(
        &self,
        machine: &Rc<Machine>,
        work: impl Future<Output = Result<(), CloudError>> + 'static,
    ) -> Transition {
        let ended = machine.clone();
        let task = self.tasks().spawn_local(async move {
            let result = work.await;
            ended.transition.replace(None);
            result
        });
        let transition = joinable(task);
        machine.transition.replace(Some(transition.clone()));
        transition
    }

    /// A task of the shard as a transition that joiners await.
    pub(super) fn spawn_joinable(&self, work: impl Future<Output = Result<(), CloudError>> + 'static) -> Transition {
        joinable(self.tasks().spawn_local(work))
    }

    /// Starts a sandbox for the machine: every boot mints a token, which
    /// ends every earlier boot's, and the boot counts once the runner
    /// connects with it. A boot that fails saves what the sandbox wrote, best
    /// effort, and lets its runner go.
    pub(super) async fn boot(&self, machine: &Machine) -> Result<(), CloudError> {
        let booted = self.start_sandbox(machine).await;
        if let Err(error) = &booted {
            let device = &machine.device.id;
            let hibernate = HibernateParams {
                device_id: device.to_string(),
            };
            // The boot's failure is the one reported; a save that fails too
            // is logged beside it.
            if let Err(saved) = self.services().cloud.machines.call(hibernate).await {
                tracing::warn!(device = %device, %error, "a Cloud whose boot failed was not saved: {saved}");
            }
            self.devices().disconnect(device, "Cloud boot failed");
        }
        booted
    }

    async fn start_sandbox(&self, machine: &Machine) -> Result<(), CloudError> {
        let services = self.services();
        let device = &machine.device.id;
        let token = new_device_token();
        services
            .control
            .rotate_device_token(device.clone(), TokenHash::of(token.expose()))
            .await?;
        let backend_url = services
            .cloud
            .backend_url()
            .cloned()
            .ok_or_else(|| CloudError::Failed("The backend does not listen yet".into()))?;
        let wake = WakeParams {
            device_id: device.to_string(),
            boot: ManagedBoot {
                backend_url,
                device_token: token,
            },
        };
        services.cloud.machines.call(wake).await?;
        let connection = services.cloud.tuning.runner_connection;
        tokio::time::timeout(connection, self.devices().until_online(device))
            .await
            .map_err(|_| CloudError::Failed("Cloud boot timeout: its runner did not connect".into()))
    }

    /// Ends the boot under way: a machine whose runner connected runs, one
    /// whose boot failed is off with the failure. A reset that took the
    /// machine over meanwhile sets its phase itself.
    pub(super) fn finish_boot(&self, machine: &Rc<Machine>, booted: &Result<(), CloudError>) {
        let mut phase = machine.phase();
        if !matches!(&*phase, Phase::Booting(_)) {
            return;
        }
        let Phase::Booting(permit) = std::mem::replace(&mut *phase, Phase::Off) else {
            unreachable!("the phase was just matched")
        };
        match booted {
            Ok(()) => {
                machine.error.replace(None);
                *phase = Phase::Running(self.running(machine, permit));
            }
            Err(error) => {
                machine.error.replace(Some(error.to_string()));
            }
        }
    }

    /// The running phase of a machine that just started: its idle watch and
    /// its maintenance loop start with it.
    pub(super) fn running(&self, machine: &Rc<Machine>, permit: CapacityPermit) -> Running {
        let now = Instant::now();
        Running {
            permit,
            started_at: now,
            checkpoint_at: Cell::new(now),
            schedules: RefCell::new(self.cloud_schedules(machine)),
        }
    }

    /// A running machine whose runner is gone: a sandbox that still runs
    /// gets the runner connection time to reconnect, with its token; one
    /// that stopped, as a manager restart stops it without a death event,
    /// boots again over its saved storage.
    async fn recover(&self, machine: &Rc<Machine>) -> Result<(), CloudError> {
        let device = &machine.device.id;
        let state = self
            .services()
            .cloud
            .machines
            .call(RuntimeStateParams {
                device_id: device.to_string(),
            })
            .await?;
        if state == RuntimeState::Running {
            let connection = self.services().cloud.tuning.runner_connection;
            return tokio::time::timeout(connection, self.devices().until_online(device))
                .await
                .map_err(|_| CloudError::Failed("Cloud runner reconnect timeout".into()));
        }
        {
            let mut phase = machine.phase();
            match std::mem::replace(&mut *phase, Phase::Off) {
                Phase::Running(running) => *phase = Phase::Booting(running.permit),
                // The machine changed meanwhile; there is nothing to recover.
                other => {
                    *phase = other;
                    return Ok(());
                }
            }
        }
        let booted = self.boot(machine).await;
        self.finish_boot(machine, &booted);
        booted
    }

    /// The manager reported that the device's sandbox exited without being
    /// asked to stop (`managed-hosts.md` § Lifecycle and capacity). A running
    /// machine is off: its runner goes, and the death counts for the crash
    /// loop, as one during a boot does. One while the backend saves or resets
    /// the machine is its own doing and is ignored.
    pub(crate) fn cloud_died(&self, device: &DeviceId) {
        let Some(machine) = self.cloud().machine().filter(|machine| machine.device.id == *device) else {
            return;
        };
        let window = self.services().cloud.tuning.crash_loop_window;
        {
            let mut phase = machine.phase();
            match &*phase {
                Phase::Off | Phase::Saving(_) | Phase::Resetting { .. } => return,
                Phase::Booting(_) => {
                    machine.record_death(window);
                    return;
                }
                Phase::Running(_) => {
                    machine.record_death(window);
                    *phase = Phase::Off;
                }
            }
        }
        tracing::warn!(device = %device, "the Cloud's sandbox stopped by itself");
        self.devices().disconnect(device, "the Cloud's sandbox stopped");
    }

    /// Stops a running machine and saves it, under a reservation of its gate
    /// that the caller holds: its runner flushes the filesystems, best
    /// effort, the manager saves and stops it, and it is off, with the error
    /// kept when the save failed.
    pub(super) async fn hibernate_reserved(&self, machine: &Rc<Machine>) -> Result<(), CloudError> {
        if let Some(transition) = machine.transition() {
            // A boot that failed left the machine off; one that succeeded is
            // stopped below.
            let _ = transition.await;
        }
        {
            let mut phase = machine.phase();
            match std::mem::replace(&mut *phase, Phase::Off) {
                Phase::Running(running) => *phase = Phase::Saving(running.permit),
                other => {
                    *phase = other;
                    return Ok(());
                }
            }
        }
        let shard = self.this();
        let target = machine.clone();
        let save = self.spawn_transition(machine, async move {
            let saved = shard.save(&target).await;
            let mut phase = target.phase();
            if matches!(&*phase, Phase::Saving(_)) {
                *phase = Phase::Off;
            }
            if let Err(error) = &saved {
                target.error.replace(Some(error.to_string()));
            }
            saved
        });
        save.await
    }

    /// Flushes and saves the machine's disks and stops its sandbox, then
    /// lets its runner go. The flush needs a live runner and is skipped
    /// without one: a broken guest never keeps the disks from being saved.
    pub(super) async fn save(&self, machine: &Machine) -> Result<(), CloudError> {
        let device = &machine.device.id;
        self.flush(device).await;
        let hibernate = HibernateParams {
            device_id: device.to_string(),
        };
        let saved = self.services().cloud.machines.call(hibernate).await;
        self.devices().disconnect(device, "Cloud stopped");
        Ok(saved?)
    }

    /// Asks the Cloud's runner to flush its writable filesystems, within the
    /// sync timeout; a runner that is gone or fails is logged.
    pub(super) async fn flush(&self, device: &DeviceId) {
        let Some(link) = self.devices().link(device) else {
            return;
        };
        match tokio::time::timeout(self.services().cloud.tuning.sync_timeout, link.sync()).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::warn!(device = %device, "the Cloud did not flush its filesystems: {error}"),
            Err(_) => tracing::warn!(device = %device, "the Cloud did not flush its filesystems in time"),
        }
    }

    /// Closes the Cloud with its shard (`backend.md` § Startup and shutdown):
    /// no admission after this, its schedules end, a reset or a transition
    /// under way finishes, and a running machine is saved and stopped. A
    /// machine something still holds is left to the manager's reconcile,
    /// which saves every machine when the backend's client closes.
    pub(crate) async fn close_cloud(&self) -> Result<(), CloudError> {
        self.cloud().stop();
        let Some(machine) = self.cloud().machine() else {
            return Ok(());
        };
        if let Some(reset) = machine.reset_task() {
            // Its failure is recorded with the operation.
            let _ = reset.await;
        }
        if let Some(transition) = machine.transition() {
            let _ = transition.await;
        }
        let Some(reservation) = machine.gate.try_reserve() else {
            tracing::warn!(device = %machine.device.id, "the Cloud is in use at shutdown; the machine manager saves it");
            return Ok(());
        };
        let saved = self.hibernate_reserved(&machine).await;
        drop(reservation);
        saved
    }
}

/// A task's end as a transition joiners await; a task that panicked fails
/// them.
fn joinable(task: tokio::task::JoinHandle<Result<(), CloudError>>) -> Transition {
    async move {
        task.await
            .unwrap_or_else(|_| Err(CloudError::Failed("A transition of the Cloud ended without an answer".into())))
    }
    .boxed_local()
    .shared()
}
