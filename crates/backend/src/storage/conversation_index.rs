//! The conversation index of the control store (`storage.md` § Control
//! records): each conversation's owner, title, archive and pin state, place
//! in the sidebar, read revision, target, execution-context revision and
//! model selection. The agent tree itself is in the conversation's own
//! database. A conversation's id is the one the browser chose, kept in the
//! case it arrived in and compared without case, so an id another
//! conversation holds in any spelling is taken.

use demi_core::Timestamp;
use demi_web_api::conversations::ConversationTarget;
use demi_web_api::ids::{ConversationId, DeviceId, ProviderId, UserId, WorkspaceId};
use garde::Validate;
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::StorageError;
use super::columns::{decode, instant};
use super::control::ControlService;

/// A conversation as the index holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConversationRecord {
    pub(crate) id: ConversationId,
    pub(crate) owner: UserId,
    pub(crate) title: String,
    pub(crate) archived: bool,
    pub(crate) pinned: bool,
    /// The output revision the user last acknowledged; it only moves
    /// forward.
    pub(crate) read_revision: u64,
    pub(crate) target: ConversationTarget,
    /// Advanced by every change of the conversation's execution context,
    /// such as a target switch or a Host attached.
    pub(crate) context_version: u64,
    /// The provider entry and model the conversation last selected.
    pub(crate) model: Option<ConversationModel>,
    pub(crate) created_at: Timestamp,
    pub(crate) updated_at: Timestamp,
}

/// The provider entry and model a conversation selected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConversationModel {
    pub(crate) provider: ProviderId,
    pub(crate) model: String,
}

/// What asking for a conversation of an id found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Creation {
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

const CONVERSATION_COLUMNS: &str = "id, user_id, title, archived, pinned, read_revision, target_kind, target_device_id,
     target_path, target_workspace_id, context_version, provider_id, model_id, created_at, updated_at";

/// A target as its typed columns: the kind and what the kind names.
pub(crate) struct TargetColumns {
    pub(crate) kind: &'static str,
    pub(crate) device: Option<String>,
    pub(crate) path: Option<String>,
    pub(crate) workspace: Option<String>,
}

impl TargetColumns {
    pub(crate) fn of(target: &ConversationTarget) -> Self {
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
    /// conversation has the id: on the Cloud, with the placeholder title,
    /// first in the owner's sidebar. A retry of the owner's finds the one it
    /// created, in the spelling it was created with.
    pub(crate) async fn create_conversation(
        &self,
        owner: UserId,
        id: ConversationId,
    ) -> Result<Creation, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let reserved = transaction
                .query_row(
                    "SELECT 1 FROM conversation_fork_operations WHERE id = ?1",
                    [id.as_str()],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if reserved {
                return Ok(Creation::Unavailable);
            }
            let target = TargetColumns::of(&ConversationTarget::Cloud { path: None });
            let inserted = transaction.execute(
                "INSERT INTO conversations (id, user_id, title, title_origin, archived, pinned, sort_order,
                   read_revision, target_kind, target_device_id, target_path, target_workspace_id, context_version,
                   user_messages, titled_messages, created_at, updated_at)
                 VALUES (?1, ?2, ?3, 'placeholder', 0, 0,
                   (SELECT COALESCE(MIN(sort_order), 0) - 1 FROM conversations WHERE user_id = ?2),
                   0, ?4, ?5, ?6, ?7, 0, 0, 0, ?8, ?8)
                 ON CONFLICT (id) DO NOTHING",
                params![
                    id.as_str(),
                    owner.as_str(),
                    PLACEHOLDER_TITLE,
                    target.kind,
                    target.device,
                    target.path,
                    target.workspace,
                    now.as_millisecond()
                ],
            )?;
            let record = conversation_by_id(&transaction, &id)?.ok_or_else(|| StorageError::Corrupt {
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
    pub(crate) async fn conversation(&self, id: ConversationId) -> Result<Option<ConversationRecord>, StorageError> {
        self.call(move |connection, _| conversation_by_id(connection, &id)).await
    }

    /// The owner's conversations that are archived, or that are not, in
    /// sidebar order: pinned first, then by the user's order.
    pub(crate) async fn conversations(
        &self,
        owner: UserId,
        archived: bool,
    ) -> Result<Vec<ConversationRecord>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare(&format!(
                "SELECT {CONVERSATION_COLUMNS} FROM conversations WHERE user_id = ?1 AND archived = ?2
                 ORDER BY pinned DESC, sort_order, id"
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

    /// Acknowledges the output up to `revision`; an acknowledgement never
    /// moves the read revision back.
    pub(crate) async fn mark_conversation_read(&self, id: ConversationId, revision: u64) -> Result<(), StorageError> {
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

    /// Records the provider entry and model the conversation selected, or
    /// that it selects none.
    pub(crate) async fn set_conversation_model(
        &self,
        id: ConversationId,
        model: Option<ConversationModel>,
    ) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            let (provider, model) = match &model {
                Some(selected) => (Some(selected.provider.as_str()), Some(selected.model.as_str())),
                None => (None, None),
            };
            connection.execute(
                "UPDATE conversations SET provider_id = ?2, model_id = ?3 WHERE id = ?1",
                params![id.as_str(), provider, model],
            )?;
            Ok(())
        })
        .await
    }

    /// Records activity in the conversation now. Activity never reorders the
    /// sidebar.
    pub(crate) async fn touch_conversation(&self, id: ConversationId) -> Result<(), StorageError> {
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

/// A revision as the INTEGER column holds it. The output revision advances
/// by one per save, so no revision the backend acknowledges comes near
/// `i64::MAX`; a larger one is held as the largest the column keeps, which
/// acknowledges everything as the larger number would.
fn revision_column(revision: u64) -> i64 {
    i64::try_from(revision).unwrap_or(i64::MAX)
}

pub(crate) fn conversation_by_id(
    connection: &Connection,
    id: &ConversationId,
) -> Result<Option<ConversationRecord>, StorageError> {
    let mut statement = connection.prepare(&format!("SELECT {CONVERSATION_COLUMNS} FROM conversations WHERE id = ?1"))?;
    let mut rows = statement.query([id.as_str()])?;
    rows.next()?.map(conversation_row).transpose()
}

/// A `conversations` row, read from its columns in `CONVERSATION_COLUMNS`.
fn conversation_row(row: &Row<'_>) -> Result<ConversationRecord, StorageError> {
    const TABLE: &str = "conversations";
    let provider: Option<String> = row.get("provider_id")?;
    let model: Option<String> = row.get("model_id")?;
    let model = match (provider, model) {
        (Some(provider), Some(model)) => Some(ConversationModel {
            provider: decode(TABLE, "provider_id", ProviderId::try_from(provider))?,
            model,
        }),
        (None, None) => None,
        _ => {
            return Err(StorageError::Corrupt {
                table: TABLE,
                column: "model_id",
                reason: "a provider and a model are selected together or not at all".into(),
            });
        }
    };
    Ok(ConversationRecord {
        id: decode(TABLE, "id", ConversationId::try_from(row.get::<_, String>("id")?))?,
        owner: decode(TABLE, "user_id", UserId::try_from(row.get::<_, String>("user_id")?))?,
        title: row.get("title")?,
        archived: row.get("archived")?,
        pinned: row.get("pinned")?,
        read_revision: decode(TABLE, "read_revision", u64::try_from(row.get::<_, i64>("read_revision")?))?,
        target: target_row(row)?,
        context_version: decode(TABLE, "context_version", u64::try_from(row.get::<_, i64>("context_version")?))?,
        model,
        created_at: instant(row, TABLE, "created_at")?,
        updated_at: instant(row, TABLE, "updated_at")?,
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
                device_id: decode(TABLE, "target_device_id", DeviceId::try_from(device.unwrap_or_default()))?,
                path: path.unwrap_or_default(),
            }
        }
        "workspace" => {
            let workspace: Option<String> = row.get("target_workspace_id")?;
            ConversationTarget::Workspace {
                workspace_id: decode(TABLE, "target_workspace_id", WorkspaceId::try_from(workspace.unwrap_or_default()))?,
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
/// § Attached hosts): a Host the conversation reaches besides its main one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AttachedHostRecord {
    pub(crate) device: DeviceId,
    /// What the model and the user call the host; unique within the
    /// conversation.
    pub(crate) name: String,
    /// Where the last `demi host shell --host` there ended; none until one
    /// ran.
    pub(crate) cwd: Option<String>,
}

impl ControlService {
    /// The conversation's attached hosts, first attached first.
    #[cfg_attr(not(test), expect(dead_code, reason = "the conversation's host access and Fork read them"))]
    pub(crate) async fn attached_hosts(&self, id: ConversationId) -> Result<Vec<AttachedHostRecord>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(
                "SELECT device_id, name, cwd FROM conversation_hosts WHERE conversation_id = ?1
                 ORDER BY attached_at, name",
            )?;
            let mut rows = statement.query([id.as_str()])?;
            let mut hosts = Vec::new();
            while let Some(row) = rows.next()? {
                hosts.push(AttachedHostRecord {
                    device: decode(
                        "conversation_hosts",
                        "device_id",
                        DeviceId::try_from(row.get::<_, String>("device_id")?),
                    )?,
                    name: row.get("name")?,
                    cwd: row.get("cwd")?,
                });
            }
            Ok(hosts)
        })
        .await
    }
}

/// Attaches `device` to the conversation under the first free name within
/// it: `name`, then `name-2`, `name-3` and so on; an empty name is the
/// device's id. A device attached already keeps its row. For a transaction
/// that attaches hosts with its other writes, such as a target switch's or a
/// Fork's.
#[cfg_attr(not(test), expect(dead_code, reason = "a target switch and a Fork attach hosts"))]
pub(crate) fn insert_attached_host(
    connection: &Connection,
    conversation: &ConversationId,
    host: &AttachedHostRecord,
    now: Timestamp,
) -> Result<(), StorageError> {
    let base = match host.name.trim() {
        "" => host.device.as_str(),
        trimmed => trimmed,
    };
    let mut statement = connection.prepare_cached("SELECT name FROM conversation_hosts WHERE conversation_id = ?1")?;
    let taken = statement
        .query_map([conversation.as_str()], |row| row.get::<_, String>(0))?
        .collect::<Result<std::collections::HashSet<String>, _>>()?;
    let mut candidate = base.to_owned();
    let mut suffix = 2;
    while taken.contains(&candidate) {
        candidate = format!("{base}-{suffix}");
        suffix += 1;
    }
    connection.execute(
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
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::storage::control::testing;

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
        let control = ControlService::open(&data.path().join("control.sqlite"), Arc::new(demi_core::SystemClock))
            .await
            .unwrap();
        let master = testing::master(&control).await.id;
        let first = created(control.create_conversation(master.clone(), conversation(1)).await.unwrap());
        assert_eq!(
            (first.title.as_str(), &first.target, first.model.clone(), first.read_revision),
            ("New conversation", &ConversationTarget::Cloud { path: None }, None, 0)
        );
        let second = created(control.create_conversation(master.clone(), conversation(2)).await.unwrap());

        // Another spelling names the same conversation, which keeps the
        // spelling it was created with.
        let upper = ConversationId::try_from(conversation(1).as_str().to_uppercase()).unwrap();
        assert_eq!(control.conversation(upper.clone()).await.unwrap(), Some(first.clone()));
        assert_eq!(
            control.create_conversation(master.clone(), upper).await.unwrap(),
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
        assert!(control.conversations(master, true).await.unwrap().is_empty());

        control.mark_conversation_read(first.id.clone(), 5).await.unwrap();
        control.mark_conversation_read(first.id.clone(), 3).await.unwrap();
        let model = ConversationModel {
            provider: ProviderId::try_from("entry-1").unwrap(),
            model: "claude-opus-4-8".into(),
        };
        control
            .set_conversation_model(first.id.clone(), Some(model.clone()))
            .await
            .unwrap();
        control.touch_conversation(first.id.clone()).await.unwrap();
        let read = control.conversation(first.id.clone()).await.unwrap().unwrap();
        assert_eq!((read.read_revision, read.model), (5, Some(model)));
        assert!(read.updated_at >= first.updated_at);

        // A row outside its type is refused, never repaired.
        testing::execute(
            &control,
            "UPDATE conversations SET provider_id = NULL WHERE id = ?1",
            vec![first.id.as_str().to_owned()],
        )
        .await;
        let refused = control.conversation(first.id.clone()).await.unwrap_err();
        assert!(
            matches!(refused, StorageError::Corrupt { table: "conversations", column: "model_id", .. }),
            "{refused}"
        );
        control.close().await.unwrap();
    }

    #[tokio::test]
    async fn a_host_attaches_once_under_a_name_free_in_its_conversation() {
        let data = tempfile::tempdir().unwrap();
        let control = ControlService::open(&data.path().join("control.sqlite"), Arc::new(demi_core::SystemClock))
            .await
            .unwrap();
        let master = testing::master(&control).await.id;
        let id = created(control.create_conversation(master.clone(), conversation(1)).await.unwrap()).id;
        let mut devices = Vec::new();
        for (name, token) in [("laptop", "one"), ("laptop", "two"), ("ci", "three")] {
            let hash = crate::auth::sessions::TokenHash::of(token);
            let device = control
                .create_device(master.clone(), name.into(), "linux".into(), hash)
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
                    for host in &attaching {
                        insert_attached_host(connection, &id, host, now)?;
                    }
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
}
