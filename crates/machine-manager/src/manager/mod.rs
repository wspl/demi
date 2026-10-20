//! The manager's state and its request path (`concurrency.md` § Machine
//! manager): one worker per device runs that device's operations in order,
//! and the manager-wide admission gate lets `reconcile` and shutdown wait for
//! the operations in flight.

pub mod admission;
#[cfg(target_os = "linux")]
mod device;
#[cfg(target_os = "linux")]
mod errors;
#[cfg(all(test, target_os = "linux"))]
mod tests;

#[cfg(target_os = "linux")]
pub use errors::{OpError, Parts};
#[cfg(target_os = "linux")]
pub use linux::{Core, Manager};

#[cfg(target_os = "linux")]
mod linux {
    use std::{cell::RefCell, collections::HashMap, rc::Rc};

    use demi_machine_manager_protocol::{
        BaseVersion, CheckpointParams, CurrentBaseVersionParams, DeviceId, HibernateParams, ImageStateParams, MachineCall, Operation, ReconcileParams, Reconciled,
        ResetParams, RuntimeState, RuntimeStateParams, WakeParams,
    };
    use tokio::{
        sync::{mpsc, oneshot},
        task::JoinSet,
    };

    use super::{
        admission::Admission,
        device::{DeviceCommand, DeviceOp, DeviceWorker},
        errors::{OpError, Parts},
    };
    use crate::{
        blocking,
        config::Config,
        network::{
            CloudNetwork,
            slots::{Slot, SlotPool},
        },
        recovery,
        sandbox::runsc::Runsc,
        server::MachineService,
        storage::store::{ImageStore, Retention},
        tools::Tools,
    };

    /// Commands waiting for one device's worker. A full queue holds back the
    /// request that sends; its admission covers the wait.
    const DEVICE_QUEUE: usize = 64;

    /// What every operation uses and nothing changes after startup.
    pub struct Core {
        pub config: Config,
        pub tools: Tools,
        pub runsc: Runsc,
        pub store: ImageStore,
        pub network: CloudNetwork,
        pub slots: SlotPool,
    }

    impl Core {
        pub fn new(config: Config, tools: Tools) -> Self {
            let runsc = Runsc::new(tools.clone(), config.runtime(), config.limits.is_some());
            let network = CloudNetwork::new(
                config.subnet,
                config.dns.clone(),
                tools.path(crate::tools::Tool::Nft).to_owned(),
            );
            Self {
                store: ImageStore::new(config.images()),
                slots: SlotPool::new(config.subnet, config.slots),
                runsc,
                network,
                tools,
                config,
            }
        }
    }

    /// The manager: its devices' workers and the admission gate.
    pub struct Manager {
        core: Rc<Core>,
        base: BaseVersion,
        admission: Admission,
        devices: RefCell<HashMap<DeviceId, mpsc::Sender<DeviceCommand>>>,
        workers: RefCell<JoinSet<()>>,
        deaths: mpsc::Sender<DeviceId>,
    }

    impl Manager {
        /// A manager whose new devices start on `base` and whose workers
        /// report sandbox deaths to `deaths`.
        pub fn new(core: Rc<Core>, base: BaseVersion, deaths: mpsc::Sender<DeviceId>) -> Self {
            Self {
                core,
                base,
                admission: Admission::new(),
                devices: RefCell::default(),
                workers: RefCell::default(),
                deaths,
            }
        }

        /// The worker of `device`, started on first use.
        fn worker(&self, device: &DeviceId) -> mpsc::Sender<DeviceCommand> {
            let mut devices = self.devices.borrow_mut();
            if let Some(sender) = devices.get(device) {
                return sender.clone();
            }
            let (sender, inbox) = mpsc::channel(DEVICE_QUEUE);
            let worker = DeviceWorker::new(
                device.clone(),
                self.core.clone(),
                self.base.clone(),
                self.deaths.clone(),
            );
            self.workers.borrow_mut().spawn_local(worker.run(inbox));
            devices.insert(device.clone(), sender.clone());
            sender
        }

        /// Runs `op` on `device`'s worker, admitted beside other device
        /// operations. The admission also covers the wait in the queue.
        async fn run(&self, device: &DeviceId, op: DeviceOp) -> Result<(), OpError> {
            let _admitted = self.admission.enter().await?;
            let (reply, answer) = oneshot::channel();
            self.worker(device)
                .send(DeviceCommand::Run { op, reply })
                .await
                .map_err(|_| OpError::WorkerStopped)?;
            answer.await.map_err(|_| OpError::WorkerStopped)?
        }

        async fn runtime_state(&self, device: &DeviceId) -> Result<RuntimeState, OpError> {
            let _admitted = self.admission.enter().await?;
            let (reply, answer) = oneshot::channel();
            self.worker(device)
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
        /// installs the network policy again with the running sandboxes'
        /// slots (`crates/machine-manager-protocol/src/lib.rs`): a
        /// backend's restart stops no Cloud and cuts none off.
        pub async fn reconcile(&self) -> Result<Reconciled, OpError> {
            let _whole = self.admission.exclusive().await?;
            let (running, slots): (Vec<DeviceId>, Vec<Slot>) =
                self.running_devices().await.into_iter().unzip();
            recovery::fence_and_save(&self.core, &running).await?;
            self.core.network.prepare(&slots).await?;
            Ok(Reconciled {
                running: running.iter().map(|device| device.as_str().to_owned()).collect(),
            })
        }

        /// The devices whose workers run a sandbox, with its slot; the
        /// caller holds the whole gate, so no worker is running an
        /// operation.
        async fn running_devices(&self) -> Vec<(DeviceId, Slot)> {
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
            let _whole = self.admission.exclusive().await?;
            let drained = self.drain().await;
            self.admission.close();
            drained
        }

        /// Hibernates every known device at once; the caller holds the
        /// whole gate, so no worker is running an operation.
        async fn drain(&self) -> Result<(), OpError> {
            let workers: Vec<_> = self.devices.borrow().values().cloned().collect();
            let outcomes =
                futures_util::future::join_all(workers.into_iter().map(|worker| async move {
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
                    answer.await.map_err(|_| OpError::WorkerStopped)?
                }))
                .await;
            let failures: Vec<OpError> = outcomes.into_iter().filter_map(Result::err).collect();
            if !failures.is_empty() {
                return Err(OpError::Shutdown(Parts(failures)));
            }
            Ok(())
        }
    }

    /// The result an `ok` reply carries for operation `O`.
    fn reply<O: Operation>(output: O::Output) -> serde_json::Value {
        serde_json::to_value(output).expect("an operation's result encodes as JSON")
    }

    impl MachineService for Manager {
        type Error = OpError;

        async fn handle(&self, call: MachineCall) -> Result<serde_json::Value, OpError> {
            match call {
                MachineCall::Reconcile(_) => Ok(reply::<ReconcileParams>(self.reconcile().await?)),
                MachineCall::CurrentBaseVersion(_) => {
                    Ok(reply::<CurrentBaseVersionParams>(self.base.clone()))
                }
                MachineCall::ImageState(params) => {
                    let device = DeviceId::parse(params.device_id)?;
                    let store = self.core.store.clone();
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
