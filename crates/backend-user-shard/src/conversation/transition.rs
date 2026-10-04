//! Changes of a conversation (`web-api.md` § Sidebar mutations, read state
//! and page synchronization; `sessions-and-targets.md` § Switch the primary
//! target, § Lifecycle access). Every change goes through one entry,
//! `Shard::transition`. A rename, a pin or a change of the model settings
//! waits while a transition holds the conversation, and is then applied as if
//! it arrived afterwards; the model settings change one at a time
//! (`settings.rs`). An archive, a restore, a target switch and a detach are
//! transitions: each holds the conversation, with its idle tree reserved, its
//! file transfers, user streams and the one-shot calls admitted like them
//! ended, and its file gate reserved; sends the conversation release to the
//! devices it leaves; and commits, advancing the execution-context revision
//! when the context changed, whether the devices took the release or not
//! (`resource-lifecycle.md` § A release that fails). A transition waits for
//! no other work: a conversation that is busy refuses it. The shard reserves
//! the idle tree; host access holds the rest and runs the transition's steps
//! (`demi_backend_host_access::transition`).

use demi_backend_database::StorageError;
use demi_backend_database::conversation_index::{ConversationChange, RecordChange, SettingsChange};
use demi_backend_host_access::root_of;
use demi_backend_host_access::transition::ChangeRefusal;
use demi_backend_page_sync::Part;
use demi_shared_gates::{Purpose, Reservation};
use demi_web_api_protocol::conversations::{
    ConversationPatch, ConversationUpdate, FieldResult, PatchField,
};
use demi_web_api_protocol::ids::ConversationId;

use crate::shard::Shard;

/// Whether a change of the record is a transition, which holds the
/// conversation: an archive, a restore and a detach. A target switch is one
/// too.
fn change_is_transition(change: &RecordChange) -> bool {
    matches!(change, RecordChange::Archived(_) | RecordChange::Detach(_))
}

impl Shard {
    /// Applies `change` to the user's conversation `id`, and shows it on the
    /// user's pages: the conversation's summary, and the order after a pin
    /// or an archive.
    pub async fn transition(
        &self,
        id: &ConversationId,
        change: ConversationChange,
    ) -> Result<(), ChangeRefusal> {
        let reorders = matches!(
            change,
            ConversationChange::Record(RecordChange::Pinned(_) | RecordChange::Archived(_))
        );
        self.apply_change(id, change).await?;
        self.mark(Part::Conversation(id.clone()));
        if reorders {
            self.mark(Part::ConversationOrder);
        }
        Ok(())
    }

    async fn apply_change(
        &self,
        id: &ConversationId,
        change: ConversationChange,
    ) -> Result<(), ChangeRefusal> {
        let host = self.host_shard();
        let record = self.services().control.conversation(id.clone()).await?;
        let Some(record) = record.filter(|record| record.owner == *self.user()) else {
            return Err(ChangeRefusal::NotFound);
        };
        // An archived conversation takes nothing but its restore; the index
        // transaction checks again when it applies the change.
        if record.archived
            && !matches!(
                change,
                ConversationChange::Record(RecordChange::Archived(_))
            )
        {
            return Err(ChangeRefusal::Archived);
        }
        match &change {
            // The conversation's own target is no change.
            ConversationChange::Target(to) if record.target == *to => return Ok(()),
            ConversationChange::Target(to) => host.check_destination(&record, to).await?,
            _ => {}
        }
        let change = match change {
            ConversationChange::Settings(change) => {
                // A settings change waits while a transition holds the
                // conversation, then takes its turn after the changes and
                // opens that came first.
                let slot = self.conversations().slot(&record.id);
                let _admitted = slot.file_gate().enter(Purpose::Demand).await;
                let _turn = slot.settings.acquire().await;
                return self.change_settings(&record.id, change).await;
            }
            ConversationChange::Record(change) if !change_is_transition(&change) => {
                // A field update waits while a transition holds the
                // conversation, and applies as if it arrived afterwards.
                let _admitted = self
                    .conversations()
                    .slot(&record.id)
                    .file_gate()
                    .enter(Purpose::Demand)
                    .await;
                if let RecordChange::Attach(attached) = &change {
                    let target = host.resolve_target(&record).await?;
                    if target.device() == Some(&attached.device) {
                        return Err(ChangeRefusal::HostIsPrimary);
                    }
                }
                return host.commit(&record.id, change).await;
            }
            change => change,
        };
        let tree = self.reserve_idle_tree(&record.id)?;
        let hold = host.hold_for_transition(&record.id, tree).await?;
        let committed = match change {
            ConversationChange::Target(to) => {
                let switched = host.switch_target(&record, to).await;
                // A deadline of the old binding never releases the new one.
                if switched.is_ok() {
                    self.restart_idle(&record.id);
                }
                switched
            }
            ConversationChange::Record(RecordChange::Archived(true)) => {
                let archived = host.archive(&record).await;
                // An archived conversation's title request and idle watch
                // end.
                if archived.is_ok() {
                    self.titles().abort(&record.id);
                    self.stop_idle(&record.id);
                }
                archived
            }
            ConversationChange::Record(RecordChange::Detach(device)) => {
                host.detach(&record, device).await
            }
            ConversationChange::Record(change) => host.commit(&record.id, change).await,
            ConversationChange::Settings(_) => {
                unreachable!("a settings change is applied before the hold")
            }
        };
        drop(hold);
        committed
    }

    /// Reserves the conversation's live tree while it does nothing by itself
    /// (`runtime.md` § Actions): no action runs or waits, no child is live
    /// and no wakeup is scheduled. A conversation without a live tree runs
    /// nothing; one whose tree works refuses the transition.
    fn reserve_idle_tree(&self, id: &ConversationId) -> Result<Option<Reservation>, ChangeRefusal> {
        let Some(tree) = self.agent().tree(&root_of(id)) else {
            return Ok(None);
        };
        // The check and the reservation are one step: no await between them.
        if !tree.is_quiescent() {
            return Err(ChangeRefusal::TurnInFlight);
        }
        tree.admission()
            .try_reserve()
            .map(Some)
            .ok_or(ChangeRefusal::TurnInFlight)
    }
}

impl Shard {
    /// Applies each field of `patch` to the user's conversation `id` on its
    /// own, the archive first, so archiving and renaming together archives
    /// and refuses the rename; a field applied stays applied whatever the
    /// others do. None when the user has no such conversation.
    pub async fn apply_patch(
        &self,
        id: &ConversationId,
        patch: ConversationPatch,
    ) -> Result<Option<ConversationUpdate>, StorageError> {
        let control = &self.services().control;
        let owned = control
            .conversation(id.clone())
            .await?
            .filter(|record| record.owner == *self.user());
        if owned.is_none() {
            return Ok(None);
        }
        let mut changes: Vec<(Vec<PatchField>, ConversationChange)> = Vec::new();
        if let Some(archived) = patch.archived {
            changes.push((
                vec![PatchField::Archived],
                RecordChange::Archived(archived).into(),
            ));
        }
        if let Some(title) = patch.title {
            changes.push((
                vec![PatchField::Title],
                RecordChange::Title(title.into_string()).into(),
            ));
        }
        if let Some(pinned) = patch.pinned {
            changes.push((
                vec![PatchField::Pinned],
                RecordChange::Pinned(pinned).into(),
            ));
        }
        // The model settings a patch names are one change, which each of its
        // fields reports.
        let settings_fields: Vec<PatchField> = [
            (patch.model.is_some(), PatchField::Model),
            (patch.thinking_effort.is_some(), PatchField::ThinkingEffort),
            (patch.service_tier_id.is_some(), PatchField::ServiceTierId),
        ]
        .into_iter()
        .filter_map(|(named, field)| named.then_some(field))
        .collect();
        if !settings_fields.is_empty() {
            let change = SettingsChange {
                model: patch.model,
                thinking_effort: patch.thinking_effort,
                service_tier_id: patch.service_tier_id,
            };
            changes.push((settings_fields, ConversationChange::Settings(change)));
        }
        if let Some(target) = patch.target {
            changes.push((vec![PatchField::Target], ConversationChange::Target(target)));
        }
        let mut results = Vec::new();
        for (fields, change) in changes {
            let outcome = self.transition(id, change).await;
            for field in fields {
                let result = match &outcome {
                    Ok(()) => FieldResult::Applied { field },
                    Err(refusal) => failed(field, refusal),
                };
                results.push(result);
            }
        }
        let Some(record) = control.conversation(id.clone()).await? else {
            return Ok(None);
        };
        let conversation = self.conversation_summary(record).await?;
        Ok(Some(ConversationUpdate {
            conversation,
            results,
        }))
    }
}

fn failed(field: PatchField, refusal: &ChangeRefusal) -> FieldResult {
    if let ChangeRefusal::Storage(error) = refusal {
        tracing::error!(
            ?field,
            error = error as &dyn std::error::Error,
            "a conversation change failed"
        );
    }
    let (code, http_status) = refusal.code();
    FieldResult::Failed {
        field,
        code,
        message: refusal.to_string(),
        http_status,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use demi_web_api_protocol::conversations::ConversationTarget;
    use demi_web_api_protocol::error::ErrorCode;
    use demi_web_api_protocol::ids::{DeviceId, UserId};

    use super::*;
    use crate::services::Services;
    use crate::shard::{ShardPlacement, ShardPool};
    use demi_backend_database::accounts::TokenHash;
    use demi_backend_database::control::testing;
    use demi_backend_database::conversation_index::{AttachedHostRecord, Creation};
    use demi_runner_protocol::wire::RunnerPlatform;

    const ID: &str = "0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a01";

    #[tokio::test(flavor = "local")]
    async fn a_transition_waits_for_no_work_and_a_field_update_waits_for_a_transition() {
        let data = tempfile::tempdir().unwrap();
        let services = Services::start_for_tests(data.path()).await;
        let control = services.control.clone();
        let owner: UserId = testing::master(&control).await.id;
        let id = ConversationId::try_from(ID).unwrap();
        assert!(matches!(
            control
                .create_conversation(owner.clone(), id.clone())
                .await
                .unwrap(),
            Creation::Created(_)
        ));
        let laptop = control
            .create_device(
                owner.clone(),
                "laptop".into(),
                RunnerPlatform::Linux,
                TokenHash::of("laptop"),
            )
            .await
            .unwrap()
            .id;
        let pool = ShardPool::start(ShardPlacement::Inline, services)
            .await
            .unwrap();
        let refusals = pool
            .shards()
            .of(&owner)
            .call(move |shard, _| async move {
                let slot = shard.conversations().slot(&id);
                let to = ConversationTarget::Device {
                    device_id: laptop.clone(),
                    path: "/work".into(),
                };
                // A Host operation in flight: a transition refuses rather
                // than wait for it.
                let operation = slot.file_gate().enter(Purpose::Demand).await;
                let mut refusals = Vec::new();
                for change in [
                    ConversationChange::Target(to.clone()),
                    RecordChange::Archived(true).into(),
                    RecordChange::Detach(laptop.clone()).into(),
                ] {
                    refusals.push(
                        shard
                            .transition(&id, change)
                            .await
                            .map_err(|refusal| refusal.code().0),
                    );
                }
                drop(operation);
                // A transition holds the conversation: a rename waits for it,
                // then applies. The rename reads the conversation first, and
                // the control database answers its one connection's calls in
                // order: once a read of the test's after it has come back and
                // the test has yielded, the rename has gone as far as it can,
                // to the gate or through its commit, which a second read would
                // then see.
                let held = slot.file_gate().try_reserve().unwrap();
                let renaming = {
                    let shard = shard.clone();
                    let id = id.clone();
                    tokio::task::spawn_local(async move {
                        shard
                            .transition(&id, RecordChange::Title("Renamed".into()).into())
                            .await
                    })
                };
                let control = &shard.services().control;
                tokio::task::yield_now().await;
                control.conversation(id.clone()).await.unwrap();
                tokio::task::yield_now().await;
                let held_title = control
                    .conversation(id.clone())
                    .await
                    .unwrap()
                    .unwrap()
                    .title;
                let waited = !renaming.is_finished();
                drop(held);
                renaming.await.unwrap().unwrap();
                let record = shard.host_shard().owned_conversation(&id).await.unwrap();
                (refusals, waited, held_title, record.title)
            })
            .await
            .unwrap();
        let (refusals, waited, held_title, title) = refusals;
        assert_eq!(refusals, [Err(ErrorCode::TurnInFlight); 3]);
        assert!(waited);
        assert_ne!(
            held_title, "Renamed",
            "the rename applied while a transition held the conversation"
        );
        assert_eq!(title, "Renamed");
        pool.close().await;
    }

    /// A conversation release as a device's runner received it, with the
    /// conversation's binding at that moment.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Released {
        device: DeviceId,
        target: ConversationTarget,
        attached: Vec<DeviceId>,
        archived: bool,
    }

    /// Connects `device` through a runner the test plays, which answers
    /// every conversation release and writes it to `log`.
    fn runner(shard: &Rc<Shard>, device: &DeviceId, log: &Rc<RefCell<Vec<Released>>>) {
        let control = shard.services().control.clone();
        let device_id = device.clone();
        let log = log.clone();
        shard.play_runner_for_tests(device, "/home/ana", move |conversation| {
            let (control, device, log) = (control.clone(), device_id.clone(), log.clone());
            Box::pin(async move {
                let record = control
                    .conversation(conversation.clone())
                    .await
                    .unwrap()
                    .unwrap();
                let attached = control.attached_hosts(conversation).await.unwrap();
                log.borrow_mut().push(Released {
                    device,
                    target: record.target,
                    attached: attached.into_iter().map(|host| host.device).collect(),
                    archived: record.archived,
                });
            })
        });
    }

    #[tokio::test(flavor = "local")]
    async fn a_switch_a_detach_and_an_archive_release_the_conversation_before_they_change_its_binding()
     {
        let data = tempfile::tempdir().unwrap();
        let services = Services::start_for_tests(data.path()).await;
        let control = services.control.clone();
        let owner: UserId = testing::master(&control).await.id;
        let id = ConversationId::try_from(ID).unwrap();
        assert!(matches!(
            control
                .create_conversation(owner.clone(), id.clone())
                .await
                .unwrap(),
            Creation::Created(_)
        ));
        let mut devices = Vec::new();
        for name in ["one", "two"] {
            let device = control
                .create_device(
                    owner.clone(),
                    name.into(),
                    RunnerPlatform::Linux,
                    TokenHash::of(name),
                )
                .await
                .unwrap();
            devices.push(device.id);
        }
        let pool = ShardPool::start(ShardPlacement::Inline, services)
            .await
            .unwrap();
        pool.shards()
            .of(&owner)
            .call(move |shard, _| async move {
                let log = Rc::new(RefCell::new(Vec::new()));
                let [one, two] = [devices[0].clone(), devices[1].clone()];
                runner(&shard, &one, &log);
                runner(&shard, &two, &log);
                let on = |device: &DeviceId| ConversationTarget::Device {
                    device_id: device.clone(),
                    path: "/work".into(),
                };
                // From the Cloud, which was never made, nothing is released.
                shard
                    .transition(&id, ConversationChange::Target(on(&one)))
                    .await
                    .unwrap();
                assert!(log.borrow().is_empty());
                let released = |device: &DeviceId,
                                target: ConversationTarget,
                                attached: &[DeviceId],
                                archived| Released {
                    device: device.clone(),
                    target,
                    attached: attached.to_vec(),
                    archived,
                };
                shard
                    .transition(&id, ConversationChange::Target(on(&two)))
                    .await
                    .unwrap();
                assert_eq!(*log.borrow(), [released(&one, on(&one), &[], false)]);
                shard
                    .transition(&id, RecordChange::Detach(one.clone()).into())
                    .await
                    .unwrap();
                assert_eq!(
                    log.borrow()[1],
                    released(&one, on(&two), std::slice::from_ref(&one), false)
                );
                // An archive releases it on the primary Host and the attached
                // ones.
                let attach = RecordChange::Attach(AttachedHostRecord {
                    device: one.clone(),
                    name: "one".into(),
                    cwd: None,
                });
                shard.transition(&id, attach.into()).await.unwrap();
                shard
                    .transition(&id, RecordChange::Archived(true).into())
                    .await
                    .unwrap();
                let attached = [one.clone()];
                assert_eq!(
                    log.borrow()[2..],
                    [
                        released(&two, on(&two), &attached, false),
                        released(&one, on(&two), &attached, false)
                    ]
                );
            })
            .await
            .unwrap();
        pool.close().await;
    }
}
