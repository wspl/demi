//! The users' subagent settings (`storage.md` § Control records;
//! `subagents.md` § Profiles, § Limits): each user's Subagents switch and
//! limits, a user with no row having subagents on at the default limits,
//! and each user's subagent profiles. A write takes values its caller
//! checked; a profile's name is unique among its user's, which the write
//! that sets it checks in its own transaction.

use std::num::NonZeroU32;

use demi_web_api_protocol::conversations::ModelSettings;
use demi_web_api_protocol::ids::{ProfileId, UserId};
use demi_web_api_protocol::subagents::{
    NewProfile, SubagentLimit, SubagentProfile, SubagentSettings, SubagentsPatch,
};
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::StorageError;
use super::columns::{decode, json, to_json};
use super::control::ControlService;

const TABLE: &str = "subagent_profiles";

/// Maximum subagents of a user who never set it (`subagents.md` § Limits);
/// the control schema's migration gives it to a limit stored as Unlimited
/// before the number was kept.
pub(crate) const DEFAULT_MAX_LIVE: NonZeroU32 = NonZeroU32::new(10).expect("10 is not zero");
/// Maximum depth of a user who never set it; the migration gives it as
/// Maximum subagents' default.
pub(crate) const DEFAULT_MAX_DEPTH: NonZeroU32 = NonZeroU32::MIN;

/// A user's switch and limits, as `user_subagents` holds them.
struct Switches {
    enabled: bool,
    max_live: SubagentLimit,
    max_depth: SubagentLimit,
}

impl Default for Switches {
    /// A user with no row: subagents on, at the default limits.
    fn default() -> Self {
        Self {
            enabled: true,
            max_live: SubagentLimit {
                limit: DEFAULT_MAX_LIVE,
                unlimited: false,
            },
            max_depth: SubagentLimit {
                limit: DEFAULT_MAX_DEPTH,
                unlimited: false,
            },
        }
    }
}

const PROFILE_COLUMNS: &str =
    "id, name, description, model, context_limit, instructions, can_spawn, enabled";

/// Why a profile write was not made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProfileRefusal {
    #[error("No such subagent profile")]
    NotFound,
    #[error("Another of your subagent profiles has that name")]
    Exists,
}

impl ControlService {
    /// `user`'s subagent settings: the switch, the limits, and the profiles
    /// in name order.
    pub async fn subagent_settings(&self, user: UserId) -> Result<SubagentSettings, StorageError> {
        self.call(move |connection, _| {
            let switches = stored_switches(connection, &user)?.unwrap_or_default();
            let mut statement = connection.prepare_cached(&format!(
                "SELECT {PROFILE_COLUMNS} FROM subagent_profiles WHERE user_id = ?1 ORDER BY name"
            ))?;
            let mut rows = statement.query([user.as_str()])?;
            let mut profiles = Vec::new();
            while let Some(row) = rows.next()? {
                profiles.push(profile_row(row)?);
            }
            Ok(SubagentSettings {
                enabled: switches.enabled,
                max_live: switches.max_live,
                max_depth: switches.max_depth,
                profiles,
            })
        })
        .await
    }

    /// Sets what `patch` names of `user`'s switch and limits, keeping the
    /// rest; the read and the write are one transaction.
    pub async fn set_subagents(&self, user: UserId, patch: SubagentsPatch) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let stored = stored_switches(&transaction, &user)?.unwrap_or_default();
            let switches = Switches {
                enabled: patch.enabled.unwrap_or(stored.enabled),
                max_live: patch.max_live.unwrap_or(stored.max_live),
                max_depth: patch.max_depth.unwrap_or(stored.max_depth),
            };
            transaction.execute(
                "INSERT INTO user_subagents
                   (user_id, enabled, max_live, max_live_unlimited, max_depth, max_depth_unlimited)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT (user_id) DO UPDATE SET enabled = excluded.enabled,
                   max_live = excluded.max_live, max_live_unlimited = excluded.max_live_unlimited,
                   max_depth = excluded.max_depth, max_depth_unlimited = excluded.max_depth_unlimited",
                params![
                    user.as_str(),
                    switches.enabled,
                    switches.max_live.limit.get(),
                    switches.max_live.unlimited,
                    switches.max_depth.limit.get(),
                    switches.max_depth.unlimited,
                ],
            )?;
            transaction.commit()?;
            Ok(())
        })
        .await
    }

    /// Creates `profile` for `user`, enabled or not, and answers it as
    /// stored.
    pub async fn create_profile(
        &self,
        user: UserId,
        profile: NewProfile,
        enabled: bool,
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
                context_limit: profile.context_limit,
                instructions: profile.instructions,
                can_spawn: profile.can_spawn,
                enabled,
            };
            transaction.execute(
                "INSERT INTO subagent_profiles
                   (id, user_id, name, description, model, context_limit, instructions, can_spawn, enabled)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    created.id.as_str(),
                    user.as_str(),
                    created.name,
                    created.description,
                    created.model.as_ref().map(to_json),
                    created.context_limit,
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
                     enabled = ?8, context_limit = ?9
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
                    changed.context_limit,
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

/// `user`'s switch and limits, when the user has a row.
fn stored_switches(connection: &Connection, user: &UserId) -> Result<Option<Switches>, StorageError> {
    let row = connection
        .query_row(
            "SELECT enabled, max_live, max_live_unlimited, max_depth, max_depth_unlimited
             FROM user_subagents WHERE user_id = ?1",
            [user.as_str()],
            |row| Ok((row.get(0)?, (row.get(1)?, row.get(2)?), (row.get(3)?, row.get(4)?))),
        )
        .optional()?;
    row.map(|(enabled, max_live, max_depth): (bool, (i64, bool), (i64, bool))| {
        Ok(Switches {
            enabled,
            max_live: limit("max_live", max_live)?,
            max_depth: limit("max_depth", max_depth)?,
        })
    })
    .transpose()
}

/// A stored limit: its whole number from 1, and whether it is lifted.
fn limit(column: &'static str, (value, unlimited): (i64, bool)) -> Result<SubagentLimit, StorageError> {
    let parsed = u32::try_from(value)
        .ok()
        .and_then(NonZeroU32::new)
        .ok_or_else(|| format!("{value} is not a whole number from 1"));
    Ok(SubagentLimit {
        limit: decode("user_subagents", column, parsed)?,
        unlimited,
    })
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
        context_limit: row
            .get::<_, Option<i64>>("context_limit")?
            .map(|tokens| decode(TABLE, "context_limit", u32::try_from(tokens)))
            .transpose()?,
        instructions: row.get("instructions")?,
        can_spawn: row.get("can_spawn")?,
        enabled: row.get("enabled")?,
    })
}
