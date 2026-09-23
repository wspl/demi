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
pub use linux::{Core, Manager};
#[cfg(target_os = "linux")]
pub use errors::{OpError, Parts};

#[cfg(target_os = "linux")]
mod linux {
    use std::{cell::RefCell, collections::HashMap, rc::Rc};

    use demi_machines_protocol::{
        BaseVersion, CheckpointParams, CurrentBaseVersionParams, DeviceId, GrowVolumeParams, HibernateParams,
        ImageStateParams, MachineCall, Operation, ReconcileParams, ResetParams, RuntimeState, RuntimeStateParams,
        WakeParams,
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
        blocking, config::Config, network::CloudNetwork, network::slots::SlotPool, recovery, sandbox::runsc::Runsc,
        server::MachineService, storage::store::ImageStore, tools::Tools,
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
            let runsc = Runsc::new(tools.clone(), config.runtime());
            let network = CloudNetwork::new(
                config.subnet,
                config.dns.clone(),
                config.backend_url.clone(),
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
            let worker = DeviceWorker::new(device.clone(), self.core.clone(), self.base.clone(), self.deaths.clone());
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
                .send(DeviceCommand::RuntimeState { reply })
                .await
                .map_err(|_| OpError::WorkerStopped)?;
            answer.await.map_err(|_| OpError::WorkerStopped)
        }

        /// Stops and saves every device, recovers what an interrupted
        /// operation left, and installs the network policy again.
        pub async fn reconcile(&self) -> Result<(), OpError> {
            let _whole = self.admission.exclusive().await?;
            self.drain().await?;
            recovery::fence_and_save(&self.core).await?;
            self.core.network.prepare().await?;
            Ok(())
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
            let outcomes = futures_util::future::join_all(workers.into_iter().map(|worker| async move {
                let (reply, answer) = oneshot::channel();
                let op = DeviceOp::Hibernate;
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
                MachineCall::Reconcile(_) => {
                    self.reconcile().await?;
                    Ok(reply::<ReconcileParams>(()))
                }
                MachineCall::CurrentBaseVersion(_) => Ok(reply::<CurrentBaseVersionParams>(self.base.clone())),
                MachineCall::ImageState(params) => {
                    let device = DeviceId::parse(params.device_id)?;
                    let store = self.core.store.clone();
                    let state = blocking::run(move |off| store.read(off, &device)).await?;
                    Ok(reply::<ImageStateParams>(state))
                }
                MachineCall::RuntimeState(params) => {
                    let device = DeviceId::parse(params.device_id)?;
                    Ok(reply::<RuntimeStateParams>(self.runtime_state(&device).await?))
                }
                MachineCall::Wake(params) => {
                    let device = DeviceId::parse(params.device_id)?;
                    self.run(&device, DeviceOp::Wake(params.boot)).await?;
                    Ok(reply::<WakeParams>(()))
                }
                MachineCall::Hibernate(params) => {
                    let device = DeviceId::parse(params.device_id)?;
                    self.run(&device, DeviceOp::Hibernate).await?;
                    Ok(reply::<HibernateParams>(()))
                }
                MachineCall::Checkpoint(params) => {
                    let device = DeviceId::parse(params.device_id)?;
                    self.run(&device, DeviceOp::Checkpoint).await?;
                    Ok(reply::<CheckpointParams>(()))
                }
                MachineCall::GrowVolume(params) => {
                    let device = DeviceId::parse(params.device_id)?;
                    self.run(&device, DeviceOp::Grow(params.volume, params.bytes)).await?;
                    Ok(reply::<GrowVolumeParams>(()))
                }
                MachineCall::Reset(params) => {
                    let device = DeviceId::parse(params.device_id)?;
                    let base = BaseVersion::parse(params.base_version)?;
                    let op = DeviceOp::Reset {
                        operation: params.operation_id,
                        base,
                    };
                    self.run(&device, op).await?;
                    Ok(reply::<ResetParams>(()))
                }
            }
        }
    }
}
