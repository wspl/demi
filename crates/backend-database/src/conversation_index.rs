//! The conversation index of the control store (`storage.md` § Control
//! records): each conversation's owner, title, archive and pin state, place
//! in the sidebar, read revision, target, execution-context revision and
//! model selection, which is the conversation's model settings (`models.md`
//! § A conversation's model settings), and when the earliest wakeup its tree
//! saved is due (`runtime.md` § Yield wakeups). The agent tree itself is in the conversation's own
//! database. A conversation's id is the one the web app chose, kept in the
//! case it arrived in and compared without case, so an id another
//! conversation holds in any spelling is taken.

use demi_shared_types::{ModelSelection, Timestamp};
use demi_web_api_protocol::conversations::{ConversationTarget, ModelChoice};
use demi_web_api_protocol::ids::{ConversationId, DeviceId, UserId, WorkspaceId};
use garde::Validate;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use super::StorageError;
use super::columns::{decode, instant, json, to_json};
use super::control::ControlService;
use super::tree::WakeupDue;

/// A conversation as the index holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationRecord {
    pub id: ConversationId,
    pub owner: UserId,
    pub title: String,
    pub archived: bool,
    pub pinned: bool,
    /// The output revision the user last acknowledged; it only moves
    /// forward.
    pub read_revision: u64,
    pub target: ConversationTarget,
    /// Advanced by every change of the conversation's execution context,
    /// such as a target switch or a Host attached.
    pub context_version: u64,
    /// The conversation's model selection; none until its first model is
    /// chosen.
    pub model: Option<ModelSelection>,
    /// How many messages the user has sent, and how many of them the title
    /// has read (`product.md` § Conversation titles).
    pub user_messages: u64,
    pub titled_messages: u64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    /// The revision of the conversation's draft, 0 before its first save.
    pub draft_revision: u64,
    /// The revision of the conversation's work panel, 0 before its first
    /// change.
    pub panel_revision: u64,
    /// The revision of the conversation's attached hosts, raised by each
    /// change of them and of the directory a host's shell recorded; 0
    /// before the first.
    pub hosts_revision: u64,
    /// How many of the conversation's permission requests are undecided.
    pub permission_requests: u64,
}

/// A conversation whose tree saved a yield wakeup, with its owner and when
/// its earliest wakeup is due.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedWakeup {
    pub conversation: ConversationId,
    pub owner: UserId,
    pub due: WakeupDue,
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
    /// The user's title; the placeholder when none.
    pub title: Option<String>,
    pub pinned: bool,
    pub target: ConversationTarget,
    pub model: Option<ModelSelection>,
}

impl Default for ConversationStart {
    fn default() -> Self {
        Self {
            title: None,
            pinned: false,
            target: ConversationTarget::Cloud { path: None },
            model: None,
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
}

/// The title a conversation has until its first send or a rename
/// (`product.md` § Conversation titles).
const PLACEHOLDER_TITLE: &str = "New conversation";

/// The sidebar's order within the active or the archived conversations:
/// pinned first, then by the user's order.
const SIDEBAR_ORDER: &str = "pinned DESC, sort_order, id";

const CONVERSATION_COLUMNS: &str = "id, user_id, title, archived, pinned, read_revision, target_kind, target_device_id,
     target_path, target_workspace_id, context_version, model, user_messages, titled_messages,
     created_at, updated_at, hosts_revision,
     COALESCE((SELECT revision FROM conversation_drafts WHERE conversation_id = conversations.id), 0) AS draft_revision,
     COALESCE((SELECT revision FROM conversation_panels WHERE conversation_id = conversations.id), 0) AS panel_revision,
     (SELECT count(*) FROM permission_requests WHERE conversation_id = conversations.id AND decision IS NULL) AS permission_requests";

/// A target as its typed columns: the kind and what the kind names.
pub struct TargetColumns {
    pub kind: &'static str,
    pub device: Option<String>,
    pub path: Option<String>,
    pub workspace: Option<String>,
}

impl TargetColumns {
    pub fn of(target: &ConversationTarget) -> Self {
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
                return Ok(Creation::Unavailable);
            }
            let (title, origin) = match start.title.as_deref() {
                Some(title) if title != PLACEHOLDER_TITLE => (title, TitleOrigin::User),
                _ => (PLACEHOLDER_TITLE, TitleOrigin::Placeholder),
            };
            let inserted = insert_conversation(
                &transaction,
                &NewConversation {
                    id: &id,
                    owner: &owner,
                    title,
                    origin,
                    pinned: start.pinned,
                    target: &start.target,
                    model: start.model.as_ref(),
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
                Creation::Unavailable
            } else if inserted == 1 {
                Creation::Created(record)
            } else {
                Creation::Existing(record)
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

    /// The conversation's latest target switch, which every node's next
    /// context block describes; none before its first.
    pub async fn last_switch(
        &self,
        id: ConversationId,
    ) -> Result<Option<TargetSwitch>, StorageError> {
        self.call(move |connection, _| {
            let text: Option<Option<String>> = connection
                .query_row(
                    "SELECT last_switch FROM conversations WHERE id = ?1",
                    [id.as_str()],
                    |row| row.get(0),
                )
                .optional()?;
            text.flatten()
                .map(|text| json("conversations", "last_switch", &text))
                .transpose()
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

    /// Acknowledges the output up to `revision`; an acknowledgement never
    /// moves the read revision back.
    pub async fn mark_conversation_read(
        &self,
        id: ConversationId,
        revision: u64,
    ) -> Result<(), StorageError> {
        let revision = revision_column(revision);
        self.call(move |connection, _| {
            connection.execute(
                "UPDATE conversations SET read_revision = MAX(read_revision, ?2) WHERE id = ?1",
                params![id.as_str(), revision],
            )?;
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
                    if *archived {
                        transaction.execute(
                            "DELETE FROM permission_requests WHERE conversation_id = ?1",
                            [id.as_str()],
                        )?;
                    }
                    transaction.execute(
                        "UPDATE conversations SET archived = ?2 WHERE id = ?1",
                        params![id.as_str(), archived],
                    )?
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

    /// Counts one more message the user sent, which makes a generated title
    /// older than the conversation; answers how many there are now.
    pub async fn count_user_message(&self, id: ConversationId) -> Result<u64, StorageError> {
        self.call(move |connection, _| {
            let count: i64 = connection.query_row(
                "UPDATE conversations SET user_messages = user_messages + 1 WHERE id = ?1 RETURNING user_messages",
                [id.as_str()],
                |row| row.get(0),
            )?;
            decode("conversations", "user_messages", u64::try_from(count))
        })
        .await
    }

    /// Makes `title`, from the first message, the conversation's while its
    /// title is still the placeholder; answers whether it did, which makes
    /// this send the one a generated title may follow.
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
    /// that landed meanwhile stays. Either way the title is current for the
    /// `seen` messages the request read. Answers whether it was written.
    pub async fn generated_title(
        &self,
        id: ConversationId,
        title: String,
        from: String,
        seen: u64,
    ) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let written = transaction.execute(
                "UPDATE conversations SET title = ?2, title_origin = 'generated' WHERE id = ?1 AND title = ?3",
                params![id.as_str(), title, from],
            )?;
            transaction.execute(
                "UPDATE conversations SET titled_messages = MAX(titled_messages, ?2) WHERE id = ?1",
                params![id.as_str(), i64::try_from(seen).expect("a count the column held fits it")],
            )?;
            transaction.commit()?;
            Ok(written == 1)
        })
        .await
    }

    /// Records when the earliest wakeup the conversation's tree saved is due,
    /// or that it saved none (`runtime.md` § Yield wakeups).
    pub async fn set_wakeup(
        &self,
        id: ConversationId,
        wakeup: Option<WakeupDue>,
    ) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "UPDATE conversations SET wakeup_at = ?2 WHERE id = ?1",
                params![id.as_str(), wakeup.map(WakeupDue::column)],
            )?;
            Ok(())
        })
        .await
    }

    /// The conversations that are not archived and whose tree saved a
    /// wakeup, each with its owner and when its earliest wakeup is due,
    /// earliest first.
    pub async fn saved_wakeups(&self) -> Result<Vec<SavedWakeup>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare(
                "SELECT id, user_id, wakeup_at FROM conversations
                 WHERE wakeup_at IS NOT NULL AND archived = 0 ORDER BY wakeup_at",
            )?;
            let mut rows = statement.query([])?;
            let mut wakeups = Vec::new();
            while let Some(row) = rows.next()? {
                wakeups.push(SavedWakeup {
                    conversation: decode(
                        "conversations",
                        "id",
                        ConversationId::try_from(row.get::<_, String>(0)?),
                    )?,
                    owner: decode(
                        "conversations",
                        "user_id",
                        UserId::try_from(row.get::<_, String>(1)?),
                    )?,
                    due: WakeupDue::from_column("conversations", row.get(2)?)?,
                });
            }
            Ok(wakeups)
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

/// Where a conversation's title came from (`product.md` § Conversation
/// titles), as its column names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleOrigin {
    Placeholder,
    User,
}

impl TitleOrigin {
    fn column(self) -> &'static str {
        match self {
            Self::Placeholder => "placeholder",
            Self::User => "user",
        }
    }
}

/// A conversation as it enters the index, seen live as it is created.
pub struct NewConversation<'a> {
    pub id: &'a ConversationId,
    pub owner: &'a UserId,
    pub title: &'a str,
    pub origin: TitleOrigin,
    pub pinned: bool,
    pub target: &'a ConversationTarget,
    pub model: Option<&'a ModelSelection>,
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
    let inserted = connection.execute(
        "INSERT INTO conversations (id, user_id, title, title_origin, archived, pinned, sort_order,
           read_revision, target_kind, target_device_id, target_path, target_workspace_id, context_version,
           model, user_messages, titled_messages, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, 0, ?11,
           (SELECT COALESCE(MIN(sort_order), 0) - 1 FROM conversations WHERE user_id = ?2),
           0, ?5, ?6, ?7, ?8, 0, ?9, 0, 0, ?10, ?10)
         ON CONFLICT (id) DO NOTHING",
        params![
            new.id.as_str(),
            new.owner.as_str(),
            new.title,
            new.origin.column(),
            target.kind,
            target.device,
            target.path,
            target.workspace,
            new.model.map(to_json),
            new.at.as_millisecond(),
            new.pinned
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

/// A `conversations` row, read from its columns in `CONVERSATION_COLUMNS`.
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
        title: row.get("title")?,
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
        user_messages: decode(
            TABLE,
            "user_messages",
            u64::try_from(row.get::<_, i64>("user_messages")?),
        )?,
        titled_messages: decode(
            TABLE,
            "titled_messages",
            u64::try_from(row.get::<_, i64>("titled_messages")?),
        )?,
        created_at: instant(row, TABLE, "created_at")?,
        updated_at: instant(row, TABLE, "updated_at")?,
        draft_revision: decode(
            "conversation_drafts",
            "revision",
            u64::try_from(row.get::<_, i64>("draft_revision")?),
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
    })
}

/// The target a row's typed columns hold.
fn target_row(row: &Row<'_>) -> Result<ConversationTarget, StorageError> {
    const TABLE: &str = "conversations";
    let kind: String = row.get("target_kind")?;
    let path: Option<String> = row.get("target_path")?;
    let target = match kind.as_str() {
        "cloud" => ConversationTarget::Cloud { path },
        "device" => {
            let device: Option<String> = row.get("target_device_id")?;
            ConversationTarget::Device {
                device_id: decode(
                    TABLE,
                    "target_device_id",
                    DeviceId::try_from(device.unwrap_or_default()),
                )?,
                path: path.unwrap_or_default(),
            }
        }
        "workspace" => {
            let workspace: Option<String> = row.get("target_workspace_id")?;
            ConversationTarget::Workspace {
                workspace_id: decode(
                    TABLE,
                    "target_workspace_id",
                    WorkspaceId::try_from(workspace.unwrap_or_default()),
                )?,
            }
        }
        other => {
            return Err(StorageError::Corrupt {
                table: TABLE,
                column: "target_kind",
                reason: format!("unknown target kind {other}"),
            });
        }
    };
    decode(TABLE, "target_path", target.validate())?;
    Ok(target)
}

/// A device attached to a conversation (`sessions-and-targets.md`
/// § Attached hosts): a Host the conversation reaches besides its primary one.
/// A Fork keeps its source's in its operation's metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttachedHostRecord {
    #[garde(skip)]
    pub device: DeviceId,
    /// What the model and the user call the host; unique within the
    /// conversation.
    #[garde(length(min = 1))]
    pub name: String,
    /// Where the last `demi host shell --host` there ended; none until one
    /// ran.
    #[serde(deserialize_with = "Option::deserialize")]
    #[garde(skip)]
    pub cwd: Option<String>,
}

impl ControlService {
    /// The conversation's attached hosts, first attached first.
    pub async fn attached_hosts(
        &self,
        id: ConversationId,
    ) -> Result<Vec<AttachedHostRecord>, StorageError> {
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
    ) -> Result<Vec<(AttachedHostRecord, Timestamp)>, StorageError> {
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

impl ControlService {
    /// The target switch's write, against the target the switch started
    /// from: none, writing nothing, when the target is no longer
    /// `expected`, so of two switches from one target exactly one wins. The
    /// winner records `switch` for every node's next context block and
    /// advances the execution-context revision, which it answers.
    pub async fn switch_conversation_target(
        &self,
        id: ConversationId,
        expected: ConversationTarget,
        to: ConversationTarget,
        switch: TargetSwitch,
        ends: SwitchEnds,
    ) -> Result<Option<u64>, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let from = TargetColumns::of(&expected);
            let target = TargetColumns::of(&to);
            // `IS` compares NULL as equal to NULL, which `=` does not.
            let revision: Option<i64> = transaction.query_row(
                "UPDATE conversations SET target_kind = ?2, target_device_id = ?3, target_path = ?4,
                   target_workspace_id = ?5, last_switch = ?6, context_version = context_version + 1, updated_at = ?7
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
                    from.kind,
                    from.device,
                    from.path,
                    from.workspace
                ],
                |row| row.get(0),
            ).optional()?;
            let Some(revision) = revision else {
                return Ok(None);
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
                let name: Option<String> = transaction
                    .query_row("SELECT name FROM devices WHERE id = ?1", [departed.as_str()], |row| row.get(0))
                    .optional()?;
                let host = AttachedHostRecord {
                    name: name.unwrap_or_else(|| departed.to_string()),
                    device: departed.clone(),
                    cwd: Some(cwd.clone()),
                };
                insert_attached_host(&transaction, &id, &host, now)?;
            }
            transaction.commit()?;
            Ok(Some(revision))
        })
        .await
    }

    /// Records where the last `demi host shell --host` on the attached
    /// `device` ended, which is where the next one there starts; answers
    /// whether that changed the directory recorded, which raises the
    /// hosts' revision.
    pub async fn set_attached_cwd(
        &self,
        id: ConversationId,
        device: DeviceId,
        cwd: String,
    ) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            // `IS NOT` treats a directory not recorded yet as different.
            let changed = transaction.execute(
                "UPDATE conversation_hosts SET cwd = ?3
                 WHERE conversation_id = ?1 AND device_id = ?2 AND cwd IS NOT ?3",
                params![id.as_str(), device.as_str(), cwd],
            )? > 0;
            if changed {
                raise_hosts_revision(&transaction, &id)?;
            }
            transaction.commit()?;
            Ok(changed)
        })
        .await
    }
}

/// The `conversation_hosts` rows of the conversation, first attached first.
fn attached_rows(
    connection: &Connection,
    id: &ConversationId,
) -> Result<Vec<(AttachedHostRecord, Timestamp)>, StorageError> {
    const TABLE: &str = "conversation_hosts";
    let mut statement = connection.prepare_cached(
        "SELECT device_id, name, cwd, attached_at FROM conversation_hosts WHERE conversation_id = ?1
         ORDER BY attached_at, name",
    )?;
    let mut rows = statement.query([id.as_str()])?;
    let mut hosts = Vec::new();
    while let Some(row) = rows.next()? {
        let host = AttachedHostRecord {
            device: decode(
                TABLE,
                "device_id",
                DeviceId::try_from(row.get::<_, String>("device_id")?),
            )?,
            name: row.get("name")?,
            cwd: row.get("cwd")?,
        };
        hosts.push((host, instant(row, TABLE, "attached_at")?));
    }
    Ok(hosts)
}

/// Attaches `device` to the conversation under the first free name within
/// it: `name`, then `name-2`, `name-3` and so on; an empty name is the
/// device's id. A device attached already keeps its row, and the answer is
/// false. For a transaction that attaches hosts with its other writes, such
/// as a target switch's or a Fork's.
pub fn insert_attached_host(
    connection: &Connection,
    conversation: &ConversationId,
    host: &AttachedHostRecord,
    now: Timestamp,
) -> Result<bool, StorageError> {
    let base = match host.name.trim() {
        "" => host.device.as_str(),
        trimmed => trimmed,
    };
    let mut statement = connection
        .prepare_cached("SELECT name FROM conversation_hosts WHERE conversation_id = ?1")?;
    let taken = statement
        .query_map([conversation.as_str()], |row| row.get::<_, String>(0))?
        .collect::<Result<std::collections::HashSet<String>, _>>()?;
    let mut candidate = base.to_owned();
    let mut suffix = 2;
    while taken.contains(&candidate) {
        candidate = format!("{base}-{suffix}");
        suffix += 1;
    }
    let inserted = connection.execute(
        "INSERT INTO conversation_hosts (conversation_id, device_id, name, cwd, attached_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (conversation_id, device_id) DO NOTHING",
        params![
            conversation.as_str(),
            host.device.as_str(),
            candidate,
            host.cwd,
            now.as_millisecond()
        ],
    )? == 1;
    if inserted {
        raise_hosts_revision(connection, conversation)?;
    }
    Ok(inserted)
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
                first.title.as_str(),
                &first.target,
                first.model.clone(),
                first.read_revision
            ),
            (
                "New conversation",
                &ConversationTarget::Cloud { path: None },
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

    #[tokio::test]
    async fn the_saved_wakeups_list_the_unarchived_conversations_earliest_first() {
        let data = tempfile::tempdir().unwrap();
        let control = ControlService::open(
            &data.path().join("control.sqlite"),
            Arc::new(demi_shared_types::SystemClock),
        )
        .await
        .unwrap();
        let master = testing::master(&control).await.id;
        for number in 1..=4 {
            control
                .create_conversation(master.clone(), conversation(number), ConversationStart::default())
                .await
                .unwrap();
        }
        let later = WakeupDue::At(Timestamp::from_millisecond(9_000).unwrap());
        control
            .set_wakeup(conversation(1), Some(later))
            .await
            .unwrap();
        control
            .set_wakeup(conversation(2), Some(WakeupDue::AtStart))
            .await
            .unwrap();
        control
            .set_wakeup(conversation(3), Some(later))
            .await
            .unwrap();
        control
            .change_conversation(conversation(3), RecordChange::Archived(true))
            .await
            .unwrap();
        // A wakeup that fired leaves its conversation with none.
        control
            .set_wakeup(conversation(4), Some(later))
            .await
            .unwrap();
        control.set_wakeup(conversation(4), None).await.unwrap();
        let saved = |number, due| SavedWakeup {
            conversation: conversation(number),
            owner: master.clone(),
            due,
        };
        assert_eq!(
            control.saved_wakeups().await.unwrap(),
            [saved(2, WakeupDue::AtStart), saved(1, later)]
        );
    }

    #[tokio::test]
    async fn a_host_attaches_once_under_a_name_free_in_its_conversation() {
        let data = tempfile::tempdir().unwrap();
        let control = ControlService::open(
            &data.path().join("control.sqlite"),
            Arc::new(demi_shared_types::SystemClock),
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
        let host = |device: &DeviceId, name: &str, cwd: Option<&str>| AttachedHostRecord {
            device: device.clone(),
            name: name.into(),
            cwd: cwd.map(str::to_owned),
        };
        let attaching = vec![
            host(&devices[0], "laptop", None),
            host(&devices[1], "laptop", Some("/work")),
            host(&devices[2], " ", None),
            // Attached already: its row stays as it is.
            host(&devices[0], "renamed", Some("/elsewhere")),
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
        // Attached in one transaction, at one time: listed by name.
        let attached = control.attached_hosts(id).await.unwrap();
        let mut expected = vec![
            host(&devices[0], "laptop", None),
            host(&devices[1], "laptop-2", Some("/work")),
            host(&devices[2], devices[2].as_str(), None),
        ];
        expected.sort_by(|first, second| first.name.cmp(&second.name));
        assert_eq!(attached, expected);
        control.close().await.unwrap();
    }

    #[tokio::test]
    async fn of_two_switches_from_one_target_one_wins_and_its_announcement_stays() {
        let data = tempfile::tempdir().unwrap();
        let control = ControlService::open(
            &data.path().join("control.sqlite"),
            Arc::new(demi_shared_types::SystemClock),
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
        let cloud = ConversationTarget::Cloud { path: None };
        let from = ExecutionTarget::Cloud {
            device_id: None,
            path: format!("/home/demi/sessions/{id}"),
        };
        let switch_to = |path: &str| {
            let to = ConversationTarget::Device {
                device_id: laptop.clone(),
                path: path.into(),
            };
            let switch = TargetSwitch {
                from: from.clone(),
                to: ExecutionTarget::Device {
                    device_id: laptop.clone(),
                    path: path.into(),
                },
            };
            let ends = SwitchEnds {
                departed: None,
                arriving: Some(laptop.clone()),
            };
            control.switch_conversation_target(id.clone(), cloud.clone(), to, switch, ends)
        };
        let (first, second) = tokio::join!(switch_to("/first"), switch_to("/second"));
        assert_eq!((first.unwrap(), second.unwrap()), (Some(1), None));
        let record = control.conversation(id.clone()).await.unwrap().unwrap();
        assert_eq!(
            (record.target, record.context_version),
            (
                ConversationTarget::Device {
                    device_id: laptop.clone(),
                    path: "/first".into()
                },
                1
            )
        );
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
        assert_eq!(
            announced.to,
            ExecutionTarget::Device {
                device_id: laptop,
                path: "/first".into()
            }
        );
        control.close().await.unwrap();
    }
}
