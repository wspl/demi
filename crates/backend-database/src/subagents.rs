//! The users' subagent settings (`storage.md` § Control records;
//! `subagents.md` § Profiles): each user's Subagent switch, a user with no
//! row having subagents on, and each user's subagent profiles. A write takes
//! values its caller checked; a profile's name is unique among its user's,
//! which the write that sets it checks in its own transaction.

use demi_web_api_protocol::conversations::ModelSettings;
use demi_web_api_protocol::ids::{ProfileId, UserId};
use demi_web_api_protocol::subagents::{NewProfile, SubagentProfile, SubagentSettings};
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::StorageError;
use super::columns::{decode, json, to_json};
use super::control::ControlService;

const TABLE: &str = "subagent_profiles";

const PROFILE_COLUMNS: &str = "id, name, description, model, instructions, can_spawn, enabled";

/// Why a profile write was not made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProfileRefusal {
    #[error("No such subagent profile")]
    NotFound,
    #[error("Another of your subagent profiles has that name")]
    Exists,
}

impl ControlService {
    /// `user`'s subagent settings: the switch, and the profiles in name
    /// order.
    pub async fn subagent_settings(&self, user: UserId) -> Result<SubagentSettings, StorageError> {
        self.call(move |connection, _| {
            let enabled: Option<bool> = connection
                .query_row(
                    "SELECT enabled FROM user_subagents WHERE user_id = ?1",
                    [user.as_str()],
                    |row| row.get(0),
                )
                .optional()?;
            let mut statement = connection.prepare_cached(&format!(
                "SELECT {PROFILE_COLUMNS} FROM subagent_profiles WHERE user_id = ?1 ORDER BY name"
            ))?;
            let mut rows = statement.query([user.as_str()])?;
            let mut profiles = Vec::new();
            while let Some(row) = rows.next()? {
                profiles.push(profile_row(row)?);
            }
            Ok(SubagentSettings {
                enabled: enabled.unwrap_or(true),
                profiles,
            })
        })
        .await
    }

    /// Records that `user` has subagents on or off.
    pub async fn set_subagents(&self, user: UserId, enabled: bool) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "INSERT INTO user_subagents (user_id, enabled) VALUES (?1, ?2)
                 ON CONFLICT (user_id) DO UPDATE SET enabled = excluded.enabled",
                params![user.as_str(), enabled],
            )?;
            Ok(())
        })
        .await
    }

    /// Creates `profile` for `user`, enabled, and answers it as stored.
    pub async fn create_profile(
        &self,
        user: UserId,
        profile: NewProfile,
    ) -> Result<Result<SubagentProfile, ProfileRefusal>, StorageError> {
        let id = ProfileId::try_from(uuid::Uuid::new_v4().to_string()).expect("a UUID is not empty");
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            if name_taken(&transaction, &user, &profile.name, None)? {
                return Ok(Err(ProfileRefusal::Exists));
            }
            let created = SubagentProfile {
                id,
                name: profile.name,
                description: profile.description,
                model: profile.model,
                instructions: profile.instructions,
                can_spawn: profile.can_spawn,
                enabled: true,
            };
            transaction.execute(
                "INSERT INTO subagent_profiles
                   (id, user_id, name, description, model, instructions, can_spawn, enabled)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    created.id.as_str(),
                    user.as_str(),
                    created.name,
                    created.description,
                    created.model.as_ref().map(to_json),
                    created.instructions,
                    created.can_spawn,
                    created.enabled,
                ],
            )?;
            transaction.commit()?;
            Ok(Ok(created))
        })
        .await
    }

    /// Changes `user`'s profile `id` with `change`, which takes the stored
    /// profile and answers it changed, and answers it as stored. The read,
    /// the change, the name's check and the write are one transaction.
    pub async fn patch_profile(
        &self,
        user: UserId,
        id: ProfileId,
        change: impl FnOnce(SubagentProfile) -> SubagentProfile + Send + 'static,
    ) -> Result<Result<SubagentProfile, ProfileRefusal>, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let Some(stored) = stored_profile(&transaction, &user, &id)? else {
                return Ok(Err(ProfileRefusal::NotFound));
            };
            let changed = change(stored);
            if name_taken(&transaction, &user, &changed.name, Some(&id))? {
                return Ok(Err(ProfileRefusal::Exists));
            }
            transaction.execute(
                "UPDATE subagent_profiles
                 SET name = ?3, description = ?4, model = ?5, instructions = ?6, can_spawn = ?7,
                     enabled = ?8
                 WHERE user_id = ?1 AND id = ?2",
                params![
                    user.as_str(),
                    id.as_str(),
                    changed.name,
                    changed.description,
                    changed.model.as_ref().map(to_json),
                    changed.instructions,
                    changed.can_spawn,
                    changed.enabled,
                ],
            )?;
            transaction.commit()?;
            Ok(Ok(changed))
        })
        .await
    }

    /// Deletes `user`'s profile `id`; false when the user has none of that
    /// id.
    pub async fn delete_profile(&self, user: UserId, id: ProfileId) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let deleted = connection.execute(
                "DELETE FROM subagent_profiles WHERE user_id = ?1 AND id = ?2",
                params![user.as_str(), id.as_str()],
            )?;
            Ok(deleted > 0)
        })
        .await
    }
}

/// `user`'s profile `id`, as stored.
fn stored_profile(
    connection: &Connection,
    user: &UserId,
    id: &ProfileId,
) -> Result<Option<SubagentProfile>, StorageError> {
    let mut statement = connection.prepare_cached(&format!(
        "SELECT {PROFILE_COLUMNS} FROM subagent_profiles WHERE user_id = ?1 AND id = ?2"
    ))?;
    let mut rows = statement.query(params![user.as_str(), id.as_str()])?;
    rows.next()?.map(profile_row).transpose()
}

/// Whether another of `user`'s profiles than `except` has `name`.
fn name_taken(
    connection: &Connection,
    user: &UserId,
    name: &str,
    except: Option<&ProfileId>,
) -> Result<bool, StorageError> {
    let taken = connection.query_row(
        "SELECT EXISTS (SELECT 1 FROM subagent_profiles
                        WHERE user_id = ?1 AND name = ?2 AND id IS NOT ?3)",
        params![user.as_str(), name, except.map(ProfileId::as_str)],
        |row| row.get(0),
    )?;
    Ok(taken)
}

/// A `subagent_profiles` row, read from its columns in `PROFILE_COLUMNS`.
fn profile_row(row: &Row<'_>) -> Result<SubagentProfile, StorageError> {
    let model: Option<String> = row.get("model")?;
    Ok(SubagentProfile {
        id: decode(TABLE, "id", ProfileId::try_from(row.get::<_, String>("id")?))?,
        name: row.get("name")?,
        description: row.get("description")?,
        model: model
            .map(|text| json::<ModelSettings>(TABLE, "model", &text))
            .transpose()?,
        instructions: row.get("instructions")?,
        can_spawn: row.get("can_spawn")?,
        enabled: row.get("enabled")?,
    })
}
