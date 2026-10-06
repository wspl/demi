//! The conversation idle clock (`resource-lifecycle.md` § Idle window): a
//! conversation that has not been active for the idle window hears the
//! conversation release on every Host it reaches, primary or attached, whose
//! runner is connected: a paired device or a running Cloud. Its watch starts
//! with the conversation's first Host admission and ends once it released;
//! a won target switch starts it again, so a deadline of the old binding
//! never releases the new one; an archive ends it. A release a Host did not
//! take ends the watch as one it took does, since the Host hears it again
//! (`resource-lifecycle.md` § A release that fails). A Host that did not
//! hear a release, because its runner was not connected or its Cloud
//! stopped, is sent none later: the connection's loss or the stop ended
//! what the release would have (`resource-lifecycle.md` § Conversation
//! release).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use demi_backend_idle_watch::{Activity, IdlePolicy, Retirement};
use demi_web_api_protocol::ids::ConversationId;
use tokio_util::task::AbortOnDropHandle;

use crate::shard::Shard;
use demi_backend_host_access::root_of;

/// The idle watch of each conversation that reached a Host.
#[derive(Default)]
pub(crate) struct ConversationWatches {
    watches: RefCell<HashMap<ConversationId, AbortOnDropHandle<()>>>,
}

impl Shard {
    /// What the conversation is doing: a turn of its tree, an operation
    /// holding its file gate, or a user stream someone has open.
    pub(crate) fn conversation_activity(&self, id: &ConversationId) -> Activity {
        let slot = self.conversations().slot(id);
        let host = Activity::of(&slot.file_gate().state()).and(Activity::of(&slot.streams.state()));
        match self.agent().tree(&root_of(id)) {
            Some(tree) => host.and(Activity::of(&tree.admission().state())),
            None => host,
        }
    }

    /// Whether the conversation's idle watch runs: from its first Host
    /// admission until it released the conversation or was ended.
    pub(crate) fn tracks_idle(&self, id: &ConversationId) -> bool {
        self.idle_watches()
            .watches
            .borrow()
            .get(id)
            .is_some_and(|watch| !watch.is_finished())
    }

    /// Starts the conversation's idle watch unless one runs, as each Host
    /// admission does.
    pub(crate) fn track_idle(&self, id: &ConversationId) {
        if self.is_closing() || self.tracks_idle(id) {
            return;
        }
        let policy = ConversationIdle {
            shard: Rc::downgrade(&self.this()),
            id: id.clone(),
        };
        let lifecycle = &self.services().lifecycle;
        let watch = demi_backend_idle_watch::watch(
            policy,
            lifecycle.idle_window,
            lifecycle.idle_poll,
            self.tasks().clone(),
        );
        let watch = AbortOnDropHandle::new(self.tasks().spawn_local(watch));
        self.idle_watches()
            .watches
            .borrow_mut()
            .insert(id.clone(), watch);
    }

    /// Starts the conversation's idle watch again, as a won target switch
    /// does.
    pub(crate) fn restart_idle(&self, id: &ConversationId) {
        self.stop_idle(id);
        self.track_idle(id);
    }

    /// Ends the conversation's idle watch, as an archive does; a release
    /// already running finishes.
    pub(crate) fn stop_idle(&self, id: &ConversationId) {
        self.idle_watches().watches.borrow_mut().remove(id);
    }

    /// Ends every conversation's idle watch, for the shard's close.
    pub(crate) fn stop_idle_watches(&self) {
        self.idle_watches().watches.borrow_mut().clear();
    }
}

/// The idle rule for a conversation on its paired devices.
struct ConversationIdle {
    shard: Weak<Shard>,
    id: ConversationId,
}

impl ConversationIdle {
    fn shard(&self) -> Result<Rc<Shard>, String> {
        self.shard
            .upgrade()
            .ok_or_else(|| "the shard is gone".to_owned())
    }
}

impl IdlePolicy for ConversationIdle {
    async fn check(&self) -> Result<Activity, String> {
        Ok(self.shard()?.conversation_activity(&self.id))
    }

    async fn reserve(&self) -> Result<Option<Retirement>, String> {
        let shard = self.shard()?;
        let tree = match shard.agent().tree(&root_of(&self.id)) {
            Some(tree) => match tree.admission().try_reserve() {
                Some(reservation) => Some(reservation),
                None => return Ok(None),
            },
            None => None,
        };
        let Some(files) = shard
            .conversations()
            .slot(&self.id)
            .file_gate()
            .try_reserve()
        else {
            return Ok(None);
        };
        let id = self.id.clone();
        Ok(Some(Box::pin(async move {
            let _held = (files, tree);
            let record = shard
                .host_shard()
                .owned_conversation(&id)
                .await
                .map_err(|error| error.to_string())?;
            shard
                .host_shard()
                .release_everywhere(&record)
                .await
                .map_err(|error| error.to_string())
        })))
    }

    async fn changed(&self) {
        let Ok(shard) = self.shard() else {
            return std::future::pending().await;
        };
        let mut files = shard.conversations().slot(&self.id).file_gate().subscribe();
        // The slot keeps the gate's sender while the shard lives.
        let _ = files.changed().await;
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;
    use std::time::Duration;

    use demi_web_api_protocol::conversations::ConversationTarget;
    use demi_web_api_protocol::ids::{DeviceId, UserId};
    use tokio::sync::watch;
    use tokio::time::Instant;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::services::Services;
    use crate::shard::PlayedRunner;
    use crate::shard::{ShardPlacement, ShardPool};
    use crate::tuning::LifecycleTuning;
    use demi_backend_database::accounts::TokenHash;
    use demi_backend_database::control::testing;
    use demi_backend_database::conversation_index::{
        AttachedHostRecord, ConversationChange, Creation, RecordChange,
    };
    use demi_runner_protocol::wire::{Outbound, RunnerPlatform};

    const ID: &str = "0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a01";

    /// The idle window, short and in real time: the watch and the Host
    /// access read the database, which answers on threads outside the
    /// runtime, and a paused clock would jump to the watch's next timer
    /// whenever the test waits for them (`concurrency.md` § Tests and time).
    /// A test that shows activity restarting the window acts again half a
    /// window after the first activity, which leaves the half as margin for
    /// the database's answers on a loaded machine.
    const WINDOW: Duration = Duration::from_millis(800);

    /// A guard against a hang, far above any wait here, not a latency bound.
    const HANG: Duration = Duration::from_secs(30);

    /// Each release a device's runner answered, and when.
    type Released = watch::Sender<Vec<(DeviceId, Instant)>>;

    /// A backend's services, whose idle window is `WINDOW`, with the
    /// conversation `ID` and the master's paired devices `names`.
    async fn fixture(
        data: &std::path::Path,
        names: &[&str],
    ) -> (std::sync::Arc<Services>, UserId, Vec<DeviceId>) {
        let lifecycle = LifecycleTuning {
            idle_window: WINDOW,
            idle_poll: Duration::from_millis(50),
            ..LifecycleTuning::default()
        };
        let services = Services::start_for_tests_with_lifecycle(data, lifecycle).await;
        let control = services.control.clone();
        let owner = testing::master(&control).await.id;
        let id = ConversationId::try_from(ID).unwrap();
        assert!(matches!(
            control
                .create_conversation(owner.clone(), id, demi_backend_database::conversation_index::ConversationStart::default())
                .await
                .unwrap(),
            Creation::Created(_)
        ));
        let mut devices = Vec::new();
        for name in names {
            let device = control
                .create_device(
                    owner.clone(),
                    (*name).into(),
                    RunnerPlatform::Linux,
                    TokenHash::of(name),
                )
                .await
                .unwrap();
            devices.push(device.id);
        }
        (services, owner, devices)
    }

    /// Connects each device through a runner the test plays, which records
    /// the releases it answers.
    fn runners(shard: &Rc<Shard>, devices: &[DeviceId]) -> (Released, Vec<PlayedRunner>) {
        let released = Released::new(Vec::new());
        let mut played = Vec::new();
        for device in devices {
            let (log, id) = (released.clone(), device.clone());
            played.push(shard.play_runner_for_tests(device, "/home/ana", move |_| {
                let (log, id) = (log.clone(), id.clone());
                Box::pin(
                    async move { log.send_modify(|released| released.push((id, Instant::now()))) },
                )
            }));
        }
        (released, played)
    }

    /// Waits until the runners answered `count` releases.
    async fn until_released(released: &Released, count: usize) {
        let mut answered = released.subscribe();
        tokio::time::timeout(HANG, answered.wait_for(|released| released.len() >= count))
            .await
            .expect("the releases arrive")
            .expect("the test keeps the log");
    }

    /// Waits until the conversation's idle watch has ended, as it does once
    /// it released the conversation.
    async fn until_retired(shard: &Shard, id: &ConversationId) {
        let deadline = Instant::now() + HANG;
        while shard.tracks_idle(id) {
            assert!(Instant::now() < deadline, "the idle watch never ended");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn on(device: &DeviceId) -> ConversationChange {
        ConversationChange::Target(ConversationTarget::Device {
            device_id: device.clone(),
            path: "/work".into(),
        })
    }

    // A window and a half of real time: the second activity comes half a
    // window after the first, to show that it restarts a full window.
    #[tokio::test(flavor = "local")]
    async fn an_hour_without_activity_releases_the_conversation_on_its_primary_and_attached_devices() {
        let data = tempfile::tempdir().unwrap();
        let (services, owner, devices) = fixture(data.path(), &["primary", "attached"]).await;
        let pool = ShardPool::start(ShardPlacement::Inline, services)
            .await
            .unwrap();
        pool.shards()
            .of(&owner)
            .call(move |shard, _| async move {
                let id = ConversationId::try_from(ID).unwrap();
                let (released, _) = runners(&shard, &devices);
                let [primary, attached] = [devices[0].clone(), devices[1].clone()];
                shard.transition(&id, on(&primary)).await.unwrap();
                let attach = RecordChange::Attach(AttachedHostRecord {
                    device: attached.clone(),
                    name: "attached".into(),
                    cwd: None,
                });
                shard.transition(&id, attach.into()).await.unwrap();
                let operate = async || {
                    shard
                        .host_shard()
                        .with_host(&id, None, &CancellationToken::new(), async |_| ())
                        .await
                        .unwrap()
                };
                operate().await;
                // Activity inside the window restarts a full window.
                tokio::time::sleep(WINDOW / 2).await;
                let active = Instant::now();
                operate().await;
                until_released(&released, 2).await;
                let mut released = released.borrow().clone();
                released.sort();
                let devices: Vec<&DeviceId> = released.iter().map(|(device, _)| device).collect();
                let mut expected = vec![&primary, &attached];
                expected.sort();
                assert_eq!(devices, expected);
                // Never before a full window after the last activity: the
                // first activity's deadline released nothing.
                for (_, at) in &released {
                    assert!(*at - active >= WINDOW, "{:?}", *at - active);
                }
                // The devices stay usable.
                operate().await;
            })
            .await
            .unwrap();
        pool.close().await;
    }

    // Three windows of real time: a direct stream stays open for two, and
    // the release comes a full window after it closed.
    #[tokio::test(flavor = "local")]
    async fn a_direct_stream_keeps_the_conversation_from_its_release_until_it_closes() {
        let data = tempfile::tempdir().unwrap();
        let (services, owner, devices) = fixture(data.path(), &["laptop"]).await;
        let pool = ShardPool::start(ShardPlacement::Inline, services)
            .await
            .unwrap();
        pool.shards()
            .of(&owner)
            .call(move |shard, _| async move {
                let id = ConversationId::try_from(ID).unwrap();
                let (released, played) = runners(&shard, &devices);
                shard.transition(&id, on(&devices[0])).await.unwrap();
                // Only the page's direct stream reaches the device: nothing
                // the backend admitted started the idle watch.
                let stream = |open| Outbound::DirectStream {
                    stream: "direct".to_owned(),
                    conversation: ID.to_owned(),
                    open,
                };
                played[0].say(&stream(true)).await;
                tokio::time::sleep(WINDOW * 2).await;
                assert_eq!(released.borrow().len(), 0, "an open direct stream is activity");
                let closed = Instant::now();
                played[0].say(&stream(false)).await;
                until_released(&released, 1).await;
                let at = released.borrow()[0].1;
                assert!(at - closed >= WINDOW, "{:?}", at - closed);
            })
            .await
            .unwrap();
        pool.close().await;
    }

    // A window and a half of real time: the switch comes half a window after
    // the activity, and the release it leads to a full window after it.
    #[tokio::test(flavor = "local")]
    async fn a_switch_restarts_the_window_so_the_old_deadline_releases_nothing_and_an_archive_ends_it()
     {
        let data = tempfile::tempdir().unwrap();
        let (services, owner, devices) = fixture(data.path(), &["old", "new"]).await;
        let pool = ShardPool::start(ShardPlacement::Inline, services)
            .await
            .unwrap();
        pool.shards()
            .of(&owner)
            .call(move |shard, _| async move {
                let id = ConversationId::try_from(ID).unwrap();
                let (released, _) = runners(&shard, &devices);
                let [old, new] = [devices[0].clone(), devices[1].clone()];
                shard.transition(&id, on(&old)).await.unwrap();
                let operate = async || {
                    shard
                        .host_shard()
                        .with_host(&id, None, &CancellationToken::new(), async |_| ())
                        .await
                        .unwrap()
                };
                operate().await;
                tokio::time::sleep(WINDOW / 2).await;
                // The switch releases the old device, once.
                let switched = Instant::now();
                shard.transition(&id, on(&new)).await.unwrap();
                let devices: Vec<DeviceId> = released
                    .borrow()
                    .iter()
                    .map(|(device, _)| device.clone())
                    .collect();
                assert_eq!(devices, std::slice::from_ref(&old));
                // The old binding's deadline releases nothing: the next
                // releases are the new binding's, on the new device and on
                // the old one, which the switch left attached, a full window
                // after the switch.
                until_released(&released, 3).await;
                until_retired(&shard, &id).await;
                let mut later = released.borrow()[1..].to_vec();
                later.sort();
                let devices: Vec<&DeviceId> = later.iter().map(|(device, _)| device).collect();
                let mut expected = vec![&new, &old];
                expected.sort();
                assert_eq!(devices, expected);
                for (_, at) in &later {
                    assert!(*at - switched >= WINDOW, "{:?}", *at - switched);
                }
                // The archive releases the conversation on both at once and
                // ends the watch the next Host admission started, so no
                // deadline releases it again.
                operate().await;
                assert!(shard.tracks_idle(&id));
                shard
                    .transition(&id, RecordChange::Archived(true).into())
                    .await
                    .unwrap();
                assert_eq!(released.borrow().len(), 5);
                assert!(!shard.tracks_idle(&id));
            })
            .await
            .unwrap();
        pool.close().await;
    }
}
