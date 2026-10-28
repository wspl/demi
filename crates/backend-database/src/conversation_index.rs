//! The conversation index of the control store.
//!
//! For each conversation it holds:
//!
//! - the owner, title, archive and pin state;
//! - the place in the sidebar and the read revision;
//! - the target and the execution-context revision;
//! - the model selection, which is the conversation's model settings
//!   (`crates/shared-types/src/catalog.rs`);
//! - the Workflow menu choice
//!   (`crates/backend-user-shard/src/conversation/workflows.rs`).
//!
//! The agent tree itself is in the conversation's own database.
//!
//! A conversation's id is the one the web app chose, kept in the case it arrived
//! in and compared without case, so an id another conversation holds in any
//! spelling is taken.

use demi_shared_types::{ModelSelection, Timestamp, WorkflowChoice};
use demi_web_api_protocol::conversations::{ConversationTarget, ModelChoice};
use demi_web_api_protocol::ids::{ConversationId, DeviceId, ProjectId, UserId, WorkspaceId};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use super::StorageError;
use super::columns::{decode, instant, json, to_json, workflow_choice, workflow_columns};
use super::control::ControlService;
use super::devices::CLOUD_NAME;
use super::selection::{Selected, Selection, Unselectable, Worktree, execution_target, select};
use super::projects::ProjectRecord;

/// A conversation as the index holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationRecord {
    pub id: ConversationId,
    pub owner: UserId,
    /// The workspace it is in.
    pub workspace: WorkspaceId,
    /// Its title; none while it is the placeholder, which holds no words
    /// and which the page shows in the reader's language.
    pub title: Option<String>,
    pub archived: bool,
    pub pinned: bool,
    /// The output revision the user last acknowledged; it only moves
    /// forward.
    pub read_revision: u64,
    /// Where it runs (`crates/backend-database/src/selection.rs`).
    pub target: Selection,
    /// Advanced by every change of the conversation's execution context,
    /// such as a target switch or a Host attached.
    pub context_version: u64,
    /// The conversation's model selection; none until its first model is
    /// chosen.
    pub model: Option<ModelSelection>,
    /// Its Workflow menu choice (`crates/backend-user-shard/src/conversation/workflows.rs`).
    pub workflow: WorkflowChoice,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    /// The revision of the conversation's draft, 0 before its first save.
    pub draft_revision: u64,
    /// How many messages the conversation keeps saved for later.
    pub saved: u64,
    /// The revision of that list, 0 before its first change.
    pub saved_revision: u64,
    /// The revision of the conversation's work panel, 0 before its first
    /// change.
    pub panel_revision: u64,
    /// The revision of the conversation's attached hosts, raised by each
    /// change of them; 0 before the first.
    pub hosts_revision: u64,
    /// How many of the conversation's permission requests are undecided.
    pub permission_requests: u64,
    /// How many of those the user has not read yet: none has been
    /// acknowledged since it was raised (`mark_conversation_read`).
    pub unread_permission_requests: u64,
    /// Whether it has a schedule, which the sidebar marks.
    pub scheduled: bool,
}

/// A change a user asks of a conversation, which `Shard::transition`
/// applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationChange {
    Record(RecordChange),
    /// A change of the conversation's model settings, which the
    /// conversation's settings order applies against the catalog.
    Settings(SettingsChange),
    /// A switch of the primary target (`crates/backend-host-access/src/transition.rs`), which the target compare-and-set commits.
    Target(ConversationTarget),
}

impl From<RecordChange> for ConversationChange {
    fn from(change: RecordChange) -> Self {
        Self::Record(change)
    }
}

/// A change of a conversation's model settings: the parts a patch names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsChange {
    /// A switch to this model, with the effort and the tier below, and the
    /// model's first effort and the vendor's default tier for a part they
    /// leave out.
    pub model: Option<ModelChoice>,
    /// The thinking effort: one the model lists, or thinking off.
    pub thinking_effort: Option<String>,
    /// The service tier, or null for the vendor's default.
    pub service_tier_id: Option<Option<String>>,
}

/// A conversation's selection resolved: the device its work runs on, and
/// the directory the work starts in there. A Cloud has no device until its
/// first use makes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[garde(allow_unvalidated)]
pub enum ExecutionTarget {
    Cloud {
        device_id: Option<DeviceId>,
        path: String,
    },
    Device {
        device_id: DeviceId,
        path: String,
    },
    Project {
        project_id: ProjectId,
        device_id: DeviceId,
        path: String,
    },
}

impl ExecutionTarget {
    pub fn device(&self) -> Option<&DeviceId> {
        match self {
            Self::Cloud { device_id, .. } => device_id.as_ref(),
            Self::Device { device_id, .. } | Self::Project { device_id, .. } => Some(device_id),
        }
    }

    pub fn path(&self) -> &str {
        match self {
            Self::Cloud { path, .. } | Self::Device { path, .. } | Self::Project { path, .. } => {
                path
            }
        }
    }
}

/// The latest target switch, which every node's next context block
/// describes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[garde(allow_unvalidated)]
pub struct TargetSwitch {
    pub from: ExecutionTarget,
    pub to: ExecutionTarget,
}

/// A change of a conversation's record, which one index transaction
/// applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordChange {
    /// Archive, or restore.
    Archived(bool),
    /// A rename: a title other than the current one, which becomes the
    /// user's.
    Title(String),
    Pinned(bool),
    /// The conversation's new model selection.
    Model(ModelSelection),
    /// The conversation's new Workflow menu choice.
    Workflow(WorkflowChoice),
    /// A device of the user's attached; one attached already stays as it is.
    Attach(AttachedHostRecord),
    /// A detach of an attached device; a device that is not attached
    /// detaches as nothing.
    Detach(DeviceId),
}

/// What a change found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeOutcome {
    Applied,
    /// A detach of a device that is not attached: nothing changed.
    Unchanged,
    /// No conversation has the id.
    Missing,
    /// The conversation is archived, and the change is not its restore.
    Archived,
}

/// What a new conversation starts with; by default the Cloud, the placeholder
/// title and no model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationStart {
    /// The workspace it is in, the owner's.
    pub workspace: WorkspaceId,
    /// The user's title; the placeholder, which holds no words, when none.
    pub title: Option<String>,
    pub pinned: bool,
    pub target: ConversationTarget,
    pub model: Option<ModelSelection>,
    /// Its Workflow menu choice, which a new conversation is given before
    /// its first message.
    pub workflow: WorkflowChoice,
}

impl ConversationStart {
    /// A new conversation of `workspace` on the Cloud, with the placeholder
    /// title and no model.
    pub fn in_workspace(workspace: WorkspaceId) -> Self {
        Self {
            workspace,
            title: None,
            pinned: false,
            target: ConversationTarget::Cloud { path: None },
            model: None,
            workflow: WorkflowChoice::Default,
        }
    }
}

/// What asking for a conversation of an id found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Creation {
    /// The index had no conversation of the id; it has this new one now.
    Created(ConversationRecord),
    /// The owner's conversation of the id, which a retry finds.
    Existing(ConversationRecord),
    /// Another user's conversation holds the id, or a Fork reserved it.
    Unavailable,
    /// What it was to start in selects nothing, and it was not created.
    Unselectable(Unselectable),
}

/// The sidebar's order within the active or the archived conversations:
/// pinned first, then by the user's order.
const SIDEBAR_ORDER: &str = "pinned DESC, sort_order, id";

const CONVERSATION_COLUMNS: &str = "id, user_id, workspace_id, title, title_origin, archived, pinned, read_revision, target_kind, target_device_id,
     target_path, target_project_id, target_worktree_name, target_worktree_path, context_version, model, workflow_kind,
     workflow_id, workflow_worktree, created_at, updated_at, hosts_revision,
     COALESCE((SELECT revision FROM conversation_drafts WHERE conversation_id = conversations.id), 0) AS draft_revision,
     COALESCE((SELECT saved_revision FROM conversation_drafts WHERE conversation_id = conversations.id), 0) AS saved_revision,
     (SELECT count(*) FROM saved_messages
      WHERE conversation_id = conversations.id
         OR project_id = (SELECT id FROM projects
                            WHERE id = conversations.target_project_id AND saved_scope = 'project')) AS saved,
     COALESCE((SELECT revision FROM conversation_panels WHERE conversation_id = conversations.id), 0) AS panel_revision,
     (SELECT count(*) FROM permission_requests WHERE conversation_id = conversations.id AND decision IS NULL) AS permission_requests,
     (SELECT count(*) FROM permission_requests WHERE conversation_id = conversations.id AND decision IS NULL AND NOT read) AS unread_permission_requests,
     EXISTS (SELECT 1 FROM schedules WHERE conversation_id = conversations.id) AS scheduled";

/// A selection as its typed columns: the kind and what the kind names.
pub struct TargetColumns {
    pub kind: &'static str,
    pub device: Option<String>,
    pub path: Option<String>,
    pub project: Option<String>,
    pub worktree_name: Option<String>,
    pub worktree_path: Option<String>,
}

impl TargetColumns {
    pub fn of(selection: &Selection) -> Self {
        match selection {
            Selection::Cloud { path } => Self {
                kind: "cloud",
                device: None,
                path: path.clone(),
                project: None,
                worktree_name: None,
                worktree_path: None,
            },
            Selection::Home { device_id } => Self {
                kind: "home",
                device: Some(device_id.as_str().to_owned()),
                path: None,
                project: None,
                worktree_name: None,
                worktree_path: None,
            },
            Selection::Project {
                project_id,
                device_id,
                path,
                worktree,
            } => Self {
                kind: "project",
                device: Some(device_id.as_str().to_owned()),
                path: Some(path.clone()),
                project: Some(project_id.as_str().to_owned()),
                worktree_name: worktree.as_ref().map(|worktree| worktree.name.clone()),
                worktree_path: worktree.as_ref().map(|worktree| worktree.path.clone()),
            },
            Selection::Directory { device_id, path } => Self {
                kind: "device",
                device: Some(device_id.as_str().to_owned()),
                path: Some(path.clone()),
                project: None,
                worktree_name: None,
                worktree_path: None,
            },
        }
    }
}

impl ControlService {
    /// The owner's conversation of `id`, which is created when no conversation has
    /// the id.
    ///
    /// The new conversation is first in the owner's sidebar, as `start` says, with
    /// its hosts attached, all in one step. A retry of the owner's finds the one it
    /// created, in the spelling it was created with, and applies nothing of
    /// `start`.
    pub async fn create_conversation(
        &self,
        owner: UserId,
        id: ConversationId,
        start: ConversationStart,
    ) -> Result<Creation, StorageError> {
        Ok(self.create_conversation_selecting(owner, id, start).await?.0)
    }

    /// [`create_conversation`](Self::create_conversation), with the
    /// project its directory's selection made, if it did.
    pub async fn create_conversation_selecting(
        &self,
        owner: UserId,
        id: ConversationId,
        start: ConversationStart,
    ) -> Result<(Creation, Option<ProjectRecord>), StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            // A Fork's destination, and a conversation whose deletion is
            // not finished, keep their ids.
            let reserved = transaction
                .query_row(
                    "SELECT 1 FROM conversation_fork_operations WHERE id = ?1
                     UNION ALL SELECT 1 FROM conversation_deletions WHERE id = ?1",
                    [id.as_str()],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if reserved {
                return Ok((Creation::Unavailable, None));
            }
            if let Some(record) = conversation_by_id(&transaction, &id)? {
                let found = if record.owner == owner { Creation::Existing(record) } else { Creation::Unavailable };
                return Ok((found, None));
            }
            if !super::workspaces::owns(&transaction, &owner, &start.workspace)? {
                return Ok((Creation::Unselectable(Unselectable::WorkspaceNotFound), None));
            }
            let Selected { selection, made } =
                match select(&transaction, &owner, &start.workspace, &id, &start.target, now)? {
                    Ok(selected) => selected,
                    Err(reason) => return Ok((Creation::Unselectable(reason), None)),
                };
            // Before it exists, a conversation's Workflow menu lists its
            // directory's workflows, so in a linked worktree its choice is
            // the worktree's file.
            let workflow = match (&selection, &start.workflow) {
                (Selection::Project { worktree: Some(_), .. }, WorkflowChoice::Workflow { id, worktree: false }) => {
                    WorkflowChoice::Workflow { id: id.clone(), worktree: true }
                }
                (_, workflow) => workflow.clone(),
            };
            let inserted = insert_conversation(
                &transaction,
                &NewConversation {
                    id: &id,
                    owner: &owner,
                    workspace: &start.workspace,
                    title: start.title.as_deref(),
                    pinned: start.pinned,
                    target: &selection,
                    model: start.model.as_ref(),
                    workflow: &workflow,
                    context_version: 0,
                    at: now,
                },
            )?;
            let record =
                conversation_by_id(&transaction, &id)?.ok_or_else(|| StorageError::Corrupt {
                    table: "conversations",
                    column: "id",
                    reason: "a conversation just created or found is missing".into(),
                })?;
            transaction.commit()?;
            Ok(if record.owner != owner {
                (Creation::Unavailable, None)
            } else if inserted == 1 {
                (Creation::Created(record), made)
            } else {
                (Creation::Existing(record), None)
            })
        })
        .await
    }

    /// The conversation of `id`, in whichever case it is spelled.
    pub async fn conversation(
        &self,
        id: ConversationId,
    ) -> Result<Option<ConversationRecord>, StorageError> {
        self.call(move |connection, _| conversation_by_id(connection, &id))
            .await
    }

    /// The conversation's latest target switch with the execution-context
    /// revision it made, which a node that has not read a block of that
    /// revision or a later one reads described; none before its first.
    pub async fn last_switch(
        &self,
        id: ConversationId,
    ) -> Result<Option<(TargetSwitch, u64)>, StorageError> {
        self.call(move |connection, _| {
            let row: Option<(Option<String>, Option<i64>)> = connection
                .query_row(
                    "SELECT last_switch, last_switch_revision FROM conversations WHERE id = ?1",
                    [id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let (text, revision) = match row {
                None | Some((None, None)) => return Ok(None),
                Some((Some(text), Some(revision))) => (text, revision),
                Some(_) => {
                    return Err(StorageError::Corrupt {
                        table: "conversations",
                        column: "last_switch_revision",
                        reason: "a switch and its revision are stored together".into(),
                    });
                }
            };
            let switch = json("conversations", "last_switch", &text)?;
            let revision = decode("conversations", "last_switch_revision", u64::try_from(revision))?;
            Ok(Some((switch, revision)))
        })
        .await
    }

    /// [`conversations`](Self::conversations) as the owner's pages and agents
    /// see them: without an integration's, which only its integration
    /// reaches (`crates/backend-database/src/integrations.rs`).
    pub async fn shown_conversations(
        &self,
        owner: UserId,
        archived: bool,
    ) -> Result<Vec<ConversationRecord>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare(&format!(
                "SELECT {CONVERSATION_COLUMNS} FROM conversations WHERE user_id = ?1 AND archived = ?2
                   AND workspace_id NOT IN (SELECT workspace_id FROM integrations)
                 ORDER BY {SIDEBAR_ORDER}"
            ))?;
            let mut rows = statement.query(params![owner.as_str(), archived])?;
            let mut conversations = Vec::new();
            while let Some(row) = rows.next()? {
                conversations.push(conversation_row(row)?);
            }
            Ok(conversations)
        })
        .await
    }

    /// The owner's conversations that are archived, or that are not, in
    /// sidebar order, an integration's included.
    pub async fn conversations(
        &self,
        owner: UserId,
        archived: bool,
    ) -> Result<Vec<ConversationRecord>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare(&format!(
                "SELECT {CONVERSATION_COLUMNS} FROM conversations WHERE user_id = ?1 AND archived = ?2
                 ORDER BY {SIDEBAR_ORDER}"
            ))?;
            let mut rows = statement.query(params![owner.as_str(), archived])?;
            let mut conversations = Vec::new();
            while let Some(row) = rows.next()? {
                conversations.push(conversation_row(row)?);
            }
            Ok(conversations)
        })
        .await
    }

    /// The owner's conversations a Host is to hold residents for: those not
    /// archived whose target resolves to `device`, or that `device` is
    /// attached to. A target that cannot be resolved holds nothing.
    pub async fn conversations_on(
        &self,
        owner: UserId,
        device: DeviceId,
    ) -> Result<Vec<ConversationId>, StorageError> {
        self.call(move |connection, _| {
            let mut attached = connection.prepare("SELECT conversation_id FROM conversation_hosts WHERE device_id = ?1")?;
            let attached: std::collections::HashSet<String> = attached
                .query_map([device.as_str()], |row| row.get(0))?
                .collect::<Result<_, _>>()?;
            let mut statement = connection.prepare(&format!(
                "SELECT {CONVERSATION_COLUMNS} FROM conversations WHERE user_id = ?1 AND archived = 0"
            ))?;
            let mut rows = statement.query(params![owner.as_str()])?;
            let mut held = Vec::new();
            while let Some(row) = rows.next()? {
                let record = conversation_row(row)?;
                let primary = super::selection::execution_target(connection, &owner, &record.id, &record.target)
                    .is_ok_and(|target| target.device() == Some(&device));
                if primary || attached.contains(record.id.as_str()) {
                    held.push(record.id);
                }
            }
            Ok(held)
        })
        .await
    }

    /// The ids of the owner's conversations in the order the product state
    /// lists them: the active ones in sidebar order, then the archived ones.
    pub async fn conversation_order(
        &self,
        owner: UserId,
    ) -> Result<Vec<ConversationId>, StorageError> {
        self.ordered(owner, "").await
    }

    /// [`conversation_order`](Self::conversation_order) as the owner's pages
    /// list them: without an integration's conversations.
    pub async fn shown_conversation_order(&self, owner: UserId) -> Result<Vec<ConversationId>, StorageError> {
        self.ordered(owner, "AND workspace_id NOT IN (SELECT workspace_id FROM integrations)").await
    }

    async fn ordered(&self, owner: UserId, shown: &'static str) -> Result<Vec<ConversationId>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare(&format!(
                "SELECT id FROM conversations WHERE user_id = ?1 {shown} ORDER BY archived, {SIDEBAR_ORDER}"
            ))?;
            let mut rows = statement.query([owner.as_str()])?;
            let mut ids = Vec::new();
            while let Some(row) = rows.next()? {
                ids.push(decode(
                    "conversations",
                    "id",
                    ConversationId::try_from(row.get::<_, String>(0)?),
                )?);
            }
            Ok(ids)
        })
        .await
    }

    /// Acknowledges the output up to `revision`, and the permission requests
    /// that wait; an acknowledgement never moves the read revision back. A
    /// request raised later is unread. Answers whether anything changed.
    pub async fn mark_conversation_read(
        &self,
        id: ConversationId,
        revision: u64,
    ) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let read = read_output(&transaction, &id, revision)?;
            let requests = transaction.execute(
                "UPDATE permission_requests SET read = 1 WHERE conversation_id = ?1 AND decision IS NULL AND NOT read",
                [id.as_str()],
            )?;
            transaction.commit()?;
            Ok(read + requests > 0)
        })
        .await
    }

    /// Acknowledges the output up to `revision` and nothing else, for output
    /// nobody asked for, such as a compaction while idle: the permission
    /// requests that wait stay unread. Answers whether it changed.
    pub async fn mark_output_read(&self, id: ConversationId, revision: u64) -> Result<bool, StorageError> {
        self.call(move |connection, _| Ok(read_output(connection, &id, revision)? > 0))
            .await
    }

    /// Applies `change` in one transaction. An archived conversation takes
    /// nothing but its restore; a rename that repeats the current title
    /// changes nothing, so the title keeps its origin.
    pub async fn change_conversation(
        &self,
        id: ConversationId,
        change: RecordChange,
    ) -> Result<ChangeOutcome, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let archived: Option<bool> = transaction
                .query_row("SELECT archived FROM conversations WHERE id = ?1", [id.as_str()], |row| row.get(0))
                .optional()?;
            let Some(archived) = archived else {
                return Ok(ChangeOutcome::Missing);
            };
            if archived && !matches!(change, RecordChange::Archived(_)) {
                return Ok(ChangeOutcome::Archived);
            }
            match &change {
                RecordChange::Archived(archived) => {
                    // An archive withdraws the permission requests, decided ones
                    // whose message was not delivered among them, and keeps the
                    // grants.
                    if *archived {
                        archive_in(&transaction, &id)?;
                    } else {
                        restore_in(&transaction, &id, now)?;
                    }
                    1
                }
                RecordChange::Title(title) => transaction.execute(
                    "UPDATE conversations SET title = ?2, title_origin = 'user' WHERE id = ?1 AND title <> ?2",
                    params![id.as_str(), title],
                )?,
                RecordChange::Pinned(pinned) => transaction.execute(
                    "UPDATE conversations SET pinned = ?2 WHERE id = ?1",
                    params![id.as_str(), pinned],
                )?,
                RecordChange::Model(model) => transaction.execute(
                    "UPDATE conversations SET model = ?2 WHERE id = ?1",
                    params![id.as_str(), to_json(model)],
                )?,
                RecordChange::Workflow(choice) => {
                    let (kind, workflow, worktree) = workflow_columns(choice);
                    transaction.execute(
                        "UPDATE conversations SET workflow_kind = ?2, workflow_id = ?3, workflow_worktree = ?4 WHERE id = ?1",
                        params![id.as_str(), kind, workflow, worktree],
                    )?
                }
                RecordChange::Attach(host) => {
                    if insert_attached_host(&transaction, &id, host, now)? {
                        advance_context(&transaction, &id)?;
                    }
                    1
                }
                RecordChange::Detach(device) => {
                    let removed = transaction.execute(
                        "DELETE FROM conversation_hosts WHERE conversation_id = ?1 AND device_id = ?2",
                        params![id.as_str(), device.as_str()],
                    )?;
                    if removed == 0 {
                        return Ok(ChangeOutcome::Unchanged);
                    }
                    advance_context(&transaction, &id)?;
                    raise_hosts_revision(&transaction, &id)?;
                    removed
                }
            };
            transaction.commit()?;
            Ok(ChangeOutcome::Applied)
        })
        .await
    }

    /// The selection `to` names for `owner`'s conversation `id`, as a move
    /// there would make it, without making anything: a project it would
    /// make is not one the conversation's selection can equal.
    pub async fn selection_for(
        &self,
        owner: UserId,
        id: ConversationId,
        to: ConversationTarget,
    ) -> Result<Result<Selection, Unselectable>, StorageError> {
        self.call(move |connection, now| {
            // Dropped without a commit: it rolls back.
            let transaction = connection.transaction()?;
            let Some(workspace) = conversation_workspace(&transaction, &id)? else {
                return Ok(Err(Unselectable::WorkspaceNotFound));
            };
            Ok(select(&transaction, &owner, &workspace, &id, &to, now)?.map(|selected| selected.selection))
        })
        .await
    }

    /// What `owner`'s conversation `id` with `selection` resolves to: its
    /// device and the directory its work starts in. Resolving never starts
    /// a machine.
    pub async fn execution_target(
        &self,
        owner: UserId,
        id: ConversationId,
        selection: Selection,
    ) -> Result<ExecutionTarget, StorageError> {
        self.call(move |connection, _| execution_target(connection, &owner, &id, &selection))
            .await
    }

    /// Makes `title`, from the first message, the conversation's while its
    /// title is still the placeholder; answers whether it did, which makes
    /// this send the one a generated title follows.
    pub async fn title_from_first_message(
        &self,
        id: ConversationId,
        title: String,
    ) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let changed = connection.execute(
                "UPDATE conversations SET title = ?2, title_origin = 'message'
                 WHERE id = ?1 AND title_origin = 'placeholder'",
                params![id.as_str(), title],
            )?;
            Ok(changed == 1)
        })
        .await
    }

    /// Writes the generated `title` while the title is still `from`, the one
    /// its request began from, in the statement that checks it, so a rename
    /// that landed meanwhile stays. Answers whether it was written.
    pub async fn generated_title(
        &self,
        id: ConversationId,
        title: String,
        from: String,
    ) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let written = connection.execute(
                "UPDATE conversations SET title = ?2, title_origin = 'generated' WHERE id = ?1 AND title = ?3",
                params![id.as_str(), title, from],
            )?;
            Ok(written == 1)
        })
        .await
    }

    /// Every user who has a conversation.
    pub async fn conversation_owners(&self) -> Result<Vec<UserId>, StorageError> {
        self.call(|connection, _| {
            let mut statement = connection.prepare("SELECT DISTINCT user_id FROM conversations")?;
            let mut rows = statement.query([])?;
            let mut owners = Vec::new();
            while let Some(row) = rows.next()? {
                owners.push(decode(
                    "conversations",
                    "user_id",
                    UserId::try_from(row.get::<_, String>(0)?),
                )?);
            }
            Ok(owners)
        })
        .await
    }

    /// Records activity in the conversation now. Activity never reorders the
    /// sidebar.
    pub async fn touch_conversation(&self, id: ConversationId) -> Result<(), StorageError> {
        self.call(move |connection, now| {
            connection.execute(
                "UPDATE conversations SET updated_at = ?2 WHERE id = ?1",
                params![id.as_str(), now.as_millisecond()],
            )?;
            Ok(())
        })
        .await
    }
}

/// Moves the conversation's read revision forward to `revision`, never back.
fn read_output(connection: &Connection, id: &ConversationId, revision: u64) -> Result<usize, StorageError> {
    Ok(connection.execute(
        "UPDATE conversations SET read_revision = ?2 WHERE id = ?1 AND read_revision < ?2",
        params![id.as_str(), revision_column(revision)],
    )?)
}

/// A conversation as it enters the index, seen live as it is created.
pub struct NewConversation<'a> {
    pub id: &'a ConversationId,
    pub owner: &'a UserId,
    pub workspace: &'a WorkspaceId,
    /// The user's title, origin `user`; none for the placeholder.
    pub title: Option<&'a str>,
    pub pinned: bool,
    pub target: &'a Selection,
    pub model: Option<&'a ModelSelection>,
    pub workflow: &'a WorkflowChoice,
    /// The execution-context revision it starts at: 0, or a Fork's source's
    /// at the Fork, which the context blocks its copied transcript name.
    pub context_version: u64,
    /// When it was created, and last active.
    pub at: Timestamp,
}

/// Inserts `new` first in its owner's sidebar, unarchived, unless a
/// conversation has its id in any spelling; answers the rows it inserted.
pub fn insert_conversation(
    connection: &Connection,
    new: &NewConversation<'_>,
) -> Result<usize, StorageError> {
    let target = TargetColumns::of(new.target);
    let (workflow_kind, workflow_id, workflow_worktree) = workflow_columns(new.workflow);
    let inserted = connection.execute(
        "INSERT INTO conversations (id, user_id, title, title_origin, archived, pinned, sort_order,
           read_revision, target_kind, target_device_id, target_path, target_project_id, context_version,
           model, created_at, updated_at, workflow_kind, workflow_id, target_worktree_name, target_worktree_path,
           workflow_worktree, workspace_id)
         VALUES (?1, ?2, ?3, ?4, 0, ?11,
           (SELECT COALESCE(MIN(sort_order), 0) - 1 FROM conversations WHERE user_id = ?2),
           0, ?5, ?6, ?7, ?8, ?14, ?9, ?10, ?10, ?12, ?13, ?15, ?16, ?17, ?18)
         ON CONFLICT (id) DO NOTHING",
        params![
            new.id.as_str(),
            new.owner.as_str(),
            new.title.unwrap_or_default(),
            if new.title.is_some() { "user" } else { "placeholder" },
            target.kind,
            target.device,
            target.path,
            target.project,
            new.model.map(to_json),
            new.at.as_millisecond(),
            new.pinned,
            workflow_kind,
            workflow_id,
            revision_column(new.context_version),
            target.worktree_name,
            target.worktree_path,
            workflow_worktree,
            new.workspace.as_str()
        ],
    )?;
    Ok(inserted)
}

/// A revision as the INTEGER column holds it. The output revision advances
/// by one per save, so no revision the backend acknowledges comes near
/// `i64::MAX`; a larger one is held as the largest the column keeps, which
/// acknowledges everything as the larger number would.
fn revision_column(revision: u64) -> i64 {
    i64::try_from(revision).unwrap_or(i64::MAX)
}

pub fn conversation_by_id(
    connection: &Connection,
    id: &ConversationId,
) -> Result<Option<ConversationRecord>, StorageError> {
    let mut statement = connection.prepare(&format!(
        "SELECT {CONVERSATION_COLUMNS} FROM conversations WHERE id = ?1"
    ))?;
    let mut rows = statement.query([id.as_str()])?;
    rows.next()?.map(conversation_row).transpose()
}

/// The choice a `conversations` row's workflow columns hold.
fn workflow_row(row: &Row<'_>) -> Result<WorkflowChoice, StorageError> {
    let kind: String = row.get("workflow_kind")?;
    workflow_choice("conversations", "workflow_kind", &kind, row.get("workflow_id")?, row.get("workflow_worktree")?)
}

/// A `conversations` row, read from its columns in `CONVERSATION_COLUMNS`.
/// A conversation's title: none for the placeholder, whose column is
/// empty, and the words of any other origin, which are never empty.
fn title_row(row: &Row<'_>) -> Result<Option<String>, StorageError> {
    let title: String = row.get("title")?;
    let placeholder = row.get::<_, String>("title_origin")? == "placeholder";
    if placeholder != title.is_empty() {
        return Err(StorageError::Corrupt {
            table: "conversations",
            column: "title",
            reason: "a placeholder title holds words, or another title none".into(),
        });
    }
    Ok((!placeholder).then_some(title))
}

/// The workspace the conversation `id` is in; none when no conversation has
/// the id.
pub(crate) fn conversation_workspace(
    connection: &Connection,
    id: &ConversationId,
) -> Result<Option<WorkspaceId>, StorageError> {
    connection
        .query_row("SELECT workspace_id FROM conversations WHERE id = ?1", [id.as_str()], |row| row.get::<_, String>(0))
        .optional()?
        .map(|workspace| decode("conversations", "workspace_id", WorkspaceId::try_from(workspace)))
        .transpose()
}

fn conversation_row(row: &Row<'_>) -> Result<ConversationRecord, StorageError> {
    const TABLE: &str = "conversations";
    let model: Option<String> = row.get("model")?;
    let model = model.map(|text| json(TABLE, "model", &text)).transpose()?;
    Ok(ConversationRecord {
        id: decode(
            TABLE,
            "id",
            ConversationId::try_from(row.get::<_, String>("id")?),
        )?,
        owner: decode(
            TABLE,
            "user_id",
            UserId::try_from(row.get::<_, String>("user_id")?),
        )?,
        workspace: decode(TABLE, "workspace_id", WorkspaceId::try_from(row.get::<_, String>("workspace_id")?))?,
        title: title_row(row)?,
        archived: row.get("archived")?,
        pinned: row.get("pinned")?,
        read_revision: decode(
            TABLE,
            "read_revision",
            u64::try_from(row.get::<_, i64>("read_revision")?),
        )?,
        target: target_row(row)?,
        context_version: decode(
            TABLE,
            "context_version",
            u64::try_from(row.get::<_, i64>("context_version")?),
        )?,
        model,
        workflow: workflow_row(row)?,
        created_at: instant(row, TABLE, "created_at")?,
        updated_at: instant(row, TABLE, "updated_at")?,
        draft_revision: decode(
            "conversation_drafts",
            "revision",
            u64::try_from(row.get::<_, i64>("draft_revision")?),
        )?,
        saved: decode(
            "saved_messages",
            "id",
            u64::try_from(row.get::<_, i64>("saved")?),
        )?,
        saved_revision: decode(
            "conversation_drafts",
            "saved_revision",
            u64::try_from(row.get::<_, i64>("saved_revision")?),
        )?,
        panel_revision: decode(
            "conversation_panels",
            "revision",
            u64::try_from(row.get::<_, i64>("panel_revision")?),
        )?,
        hosts_revision: decode(
            TABLE,
            "hosts_revision",
            u64::try_from(row.get::<_, i64>("hosts_revision")?),
        )?,
        permission_requests: decode(
            "permission_requests",
            "id",
            u64::try_from(row.get::<_, i64>("permission_requests")?),
        )?,
        unread_permission_requests: decode(
            "permission_requests",
            "read",
            u64::try_from(row.get::<_, i64>("unread_permission_requests")?),
        )?,
        scheduled: row.get("scheduled")?,
    })
}

/// The selection a row's typed columns hold.
fn target_row(row: &Row<'_>) -> Result<Selection, StorageError> {
    const TABLE: &str = "conversations";
    let kind: String = row.get("target_kind")?;
    let path: Option<String> = row.get("target_path")?;
    let device = || -> Result<DeviceId, StorageError> {
        let device: Option<String> = row.get("target_device_id")?;
        decode(TABLE, "target_device_id", DeviceId::try_from(device.unwrap_or_default()))
    };
    let path_of = |path: Option<String>| -> Result<String, StorageError> {
        path.filter(|path| !path.is_empty()).ok_or_else(|| StorageError::Corrupt {
            table: TABLE,
            column: "target_path",
            reason: "a directory selection without its directory".into(),
        })
    };
    Ok(match kind.as_str() {
        "cloud" => Selection::Cloud { path },
        "home" => Selection::Home { device_id: device()? },
        "device" => Selection::Directory {
            device_id: device()?,
            path: path_of(path)?,
        },
        "project" => {
            let project: Option<String> = row.get("target_project_id")?;
            let name: Option<String> = row.get("target_worktree_name")?;
            let worktree: Option<String> = row.get("target_worktree_path")?;
            Selection::Project {
                project_id: decode(
                    TABLE,
                    "target_project_id",
                    ProjectId::try_from(project.unwrap_or_default()),
                )?,
                device_id: device()?,
                path: path_of(path)?,
                worktree: name.zip(worktree).map(|(name, path)| Worktree { name, path }),
            }
        }
        other => {
            return Err(StorageError::Corrupt {
                table: TABLE,
                column: "target_kind",
                reason: format!("unknown target kind {other}"),
            });
        }
    })
}

/// A device attached to a conversation: a Host the conversation reaches besides its primary one.
/// A Fork keeps its source's in its operation's metadata. It has no name of
/// its own: it goes by its device's, which [`AttachedHost`] carries as read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttachedHostRecord {
    #[garde(skip)]
    pub device: DeviceId,
    /// The directory its commands start in, fixed when it was attached: the
    /// conversation's directory there for a departed primary target; none
    /// for a device `demi host attach` attached, whose commands start in its
    /// home.
    #[serde(deserialize_with = "Option::deserialize")]
    #[garde(skip)]
    pub cwd: Option<String>,
}

/// An attached host as the conversation reaches it: its device, the name it
/// goes by, which is its device's current name, with `-2`, `-3` and so on
/// after the name when the primary Host or a host attached before it goes by
/// that name already, and the directory its commands start in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachedHost {
    pub device: DeviceId,
    pub name: String,
    pub cwd: Option<String>,
}

impl AttachedHost {
    /// What storage keeps of it, which a Fork copies.
    pub fn record(&self) -> AttachedHostRecord {
        AttachedHostRecord {
            device: self.device.clone(),
            cwd: self.cwd.clone(),
        }
    }
}

impl ControlService {
    /// The conversation's attached hosts, first attached first.
    pub async fn attached_hosts(
        &self,
        id: ConversationId,
    ) -> Result<Vec<AttachedHost>, StorageError> {
        let listed = self
            .call(move |connection, _| attached_rows(connection, &id))
            .await?;
        Ok(listed.into_iter().map(|(host, _)| host).collect())
    }

    /// The conversation's attached hosts with when each was attached, first
    /// attached first, as the web app lists them.
    pub async fn attached_host_listing(
        &self,
        id: ConversationId,
    ) -> Result<Vec<(AttachedHost, Timestamp)>, StorageError> {
        self.call(move |connection, _| attached_rows(connection, &id))
            .await
    }
}

/// The two ends of a target switch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchEnds {
    /// The device the switch leaves, attached afterwards where it was left.
    pub departed: Option<(DeviceId, String)>,
    /// The device the switch reaches, detached if it was attached: a Host is
    /// primary or attached, never both.
    pub arriving: Option<DeviceId>,
}

/// What a target switch's write did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwitchWrite {
    /// The switch is made: where it went from and to, the execution-context
    /// revision it made, and the project its destination's selection
    /// made, if it did.
    Switched {
        switch: TargetSwitch,
        revision: u64,
        made: Option<ProjectRecord>,
    },
    /// The destination selects what the conversation has: nothing changed.
    Unchanged,
    /// The selection is no longer the one the switch started from.
    Conflict,
    Unselectable(Unselectable),
}

impl ControlService {
    /// The target switch's write, against `expected`, the selection the switch
    /// started from, and `from`, what it resolved to.
    ///
    /// It selects `to`'s directory, which makes its project when the user has
    /// none (`crates/backend-database/src/selection.rs`), all in one transaction,
    /// so of two switches from one selection exactly one wins.
    ///
    /// The winner records the switch for every node's next context block and
    /// advances the execution-context revision, which it answers. An archived
    /// conversation does not move.
    pub async fn switch_conversation_target(
        &self,
        id: ConversationId,
        expected: Selection,
        from: ExecutionTarget,
        to: ConversationTarget,
        ends: SwitchEnds,
    ) -> Result<SwitchWrite, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let owner: Option<String> = transaction
                .query_row("SELECT user_id FROM conversations WHERE id = ?1", [id.as_str()], |row| row.get(0))
                .optional()?;
            let Some(owner) = owner else {
                return Ok(SwitchWrite::Conflict);
            };
            let owner = decode("conversations", "user_id", UserId::try_from(owner))?;
            let Some(workspace) = conversation_workspace(&transaction, &id)? else {
                return Ok(SwitchWrite::Conflict);
            };
            let Selected { selection, made } = match select(&transaction, &owner, &workspace, &id, &to, now)? {
                Ok(selected) => selected,
                Err(reason) => return Ok(SwitchWrite::Unselectable(reason)),
            };
            if selection == expected {
                return Ok(SwitchWrite::Unchanged);
            }
            let switch = TargetSwitch {
                from,
                to: execution_target(&transaction, &owner, &id, &selection)?,
            };
            let before = TargetColumns::of(&expected);
            let target = TargetColumns::of(&selection);
            // `IS` compares NULL as equal to NULL, which `=` does not.
            let revision: Option<i64> = transaction.query_row(
                "UPDATE conversations SET target_kind = ?2, target_device_id = ?3, target_path = ?4,
                   target_project_id = ?5, target_worktree_name = ?12, target_worktree_path = ?13,
                   last_switch = ?6, last_switch_revision = context_version + 1,
                   context_version = context_version + 1, updated_at = ?7
                 WHERE id = ?1 AND archived = 0 AND target_kind = ?8 AND target_device_id IS ?9
                   AND target_path IS ?10 AND target_project_id IS ?11 AND target_worktree_path IS ?14
                 RETURNING context_version",
                params![
                    id.as_str(),
                    target.kind,
                    target.device,
                    target.path,
                    target.project,
                    to_json(&switch),
                    now.as_millisecond(),
                    before.kind,
                    before.device,
                    before.path,
                    before.project,
                    target.worktree_name,
                    target.worktree_path,
                    before.worktree_path
                ],
                |row| row.get(0),
            ).optional()?;
            let Some(revision) = revision else {
                return Ok(SwitchWrite::Conflict);
            };
            let revision = decode("conversations", "context_version", u64::try_from(revision))?;
            let project = |columns: &TargetColumns| {
                columns
                    .project
                    .as_ref()
                    .map(|id| decode("conversations", "target_project_id", ProjectId::try_from(id.as_str())))
                    .transpose()
            };
            super::saved::moved(&transaction, &id, project(&before)?.as_ref(), project(&target)?.as_ref(), now)?;
            if let Some(arriving) = &ends.arriving {
                let removed = transaction.execute(
                    "DELETE FROM conversation_hosts WHERE conversation_id = ?1 AND device_id = ?2",
                    params![id.as_str(), arriving.as_str()],
                )?;
                if removed > 0 {
                    raise_hosts_revision(&transaction, &id)?;
                }
            }
            if let Some((departed, cwd)) = &ends.departed
                && Some(departed) != ends.arriving.as_ref()
            {
                let host = AttachedHostRecord {
                    device: departed.clone(),
                    cwd: Some(cwd.clone()),
                };
                insert_attached_host(&transaction, &id, &host, now)?;
            }
            // The attached hosts' names avoid the primary Host's, so a new
            // primary can change them.
            let attached: bool = transaction.query_row(
                "SELECT EXISTS (SELECT 1 FROM conversation_hosts WHERE conversation_id = ?1)",
                [id.as_str()],
                |row| row.get(0),
            )?;
            if attached {
                raise_hosts_revision(&transaction, &id)?;
            }
            transaction.commit()?;
            Ok(SwitchWrite::Switched { switch, revision, made })
        })
        .await
    }
}

/// Archives the conversation `id`. An archive withdraws the permission
/// requests, decided ones whose message was not delivered among them, and
/// keeps the grants.
pub(crate) fn archive_in(connection: &Connection, id: &ConversationId) -> Result<(), StorageError> {
    connection.execute("DELETE FROM permission_requests WHERE conversation_id = ?1", [id.as_str()])?;
    connection.execute("UPDATE conversations SET archived = 1 WHERE id = ?1", [id.as_str()])?;
    Ok(())
}

/// Restores the conversation `id`. One whose project was deleted while it
/// was archived brings it back (`crates/backend-database/src/projects.rs`);
/// one whose project is gone moves back to its directory, into that
/// directory's project, made again (`crates/backend-database/src/selection.rs`);
/// one whose device was revoked meanwhile keeps its directory there.
///
/// Its schedules count from now in the same step, as a schedule turned on
/// does, so nothing its archive held back fires: no time it passed, and no
/// change a watch's command shows since, whose next check only records its
/// output again.
fn restore_in(connection: &Connection, id: &ConversationId, now: Timestamp) -> Result<(), StorageError> {
    connection.execute("UPDATE conversations SET archived = 0 WHERE id = ?1", [id.as_str()])?;
    connection.execute(
        "UPDATE schedules SET since = ?2, watch_stdout = NULL, watch_digest = NULL, watch_exit = NULL,
           watch_checked_at = NULL
         WHERE conversation_id = ?1",
        rusqlite::params![id.as_str(), now.as_millisecond()],
    )?;
    let Some(record) = conversation_by_id(connection, id)? else {
        return Ok(());
    };
    let Selection::Project { project_id, device_id, path, worktree } = &record.target else {
        return Ok(());
    };
    // A worktree's conversation goes back to the worktree, which, its
    // project gone, is a project as any directory is.
    let path = worktree.as_ref().map_or(path, |worktree| &worktree.path);
    if super::projects::project_in(connection, project_id)?.is_some() {
        super::projects::revive(connection, project_id)?;
        return Ok(());
    }
    let Some(device) = super::devices::device_in(connection, device_id)? else {
        return Ok(());
    };
    let selected = super::selection::project_of(connection, &record.workspace, &device, path, now)?;
    super::devices::set_selection(connection, id, &selected.selection)?;
    let to = match &selected.selection {
        Selection::Project { project_id, .. } => Some(project_id),
        _ => None,
    };
    super::saved::moved(connection, id, None, to, now)
}

/// The conversation's attached hosts, first attached first, each by its device's
/// current name.
///
/// - A host whose name the primary Host or one attached before it goes by already
///   takes the first free `name-2`, `name-3` and so on, so no name stands for two
///   hosts and a name stays the same device while the target, the hosts and their
///   devices' names do.
/// - Hosts attached at one time, as a Fork's are, keep the order they were
///   attached in, which their row ids hold.
/// - The primary Host goes by the name the header shows: its device's, Cloud for
///   the Cloud.
fn attached_rows(
    connection: &Connection,
    id: &ConversationId,
) -> Result<Vec<(AttachedHost, Timestamp)>, StorageError> {
    const TABLE: &str = "conversation_hosts";
    let primary: Option<String> = connection
        .prepare_cached(
            "SELECT CASE
               WHEN conversation.target_kind = 'cloud' THEN ?2
               ELSE (SELECT CASE device.kind WHEN 'managed' THEN ?2 ELSE device.name END
                     FROM devices device WHERE device.id = conversation.target_device_id)
             END
             FROM conversations conversation WHERE conversation.id = ?1",
        )?
        .query_row(params![id.as_str(), CLOUD_NAME], |row| row.get(0))
        .optional()?
        .flatten();
    let mut statement = connection.prepare_cached(
        "SELECT host.device_id, device.name, host.cwd, host.attached_at
         FROM conversation_hosts host JOIN devices device ON device.id = host.device_id
         WHERE host.conversation_id = ?1
         ORDER BY host.attached_at, host.rowid",
    )?;
    let mut rows = statement.query([id.as_str()])?;
    let mut taken: std::collections::HashSet<String> = primary.into_iter().collect();
    let mut hosts = Vec::new();
    while let Some(row) = rows.next()? {
        let device: String = row.get("name")?;
        let mut name = device.clone();
        let mut suffix = 2;
        while taken.contains(&name) {
            name = format!("{device}-{suffix}");
            suffix += 1;
        }
        taken.insert(name.clone());
        let host = AttachedHost {
            device: decode(
                TABLE,
                "device_id",
                DeviceId::try_from(row.get::<_, String>("device_id")?),
            )?,
            name,
            cwd: row.get("cwd")?,
        };
        hosts.push((host, instant(row, TABLE, "attached_at")?));
    }
    Ok(hosts)
}

/// Attaches `host` to the conversation. A device attached already keeps its
/// row, and the answer is false. For a transaction that attaches hosts with
/// its other writes, such as a target switch's or a Fork's.
pub fn insert_attached_host(
    connection: &Connection,
    conversation: &ConversationId,
    host: &AttachedHostRecord,
    now: Timestamp,
) -> Result<bool, StorageError> {
    let inserted = connection.execute(
        "INSERT INTO conversation_hosts (conversation_id, device_id, cwd, attached_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (conversation_id, device_id) DO NOTHING",
        params![
            conversation.as_str(),
            host.device.as_str(),
            host.cwd,
            now.as_millisecond()
        ],
    )? == 1;
    if inserted {
        raise_hosts_revision(connection, conversation)?;
    }
    add_reachable_host(connection, conversation, &host.device, now)?;
    Ok(inserted)
}

/// Puts `device` on the list of hosts the conversation may reach, where it
/// stays for the conversation's life; a device gone meanwhile, such as one
/// revoked, joins nothing.
pub fn add_reachable_host(
    connection: &Connection,
    conversation: &ConversationId,
    device: &DeviceId,
    now: Timestamp,
) -> Result<(), StorageError> {
    connection.execute(
        "INSERT INTO reachable_hosts (conversation_id, device_id, added_at)
         SELECT ?1, id, ?3 FROM devices WHERE id = ?2
         ON CONFLICT (conversation_id, device_id) DO NOTHING",
        params![conversation.as_str(), device.as_str(), now.as_millisecond()],
    )?;
    Ok(())
}

/// Whether the conversation may reach `device`.
///
/// It may when the device is:
///
/// - the conversation's primary Host;
/// - the device of its project, or the user's Cloud for a conversation there;
/// - on its list.
///
/// Its agents work on any such host without asking again: attach it, detach it,
/// move to it. Any other host, the Cloud included, needs the user's permission
/// when the agent first tries, one host per request, and an allowed host stays on
/// the list for the conversation's life.
///
/// Listing the user's devices asks nothing: seeing a device is not reaching it.
pub fn may_reach(
    connection: &Connection,
    conversation: &ConversationId,
    device: &DeviceId,
) -> Result<bool, StorageError> {
    Ok(connection.query_row(
        "SELECT EXISTS (SELECT 1 FROM reachable_hosts WHERE conversation_id = ?1 AND device_id = ?2)
             OR EXISTS (
               SELECT 1 FROM conversations c WHERE c.id = ?1 AND (
                 c.target_device_id = ?2
                 OR (c.target_kind = 'cloud'
                   AND ?2 = (SELECT u.cloud_device_id FROM users u WHERE u.id = c.user_id))))",
        params![conversation.as_str(), device.as_str()],
        |row| row.get(0),
    )?)
}

/// Raises the revision of the conversation's attached hosts, which its
/// summary carries, so a page that shows them reads them again.
pub fn raise_hosts_revision(
    connection: &Connection,
    conversation: &ConversationId,
) -> Result<(), StorageError> {
    connection.execute(
        "UPDATE conversations SET hosts_revision = hosts_revision + 1 WHERE id = ?1",
        [conversation.as_str()],
    )?;
    Ok(())
}

/// Advances the conversation's execution-context revision, which every node
/// observes before its next inference.
pub(crate) fn advance_context(
    connection: &Connection,
    conversation: &ConversationId,
) -> Result<(), StorageError> {
    connection.execute(
        "UPDATE conversations SET context_version = context_version + 1 WHERE id = ?1",
        [conversation.as_str()],
    )?;
    Ok(())
}
