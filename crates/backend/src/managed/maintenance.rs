//! The schedules of a running Cloud (`managed-hosts.md` § Lifecycle and
//! capacity): its idle watch, which stops it once no conversation using it
//! has been active for the idle window (`resource-lifecycle.md`), and its
//! maintenance loop, which saves it every checkpoint interval and stops it
//! at its lifetime cap. Both belong to the running phase and end with it.
//! Maintenance holds the Cloud's gate for maintenance, not demand, so it
//! never restarts the idle clock; an idle stop that finds maintenance
//! holding the gate waits for it to let go.

use std::rc::{Rc, Weak};

use demi_gates::{GateLease, Purpose};
use demi_machines_protocol::CheckpointParams;
use tokio::time::Instant;
use tokio_util::task::AbortOnDropHandle;

use super::machine::Machine;
use crate::conversation::root_of;
use crate::lifecycle::{self, Activity, IdlePolicy, Retirement};
use crate::shard::Shard;

impl Shard {
    /// Starts the idle watch and the maintenance loop of a machine that just
    /// started running.
    pub(super) fn cloud_schedules(&self, machine: &Rc<Machine>) -> Vec<AbortOnDropHandle<()>> {
        let window = self.services().lifecycle.idle_window;
        let sweep = self.services().cloud.tuning.sweep;
        let idle = CloudIdle {
            shard: Rc::downgrade(&self.this()),
            machine: machine.clone(),
        };
        let watch = lifecycle::watch(idle, window, sweep, self.tasks().clone());
        let maintenance = maintain(Rc::downgrade(&self.this()), machine.clone());
        vec![
            AbortOnDropHandle::new(self.tasks().spawn_local(watch)),
            AbortOnDropHandle::new(self.tasks().spawn_local(maintenance)),
        ]
    }

    /// What the conversations using the Cloud, in any role, are doing: a
    /// turn of their tree, or an operation holding their file gate, keeps
    /// the Cloud awake.
    async fn cloud_conversations_activity(&self) -> Result<Activity, String> {
        let uses = self.cloud_uses().await.map_err(|error| error.to_string())?;
        Ok(uses
            .iter()
            .fold(Activity::default(), |activity, (id, _)| activity.and(self.conversation_activity(id))))
    }

    /// One maintenance round of a running machine: the lifetime cap, else a
    /// checkpoint once the interval passed. Nothing runs beside a transition
    /// or another round's checkpoint.
    async fn maintain_cloud(&self, machine: &Rc<Machine>) {
        if self.cloud().is_closed() || machine.transition().is_some() || machine.gate.state().maintenance > 0 {
            return;
        }
        let Some((started_at, checkpoint_at)) = machine.running_times() else {
            return;
        };
        let tuning = &self.services().cloud.tuning;
        let now = Instant::now();
        if now.duration_since(started_at) >= tuning.lifetime_cap {
            self.stop_at_lifetime_cap(machine).await;
        } else if now.duration_since(checkpoint_at) >= tuning.checkpoint_interval {
            let Some(lease) = machine.gate.try_enter(Purpose::Maintenance) else {
                return;
            };
            // A task of its own, so the machine leaving the running phase
            // does not cut the checkpoint short.
            let shard = self.this();
            let machine = machine.clone();
            self.tasks().spawn_local(async move { shard.checkpoint(&machine, lease, now).await });
        }
    }

    /// Publishes the running machine's disks, keeping its processes
    /// (`managed-hosts.md` § Save a generation), with its runner's liveness
    /// check paused for as long as the manager holds the sandbox.
    async fn checkpoint(&self, machine: &Machine, lease: GateLease, at: Instant) {
        let device = &machine.device.id;
        let link = self.devices().link(device);
        if let Some(link) = &link {
            link.pause_liveness();
        }
        let saved = self
            .services()
            .cloud
            .machines
            .call(CheckpointParams {
                device_id: device.to_string(),
            })
            .await;
        if let Some(link) = &link {
            link.resume_liveness();
        }
        drop(lease);
        match saved {
            Ok(()) => machine.checkpointed(at),
            // The next round tries again; a sandbox the failure stopped
            // reports its death.
            Err(error) => tracing::warn!(device = %device, "the Cloud's checkpoint failed: {error}"),
        }
    }

    /// Stops a machine that ran for its lifetime cap (`managed-hosts.md`
    /// § Lifecycle and capacity). A turn in flight postpones it, and so does
    /// a file transfer or a stream the user has open. Otherwise new
    /// operations are held back, the jobs nothing attends end with the
    /// runner's connection, and once their leases are released the machine
    /// is saved and stopped as an idle stop would stop it.
    async fn stop_at_lifetime_cap(&self, machine: &Rc<Machine>) {
        let uses = match self.cloud_uses().await {
            Ok(uses) => uses,
            Err(error) => {
                tracing::warn!("the Cloud's lifetime cap could not read its conversations: {error}");
                return;
            }
        };
        let attended = uses.iter().any(|(id, _)| {
            let turning = self
                .agent()
                .tree(&root_of(id))
                .is_some_and(|tree| tree.admission().state().demand > 0);
            turning || self.conversations().slot(id).transfers.any_open()
        });
        if attended {
            return;
        }
        let device = machine.device.id.clone();
        let hold = self.services().cloud.tuning.reset_hold;
        let reserving = machine.gate.reserve();
        tokio::pin!(reserving);
        // The reservation waits before the jobs end: nothing new slips in
        // while their leases drain.
        let reserved = match futures_util::future::poll_immediate(reserving.as_mut()).await {
            Some(reserved) => reserved,
            None => {
                self.devices().disconnect(&device, "Cloud reached its lifetime cap");
                match tokio::time::timeout(hold, reserving).await {
                    Ok(reserved) => reserved,
                    Err(_) => {
                        tracing::warn!(device = %device, "the Cloud's jobs did not end at its lifetime cap");
                        return;
                    }
                }
            }
        };
        let Some(held) = self.hold_uses_for_idle(&uses) else {
            // A conversation is changing; the next round tries again.
            return;
        };
        // A task of its own: leaving the running phase ends this loop.
        let shard = self.this();
        let machine = machine.clone();
        self.tasks().spawn_local(async move {
            let _held = (reserved, held);
            if let Err(error) = shard.hibernate_reserved(&machine).await {
                tracing::warn!(device = %machine.device.id, "the Cloud was not saved at its lifetime cap: {error}");
            }
        });
    }
}

/// The idle rule for the Cloud (`resource-lifecycle.md` § Idle window): no
/// conversation using it, in any role, active within the window, no job
/// running on it and no operation holding it.
struct CloudIdle {
    shard: Weak<Shard>,
    machine: Rc<Machine>,
}

impl CloudIdle {
    fn shard(&self) -> Result<Rc<Shard>, String> {
        self.shard.upgrade().ok_or_else(|| "the shard is gone".to_owned())
    }
}

impl IdlePolicy for CloudIdle {
    async fn check(&self) -> Result<Activity, String> {
        let shard = self.shard()?;
        let machine = &self.machine;
        let mut activity = Activity::of(&machine.gate.state());
        activity.busy |= !machine.is_running() || machine.transition().is_some();
        let jobs = shard
            .devices()
            .link(&machine.device.id)
            .map_or(0, |link| link.running_jobs());
        activity.busy |= jobs > 0;
        Ok(activity.and(shard.cloud_conversations_activity().await?))
    }

    async fn reserve(&self) -> Result<Option<Retirement>, String> {
        let shard = self.shard()?;
        let Some(gate) = self.machine.gate.try_reserve() else {
            return Ok(None);
        };
        let uses = shard.cloud_uses().await.map_err(|error| error.to_string())?;
        let Some(held) = shard.hold_uses_for_idle(&uses) else {
            return Ok(None);
        };
        let machine = self.machine.clone();
        Ok(Some(Box::pin(async move {
            let _held = (gate, held);
            shard.hibernate_reserved(&machine).await.map_err(|error| error.to_string())
        })))
    }

    async fn changed(&self) {
        let mut gate = self.machine.gate.subscribe();
        // The machine holds the gate's sender while its watch runs.
        let _ = gate.changed().await;
    }
}

/// The maintenance loop of a running machine: a round every sweep.
async fn maintain(shard: Weak<Shard>, machine: Rc<Machine>) {
    let Some(sweep) = shard.upgrade().map(|shard| shard.services().cloud.tuning.sweep) else {
        return;
    };
    let mut rounds = tokio::time::interval_at(Instant::now() + sweep, sweep);
    rounds.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        rounds.tick().await;
        let Some(shard) = shard.upgrade() else {
            return;
        };
        shard.maintain_cloud(&machine).await;
    }
}
