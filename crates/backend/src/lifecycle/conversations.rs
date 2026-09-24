//! The conversation idle clock (`resource-lifecycle.md` § Idle window): a
//! conversation that has not been active for the idle window hears the
//! conversation release on every connected paired device it reaches, main
//! or attached. Its watch starts with the conversation's first Host
//! admission and ends once it released; a won target switch starts it
//! again, so a deadline of the old binding never releases the new one; an
//! archive ends it. A Cloud never hears a release: it reclaims everything
//! when it stops, which its own watch decides.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use demi_web_api::ids::ConversationId;
use tokio_util::task::AbortOnDropHandle;

use super::{Activity, IdlePolicy, Retirement};
use crate::conversation::root_of;
use crate::shard::Shard;

/// The idle watch of each conversation that reached a Host.
#[derive(Default)]
pub(crate) struct ConversationWatches {
    watches: RefCell<HashMap<ConversationId, AbortOnDropHandle<()>>>,
}

impl Shard {
    /// What the conversation is doing: a turn of its tree, or an operation
    /// holding its file gate.
    pub(crate) fn conversation_activity(&self, id: &ConversationId) -> Activity {
        let files = Activity::of(&self.conversations().slot(id).files.state());
        match self.agent().tree(&root_of(id)) {
            Some(tree) => files.and(Activity::of(&tree.admission().state())),
            None => files,
        }
    }

    /// Starts the conversation's idle watch unless one runs, as each Host
    /// admission does.
    pub(crate) fn track_idle(&self, id: &ConversationId) {
        if self.is_closing() {
            return;
        }
        let mut watches = self.idle_watches().watches.borrow_mut();
        if watches.get(id).is_some_and(|watch| !watch.is_finished()) {
            return;
        }
        let policy = ConversationIdle {
            shard: Rc::downgrade(&self.this()),
            id: id.clone(),
        };
        let lifecycle = &self.services().lifecycle;
        let watch = super::watch(policy, lifecycle.idle_window, lifecycle.idle_poll, self.tasks().clone());
        watches.insert(id.clone(), AbortOnDropHandle::new(self.tasks().spawn_local(watch)));
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
        self.shard.upgrade().ok_or_else(|| "the shard is gone".to_owned())
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
        let Some(files) = shard.conversations().slot(&self.id).files.try_reserve() else {
            return Ok(None);
        };
        let id = self.id.clone();
        Ok(Some(Box::pin(async move {
            let _held = (files, tree);
            let record = shard.owned_conversation(&id).await.map_err(|error| error.to_string())?;
            shard.release_everywhere(&record).await.map_err(|error| error.to_string())
        })))
    }

    async fn changed(&self) {
        let Ok(shard) = self.shard() else {
            return std::future::pending().await;
        };
        let mut files = shard.conversations().slot(&self.id).files.subscribe();
        // The slot keeps the gate's sender while the shard lives.
        let _ = files.changed().await;
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    use demi_web_api::conversations::ConversationTarget;
    use demi_web_api::ids::{DeviceId, UserId};
    use tokio::time::Instant;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::auth::sessions::TokenHash;
    use crate::backend::Services;
    use crate::shard::{ShardPlacement, ShardPool};
    use crate::storage::control::testing;
    use crate::storage::conversation_index::{AttachedHostRecord, ConversationChange, Creation, RecordChange};

    const ID: &str = "0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a01";
    const HOUR: Duration = Duration::from_secs(60 * 60);
    const MINUTE: Duration = Duration::from_secs(60);

    /// Each release a device's runner answered, and when.
    type Released = Rc<RefCell<Vec<(DeviceId, Instant)>>>;

    /// A backend's services with the conversation `ID` and the master's
    /// paired devices `names`.
    async fn fixture(data: &std::path::Path, names: &[&str]) -> (std::sync::Arc<Services>, UserId, Vec<DeviceId>) {
        let services = Services::start_for_tests(data).await;
        let control = services.control.clone();
        let owner = testing::master(&control).await.id;
        let id = ConversationId::try_from(ID).unwrap();
        assert!(matches!(
            control.create_conversation(owner.clone(), id).await.unwrap(),
            Creation::Created(_)
        ));
        let mut devices = Vec::new();
        for name in names {
            let device = control
                .create_device(owner.clone(), (*name).into(), "linux".into(), TokenHash::of(name))
                .await
                .unwrap();
            devices.push(device.id);
        }
        (services, owner, devices)
    }

    /// Connects each device through a runner the test plays, which records
    /// the releases it answers.
    fn runners(shard: &Rc<Shard>, devices: &[DeviceId]) -> Released {
        let released: Released = Rc::default();
        for device in devices {
            let (log, id) = (released.clone(), device.clone());
            shard.play_runner_for_tests(device, "/home/ana", move |_| {
                let (log, id) = (log.clone(), id.clone());
                Box::pin(async move { log.borrow_mut().push((id, Instant::now())) })
            });
        }
        released
    }

    fn on(device: &DeviceId) -> ConversationChange {
        ConversationChange::Target(ConversationTarget::Device {
            device_id: device.clone(),
            path: "/work".into(),
        })
    }

    #[tokio::test(flavor = "local", start_paused = true)]
    async fn an_hour_without_activity_releases_the_conversation_on_its_main_and_attached_devices() {
        let data = tempfile::tempdir().unwrap();
        let (services, owner, devices) = fixture(data.path(), &["main", "attached"]).await;
        let pool = ShardPool::start(ShardPlacement::Inline, services).await.unwrap();
        pool.shards()
            .of(&owner)
            .call(move |shard, _| async move {
                let id = ConversationId::try_from(ID).unwrap();
                let released = runners(&shard, &devices);
                let [main, attached] = [devices[0].clone(), devices[1].clone()];
                shard.transition(&id, on(&main)).await.unwrap();
                let attach = RecordChange::Attach(AttachedHostRecord {
                    device: attached.clone(),
                    name: "attached".into(),
                    cwd: None,
                });
                shard.transition(&id, attach.into()).await.unwrap();
                let operate = async || {
                    shard
                        .with_host(&id, None, &CancellationToken::new(), async |_| ())
                        .await
                        .unwrap()
                };
                operate().await;
                // Activity just before the deadline restarts a full window.
                tokio::time::sleep(HOUR - MINUTE).await;
                operate().await;
                let active = Instant::now();
                tokio::time::sleep(2 * MINUTE).await;
                assert!(released.borrow().is_empty());
                tokio::time::sleep(HOUR).await;
                let mut released = released.borrow().clone();
                released.sort();
                let devices: Vec<&DeviceId> = released.iter().map(|(device, _)| device).collect();
                let mut expected = vec![&main, &attached];
                expected.sort();
                assert_eq!(devices, expected);
                // Never before the window: the release reads the database,
                // whose own thread lets the paused clock jump to the next
                // timer while it waits, so it lands a few polls after it.
                for (_, at) in &released {
                    assert!(*at - active >= HOUR && *at - active <= HOUR + 5 * MINUTE, "{:?}", *at - active);
                }
                // The devices stay usable.
                operate().await;
            })
            .await
            .unwrap();
        pool.close().await;
    }

    #[tokio::test(flavor = "local", start_paused = true)]
    async fn a_conversation_on_the_cloud_sends_its_cloud_no_release_when_it_idles() {
        let data = tempfile::tempdir().unwrap();
        let (services, owner, _) = fixture(data.path(), &[]).await;
        let cloud = services.control.managed_device_or_create(owner.clone()).await.unwrap().id;
        let pool = ShardPool::start(ShardPlacement::Inline, services).await.unwrap();
        pool.shards()
            .of(&owner)
            .call(move |shard, _| async move {
                let id = ConversationId::try_from(ID).unwrap();
                // The Cloud's runner is connected, and the conversation's
                // watch runs, as a Host admission starts it.
                let released = runners(&shard, &[cloud]);
                shard.track_idle(&id);
                tokio::time::sleep(3 * HOUR).await;
                assert!(released.borrow().is_empty());
            })
            .await
            .unwrap();
        pool.close().await;
    }

    #[tokio::test(flavor = "local", start_paused = true)]
    async fn a_switch_restarts_the_window_so_the_old_deadline_releases_nothing_and_an_archive_ends_it() {
        let data = tempfile::tempdir().unwrap();
        let (services, owner, devices) = fixture(data.path(), &["old", "new"]).await;
        let pool = ShardPool::start(ShardPlacement::Inline, services).await.unwrap();
        pool.shards()
            .of(&owner)
            .call(move |shard, _| async move {
                let id = ConversationId::try_from(ID).unwrap();
                let released = runners(&shard, &devices);
                let [old, new] = [devices[0].clone(), devices[1].clone()];
                shard.transition(&id, on(&old)).await.unwrap();
                shard
                    .with_host(&id, None, &CancellationToken::new(), async |_| ())
                    .await
                    .unwrap();
                tokio::time::sleep(HOUR - MINUTE).await;
                // The switch releases the old device, once.
                shard.transition(&id, on(&new)).await.unwrap();
                tokio::time::sleep(2 * MINUTE).await;
                let devices: Vec<DeviceId> = released.borrow().iter().map(|(device, _)| device.clone()).collect();
                assert_eq!(devices, [old.clone()]);
                // The archive releases the conversation everywhere and ends
                // its watch: no deadline releases it again.
                shard
                    .transition(&id, RecordChange::Archived(true).into())
                    .await
                    .unwrap();
                assert_eq!(released.borrow().len(), 3);
                tokio::time::sleep(3 * HOUR).await;
                assert_eq!(released.borrow().len(), 3);
            })
            .await
            .unwrap();
        pool.close().await;
    }
}
