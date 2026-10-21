//! Changes of a conversation (`crates/backend-host-access/src/transition.rs`, §
//! Lifecycle access).
//!
//! Every change goes through one entry, `Shard::transition`.
//!
//! - A rename, a pin or a change of the model settings waits while a transition
//!   holds the conversation, and is then applied as if it arrived afterwards.
//!   The model settings change one at a time (`settings.rs`).
//! - An archive, a restore and a detach are transitions. Each holds the
//!   conversation, with its idle tree reserved, its file transfers, user
//!   streams and the one-shot calls admitted like them ended, and its file gate
//!   reserved. Each sends the conversation release to the devices it leaves,
//!   and commits, whether the devices took the release or not
//!   (`crates/backend-user-shard/src/lifecycle/conversations.rs`).
//! - A move stops and waits for nothing
//!   (`docs/decisions/0027-a-move-applies-at-once.md`): it checks its
//!   destination, ends the transfers and streams, and commits, advancing the
//!   execution-context revision, so each agent's next request names the new
//!   place. The commands running then finish where they started; the device
//!   it left hears the release once they ended. Each move logs one line with
//!   its id, its Hosts, how long each step took and its outcome.
//!
//! A transition waits for no other work: a conversation that is busy refuses
//! it. The shard reserves the idle tree; host access holds the rest and runs the
//! transition's steps (`demi_backend_host_access::transition`).

use demi_backend_database::workspaces::WorkspaceDeletion;
use demi_backend_database::{Selection, StorageError};
use demi_backend_database::conversation_index::{
    ConversationChange, ConversationRecord, RecordChange, SettingsChange,
};
use demi_backend_host_access::root_of;
use demi_backend_host_access::transition::{ChangeRefusal, Switched};
use demi_backend_page_sync::Part;
use demi_shared_gates::{Purpose, Reservation};
use demi_web_api_protocol::conversations::{
    ConversationPatch, ConversationTarget, ConversationUpdate, FieldResult, PatchField,
};
use demi_web_api_protocol::devices::DeviceKind;
use demi_web_api_protocol::ids::{ConversationId, DeviceId, WorkspaceId};
use std::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::shard::Shard;

/// How long each step of a move took, in milliseconds, as its log line says.
#[derive(Default)]
struct MoveSteps {
    check: u64,
    transfers: u64,
    commit: u64,
}

/// Whether a change of the record is a transition, which holds the
/// conversation: an archive, a restore and a detach.
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
        self.transition_switching(id, change, &CancellationToken::new()).await.map(drop)
    }

    /// Moves the user's conversation `id` to `to` through `transition`,
    /// answering the move made; none when it runs there already. `cancel`
    /// ends the move until its commit starts, which it then does not
    /// interrupt.
    pub(crate) async fn move_to(
        &self,
        id: &ConversationId,
        to: ConversationTarget,
        cancel: &CancellationToken,
    ) -> Result<Option<Switched>, ChangeRefusal> {
        self.transition_switching(id, ConversationChange::Target(to), cancel).await
    }

    /// `transition`, answering the switch it made when `change` is a target
    /// switch that moved the conversation.
    async fn transition_switching(
        &self,
        id: &ConversationId,
        change: ConversationChange,
        cancel: &CancellationToken,
    ) -> Result<Option<Switched>, ChangeRefusal> {
        let reorders = matches!(
            change,
            ConversationChange::Record(RecordChange::Pinned(_) | RecordChange::Archived(_))
        );
        // A change of what the sidebar shows goes to every page; one of the
        // metadata alone, which the sidebar shows none of, only to the pages
        // that show the conversation.
        let listed = matches!(
            change,
            ConversationChange::Target(_)
                | ConversationChange::Record(RecordChange::Title(_) | RecordChange::Pinned(_) | RecordChange::Archived(_))
        );
        let switched = self.apply_change(id, change, cancel).await?;
        self.mark(if listed { Part::Conversation(id.clone()) } else { Part::Metadata(id.clone()) });
        if reorders {
            self.mark(Part::ConversationOrder);
        }
        Ok(switched)
    }

    async fn apply_change(
        &self,
        id: &ConversationId,
        change: ConversationChange,
        cancel: &CancellationToken,
    ) -> Result<Option<Switched>, ChangeRefusal> {
        let host = self.host_shard();
        let record = self.owned_record(id).await?;
        if let ConversationChange::Target(to) = change {
            return self.move_conversation(&record, to, cancel).await;
        }
        // An archive, a restore and a detach read the record they change and
        // what they release within the conversation's change boundary, which
        // a move holds from its read to its commit.
        let boundary = match &change {
            ConversationChange::Record(change) if change_is_transition(change) => {
                Some(self.conversations().slot(&record.id).changes.acquire().await)
            }
            _ => None,
        };
        let record = match &boundary {
            Some(_) => self.owned_record(id).await?,
            None => record,
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
        let change = match change {
            ConversationChange::Settings(change) => {
                // A settings change waits while a transition holds the
                // conversation, then takes its turn after the changes and
                // opens that came first.
                let slot = self.conversations().slot(&record.id);
                let _admitted = slot.file_gate().enter(Purpose::Demand).await;
                let _turn = slot.settings.acquire().await;
                return self.change_settings(&record.id, change).await.map(|()| None);
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
                return host.commit(&record.id, change).await.map(|()| None);
            }
            change => change,
        };
        let tree = self.reserve_idle_tree(&record.id)?;
        let hold = host.hold_for_transition(&record.id, tree).await?;
        let committed = match change {
            ConversationChange::Record(RecordChange::Archived(true)) => {
                let archived = host.archive(&record).await;
                if archived.is_ok() {
                    self.archived(&record.id);
                }
                archived.map(|()| None)
            }
            ConversationChange::Record(RecordChange::Archived(false)) => {
                // A conversation whose project was deleted while it was
                // archived makes it again (`crates/backend-database/src/selection.rs`).
                let remakes = match &record.target {
                    Selection::Workspace { workspace_id, .. } => self
                        .services()
                        .control
                        .workspace(workspace_id.clone())
                        .await?
                        .is_none(),
                    _ => false,
                };
                let restored = host.commit(&record.id, RecordChange::Archived(false)).await;
                if restored.is_ok() && remakes {
                    self.mark(Part::Workspaces);
                }
                restored.map(|()| None)
            }
            ConversationChange::Record(RecordChange::Detach(device)) => {
                host.detach(&record, device).await.map(|()| None)
            }
            ConversationChange::Record(change) => host.commit(&record.id, change).await.map(|()| None),
            ConversationChange::Settings(_) | ConversationChange::Target(_) => {
                unreachable!("a settings change and a move are applied before the hold")
            }
        };
        drop(hold);
        drop(boundary);
        committed
    }

    /// The user's conversation `id` as the index holds it now.
    async fn owned_record(&self, id: &ConversationId) -> Result<ConversationRecord, ChangeRefusal> {
        self.services()
            .control
            .conversation(id.clone())
            .await?
            .filter(|record| record.owner == *self.user())
            .ok_or(ChangeRefusal::NotFound)
    }

    /// Moves the conversation `record` to `to` at once, the one operation the
    /// page's move and `demi host move` take; none when `to` selects where it
    /// runs already (§ A move). Every move, whatever its outcome, logs one
    /// line.
    async fn move_conversation(
        &self,
        record: &ConversationRecord,
        to: ConversationTarget,
        cancel: &CancellationToken,
    ) -> Result<Option<Switched>, ChangeRefusal> {
        let started = Instant::now();
        let id = uuid::Uuid::new_v4();
        // A Host by its device's id; the Cloud before its first use has none.
        let named = |device: Option<&DeviceId>| device.map_or_else(|| "cloud".to_owned(), ToString::to_string);
        let from_host = named(self.host_shard().resolve_target(record).await.ok().as_ref().and_then(|from| from.device()));
        let asked = match &to {
            ConversationTarget::Device { device_id, .. } => device_id.to_string(),
            ConversationTarget::Workspace { workspace_id } => format!("project {workspace_id}"),
            ConversationTarget::Cloud { .. } => "cloud".to_owned(),
        };
        let mut steps = MoveSteps::default();
        let moved = self.move_steps(record, to, cancel, &mut steps).await;
        let to_host = match &moved {
            Ok(Some(switched)) => named(switched.switch.to.device()),
            _ => asked,
        };
        let outcome = match &moved {
            Ok(Some(_)) => "moved",
            Ok(None) => "unchanged",
            Err(_) => "refused",
        };
        let reason = moved.as_ref().err().map(|refusal| refusal.code().0.to_string());
        tracing::info!(
            move_id = %id,
            conversation = %record.id,
            from = %from_host,
            to = %to_host,
            check_ms = steps.check,
            transfers_ms = steps.transfers,
            commit_ms = steps.commit,
            total_ms = started.elapsed().as_millis() as u64,
            outcome,
            reason,
            "a conversation move ended"
        );
        moved
    }

    /// The steps of [`move_conversation`](Self::move_conversation), each
    /// timed into `steps`: the check and the wait for the change boundary,
    /// which `cancel` ends, then the commit, which it does not.
    async fn move_steps(
        &self,
        record: &ConversationRecord,
        to: ConversationTarget,
        cancel: &CancellationToken,
        steps: &mut MoveSteps,
    ) -> Result<Option<Switched>, ChangeRefusal> {
        let host = self.host_shard();
        let slot = self.conversations().slot(&record.id);
        // A destination's directory is the one its Host names.
        let step = Instant::now();
        let to = tokio::select! {
            to = host.move_destination(to) => to,
            () = cancel.cancelled() => Err(ChangeRefusal::Cancelled),
        };
        steps.check = step.elapsed().as_millis() as u64;
        let to = to?;
        let _boundary = tokio::select! {
            boundary = slot.changes.acquire() => boundary,
            () = cancel.cancelled() => return Err(ChangeRefusal::Cancelled),
        };
        // What an archive, a restore, a detach or another move made meanwhile
        // is read within the boundary; a destination that selects the
        // conversation's own target is no change.
        let record = &self.owned_record(&record.id).await?;
        if record.archived {
            return Err(ChangeRefusal::Archived);
        }
        if host.selection_for(record, &to).await? == record.target {
            return Ok(None);
        }
        // Commands still running on an offline device it leaves let go of
        // the conversation (`crates/backend-host-access/src/transition.rs`).
        let leaving = self.leaving_offline(record).await?;
        if let Some(device) = &leaving {
            slot.leave_jobs_on(device);
        }
        let step = Instant::now();
        let transfers = slot.transfers.close().await;
        steps.transfers = step.elapsed().as_millis() as u64;
        let step = Instant::now();
        let switched = host.switch_target(record, to).await;
        steps.commit = step.elapsed().as_millis() as u64;
        drop(transfers);
        let Some(switched) = switched? else {
            return Ok(None);
        };
        if switched.made.is_some() {
            self.mark(Part::Workspaces);
        }
        if let Some(device) = &leaving {
            self.fail_waiting_for(record, device).await?;
        }
        self.switched(&record.id).await;
        if let Some(left) = switched.switch.from.device()
            && Some(left) != switched.switch.to.device()
        {
            self.release_departed(&record.id, left.clone());
        }
        Ok(Some(switched))
    }

    /// Releases `device`, which a move of the conversation `id` left, once
    /// nothing of the conversation runs there, as a task of the shard; the
    /// shard's close ends the wait.
    fn release_departed(&self, id: &ConversationId, device: DeviceId) {
        let shard = self.this();
        let id = id.clone();
        self.tasks().spawn_local(async move {
            tokio::select! {
                () = shard.host_shard().release_departed(&id, &device) => {}
                () = shard.closed() => {}
            }
        });
    }

    /// What follows the archive of the conversation `id`: its title
    /// request and idle watch end, and its requests are withdrawn, so a
    /// command that waits for a credential request's answer returns.
    fn archived(&self, id: &ConversationId) {
        self.titles().abort(id);
        self.stop_idle(id);
        demi_backend_permissions::changed(self.permission_shard(), id);
        demi_backend_permissions::withdrawn(self.permission_shard(), id);
        // The automations that wake it pause, saying why.
        let shard = self.this();
        let id = id.clone();
        self.tasks().spawn_local(async move {
            shard
                .automation_target_gone(&id, demi_web_api_protocol::automations::PauseReason::ConversationArchived)
                .await;
        });
    }

    /// Deletes the user's project `id` as one transition: every conversation in it
    /// is held as its archive holds it, released on its Hosts, and archived in the
    /// transaction that deletes the workspace.
    ///
    /// A conversation of it whose work runs refuses the deletion, as it refuses
    /// its archive, and nothing changes; so does one that moved in or was created
    /// meanwhile.
    pub async fn delete_project(&self, id: &WorkspaceId) -> Result<(), ChangeRefusal> {
        let control = &self.services().control;
        let host = self.host_shard();
        let owned = control
            .workspace(id.clone())
            .await?
            .is_some_and(|workspace| workspace.user == *self.user());
        if !owned {
            return Err(ChangeRefusal::WorkspaceNotFound);
        }
        let conversations = control
            .workspace_conversations(self.user().clone(), id.clone())
            .await?;
        // Each conversation's change boundary before its record is read, as
        // its archive takes it.
        let mut boundaries = Vec::new();
        for conversation in &conversations {
            boundaries.push(self.conversations().slot(conversation).changes.acquire().await);
        }
        let mut records = Vec::new();
        let mut holds = Vec::new();
        for conversation in &conversations {
            let record = control
                .conversation(conversation.clone())
                .await?
                .ok_or(ChangeRefusal::NotFound)?;
            let tree = self.reserve_idle_tree(conversation)?;
            holds.push(host.hold_for_transition(conversation, tree).await?);
            records.push(record);
        }
        for record in &records {
            host.release_everywhere(record).await?;
        }
        let deleted = control
            .delete_workspace(self.user().clone(), id.clone(), conversations.clone())
            .await?;
        drop(holds);
        drop(boundaries);
        match deleted {
            WorkspaceDeletion::Deleted => {}
            WorkspaceDeletion::Missing => return Err(ChangeRefusal::WorkspaceNotFound),
            WorkspaceDeletion::Changed => return Err(ChangeRefusal::TurnInFlight),
        }
        for conversation in &conversations {
            self.archived(conversation);
            self.mark(Part::Conversation(conversation.clone()));
        }
        self.mark(Part::ConversationOrder);
        self.mark(Part::Workspaces);
        Ok(())
    }

    /// What follows a switch the conversation `id` made, under its hold: its
    /// idle watch starts again, since a deadline of the old binding never
    /// releases the new one, and the pages hear that every child works in
    /// the conversation's new directory (`crates/agent-server/src/node/mod.rs`).
    pub(crate) async fn switched(&self, id: &ConversationId) {
        // What it kept of its project's files where it worked is released at
        // once, before anything awaits; its next request keeps where it
        // works now.
        self.project_files().release(id);
        self.restart_idle(id);
        self.agent().placement_changed(&root_of(id)).await;
    }

    /// The paired device a move of `record` leaves while its runner is not
    /// connected, if it does.
    async fn leaving_offline(&self, record: &ConversationRecord) -> Result<Option<DeviceId>, ChangeRefusal> {
        let target = self.host_shard().resolve_target(record).await?;
        let Some(device) = target.device() else {
            return Ok(None);
        };
        let paired = self
            .services()
            .control
            .device(device.clone())
            .await?
            .is_some_and(|device| device.kind == DeviceKind::User);
        Ok((paired && !self.devices().connected(device)).then(|| device.clone()))
    }

    /// Reserves the conversation's live tree while it does nothing by itself: no
    /// action runs or waits, no child is live and no command runs. A
    /// conversation without a live tree runs nothing; one whose tree works
    /// refuses the transition.
    fn reserve_idle_tree(&self, id: &ConversationId) -> Result<Option<Reservation>, ChangeRefusal> {
        let Some(tree) = self.agent().tree(&root_of(id)) else {
            return Ok(None);
        };
        // The check and the reservation are one step: no await between them.
        if tree.works() || tree.runs_commands() {
            return Err(ChangeRefusal::TurnInFlight);
        }
        tree.admission()
            .try_reserve()
            .map(Some)
            .ok_or(ChangeRefusal::TurnInFlight)
    }

    /// Fails the jobs that wait for `device`, the offline device the move
    /// of `record` left, with what the agent reads of them: where the
    /// conversation runs now, and that the command did not run
    /// (`crates/backend-host-access/src/access.rs`).
    async fn fail_waiting_for(&self, record: &ConversationRecord, device: &DeviceId) -> Result<(), ChangeRefusal> {
        let control = &self.services().control;
        let left = control
            .device(device.clone())
            .await?
            .map_or_else(|| device.to_string(), |left| left.name);
        let record = control
            .conversation(record.id.clone())
            .await?
            .ok_or(ChangeRefusal::NotFound)?;
        let target = self.host_shard().resolve_target(&record).await?;
        let (_, now) = self.primary_host(&target).await?;
        let reason = format!("This conversation now runs on {now}; this command did not run, since {left} was offline.");
        self.conversations().slot(&record.id).fail_waiting_for(device, &reason);
        Ok(())
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
        if let Some(workflow) = patch.workflow {
            changes.push((
                vec![PatchField::Workflow],
                RecordChange::Workflow(workflow).into(),
            ));
        }
        if let Some(target) = patch.target {
            changes.push((vec![PatchField::Target], ConversationChange::Target(target)));
        }
        let mut results = Vec::new();
        for (fields, change) in changes {
            let outcome = self.transition_switching(id, change, &CancellationToken::new()).await;
            for field in fields {
                let result = match &outcome {
                    Ok(_) => FieldResult::Applied { field },
                    Err(refusal) => failed(field, refusal),
                };
                results.push(result);
            }
        }
        let Some(record) = control.conversation(id.clone()).await? else {
            return Ok(None);
        };
        let conversation = self.conversation_summary(record.clone()).await?;
        let metadata = self.conversation_metadata(record).await?;
        Ok(Some(ConversationUpdate {
            conversation,
            metadata,
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
    use std::rc::Rc;

    use demi_web_api_protocol::conversations::ConversationTarget;
    use demi_web_api_protocol::error::ErrorCode;
    use demi_web_api_protocol::ids::{DeviceId, UserId, WorkspaceId};

    use super::*;
    use crate::services::Services;
    use crate::shard::{ShardPlacement, ShardPool};
    use demi_backend_database::accounts::TokenHash;
    use demi_backend_database::control::{ControlService, testing};
    use demi_backend_database::conversation_index::{AttachedHostRecord, Creation};
    use demi_runner_protocol::wire::RunnerPlatform;

    const ID: &str = "0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a01";

    #[tokio::test(flavor = "local")]
    async fn a_move_applies_while_work_holds_the_conversation_a_transition_waits_for_no_work_and_a_field_update_waits_for_a_transition() {
        let data = tempfile::tempdir().unwrap();
        let services = Services::start_for_tests(data.path()).await;
        let control = services.control.clone();
        let owner: UserId = testing::master(&control).await.id;
        let id = ConversationId::try_from(ID).unwrap();
        assert!(matches!(
            control
                .create_conversation(owner.clone(), id.clone(), demi_backend_database::conversation_index::ConversationStart::default())
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
                // Its runner is connected, so the move's directory resolves.
                let _runner = shard
                    .play_runner_for_tests(&laptop, "/home/ana", |_| Box::pin(async {}))
                    .await;
                let slot = shard.conversations().slot(&id);
                let to = ConversationTarget::Device {
                    device_id: laptop.clone(),
                    path: "/work".into(),
                };
                // A Host operation in flight: a move applies at once, and a
                // transition refuses rather than wait for it.
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
                // A transition holds the conversation: a rename waits for it, then
                // applies.
                //
                // The rename reads the conversation first, and the control database
                // answers its one connection's calls in order: once a read of the
                // test's after it has come back and the test has yielded, the rename
                // has gone as far as it can, to the gate or through its commit,
                // which a second read would then see.
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
        assert_eq!(
            refusals,
            [Ok(()), Err(ErrorCode::TurnInFlight), Err(ErrorCode::TurnInFlight)]
        );
        assert!(waited);
        assert_ne!(
            held_title.as_deref(), Some("Renamed"),
            "the rename applied while a transition held the conversation"
        );
        assert_eq!(title.as_deref(), Some("Renamed"));
        pool.close().await;
    }

    /// Moves the conversation `ID` to `/work` on a Linux device whose
    /// runner the test plays as `silent` says, counting what the runner is
    /// asked of a path; answers how many questions that was, the move's
    /// refusal code and message, and whether the conversation is where it was.
    async fn move_counting(silent: bool) -> (usize, Option<(ErrorCode, String)>, bool) {
        let data = tempfile::tempdir().unwrap();
        let services = Services::start_for_tests(data.path()).await;
        let control = services.control.clone();
        let owner: UserId = testing::master(&control).await.id;
        let id = ConversationId::try_from(ID).unwrap();
        control
            .create_conversation(owner.clone(), id.clone(), demi_backend_database::conversation_index::ConversationStart::default())
            .await
            .unwrap();
        let laptop = control
            .create_device(owner.clone(), "laptop".into(), RunnerPlatform::Linux, TokenHash::of("laptop"))
            .await
            .unwrap()
            .id;
        let pool = ShardPool::start(ShardPlacement::Inline, services).await.unwrap();
        let answer = pool
            .shards()
            .of(&owner)
            .call(move |shard, _| async move {
                let asked = Rc::new(std::cell::Cell::new(0));
                let host = crate::shard::PlayedHost {
                    platform: RunnerPlatform::Linux,
                    home: "/home/ana".into(),
                    resolve: Box::new({
                        let asked = asked.clone();
                        move |path| {
                            asked.set(asked.get() + 1);
                            Some(path.to_owned())
                        }
                    }),
                    silent,
                };
                let _runner = shard.play_host_for_tests(&laptop, host, |_| Box::pin(async {})).await;
                let before = shard.services().control.conversation(id.clone()).await.unwrap().unwrap().target;
                let to = ConversationTarget::Device {
                    device_id: laptop.clone(),
                    path: "/work".into(),
                };
                let moved = shard.transition(&id, ConversationChange::Target(to)).await;
                let after = shard.services().control.conversation(id.clone()).await.unwrap().unwrap().target;
                (
                    asked.get(),
                    moved.err().map(|refusal| (refusal.code().0, refusal.to_string())),
                    before == after,
                )
            })
            .await
            .unwrap();
        pool.close().await;
        answer
    }

    /// A move checks its directory with one question to the destination's
    /// Host, which names the directory and says it is one.
    ///
    /// Planted defect this catches: a check that asks for the real path and
    /// then whether it is a directory, two round trips to a far Host.
    #[tokio::test(flavor = "local")]
    async fn a_move_checks_its_directory_with_one_question_to_its_host() {
        let (asked, refused, stayed) = move_counting(false).await;
        assert_eq!((asked, refused, stayed), (1, None, false));
    }

    /// A Host that does not answer the question refuses the move within
    /// the question's deadline, saying why, and the conversation stays.
    ///
    /// Planted defect this catches: a move that waits for the runner as long
    /// as its connection lives.
    // The deadline on the paused clock: a few milliseconds.
    #[tokio::test(flavor = "local", start_paused = true)]
    async fn a_move_to_a_host_that_does_not_answer_fails_with_why_and_changes_nothing() {
        let (_, refused, stayed) = move_counting(true).await;
        assert_eq!(
            refused,
            Some((
                ErrorCode::HostOperationFailed,
                "laptop did not answer within 10 seconds whether /work is a directory there".to_owned()
            ))
        );
        assert!(stayed);
    }

    /// A conversation `ID` of a new owner on the Cloud, with the Linux
    /// devices `one` and `two`, in a started pool.
    async fn two_devices() -> (tempfile::TempDir, ShardPool, ControlService, UserId, DeviceId, DeviceId) {
        let data = tempfile::tempdir().unwrap();
        let services = Services::start_for_tests(data.path()).await;
        let control = services.control.clone();
        let owner: UserId = testing::master(&control).await.id;
        control
            .create_conversation(
                owner.clone(),
                ConversationId::try_from(ID).unwrap(),
                demi_backend_database::conversation_index::ConversationStart::default(),
            )
            .await
            .unwrap();
        let mut devices = Vec::new();
        for name in ["one", "two"] {
            let device = control
                .create_device(owner.clone(), name.into(), RunnerPlatform::Linux, TokenHash::of(name))
                .await
                .unwrap();
            devices.push(device.id);
        }
        let pool = ShardPool::start(ShardPlacement::Inline, services).await.unwrap();
        let [one, two] = [devices[0].clone(), devices[1].clone()];
        (data, pool, control, owner, one, two)
    }

    /// The releases played runners heard, by device, in order.
    type Heard = Rc<tokio::sync::watch::Sender<Vec<DeviceId>>>;

    /// Connects `device` through a runner the test plays, whose Host names
    /// a path a directory when `resolve` does, and which never answers what a
    /// directory is when `silent`. It writes each release it receives to
    /// `heard` and answers it once `answer` holds true.
    async fn holding_runner(
        shard: &Rc<Shard>,
        device: &DeviceId,
        resolve: fn(&str) -> Option<String>,
        silent: bool,
        heard: &Heard,
        answer: &tokio::sync::watch::Receiver<bool>,
    ) {
        let host = crate::shard::PlayedHost {
            platform: RunnerPlatform::Linux,
            home: "/home/ana".into(),
            resolve: Box::new(resolve),
            silent,
        };
        let (heard, answer, released) = (heard.clone(), answer.clone(), device.clone());
        shard
            .play_host_for_tests(device, host, move |_| {
                let (heard, mut answer, released) = (heard.clone(), answer.clone(), released.clone());
                Box::pin(async move {
                    heard.send_modify(|heard| heard.push(released));
                    // The test holds the sender while it plays the runner.
                    let _ = answer.wait_for(|answer| *answer).await;
                })
            })
            .await;
    }

    /// Every path is a directory.
    fn anywhere(path: &str) -> Option<String> {
        Some(path.to_owned())
    }

    fn on(device: &DeviceId, path: &str) -> ConversationChange {
        ConversationChange::Target(ConversationTarget::Device {
            device_id: device.clone(),
            path: path.into(),
        })
    }

    /// Between the moment nothing of the conversation runs on the Host a move
    /// left and the answer to its release, no work of the conversation starts
    /// there: a command on that Host, attached now, waits until the release
    /// is answered, so the release never ends the services it uses.
    ///
    /// Planted defect this catches: a release that checks the Host is idle and
    /// then sends the release without holding the Host's admissions.
    // Two played runners: a few milliseconds.
    #[tokio::test(flavor = "local")]
    async fn work_on_the_host_a_move_left_waits_until_its_release_is_answered() {
        let (_data, pool, _control, owner, one, two) = two_devices().await;
        pool.shards()
            .of(&owner)
            .call(move |shard, _| async move {
                let id = ConversationId::try_from(ID).unwrap();
                let heard: Heard = Rc::new(tokio::sync::watch::Sender::new(Vec::new()));
                let (answer, answering) = tokio::sync::watch::channel(false);
                for device in [&one, &two] {
                    holding_runner(&shard, device, anywhere, false, &heard, &answering).await;
                }
                shard.transition(&id, on(&one, "/work")).await.unwrap();
                // A command runs on one while the conversation moves to two.
                let (started, running) = tokio::sync::oneshot::channel();
                let (finish, finished) = tokio::sync::oneshot::channel::<()>();
                let command = tokio::task::spawn_local({
                    let (shard, id) = (shard.clone(), id.clone());
                    async move {
                        shard
                            .host_shard()
                            .with_host(&id, None, &CancellationToken::new(), async |_| {
                                started.send(()).unwrap();
                                finished.await.unwrap();
                            })
                            .await
                            .unwrap();
                    }
                });
                running.await.unwrap();
                shard.transition(&id, on(&two, "/work")).await.unwrap();
                assert!(heard.borrow().is_empty(), "nothing is released while a command runs there");
                finish.send(()).unwrap();
                command.await.unwrap();
                heard.subscribe().wait_for(|heard| heard.len() == 1).await.unwrap();
                // While the release is on its way, a command on one waits.
                let (entered, mut admitted) = tokio::sync::oneshot::channel();
                let next = tokio::task::spawn_local({
                    let (shard, id, one) = (shard.clone(), id.clone(), one.clone());
                    async move {
                        shard
                            .host_shard()
                            .with_host(&id, Some(&one), &CancellationToken::new(), async |_| {
                                entered.send(()).unwrap();
                            })
                            .await
                            .unwrap();
                    }
                });
                let mut waiting = shard.conversations().slot(&id).host_gate(&one).waiting();
                let early = tokio::select! {
                    biased;
                    _ = &mut admitted => true,
                    _ = waiting.wait_for(|waiting| *waiting == 1) => false,
                };
                assert!(!early, "a command started on one while its release was on its way");
                answer.send(true).unwrap();
                next.await.unwrap();
                admitted.await.unwrap();
                assert_eq!(*heard.borrow(), [one]);
            })
            .await
            .unwrap();
        pool.close().await;
    }

    /// A move that comes while an archive releases the conversation waits for
    /// the archive and is refused, so every Host the archived conversation
    /// reaches has heard its release.
    ///
    /// Planted defect this catches: an archive that releases the Hosts of the
    /// record it read before a move committed, leaving the new Host
    /// unreleased.
    // Two played runners: a few milliseconds.
    #[tokio::test(flavor = "local")]
    async fn a_move_during_an_archive_waits_for_it_and_every_host_it_reaches_is_released() {
        let (_data, pool, control, owner, one, two) = two_devices().await;
        pool.shards()
            .of(&owner)
            .call(move |shard, _| async move {
                let id = ConversationId::try_from(ID).unwrap();
                let heard: Heard = Rc::new(tokio::sync::watch::Sender::new(Vec::new()));
                let (answer, answering) = tokio::sync::watch::channel(false);
                for device in [&one, &two] {
                    holding_runner(&shard, device, anywhere, false, &heard, &answering).await;
                }
                shard.transition(&id, on(&one, "/work")).await.unwrap();
                let archive = tokio::task::spawn_local({
                    let (shard, id) = (shard.clone(), id.clone());
                    async move { shard.transition(&id, RecordChange::Archived(true).into()).await }
                });
                heard.subscribe().wait_for(|heard| heard.len() == 1).await.unwrap();
                let mut moving = tokio::task::spawn_local({
                    let (shard, id, two) = (shard.clone(), id.clone(), two.clone());
                    async move { shard.transition(&id, on(&two, "/work")).await }
                });
                let mut waiting = shard.conversations().slot(&id).changes.waiting();
                let moved = tokio::select! {
                    biased;
                    moved = &mut moving => Some(moved.unwrap()),
                    _ = waiting.wait_for(|waiting| *waiting == 1) => None,
                };
                assert!(moved.is_none(), "the move did not wait for the archive: {moved:?}");
                answer.send(true).unwrap();
                archive.await.unwrap().unwrap();
                assert_eq!(moving.await.unwrap().map_err(|refusal| refusal.code().0), Err(ErrorCode::ConversationArchived));
                let record = control.conversation(id.clone()).await.unwrap().unwrap();
                let target = control.execution_target(record.owner, record.id, record.target).await.unwrap();
                assert_eq!(target.device(), Some(&one));
                assert!(heard.borrow().contains(&one), "{:?}", heard.borrow());
            })
            .await
            .unwrap();
        pool.close().await;
    }

    /// A move whose caller goes away while its Host has not answered moves
    /// nothing.
    ///
    /// Planted defect this catches: a move that goes on after its caller,
    /// such as a stopped `demi host move`, went away.
    // Two played runners, one of which never answers: a few milliseconds.
    #[tokio::test(flavor = "local")]
    async fn a_move_cancelled_before_its_commit_moves_nothing() {
        let (_data, pool, control, owner, one, two) = two_devices().await;
        pool.shards()
            .of(&owner)
            .call(move |shard, _| async move {
                let id = ConversationId::try_from(ID).unwrap();
                let heard: Heard = Rc::new(tokio::sync::watch::Sender::new(Vec::new()));
                let (_answer, answering) = tokio::sync::watch::channel(true);
                holding_runner(&shard, &one, anywhere, false, &heard, &answering).await;
                holding_runner(&shard, &two, anywhere, true, &heard, &answering).await;
                shard.transition(&id, on(&one, "/work")).await.unwrap();
                let before = control.conversation(id.clone()).await.unwrap().unwrap().target;
                let cancel = CancellationToken::new();
                let to = ConversationTarget::Device { device_id: two.clone(), path: "/work".into() };
                let moving = shard.move_to(&id, to, &cancel);
                cancel.cancel();
                assert_eq!(moving.await.map_err(|refusal| refusal.to_string()), Err("The move was cancelled; the conversation did not move".to_owned()));
                assert_eq!(control.conversation(id.clone()).await.unwrap().unwrap().target, before);
            })
            .await
            .unwrap();
        pool.close().await;
    }

    /// A move into a project of a paired device asks the device whether the
    /// project's directory is there, as a move to a directory does, and one
    /// that is gone refuses the move and changes nothing.
    ///
    /// Planted defect this catches: a move into a project that trusts the
    /// directory the project recorded.
    // One played runner: a few milliseconds.
    #[tokio::test(flavor = "local")]
    async fn a_move_into_a_project_whose_directory_is_gone_fails_and_changes_nothing() {
        let (_data, pool, control, owner, one, _two) = two_devices().await;
        pool.shards()
            .of(&owner.clone())
            .call(move |shard, _| async move {
                let id = ConversationId::try_from(ID).unwrap();
                let heard: Heard = Rc::new(tokio::sync::watch::Sender::new(Vec::new()));
                let (_answer, answering) = tokio::sync::watch::channel(true);
                let here = |path: &str| (path != "/gone").then(|| path.to_owned());
                holding_runner(&shard, &one, here, false, &heard, &answering).await;
                let gone = WorkspaceId::try_from("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a09").unwrap();
                control
                    .create_workspace(gone.clone(), owner.clone(), one.clone(), "/gone".into(), "gone".into())
                    .await
                    .unwrap();
                let before = control.conversation(id.clone()).await.unwrap().unwrap().target;
                let moved = shard
                    .transition(&id, ConversationChange::Target(ConversationTarget::Workspace { workspace_id: gone }))
                    .await;
                assert_eq!(
                    moved.map_err(|refusal| (refusal.code().0, refusal.to_string())),
                    Err((ErrorCode::FsNotFound, "/gone is not a directory on one".to_owned()))
                );
                assert_eq!(control.conversation(id.clone()).await.unwrap().unwrap().target, before);
            })
            .await
            .unwrap();
        pool.close().await;
    }

    thread_local! {
        /// The log lines of moves that ended on this thread while a test
        /// records them: each outcome, and whether it says how long its check
        /// took.
        static MOVE_LINES: std::cell::RefCell<Option<Vec<(String, bool)>>> = const { std::cell::RefCell::new(None) };
    }

    /// Records the lines of moves into [`MOVE_LINES`]. It is the process's
    /// one subscriber, set once: a subscriber of one thread misses lines
    /// whose interest another test's thread cached while none was set.
    struct MoveLines;

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for MoveLines {
        fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
            #[derive(Default)]
            struct Fields {
                message: String,
                outcome: String,
                checked: bool,
            }
            impl tracing::field::Visit for Fields {
                fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                    if field.name() == "outcome" {
                        self.outcome = value.to_owned();
                    }
                }
                fn record_u64(&mut self, field: &tracing::field::Field, _: u64) {
                    self.checked |= field.name() == "check_ms";
                }
                fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                    if field.name() == "message" {
                        self.message = format!("{value:?}");
                    }
                }
            }
            let mut fields = Fields::default();
            event.record(&mut fields);
            if fields.message == "a conversation move ended" {
                MOVE_LINES.with_borrow_mut(|lines| {
                    if let Some(lines) = lines {
                        lines.push((fields.outcome, fields.checked));
                    }
                });
            }
        }
    }

    /// Every move writes one log line with its outcome and the time of its
    /// check, a refused one too: one to a relative directory, one to a
    /// directory that is not there, and one made.
    ///
    /// Planted defect this catches: a check the caller of the move makes
    /// before it, whose refusal no line records.
    // One played runner: a few milliseconds.
    #[tokio::test(flavor = "local")]
    async fn every_move_writes_one_log_line_whatever_its_outcome() {
        use tracing_subscriber::layer::SubscriberExt;
        static LOGGING: std::sync::Once = std::sync::Once::new();
        LOGGING.call_once(|| {
            tracing::subscriber::set_global_default(tracing_subscriber::registry().with(MoveLines))
                .expect("no other test of the binary sets the process's subscriber");
        });
        MOVE_LINES.set(Some(Vec::new()));
        let (_data, pool, _control, owner, one, _two) = two_devices().await;
        pool.shards()
            .of(&owner)
            .call(move |shard, _| async move {
                let id = ConversationId::try_from(ID).unwrap();
                let heard: Heard = Rc::new(tokio::sync::watch::Sender::new(Vec::new()));
                let (_answer, answering) = tokio::sync::watch::channel(true);
                let here = |path: &str| (path != "/gone").then(|| path.to_owned());
                holding_runner(&shard, &one, here, false, &heard, &answering).await;
                for path in ["work", "/gone", "/work"] {
                    // Two refusals, then the move.
                    let _ = shard.transition(&id, on(&one, path)).await;
                }
            })
            .await
            .unwrap();
        pool.close().await;
        let expected = [("refused", true), ("refused", true), ("moved", true)].map(|(outcome, checked)| (outcome.to_owned(), checked));
        assert_eq!(MOVE_LINES.take(), Some(expected.to_vec()));
    }

    /// A directory is the one its Host names
    /// (`crates/backend-database/src/selection.rs`): on Windows
    /// `C:/Users/ana/site/` and `c:\users\ana\site` are the one project
    /// `C:\Users\ana\site`, and a link to the home is the home.
    ///
    /// Planted defect this catches: a move that selects by the spelling it was
    /// given, which makes two projects of one directory and a project of the home.
    #[tokio::test(flavor = "local")]
    async fn two_spellings_of_a_directory_select_one_project_and_a_link_to_the_home_is_the_home() {
        let data = tempfile::tempdir().unwrap();
        let services = Services::start_for_tests(data.path()).await;
        let control = services.control.clone();
        let owner: UserId = testing::master(&control).await.id;
        let first = ConversationId::try_from(ID).unwrap();
        let second = ConversationId::try_from("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a02").unwrap();
        for id in [&first, &second] {
            control
                .create_conversation(owner.clone(), id.clone(), demi_backend_database::conversation_index::ConversationStart::default())
                .await
                .unwrap();
        }
        let desktop = control
            .create_device(owner.clone(), "desktop".into(), RunnerPlatform::Win32, TokenHash::of("desktop"))
            .await
            .unwrap()
            .id;
        let pool = ShardPool::start(ShardPlacement::Inline, services).await.unwrap();
        let (targets, workspaces) = pool
            .shards()
            .of(&owner)
            .call({
                let (first, second, desktop, control, owner) =
                    (first.clone(), second.clone(), desktop.clone(), control.clone(), owner.clone());
                move |shard, _| async move {
                    // The system names a directory in its own case, with
                    // its separators, through `D:\Links\ana`, a link to the
                    // home.
                    let host = crate::shard::PlayedHost {
                        platform: RunnerPlatform::Win32,
                        home: r"C:\Users\ana".into(),
                        resolve: Box::new(|path| {
                            let spelled = path.to_lowercase().replace('/', r"\");
                            match spelled.trim_end_matches('\\') {
                                r"c:\users\ana\site" => Some(r"C:\Users\ana\site".to_owned()),
                                r"c:\users\ana" | r"d:\links\ana" => Some(r"C:\Users\ana".to_owned()),
                                _ => None,
                            }
                        }),
                        silent: false,
                    };
                    let _runner = shard.play_host_for_tests(&desktop, host, |_| Box::pin(async {})).await;
                    let mut targets = Vec::new();
                    for (id, path) in [
                        (&first, "C:/Users/ana/site/"),
                        (&second, r"c:\users\ana\site"),
                        (&first, r"D:\Links\ana"),
                    ] {
                        let to = ConversationTarget::Device {
                            device_id: desktop.clone(),
                            path: path.into(),
                        };
                        shard.transition(id, ConversationChange::Target(to)).await.unwrap();
                        targets.push(control.conversation(id.clone()).await.unwrap().unwrap().target);
                    }
                    (targets, control.workspaces(owner).await.unwrap())
                }
            })
            .await
            .unwrap();
        let [workspace] = workspaces.as_slice() else {
            panic!("one project, not {workspaces:?}");
        };
        assert_eq!(workspace.path, r"C:\Users\ana\site");
        let project = Selection::Workspace {
            workspace_id: workspace.id.clone(),
            device_id: desktop.clone(),
            path: r"C:\Users\ana\site".into(),
        };
        assert_eq!(targets, [project.clone(), project, Selection::Home { device_id: desktop }]);
        pool.close().await;
    }

    /// A conversation release as a device's runner received it, with the
    /// conversation's binding at that moment.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Released {
        device: DeviceId,
        /// Where the conversation ran then: its device and directory.
        target: (Option<DeviceId>, String),
        attached: Vec<DeviceId>,
        archived: bool,
    }

    /// The releases the played runners received, in order.
    type ReleaseLog = Rc<tokio::sync::watch::Sender<Vec<Released>>>;

    /// Waits until `log` holds `count` releases.
    async fn until_logged(log: &ReleaseLog, count: usize) {
        log.subscribe()
            .wait_for(|log| log.len() >= count)
            .await
            .expect("the test keeps the log");
    }

    /// Connects `device` through a runner the test plays, which answers
    /// every conversation release and writes it to `log`.
    async fn runner(shard: &Rc<Shard>, device: &DeviceId, log: &ReleaseLog) {
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
                let resolved = control
                    .execution_target(record.owner, record.id, record.target)
                    .await
                    .unwrap();
                log.send_modify(|log| log.push(Released {
                    device,
                    target: (resolved.device().cloned(), resolved.path().to_owned()),
                    attached: attached.into_iter().map(|host| host.device).collect(),
                    archived: record.archived,
                }));
            })
        })
        .await;
    }

    #[tokio::test(flavor = "local")]
    async fn a_detach_and_an_archive_release_the_conversation_before_they_change_its_binding_and_a_move_after()
     {
        let data = tempfile::tempdir().unwrap();
        let services = Services::start_for_tests(data.path()).await;
        let control = services.control.clone();
        let owner: UserId = testing::master(&control).await.id;
        let id = ConversationId::try_from(ID).unwrap();
        assert!(matches!(
            control
                .create_conversation(owner.clone(), id.clone(), demi_backend_database::conversation_index::ConversationStart::default())
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
                let log: ReleaseLog = Rc::new(tokio::sync::watch::Sender::new(Vec::new()));
                let [one, two] = [devices[0].clone(), devices[1].clone()];
                runner(&shard, &one, &log).await;
                runner(&shard, &two, &log).await;
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
                    target: match target {
                        ConversationTarget::Device { device_id, path } => (Some(device_id), path),
                        other => panic!("the test moves to device directories, not {other:?}"),
                    },
                    attached: attached.to_vec(),
                    archived,
                };
                // A move commits first and releases the device it left once
                // nothing of the conversation runs there, which it left
                // attached.
                shard
                    .transition(&id, ConversationChange::Target(on(&two)))
                    .await
                    .unwrap();
                until_logged(&log, 1).await;
                assert_eq!(*log.borrow(), [released(&one, on(&two), std::slice::from_ref(&one), false)]);
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
