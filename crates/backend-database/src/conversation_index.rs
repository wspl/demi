//! The conversation index of the control store (`storage.md` § Control
//! records): each conversation's owner, title, archive and pin state, place
//! in the sidebar, read revision, target, execution-context revision and
//! model selection, which is the conversation's model settings (`models.md`
//! § A conversation's model settings), and Workflow menu choice
//! (`subagents.md` § Workflows of a project). The agent tree itself is in the
//! conversation's own database. A conversation's id is the one the web app chose, kept in the
//! case it arrived in and compared without case, so an id another
//! conversation holds in any spelling is taken.

use demi_shared_types::{ModelSelection, Timestamp, WorkflowChoice};
use demi_web_api_protocol::conversations::{ConversationTarget, ModelChoice};
use demi_web_api_protocol::ids::{ConversationId, DeviceId, UserId, WorkspaceId};
use garde::Validate;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use super::StorageError;
use super::columns::{decode, instant, json, to_json, workflow_choice, workflow_columns};
use super::control::ControlService;
use super::devices::CLOUD_NAME;
use super::selection::{Selected, Selection, Unselectable, execution_target, select};
use super::workspaces::WorkspaceRecord;

/// A conversation as the index holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationRecord {
    pub id: ConversationId,
    pub owner: UserId,
    /// Its title; none while it is the placeholder, which holds no words
    /// and which the page shows in the reader's language (`conversations.md`
    /// § Conversation titles).
    pub title: Option<String>,
    pub archived: bool,
    pub pinned: bool,
    /// The output revision the user last acknowledged; it only moves
    /// forward.
    pub read_revision: u64,
    /// Where it runs (`sessions-and-targets.md` § Resolve a target).
    pub target: Selection,
    /// Advanced by every change of the conversation's execution context,
    /// such as a target switch or a Host attached.
    pub context_version: u64,
    /// The conversation's model selection; none until its first model is
    /// chosen.
    pub model: Option<ModelSelection>,
    /// Its Workflow menu choice (`subagents.md` § Workflows of a project).
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
}

/// A change a user asks of a conversation, which `Shard::transition`
/// applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationChange {
    Record(RecordChange),
    /// A change of the conversation's model settings, which the
    /// conversation's settings order applies against the catalog.
    Settings(SettingsChange),
    /// A switch of the primary target (`sessions-and-targets.md` § Switch the
    /// primary target), which the target compare-and-set commits.
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
    Workspace {
        workspace_id: WorkspaceId,
        device_id: DeviceId,
        path: String,
    },
}

impl ExecutionTarget {
    pub fn device(&self) -> Option<&DeviceId> {
        match self {
            Self::Cloud { device_id, .. } => device_id.as_ref(),
            Self::Device { device_id, .. } | Self::Workspace { device_id, .. } => Some(device_id),
        }
    }

    pub fn path(&self) -> &str {
        match self {
            Self::Cloud { path, .. } | Self::Device { path, .. } | Self::Workspace { path, .. } => {
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
    /// A device of the user's attached (`sessions-and-targets.md` § Attached
    /// hosts); one attached already stays as it is.
    Attach(AttachedHostRecord),
    /// A detach of an attached device; a device that is not attached
    /// detaches as nothing.
    Detach(DeviceId),
}

/// What a change found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeOutcome {
    Applied,
    /// No conversation has the id.
    Missing,
    /// The conversation is archived, and the change is not its restore.
    Archived,
}

/// What a new conversation starts with (`web-api.md` § Conversation
/// creation and Fork); by default the Cloud, the placeholder title and no
/// model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationStart {
    /// The user's title; the placeholder, which holds no words, when none.
    pub title: Option<String>,
    pub pinned: bool,
    pub target: ConversationTarget,
    pub model: Option<ModelSelection>,
    /// Its Workflow menu choice, which a new conversation is given before
    /// its first message.
    pub workflow: WorkflowChoice,
}

impl Default for ConversationStart {
    fn default() -> Self {
        Self {
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

const CONVERSATION_COLUMNS: &str = "id, user_id, title, title_origin, archived, pinned, read_revision, target_kind, target_device_id,
     target_path, target_workspace_id, context_version, model, workflow_kind, workflow_id, created_at, updated_at, hosts_revision,
     COALESCE((SELECT revision FROM conversation_drafts WHERE conversation_id = conversations.id), 0) AS draft_revision,
     COALESCE((SELECT saved_revision FROM conversation_drafts WHERE conversation_id = conversations.id), 0) AS saved_revision,
     (SELECT count(*) FROM saved_messages WHERE conversation_id = conversations.id) AS saved,
     COALESCE((SELECT revision FROM conversation_panels WHERE conversation_id = conversations.id), 0) AS panel_revision,
     (SELECT count(*) FROM permission_requests WHERE conversation_id = conversations.id AND decision IS NULL) AS permission_requests,
     (SELECT count(*) FROM permission_requests WHERE conversation_id = conversations.id AND decision IS NULL AND NOT read) AS unread_permission_requests";

/// A selection as its typed columns: the kind and what the kind names.
pub struct TargetColumns {
    pub kind: &'static str,
    pub device: Option<String>,
    pub path: Option<String>,
    pub workspace: Option<String>,
}

impl TargetColumns {
    pub fn of(selection: &Selection) -> Self {
        match selection {
            Selection::Cloud { path } => Self {
                kind: "cloud",
                device: None,
                path: path.clone(),
                workspace: None,
            },
            Selection::Home { device_id } => Self {
                kind: "home",
                device: Some(device_id.as_str().to_owned()),
                path: None,
                workspace: None,
            },
            Selection::Workspace {
                workspace_id,
                device_id,
                path,
            } => Self {
                kind: "workspace",
                device: Some(device_id.as_str().to_owned()),
                path: Some(path.clone()),
                workspace: Some(workspace_id.as_str().to_owned()),
            },
            Selection::Directory { device_id, path } => Self {
                kind: "device",
                device: Some(device_id.as_str().to_owned()),
                path: Some(path.clone()),
                workspace: None,
            },
        }
    }
}

/// A destination as the pending move's typed columns hold it.
struct DestinationColumns {
    kind: &'static str,
    device: Option<String>,
    path: Option<String>,
    workspace: Option<String>,
}

impl DestinationColumns {
    fn of(target: &ConversationTarget) -> Self {
        match target {
            ConversationTarget::Cloud { path } => Self {
                kind: "cloud",
                device: None,
                path: path.clone(),
                workspace: None,
            },
            ConversationTarget::Device { device_id, path } => Self {
                kind: "device",
                device: Some(device_id.as_str().to_owned()),
                path: Some(path.clone()),
                workspace: None,
            },
            ConversationTarget::Workspace { workspace_id } => Self {
                kind: "workspace",
                device: None,
                path: None,
                workspace: Some(workspace_id.as_str().to_owned()),
            },
        }
    }
}

impl ControlService {
    /// The owner's conversation of `id`, which is created when no
    /// conversation has the id: first in the owner's sidebar, as `start`
    /// says, with its hosts attached, all in one step. A retry of the
    /// owner's finds the one it created, in the spelling it was created
    /// with, and applies nothing of `start`.
    pub async fn create_conversation(
        &self,
        owner: UserId,
        id: ConversationId,
        start: ConversationStart,
    ) -> Result<Creation, StorageError> {
        Ok(self.create_conversation_selecting(owner, id, start).await?.0)
    }

    /// [`create_conversation`](Self::create_conversation), with the
    /// workspace its directory's selection made, if it did.
    pub async fn create_conversation_selecting(
        &self,
        owner: UserId,
        id: ConversationId,
        start: ConversationStart,
    ) -> Result<(Creation, Option<WorkspaceRecord>), StorageError> {
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
            let Selected { selection, made } = match select(&transaction, &owner, &id, &start.target, now)? {
                Ok(selected) => selected,
                Err(reason) => return Ok((Creation::Unselectable(reason), None)),
            };
            let inserted = insert_conversation(
                &transaction,
                &NewConversation {
                    id: &id,
                    owner: &owner,
                    title: start.title.as_deref(),
                    pinned: start.pinned,
                    target: &selection,
                    model: start.model.as_ref(),
                    workflow: &start.workflow,
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

    /// The owner's conversations that are archived, or that are not, in
    /// sidebar order.
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

    /// The ids of the owner's conversations in the order the product state
    /// lists them: the active ones in sidebar order, then the archived ones.
    pub async fn conversation_order(
        &self,
        owner: UserId,
    ) -> Result<Vec<ConversationId>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare(&format!(
                "SELECT id FROM conversations WHERE user_id = ?1 ORDER BY archived, {SIDEBAR_ORDER}"
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
    /// request raised later is unread (`conversations.md` § Conversations and
    /// projects).
    pub async fn mark_conversation_read(
        &self,
        id: ConversationId,
        revision: u64,
    ) -> Result<(), StorageError> {
        let revision = revision_column(revision);
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            transaction.execute(
                "UPDATE conversations SET read_revision = MAX(read_revision, ?2) WHERE id = ?1",
                params![id.as_str(), revision],
            )?;
            transaction.execute(
                "UPDATE permission_requests SET read = 1 WHERE conversation_id = ?1 AND decision IS NULL",
                [id.as_str()],
            )?;
            transaction.commit()?;
            Ok(())
        })
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
                    // An archive withdraws the permission requests, decided
                    // ones whose message was not delivered among them, and
                    // keeps the grants (`permissions.md` § Requests).
                    // It drops the move and the detaches an agent asked for
                    // as well (`sessions-and-targets.md` § Switch the primary
                    // target).
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
                    let (kind, workflow) = workflow_columns(choice);
                    transaction.execute(
                        "UPDATE conversations SET workflow_kind = ?2, workflow_id = ?3 WHERE id = ?1",
                        params![id.as_str(), kind, workflow],
                    )?
                }
                RecordChange::Attach(host) => {
                    if insert_attached_host(&transaction, &id, host, now)? {
                        advance_context(&transaction, &id)?;
                    } else {
                        // Attached already: an attach cancels the detach an
                        // agent asked for.
                        transaction.execute(
                            "UPDATE conversation_hosts SET detaching = 0
                             WHERE conversation_id = ?1 AND device_id = ?2",
                            params![id.as_str(), host.device.as_str()],
                        )?;
                    }
                    1
                }
                RecordChange::Detach(device) => {
                    let removed = transaction.execute(
                        "DELETE FROM conversation_hosts WHERE conversation_id = ?1 AND device_id = ?2",
                        params![id.as_str(), device.as_str()],
                    )?;
                    if removed > 0 {
                        advance_context(&transaction, &id)?;
                        raise_hosts_revision(&transaction, &id)?;
                    }
                    removed
                }
            };
            transaction.commit()?;
            Ok(ChangeOutcome::Applied)
        })
        .await
    }

    /// The selection `to` names for `owner`'s conversation `id`, as a move
    /// there would make it, without making anything: a workspace it would
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
            Ok(select(&transaction, &owner, &id, &to, now)?.map(|selected| selected.selection))
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

/// A conversation as it enters the index, seen live as it is created.
pub struct NewConversation<'a> {
    pub id: &'a ConversationId,
    pub owner: &'a UserId,
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
    let (workflow_kind, workflow_id) = workflow_columns(new.workflow);
    let inserted = connection.execute(
        "INSERT INTO conversations (id, user_id, title, title_origin, archived, pinned, sort_order,
           read_revision, target_kind, target_device_id, target_path, target_workspace_id, context_version,
           model, created_at, updated_at, workflow_kind, workflow_id)
         VALUES (?1, ?2, ?3, ?4, 0, ?11,
           (SELECT COALESCE(MIN(sort_order), 0) - 1 FROM conversations WHERE user_id = ?2),
           0, ?5, ?6, ?7, ?8, ?14, ?9, ?10, ?10, ?12, ?13)
         ON CONFLICT (id) DO NOTHING",
        params![
            new.id.as_str(),
            new.owner.as_str(),
            new.title.unwrap_or_default(),
            if new.title.is_some() { "user" } else { "placeholder" },
            target.kind,
            target.device,
            target.path,
            target.workspace,
            new.model.map(to_json),
            new.at.as_millisecond(),
            new.pinned,
            workflow_kind,
            workflow_id,
            revision_column(new.context_version)
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
    workflow_choice("conversations", "workflow_kind", &kind, row.get("workflow_id")?)
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
        "workspace" => {
            let workspace: Option<String> = row.get("target_workspace_id")?;
            Selection::Workspace {
                workspace_id: decode(
                    TABLE,
                    "target_workspace_id",
                    WorkspaceId::try_from(workspace.unwrap_or_default()),
                )?,
                device_id: device()?,
                path: path_of(path)?,
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

/// A device attached to a conversation (`sessions-and-targets.md`
/// § Attached hosts): a Host the conversation reaches besides its primary one.
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
    /// home (`sessions-and-targets.md` § Attached hosts).
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
    /// revision it made, and the workspace its destination's selection
    /// made, if it did.
    Switched {
        switch: TargetSwitch,
        revision: u64,
        made: Option<WorkspaceRecord>,
    },
    /// The destination selects what the conversation has: nothing changed.
    Unchanged,
    /// The selection is no longer the one the switch started from.
    Conflict,
    Unselectable(Unselectable),
}

impl ControlService {
    /// The target switch's write, against `expected`, the selection the
    /// switch started from, and `from`, what it resolved to: it selects
    /// `to`'s directory, which makes its workspace when the user has none
    /// (`sessions-and-targets.md` § Resolve a target), all in one
    /// transaction, so of two switches from one selection exactly one
    /// wins. The winner records the switch for every node's next context
    /// block and advances the execution-context revision, which it
    /// answers; when it makes the pending move `pending`, it clears it.
    pub async fn switch_conversation_target(
        &self,
        id: ConversationId,
        expected: Selection,
        from: ExecutionTarget,
        to: ConversationTarget,
        ends: SwitchEnds,
        pending: Option<String>,
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
            let Selected { selection, made } = match select(&transaction, &owner, &id, &to, now)? {
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
                   target_workspace_id = ?5, last_switch = ?6, last_switch_revision = context_version + 1,
                   context_version = context_version + 1, updated_at = ?7
                 WHERE id = ?1 AND target_kind = ?8 AND target_device_id IS ?9 AND target_path IS ?10
                   AND target_workspace_id IS ?11
                 RETURNING context_version",
                params![
                    id.as_str(),
                    target.kind,
                    target.device,
                    target.path,
                    target.workspace,
                    to_json(&switch),
                    now.as_millisecond(),
                    before.kind,
                    before.device,
                    before.path,
                    before.workspace
                ],
                |row| row.get(0),
            ).optional()?;
            let Some(revision) = revision else {
                return Ok(SwitchWrite::Conflict);
            };
            let revision = decode("conversations", "context_version", u64::try_from(revision))?;
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
            if let Some(pending) = &pending {
                clear_pending_move(&transaction, &id, pending)?;
            }
            transaction.commit()?;
            Ok(SwitchWrite::Switched { switch, revision, made })
        })
        .await
    }
}

/// Archives the conversation `id`. An archive withdraws the permission
/// requests, decided ones whose message was not delivered among them, and
/// keeps the grants (`permissions.md` § Requests); it drops the move and
/// the detaches an agent asked for as well (`sessions-and-targets.md`
/// § Switch the primary target).
pub(crate) fn archive_in(connection: &Connection, id: &ConversationId) -> Result<(), StorageError> {
    connection.execute(
        "UPDATE conversation_hosts SET detaching = 0 WHERE conversation_id = ?1",
        [id.as_str()],
    )?;
    connection.execute("DELETE FROM permission_requests WHERE conversation_id = ?1", [id.as_str()])?;
    connection.execute(
        "UPDATE conversations SET archived = 1, pending_id = NULL, pending_kind = NULL,
           pending_device_id = NULL, pending_path = NULL, pending_workspace_id = NULL
         WHERE id = ?1",
        [id.as_str()],
    )?;
    Ok(())
}

/// Restores the conversation `id`. One whose project was deleted while it
/// was archived moves back to its directory, into that directory's
/// workspace, made again (`sessions-and-targets.md` § Resolve a target);
/// one whose device was revoked meanwhile keeps its directory there.
fn restore_in(connection: &Connection, id: &ConversationId, now: Timestamp) -> Result<(), StorageError> {
    connection.execute("UPDATE conversations SET archived = 0 WHERE id = ?1", [id.as_str()])?;
    let Some(record) = conversation_by_id(connection, id)? else {
        return Ok(());
    };
    let Selection::Workspace { workspace_id, device_id, path } = &record.target else {
        return Ok(());
    };
    if super::workspaces::workspace_in(connection, workspace_id)?.is_some() {
        return Ok(());
    }
    let Some(device) = super::devices::device_in(connection, device_id)? else {
        return Ok(());
    };
    let selected = super::selection::project_of(connection, &device, path, now)?;
    super::devices::set_selection(connection, id, &selected.selection)
}

/// A move an agent asked for, which waits for its conversation's tree to be
/// idle (`sessions-and-targets.md` § Switch the primary target).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingMove {
    pub id: String,
    pub target: ConversationTarget,
}

/// What waits for a conversation's tree to be idle: the move an agent asked
/// for, and the attached devices an agent asked to detach.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PendingChanges {
    pub moving: Option<PendingMove>,
    pub detaching: Vec<DeviceId>,
}

impl PendingChanges {
    pub fn is_empty(&self) -> bool {
        self.moving.is_none() && self.detaching.is_empty()
    }
}

impl ControlService {
    /// Records the move to `target` an agent asked for, in place of the one
    /// that waited, and answers its id; an archived conversation takes none.
    pub async fn set_pending_move(
        &self,
        id: ConversationId,
        target: ConversationTarget,
    ) -> Result<Result<String, ChangeOutcome>, StorageError> {
        self.call(move |connection, _| {
            let pending = uuid::Uuid::new_v4().to_string();
            let columns = DestinationColumns::of(&target);
            let set = connection.execute(
                "UPDATE conversations SET pending_id = ?2, pending_kind = ?3, pending_device_id = ?4,
                   pending_path = ?5, pending_workspace_id = ?6
                 WHERE id = ?1 AND archived = 0",
                params![
                    id.as_str(),
                    pending,
                    columns.kind,
                    columns.device,
                    columns.path,
                    columns.workspace
                ],
            )?;
            if set == 1 {
                return Ok(Ok(pending));
            }
            let archived: Option<bool> = connection
                .query_row("SELECT archived FROM conversations WHERE id = ?1", [id.as_str()], |row| row.get(0))
                .optional()?;
            Ok(Err(match archived {
                Some(_) => ChangeOutcome::Archived,
                None => ChangeOutcome::Missing,
            }))
        })
        .await
    }

    /// Marks the attached `device` to detach once the conversation's tree is
    /// idle, as an agent's `demi host detach` asks; answers whether it is
    /// attached, false marking nothing.
    pub async fn mark_detach(
        &self,
        id: ConversationId,
        device: DeviceId,
    ) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let marked = connection.execute(
                "UPDATE conversation_hosts SET detaching = 1
                 WHERE conversation_id = ?1 AND device_id = ?2",
                params![id.as_str(), device.as_str()],
            )?;
            Ok(marked == 1)
        })
        .await
    }

    /// What waits for the conversation's tree to be idle.
    pub async fn pending_changes(&self, id: ConversationId) -> Result<PendingChanges, StorageError> {
        self.call(move |connection, _| {
            const TABLE: &str = "conversations";
            let moving = connection
                .query_row(
                    "SELECT pending_id, pending_kind, pending_device_id, pending_path, pending_workspace_id
                     FROM conversations WHERE id = ?1 AND pending_id IS NOT NULL",
                    [id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, Option<String>>(2)?,
                            row.get::<_, Option<String>>(3)?,
                            row.get::<_, Option<String>>(4)?,
                        ))
                    },
                )
                .optional()?;
            let moving = match moving {
                None => None,
                Some((pending, kind, device, path, workspace)) => {
                    let target = match kind.as_str() {
                        "cloud" => ConversationTarget::Cloud { path },
                        "device" => ConversationTarget::Device {
                            device_id: decode(
                                TABLE,
                                "pending_device_id",
                                DeviceId::try_from(device.unwrap_or_default()),
                            )?,
                            path: path.unwrap_or_default(),
                        },
                        "workspace" => ConversationTarget::Workspace {
                            workspace_id: decode(
                                TABLE,
                                "pending_workspace_id",
                                WorkspaceId::try_from(workspace.unwrap_or_default()),
                            )?,
                        },
                        other => {
                            return Err(StorageError::Corrupt {
                                table: TABLE,
                                column: "pending_kind",
                                reason: format!("unknown target kind {other}"),
                            });
                        }
                    };
                    decode(TABLE, "pending_path", target.validate())?;
                    Some(PendingMove {
                        id: pending,
                        target,
                    })
                }
            };
            let detaching = connection
                .prepare_cached(
                    "SELECT device_id FROM conversation_hosts
                     WHERE conversation_id = ?1 AND detaching = 1 ORDER BY attached_at, rowid",
                )?
                .query_map([id.as_str()], |row| row.get::<_, String>(0))?
                .map(|device| {
                    decode(
                        "conversation_hosts",
                        "device_id",
                        DeviceId::try_from(device?),
                    )
                })
                .collect::<Result<_, _>>()?;
            Ok(PendingChanges { moving, detaching })
        })
        .await
    }

    /// Drops the pending move `pending`, as one that failed when it was
    /// made; a newer one that replaced it stays.
    pub async fn drop_pending_move(
        &self,
        id: ConversationId,
        pending: String,
    ) -> Result<(), StorageError> {
        self.call(move |connection, _| clear_pending_move(connection, &id, &pending))
            .await
    }

    /// The conversations of every user that hold a pending change, with
    /// their owners, which a start makes once their trees are idle.
    pub async fn conversations_pending(&self) -> Result<Vec<(UserId, ConversationId)>, StorageError> {
        self.call(move |connection, _| {
            connection
                .prepare_cached(
                    "SELECT user_id, id FROM conversations WHERE archived = 0 AND (pending_id IS NOT NULL
                       OR EXISTS (SELECT 1 FROM conversation_hosts
                                  WHERE conversation_id = conversations.id AND detaching = 1))",
                )?
                .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
                .map(|row| {
                    let (owner, id) = row?;
                    Ok((
                        decode("conversations", "user_id", UserId::try_from(owner))?,
                        decode("conversations", "id", ConversationId::try_from(id.as_str()))?,
                    ))
                })
                .collect()
        })
        .await
    }
}

/// Clears the conversation's pending move when it is still `pending`.
fn clear_pending_move(
    connection: &Connection,
    id: &ConversationId,
    pending: &str,
) -> Result<(), StorageError> {
    connection.execute(
        "UPDATE conversations SET pending_id = NULL, pending_kind = NULL, pending_device_id = NULL,
           pending_path = NULL, pending_workspace_id = NULL
         WHERE id = ?1 AND pending_id = ?2",
        params![id.as_str(), pending],
    )?;
    Ok(())
}

/// The conversation's attached hosts, first attached first, each by its
/// device's current name: a host whose name the primary Host or one
/// attached before it goes by already takes the first free `name-2`,
/// `name-3` and so on, so no name stands for two hosts and a name stays the
/// same device while the target, the hosts and their devices' names do.
/// Hosts attached at one time, as a Fork's are, keep the order they were
/// attached in, which their row ids hold. The primary Host goes by the name
/// the header shows: its device's, Cloud for the Cloud.
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

/// Puts `device` on the list of hosts the conversation may reach
/// (`sessions-and-targets.md` § Hosts a conversation may reach), where it
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

/// Whether the conversation may reach `device` (`sessions-and-targets.md`
/// § Hosts a conversation may reach): it is the conversation's primary
/// Host, the device of its project or the user's Cloud for a conversation
/// there, or on its list.
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
                 OR (c.target_kind = 'cloud' AND EXISTS (
                   SELECT 1 FROM devices d WHERE d.id = ?2 AND d.kind = 'managed' AND d.machine = 'cloud' AND d.user_id = c.user_id))))",
        params![conversation.as_str(), device.as_str()],
        |row| row.get(0),
    )?)
}

/// Raises the revision of the conversation's attached hosts, which its
/// summary carries, so a page that shows them reads them again
/// (`web-api.md` § Sidebar mutations and read state).
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
fn advance_context(
    connection: &Connection,
    conversation: &ConversationId,
) -> Result<(), StorageError> {
    connection.execute(
        "UPDATE conversations SET context_version = context_version + 1 WHERE id = ?1",
        [conversation.as_str()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::control::testing;
    use demi_runner_protocol::wire::RunnerPlatform;

    fn conversation(number: u8) -> ConversationId {
        ConversationId::try_from(format!("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a{number:02x}")).unwrap()
    }

    fn created(creation: Creation) -> ConversationRecord {
        match creation {
            Creation::Created(record) => record,
            other => panic!("expected a new conversation, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn the_index_finds_a_conversation_by_any_spelling_and_lists_the_newest_first() {
        let data = tempfile::tempdir().unwrap();
        let control = ControlService::open(
            &data.path().join("control.sqlite"),
            Arc::new(demi_shared_types::SystemClock),
            std::sync::Arc::new(crate::providers::NoSealedConfigs),
        )
        .await
        .unwrap();
        let master = testing::master(&control).await.id;
        let first = created(
            control
                .create_conversation(master.clone(), conversation(1), ConversationStart::default())
                .await
                .unwrap(),
        );
        assert_eq!(
            (
                first.title.as_deref(),
                &first.target,
                first.model.clone(),
                first.read_revision
            ),
            (
                None,
                &Selection::Cloud { path: None },
                None,
                0
            )
        );
        let second = created(
            control
                .create_conversation(master.clone(), conversation(2), ConversationStart::default())
                .await
                .unwrap(),
        );

        // Another spelling names the same conversation, which keeps the
        // spelling it was created with.
        let upper = ConversationId::try_from(conversation(1).as_str().to_uppercase()).unwrap();
        assert_eq!(
            control.conversation(upper.clone()).await.unwrap(),
            Some(first.clone())
        );
        assert_eq!(
            control
                .create_conversation(master.clone(), upper, ConversationStart::default())
                .await
                .unwrap(),
            Creation::Existing(first.clone())
        );

        let listed: Vec<ConversationId> = control
            .conversations(master.clone(), false)
            .await
            .unwrap()
            .into_iter()
            .map(|record| record.id)
            .collect();
        assert_eq!(listed, [second.id.clone(), first.id.clone()]);
        assert!(
            control
                .conversations(master, true)
                .await
                .unwrap()
                .is_empty()
        );

        control
            .mark_conversation_read(first.id.clone(), 5)
            .await
            .unwrap();
        control
            .mark_conversation_read(first.id.clone(), 3)
            .await
            .unwrap();
        let model = demi_agent_store::testing::model_of("entry-1", "claude-opus-4-8");
        let changed = control
            .change_conversation(first.id.clone(), RecordChange::Model(model.clone()))
            .await
            .unwrap();
        assert_eq!(changed, ChangeOutcome::Applied);
        control.touch_conversation(first.id.clone()).await.unwrap();
        let read = control
            .conversation(first.id.clone())
            .await
            .unwrap()
            .unwrap();
        assert_eq!((read.read_revision, read.model), (5, Some(model)));
        assert!(read.updated_at >= first.updated_at);

        // A row outside its type is refused, never repaired.
        testing::execute(
            &control,
            r#"UPDATE conversations SET model = '{"providerId":""}' WHERE id = ?1"#,
            vec![first.id.as_str().to_owned()],
        )
        .await;
        let refused = control.conversation(first.id.clone()).await.unwrap_err();
        assert!(
            matches!(
                refused,
                StorageError::Corrupt {
                    table: "conversations",
                    column: "model",
                    ..
                }
            ),
            "{refused}"
        );
        control.close().await.unwrap();
    }

    /// An attached host goes by its device's name, a later one by that name
    /// with a suffix when an earlier one goes by it, and by the device's new
    /// name once the user renames the device, which the conversation's page
    /// and its agents' next context block learn of.
    #[tokio::test]
    async fn an_attached_host_goes_by_its_devices_name_with_a_suffix_and_follows_a_rename() {
        use crate::devices::DeviceChange;

        let data = tempfile::tempdir().unwrap();
        let control = ControlService::open(
            &data.path().join("control.sqlite"),
            Arc::new(demi_shared_types::SystemClock),
            std::sync::Arc::new(crate::providers::NoSealedConfigs),
        )
        .await
        .unwrap();
        let master = testing::master(&control).await.id;
        let id = created(
            control
                .create_conversation(master.clone(), conversation(1), ConversationStart::default())
                .await
                .unwrap(),
        )
        .id;
        let mut devices = Vec::new();
        for (name, token) in [("laptop", "one"), ("laptop", "two"), ("ci", "three")] {
            let hash = crate::accounts::TokenHash::of(token);
            let device = control
                .create_device(master.clone(), name.into(), RunnerPlatform::Linux, hash)
                .await
                .unwrap();
            devices.push(device.id);
        }
        let record = |device: &DeviceId, cwd: Option<&str>| AttachedHostRecord {
            device: device.clone(),
            cwd: cwd.map(str::to_owned),
        };
        let attaching = vec![
            record(&devices[0], None),
            record(&devices[1], Some("/work")),
            record(&devices[2], None),
            // Attached already: its row stays as it is.
            record(&devices[0], Some("/elsewhere")),
        ];
        control
            .call({
                let id = id.clone();
                move |connection, now| {
                    let mut attached = Vec::new();
                    for host in &attaching {
                        attached.push(insert_attached_host(connection, &id, host, now)?);
                    }
                    assert_eq!(attached, [true, true, true, false]);
                    Ok(())
                }
            })
            .await
            .unwrap();
        let listed = async || -> Vec<(String, Option<String>)> {
            control
                .attached_hosts(id.clone())
                .await
                .unwrap()
                .into_iter()
                .map(|host| (host.name, host.cwd))
                .collect()
        };
        let named = |names: [&str; 3]| -> Vec<(String, Option<String>)> {
            names
                .iter()
                .zip([None, Some("/work".to_owned()), None])
                .map(|(name, cwd)| ((*name).to_owned(), cwd))
                .collect()
        };
        // Attached in one transaction, at one time: listed in the order they
        // were attached, the second laptop with a suffix.
        assert_eq!(listed().await, named(["laptop", "laptop-2", "ci"]));

        let before = control.conversation(id.clone()).await.unwrap().unwrap();
        let rename = |device: &DeviceId, name: &str| {
            control.change_device(
                device.clone(),
                DeviceChange {
                    name: Some(name.into()),
                    route: None,
                },
            )
        };
        let renamed = rename(&devices[1], "studio").await.unwrap().unwrap();
        assert_eq!(renamed.conversations, [id.clone()]);
        assert_eq!(listed().await, named(["laptop", "studio", "ci"]));
        let after = control.conversation(id.clone()).await.unwrap().unwrap();
        assert_eq!(
            (after.hosts_revision, after.context_version),
            (before.hosts_revision + 1, before.context_version + 1)
        );
        // The first attached keeps the name both devices now have.
        rename(&devices[0], "studio").await.unwrap().unwrap();
        assert_eq!(listed().await, named(["studio", "studio-2", "ci"]));
        // A change that keeps the name tells no conversation.
        let unchanged = rename(&devices[2], "ci").await.unwrap().unwrap();
        assert!(unchanged.conversations.is_empty());
        control.close().await.unwrap();
    }

    /// An attached device named as the primary Host goes by its name with a
    /// suffix, the Cloud's name counting for a Cloud conversation, so
    /// `--host <name>` never names two hosts; a switch that changes the
    /// primary's name renames the attached hosts, which the page learns of.
    /// Planted defect this catches: a primary left out of the names taken,
    /// which lists the attached `laptop` under the primary's name.
    #[tokio::test]
    async fn an_attached_host_named_as_the_primary_takes_a_suffix() {
        let data = tempfile::tempdir().unwrap();
        let control = ControlService::open(
            &data.path().join("control.sqlite"),
            Arc::new(demi_shared_types::SystemClock),
            std::sync::Arc::new(crate::providers::NoSealedConfigs),
        )
        .await
        .unwrap();
        let master = testing::master(&control).await.id;
        let mut devices = Vec::new();
        for (name, token) in [("laptop", "one"), ("laptop", "two"), ("Cloud", "three")] {
            let hash = crate::accounts::TokenHash::of(token);
            let device = control
                .create_device(master.clone(), name.into(), RunnerPlatform::Linux, hash)
                .await
                .unwrap();
            testing::report_home(&control, &device.id, "/home/ana").await;
            devices.push(device.id);
        }
        let on_laptop = ConversationTarget::Device {
            device_id: devices[0].clone(),
            path: "/work".into(),
        };
        let start = |target: ConversationTarget| ConversationStart {
            target,
            ..ConversationStart::default()
        };
        let id = created(
            control
                .create_conversation(master.clone(), conversation(1), start(on_laptop.clone()))
                .await
                .unwrap(),
        )
        .id;
        let in_cloud = created(
            control
                .create_conversation(master.clone(), conversation(2), start(ConversationTarget::Cloud { path: None }))
                .await
                .unwrap(),
        )
        .id;
        for (conversation, device) in [(&id, &devices[1]), (&in_cloud, &devices[2])] {
            let attach = RecordChange::Attach(AttachedHostRecord {
                device: device.clone(),
                cwd: None,
            });
            control.change_conversation(conversation.clone(), attach).await.unwrap();
        }
        let names = async |conversation: &ConversationId| -> Vec<String> {
            control
                .attached_hosts(conversation.clone())
                .await
                .unwrap()
                .into_iter()
                .map(|host| host.name)
                .collect()
        };
        assert_eq!(names(&id).await, ["laptop-2"]);
        assert_eq!(names(&in_cloud).await, ["Cloud-2"]);

        let before = control.conversation(id.clone()).await.unwrap().unwrap();
        let from = ExecutionTarget::Device {
            device_id: devices[0].clone(),
            path: "/work".into(),
        };
        let to = ConversationTarget::Cloud { path: Some("/home/demi".into()) };
        let ends = SwitchEnds { departed: None, arriving: None };
        let switched = control
            .switch_conversation_target(id.clone(), before.target.clone(), from, to, ends, None)
            .await
            .unwrap();
        assert!(matches!(switched, SwitchWrite::Switched { .. }), "the switch wins");
        assert_eq!(names(&id).await, ["laptop"]);
        let after = control.conversation(id.clone()).await.unwrap().unwrap();
        assert_eq!(after.hosts_revision, before.hosts_revision + 1);
        control.close().await.unwrap();
    }

    #[tokio::test]
    async fn of_two_switches_from_one_target_one_wins_and_its_announcement_stays() {
        let data = tempfile::tempdir().unwrap();
        let control = ControlService::open(
            &data.path().join("control.sqlite"),
            Arc::new(demi_shared_types::SystemClock),
            std::sync::Arc::new(crate::providers::NoSealedConfigs),
        )
        .await
        .unwrap();
        let master = testing::master(&control).await.id;
        let id = created(
            control
                .create_conversation(master.clone(), conversation(1), ConversationStart::default())
                .await
                .unwrap(),
        )
        .id;
        let hash = crate::accounts::TokenHash::of("laptop");
        let laptop = control
            .create_device(master.clone(), "laptop".into(), RunnerPlatform::Linux, hash)
            .await
            .unwrap()
            .id;
        testing::report_home(&control, &laptop, "/home/ana").await;
        let cloud = Selection::Cloud { path: None };
        let from = ExecutionTarget::Cloud {
            device_id: None,
            path: format!("/home/demi/sessions/{id}"),
        };
        let switch_to = |path: &str| {
            let to = ConversationTarget::Device {
                device_id: laptop.clone(),
                path: path.into(),
            };
            let ends = SwitchEnds {
                departed: None,
                arriving: Some(laptop.clone()),
            };
            control.switch_conversation_target(id.clone(), cloud.clone(), from.clone(), to, ends, None)
        };
        let (first, second) = tokio::join!(switch_to("/first"), switch_to("/second"));
        let revisions = [first.unwrap(), second.unwrap()].map(|write| match write {
            SwitchWrite::Switched { revision, .. } => Some(revision),
            _ => None,
        });
        assert_eq!(revisions, [Some(1), None]);
        let record = control.conversation(id.clone()).await.unwrap().unwrap();
        let Selection::Workspace { path, .. } = &record.target else {
            panic!("the switch to /first selects its project, not {:?}", record.target);
        };
        assert_eq!((path.as_str(), record.context_version), ("/first", 1));
        // The winner's announcement is the one every node reads.
        let announced: String = control
            .call(move |connection, _| {
                Ok(connection.query_row(
                    "SELECT last_switch FROM conversations WHERE id = ?1",
                    [id.as_str()],
                    |row| row.get(0),
                )?)
            })
            .await
            .unwrap();
        let announced: TargetSwitch = serde_json::from_str(&announced).unwrap();
        assert_eq!((announced.to.device(), announced.to.path()), (Some(&laptop), "/first"));
        control.close().await.unwrap();
    }

    /// An archive drops what an agent asked for to happen once its
    /// conversation's work ends: the pending move and the pending detaches
    /// (`sessions-and-targets.md` § Switch the primary target). A restore
    /// brings neither back.
    #[tokio::test]
    async fn an_archive_drops_the_pending_move_and_the_pending_detaches() {
        let data = tempfile::tempdir().unwrap();
        let control = ControlService::open(
            &data.path().join("control.sqlite"),
            Arc::new(demi_shared_types::SystemClock),
            std::sync::Arc::new(crate::providers::NoSealedConfigs),
        )
        .await
        .unwrap();
        let master = testing::master(&control).await.id;
        let id = created(
            control
                .create_conversation(master.clone(), conversation(1), ConversationStart::default())
                .await
                .unwrap(),
        )
        .id;
        let laptop = control
            .create_device(
                master.clone(),
                "laptop".into(),
                RunnerPlatform::Linux,
                crate::accounts::TokenHash::of("laptop"),
            )
            .await
            .unwrap()
            .id;
        let attach = RecordChange::Attach(AttachedHostRecord {
            device: laptop.clone(),
            cwd: None,
        });
        control.change_conversation(id.clone(), attach).await.unwrap();
        let to = ConversationTarget::Device {
            device_id: laptop.clone(),
            path: "/work".into(),
        };
        control.set_pending_move(id.clone(), to).await.unwrap().unwrap();
        assert!(control.mark_detach(id.clone(), laptop.clone()).await.unwrap());
        let pending = control.pending_changes(id.clone()).await.unwrap();
        assert!(pending.moving.is_some() && pending.detaching == [laptop.clone()], "{pending:?}");

        for archived in [true, false] {
            control
                .change_conversation(id.clone(), RecordChange::Archived(archived))
                .await
                .unwrap();
        }
        assert_eq!(control.pending_changes(id.clone()).await.unwrap(), PendingChanges::default());
        // The device stays attached: only the detach waiting for it is gone.
        let attached = control.attached_hosts(id.clone()).await.unwrap();
        assert_eq!(attached.len(), 1);
        assert!(control.conversations_pending().await.unwrap().is_empty());
    }

}
