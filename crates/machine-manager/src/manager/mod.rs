//! The manager's state and its request path: one worker per device runs that device's operations in
//! order, and the manager-wide admission gate lets `reconcile` and shutdown wait for the operations
//! in flight.

pub mod admission;
#[cfg(target_os = "linux")]
mod device;
#[cfg(target_os = "linux")]
mod errors;

#[cfg(target_os = "linux")]
pub use device::Placement;
#[cfg(target_os = "linux")]
pub use errors::{OpError, Parts};
#[cfg(target_os = "linux")]
pub use linux::{Core, Manager};

#[cfg(target_os = "linux")]
mod linux {
    use std::{
        cell::RefCell,
        collections::HashMap,
        net::Ipv4Addr,
        rc::{Rc, Weak},
    };

    use demi_machine_manager_protocol::{
        BaseVersion, Capacity, CheckpointParams, CurrentBaseVersionParams, DRAIN_RECORD, DeviceId, Download, DrainRecord, HibernateParams, ImageStateParams, Isolation, LeftRecord, MachineCall, ManagerEvent, NetworkRules, Offer, Operation,
        ReconcileParams, Reconciled, ResetParams, RuntimeOffer, RuntimeState, RuntimeStateParams, SandboxMachine, SetIsolationParams, SetNetworkRulesParams, Unavailable, WakeParams,
    };
    use tokio::{
        sync::{mpsc, oneshot, watch},
        task::JoinSet,
    };
    use tokio_util::{sync::CancellationToken, task::AbortOnDropHandle};

    use super::{
        admission::Admission,
        device::{DeviceCommand, DeviceWorker, Mover, Placement},
        errors::{OpError, Parts},
    };
    use crate::{
        blocking,
        config::Config,
        network::slots::Slot,
        recovery,
        runtime::{
            DeviceOp, DeviceRuntime, Left, Reports, Running,
            gvisor::{self, Gvisor},
            moves,
            simple::{self, Simple},
        },
        server::MachineService,
        storage::{
            store::{ImageStore, Retention},
            working::WorkingPair,
        },
        tools::Tools,
    };

    /// Commands waiting for one device's worker. A full queue holds back the
    /// request that sends; its admission covers the wait.
    const DEVICE_QUEUE: usize = 64;

    /// What every operation uses and nothing changes after startup.
    pub struct Core {
        pub config: Config,
        pub tools: Tools,
        /// The gVisor runtime, or why this manager cannot run it.
        pub gvisor: Result<Gvisor, String>,
        pub simple: Simple,
    }

    impl Core {
        /// A core that runs gVisor, with the resolvers `gvisor` holds, unless
        /// it says why it cannot.
        pub fn new(config: Config, tools: Tools, gvisor: Result<Vec<Ipv4Addr>, String>) -> Self {
            Self {
                gvisor: gvisor.map(|dns| Gvisor::new(&config, &tools, dns)),
                simple: Simple::new(&config),
                tools,
                config,
            }
        }

        /// The gVisor runtime, which only a manager that runs it reaches.
        pub fn gvisor(&self) -> &Gvisor {
            self.gvisor
                .as_ref()
                .expect("only a manager that runs gVisor takes a gVisor path")
        }

        /// Imported bases: `<data>/images/bases`.
        pub fn bases(&self) -> std::path::PathBuf {
            ImageStore::new(self.config.images()).bases()
        }

        /// Where the devices' storage is on each runtime, which a blocking
        /// job reads.
        fn storage(&self) -> Storage {
            Storage {
                images: ImageStore::new(self.config.images()),
                working: self.config.working(),
                trees: self.simple.trees.clone(),
            }
        }
    }

    /// Where the devices' storage is: gVisor's generations and working
    /// pairs, and Simple Isolation's trees.
    struct Storage {
        images: ImageStore,
        working: std::path::PathBuf,
        trees: simple::Trees,
    }

    impl Storage {
        /// Whether `device`'s storage is on gVisor: a generation or a
        /// working pair.
        fn on_gvisor(&self, off: &blocking::OffLoop, device: &DeviceId) -> Result<bool, OpError> {
            Ok(self.images.read(off, device)?.is_some() || WorkingPair::new(&self.working, device).directory().exists())
        }
    }

    /// The manager: its devices' workers and the admission gate.
    pub struct Manager {
        /// Itself, for the moves it runs in the background.
        this: Weak<Manager>,
        core: Rc<Core>,
        base: BaseVersion,
        admission: Admission,
        devices: RefCell<HashMap<DeviceId, mpsc::Sender<DeviceCommand>>>,
        workers: RefCell<JoinSet<()>>,
        deaths: mpsc::Sender<DeviceId>,
        changes: mpsc::Sender<DeviceId>,
        /// What Clouds reach, as the backend last named it.
        network_rules: RefCell<NetworkRules>,
        placement: Rc<Placement>,
        /// The moves of the stopped devices to the chosen isolation, one at
        /// a time; a new choice replaces them.
        moving: RefCell<Option<AbortOnDropHandle<()>>>,
        /// Cancels the download of gVisor's release under way, which choosing
        /// Simple Isolation does.
        downloading: RefCell<Option<CancellationToken>>,
    }

    impl Manager {
        /// A manager whose new devices start on `base`, that offers
        /// `offers`, and whose workers report sandbox deaths to `deaths` and
        /// each operation's end to `changes`.
        pub fn new(
            core: Rc<Core>,
            base: BaseVersion,
            offers: Vec<RuntimeOffer>,
            deaths: mpsc::Sender<DeviceId>,
            changes: mpsc::Sender<DeviceId>,
        ) -> Rc<Self> {
            Rc::new_cyclic(|this| Self {
                this: this.clone(),
                core,
                base,
                admission: Admission::new(),
                devices: RefCell::default(),
                workers: RefCell::default(),
                network_rules: RefCell::default(),
                placement: Rc::new(Placement::new(offers)),
                moving: RefCell::default(),
                downloading: RefCell::default(),
                deaths,
                changes,
            })
        }

        /// Where `device`'s worker and runtime report to the server.
        fn reports(&self, device: &DeviceId) -> Reports {
            Reports::new(device.clone(), self.deaths.clone(), self.changes.clone())
        }

        /// The worker of `device`, started on first use on the runtime its
        /// storage is on, or for a device without storage the one
        /// [`Placement::for_new`] names. A device whose storage is on gVisor
        /// where this manager cannot run it has no worker. A device whose
        /// storage is on both, which a move cut short leaves, keeps the
        /// newer.
        async fn worker(&self, device: &DeviceId) -> Result<mpsc::Sender<DeviceCommand>, OpError> {
            if let Some(sender) = self.devices.borrow().get(device) {
                return Ok(sender.clone());
            }
            let mut on = self.storage_of(device).await?;
            if on == (true, true) {
                moves::settle(&self.core, device).await?;
                on = self.storage_of(device).await?;
            }
            // Another request may have started it meanwhile.
            if let Some(sender) = self.devices.borrow().get(device) {
                return Ok(sender.clone());
            }
            let isolation = match (&self.core.gvisor, on) {
                (Err(reason), (true, _)) => return Err(OpError::GvisorUnavailable(reason.clone())),
                (_, (true, _)) => Isolation::Gvisor,
                (_, (false, true)) => Isolation::Simple,
                (_, (false, false)) => self.placement.for_new(),
            };
            let reports = self.reports(device);
            let runtime = DeviceRuntime::on(isolation, device.clone(), self.core.clone(), self.base.clone(), reports.clone());
            Ok(self.start(device, runtime, reports))
        }

        /// Whether `device`'s storage is on gVisor, and whether on Simple
        /// Isolation.
        async fn storage_of(&self, device: &DeviceId) -> Result<(bool, bool), OpError> {
            let (storage, id) = (self.core.storage(), device.clone());
            blocking::run(move |off| -> Result<(bool, bool), OpError> {
                Ok((storage.on_gvisor(off, &id)?, storage.trees.read(off, &id)?.is_some()))
            })
            .await
        }

        /// Starts `device`'s worker on `runtime`.
        fn start(&self, device: &DeviceId, runtime: DeviceRuntime, reports: Reports) -> mpsc::Sender<DeviceCommand> {
            let (sender, inbox) = mpsc::channel(DEVICE_QUEUE);
            let mover = Mover {
                device: device.clone(),
                core: self.core.clone(),
                base: self.base.clone(),
                placement: self.placement.clone(),
            };
            let worker = DeviceWorker::new(runtime, reports, mover);
            self.workers.borrow_mut().spawn_local(worker.run(inbox));
            self.devices.borrow_mut().insert(device.clone(), sender.clone());
            sender
        }

        /// Gives `device` a worker that runs `sandbox`, which a manager
        /// before this one left running (`crate::handover`).
        pub fn adopt(&self, device: DeviceId, sandbox: crate::sandbox::Sandbox) {
            let reports = self.reports(&device);
            let runtime = DeviceRuntime::Gvisor(gvisor::Device::adopted(
                device.clone(),
                self.core.clone(),
                self.base.clone(),
                reports.clone(),
                sandbox,
            ));
            self.start(&device, runtime, reports);
        }

        /// Gives `device` a worker that runs `process`, the proot a manager
        /// before this one left running (`crate::handover`).
        pub fn adopt_simple(&self, device: DeviceId, process: simple::Process) {
            let reports = self.reports(&device);
            let runtime = DeviceRuntime::Simple(simple::Device::adopted(
                device.clone(),
                self.core.clone(),
                self.base.clone(),
                process,
            ));
            self.start(&device, runtime, reports);
        }

        /// Leaves every running sandbox to the next manager: waits for the
        /// operations in flight, then takes each sandbox from its worker
        /// without stopping it, and refuses every request still waiting, as
        /// a shutdown does. Answers what the next manager takes over.
        pub async fn leave(&self) -> Result<LeftRecord, OpError> {
            self.placement.stop();
            let _whole = self.admission.exclusive().await?;
            let workers: Vec<_> = self.devices.borrow().values().cloned().collect();
            let mut left = LeftRecord {
                sandboxes: Vec::new(),
                simple: Vec::new(),
            };
            for worker in workers {
                let (reply, answer) = oneshot::channel();
                // A worker that stopped runs no sandbox.
                if worker.send(DeviceCommand::Leave { reply }).await.is_err() {
                    continue;
                }
                match answer.await {
                    Ok(Some(Left::Gvisor(sandbox))) => left.sandboxes.push(sandbox),
                    Ok(Some(Left::Simple(process))) => left.simple.push(process),
                    _ => {}
                }
            }
            self.admission.close();
            Ok(left)
        }

        /// Runs `op` on `device`'s worker, admitted beside other device
        /// operations. The admission also covers the wait in the queue.
        async fn run(&self, device: &DeviceId, op: DeviceOp) -> Result<(), OpError> {
            let _admitted = self.admission.enter().await?;
            let (reply, answer) = oneshot::channel();
            self.worker(device)
                .await?
                .send(DeviceCommand::Run { op, reply })
                .await
                .map_err(|_| OpError::WorkerStopped)?;
            answer.await.map_err(|_| OpError::WorkerStopped)?
        }

        async fn runtime_state(&self, device: &DeviceId) -> Result<RuntimeState, OpError> {
            let _admitted = self.admission.enter().await?;
            let (reply, answer) = oneshot::channel();
            self.worker(device)
                .await?
                .send(DeviceCommand::Running { reply })
                .await
                .map_err(|_| OpError::WorkerStopped)?;
            Ok(match answer.await.map_err(|_| OpError::WorkerStopped)? {
                Some(_) => RuntimeState::Running,
                None => RuntimeState::Stopped,
            })
        }

        /// Reports the devices whose sandboxes run, which stay running,
        /// recovers what an interrupted operation left of the others, and
        /// installs the firewall again with the running sandboxes' slots and
        /// the network rules the backend last named: a backend's restart
        /// stops no Cloud and cuts none off.
        pub async fn reconcile(&self) -> Result<Reconciled, OpError> {
            let _whole = self.admission.exclusive().await?;
            let (running, states): (Vec<DeviceId>, Vec<Running>) = self.running_devices().await.into_iter().unzip();
            self.core.simple.recover(&running).await?;
            if let Ok(gvisor) = &self.core.gvisor {
                recovery::fence_and_save(&self.core, &running).await?;
                let rules = self.network_rules.borrow().clone();
                gvisor.network.prepare(&slots(&states), &rules).await?;
            }
            Ok(Reconciled {
                running: running.iter().map(|device| device.as_str().to_owned()).collect(),
            })
        }

        /// Chooses the isolation new devices start on, and moves every
        /// stopped device whose storage is on the other runtime to it, one
        /// at a time in the background; a running one moves when it stops.
        /// A failed move to the isolation chosen before is forgotten.
        pub async fn set_isolation(&self, isolation: Isolation) -> Result<(), OpError> {
            let forgotten = self.placement.choose(isolation);
            match isolation {
                Isolation::Gvisor if self.placement.awaits_download(Isolation::Gvisor) => self.download_gvisor(),
                Isolation::Gvisor => {}
                Isolation::Simple => {
                    if let Some(download) = self.downloading.take() {
                        download.cancel();
                        self.placement.set_download(Isolation::Gvisor, Download::Waiting);
                    }
                }
            }
            for device in forgotten {
                // The server stops listening at shutdown, when no one is left
                // to tell.
                let _ = self.changes.send(device).await;
            }
            let this = self.this.clone();
            let moving = tokio::task::spawn_local(async move {
                let Some(manager) = this.upgrade() else {
                    return;
                };
                if let Err(error) = manager.move_stopped().await {
                    tracing::error!(event = "cloud_move.list_failed", error = %crate::server::chain(&error), "the Clouds to move could not be listed");
                }
            });
            self.moving.replace(Some(AbortOnDropHandle::new(moving)));
            Ok(())
        }

        /// Hears each change of what the manager can do with each isolation.
        pub fn watch_offers(&self) -> watch::Receiver<Vec<RuntimeOffer>> {
            self.placement.watch_offers()
        }

        /// Fetches gVisor's release in the background, then has systemd
        /// restart the manager, whose next start runs gVisor. Choosing
        /// Simple Isolation, or the manager's stop, cancels it.
        fn download_gvisor(&self) {
            let Some(directory) = self.core.config.gvisor_directory() else {
                return;
            };
            let cancel = self.placement.stopping_child();
            if let Some(earlier) = self.downloading.replace(Some(cancel.clone())) {
                earlier.cancel();
            }
            let placement = self.placement.clone();
            placement.set_download(Isolation::Gvisor, Download::Downloading { received: 0, total: None });
            tokio::task::spawn_local(async move {
                // A change of what a page shows each time the download takes a
                // piece would flood the connections.
                const EVERY: std::time::Duration = std::time::Duration::from_millis(500);
                let mut told = std::time::Instant::now();
                let fetched = crate::gvisor_release::fetch(&directory, &cancel, |received, total| {
                    if told.elapsed() >= EVERY {
                        told = std::time::Instant::now();
                        placement.set_download(Isolation::Gvisor, Download::Downloading { received, total });
                    }
                })
                .await;
                // What cancelled it says what the download is now.
                if cancel.is_cancelled() {
                    return;
                }
                if let Err(error) = fetched {
                    tracing::warn!(event = "gvisor_release.fetch_failed", %error, "gVisor's release could not be downloaded");
                    placement.set_download(Isolation::Gvisor, Download::Failed { message: error.to_string() });
                    return;
                }
                tracing::info!(event = "gvisor_release.fetched", directory = %directory.display(), "gVisor's release is downloaded; the manager restarts to run it");
                placement.set_download(Isolation::Gvisor, Download::Starting);
                if let Err(reason) = crate::handover::restart().await {
                    tracing::warn!(event = "gvisor_release.restart_failed", reason, "the manager could not restart to run gVisor");
                    placement.set_offer(Isolation::Gvisor, Offer::Unavailable(Unavailable { reason }));
                }
            });
        }

        /// Asks each device whose storage is on the other runtime than the
        /// chosen isolation to move, waiting for each before the next, so
        /// one copy runs at a time.
        async fn move_stopped(&self) -> Result<(), OpError> {
            let storage = self.core.storage();
            let devices = blocking::run(move |off| -> Result<Vec<(DeviceId, Isolation)>, OpError> {
                let mut devices: Vec<(DeviceId, Isolation)> =
                    storage.images.devices(off)?.into_iter().map(|device| (device, Isolation::Gvisor)).collect();
                devices.extend(storage.trees.devices(off)?.into_iter().map(|device| (device, Isolation::Simple)));
                Ok(devices)
            })
            .await?;
            for (device, isolation) in devices {
                if isolation == self.placement.chosen() {
                    continue;
                }
                let Ok(_admitted) = self.admission.enter().await else {
                    return Ok(());
                };
                let worker = match self.worker(&device).await {
                    Ok(worker) => worker,
                    Err(error) => {
                        tracing::warn!(event = "cloud_move.skipped", device_id = %device, error = %crate::server::chain(&error), "a Cloud to move has no worker");
                        continue;
                    }
                };
                let (reply, answer) = oneshot::channel();
                // A worker that stopped moves nothing.
                if worker.send(DeviceCommand::Settle { reply }).await.is_ok() {
                    let _ = answer.await;
                }
            }
            Ok(())
        }

        /// Installs the firewall with `rules` and the running sandboxes'
        /// slots; under the whole gate, so no sandbox starts or stops while
        /// the table is replaced.
        pub async fn set_network_rules(&self, rules: NetworkRules) -> Result<(), OpError> {
            let _whole = self.admission.exclusive().await?;
            // A manager without gVisor keeps the rules for no Cloud: Simple
            // Isolation's are on the host's network.
            if let Ok(gvisor) = &self.core.gvisor {
                let states: Vec<Running> = self.running_devices().await.into_iter().map(|(_, state)| state).collect();
                gvisor.network.prepare(&slots(&states), &rules).await?;
            }
            *self.network_rules.borrow_mut() = rules;
            Ok(())
        }

        /// The devices whose workers run a sandbox; the caller holds the
        /// whole gate, so no worker is running an operation.
        async fn running_devices(&self) -> Vec<(DeviceId, Running)> {
            let workers: Vec<_> = self
                .devices
                .borrow()
                .iter()
                .map(|(device, worker)| (device.clone(), worker.clone()))
                .collect();
            let mut running = Vec::new();
            for (device, worker) in workers {
                let (reply, answer) = oneshot::channel();
                // A worker that stopped runs no sandbox.
                if worker.send(DeviceCommand::Running { reply }).await.is_err() {
                    continue;
                }
                if let Ok(Some(slot)) = answer.await {
                    running.push((device, slot));
                }
            }
            running.sort_by(|(left, _), (right, _)| left.cmp(right));
            running
        }

        /// Shutdown: waits for the operations in flight, stops and saves
        /// every device, then refuses every request still waiting, so none
        /// can start a sandbox after the drain.
        pub async fn close(&self) -> Result<(), OpError> {
            self.placement.stop();
            let _whole = self.admission.exclusive().await?;
            let drained = self.drain().await;
            self.admission.close();
            drained
        }

        /// Hibernates every known device at once; the caller holds the
        /// whole gate, so no worker is running an operation. How many of
        /// the running Clouds are saved goes to the drain's record, which an
        /// upgrade shows while it waits for the stop.
        async fn drain(&self) -> Result<(), OpError> {
            let running: std::collections::BTreeSet<DeviceId> =
                self.running_devices().await.into_iter().map(|(device, _)| device).collect();
            let saving = u32::try_from(running.len()).unwrap_or(u32::MAX);
            let saved = std::sync::atomic::AtomicU32::new(0);
            record_drain(DrainRecord { saved: 0, saving });
            let workers: Vec<_> = self
                .devices
                .borrow()
                .iter()
                .map(|(device, worker)| (device.clone(), worker.clone()))
                .collect();
            let outcomes =
                futures_util::future::join_all(workers.into_iter().map(|(device, worker)| {
                    let (running, saved) = (&running, &saved);
                    async move {
                        let (reply, answer) = oneshot::channel();
                        // The manager's own save names no machine and removes
                        // no fallback.
                        let op = DeviceOp::Hibernate {
                            retention: Retention::Previous,
                        };
                        worker
                            .send(DeviceCommand::Run { op, reply })
                            .await
                            .map_err(|_| OpError::WorkerStopped)?;
                        let outcome = answer.await.map_err(|_| OpError::WorkerStopped)?;
                        if running.contains(&device) {
                            let saved = saved.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                            record_drain(DrainRecord { saved, saving });
                        }
                        outcome
                    }
                }))
                .await;
            let failures: Vec<OpError> = outcomes.into_iter().filter_map(Result::err).collect();
            if !failures.is_empty() {
                return Err(OpError::Shutdown(Parts(failures)));
            }
            Ok(())
        }
    }

    /// The network slots of the running gVisor sandboxes among `states`.
    fn slots(states: &[Running]) -> Vec<(Slot, SandboxMachine)> {
        states
            .iter()
            .filter_map(|state| Some((state.slot.clone()?, state.machine)))
            .collect()
    }

    /// Writes `record` whole to the drain's record; one that cannot be
    /// written leaves the upgrade showing the count it read last.
    fn record_drain(record: DrainRecord) {
        let path = std::path::Path::new(DRAIN_RECORD);
        let staged = path.with_extension("new");
        let written = serde_json::to_vec(&record)
            .map_err(std::io::Error::other)
            .and_then(|bytes| std::fs::write(&staged, bytes))
            .and_then(|()| std::fs::rename(&staged, path));
        if let Err(error) = written {
            tracing::debug!(event = "drain.record_failed", %error, "the drain's record was not written");
        }
    }

    /// The result an `ok` reply carries for operation `O`.
    fn reply<O: Operation>(output: O::Output) -> serde_json::Value {
        serde_json::to_value(output).expect("an operation's result encodes as JSON")
    }

    impl MachineService for Manager {
        type Error = OpError;

        /// A device's status from its storage: a gVisor generation's
        /// capacities and base, or a Simple Isolation tree's base, whose
        /// files have no capacities.
        async fn status(&self, device: &DeviceId) -> Option<ManagerEvent> {
            let (storage, id) = (self.core.storage(), device.clone());
            let read = blocking::run(move |off| -> Result<_, OpError> {
                let gvisor = storage.images.read(off, &id)?;
                let simple = match gvisor {
                    Some(_) => None,
                    None => storage.trees.read(off, &id)?,
                };
                Ok((gvisor, simple))
            })
            .await;
            match read {
                Ok((state, simple)) => Some(ManagerEvent::CloudStatus {
                    device_id: device.to_string(),
                    newer_image: state.as_ref().map(|state| &state.base_version)
                        .or(simple.as_ref().map(|record| &record.base_version))
                        .is_some_and(|base| *base != self.base),
                    isolation: match (&state, &simple) {
                        (Some(_), _) => Isolation::Gvisor,
                        (None, Some(_)) => Isolation::Simple,
                        (None, None) => self.placement.for_new(),
                    },
                    volumes: state.map(|state| Capacity {
                        system_bytes: state.system_bytes,
                        home_bytes: state.home_bytes,
                    }),
                    move_status: self.placement.move_status(device),
                }),
                Err(error) => {
                    // The backend keeps the status it had.
                    tracing::warn!(event = "manager_status.read_failed", device_id = %device, %error, "a device's stored image could not be read");
                    None
                }
            }
        }

        async fn snapshot(&self) -> Vec<ManagerEvent> {
            let storage = self.core.storage();
            let devices = match blocking::run(move |off| -> Result<Vec<DeviceId>, OpError> {
                let mut devices = storage.images.devices(off)?;
                devices.extend(storage.trees.devices(off)?);
                Ok(devices)
            })
            .await
            {
                Ok(devices) => devices,
                Err(error) => {
                    tracing::warn!(event = "manager_status.list_failed", %error, "the devices with stored images could not be listed");
                    return Vec::new();
                }
            };
            let mut events = Vec::new();
            for device in devices {
                events.extend(self.status(&device).await);
            }
            events
        }

        fn runtimes(&self) -> Vec<RuntimeOffer> {
            self.placement.offers()
        }

        async fn handle(&self, call: MachineCall) -> Result<serde_json::Value, OpError> {
            match call {
                MachineCall::Reconcile(_) => Ok(reply::<ReconcileParams>(self.reconcile().await?)),
                MachineCall::SetIsolation(params) => {
                    self.set_isolation(params.isolation).await?;
                    Ok(reply::<SetIsolationParams>(()))
                }
                MachineCall::SetNetworkRules(params) => {
                    self.set_network_rules(params.network_rules).await?;
                    Ok(reply::<SetNetworkRulesParams>(()))
                }
                MachineCall::CurrentBaseVersion(_) => {
                    Ok(reply::<CurrentBaseVersionParams>(self.base.clone()))
                }
                MachineCall::ImageState(params) => {
                    let device = DeviceId::parse(params.device_id)?;
                    let store = ImageStore::new(self.core.config.images());
                    let state = blocking::run(move |off| store.read(off, &device)).await?;
                    Ok(reply::<ImageStateParams>(state))
                }
                MachineCall::RuntimeState(params) => {
                    let device = DeviceId::parse(params.device_id)?;
                    Ok(reply::<RuntimeStateParams>(
                        self.runtime_state(&device).await?,
                    ))
                }
                MachineCall::Wake(params) => {
                    let device = DeviceId::parse(params.device_id)?;
                    let op = DeviceOp::Wake {
                        boot: params.boot,
                        machine: params.machine,
                        capacity: params.capacity,
                    };
                    self.run(&device, op).await?;
                    Ok(reply::<WakeParams>(()))
                }
                MachineCall::Hibernate(params) => {
                    let device = DeviceId::parse(params.device_id)?;
                    let op = DeviceOp::Hibernate {
                        retention: Retention::of(params.machine),
                    };
                    self.run(&device, op).await?;
                    Ok(reply::<HibernateParams>(()))
                }
                MachineCall::Checkpoint(params) => {
                    let device = DeviceId::parse(params.device_id)?;
                    let op = DeviceOp::Checkpoint {
                        retention: Retention::of(params.machine),
                    };
                    self.run(&device, op).await?;
                    Ok(reply::<CheckpointParams>(()))
                }
                MachineCall::Reset(params) => {
                    let device = DeviceId::parse(params.device_id)?;
                    let base = BaseVersion::parse(params.base_version)?;
                    let op = DeviceOp::Reset {
                        operation: params.operation_id,
                        base,
                        capacity: params.capacity,
                        retention: Retention::of(params.machine),
                    };
                    self.run(&device, op).await?;
                    Ok(reply::<ResetParams>(()))
                }
            }
        }
    }
}
