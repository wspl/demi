//! A user's integrations: programs that build an agent product on Demi through
//! its API (`crates/backend-user-shard/src/integrations.rs`).
//!
//! An integration's conversations are in a workspace of its own, which no page
//! lists, so no sidebar, search or switcher of its creator shows them. Its
//! token is kept as its hash, as a session's is: the token is shown once, when
//! it is made.

use demi_shared_types::{IntegrationCapabilities, Timestamp};
use demi_web_api_protocol::ids::{ConversationId, DeviceId, IntegrationId, UserId, WorkspaceId};
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::StorageError;
use super::accounts::TokenHash;
use super::columns::{decode, json, to_json};
use super::control::ControlService;
use super::workspaces::insert_integration_workspace;

const TABLE: &str = "integrations";

/// The name a new integration takes, and the root of the names of the ones
/// after it: *Integration*, *Integration 2*.
pub const INTEGRATION_NAME: &str = "Integration";

/// An integration as storage keeps it, without its token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationRecord {
    pub id: IntegrationId,
    pub user: UserId,
    /// The workspace its conversations are in, which no page lists.
    pub workspace: WorkspaceId,
    pub name: String,
    pub capabilities: IntegrationCapabilities,
    pub created_at: Timestamp,
}

/// A change of an integration's settings; a field left out stays.
#[derive(Debug, Clone, Default)]
pub struct IntegrationPatch {
    pub name: Option<String>,
    pub capabilities: Option<IntegrationCapabilities>,
}

/// What an integration's conversation keeps beside the conversation's own
/// record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationConversation {
    pub conversation: ConversationId,
    pub integration: IntegrationId,
    pub key: Option<String>,
    /// The workflow it was created with, YAML; none for no subagent
    /// profiles.
    pub workflow: Option<String>,
}

/// One of an integration's Clouds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationCloud {
    pub device: DeviceId,
    pub integration: IntegrationId,
    pub key: Option<String>,
}

const COLUMNS: &str = "id, user_id, workspace_id, name, capabilities, created_at";

impl ControlService {
    /// Makes an integration of `user`'s with every setting at its default,
    /// named *Integration*, or *Integration 2* and on for a user who has one
    /// by that name, with its workspace; answers it with its token, which
    /// storage keeps only the hash of.
    pub async fn create_integration(&self, user: UserId) -> Result<(IntegrationRecord, String), StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let token = new_token();
            let id = IntegrationId::try_from(uuid::Uuid::new_v4().to_string()).expect("a UUID is not empty");
            let workspace = insert_integration_workspace(&transaction, &user, now)?;
            let name = next_name(&transaction, &user)?;
            transaction.execute(
                "INSERT INTO integrations (id, user_id, workspace_id, name, token_hash, capabilities, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    id.as_str(),
                    user.as_str(),
                    workspace.as_str(),
                    name,
                    TokenHash::of(&token).as_str(),
                    to_json(&IntegrationCapabilities::default()),
                    now.as_millisecond(),
                ],
            )?;
            let record = integration_in(&transaction, &id)?.ok_or(StorageError::Corrupt {
                table: TABLE,
                column: "id",
                reason: "an integration just made was not found".into(),
            })?;
            transaction.commit()?;
            Ok((record, token))
        })
        .await
    }

    /// `user`'s integrations, oldest first.
    pub async fn integrations(&self, user: UserId) -> Result<Vec<IntegrationRecord>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(&format!(
                "SELECT {COLUMNS} FROM integrations WHERE user_id = ?1 ORDER BY created_at, id"
            ))?;
            let mut rows = statement.query([user.as_str()])?;
            let mut integrations = Vec::new();
            while let Some(row) = rows.next()? {
                integrations.push(integration_row(row)?);
            }
            Ok(integrations)
        })
        .await
    }

    /// How many Clouds and how many conversations the integration `id`
    /// holds.
    pub async fn integration_counts(&self, id: IntegrationId) -> Result<(u32, u32), StorageError> {
        self.call(move |connection, _| {
            Ok(connection.query_row(
                "SELECT (SELECT COUNT(*) FROM integration_clouds WHERE integration_id = ?1),
                        (SELECT COUNT(*) FROM integration_conversations WHERE integration_id = ?1)",
                [id.as_str()],
                |row| Ok((row.get::<_, u32>(0)?, row.get::<_, u32>(1)?)),
            )?)
        })
        .await
    }

    pub async fn integration(&self, id: IntegrationId) -> Result<Option<IntegrationRecord>, StorageError> {
        self.call(move |connection, _| integration_in(connection, &id)).await
    }

    /// The integration whose token hashes to `token`; none for a token no
    /// integration has, and for one of an account that is disabled.
    pub async fn integration_by_token(&self, token: TokenHash) -> Result<Option<IntegrationRecord>, StorageError> {
        self.call(move |connection, _| {
            Ok(connection
                .query_row(
                    &format!(
                        "SELECT {COLUMNS} FROM integrations WHERE token_hash = ?1
                           AND user_id IN (SELECT id FROM users WHERE disabled = 0)"
                    ),
                    [token.as_str()],
                    |row| Ok(integration_row(row)),
                )
                .optional()?
                .transpose()?)
        })
        .await
    }

    /// Applies `patch` to the integration `id`; answers it as it is now, none
    /// for an integration that is gone.
    pub async fn update_integration(
        &self,
        id: IntegrationId,
        patch: IntegrationPatch,
    ) -> Result<Option<IntegrationRecord>, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            if let Some(name) = &patch.name {
                transaction.execute("UPDATE integrations SET name = ?2 WHERE id = ?1", params![id.as_str(), name])?;
            }
            if let Some(capabilities) = &patch.capabilities {
                transaction.execute(
                    "UPDATE integrations SET capabilities = ?2 WHERE id = ?1",
                    params![id.as_str(), to_json(capabilities)],
                )?;
            }
            let record = integration_in(&transaction, &id)?;
            transaction.commit()?;
            Ok(record)
        })
        .await
    }

    /// Gives the integration `id` a new token, which the old one no longer
    /// opens; answers the new token, none for an integration that is gone.
    pub async fn replace_integration_token(&self, id: IntegrationId) -> Result<Option<String>, StorageError> {
        self.call(move |connection, _| {
            let token = new_token();
            let changed = connection.execute(
                "UPDATE integrations SET token_hash = ?2 WHERE id = ?1",
                params![id.as_str(), TokenHash::of(&token).as_str()],
            )?;
            Ok((changed == 1).then_some(token))
        })
        .await
    }

    /// Records the conversation `conversation`, just made in the
    /// integration's workspace, as the integration's, with its key and
    /// workflow. A key another conversation of the integration has answers
    /// that conversation instead, and records nothing.
    pub async fn record_integration_conversation(
        &self,
        record: IntegrationConversation,
    ) -> Result<Option<ConversationId>, StorageError> {
        self.call(move |connection, _| {
            let inserted = connection.execute(
                "INSERT INTO integration_conversations (conversation_id, integration_id, external_key, workflow)
                 VALUES (?1, ?2, ?3, ?4) ON CONFLICT DO NOTHING",
                params![
                    record.conversation.as_str(),
                    record.integration.as_str(),
                    record.key,
                    record.workflow
                ],
            )?;
            if inserted == 1 {
                return Ok(None);
            }
            integration_conversation_by_key(connection, &record.integration, record.key.as_deref())
        })
        .await
    }

    /// The integration's conversations, newest first, each with its key.
    pub async fn integration_conversations(
        &self,
        integration: IntegrationId,
    ) -> Result<Vec<IntegrationConversation>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(
                "SELECT i.conversation_id, i.integration_id, i.external_key, i.workflow
                 FROM integration_conversations i JOIN conversations c ON c.id = i.conversation_id
                 WHERE i.integration_id = ?1 ORDER BY c.created_at DESC, c.id",
            )?;
            let mut rows = statement.query([integration.as_str()])?;
            let mut conversations = Vec::new();
            while let Some(row) = rows.next()? {
                conversations.push(integration_conversation_row(row)?);
            }
            Ok(conversations)
        })
        .await
    }

    /// The plugin the integration last declared, as JSON; none before its
    /// first declaration.
    pub async fn integration_declaration(&self, id: IntegrationId) -> Result<Option<String>, StorageError> {
        self.call(move |connection, _| {
            Ok(connection
                .query_row("SELECT declaration FROM integrations WHERE id = ?1", [id.as_str()], |row| {
                    row.get::<_, Option<String>>(0)
                })
                .optional()?
                .flatten())
        })
        .await
    }

    /// Keeps `declaration`, JSON, as the plugin the integration declared.
    pub async fn declare_integration(&self, id: IntegrationId, declaration: String) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "UPDATE integrations SET declaration = ?2 WHERE id = ?1",
                params![id.as_str(), declaration],
            )?;
            Ok(())
        })
        .await
    }

    /// The integration's conversation of key `key`.
    pub async fn integration_conversation_by_key(
        &self,
        integration: IntegrationId,
        key: String,
    ) -> Result<Option<ConversationId>, StorageError> {
        self.call(move |connection, _| integration_conversation_by_key(connection, &integration, Some(&key)))
            .await
    }

    /// What the conversation `id` keeps as an integration's; none for a
    /// conversation of the user's own.
    pub async fn integration_conversation(
        &self,
        id: ConversationId,
    ) -> Result<Option<IntegrationConversation>, StorageError> {
        self.call(move |connection, _| {
            Ok(connection
                .query_row(
                    "SELECT conversation_id, integration_id, external_key, workflow
                     FROM integration_conversations WHERE conversation_id = ?1",
                    [id.as_str()],
                    |row| Ok(integration_conversation_row(row)),
                )
                .optional()?
                .transpose()?)
        })
        .await
    }

    /// Makes a Cloud of the integration's, of the integration's user, with
    /// `key`; a key another Cloud of the integration has answers that Cloud
    /// instead, and makes nothing.
    pub async fn create_integration_cloud(
        &self,
        integration: IntegrationId,
        key: Option<String>,
    ) -> Result<IntegrationCloud, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            if let Some(key) = &key
                && let Some(found) = integration_cloud_by_key(&transaction, &integration, key)?
            {
                return Ok(found);
            }
            let record = integration_in(&transaction, &integration)?.ok_or(StorageError::Corrupt {
                table: TABLE,
                column: "id",
                reason: "a Cloud of an integration that is gone".into(),
            })?;
            let device = super::devices::insert_cloud(&transaction, &record.user, now)?;
            transaction.execute(
                "INSERT INTO integration_clouds (device_id, integration_id, external_key) VALUES (?1, ?2, ?3)",
                params![device.id.as_str(), integration.as_str(), key],
            )?;
            transaction.commit()?;
            Ok(IntegrationCloud {
                device: device.id,
                integration,
                key,
            })
        })
        .await
    }

    /// The integration's Clouds, oldest first.
    pub async fn integration_clouds(&self, integration: IntegrationId) -> Result<Vec<IntegrationCloud>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare_cached(
                "SELECT c.device_id, c.integration_id, c.external_key FROM integration_clouds c
                 JOIN devices d ON d.id = c.device_id
                 WHERE c.integration_id = ?1 ORDER BY d.claimed_at, d.id",
            )?;
            let mut rows = statement.query([integration.as_str()])?;
            let mut clouds = Vec::new();
            while let Some(row) = rows.next()? {
                clouds.push(integration_cloud_row(row)?);
            }
            Ok(clouds)
        })
        .await
    }

    /// The integration's Cloud of key `key`.
    pub async fn integration_cloud_by_key(
        &self,
        integration: IntegrationId,
        key: String,
    ) -> Result<Option<IntegrationCloud>, StorageError> {
        self.call(move |connection, _| integration_cloud_by_key(connection, &integration, &key)).await
    }

    /// The integration a Cloud `device` is of; none for one of the user's
    /// own.
    pub async fn integration_cloud(&self, device: DeviceId) -> Result<Option<IntegrationCloud>, StorageError> {
        self.call(move |connection, _| {
            Ok(connection
                .query_row(
                    "SELECT device_id, integration_id, external_key FROM integration_clouds WHERE device_id = ?1",
                    [device.as_str()],
                    |row| Ok(integration_cloud_row(row)),
                )
                .optional()?
                .transpose()?)
        })
        .await
    }
}

/// A token of an integration: 256 random bits in hexadecimal, after a prefix
/// that tells it from a person's secrets where it is pasted.
fn new_token() -> String {
    format!("demi_int_{}", hex::encode(rand::random::<[u8; 32]>()))
}

/// The name a new integration of `user`'s takes: *Integration*, or the first
/// *Integration N* from 2 on that none of theirs has.
fn next_name(connection: &Connection, user: &UserId) -> Result<String, StorageError> {
    let taken = |name: &str| -> Result<bool, StorageError> {
        Ok(connection
            .query_row(
                "SELECT 1 FROM integrations WHERE user_id = ?1 AND name = ?2",
                params![user.as_str(), name],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    };
    if !taken(INTEGRATION_NAME)? {
        return Ok(INTEGRATION_NAME.to_owned());
    }
    let mut number = 2;
    loop {
        let name = format!("{INTEGRATION_NAME} {number}");
        if !taken(&name)? {
            return Ok(name);
        }
        number += 1;
    }
}

fn integration_in(connection: &Connection, id: &IntegrationId) -> Result<Option<IntegrationRecord>, StorageError> {
    Ok(connection
        .query_row(&format!("SELECT {COLUMNS} FROM integrations WHERE id = ?1"), [id.as_str()], |row| {
            Ok(integration_row(row))
        })
        .optional()?
        .transpose()?)
}

fn integration_row(row: &Row<'_>) -> Result<IntegrationRecord, StorageError> {
    Ok(IntegrationRecord {
        id: decode(TABLE, "id", IntegrationId::try_from(row.get::<_, String>("id")?))?,
        user: decode(TABLE, "user_id", UserId::try_from(row.get::<_, String>("user_id")?))?,
        workspace: decode(TABLE, "workspace_id", WorkspaceId::try_from(row.get::<_, String>("workspace_id")?))?,
        name: row.get("name")?,
        capabilities: json(TABLE, "capabilities", &row.get::<_, String>("capabilities")?)?,
        created_at: decode(TABLE, "created_at", Timestamp::from_millisecond(row.get("created_at")?))?,
    })
}

fn integration_conversation_by_key(
    connection: &Connection,
    integration: &IntegrationId,
    key: Option<&str>,
) -> Result<Option<ConversationId>, StorageError> {
    let Some(key) = key else {
        return Ok(None);
    };
    connection
        .query_row(
            "SELECT conversation_id FROM integration_conversations WHERE integration_id = ?1 AND external_key = ?2",
            params![integration.as_str(), key],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(|id| decode("integration_conversations", "conversation_id", ConversationId::try_from(id.as_str())))
        .transpose()
}

fn integration_conversation_row(row: &Row<'_>) -> Result<IntegrationConversation, StorageError> {
    const TABLE: &str = "integration_conversations";
    Ok(IntegrationConversation {
        conversation: decode(TABLE, "conversation_id", ConversationId::try_from(row.get::<_, String>(0)?.as_str()))?,
        integration: decode(TABLE, "integration_id", IntegrationId::try_from(row.get::<_, String>(1)?))?,
        key: row.get(2)?,
        workflow: row.get(3)?,
    })
}

fn integration_cloud_by_key(
    connection: &Connection,
    integration: &IntegrationId,
    key: &str,
) -> Result<Option<IntegrationCloud>, StorageError> {
    Ok(connection
        .query_row(
            "SELECT device_id, integration_id, external_key FROM integration_clouds
             WHERE integration_id = ?1 AND external_key = ?2",
            params![integration.as_str(), key],
            |row| Ok(integration_cloud_row(row)),
        )
        .optional()?
        .transpose()?)
}

fn integration_cloud_row(row: &Row<'_>) -> Result<IntegrationCloud, StorageError> {
    const TABLE: &str = "integration_clouds";
    Ok(IntegrationCloud {
        device: decode(TABLE, "device_id", DeviceId::try_from(row.get::<_, String>(0)?))?,
        integration: decode(TABLE, "integration_id", IntegrationId::try_from(row.get::<_, String>(1)?))?,
        key: row.get(2)?,
    })
}
