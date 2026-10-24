//! Changes of a conversation (`crates/backend-host-access/src/transition.rs`, §
//! Lifecycle access).
//!
//! Every change goes through one entry, `Shard::transition`.
//!
//! - A rename, a pin or a change of the model settings waits while a transition
//!   holds the conversation, and is then applied as if it arrived afterwards.
//!   The model settings change one at a time (`settings.rs`).
//! - An archive and a restore are transitions. Each holds the
//!   conversation, with its idle tree reserved, its file transfers, user
//!   streams and the one-shot calls admitted like them ended, and its file gate
//!   reserved. Each commits, then tells the connected devices it leaves to
//!   release the conversation, and waits for none (`crates/backend-user-shard/src/lifecycle/conversations.rs`).
//! - A detach applies at once and waits for no Host, whatever the conversation
//!   does, as a move does: the Host leaves, its jobs are asked to stop, and it
//!   answers; the Host hears the release once they ended.
//! - A move stops and waits for nothing: it reads its destination as typed, ends the
//!   transfers and streams, and commits, advancing the execution-context
//!   revision, so each agent's next request names the new place. The commands
//!   running then finish where they started; the device it left hears the
//!   release once they ended. Each move logs one line with its id, its Hosts,
//!   how long each step took and its outcome.
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
/// conversation: an archive and a restore.
fn change_is_transition(change: &RecordChange) -> bool {
    matches!(change, RecordChange::Archived(_))
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
        // An archived conversation takes nothing but its restore; the index
        // transaction checks again when it applies the change. A move
        // refuses it itself, so that its log line is written.
        if record.archived
            && !matches!(
                change,
                ConversationChange::Record(RecordChange::Archived(_)) | ConversationChange::Target(_)
            )
        {
            return Err(ChangeRefusal::Archived);
        }
        let change = match change {
            ConversationChange::Target(to) => return self.move_conversation(&record, to, cancel).await,
            ConversationChange::Settings(change) => {
                // A settings change waits while a transition holds the
                // conversation, then takes its turn after the changes and
                // opens that came first.
                let slot = self.conversations().slot(&record.id);
                let _admitted = slot.file_gate().enter(Purpose::Demand).await;
                let _turn = slot.settings.acquire().await;
                return self.change_settings(&record.id, change).await.map(|()| None);
            }
            ConversationChange::Record(RecordChange::Detach(device)) => {
                // A detach holds nothing and waits for no work: it ends the
                // conversation's jobs on the Host
                // (`crates/backend-host-access/src/transition.rs`).
                let removed = host.detach(&record, device.clone()).await?;
                if removed {
                    self.release_detached(&record.id, device);
                }
                return Ok(None);
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
                // archived brings it back (`crates/backend-database/src/workspaces.rs`).
                let remakes = match &record.target {
                    Selection::Workspace { workspace_id, .. } => self
                        .services()
                        .control
                        .workspace(workspace_id.clone())
                        .await?
                        .is_none_or(|workspace| workspace.deleted),
                    _ => false,
                };
                let restored = host.commit(&record.id, RecordChange::Archived(false)).await;
                if restored.is_ok() && remakes {
                    self.mark(Part::Workspaces);
                }
                if restored.is_ok() {
                    // Its schedules wake it again, counting from now.
                    self.conversation_unarchived_schedules(&record.id);
                }
                restored.map(|()| None)
            }
            ConversationChange::Record(change) => host.commit(&record.id, change).await.map(|()| None),
            ConversationChange::Settings(_) | ConversationChange::Target(_) => {
                unreachable!("a settings change and a move are applied before the hold")
            }
        };
        drop(hold);
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
            event = "conversation_move.ended",
            move_id = %id,
            conversation = %record.id,
            from_host_id = %from_host,
            host_id = %to_host,
            check_ms = steps.check,
            transfers_ms = steps.transfers,
            commit_ms = steps.commit,
            ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            outcome,
            reason,
            "a conversation move ended"
        );
        moved
    }

    /// The steps of [`move_conversation`](Self::move_conversation), each
    /// timed into `steps`: the check, which `cancel` ends, then the commit,
    /// which it does not.
    async fn move_steps(
        &self,
        record: &ConversationRecord,
        to: ConversationTarget,
        cancel: &CancellationToken,
        steps: &mut MoveSteps,
    ) -> Result<Option<Switched>, ChangeRefusal> {
        if record.archived {
            return Err(ChangeRefusal::Archived);
        }
        let host = self.host_shard();
        let slot = self.conversations().slot(&record.id);
        // The worktrees the Host reported before the move are its
        // destination's, as a command that adds one and then moves there
        // expects (`crate::shard::worktrees`).
        self.worktrees_stored().await;
        // A destination's directory is stored as typed; its Host names it at
        // the first use.
        let step = Instant::now();
        let to = tokio::select! {
            to = host.move_destination(to) => to,
            () = cancel.cancelled() => Err(ChangeRefusal::Cancelled),
        };
        steps.check = step.elapsed().as_millis() as u64;
        let to = to?;
        if cancel.is_cancelled() {
            return Err(ChangeRefusal::Cancelled);
        }
        // A destination that selects the conversation's own target is no
        // change.
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
        // A move may make or bring back a project, and changes the count of
        // a project's saved messages it joins with its own; the pages of the
        // conversations that share the list it shows now read it again.
        self.mark(Part::Workspaces);
        if let Ok(sharers) = self.services().control.saved_sharers(record.id.clone()).await {
            for sharer in sharers.conversations {
                self.mark(Part::Metadata(sharer));
            }
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
    /// the commands running there ended, as a task of the shard; the shard's
    /// close ends the wait.
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

    /// Releases `device`, which a detach of the conversation `id` removed,
    /// once the commands running there ended, as a task of the shard; the
    /// shard's close ends the wait.
    fn release_detached(&self, id: &ConversationId, device: DeviceId) {
        let shard = self.this();
        let id = id.clone();
        self.tasks().spawn_local(async move {
            tokio::select! {
                () = shard.host_shard().release_detached(&id, &device) => {}
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
        // Its schedules wake nothing now; what they had under way drops.
        let shard = self.this();
        let id = id.clone();
        self.tasks().spawn_local(async move {
            shard.conversation_archived_schedules(&id).await;
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
        let mut records = Vec::new();
        let mut released = Vec::new();
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
            released.push((record.id.clone(), host.release_devices(record).await?));
        }
        let deleted = control
            .delete_workspace(self.user().clone(), id.clone(), conversations.clone())
            .await?;
        drop(holds);
        match deleted {
            WorkspaceDeletion::Deleted => {}
            WorkspaceDeletion::Missing => return Err(ChangeRefusal::WorkspaceNotFound),
            WorkspaceDeletion::Changed => return Err(ChangeRefusal::TurnInFlight),
        }
        for (conversation, devices) in &released {
            self.archived(conversation);
            self.mark(Part::Conversation(conversation.clone()));
            // The connected Hosts are told now, the others by their `hello`.
            host.tell_release(conversation, devices, true);
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
            event = "conversation_change.failed",
            field = ?field,
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
