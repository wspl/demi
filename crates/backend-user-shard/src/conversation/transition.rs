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
//! (`crates/backend-user-shard/src/lifecycle/conversations.rs`). A transition waits for
//! no other work: a conversation that is busy refuses it. The shard reserves
//! the idle tree; host access holds the rest and runs the transition's steps
//! (`demi_backend_host_access::transition`).

use demi_backend_database::workspaces::WorkspaceDeletion;
use demi_backend_database::{Selection, StorageError};
use demi_backend_database::conversation_index::{
    ConversationChange, ConversationRecord, RecordChange, SettingsChange,
};
use demi_backend_host_access::root_of;
use demi_backend_host_access::transition::{ChangeRefusal, Switched};
use demi_shared_types::{AgentMessage, AgentMessageEvent, BlockId, NodeId};
use demi_backend_page_sync::Part;
use demi_shared_gates::{Purpose, Reservation};
use demi_web_api_protocol::conversations::{
    ConversationPatch, ConversationUpdate, FieldResult, PatchField,
};
use demi_web_api_protocol::devices::DeviceKind;
use demi_web_api_protocol::ids::{ConversationId, DeviceId, WorkspaceId};

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
        self.transition_switching(id, change).await.map(drop)
    }

    /// `transition`, answering the switch it made when `change` is a target
    /// switch that moved the conversation.
    async fn transition_switching(
        &self,
        id: &ConversationId,
        change: ConversationChange,
    ) -> Result<Option<Switched>, ChangeRefusal> {
        let reorders = matches!(
            change,
            ConversationChange::Record(RecordChange::Pinned(_) | RecordChange::Archived(_))
        );
        let switched = self.apply_change(id, change).await?;
        self.mark(Part::Conversation(id.clone()));
        if reorders {
            self.mark(Part::ConversationOrder);
        }
        Ok(switched)
    }

    async fn apply_change(
        &self,
        id: &ConversationId,
        change: ConversationChange,
    ) -> Result<Option<Switched>, ChangeRefusal> {
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
        // A destination's directory is the one its Host names, and a
        // destination that selects the conversation's own target is no
        // change.
        let change = match change {
            ConversationChange::Target(to) => ConversationChange::Target(host.resolved_destination(to).await?),
            change => change,
        };
        if let ConversationChange::Target(to) = &change
            && host.selection_for(&record, to).await? == record.target
        {
            return Ok(None);
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
        // A switch away from an offline device leaves the commands still
        // running there to its runner (`sessions-and-targets.md` § Switch
        // the primary target).
        let leaving = match &change {
            ConversationChange::Target(_) => self.leaving_offline(&record).await?,
            _ => None,
        };
        let tree = self.reserve_idle_tree(&record.id, leaving.as_ref())?;
        if let Some(device) = &leaving {
            self.conversations().slot(&record.id).leave_jobs_on(device);
        }
        let hold = host.hold_for_transition(&record.id, tree).await?;
        let committed = match change {
            ConversationChange::Target(to) => {
                let switched = host.switch_target(&record, to, None).await;
                if let Ok(Some(switched)) = &switched {
                    if switched.made.is_some() {
                        self.mark(Part::Workspaces);
                    }
                    if let Some(device) = &leaving {
                        self.fail_waiting_for(&record, device).await?;
                    }
                    self.switched(&record.id).await;
                }
                switched
            }
            ConversationChange::Record(RecordChange::Archived(true)) => {
                let archived = host.archive(&record).await;
                if archived.is_ok() {
                    self.archived(&record.id);
                }
                archived.map(|()| None)
            }
            ConversationChange::Record(RecordChange::Archived(false)) => {
                // A conversation whose project was deleted while it was
                // archived makes it again (`sessions-and-targets.md`
                // § Resolve a target).
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
            ConversationChange::Settings(_) => {
                unreachable!("a settings change is applied before the hold")
            }
        };
        drop(hold);
        committed
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

    /// Deletes the user's project `id` as one transition (`conversations.md`
    /// § Conversations and projects, `web-api-hosts.md` § Workspaces, devices,
    /// and attached hosts): every conversation in it is held as its archive
    /// holds it, released on its Hosts, and archived in the transaction
    /// that deletes the workspace. A conversation of it whose work runs
    /// refuses the deletion, as it refuses its archive, and nothing
    /// changes; so does one that moved in or was created meanwhile.
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
        let mut holds = Vec::new();
        for conversation in &conversations {
            let record = control
                .conversation(conversation.clone())
                .await?
                .ok_or(ChangeRefusal::NotFound)?;
            let tree = self.reserve_idle_tree(conversation, None)?;
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

    /// The paired device a switch of `record` leaves while its runner is not
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
        Ok((paired && !self.devices().online(device)).then(|| device.clone()))
    }

    /// Reserves the conversation's live tree while it does nothing by itself
    /// (`runtime.md` § Actions): no action runs or waits, no child is live
    /// and no command runs. A switch that `leaves` an offline device takes
    /// the tree with commands running, which run on there, and also a tree
    /// whose every running node only waits for that device: its waiting
    /// jobs were never sent, the switch fails them, and the nodes go on on
    /// the new Host, so nothing is reserved (`sessions-and-targets.md`
    /// § Switch the primary target). A conversation without a live tree runs
    /// nothing; one whose tree works otherwise refuses the transition.
    fn reserve_idle_tree(
        &self,
        id: &ConversationId,
        leaves: Option<&DeviceId>,
    ) -> Result<Option<Reservation>, ChangeRefusal> {
        let Some(tree) = self.agent().tree(&root_of(id)) else {
            return Ok(None);
        };
        // The check and the reservation are one step: no await between them.
        if leaves.is_some() && tree.only_waits() {
            return Ok(None);
        }
        if tree.works() || (tree.runs_commands() && leaves.is_none()) {
            return Err(ChangeRefusal::TurnInFlight);
        }
        tree.admission()
            .try_reserve()
            .map(Some)
            .ok_or(ChangeRefusal::TurnInFlight)
    }

    /// Fails the jobs that wait for `device`, the offline device the switch
    /// of `record` left, with what the agent reads of them: where the
    /// conversation runs now, and that the command did not run
    /// (`sessions-and-targets.md` § Host operations).
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
            let outcome = self.transition_switching(id, change).await;
            // The switch is made; the user's message about it wakes the
            // root, which reads it with the new context block.
            if patch.notify_agent
                && let Ok(Some(switched)) = &outcome
            {
                self.tell_of_move(id, switched).await;
            }
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
        let conversation = self.conversation_summary(record).await?;
        Ok(Some(ConversationUpdate {
            conversation,
            results,
        }))
    }
}

impl Shard {
    /// Tells the conversation's root of the user's move `switched`, through
    /// the entry a permission decision's message takes, which opens a closed
    /// tree and wakes an idle root (`sessions-and-targets.md` § Switch the
    /// primary target). The message's id is the revision the move made, so
    /// delivering it again changes nothing. A message that is not admitted
    /// leaves the move made, and the agent reads it from its next context
    /// block.
    async fn tell_of_move(&self, id: &ConversationId, switched: &Switched) {
        let (from, to) = (&switched.switch.from, &switched.switch.to);
        let content = format!(
            "The user moved this conversation from {} to {}. Files did not move. Check what this means for the work so far, and tell the user.",
            self.place(from).await,
            self.place(to).await,
        );
        let host = self.target_host(to).await;
        let home = to.device().and_then(|device| self.devices().home(device));
        let at = self.services().clock.now();
        let message = |recipient: &NodeId| AgentMessage {
            id: BlockId::try_from(format!("moved:{}", switched.revision))
                .expect("a move's message id is not empty"),
            sender: None,
            recipient_id: recipient.clone(),
            timestamp: at,
            content: content.clone(),
            event: AgentMessageEvent::Moved {
                host: host.clone(),
                path: to.path().to_owned(),
                home: home.clone(),
            },
            origin: None,
        };
        if let Err(error) = self.permission_shard().admit(id, &root_of(id), &message).await {
            tracing::warn!(conversation = %id, "the agent was not told of the move; it reads it from its next context block: {error}");
        }
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
            held_title.as_deref(), Some("Renamed"),
            "the rename applied while a transition held the conversation"
        );
        assert_eq!(title.as_deref(), Some("Renamed"));
        pool.close().await;
    }

    /// A directory is the one its Host names (`sessions-and-targets.md`
    /// § Resolve a target): on Windows `C:/Users/ana/site/` and
    /// `c:\users\ana\site` are the one project `C:\Users\ana\site`, and a
    /// link to the home is the home.
    /// Planted defect this catches: a move that selects by the spelling it
    /// was given, which makes two projects of one directory and a project
    /// of the home.
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

    /// Connects `device` through a runner the test plays, which answers
    /// every conversation release and writes it to `log`.
    async fn runner(shard: &Rc<Shard>, device: &DeviceId, log: &Rc<RefCell<Vec<Released>>>) {
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
                log.borrow_mut().push(Released {
                    device,
                    target: (resolved.device().cloned(), resolved.path().to_owned()),
                    attached: attached.into_iter().map(|host| host.device).collect(),
                    archived: record.archived,
                });
            })
        })
        .await;
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
                let log = Rc::new(RefCell::new(Vec::new()));
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
