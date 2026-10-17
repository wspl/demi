//! The schemas of the control database and of each conversation's database
//! (`storage.md` § Schemas and migrations): one SQL text each, applied to a
//! new database in a transaction, whose digest is the version a database
//! records, and the history of the schemas published releases shipped
//! before it, each with the migration to the next. Times are integer
//! milliseconds since the Unix epoch; a closed set is text a CHECK limits;
//! JSON columns are text their reader decodes and validates; sealed values
//! are BLOBs.

use std::{collections::HashMap, path::Path};

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};
use sha2::{Digest, Sha256};

use demi_agent_store::CheckpointState;
use bytes::Bytes;
use demi_shared_types::{BlobRef, ReportEvent, Timestamp};
use demi_web_api_protocol::ids::ProviderId;

use super::StorageError;
use super::columns::{decode, json, to_json};
use super::providers::{NoSealedConfigs, ResealConfigs};

/// A database's schema: its SQL, whose digest names it, and the schemas
/// that published releases shipped before it, oldest first.
pub(crate) struct Schema {
    sql: &'static str,
    history: &'static [Shipped],
}

/// A schema a published release shipped, and the migration from it to the
/// schema after it in the history, or to the current one after the last.
pub(crate) struct Shipped {
    sql: &'static str,
    migration: Migration,
}

/// What turns a database of one schema into one of the next.
pub(crate) enum Migration {
    Sql(&'static str),
    /// A change SQL cannot express, such as re-encoding a stored value,
    /// which may read the blobs the database's values name.
    Code(fn(&Transaction<'_>, &dyn ReadBlob) -> rusqlite::Result<()>),
    /// A change that re-encodes sealed values, which only their owner can
    /// open and seal again.
    Reseal(fn(&Transaction<'_>, &dyn ResealConfigs) -> rusqlite::Result<()>),
}

/// How a migration reads the bytes of a blob a database's values name, in
/// the namespace they are in; none when the namespace does not hold it.
/// It runs on the database's thread, which waits for it.
pub trait ReadBlob {
    fn read(&self, blob: &BlobRef) -> Result<Option<Bytes>, String>;
}

/// A database whose values name no blob, such as the control database.
pub(crate) struct NoBlobs;

impl ReadBlob for NoBlobs {
    fn read(&self, blob: &BlobRef) -> Result<Option<Bytes>, String> {
        Err(format!("this database names no blob, yet a migration read {blob}"))
    }
}

/// The control database's. Its history holds the schema of each published
/// release before the one that ships the current schema, and of each
/// development build a server keeps data of (`upgrades.md` § A development
/// build on a server): 0.1.22-dev.20261011T123543Z's, the one with the
/// plugins' secrets before a request had a read state, the one with the
/// users' credentials before passkeys, the one with passkeys before the
/// signed-in browsers, the one with the signed-in browsers before the
/// messages saved for later, and the one with them before administered
/// accounts, the one with administered accounts before a conversation's
/// workflow, the one with the workflow before the edits and sends of
/// saved messages were kept, the one with them before an attached host
/// went by its device's name, the one with device names before a
/// subagent profile had a Context of its own, and the one with a profile's
/// Context before an API-key entry's sealed configuration had a format,
/// and the one with that format before account rotation, and the one with
/// account rotation before a placeholder title held no words, the one with
/// placeholder titles before interrupted turns were kept, and the one with
/// interrupted turns before a conversation kept the hosts it may reach,
/// the one with them before shares, the one with shares before a
/// device's mark had a number, 0.1.22-dev.20261015T105826Z's, before a
/// project remembered the model and workflow its next conversation starts
/// with, 0.1.22-dev.20261015T134426Z's, before a quota window had a
/// kind instead of an English name, 0.1.22-dev.20261016T043704Z's,
/// before the usage ledger kept a row per vendor call and before the
/// Claude Code machine and the accounts whose CLI keeps their sign-in, and
/// 0.1.22-dev.20261016T095626Z's, before each account had the size of its
/// Cloud disks, and 0.1.22's, before a ledger row could be an image's.
/// The schemas with the subagent
/// limits, without Detect Title, with the kept limits and with the plugins'
/// secrets and read states no server keeps, but development databases do,
/// and the server's migrate through them.
pub(crate) const CONTROL: Schema = Schema {
    sql: CONTROL_V1,
    history: &[
        Shipped {
            sql: include_str!("schema/control-0.1.11.sql"),
            migration: Migration::Sql(CONTROL_FROM_0_1_11),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.13.sql"),
            migration: Migration::Sql(CONTROL_FROM_0_1_13),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.15.sql"),
            migration: Migration::Sql(CONTROL_FROM_0_1_15),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.16.sql"),
            migration: Migration::Sql(CONTROL_FROM_0_1_16),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.17.sql"),
            migration: Migration::Sql(CONTROL_FROM_0_1_17),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.18.sql"),
            migration: Migration::Sql(CONTROL_FROM_0_1_18),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.20.sql"),
            migration: Migration::Sql(CONTROL_FROM_0_1_20),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.21.sql"),
            migration: Migration::Code(control_from_0_1_21),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.22-dev.20261011T123543Z.sql"),
            migration: Migration::Sql(CONTROL_FROM_0_1_22_DEV_20261011T123543Z),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.22-dev.subagent-limits.sql"),
            migration: Migration::Sql(CONTROL_FROM_SUBAGENT_LIMITS),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.22-dev.no-detect-title.sql"),
            migration: Migration::Sql(CONTROL_FROM_NO_DETECT_TITLE),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.22-dev.kept-limits.sql"),
            migration: Migration::Sql(CONTROL_FROM_KEPT_LIMITS),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.22-dev.plugin-secrets-unread.sql"),
            migration: Migration::Sql(CONTROL_FROM_PLUGIN_SECRETS_UNREAD),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.22-dev.plugin-secrets.sql"),
            migration: Migration::Sql(CONTROL_FROM_PLUGIN_SECRETS),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.22-dev.credentials.sql"),
            migration: Migration::Sql(CONTROL_FROM_CREDENTIALS),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.22-dev.passkeys.sql"),
            migration: Migration::Sql(CONTROL_FROM_PASSKEYS),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.22-dev.signed-in-browsers.sql"),
            migration: Migration::Sql(CONTROL_FROM_SIGNED_IN_BROWSERS),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.22-dev.saved-messages.sql"),
            migration: Migration::Sql(CONTROL_FROM_SAVED_MESSAGES),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.22-dev.administered-accounts.sql"),
            migration: Migration::Sql(CONTROL_FROM_ADMINISTERED_ACCOUNTS),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.22-dev.workflows.sql"),
            migration: Migration::Sql(CONTROL_FROM_WORKFLOWS),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.22-dev.saved-edits.sql"),
            migration: Migration::Code(control_from_saved_edits),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.22-dev.device-names.sql"),
            migration: Migration::Sql(CONTROL_FROM_DEVICE_NAMES),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.22-dev.profile-contexts.sql"),
            migration: Migration::Reseal(control_sealed_config_format_2),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.22-dev.config-format-2.sql"),
            migration: Migration::Sql(CONTROL_FROM_CONFIG_FORMAT_2),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.22-dev.account-rotation.sql"),
            migration: Migration::Sql(CONTROL_FROM_ACCOUNT_ROTATION),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.22-dev.placeholder-titles.sql"),
            migration: Migration::Sql(CONTROL_FROM_PLACEHOLDER_TITLES),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.22-dev.interrupted-turns.sql"),
            migration: Migration::Sql(CONTROL_FROM_INTERRUPTED_TURNS),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.22-dev.reachable-hosts.sql"),
            migration: Migration::Sql(CONTROL_FROM_REACHABLE_HOSTS),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.22-dev.shares.sql"),
            migration: Migration::Sql(CONTROL_FROM_SHARES),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.22-dev.20261015T105826Z.sql"),
            migration: Migration::Code(control_from_0_1_22_dev_20261015t105826z),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.22-dev.20261015T134426Z.sql"),
            migration: Migration::Code(snapshots_with_window_kinds),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.22-dev.20261016T043704Z.sql"),
            migration: Migration::Code(control_from_0_1_22_dev_20261016t043704z),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.22-dev.20261016T095626Z.sql"),
            migration: Migration::Sql(CONTROL_FROM_0_1_22_DEV_20261016T095626Z),
        },
        Shipped {
            sql: include_str!("schema/control-0.1.22.sql"),
            migration: Migration::Sql(CONTROL_FROM_0_1_22),
        },
    ],
};

/// From 0.1.22-dev.20261016T095626Z's control schema, which a server keeps
/// data of, to 0.1.22's: each account has the size of its Cloud
/// disks, 16 GB each until an administrator changes it (`users.md` § User
/// system).
const CONTROL_FROM_0_1_22_DEV_20261016T095626Z: &str = "
ALTER TABLE users ADD COLUMN cloud_system_gb INTEGER NOT NULL DEFAULT 16 CHECK (cloud_system_gb BETWEEN 1 AND 1024);
ALTER TABLE users ADD COLUMN cloud_home_gb INTEGER NOT NULL DEFAULT 16 CHECK (cloud_home_gb BETWEEN 1 AND 1024);
";

/// From 0.1.22's control schema to the current one: a usage ledger row may
/// be an image's (`images.md` § Accounts and failures), which SQLite's
/// checks let in only on a table made again, its rows as they were; each
/// browser's push subscription is kept (`turns.md` § Notifications); and a
/// running job may wait to be sent to its device (`sessions-and-targets.md`
/// § Host operations), which every one recorded before was.
const CONTROL_FROM_0_1_22: &str = "
ALTER TABLE running_jobs ADD COLUMN sent INTEGER NOT NULL DEFAULT 1 CHECK (sent IN (0, 1));
CREATE TABLE usage_ledger_images (
  id                    TEXT PRIMARY KEY,
  user_id               TEXT NOT NULL REFERENCES users (id),
  conversation_id       TEXT NOT NULL,
  -- Null for a request outside a session, such as a title.
  agent                 INTEGER CHECK (agent >= 0),
  turn_id               TEXT,
  purpose               TEXT CHECK (purpose IN ('turn', 'compaction', 'title', 'image')),
  provider_id           TEXT NOT NULL,
  -- Null for an API key.
  account_id            TEXT,
  model_id              TEXT NOT NULL,
  request_id            TEXT,
  vendor_id             TEXT,
  input_tokens          INTEGER,
  output_tokens         INTEGER,
  cache_read_tokens     INTEGER,
  cache_write_tokens    INTEGER,
  cache_write_1h_tokens INTEGER,
  -- 'answered', 'cancelled' or the failure code of a refused call.
  outcome               TEXT,
  quota                 TEXT,
  started_at            INTEGER,
  ended_at              INTEGER NOT NULL
) STRICT;
INSERT INTO usage_ledger_images SELECT * FROM usage_ledger ORDER BY rowid;
DROP TABLE usage_ledger;
ALTER TABLE usage_ledger_images RENAME TO usage_ledger;
CREATE INDEX usage_ledger_user ON usage_ledger (user_id, ended_at);
-- A browser's push subscription of a user (turns.md § Notifications), one
-- per session, which it serves and which takes it along when it ends: its
-- push service's endpoint, which names the browser's subscription, its two
-- keys and the events it chose.
CREATE TABLE push_subscriptions (
  session_id       TEXT PRIMARY KEY REFERENCES web_sessions (id) ON DELETE CASCADE,
  endpoint         TEXT NOT NULL UNIQUE,
  user_id          TEXT NOT NULL REFERENCES users (id),
  p256dh           TEXT NOT NULL,
  auth             TEXT NOT NULL,
  turn_finishes    INTEGER NOT NULL CHECK (turn_finishes IN (0, 1)),
  turn_fails       INTEGER NOT NULL CHECK (turn_fails IN (0, 1)),
  needs_permission INTEGER NOT NULL CHECK (needs_permission IN (0, 1)),
  agent_notifies   INTEGER NOT NULL CHECK (agent_notifies IN (0, 1)),
  created_at       INTEGER NOT NULL
) STRICT;
CREATE INDEX push_subscriptions_of_user ON push_subscriptions (user_id);
";

/// From 0.1.22-dev.20261016T043704Z's control schema, which a server keeps
/// data of, to the current one, in two steps. First the usage ledger keeps
/// a row per vendor call (`usage-and-quota.md` § Usage ledger), a managed
/// device says which managed machine it is, and a Claude Code account may
/// hold no secret, [`CONTROL_FROM_0_1_22_DEV_20261016T043704Z`]. Then where a
/// conversation runs decides its project (`upgrades.md` § What crosses
/// releases): a device keeps the home its runner last reported, none until
/// its runner's next hello; a Cloud workspace's directory loses a trailing
/// separator, and the user has one workspace per device and directory, of
/// several the oldest, which the others' conversations and pending moves
/// move to; a Cloud workspace at the Cloud's home goes, its conversations
/// running in the home; a workspace selection keeps its device and
/// directory; a Cloud conversation in a directory of its own choosing moves
/// to the Cloud's home, `/home/demi`, a conversation's directory under
/// `/home/demi/sessions`, or else the workspace of its directory, made when
/// there is none; and a permission request of a category and a host at
/// once, which only the removed `demi conversation move` raised, goes. A
/// paired device's workspaces and conversations wait for its runner to
/// resolve their directories when it first reports its home (`devices.rs`).
fn control_from_0_1_22_dev_20261016t043704z(transaction: &Transaction<'_>, blobs: &dyn ReadBlob) -> rusqlite::Result<()> {
    transaction.execute_batch(CONTROL_FROM_0_1_22_DEV_20261016T043704Z)?;
    control_by_directory(transaction, blobs)
}

/// The usage ledger's step of [`control_from_0_1_22_dev_20261016t043704z`],
/// to the ledger that keeps a row per vendor call with
/// its agent, turn, purpose, account, ids, outcome, observed quota and
/// times (`usage-and-quota.md` § Usage ledger). A row written before keeps
/// null for what it lacked; its `created_at`, written as its response
/// ended, is its `ended_at`. A call's counts are null when the vendor
/// reported none. In the same schema a managed device says which managed
/// machine it is, the user's Cloud or the master's Claude Code machine
/// (`managed-hosts.md` § The Claude Code machine), and a subscription
/// account may hold no secret, as a Claude Code account whose CLI keeps its
/// sign-in does, and says whether the vendor signed it out
/// (`claude-code.md` § Accounts and sign-in). Every managed device of the
/// earlier schema is a Cloud. A Claude Code account keeps the tokens it
/// holds until its first use hands them over. No table refers to
/// `provider_credentials`, so it is made again.
const CONTROL_FROM_0_1_22_DEV_20261016T043704Z: &str = "
CREATE TABLE usage_ledger_calls (
  id                    TEXT PRIMARY KEY,
  user_id               TEXT NOT NULL REFERENCES users (id),
  conversation_id       TEXT NOT NULL,
  agent                 INTEGER CHECK (agent >= 0),
  turn_id               TEXT,
  purpose               TEXT CHECK (purpose IN ('turn', 'compaction', 'title')),
  provider_id           TEXT NOT NULL,
  account_id            TEXT,
  model_id              TEXT NOT NULL,
  request_id            TEXT,
  vendor_id             TEXT,
  input_tokens          INTEGER,
  output_tokens         INTEGER,
  cache_read_tokens     INTEGER,
  cache_write_tokens    INTEGER,
  cache_write_1h_tokens INTEGER,
  outcome               TEXT,
  quota                 TEXT,
  started_at            INTEGER,
  ended_at              INTEGER NOT NULL
) STRICT;
INSERT INTO usage_ledger_calls (id, user_id, conversation_id, provider_id, model_id,
    input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, ended_at)
  SELECT id, user_id, conversation_id, provider_id, model_id,
    input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, created_at
  FROM usage_ledger ORDER BY rowid;
DROP TABLE usage_ledger;
ALTER TABLE usage_ledger_calls RENAME TO usage_ledger;
CREATE INDEX usage_ledger_user ON usage_ledger (user_id, ended_at);
ALTER TABLE devices ADD COLUMN machine TEXT CHECK (machine IN ('cloud', 'claude_code'));
UPDATE devices SET machine = 'cloud' WHERE kind = 'managed';
DROP INDEX devices_one_managed;
CREATE UNIQUE INDEX devices_one_machine ON devices (user_id, machine) WHERE kind = 'managed';
CREATE TABLE provider_credentials_new (
  provider_id  TEXT NOT NULL REFERENCES providers (id) ON DELETE CASCADE,
  id           TEXT NOT NULL,
  identity_key TEXT,
  label        TEXT NOT NULL,
  detail       TEXT,
  source       TEXT NOT NULL,
  secret       BLOB,
  version      INTEGER NOT NULL CHECK (version >= 1),
  quota        TEXT,
  updated_at   INTEGER NOT NULL,
  position     INTEGER NOT NULL DEFAULT 0 CHECK (position >= 0),
  signed_out   INTEGER NOT NULL DEFAULT 0 CHECK (signed_out IN (0, 1)),
  PRIMARY KEY (provider_id, id)
) STRICT;
INSERT INTO provider_credentials_new
  (provider_id, id, identity_key, label, detail, source, secret, version, quota, updated_at, position)
  SELECT provider_id, id, identity_key, label, detail, source, secret, version, quota, updated_at, position
  FROM provider_credentials;
DROP TABLE provider_credentials;
ALTER TABLE provider_credentials_new RENAME TO provider_credentials;
CREATE UNIQUE INDEX provider_credentials_identity ON provider_credentials (provider_id, identity_key)
  WHERE identity_key IS NOT NULL;
";

/// The projects-by-directory step of
/// [`control_from_0_1_22_dev_20261016t043704z`].
fn control_by_directory(transaction: &Transaction<'_>, _: &dyn ReadBlob) -> rusqlite::Result<()> {
    transaction.execute_batch(
        "
ALTER TABLE devices ADD COLUMN home TEXT;
-- A Cloud's directory as its Host names it, without a trailing separator; a
-- paired device's waits for its runner (devices.rs, settle_first_home).
UPDATE workspaces SET path = CASE WHEN rtrim(path, '/') = '' THEN '/' ELSE rtrim(path, '/') END
  WHERE device_id IN (SELECT id FROM devices WHERE kind = 'managed');
-- Workspaces of one directory become the oldest of them.
CREATE TEMP TABLE kept_workspaces AS
  SELECT w.id AS id, (SELECT k.id FROM workspaces k
      WHERE k.user_id = w.user_id AND k.device_id = w.device_id AND k.path = w.path
      ORDER BY k.created_at, k.id LIMIT 1) AS kept
  FROM workspaces w;
UPDATE conversations SET target_workspace_id = (SELECT kept FROM kept_workspaces WHERE id = target_workspace_id)
  WHERE target_workspace_id IS NOT NULL;
UPDATE conversations SET pending_workspace_id = (SELECT kept FROM kept_workspaces WHERE id = pending_workspace_id)
  WHERE pending_workspace_id IS NOT NULL;
DELETE FROM workspaces WHERE id IN (SELECT id FROM kept_workspaces WHERE kept <> id);
DROP TABLE kept_workspaces;
CREATE UNIQUE INDEX workspaces_directory ON workspaces (user_id, device_id, path);
",
    )?;
    // The table's CHECK changes. Making the table again would drop the
    // table every conversation's rows name, which cascades while foreign
    // keys are on, as they are in a migration's transaction; its columns
    // stay as they are, so its text changes in place, the procedure SQLite
    // gives for a change of constraints alone (lang_altertable.html § 7),
    // to the current schema's own text. The rows the new CHECK refuses are
    // rewritten next, and the integrity check, which tests CHECK
    // constraints, proves none is left.
    let start = CONTROL_V1
        .find("CREATE TABLE conversations (")
        .expect("the control schema makes the conversations table");
    let end = start + CONTROL_V1[start..].find(") STRICT;").expect("the table ends") + ") STRICT".len();
    let version: i64 = transaction.pragma_query_value(None, "schema_version", |row| row.get(0))?;
    transaction.pragma_update(None, "writable_schema", true)?;
    transaction.execute(
        "UPDATE sqlite_schema SET sql = ?1 WHERE type = 'table' AND name = 'conversations'",
        [&CONTROL_V1[start..end]],
    )?;
    transaction.pragma_update(None, "schema_version", version + 1)?;
    transaction.pragma_update(None, "writable_schema", false)?;
    transaction.execute_batch(
        "
UPDATE conversations SET
    target_device_id = (SELECT device_id FROM workspaces WHERE id = target_workspace_id),
    target_path = (SELECT path FROM workspaces WHERE id = target_workspace_id)
  WHERE target_kind = 'workspace';
-- A Cloud workspace at the Cloud's home goes: its conversations run in the
-- home, outside projects, and a move into it waits as a move to the home.
CREATE TEMP TABLE cloud_homes AS
  SELECT w.id AS id, w.device_id AS device_id FROM workspaces w JOIN devices d ON d.id = w.device_id
  WHERE d.kind = 'managed' AND w.path = '/home/demi';
UPDATE conversations SET target_kind = 'home', target_path = NULL, target_workspace_id = NULL
  WHERE target_workspace_id IN (SELECT id FROM cloud_homes);
UPDATE conversations SET pending_kind = 'cloud', pending_path = '/home/demi', pending_workspace_id = NULL
  WHERE pending_workspace_id IN (SELECT id FROM cloud_homes);
DELETE FROM workspaces WHERE id IN (SELECT id FROM cloud_homes);
DROP TABLE cloud_homes;
-- A permission request asks for one category or one host now. Only
-- `demi conversation move`, which this release removes, asked for a
-- category and a host at once; such a request goes, as the command did.
DELETE FROM permission_requests WHERE kind = 'permission'
  AND json_array_length(categories) + (host_device_id IS NOT NULL) + (pairing_code IS NOT NULL) <> 1;
",
    )?;
    let moving = transaction
        .prepare("SELECT id, user_id, target_path FROM conversations WHERE target_kind = 'cloud' AND target_path IS NOT NULL")?
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let now = Timestamp::truncate(jiff::Timestamp::now());
    for (id, owner, path) in moving {
        let id = demi_web_api_protocol::ids::ConversationId::try_from(id.as_str()).map_err(corrupt)?;
        let owner = demi_web_api_protocol::ids::UserId::try_from(owner).map_err(corrupt)?;
        let cloud = crate::devices::cloud_device_in(transaction, &owner, now).map_err(storage)?;
        let selected = crate::selection::select_directory(transaction, &id, &cloud, &path, now)
            .map_err(storage)?
            .map_err(|reason| corrupt_text(&format!("a Cloud directory is unselectable: {reason:?}")))?;
        crate::devices::set_selection(transaction, &id, &selected.selection).map_err(storage)?;
    }
    let problems: Vec<String> = transaction
        .prepare("PRAGMA integrity_check")?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    if problems != ["ok"] {
        return Err(corrupt_text(&format!("the conversations' targets fail their check: {problems:?}")));
    }
    Ok(())
}

/// From 0.1.22-dev.20261015T105826Z's control schema, which a server keeps
/// data of, to the one in which a workspace keeps what its project's next
/// conversation starts with (`web-api-settings.md` § User preferences), none before
/// the first choice, and a Fork's metadata holds the workflow its
/// destination keeps (format 3). A Fork reserved before was reserved to
/// start with Global Workflow, which its record keeps.
fn control_from_0_1_22_dev_20261015t105826z(
    transaction: &Transaction<'_>,
    _: &dyn ReadBlob,
) -> rusqlite::Result<()> {
    transaction.execute_batch(
        "
ALTER TABLE workspaces ADD COLUMN last_model TEXT;
ALTER TABLE workspaces ADD COLUMN last_workflow_kind TEXT
    CHECK (last_workflow_kind IN ('default', 'none', 'workflow'));
ALTER TABLE workspaces ADD COLUMN last_workflow_id TEXT
    CHECK ((last_workflow_kind IS 'workflow') = (last_workflow_id IS NOT NULL));
",
    )?;
    let records = transaction
        .prepare("SELECT id, metadata FROM conversation_fork_operations")?
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut update = transaction.prepare("UPDATE conversation_fork_operations SET metadata = ?2 WHERE id = ?1")?;
    for (id, text) in records {
        let earlier: fork_format_2::ForkMetadata =
            json("conversation_fork_operations", "metadata", &text).map_err(corrupt)?;
        let metadata = crate::forks::ForkMetadata {
            title: earlier.title,
            target: earlier.target,
            model: earlier.model,
            workflow: demi_shared_types::WorkflowChoice::Default,
            created_at: earlier.created_at,
            attached_hosts: earlier.attached_hosts,
            context_version: earlier.context_version,
        };
        update.execute(rusqlite::params![id, to_json(&metadata)])?;
    }
    Ok(())
}

/// Fork metadata format 2, without the destination's workflow, as the
/// control schema before [`control_from_0_1_22_dev_20261015t105826z`]
/// stored it.
mod fork_format_2 {
    use demi_shared_types::{ModelSelection, Timestamp};
    use demi_web_api_protocol::conversations::ConversationTarget;
    use serde::{Deserialize, Serialize};

    use crate::conversation_index::AttachedHostRecord;

    #[derive(Serialize, Deserialize, garde::Validate)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(super) struct ForkMetadata {
        #[serde(deserialize_with = "Option::deserialize")]
        #[garde(length(min = 1))]
        pub title: Option<String>,
        #[garde(dive)]
        pub target: ConversationTarget,
        #[serde(deserialize_with = "Option::deserialize")]
        #[garde(dive)]
        pub model: Option<ModelSelection>,
        #[garde(skip)]
        pub created_at: Timestamp,
        #[garde(dive)]
        pub attached_hosts: Vec<AttachedHostRecord>,
        #[garde(skip)]
        pub context_version: u64,
    }
}

/// From the control schema that keeps each share's record to the one in
/// which a paired device keeps the number of its mark (`conversations.md`
/// § Conversations and projects). A device of the earlier schema takes its
/// number at its runner's next hello.
const CONTROL_FROM_SHARES: &str = "
ALTER TABLE devices ADD COLUMN mark_number INTEGER CHECK (mark_number > 0);
";

/// From the control schema in which a conversation keeps the hosts it may
/// reach to the one that keeps each share's record (`sharing.md` § Storage
/// and routes). A database of the earlier schema has shared nothing.
const CONTROL_FROM_REACHABLE_HOSTS: &str = "
CREATE TABLE shares (
  token           TEXT PRIMARY KEY,
  user_id         TEXT NOT NULL REFERENCES users (id),
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  kind            TEXT NOT NULL CHECK (kind IN ('conversation', 'answer')),
  block_id        TEXT,
  title           TEXT,
  taken_at        INTEGER NOT NULL,
  CHECK ((kind = 'answer') = (block_id IS NOT NULL))
) STRICT;
CREATE UNIQUE INDEX shares_of_conversation ON shares (conversation_id) WHERE kind = 'conversation';
CREATE UNIQUE INDEX shares_of_answer ON shares (conversation_id, block_id) WHERE kind = 'answer';
";

/// From the control schema that keeps the turns a shutdown interrupted to
/// the one in which a conversation keeps the hosts it may reach and a request
/// may ask for one (`sessions-and-targets.md` § Hosts a conversation may
/// reach, `permissions.md` § Hosts), which replace the Manage Devices
/// category. Every attached host seeds its conversation's list: a host
/// attached by name took Manage Devices to attach, and one attached with a
/// directory is a primary target the conversation left, which the list
/// holds anyway. The grants of Manage Devices go, and so do the undecided
/// requests that asked for it, which no command asks for any more; a
/// decided one stays until its message is delivered.
const CONTROL_FROM_INTERRUPTED_TURNS: &str = "
ALTER TABLE permission_requests ADD COLUMN host_device_id TEXT
    CHECK (host_device_id IS NULL OR kind = 'permission');
ALTER TABLE permission_requests ADD COLUMN pairing_code TEXT
    CHECK (pairing_code IS NULL OR (kind = 'permission' AND host_device_id IS NULL));
CREATE TABLE reachable_hosts (
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  device_id       TEXT NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
  added_at        INTEGER NOT NULL,
  PRIMARY KEY (conversation_id, device_id)
) STRICT;
CREATE INDEX reachable_hosts_of_device ON reachable_hosts (device_id);
CREATE TABLE pairing_passes (
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  code            TEXT NOT NULL,
  allowed_at      INTEGER NOT NULL,
  PRIMARY KEY (conversation_id, code)
) STRICT;
INSERT INTO reachable_hosts (conversation_id, device_id, added_at)
  SELECT conversation_id, device_id, attached_at FROM conversation_hosts;
DELETE FROM permission_grants WHERE category = 'host.devices';
DELETE FROM permission_requests
  WHERE kind = 'permission' AND decision IS NULL
    AND EXISTS (SELECT 1 FROM json_each(categories) WHERE value = 'host.devices');
";

/// From the control schema where a placeholder title holds no words to the
/// one that keeps the root turns a graceful shutdown interrupted
/// (`failures-and-recovery.md` § The unfinished turn). A database a shutdown
/// of the earlier release closed marks none: its interrupted turns wait for
/// the user's Resume.
const CONTROL_FROM_PLACEHOLDER_TITLES: &str = "
CREATE TABLE interrupted_turns (
  conversation_id TEXT PRIMARY KEY COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE
) STRICT;
";

/// From the control schema with account rotation, whose placeholder titles
/// held the English "New conversation", to the one where a placeholder
/// title holds no words (`conversations.md` § Conversation titles).
const CONTROL_FROM_ACCOUNT_ROTATION: &str =
    "UPDATE conversations SET title = '' WHERE title_origin = 'placeholder';";

/// From the control schema whose API-key entries sealed configuration
/// format 2 to the one with account rotation (`providers.md` § Account
/// rotation): every entry keeps its manual rotation, and each entry's
/// accounts keep the order they were listed in, by their ids.
const CONTROL_FROM_CONFIG_FORMAT_2: &str = "
ALTER TABLE providers ADD COLUMN rotation TEXT NOT NULL DEFAULT 'manual'
    CHECK (rotation IN ('manual', 'in_order', 'balanced')
      AND (credential_kind = 'subscription' OR rotation = 'manual'));
ALTER TABLE provider_credentials ADD COLUMN position INTEGER NOT NULL DEFAULT 0 CHECK (position >= 0);
UPDATE provider_credentials SET position = (
  SELECT count(*) FROM provider_credentials AS earlier
  WHERE earlier.provider_id = provider_credentials.provider_id AND earlier.id < provider_credentials.id
);
";

/// From the control schema whose API-key entries sealed their bare
/// configuration, format 1, to provider configuration format 2, which
/// tells a compatible entry by its naming no vendor (`models.md` § A
/// provider is in the list from the start). The vault opens, decodes and
/// checks each one and seals it again; one it cannot read stops the
/// migration, so the database stays as it was.
fn control_sealed_config_format_2(
    transaction: &Transaction<'_>,
    seals: &dyn ResealConfigs,
) -> rusqlite::Result<()> {
    let rows = transaction
        .prepare("SELECT id, provider_type, config FROM providers WHERE config IS NOT NULL")?
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Vec<u8>>(2)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut update = transaction.prepare("UPDATE providers SET config = ?2 WHERE id = ?1")?;
    for (id, family, sealed) in rows {
        let entry = decode("providers", "id", ProviderId::try_from(id.clone())).map_err(corrupt)?;
        let resealed = seals
            .provider_config(&entry, &family, &sealed)
            .map_err(|reason| {
                corrupt(StorageError::Corrupt {
                    table: "providers",
                    column: "config",
                    reason: format!("entry {id}: {reason}"),
                })
            })?;
        update.execute(rusqlite::params![id, resealed])?;
    }
    Ok(())
}

/// From the control schema in which an attached host goes by its device's
/// name to the one in which a subagent profile has a Context of its own
/// (`subagents.md` § Profiles): every stored profile follows the user's
/// limit, as it did.
const CONTROL_FROM_DEVICE_NAMES: &str =
    "ALTER TABLE subagent_profiles ADD COLUMN context_limit INTEGER CHECK (context_limit >= 1);";

/// From the control schema that kept the edits and sends of saved messages
/// to the one where an attached host goes by its device's name
/// (`sessions-and-targets.md` § Attached hosts) and a switch is known by the
/// revision it made.
///
/// The hosts' table loses the names conversations gave them, rebuilt since
/// SQLite drops no column a constraint names, with the rows in the order the
/// names were listed in, which their row ids keep. A Fork's metadata goes
/// from format 1, whose attached hosts carry names, to format 2, which
/// holds the source's execution-context revision at the Fork instead: each
/// record is decoded as format 1 and the result checked as format 2, so a
/// record of neither stops the migration. A reservation stays once its
/// destination is published, which never reads it again, and keeps the
/// destination's revision; one that waits
/// takes its source's revision now, at least the one at the Fork and so as
/// good. One whose source is gone too, deleted while the Fork waited, takes
/// 0: none is known, and the cost, a first switch of the destination that
/// its copied blocks hide, is the one a switch made before the upgrade
/// takes, below. A switch made before records revision
/// 0, which every node read or passed, since the revision it made is not
/// known: the conversation's current one may have moved past it while its
/// nodes were idle, and would announce a switch they saw once more. So a
/// switch made while its nodes were idle that none of them had read yet
/// loses its one announcement, which is accepted: the conversation's
/// transcript shows the user the move.
fn control_from_saved_edits(transaction: &Transaction<'_>, _: &dyn ReadBlob) -> rusqlite::Result<()> {
    transaction.execute_batch(
        "
CREATE TABLE conversation_hosts_next (
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id),
  device_id       TEXT NOT NULL REFERENCES devices (id),
  cwd             TEXT,
  attached_at     INTEGER NOT NULL,
  detaching       INTEGER NOT NULL DEFAULT 0 CHECK (detaching IN (0, 1)),
  PRIMARY KEY (conversation_id, device_id)
) STRICT;
INSERT INTO conversation_hosts_next (conversation_id, device_id, cwd, attached_at, detaching)
  SELECT conversation_id, device_id, cwd, attached_at, detaching FROM conversation_hosts
  ORDER BY attached_at, name;
DROP TABLE conversation_hosts;
ALTER TABLE conversation_hosts_next RENAME TO conversation_hosts;
CREATE INDEX conversation_hosts_device ON conversation_hosts (device_id);
ALTER TABLE conversations ADD COLUMN last_switch_revision INTEGER;
UPDATE conversations SET last_switch_revision = 0 WHERE last_switch IS NOT NULL;
",
    )?;
    let records = transaction
        .prepare(
            "SELECT fork.id, fork.metadata,
               coalesce(destination.context_version, source.context_version, 0)
             FROM conversation_fork_operations fork
               LEFT JOIN conversations destination ON destination.id = fork.id
               LEFT JOIN conversations source ON source.id = fork.source_id",
        )?
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut update = transaction.prepare("UPDATE conversation_fork_operations SET metadata = ?2 WHERE id = ?1")?;
    for (id, text, revision) in records {
        let context_version = decode("conversations", "context_version", u64::try_from(revision)).map_err(corrupt)?;
        let named: fork_format_1::ForkMetadata =
            json("conversation_fork_operations", "metadata", &text).map_err(corrupt)?;
        let metadata = fork_format_2::ForkMetadata {
            title: Some(named.title),
            target: named.target,
            model: named.model,
            created_at: named.created_at,
            attached_hosts: named
                .attached_hosts
                .into_iter()
                .map(|host| crate::conversation_index::AttachedHostRecord {
                    device: host.device,
                    cwd: host.cwd,
                })
                .collect(),
            context_version,
        };
        let text = to_json(&metadata);
        let _: fork_format_2::ForkMetadata =
            json("conversation_fork_operations", "metadata", &text).map_err(corrupt)?;
        update.execute(rusqlite::params![id, text])?;
    }
    Ok(())
}

/// Fork metadata format 1, which named each attached host, as the control
/// schema before [`control_from_saved_edits`] stored it.
mod fork_format_1 {
    use demi_shared_types::{ModelSelection, Timestamp};
    use demi_web_api_protocol::conversations::ConversationTarget;
    use demi_web_api_protocol::ids::DeviceId;
    use serde::Deserialize;

    #[derive(Deserialize, garde::Validate)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(super) struct ForkMetadata {
        #[garde(length(min = 1))]
        pub title: String,
        #[garde(dive)]
        pub target: ConversationTarget,
        #[serde(deserialize_with = "Option::deserialize")]
        #[garde(dive)]
        pub model: Option<ModelSelection>,
        #[garde(skip)]
        pub created_at: Timestamp,
        #[garde(dive)]
        pub attached_hosts: Vec<AttachedHost>,
    }

    #[derive(Deserialize, garde::Validate)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub(super) struct AttachedHost {
        #[garde(skip)]
        pub device: DeviceId,
        // Format 1's name, which format 2 drops: decoded and checked so a
        // record without one is not taken for format 1.
        #[garde(length(min = 1))]
        #[allow(dead_code)]
        pub name: String,
        #[serde(deserialize_with = "Option::deserialize")]
        #[garde(skip)]
        pub cwd: Option<String>,
    }
}

/// From the control schema with a conversation's workflow to the one that
/// keeps the edits of messages saved for later and their sends for a day
/// (`storage.md` § Control records): two new tables, empty.
const CONTROL_FROM_WORKFLOWS: &str = "
-- An edit of a message saved for later, by the page's id for it, with its
-- answer, kept a day so the same edit sent again answers as it first did.
-- answer: the edit's answer, `{ draft, revisions }` as `web-api.md`
-- § Messages saved for later names it, as JSON.
CREATE TABLE saved_edits (
  op              TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  answer          TEXT NOT NULL,
  made_at         INTEGER NOT NULL
) STRICT;
CREATE INDEX saved_edits_by_age ON saved_edits (made_at);

-- A message saved for later that a send took, with the turn the send became,
-- kept a day so the same send, or a save of it, sent again is known.
-- document: its text and files as saved_messages keeps them.
CREATE TABLE saved_sends (
  id              TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  turn_id         TEXT NOT NULL,
  saved_at        INTEGER NOT NULL,
  document        TEXT NOT NULL,
  sent_at         INTEGER NOT NULL
) STRICT;
CREATE INDEX saved_sends_by_age ON saved_sends (sent_at);
";

/// From the control schema with administered accounts to the one with a
/// conversation's Workflow menu choice (`subagents.md` § Workflows of a
/// project): every conversation uses Global Workflow, as a new one does.
const CONTROL_FROM_ADMINISTERED_ACCOUNTS: &str = "
ALTER TABLE conversations ADD COLUMN workflow_kind TEXT NOT NULL DEFAULT 'default' CHECK (workflow_kind IN ('default', 'none', 'workflow'));
ALTER TABLE conversations ADD COLUMN workflow_id TEXT CHECK ((workflow_kind = 'workflow') = (workflow_id IS NOT NULL));
";

/// From the control schema with the messages saved for later to the one
/// with administered accounts (`storage.md` § Control records): an account
/// is enabled and has chosen its own password, as every account that
/// exists did, and was last active when its latest session was; a session was not made by an administrator's code; each
/// user's one handoff code becomes their handoff code of its kind, and
/// no sign-in code exists. The codes' table is rebuilt, since SQLite changes
/// a primary key only so.
const CONTROL_FROM_SAVED_MESSAGES: &str = "
ALTER TABLE users ADD COLUMN disabled INTEGER NOT NULL DEFAULT 0 CHECK (disabled IN (0, 1));
ALTER TABLE users ADD COLUMN chooses_password INTEGER NOT NULL DEFAULT 0 CHECK (chooses_password IN (0, 1));
ALTER TABLE users ADD COLUMN last_active_at INTEGER;
UPDATE users SET last_active_at = (SELECT max(last_active_at) FROM web_sessions WHERE user_id = users.id);
ALTER TABLE web_sessions ADD COLUMN issued_by TEXT REFERENCES users (id);
CREATE TABLE handoff_codes_new (
  user_id    TEXT NOT NULL REFERENCES users (id),
  kind       TEXT NOT NULL CHECK (kind IN ('handoff', 'sign_in')),
  issued_by  TEXT REFERENCES users (id),
  id         TEXT NOT NULL UNIQUE,
  code_hash  TEXT NOT NULL UNIQUE,
  to_path    TEXT NOT NULL,
  expires_at INTEGER NOT NULL,
  spent      INTEGER NOT NULL CHECK (spent IN (0, 1)),
  PRIMARY KEY (user_id, kind),
  CHECK ((kind = 'sign_in') = (issued_by IS NOT NULL))
) STRICT;
INSERT INTO handoff_codes_new (user_id, kind, id, code_hash, to_path, expires_at, spent)
  SELECT user_id, 'handoff', id, code_hash, to_path, expires_at, spent FROM handoff_codes;
DROP TABLE handoff_codes;
ALTER TABLE handoff_codes_new RENAME TO handoff_codes;
";

/// From the control schema with the signed-in browsers to the one with the
/// messages saved for later (`storage.md` § Control records): a new table,
/// empty, and each draft's row gains the revision of the list, 0 while
/// nothing was saved. The drafts' table is rebuilt, since SQLite adds a
/// column only after its last.
const CONTROL_FROM_SIGNED_IN_BROWSERS: &str = "
CREATE TABLE conversation_drafts_next (
  conversation_id   TEXT PRIMARY KEY COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  revision          INTEGER NOT NULL CHECK (revision >= 1),
  document          TEXT NOT NULL,
  written           INTEGER NOT NULL CHECK (written BETWEEN 1 AND revision),
  replaced_revision INTEGER,
  replaced          TEXT,
  saved_revision    INTEGER NOT NULL CHECK (saved_revision >= 0),
  updated_at        INTEGER NOT NULL,
  CHECK ((replaced IS NULL) = (replaced_revision IS NULL))
) STRICT;
INSERT INTO conversation_drafts_next
  (conversation_id, revision, document, written, replaced_revision, replaced, saved_revision, updated_at)
  SELECT conversation_id, revision, document, written, replaced_revision, replaced, 0, updated_at
  FROM conversation_drafts;
DROP TABLE conversation_drafts;
ALTER TABLE conversation_drafts_next RENAME TO conversation_drafts;
CREATE TABLE saved_messages (
  id              TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  saved_at        INTEGER NOT NULL,
  document        TEXT NOT NULL
) STRICT;
CREATE INDEX saved_messages_of_conversation ON saved_messages (conversation_id, saved_at);
";

/// From the control schema with passkeys to the one with the signed-in
/// browsers and handoff codes (`storage.md` § Control records): each
/// session gets an id, and the browser it signed in from, which no session
/// before kept, reads `Web Browser`; when it signed in and was last active,
/// which none kept either, read its last renewal, 30 days before its
/// expiry. No session was verified. The handoff codes start empty.
const CONTROL_FROM_PASSKEYS: &str = "
CREATE TABLE web_sessions_new (
  token_hash     TEXT PRIMARY KEY,
  id             TEXT NOT NULL UNIQUE,
  user_id        TEXT NOT NULL REFERENCES users (id),
  expires_at     INTEGER NOT NULL,
  name           TEXT NOT NULL,
  created_at     INTEGER NOT NULL,
  last_active_at INTEGER NOT NULL,
  verified_at    INTEGER,
  handoff_id     TEXT
) STRICT;
INSERT INTO web_sessions_new (token_hash, id, user_id, expires_at, name, created_at, last_active_at)
  SELECT token_hash, lower(hex(randomblob(16))), user_id, expires_at, 'Web Browser',
         expires_at - 2592000000, expires_at - 2592000000
  FROM web_sessions;
DROP TABLE web_sessions;
ALTER TABLE web_sessions_new RENAME TO web_sessions;
CREATE INDEX web_sessions_expiry ON web_sessions (expires_at);
CREATE INDEX web_sessions_of_user ON web_sessions (user_id, created_at);
CREATE TABLE handoff_codes (
  user_id    TEXT PRIMARY KEY REFERENCES users (id),
  id         TEXT NOT NULL UNIQUE,
  code_hash  TEXT NOT NULL UNIQUE,
  to_path    TEXT NOT NULL,
  expires_at INTEGER NOT NULL,
  spent      INTEGER NOT NULL CHECK (spent IN (0, 1))
) STRICT;
";

/// From the control schema with the users' credentials to the one with
/// passkeys (`storage.md` § Passwords and credentials at rest): two new
/// tables, empty.
const CONTROL_FROM_CREDENTIALS: &str = "
CREATE TABLE passkeys (
  id           TEXT PRIMARY KEY,
  user_id      TEXT NOT NULL REFERENCES users (id),
  name         TEXT NOT NULL,
  credential   TEXT NOT NULL,
  created_at   INTEGER NOT NULL,
  last_used_at INTEGER
) STRICT;
CREATE INDEX passkeys_of_user ON passkeys (user_id, created_at);
CREATE TABLE passkey_challenges (
  id         TEXT PRIMARY KEY,
  kind       TEXT NOT NULL CHECK (kind IN ('sign_in', 'registration')),
  user_id    TEXT REFERENCES users (id),
  state      TEXT NOT NULL,
  expires_at INTEGER NOT NULL,
  CHECK ((kind = 'registration') = (user_id IS NOT NULL))
) STRICT;
CREATE INDEX passkey_challenges_expiry ON passkey_challenges (expires_at);
";

/// From the control schema with the plugins' secrets to the one with the
/// users' credentials (`credentials.md` § Storage and expiry): two new
/// tables, empty; and a conversation's requests of three kinds, of which
/// every stored one is a permission request. The requests' table is
/// rebuilt, since SQLite cannot make a column nullable in place.
const CONTROL_FROM_PLUGIN_SECRETS: &str = "
CREATE TABLE user_credentials (
  id           TEXT PRIMARY KEY,
  user_id      TEXT NOT NULL REFERENCES users (id),
  name         TEXT NOT NULL,
  note         TEXT NOT NULL,
  fields       TEXT NOT NULL,
  sealed       BLOB NOT NULL,
  keep_until   INTEGER,
  last_used_at INTEGER,
  created_at   INTEGER NOT NULL,
  updated_at   INTEGER NOT NULL,
  UNIQUE (user_id, name)
) STRICT;
CREATE INDEX user_credentials_by_expiry ON user_credentials (keep_until) WHERE keep_until IS NOT NULL;
CREATE TABLE permission_requests_next (
  id                TEXT PRIMARY KEY,
  conversation_id   TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  kind              TEXT NOT NULL CHECK (kind IN ('permission', 'credential', 'allowance')),
  categories        TEXT,
  credential_name   TEXT,
  credential_id     TEXT REFERENCES user_credentials (id) ON DELETE SET NULL,
  ask               TEXT,
  command           TEXT NOT NULL,
  node_id           TEXT NOT NULL,
  agent_number      INTEGER,
  agent_description TEXT,
  created_at        INTEGER NOT NULL,
  decision          TEXT CHECK (decision IN ('allowed', 'denied')),
  decided_at        INTEGER,
  read              INTEGER NOT NULL DEFAULT 0 CHECK (read IN (0, 1)),
  CHECK ((kind = 'permission') = (categories IS NOT NULL)),
  CHECK ((kind = 'permission') = (credential_name IS NULL)),
  CHECK ((kind = 'credential') = (ask IS NOT NULL)),
  CHECK (kind = 'allowance' OR credential_id IS NULL),
  CHECK (kind <> 'credential' OR decision IS NULL),
  CHECK ((agent_number IS NULL) = (agent_description IS NULL)),
  CHECK ((decision IS NULL) = (decided_at IS NULL))
) STRICT;
INSERT INTO permission_requests_next
  (id, conversation_id, kind, categories, command, node_id, agent_number, agent_description,
   created_at, decision, decided_at, read)
  SELECT id, conversation_id, 'permission', categories, command, node_id, agent_number,
         agent_description, created_at, decision, decided_at, read
  FROM permission_requests;
DROP TABLE permission_requests;
ALTER TABLE permission_requests_next RENAME TO permission_requests;
CREATE INDEX permission_requests_of_conversation ON permission_requests (conversation_id, created_at);
CREATE TABLE credential_allowances (
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  credential_id   TEXT NOT NULL REFERENCES user_credentials (id) ON DELETE CASCADE,
  allowed_at      INTEGER NOT NULL,
  PRIMARY KEY (conversation_id, credential_id)
) STRICT;
CREATE INDEX credential_allowances_of_credential ON credential_allowances (credential_id);
";

/// From the control schema with the plugins' secrets, which a server keeps,
/// to the one where a request has a read state (`conversations.md`
/// § Conversations and projects): a request that waits has not been read
/// yet.
const CONTROL_FROM_PLUGIN_SECRETS_UNREAD: &str =
    "ALTER TABLE permission_requests ADD COLUMN read INTEGER NOT NULL DEFAULT 0 CHECK (read IN (0, 1));";

/// From the control schema whose subagent limits keep their numbers to the
/// one with the plugins' secrets and OAuth clients (`plugins.md` § Secrets,
/// § OAuth sign-in): two new tables, empty.
const CONTROL_FROM_KEPT_LIMITS: &str = "
CREATE TABLE plugin_secrets (
  user_id  TEXT NOT NULL REFERENCES users (id),
  plugin   TEXT NOT NULL,
  name     TEXT NOT NULL,
  sealed   BLOB NOT NULL,
  revision INTEGER NOT NULL CHECK (revision >= 1),
  PRIMARY KEY (user_id, plugin, name)
) STRICT, WITHOUT ROWID;
CREATE TABLE plugin_oauth_clients (
  plugin       TEXT NOT NULL,
  issuer       TEXT NOT NULL,
  redirect_uri TEXT NOT NULL,
  client_id    TEXT NOT NULL,
  PRIMARY KEY (plugin, issuer)
) STRICT, WITHOUT ROWID;
";

/// Each conversation's database's. Its history holds the schema of each
/// published release before the one that ships the current schema, and of
/// each development build a server keeps data of (`upgrades.md` § A
/// development build on a server); the one of transcript block format 5 in
/// which a node says which agent it was forked from is the last of those.
/// The schemas of transcript block formats 4 and 5 before it no server
/// keeps, but development databases do, and the server's migrate through
/// them, as they do through the one before a node said which agent it was
/// forked from, through the one of transcript block format 6 before a
/// child's row kept its working directory, through the one with child
/// directories before a node kept its profile's Context, and through the
/// one of format 6 before a shell call marked the paths outside its
/// agent's directory, and the one of format 7 before account rotation, and the one of format 8
/// before the shares' snapshots. The last of those a server keeps is the
/// one `0.1.22-dev.20261015T105826Z` deployed, of format 8, the one
/// `0.1.22-dev.20261015T134426Z` deployed, of format 9, and the one
/// `0.1.22-dev.20261016T043704Z` deployed, whose transcripts are trees of
/// rows. 0.1.22 shipped that one too, before a running command kept its
/// turn.
pub(crate) const CONVERSATION: Schema = Schema {
    sql: CONVERSATION_V1,
    history: &[
        Shipped {
            sql: include_str!("schema/conversation-0.1.11.sql"),
            migration: Migration::Sql(CONVERSATION_FROM_0_1_11),
        },
        Shipped {
            sql: include_str!("schema/conversation-0.1.20.sql"),
            migration: Migration::Sql(CONVERSATION_FROM_0_1_20),
        },
        Shipped {
            sql: include_str!("schema/conversation-0.1.21.sql"),
            migration: Migration::Code(from_0_1_21),
        },
        Shipped {
            sql: include_str!("schema/conversation-0.1.22-dev.20261011T064359Z.sql"),
            migration: Migration::Code(relist_blocks),
        },
        Shipped {
            sql: include_str!("schema/conversation-0.1.22-dev.20261011T123543Z.sql"),
            migration: Migration::Code(from_0_1_22_dev_20261011t123543z),
        },
        Shipped {
            sql: include_str!("schema/conversation-0.1.22-dev.block-format-4.sql"),
            migration: Migration::Code(boundaries_with_kept_messages),
        },
        Shipped {
            sql: include_str!("schema/conversation-0.1.22-dev.block-format-5.sql"),
            migration: Migration::Sql(CONVERSATION_FROM_BLOCK_FORMAT_5),
        },
        Shipped {
            sql: include_str!("schema/conversation-0.1.22-dev.added-instructions.sql"),
            migration: Migration::Sql(CONVERSATION_FROM_ADDED_INSTRUCTIONS),
        },
        Shipped {
            sql: include_str!("schema/conversation-0.1.22-dev.forked-from.sql"),
            migration: Migration::Code(relist_blocks),
        },
        Shipped {
            sql: include_str!("schema/conversation-0.1.22-dev.block-format-6.sql"),
            migration: Migration::Code(nodes_with_their_directories),
        },
        Shipped {
            sql: include_str!("schema/conversation-0.1.22-dev.child-directories.sql"),
            migration: Migration::Sql(CONVERSATION_FROM_CHILD_DIRECTORIES),
        },
        Shipped {
            sql: include_str!("schema/conversation-0.1.22-dev.context-limits.sql"),
            migration: Migration::Code(views_with_outside_paths),
        },
        Shipped {
            sql: include_str!("schema/conversation-0.1.22-dev.block-format-7.sql"),
            migration: Migration::Sql(CONVERSATION_FROM_BLOCK_FORMAT_7),
        },
        Shipped {
            sql: include_str!("schema/conversation-0.1.22-dev.block-format-8.sql"),
            migration: Migration::Sql(CONVERSATION_FROM_BLOCK_FORMAT_8),
        },
        Shipped {
            sql: include_str!("schema/conversation-0.1.22-dev.20261015T105826Z.sql"),
            migration: Migration::Code(calls_with_their_ends),
        },
        Shipped {
            sql: include_str!("schema/conversation-0.1.22-dev.20261015T134426Z.sql"),
            migration: Migration::Code(switches_with_window_names),
        },
        Shipped {
            sql: include_str!("schema/conversation-0.1.22-dev.20261016T043704Z.sql"),
            migration: Migration::Code(commands_with_their_intervals),
        },
        Shipped {
            sql: include_str!("schema/conversation-0.1.22.sql"),
            migration: Migration::Sql(CONVERSATION_FROM_0_1_22),
        },
    ],
};

/// From 0.1.22's conversation schema to the current one: a running
/// command keeps the turn
/// whose shell call started it, which a command's call's usage ledger row
/// names (`usage-and-quota.md` § Usage ledger); one recorded before keeps
/// none. A command whose start waits for its Host keeps what it starts
/// (`sessions-and-targets.md` § Host operations); every one recorded before
/// was sent.
const CONVERSATION_FROM_0_1_22: &str = "
ALTER TABLE running_commands ADD COLUMN turn_id TEXT;
ALTER TABLE running_commands ADD COLUMN unsent TEXT;
";

/// From 0.1.22-dev.20261015T134426Z's control schema, which a server keeps
/// data of, to the one whose quota snapshots (format 2) give each window a
/// kind, which with its period and its scope the page names it from in the
/// user's language (`usage-and-quota.md` § Window names), instead of the
/// English name format 1 held. Each window's kind is the one its old name
/// says, read by its vendor's old namer ([`old_window_name`]); a name that
/// namer did not make, or a vendor with none, fails the migration. Each
/// snapshot is decoded and checked as format 2 before it is written back.
fn snapshots_with_window_kinds(transaction: &Transaction<'_>, _: &dyn ReadBlob) -> rusqlite::Result<()> {
    // The shares made before a share held only what was said, each of
    // which held every block and its subagents, are removed, and their
    // links answer as ones that never existed (`sharing.md` § Storage and
    // routes); each conversation's database drops their lists
    // ([`switches_with_window_names`]).
    transaction.execute("DELETE FROM shares", [])?;
    let rows = transaction
        .prepare(
            "SELECT c.provider_id, c.id, c.quota, p.provider_type FROM provider_credentials c
               JOIN providers p ON p.id = c.provider_id WHERE c.quota IS NOT NULL",
        )?
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut update = transaction.prepare("UPDATE provider_credentials SET quota = ?3 WHERE provider_id = ?1 AND id = ?2")?;
    for (provider, id, text, provider_type) in rows {
        let namer = match provider_type.as_str() {
            "codex" => OldNamer::Codex,
            "claude-code" => OldNamer::ClaudeCode,
            "grok-build" => OldNamer::GrokBuild,
            other => return Err(corrupt_text(&format!("a {other} account kept a quota snapshot, which no namer made"))),
        };
        let mut snapshot: serde_json::Value = serde_json::from_str(&text).map_err(corrupt)?;
        let windows = snapshot
            .get_mut("windows")
            .and_then(serde_json::Value::as_array_mut)
            .ok_or_else(|| corrupt_text("a stored quota snapshot has no windows"))?;
        for window in windows {
            let fields = window
                .as_object_mut()
                .ok_or_else(|| corrupt_text("a stored quota window is not an object"))?;
            let label = match fields.remove("label") {
                Some(serde_json::Value::String(label)) => label,
                _ => return Err(corrupt_text("a stored quota window of format 1 has no name")),
            };
            let kind = old_window_name(&label, Some(namer))?.kind;
            fields.insert("kind".to_owned(), serde_json::to_value(kind).map_err(corrupt)?);
        }
        let snapshot: demi_shared_types::QuotaSnapshot = demi_shared_types::decode_value(snapshot).map_err(corrupt)?;
        update.execute(rusqlite::params![provider, id, to_json(&snapshot)])?;
    }
    Ok(())
}

/// From the conversation schema that `0.1.22-dev.20261015T134426Z`
/// deployed, of transcript block format 9, to format 10, in which an
/// `account_switch` block holds what names the window that was used up, its
/// kind, period and scope, instead of its English name, so the line reads in
/// the viewer's language (`usage-and-quota.md` § Window names). The old
/// name says them, read by every vendor's old namer, since the block does
/// not name the vendor ([`old_window_name`]); a name none made fails the
/// migration. Each block is decoded and checked as format 10 before it is
/// written back. Then the shares become lists of what was said
/// ([`shares_as_lists`]). Last, each transcript becomes a tree of rows of
/// one path ([`ONE_PATH`]).
fn switches_with_window_names(transaction: &Transaction<'_>, _: &dyn ReadBlob) -> rusqlite::Result<()> {
    let switches = "json_extract(block, '$.type') = 'account_switch'";
    let rows = transaction
        .prepare(&format!("SELECT node_id, idx, block FROM blocks WHERE {switches}"))?
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut update = transaction.prepare("UPDATE blocks SET block = ?3 WHERE node_id = ?1 AND idx = ?2")?;
    for (node, index, text) in rows {
        update.execute(rusqlite::params![node, index, named_switch(&text)?])?;
    }
    shares_as_lists(transaction)?;
    transaction.execute_batch(ONE_PATH)
}

/// The least and the greatest interval a command reported at when
/// `0.1.22-dev.20261016T043704Z` was deployed, in milliseconds, which bound
/// the interval its calls asked for as they bounded it then.
const INTERVAL_BOUNDS_MS: (i64, i64) = (15_000, 600_000);

/// From the conversation schema that `0.1.22-dev.20261016T043704Z`
/// deployed to the one in which each running command's record keeps the
/// interval its call asked it to report at, read from the call's input in the transcript as that build
/// bounded it, or none for a call that asked for none (`shell-tool.md`
/// § Command reports): a command whose call a restart interrupted reports
/// as its call asked. A record whose call the transcript lacks, or whose
/// call's input names no interval, fails the migration.
fn commands_with_their_intervals(transaction: &Transaction<'_>, _: &dyn ReadBlob) -> rusqlite::Result<()> {
    transaction.execute_batch("ALTER TABLE running_commands ADD COLUMN interval_ms INTEGER CHECK (interval_ms >= 1);")?;
    let rows = transaction
        .prepare("SELECT command_id, node_id, tool_use_id FROM running_commands")?
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut call = transaction.prepare(
        "SELECT json_extract(block, '$.input') FROM blocks
         WHERE node_id = ?1 AND json_extract(block, '$.type') = 'tool_call'
           AND json_extract(block, '$.toolUseId') = ?2
         LIMIT 1",
    )?;
    let mut update = transaction.prepare("UPDATE running_commands SET interval_ms = ?2 WHERE command_id = ?1")?;
    for (command, node, tool_use_id) in rows {
        let input: Option<String> = call
            .query_row(rusqlite::params![node, tool_use_id], |row| row.get(0))
            .optional()?;
        let Some(input) = input else {
            let reason = format!("the transcript lacks the call that started command {command}");
            return Err(corrupt(std::io::Error::other(reason)));
        };
        let input: serde_json::Value = serde_json::from_str(&input).map_err(corrupt)?;
        let interval_ms = match input.get("intervalMs") {
            Some(serde_json::Value::Null) => None,
            Some(asked) => {
                let Some(asked) = asked.as_i64() else {
                    let reason = format!("the call that started command {command} asked for no whole milliseconds");
                    return Err(corrupt(std::io::Error::other(reason)));
                };
                let (floor, cap) = INTERVAL_BOUNDS_MS;
                Some(asked.clamp(floor, cap))
            }
            None => {
                let reason = format!("the call that started command {command} names no interval");
                return Err(corrupt(std::io::Error::other(reason)));
            }
        };
        update.execute(rusqlite::params![command, interval_ms])?;
    }
    Ok(())
}

/// From a transcript whose rows were its blocks' positions to a tree of
/// rows (`message-editing.md` § Versions): a node's rows below its block
/// count, which are all it kept, are one path, each row following the one
/// before it and its last row the leaf; its runs are that one run.
const ONE_PATH: &str = "
ALTER TABLE blocks ADD COLUMN follows INTEGER CHECK (follows < idx);
ALTER TABLE blocks ADD COLUMN replaces INTEGER CHECK (replaces < idx);
UPDATE blocks SET follows = idx - 1 WHERE idx > 0;
CREATE INDEX blocks_by_follows ON blocks (node_id, follows);
CREATE INDEX blocks_branching ON blocks (node_id, follows) WHERE follows IS NOT idx - 1;
CREATE INDEX blocks_carrying ON blocks (node_id, replaces) WHERE replaces IS NOT NULL;
CREATE TABLE path_runs (
  node_id TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
  start   INTEGER NOT NULL CHECK (start >= 0),
  first   INTEGER NOT NULL CHECK (first >= 0),
  length  INTEGER NOT NULL CHECK (length >= 1),
  PRIMARY KEY (node_id, start)
) STRICT, WITHOUT ROWID;
INSERT INTO path_runs (node_id, start, first, length)
  SELECT id, 0, 0, block_count FROM nodes WHERE block_count > 0;
ALTER TABLE nodes ADD COLUMN leaf INTEGER CHECK (leaf >= 0);
ALTER TABLE nodes ADD COLUMN cut INTEGER CHECK (cut >= 0);
ALTER TABLE nodes ADD COLUMN started_by TEXT;
UPDATE nodes SET leaf = block_count - 1 WHERE block_count > 0;
ALTER TABLE nodes DROP COLUMN block_count;
";

/// The stored `account_switch` block `text` of format 9 as format 10 holds
/// it: its window's name, a string or null, as what names it. A window of
/// another type is corrupt data, which fails the migration.
fn named_switch(text: &str) -> rusqlite::Result<String> {
    let mut block: serde_json::Value = serde_json::from_str(text).map_err(corrupt)?;
    let fields = block
        .as_object_mut()
        .ok_or_else(|| corrupt_text("a stored block is not an object"))?;
    let window = match fields.remove("window") {
        Some(serde_json::Value::String(label)) => serde_json::to_value(old_window_name(&label, None)?).map_err(corrupt)?,
        Some(serde_json::Value::Null) => serde_json::Value::Null,
        _ => return Err(corrupt_text("a stored account switch of format 9 names its window by no string")),
    };
    fields.insert("window".to_owned(), window);
    let block: demi_shared_types::Block = demi_shared_types::decode_value(block).map_err(corrupt)?;
    Ok(to_json(&block))
}

/// The vendor whose old namer made a window's English name, when the
/// stored row says it: a snapshot's provider type. An account switch's
/// block does not say it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum OldNamer {
    Codex,
    ClaudeCode,
    GrokBuild,
}

/// What names a window that `namer`'s old code named `label` in English,
/// before a window had a kind, or with no namer any of theirs; a label none
/// of them made is corrupt data, which fails the migration.
/// - Codex: `Primary` or `Secondary` for an unknown length, `Weekly`,
///   `Daily`, or the length in the largest unit that divides it, `3-day`,
///   `5-hour`, `1.5-minute`.
/// - Claude Code: `5h session`, `7d all models`, `7d <model>`, `5h <model>`,
///   `Other limit`, and the model's name alone for a limit on one model
///   whose period it could not tell.
/// - Grok Build: `Weekly credits`, `Monthly credits`, `On-demand cap`, and
///   `Requests (short window)` and `Tokens (short window)`.
///
/// Without a namer, a label of the Codex length's shape must be one, and
/// only one that is none of the other forms is a model's name.
fn old_window_name(label: &str, namer: Option<OldNamer>) -> rusqlite::Result<demi_shared_types::QuotaWindowName> {
    use demi_shared_types::{QuotaKind, QuotaScope, QuotaWindowName};

    const HOUR: u64 = 3600;
    const DAY: u64 = 24 * HOUR;
    const WEEK: u64 = 7 * DAY;
    let name = |kind, period_seconds, model: Option<&str>| QuotaWindowName {
        kind,
        period_seconds,
        scope: model.map(|model| QuotaScope {
            kind: "model".to_owned(),
            label: Some(model.to_owned()),
        }),
    };
    let limit = |period, model| Ok(name(QuotaKind::Limit, period, model));
    let unknown = || Err(corrupt_text(&format!("no namer made the quota window name {label:?}")));
    let by = |vendor: OldNamer| namer.is_none_or(|namer| namer == vendor);
    if by(OldNamer::Codex) {
        match label {
            "Primary" | "Secondary" => return limit(None, None),
            "Weekly" => return limit(Some(WEEK), None),
            "Daily" => return limit(Some(DAY), None),
            _ => {}
        }
        let length = label.rsplit_once('-').and_then(|(count, unit)| {
            let unit = match unit {
                "week" => WEEK,
                "day" => DAY,
                "hour" => HOUR,
                "minute" => 60,
                _ => return None,
            };
            Some((count, unit))
        });
        if let Some((count, unit)) = length {
            return match codex_length(count, unit) {
                Some(period) => limit(period, None),
                None => unknown(),
            };
        }
    }
    if by(OldNamer::GrokBuild) {
        match label {
            "Weekly credits" => return Ok(name(QuotaKind::Credits, Some(WEEK), None)),
            "Monthly credits" => return Ok(name(QuotaKind::Credits, None, None)),
            "On-demand cap" => return Ok(name(QuotaKind::Cap, None, None)),
            "Requests (short window)" => return Ok(name(QuotaKind::Requests, None, None)),
            "Tokens (short window)" => return Ok(name(QuotaKind::Tokens, None, None)),
            _ => {}
        }
    }
    if by(OldNamer::ClaudeCode) {
        match label {
            "5h session" => return limit(Some(5 * HOUR), None),
            "7d all models" | "7d" => return limit(Some(WEEK), None),
            "5h" => return limit(Some(5 * HOUR), None),
            "Other limit" => return limit(None, None),
            "" => return unknown(),
            _ => {}
        }
        if let Some(model) = label.strip_prefix("7d ") {
            return limit(Some(WEEK), Some(model));
        }
        if let Some(model) = label.strip_prefix("5h ") {
            return limit(Some(5 * HOUR), Some(model));
        }
        return limit(None, Some(label));
    }
    unknown()
}

/// The length Codex's old namer wrote as `count` of a unit of `unit`
/// seconds, `1.5` of a minute: its seconds, or none, as Codex stored it,
/// when they are not whole; `None` when `count` is no length it wrote,
/// not a positive number in Rust's own form or too long to keep.
fn codex_length(text: &str, unit: u64) -> Option<Option<u64>> {
    let count: f64 = text.parse().ok().filter(|count: &f64| count.is_finite() && *count > 0.0)?;
    // The namer wrote the quotient in Rust's shortest form, so a count in
    // any other form, `1.50` or `1e3`, is not one it wrote.
    if count.to_string() != text {
        return None;
    }
    let seconds = count * unit as f64;
    if seconds > demi_shared_types::MAX_SAFE_INTEGER as f64 {
        return None;
    }
    if seconds.fract() != 0.0 || seconds < 1.0 {
        return Some(None);
    }
    // Whole and below 2^53, so the conversion is exact.
    Some(Some(seconds as u64))
}

/// From the conversation schema that `0.1.22-dev.20261015T105826Z`
/// deployed, of transcript block format 8, to format 9, in which a call's
/// block records when it returned, `endedAt`, which a run's time adds up
/// (`rendering.md` § Work groups). A stored call kept no record of it: one
/// that returned ends the time its shell view says its command ran until
/// then, `runningMs`, after it started, and a call of another tool where
/// it started; a call still executing has not returned. Each block is
/// decoded and checked as format 9 before it is written back; the shares'
/// snapshots are removed later ([`shares_as_lists`]).
fn calls_with_their_ends(transaction: &Transaction<'_>, _: &dyn ReadBlob) -> rusqlite::Result<()> {
    let calls = "json_extract(block, '$.type') = 'tool_call'";
    let rows = transaction
        .prepare(&format!("SELECT node_id, idx, block FROM blocks WHERE {calls}"))?
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut update = transaction.prepare("UPDATE blocks SET block = ?3 WHERE node_id = ?1 AND idx = ?2")?;
    for (node, index, text) in rows {
        if let Some(block) = ended_call(&text)? {
            update.execute(rusqlite::params![node, index, block])?;
        }
    }
    Ok(())
}

/// From the shares whose snapshots copied every block of a conversation
/// and its subagents to shares that list what was said (`sharing.md`
/// § Storage and routes): the shares made before are removed, as the
/// control database removes their records
/// ([`snapshots_with_window_kinds`]), and the tables of the lists start
/// empty. Each blob a block names is listed in `block_blobs` and each
/// attachment a text names in `block_attachments`, each block decoded once.
fn shares_as_lists(transaction: &Transaction<'_>) -> rusqlite::Result<()> {
    transaction.execute_batch(SHARES_AS_LISTS)?;
    let mut blocks = transaction.prepare("SELECT node_id, idx, block FROM blocks")?;
    let mut rows = blocks.query([])?;
    while let Some(row) = rows.next()? {
        let block: demi_shared_types::Block = serde_json::from_str(&row.get::<_, String>(2)?).map_err(corrupt)?;
        crate::tree::name_block(transaction, &row.get::<_, String>(0)?, row.get(1)?, &block)?;
    }
    Ok(())
}

/// The tables [`shares_as_lists`] adds and replaces, without the shares
/// made before.
const SHARES_AS_LISTS: &str = "
DROP TABLE share_blocks;
DROP TABLE share_agents;
DELETE FROM share_attachments;
CREATE TABLE block_blobs (
  node_id TEXT NOT NULL,
  idx     INTEGER NOT NULL,
  blob    TEXT NOT NULL,
  PRIMARY KEY (node_id, idx, blob),
  FOREIGN KEY (node_id, idx) REFERENCES blocks (node_id, idx) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;
CREATE INDEX block_blobs_by_blob ON block_blobs (blob);
CREATE TABLE block_attachments (
  node_id    TEXT NOT NULL,
  idx        INTEGER NOT NULL,
  attachment INTEGER NOT NULL CHECK (attachment >= 1),
  PRIMARY KEY (node_id, idx, attachment),
  FOREIGN KEY (node_id, idx) REFERENCES blocks (node_id, idx) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;
CREATE TABLE share_entries (
  token    TEXT NOT NULL,
  position INTEGER NOT NULL CHECK (position >= 0),
  node_id  TEXT,
  idx      INTEGER CHECK (idx >= 0),
  steps    INTEGER CHECK (steps >= 1),
  CHECK ((node_id IS NULL) = (idx IS NULL)),
  CHECK (steps IS NULL OR node_id IS NULL),
  PRIMARY KEY (token, position)
) STRICT;
CREATE INDEX share_entries_by_row ON share_entries (node_id, idx, token);
CREATE TABLE share_blocks (
  token TEXT NOT NULL,
  idx   INTEGER NOT NULL CHECK (idx >= 0),
  block TEXT NOT NULL,
  PRIMARY KEY (token, idx)
) STRICT;
CREATE TABLE share_blobs (
  token TEXT NOT NULL,
  blob  TEXT NOT NULL,
  PRIMARY KEY (token, blob)
) STRICT, WITHOUT ROWID;
";

/// The stored call `text` as format 9 holds it, decoded and encoded again;
/// none when it holds its end already, as one an earlier migration wrote.
fn ended_call(text: &str) -> rusqlite::Result<Option<String>> {
    let mut block: serde_json::Value = serde_json::from_str(text).map_err(corrupt)?;
    if !with_its_end(&mut block)? {
        return Ok(None);
    }
    let block: demi_shared_types::Block = demi_shared_types::decode_value(block).map_err(corrupt)?;
    Ok(Some(to_json(&block)))
}

/// A stored call of transcript block format 8 or before as format 9 holds
/// it, with when it returned (`calls_with_their_ends`); true when it
/// changed. A start or a time that is not what its format says is corrupt
/// data, which fails the migration. The migrations before that format's
/// decode the blocks they relist as the current format, so they read a
/// call through this too.
fn with_its_end(block: &mut serde_json::Value) -> rusqlite::Result<bool> {
    let Some(fields) = block.as_object_mut() else {
        return Ok(false);
    };
    if fields.get("type").and_then(serde_json::Value::as_str) != Some("tool_call") || fields.contains_key("endedAt") {
        return Ok(false);
    }
    let ended_at = if fields.get("status").and_then(serde_json::Value::as_str) == Some("executing") {
        serde_json::Value::Null
    } else {
        let field = |value: Option<&serde_json::Value>| value.cloned().unwrap_or_default();
        let started: Timestamp = serde_json::from_value(field(fields.get("createdAt"))).map_err(corrupt)?;
        let shell = fields
            .get("view")
            .filter(|view| view.get("kind").and_then(serde_json::Value::as_str) == Some("shell"));
        let ran = match shell {
            Some(view) => serde_json::from_value::<i64>(field(view.get("runningMs"))).map_err(corrupt)?,
            None => 0,
        };
        let ended = Timestamp::from_millisecond(started.as_millisecond() + ran).map_err(corrupt)?;
        serde_json::to_value(ended).map_err(corrupt)?
    };
    fields.insert("endedAt".to_owned(), ended_at);
    Ok(true)
}

/// From the conversation schema of transcript block format 8 to the one
/// that keeps the shares' snapshots (`sharing.md` § Storage and routes). A
/// database of the earlier schema holds none.
const CONVERSATION_FROM_BLOCK_FORMAT_8: &str = "
CREATE TABLE share_blocks (
  token TEXT NOT NULL,
  agent INTEGER NOT NULL CHECK (agent >= 0),
  idx   INTEGER NOT NULL CHECK (idx >= 0),
  block TEXT NOT NULL,
  PRIMARY KEY (token, agent, idx)
) STRICT;
CREATE INDEX share_blocks_by_type ON share_blocks (token, agent, json_extract(block, '$.type'), idx);
CREATE TABLE share_agents (
  token       TEXT NOT NULL,
  number      INTEGER NOT NULL CHECK (number >= 1),
  description TEXT NOT NULL,
  profile     TEXT,
  PRIMARY KEY (token, number)
) STRICT;
CREATE TABLE share_attachments (
  token      TEXT NOT NULL,
  number     INTEGER NOT NULL CHECK (number >= 1),
  name       TEXT NOT NULL,
  media_type TEXT NOT NULL,
  size       INTEGER NOT NULL CHECK (size >= 0),
  width      INTEGER CHECK (width >= 1),
  height     INTEGER CHECK (height >= 1),
  blob       TEXT NOT NULL,
  CHECK ((width IS NULL) = (height IS NULL)),
  PRIMARY KEY (token, number)
) STRICT;
";

/// From the conversation schema of transcript block format 7 and checkpoint
/// state format 3 to block format 8, which adds the `account_switch` block,
/// and state format 4, which keeps an agent's account (`providers.md`
/// § Account rotation). Every stored block and state is one of the new
/// formats as it is: none switched accounts, and none has taken an account
/// of its own yet.
const CONVERSATION_FROM_BLOCK_FORMAT_7: &str = "";

/// From the conversation schema of transcript block format 6 to format 7,
/// in which a shell call's own files mark the paths outside its agent's
/// working directory `outside` in place of the paths in a temporary
/// directory `temporary`, and its renames and removals mark none
/// (`edit-tracking.md` § Within the working directory). This database does
/// not know the root's directory, so a file marked temporary is marked
/// outside, and the stored run and request lists, which already left those
/// out, stay as they are.
fn views_with_outside_paths(transaction: &Transaction<'_>, _: &dyn ReadBlob) -> rusqlite::Result<()> {
    let rows = transaction
        .prepare("SELECT node_id, idx, block FROM blocks WHERE json_extract(block, '$.view.commandId') IS NOT NULL")?
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut update = transaction.prepare("UPDATE blocks SET block = ?3 WHERE node_id = ?1 AND idx = ?2")?;
    for (node, index, text) in rows {
        let mut block: serde_json::Value = serde_json::from_str(&text).map_err(corrupt)?;
        if with_outside_paths(&mut block).map_err(corrupt)? {
            with_its_end(&mut block)?;
            let block: demi_shared_types::Block = demi_shared_types::decode_value(block).map_err(corrupt)?;
            update.execute(rusqlite::params![node, index, super::columns::to_json(&block)])?;
        }
    }
    Ok(())
}

/// A stored shell view of transcript block format 6 or before as format 7
/// holds it: each own file's `temporary` becomes `outside`, and its renames
/// and removals lose theirs; true when it changed. A flag that is not a
/// boolean is corrupt data, which fails the migration. The migrations
/// before that format's decode the blocks they relist as the current
/// format, so they read a view through this too.
fn with_outside_paths(block: &mut serde_json::Value) -> Result<bool, serde_json::Error> {
    let Some(view) = block.get_mut("view").and_then(serde_json::Value::as_object_mut) else {
        return Ok(false);
    };
    let mut changed = false;
    if let Some(files) = view.get_mut("files").and_then(serde_json::Value::as_array_mut) {
        for file in files.iter_mut().filter_map(serde_json::Value::as_object_mut) {
            if let Some(temporary) = file.remove("temporary") {
                changed = true;
                if serde_json::from_value::<bool>(temporary)? {
                    file.insert("outside".to_owned(), serde_json::Value::Bool(true));
                }
            }
        }
    }
    if let Some(changes) = view.get_mut("pathChanges").and_then(serde_json::Value::as_array_mut) {
        for change in changes.iter_mut().filter_map(serde_json::Value::as_object_mut) {
            if let Some(temporary) = change.remove("temporary") {
                changed = true;
                serde_json::from_value::<bool>(temporary)?;
            }
        }
    }
    Ok(changed)
}

/// From the conversation schema in which a child's row keeps its working
/// directory to the one in which it keeps the Context its profile chose
/// (`subagents.md` § Profiles): every stored node follows the user's limit,
/// as it did.
const CONVERSATION_FROM_CHILD_DIRECTORIES: &str =
    "ALTER TABLE nodes ADD COLUMN context_limit INTEGER CHECK (context_limit >= 1);";

/// From the conversation schema of transcript block format 6, whose
/// checkpoint state (format 2) kept the node's working directory, to the
/// one in which a child's row keeps it with where the conversation worked
/// when it was set (`subagents.md` § Working directory), checkpoint state
/// format 3: each child's directory moves from its state into its row, and
/// the root's goes, since the root works in the conversation's directory.
/// Where the conversation worked then is not in this database, and no
/// device is recorded: a child of that schema worked in the conversation's
/// directory, and it does again, since no device the conversation runs on
/// now is none. Format 6, which no server has kept, now also lets a
/// `context` block name a directory, which no stored block does, so the
/// blocks are as they were. Each state is decoded and checked as format 3
/// before it is written back.
fn nodes_with_their_directories(transaction: &Transaction<'_>, _: &dyn ReadBlob) -> rusqlite::Result<()> {
    transaction.execute_batch(
        "ALTER TABLE nodes ADD COLUMN cwd TEXT CHECK (cwd <> '');
         ALTER TABLE nodes ADD COLUMN cwd_device TEXT;
         ALTER TABLE nodes ADD COLUMN cwd_under TEXT CHECK (cwd_under <> '');",
    )?;
    let states = transaction
        .prepare("SELECT id, parent_id IS NOT NULL, state FROM nodes")?
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?, row.get::<_, String>(2)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut update =
        transaction.prepare("UPDATE nodes SET state = ?2, cwd = ?3, cwd_under = ?3 WHERE id = ?1")?;
    for (id, child, state) in states {
        let mut fields: serde_json::Map<String, serde_json::Value> =
            decode("nodes", "state", serde_json::from_str(&state)).map_err(corrupt)?;
        // The root's goes whatever it was; a child's is its row's now.
        let cwd = match (fields.remove("cwd"), child) {
            (_, false) => None,
            (Some(serde_json::Value::String(cwd)), true) => Some(cwd),
            (_, true) => {
                let reason = format!("the state of the child {id} names no working directory");
                return Err(corrupt(std::io::Error::other(reason)));
            }
        };
        let state: CheckpointState = json("nodes", "state", &to_json(&fields)).map_err(corrupt)?;
        update.execute(rusqlite::params![id, to_json(&state), cwd])?;
    }
    Ok(())
}

/// From the conversation schema in which a node's instructions are the
/// ones its profile adds, before a node said which agent it was forked
/// from (`subagents.md` § Fork): no node of it was forked.
const CONVERSATION_FROM_ADDED_INSTRUCTIONS: &str =
    "ALTER TABLE nodes ADD COLUMN forked_from INTEGER CHECK (forked_from >= 1);";

/// From the conversation schema of transcript block format 5 before a
/// node's instructions were the ones its profile adds (`subagents.md`
/// § Persistence): only the column's comment changed, and what a node
/// stored there before reads as what its profile added.
const CONVERSATION_FROM_BLOCK_FORMAT_5: &str = "";

/// From the conversation schema of transcript block format 4 to format 5,
/// where a `compaction_boundary` holds the user's messages its pass kept
/// (`compaction.md` § One pass): no pass before kept any, so each stored
/// boundary keeps none. The tables are as they were.
fn boundaries_with_kept_messages(transaction: &Transaction<'_>, _: &dyn ReadBlob) -> rusqlite::Result<()> {
    let rows = transaction
        .prepare("SELECT node_id, idx, block FROM blocks WHERE json_extract(block, '$.type') = 'compaction_boundary'")?
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut update = transaction.prepare("UPDATE blocks SET block = ?3 WHERE node_id = ?1 AND idx = ?2")?;
    for (node, index, text) in rows {
        let mut block: serde_json::Value = serde_json::from_str(&text).map_err(corrupt)?;
        if with_no_kept_messages(&mut block) {
            let block: demi_shared_types::Block = demi_shared_types::decode_value(block).map_err(corrupt)?;
            update.execute(rusqlite::params![node, index, super::columns::to_json(&block)])?;
        }
    }
    Ok(())
}

/// A stored `compaction_boundary` of transcript block format 4 or before
/// as format 5 holds it, with no kept messages; true when it changed. The
/// migrations before that format's decode the blocks they relist as the
/// current format, so they read a boundary through this too.
fn with_no_kept_messages(block: &mut serde_json::Value) -> bool {
    let Some(fields) = block.as_object_mut() else {
        return false;
    };
    if fields.get("type").and_then(serde_json::Value::as_str) != Some("compaction_boundary")
        || fields.contains_key("kept")
    {
        return false;
    }
    fields.insert("kept".to_owned(), serde_json::Value::Array(Vec::new()));
    true
}

/// From the conversation schema of the development build
/// 0.1.22-dev.20261011T123543Z, which a server keeps data of
/// (`upgrades.md` § A development build on a server): a command's records
/// keep the title of the call that started it, which its reports name
/// (`shell-tool.md` § Command reports), and the blocks move from transcript
/// block format 3 to format 4 ([`relist_blocks`]). No record that build
/// wrote kept a title, and none is guessed: theirs stay empty, and their
/// reports name the command by its number.
fn from_0_1_22_dev_20261011t123543z(
    transaction: &Transaction<'_>,
    blobs: &dyn ReadBlob,
) -> rusqlite::Result<()> {
    transaction.execute_batch(
        "
ALTER TABLE command_outputs ADD COLUMN title TEXT NOT NULL DEFAULT '';
ALTER TABLE running_commands ADD COLUMN title TEXT NOT NULL DEFAULT '';
",
    )?;
    relist_blocks(transaction, blobs)
}

/// From the control schema of the development build
/// 0.1.22-dev.20261011T123543Z, which a server keeps data of: each user's
/// subagent settings hold the two limits (`subagents.md` § Limits), which a
/// user who turned the Subagents switch has at their defaults, as a user
/// with no row has.
const CONTROL_FROM_0_1_22_DEV_20261011T123543Z: &str = "
ALTER TABLE user_subagents ADD COLUMN max_live INTEGER DEFAULT 10 CHECK (max_live >= 1);
ALTER TABLE user_subagents ADD COLUMN max_depth INTEGER DEFAULT 1 CHECK (max_depth >= 1);
";

/// From the control schema with the subagent limits: Detect Title is gone
/// (`conversations.md` § Conversation titles), and with it the counts of the
/// messages the user sent and the last generated title had seen, which
/// only it read.
const CONTROL_FROM_SUBAGENT_LIMITS: &str = "
ALTER TABLE conversations DROP COLUMN user_messages;
ALTER TABLE conversations DROP COLUMN titled_messages;
";

/// From the control schema without Detect Title: each subagent limit keeps
/// its number while it is lifted (`subagents.md` § Limits), so a limit
/// stored as Unlimited, a null, takes its default number and stays
/// Unlimited. The table is rebuilt, since SQLite cannot make a column NOT
/// NULL in place.
const CONTROL_FROM_NO_DETECT_TITLE: &str = "
CREATE TABLE user_subagents_kept (
  user_id             TEXT PRIMARY KEY REFERENCES users (id),
  enabled             INTEGER NOT NULL CHECK (enabled IN (0, 1)),
  max_live            INTEGER NOT NULL CHECK (max_live >= 1),
  max_live_unlimited  INTEGER NOT NULL CHECK (max_live_unlimited IN (0, 1)),
  max_depth           INTEGER NOT NULL CHECK (max_depth >= 1),
  max_depth_unlimited INTEGER NOT NULL CHECK (max_depth_unlimited IN (0, 1))
) STRICT;
INSERT INTO user_subagents_kept
  SELECT user_id, enabled, coalesce(max_live, 10), max_live IS NULL,
         coalesce(max_depth, 1), max_depth IS NULL
  FROM user_subagents;
DROP TABLE user_subagents;
ALTER TABLE user_subagents_kept RENAME TO user_subagents;
";

/// From 0.1.11's control schema: the retention pass, which read when each
/// conversation was last live, is gone, and the column with it.
const CONTROL_FROM_0_1_11: &str = "
ALTER TABLE conversations DROP COLUMN live_at;
";

/// From 0.1.13's control schema: Host expose is gone, and its records with
/// it; a device keeps the operating system its runner reports, which a
/// device migrated from 0.1.13 learns at its runner's next hello.
const CONTROL_FROM_0_1_13: &str = "
DROP TABLE exposes;
ALTER TABLE devices ADD COLUMN os TEXT;
";

/// From 0.1.15's control schema: a conversation counts the changes of its
/// attached hosts, from 0.
const CONTROL_FROM_0_1_15: &str = "
ALTER TABLE conversations ADD COLUMN hosts_revision INTEGER NOT NULL DEFAULT 0 CHECK (hosts_revision >= 0);
";

/// From 0.1.16's control schema: a device keeps the runner release its
/// runner reports, which a device migrated from 0.1.16 learns at its
/// runner's next hello; and a conversation's deletion is recorded as
/// pending until its last step (`storage.md` § Deleting a conversation).
const CONTROL_FROM_0_1_16: &str = "
ALTER TABLE devices ADD COLUMN runner_version TEXT;
CREATE TABLE conversation_deletions (
  id         TEXT PRIMARY KEY COLLATE NOCASE,
  user_id    TEXT NOT NULL REFERENCES users (id),
  deleted_at INTEGER NOT NULL
) STRICT;
";

/// From 0.1.17's control schema: a device records whether the backend's
/// shutdown ended its last connection, which no device migrated from 0.1.17
/// did.
const CONTROL_FROM_0_1_17: &str = "
ALTER TABLE devices ADD COLUMN ended_by_shutdown INTEGER NOT NULL DEFAULT 0 CHECK (ended_by_shutdown IN (0, 1));
";

/// From 0.1.18's control schema: the deployment keeps its namespace at the
/// preview domain, which a deployment migrated from 0.1.18 registers at its
/// next start.
const CONTROL_FROM_0_1_18: &str = "
CREATE TABLE preview_namespace (
  id         INTEGER PRIMARY KEY CHECK (id = 1),
  namespace  TEXT NOT NULL,
  secret     BLOB NOT NULL,
  expires_at INTEGER NOT NULL,
  origins    TEXT NOT NULL
) STRICT;
";

/// From 0.1.20's control schema: a device keeps how pages reach it, which
/// is Automatic for every device migrated from 0.1.20, as for a new one
/// (`direct-channel.md` § Choosing the path).
const CONTROL_FROM_0_1_20: &str = "
ALTER TABLE devices ADD COLUMN route TEXT NOT NULL DEFAULT 'automatic' CHECK (route IN ('automatic', 'direct', 'server'));
";

/// From 0.1.21's control schema: a conversation keeps the move an agent
/// asked for and an attached host the detach it waits with, none for every
/// conversation 0.1.21 left; a permission request names its categories, the
/// one category of each request 0.1.21 stored. SQLite cannot drop a column
/// an index names, so the requests' table is made anew, as for 0.1.11's
/// conversations. The web preview is gone, and the deployment's namespace
/// at the preview domain with it. Users keep personal instructions, which
/// no user migrated from 0.1.21 has. Yield wakeups are gone, and the index
/// of when each conversation's earliest one is due with them. Running jobs
/// are recorded by device, none of which 0.1.21 kept, since its jobs ended
/// with their connections, and a device keeps its runner's instance, which
/// no runner of 0.1.21 named.
const CONTROL_FROM_0_1_21: &str = "
DROP INDEX conversations_wakeup;
ALTER TABLE conversations DROP COLUMN wakeup_at;
CREATE TABLE user_instructions (
  user_id TEXT PRIMARY KEY REFERENCES users (id),
  text    TEXT NOT NULL
) STRICT;
ALTER TABLE conversations ADD COLUMN pending_id TEXT;
ALTER TABLE conversations ADD COLUMN pending_kind TEXT CHECK (pending_kind IN ('cloud', 'device', 'workspace'));
ALTER TABLE conversations ADD COLUMN pending_device_id TEXT;
ALTER TABLE conversations ADD COLUMN pending_path TEXT;
ALTER TABLE conversations ADD COLUMN pending_workspace_id TEXT CHECK (
    (pending_kind IS NULL
      AND pending_id IS NULL AND pending_device_id IS NULL AND pending_path IS NULL
      AND pending_workspace_id IS NULL)
    OR (pending_kind = 'cloud' AND pending_id IS NOT NULL
      AND pending_device_id IS NULL AND pending_workspace_id IS NULL)
    OR (pending_kind = 'device' AND pending_id IS NOT NULL
      AND pending_device_id IS NOT NULL AND pending_path IS NOT NULL AND pending_workspace_id IS NULL)
    OR (pending_kind = 'workspace' AND pending_id IS NOT NULL
      AND pending_workspace_id IS NOT NULL AND pending_device_id IS NULL AND pending_path IS NULL)
  );
ALTER TABLE conversation_hosts ADD COLUMN detaching INTEGER NOT NULL DEFAULT 0 CHECK (detaching IN (0, 1));

CREATE TABLE permission_requests_next (
  id                TEXT PRIMARY KEY,
  conversation_id   TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  categories        TEXT NOT NULL,
  command           TEXT NOT NULL,
  node_id           TEXT NOT NULL,
  agent_number      INTEGER,
  agent_description TEXT,
  created_at        INTEGER NOT NULL,
  decision          TEXT CHECK (decision IN ('allowed', 'denied')),
  decided_at        INTEGER,
  CHECK ((agent_number IS NULL) = (agent_description IS NULL)),
  CHECK ((decision IS NULL) = (decided_at IS NULL))
) STRICT;
INSERT INTO permission_requests_next
  (id, conversation_id, categories, command, node_id, agent_number, agent_description, created_at, decision, decided_at)
  SELECT id, conversation_id, json_array(category), command, node_id, agent_number, agent_description, created_at, decision, decided_at
  FROM permission_requests;
DROP TABLE permission_requests;
ALTER TABLE permission_requests_next RENAME TO permission_requests;
CREATE INDEX permission_requests_of_conversation ON permission_requests (conversation_id, created_at);
DROP TABLE preview_namespace;
CREATE TABLE running_jobs (
  job_id          TEXT PRIMARY KEY,
  device_id       TEXT NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE
) STRICT;
CREATE INDEX running_jobs_of_device ON running_jobs (device_id);
ALTER TABLE devices ADD COLUMN runner_instance INTEGER;
ALTER TABLE devices ADD COLUMN cloud_started_at INTEGER;
";

/// From 0.1.11's conversation schema. SQLite cannot change a table's CHECK
/// or drop a column a CHECK names, so such a table is made anew under
/// another name, its rows are copied, and it takes the old one's place
/// (SQLite's `ALTER TABLE`, § Making Other Kinds Of Table Schema Changes).
const CONVERSATION_FROM_0_1_11: &str = "
-- The retention pass and the blob collector are gone, and with them
-- blob_refs, an index derived from the blocks that holds nothing else.
DROP TABLE blob_refs;

-- The attachment sequence.
CREATE TABLE sequences_next (
  name TEXT PRIMARY KEY CHECK (name IN ('command', 'shell', 'agent', 'tab', 'attachment')),
  next INTEGER NOT NULL CHECK (next >= 1)
) STRICT;
INSERT INTO sequences_next (name, next) SELECT name, next FROM sequences;
DROP TABLE sequences;
ALTER TABLE sequences_next RENAME TO sequences;

-- An output is no longer removed: the removed_at column goes, and the index
-- the retention pass searched. A row of an output 0.1.11 removed has no
-- state here, so its copy fails the CHECK, and with it the migration, which
-- leaves the database as 0.1.11 left it.
DROP INDEX command_outputs_expiry;
CREATE TABLE command_outputs_next (
  command_id     TEXT PRIMARY KEY,
  ended_at       INTEGER NOT NULL,
  blob           TEXT,
  missing_bytes  INTEGER CHECK (missing_bytes >= 0),
  missing_reason TEXT,
  media          TEXT,
  not_stored     TEXT,
  CHECK ((blob IS NOT NULL) + (not_stored IS NOT NULL) = 1),
  CHECK ((missing_bytes IS NULL) = (missing_reason IS NULL)),
  CHECK (missing_bytes IS NULL OR blob IS NOT NULL),
  CHECK ((media IS NULL) = (blob IS NULL))
) STRICT;
INSERT INTO command_outputs_next
  (command_id, ended_at, blob, missing_bytes, missing_reason, media, not_stored)
  SELECT command_id, ended_at, blob, missing_bytes, missing_reason, media, not_stored
  FROM command_outputs;
DROP TABLE command_outputs;
ALTER TABLE command_outputs_next RENAME TO command_outputs;

-- The attachments the agent uploads.
CREATE TABLE attachments (
  number     INTEGER PRIMARY KEY CHECK (number >= 1),
  name       TEXT NOT NULL,
  media_type TEXT NOT NULL,
  size       INTEGER NOT NULL CHECK (size >= 0),
  width      INTEGER CHECK (width >= 1),
  height     INTEGER CHECK (height >= 1),
  blob       TEXT NOT NULL,
  CHECK ((width IS NULL) = (height IS NULL))
) STRICT;
";

/// From 0.1.20's conversation schema: an output's record keeps how its
/// command ended, which no row 0.1.20 wrote says, so theirs stays null. The
/// table is made anew, as for 0.1.11, so the column takes its place among
/// the others.
const CONVERSATION_FROM_0_1_20: &str = "
CREATE TABLE command_outputs_next (
  command_id     TEXT PRIMARY KEY,
  ended_at       INTEGER NOT NULL,
  ending         TEXT,
  blob           TEXT,
  missing_bytes  INTEGER CHECK (missing_bytes >= 0),
  missing_reason TEXT,
  media          TEXT,
  not_stored     TEXT,
  CHECK ((blob IS NOT NULL) + (not_stored IS NOT NULL) = 1),
  CHECK ((missing_bytes IS NULL) = (missing_reason IS NULL)),
  CHECK (missing_bytes IS NULL OR blob IS NOT NULL),
  CHECK ((media IS NULL) = (blob IS NULL))
) STRICT;
INSERT INTO command_outputs_next
  (command_id, ended_at, blob, missing_bytes, missing_reason, media, not_stored)
  SELECT command_id, ended_at, blob, missing_bytes, missing_reason, media, not_stored
  FROM command_outputs;
DROP TABLE command_outputs;
ALTER TABLE command_outputs_next RENAME TO command_outputs;
";

/// From 0.1.21's control schema ([`CONTROL_FROM_0_1_21`]); and a stored
/// catalog's models no longer say whether they read attachments or video:
/// their accepted types come from the source's input modalities alone
/// (`models.md` § Accepted attachment types), so the two flags go and each
/// model keeps the list it held, null where the source stated none. Each
/// record changed is decoded and checked as the format it becomes before
/// it is written back.
fn control_from_0_1_21(transaction: &Transaction<'_>, _: &dyn ReadBlob) -> rusqlite::Result<()> {
    transaction.execute_batch(CONTROL_FROM_0_1_21)?;
    let records = transaction
        .prepare("SELECT provider_id, record FROM model_catalogs")?
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut update = transaction.prepare("UPDATE model_catalogs SET record = ?2 WHERE provider_id = ?1")?;
    for (provider, record) in records {
        let mut value: serde_json::Value = serde_json::from_str(&record).map_err(corrupt)?;
        if !without_attachment_flags(&mut value) {
            continue;
        }
        let record: crate::providers::CatalogRecord =
            json("model_catalogs", "record", &to_json(&value)).map_err(corrupt)?;
        update.execute(rusqlite::params![provider, to_json(&record)])?;
    }
    Ok(())
}

/// Removes every catalog model's `supportsAttachments` and `supportsVideo`
/// from `value`; true when it held some.
fn without_attachment_flags(value: &mut serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(fields) => {
            let mut changed = fields.remove("supportsAttachments").is_some();
            changed |= fields.remove("supportsVideo").is_some();
            for field in fields.values_mut() {
                changed |= without_attachment_flags(field);
            }
            changed
        }
        serde_json::Value::Array(items) => items
            .iter_mut()
            .fold(false, |changed, item| without_attachment_flags(item) | changed),
        _ => false,
    }
}

/// From 0.1.21's conversation schema: commands no longer run in shells, the
/// model has one tool, `shell`, and command reports replace yield wakeups
/// (`shell-tool.md` § Command reports). A command's output keeps no media: the
/// model views a file again when it needs it (`shell-tool.md` § Where media are
/// kept), so the media column goes, which a CHECK names, and the table is
/// made anew, as for 0.1.11. Block format 2 also lets a block keep
/// the entries of a vendor's own record of the session (`claude-code.md`
/// § The session a process resumes), which no block 0.1.21 stored has, so
/// leaving the field out writes them as format 2. Commands outlive their
/// connections: a running one is recorded, none of which 0.1.21 kept, and a
/// lost one says why (command ending format 2), which for 0.1.21's, lost
/// with their connections, is that. A page of history is read by the
/// blocks' types and found by a block's id or a command's, which the new
/// indexes serve.
fn from_0_1_21(transaction: &Transaction<'_>, _: &dyn ReadBlob) -> rusqlite::Result<()> {
    transaction.execute_batch(
        "
DELETE FROM sequences WHERE name = 'shell';
CREATE TABLE sequences_next (
  name TEXT PRIMARY KEY CHECK (name IN ('command', 'agent', 'tab', 'attachment')),
  next INTEGER NOT NULL CHECK (next >= 1)
) STRICT;
INSERT INTO sequences_next (name, next) SELECT name, next FROM sequences;
DROP TABLE sequences;
ALTER TABLE sequences_next RENAME TO sequences;
ALTER TABLE nodes DROP COLUMN wakeup_at;
CREATE TABLE command_outputs_next (
  command_id     TEXT PRIMARY KEY,
  ended_at       INTEGER NOT NULL,
  ending         TEXT,
  blob           TEXT,
  missing_bytes  INTEGER CHECK (missing_bytes >= 0),
  missing_reason TEXT,
  not_stored     TEXT,
  CHECK ((blob IS NOT NULL) + (not_stored IS NOT NULL) = 1),
  CHECK ((missing_bytes IS NULL) = (missing_reason IS NULL)),
  CHECK (missing_bytes IS NULL OR blob IS NOT NULL)
) STRICT;
INSERT INTO command_outputs_next
  (command_id, ended_at, ending, blob, missing_bytes, missing_reason, not_stored)
  SELECT command_id, ended_at, ending, blob, missing_bytes, missing_reason, not_stored
  FROM command_outputs;
DROP TABLE command_outputs;
ALTER TABLE command_outputs_next RENAME TO command_outputs;
CREATE TABLE running_commands (
  command_id  TEXT PRIMARY KEY,
  node_id     TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
  device_id   TEXT NOT NULL,
  job_id      TEXT NOT NULL,
  tool_use_id TEXT NOT NULL,
  started_at  INTEGER NOT NULL,
  places      TEXT NOT NULL
) STRICT;
CREATE INDEX running_commands_of_node ON running_commands (node_id);
CREATE INDEX blocks_by_type ON blocks (node_id, json_extract(block, '$.type'), idx);
CREATE INDEX blocks_by_id ON blocks (json_extract(block, '$.id'));
CREATE INDEX blocks_by_command ON blocks (json_extract(block, '$.view.commandId'));
",
    )?;
    transaction.execute(
        "UPDATE command_outputs SET ending = json_set(ending, '$.reason', ?1)
         WHERE json_extract(ending, '$.kind') = 'lost'",
        [LOST_IN_0_1_21],
    )?;
    blocks_of_one_tool(transaction)?;
    states_without_wakeups(transaction)
}

/// Turns a stored error into the migration's failure, which leaves the
/// database as 0.1.21 left it.
fn corrupt(error: impl std::error::Error + Send + Sync + 'static) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(error))
}

/// A storage step's failure inside a migration.
fn storage(error: StorageError) -> rusqlite::Error {
    match error {
        StorageError::Sqlite(error) => error,
        other => corrupt(other),
    }
}

/// Stored data that is not what its format says, in words.
fn corrupt_text(message: &str) -> rusqlite::Error {
    corrupt(std::io::Error::other(message.to_owned()))
}

/// From 0.1.21's checkpoint states to checkpoint state format 2: the yield
/// wakeups a node saved go, none of them to fire, and the state keeps the
/// command reports and the commands' intervals, none for a node 0.1.21
/// saved. Format 2 is not the current one: the migration to format 3
/// ([`nodes_with_their_directories`]) decodes and checks each state.
fn states_without_wakeups(transaction: &Transaction<'_>) -> rusqlite::Result<()> {
    let states = transaction
        .prepare("SELECT id, state FROM nodes")?
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut update = transaction.prepare("UPDATE nodes SET state = ?2 WHERE id = ?1")?;
    for (id, state) in states {
        let mut fields: serde_json::Map<String, serde_json::Value> =
            decode("nodes", "state", serde_json::from_str(&state)).map_err(corrupt)?;
        fields.remove("wakeups");
        fields.insert("reports".to_owned(), serde_json::json!([]));
        fields.insert("intervals".to_owned(), serde_json::json!([]));
        update.execute(rusqlite::params![id, to_json(&fields)])?;
    }
    Ok(())
}

/// From 0.1.21's transcript blocks (transcript block format 2): a stored
/// call of `shell_exec` is the `shell` call it became, with its input, and
/// its repeat guard's view is `repeated_shell`; a call of `yield` keeps its
/// name, which replays as text (`transcript.md` § Replay), and loses its
/// `yield_wakeup` view, which no tool has any more; a stored shell view no
/// longer names a shell; and a fired wakeup becomes what it stood for
/// (`rendering.md` § Rendering boundary): one a command's end fired, that
/// command's report, titled by the `description` of the call that started
/// the command; one its time fired, a `resume` block. Each block changed is
/// decoded and written again through the block's own encoding; a block that
/// does not decode stops the migration.
fn blocks_of_one_tool(transaction: &Transaction<'_>) -> rusqlite::Result<()> {
    let titles = command_titles_of_0_1_21(transaction)?;
    let mut blocks = Vec::new();
    {
        let mut statement = transaction.prepare("SELECT node_id, idx, block FROM blocks")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let text: String = row.get(2)?;
            let mut block: serde_json::Value = serde_json::from_str(&text).map_err(corrupt)?;
            if migrate_block(&mut block, &titles) {
                with_its_end(&mut block)?;
                let block: demi_shared_types::Block =
                    demi_shared_types::decode_value(block).map_err(corrupt)?;
                let node: String = row.get(0)?;
                let index: i64 = row.get(1)?;
                blocks.push((node, index, super::columns::to_json(&block)));
            }
        }
    }
    let mut update = transaction.prepare("UPDATE blocks SET block = ?3 WHERE node_id = ?1 AND idx = ?2")?;
    for (node, index, block) in blocks {
        update.execute(rusqlite::params![node, index, block])?;
    }
    Ok(())
}

/// From the conversation schemas of the development builds a server keeps
/// data of (`upgrades.md` § A development build on a server), each
/// block's run and each shell call's lists computed anew: from
/// 0.1.22-dev.20261011T064359Z's, whose blocks have neither, to transcript
/// block format 3, where each block of a run carries its run, and each
/// shell call's view its own files with those it removed and renamed, its
/// run's and its request's (`edit-tracking.md` § What the conversation
/// shows); from 0.1.22-dev.20261011T123543Z's, format 3, whose runs end at
/// a `resume` block and at a Stop the user continued, to format 4, where
/// the steps after them join the run before (`rendering.md` § Work groups),
/// with the run's files to match; from format 5's in which a node says
/// which agent it was forked from, which the server keeps data of, to
/// format 6, where redacted reasoning is a step of its run, and a run it
/// opens is named after it (format 6 also leaves out of a `response` the
/// usage its provider did not report, `claude-code.md` § Reading: every
/// stored response carries one, which format 6 reads as it is; responses
/// whose usage summed a Claude Code run stay as recorded, and the next
/// response anchors the estimate in their place, `compaction.md` § Context
/// estimate). Each agent's transcript is walked in
/// order from the edit copies in the conversation's blob namespace, by the
/// code that computes them for a new block. The tables are as they were. A
/// copy that cannot be read stops the migration, which the next open tries
/// again; one the namespace no longer holds leaves its file named alone.
fn relist_blocks(transaction: &Transaction<'_>, blobs: &dyn ReadBlob) -> rusqlite::Result<()> {
    let nodes = transaction
        .prepare("SELECT DISTINCT node_id FROM blocks")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut update = transaction.prepare("UPDATE blocks SET block = ?3 WHERE node_id = ?1 AND idx = ?2")?;
    for node in nodes {
        let rows = transaction
            .prepare("SELECT idx, block FROM blocks WHERE node_id = ?1 ORDER BY idx")?
            .query_map([&node], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let stored = rows
            .iter()
            .map(|(_, text)| {
                let mut value = serde_json::from_str::<serde_json::Value>(text).map_err(corrupt)?;
                with_no_kept_messages(&mut value);
                with_outside_paths(&mut value).map_err(corrupt)?;
                with_its_end(&mut value)?;
                demi_shared_types::decode_value::<demi_shared_types::Block>(value).map_err(corrupt)
            })
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut blocks = stored.clone();
        demi_agent_store::changes::relist(&mut blocks, |blob| blobs.read(blob))
            .map_err(|error| corrupt(std::io::Error::other(format!("an edit copy was not read: {error}"))))?;
        for (((index, _), before), after) in rows.iter().zip(&stored).zip(&blocks) {
            if before != after {
                update.execute(rusqlite::params![node, index, super::columns::to_json(after)])?;
            }
        }
    }
    Ok(())
}

/// The title of each command a call of 0.1.21's started, by its number: the
/// call's `description`, as the stored view of a `shell_exec` call names its
/// command. A `shell_status` look's view names the command it looked at,
/// which it did not start.
fn command_titles_of_0_1_21(transaction: &Transaction<'_>) -> rusqlite::Result<HashMap<String, String>> {
    use serde_json::Value;

    let mut titles = HashMap::new();
    let mut statement = transaction.prepare("SELECT block FROM blocks ORDER BY node_id, idx")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let text: String = row.get(0)?;
        let block: Value = serde_json::from_str(&text).map_err(corrupt)?;
        if block["type"] != "tool_call" || block["toolName"] != "shell_exec" {
            continue;
        }
        let Some(command) = block["view"]["commandId"].as_str() else {
            continue;
        };
        let input: Value = block["input"]
            .as_str()
            .and_then(|input| serde_json::from_str(input).ok())
            .unwrap_or(Value::Null);
        let title = input["description"].as_str().unwrap_or_default();
        titles.entry(command.to_owned()).or_insert_with(|| title.to_owned());
    }
    Ok(titles)
}

/// Changes one of 0.1.21's blocks into what it became; false when it stays
/// as it is. `titles` names the commands 0.1.21's calls started.
fn migrate_block(block: &mut serde_json::Value, titles: &HashMap<String, String>) -> bool {
    use serde_json::Value;

    let Some(fields) = block.as_object_mut() else {
        return false;
    };
    let mut changed = false;
    match fields.get("type").and_then(Value::as_str) {
        Some("tool_call") => {
            if fields.get("toolName").and_then(Value::as_str) == Some("shell_exec") {
                fields.insert("toolName".to_owned(), Value::from("shell"));
                changed = true;
            }
            let kind = fields
                .get("view")
                .and_then(|view| view.get("kind"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            match kind.as_deref() {
                Some("shell") => {
                    if let Some(view) = fields.get_mut("view").and_then(Value::as_object_mut) {
                        changed |= view.remove("shellId").is_some();
                    }
                }
                Some("repeated_shell_exec") => {
                    fields["view"]["kind"] = Value::from("repeated_shell");
                    changed = true;
                }
                Some("yield_wakeup") => {
                    fields.insert("view".to_owned(), Value::Null);
                    changed = true;
                }
                _ => {}
            }
        }
        Some("wakeup") => {
            match fields.remove("command") {
                Some(command) => {
                    let report = report_of_0_1_21(&command, titles);
                    fields.insert("reports".to_owned(), Value::Array(vec![report]));
                }
                None => {
                    fields.insert("type".to_owned(), Value::from("resume"));
                    fields.remove("placement");
                }
            }
            changed = true;
        }
        _ => {}
    }
    changed
}

/// Why every command 0.1.21 recorded lost was lost.
const LOST_IN_0_1_21: &str = "the connection to its Host was lost";

/// The report a wakeup 0.1.21 fired for `command`'s end stood for: the
/// command, its call's title, and how it ended. It carries no output: the
/// model read none with it then.
fn report_of_0_1_21(command: &serde_json::Value, titles: &HashMap<String, String>) -> serde_json::Value {
    let id = command["commandId"].as_str().unwrap_or_default();
    let end = &command["end"];
    let event = match end["kind"].as_str() {
        Some("exited") => ReportEvent::Ended {
            exit_code: end["exitCode"].as_i64().and_then(|code| i32::try_from(code).ok()),
        },
        Some("stopped") => ReportEvent::Stopped { by: None },
        // 0.1.21 lost a command only with its Host's connection.
        Some("lost") => ReportEvent::Lost {
            reason: LOST_IN_0_1_21.to_owned(),
        },
        _ => ReportEvent::Ended { exit_code: None },
    };
    serde_json::json!({
        "commandId": id,
        "title": titles.get(id).cloned().unwrap_or_default(),
        "event": event,
        "output": "",
    })
}

/// A kind of database, as a server's upgrade asks about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseKind {
    Control,
    Conversation,
}

impl DatabaseKind {
    fn schema(self) -> &'static Schema {
        match self {
            Self::Control => &CONTROL,
            Self::Conversation => &CONVERSATION,
        }
    }
}

/// What this build does with a database as it opens it (`upgrades.md`
/// § Prepare).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaFit {
    /// It records this build's schema and opens as it is.
    Current,
    /// It holds nothing yet or records a schema in the history: opening
    /// gives it this build's schema.
    Migrates,
    /// It records another schema, as a newer release or a development build
    /// of another unpublished schema made it: opening refuses it.
    Other,
}

/// What this build would do with the database at `path` as it opens it. The
/// database is read, never written.
pub fn schema_fit(path: &Path, kind: DatabaseKind) -> Result<SchemaFit, StorageError> {
    let connection = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let recorded: i32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    // A database that records no version and holds no table is new, and
    // `apply` gives it the schema.
    if recorded == 0 && tables(&connection)? == 0 {
        return Ok(SchemaFit::Migrates);
    }
    Ok(match kind.schema().current(recorded) {
        Some(true) => SchemaFit::Current,
        Some(false) => SchemaFit::Migrates,
        None => SchemaFit::Other,
    })
}

/// How many tables the database of `connection` holds.
fn tables(connection: &Connection) -> rusqlite::Result<i64> {
    connection.query_row(
        "SELECT count(*) FROM sqlite_schema WHERE type = 'table'",
        [],
        |row| row.get(0),
    )
}

/// The versions a database of `kind` records, for the tests of what reads
/// only a database's version, as an upgrade's preparation does.
#[cfg(feature = "testing")]
pub mod testing {
    use std::path::Path;

    use rusqlite::Connection;

    use super::{DatabaseKind, version_of};

    /// The version of this build's schema.
    pub fn current_version(kind: DatabaseKind) -> i32 {
        kind.schema().version()
    }

    /// The version of the newest schema a published release shipped before
    /// this build's.
    pub fn shipped_version(kind: DatabaseKind) -> i32 {
        let shipped = kind.schema().history.last().expect("a schema has a history");
        version_of(shipped.sql)
    }

    /// The columns a shipped schema has that this build's dropped, each with
    /// the value of a row of this build's that it holds: a node's block
    /// count is the length of its current path.
    const DROPPED: &[(&str, &str, &str)] = &[(
        "nodes",
        "block_count",
        "(SELECT coalesce(max(own.start + own.length), 0) FROM current.path_runs AS own WHERE own.node_id = id)",
    )];

    /// Whether the shipped schema's `table` has `column`.
    fn shipped_has(connection: &rusqlite::Connection, table: &str, column: &str) -> bool {
        connection
            .query_row(
                "SELECT count(*) FROM pragma_table_info(?1, 'main') WHERE name = ?2",
                [table, column],
                |row| row.get::<_, i64>(0),
            )
            .expect("the columns are listed")
            > 0
    }

    /// Turns the closed database of `kind` at `path`, of this build's
    /// schema, into one of the newest schema a published release shipped
    /// before it, as an upgrade finds it: the tables, columns and indexes of
    /// that schema, with the rows of this one in the columns both have, and
    /// that schema's version. Its values stay as this build wrote them.
    pub fn store_as_shipped(path: &Path, kind: DatabaseKind) {
        store_as_shipped_before(path, kind, 0);
    }

    /// The same, as the schema shipped `newer` schemas before the last one
    /// in the history, for a test of a migration that the schemas shipped
    /// since have moved away from the end of the history.
    pub fn store_as_shipped_before(path: &Path, kind: DatabaseKind, newer: usize) {
        let history = kind.schema().history;
        let shipped = history
            .get(history.len().checked_sub(newer + 1).expect("the history goes back that far"))
            .expect("a schema has a history");
        let earlier = path.with_extension("shipped");
        let connection = Connection::open(&earlier).expect("a database opens beside it");
        connection
            .execute_batch(shipped.sql)
            .expect("the shipped schema applies");
        connection
            .execute(
                "ATTACH DATABASE ?1 AS current",
                [path.to_str().expect("a test's path is UTF-8")],
            )
            .expect("the database attaches");
        let tables = connection
            .prepare("SELECT name FROM main.sqlite_schema WHERE type = 'table'")
            .and_then(|mut statement| {
                statement
                    .query_map([], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .expect("the shipped tables are listed");
        for table in tables {
            let columns = connection
                .prepare(
                    "SELECT name FROM pragma_table_info(?1, 'main')
                     WHERE name IN (SELECT name FROM pragma_table_info(?1, 'current'))",
                )
                .and_then(|mut statement| {
                    statement
                        .query_map([&table], |row| row.get::<_, String>(0))?
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .expect("the columns are listed");
            if columns.is_empty() {
                continue;
            }
            // A column this build dropped holds what it was derived into.
            let derived: Vec<(&str, &str)> = DROPPED
                .iter()
                .filter(|(owner, column, _)| *owner == table && shipped_has(&connection, &table, column))
                .map(|(_, column, value)| (*column, *value))
                .collect();
            let names = columns
                .iter()
                .map(String::as_str)
                .chain(derived.iter().map(|(column, _)| *column))
                .collect::<Vec<_>>()
                .join(", ");
            let values = columns
                .iter()
                .map(String::as_str)
                .chain(derived.iter().map(|(_, value)| *value))
                .collect::<Vec<_>>()
                .join(", ");
            connection
                .execute_batch(&format!(
                    "INSERT INTO main.{table} ({names}) SELECT {values} FROM current.{table}"
                ))
                .expect("the rows are copied");
        }
        connection
            .execute_batch("DETACH DATABASE current")
            .expect("the database detaches");
        connection
            .pragma_update(None, "user_version", version_of(shipped.sql))
            .expect("the version is recorded");
        drop(connection);
        // The copy read the write-ahead log of the database it replaces,
        // which would otherwise be applied to the copy.
        for suffix in ["-wal", "-shm"] {
            let mut log = path.as_os_str().to_owned();
            log.push(suffix);
            match std::fs::remove_file(&log) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    panic!("{log:?} is removed: {error}")
                }
                _ => {}
            }
        }
        std::fs::rename(&earlier, path).expect("the copy replaces the database");
    }
}

/// The version a database of the schema `sql` records in `user_version`:
/// the first 31 bits of its SHA-256, never 0, which SQLite gives a database
/// that records none. Any edit of the SQL is another version.
pub(crate) fn version_of(sql: &str) -> i32 {
    let digest = Sha256::digest(sql.as_bytes());
    let bits = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) >> 1;
    let version = i32::try_from(bits).expect("31 bits fit");
    version.max(1)
}

impl Schema {
    pub(crate) fn version(&self) -> i32 {
        version_of(self.sql)
    }

    /// The migrations that bring a database of `recorded`'s schema to this
    /// one, in order; none for a version the history does not hold.
    fn migrations(&self, recorded: i32) -> Option<&'static [Shipped]> {
        let from = self
            .history
            .iter()
            .position(|shipped| version_of(shipped.sql) == recorded)?;
        Some(&self.history[from..])
    }

    /// Gives a new database at `path` this schema, migrates one of a schema
    /// in the history, and accepts one that records this schema's version,
    /// each in one transaction. Any other database was made by a newer
    /// release or another build of Demi: it is refused, never changed.
    #[cfg(test)]
    pub(crate) fn apply(
        &self,
        connection: &mut Connection,
        path: &Path,
        blobs: &dyn ReadBlob,
    ) -> Result<(), StorageError> {
        self.migrate(connection, path, blobs, &NoSealedConfigs, || {})
    }

    /// [`Schema::apply`] for a database whose migrations re-encode sealed
    /// values with `seals`: the control database.
    pub(crate) fn apply_sealed(
        &self,
        connection: &mut Connection,
        path: &Path,
        seals: &dyn ResealConfigs,
    ) -> Result<(), StorageError> {
        self.migrate(connection, path, &NoBlobs, seals, || {})
    }

    /// [`Schema::apply`], which calls `migrating` once its transaction holds
    /// the database's write lock and before it migrates a database of a
    /// schema in the history: where a test holds a conversation's migration
    /// (`conversations::MigrationHold`).
    pub(crate) fn apply_with(
        &self,
        connection: &mut Connection,
        path: &Path,
        blobs: &dyn ReadBlob,
        migrating: impl FnOnce(),
    ) -> Result<(), StorageError> {
        self.migrate(connection, path, blobs, &NoSealedConfigs, migrating)
    }

    fn migrate(
        &self,
        connection: &mut Connection,
        path: &Path,
        blobs: &dyn ReadBlob,
        seals: &dyn ResealConfigs,
        migrating: impl FnOnce(),
    ) -> Result<(), StorageError> {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let recorded: i32 =
            transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if recorded == self.version() {
            return Ok(());
        }
        if recorded == 0 && tables(&transaction)? == 0 {
            transaction.execute_batch(self.sql)?;
        } else {
            let migrations = self
                .migrations(recorded)
                .ok_or_else(|| StorageError::OtherSchema {
                    path: path.to_owned(),
                })?;
            migrating();
            for shipped in migrations {
                let migrated = match shipped.migration {
                    Migration::Sql(sql) => transaction.execute_batch(sql),
                    Migration::Code(migrate) => migrate(&transaction, blobs),
                    Migration::Reseal(migrate) => migrate(&transaction, seals),
                };
                migrated.map_err(|source| StorageError::Migration {
                    path: path.to_owned(),
                    from: version_of(shipped.sql),
                    source,
                })?;
            }
        }
        transaction.pragma_update(None, "user_version", self.version())?;
        transaction.commit()?;
        Ok(())
    }

    /// Whether a database that records `recorded` opens as it is, or must be
    /// migrated by `apply` first; `None` for one this build cannot open.
    pub(crate) fn current(&self, recorded: i32) -> Option<bool> {
        if recorded == self.version() {
            return Some(true);
        }
        self.migrations(recorded).map(|_| false)
    }
}

const CONTROL_V1: &str = r"
-- Identity. One master: two concurrent setups cannot both create one.
CREATE TABLE users (
  id            TEXT PRIMARY KEY,
  email         TEXT NOT NULL UNIQUE COLLATE NOCASE,
  nickname      TEXT NOT NULL,
  password_hash TEXT NOT NULL,
  role          TEXT NOT NULL CHECK (role IN ('master', 'admin', 'user')),
  created_at    INTEGER NOT NULL,
  -- A disabled account is signed out and refused every sign-in; nothing of it is removed.
  disabled      INTEGER NOT NULL DEFAULT 0 CHECK (disabled IN (0, 1)),
  -- True until the owner of an account an administrator made or reset chooses its own password.
  chooses_password INTEGER NOT NULL DEFAULT 0 CHECK (chooses_password IN (0, 1)),
  -- Written with a session's last activity, so it outlives the sessions; none before the first sign-in.
  last_active_at INTEGER,
  -- The size of the account's Cloud disks in whole GB, which its Cloud's next start takes.
  cloud_system_gb INTEGER NOT NULL DEFAULT 16 CHECK (cloud_system_gb BETWEEN 1 AND 1024),
  cloud_home_gb   INTEGER NOT NULL DEFAULT 16 CHECK (cloud_home_gb BETWEEN 1 AND 1024)
) STRICT;
CREATE UNIQUE INDEX users_one_master ON users (role) WHERE role = 'master';

-- A signed-in browser: the SHA-256 of its cookie's token, and its id in the
-- account's list. Its last activity is written at most once an hour; it was
-- verified when its user last proved it was them on it; a handoff code that
-- made it is named, though the code's row is replaced by the next one; the
-- administrator whose sign-in code made it is named too.
CREATE TABLE web_sessions (
  token_hash     TEXT PRIMARY KEY,
  id             TEXT NOT NULL UNIQUE,
  user_id        TEXT NOT NULL REFERENCES users (id),
  expires_at     INTEGER NOT NULL,
  name           TEXT NOT NULL,
  created_at     INTEGER NOT NULL,
  last_active_at INTEGER NOT NULL,
  verified_at    INTEGER,
  handoff_id     TEXT,
  issued_by      TEXT REFERENCES users (id)
) STRICT;
CREATE INDEX web_sessions_expiry ON web_sessions (expires_at);
CREATE INDEX web_sessions_of_user ON web_sessions (user_id, created_at);

-- A browser's push subscription of a user (turns.md § Notifications), one
-- per session, which it serves and which takes it along when it ends: its
-- push service's endpoint, which names the browser's subscription, its two
-- keys and the events it chose.
CREATE TABLE push_subscriptions (
  session_id       TEXT PRIMARY KEY REFERENCES web_sessions (id) ON DELETE CASCADE,
  endpoint         TEXT NOT NULL UNIQUE,
  user_id          TEXT NOT NULL REFERENCES users (id),
  p256dh           TEXT NOT NULL,
  auth             TEXT NOT NULL,
  turn_finishes    INTEGER NOT NULL CHECK (turn_finishes IN (0, 1)),
  turn_fails       INTEGER NOT NULL CHECK (turn_fails IN (0, 1)),
  needs_permission INTEGER NOT NULL CHECK (needs_permission IN (0, 1)),
  agent_notifies   INTEGER NOT NULL CHECK (agent_notifies IN (0, 1)),
  created_at       INTEGER NOT NULL
) STRICT;
CREATE INDEX push_subscriptions_of_user ON push_subscriptions (user_id);

-- Each user's one handoff code and one sign-in code, by the SHA-256 of the
-- code, with the path it opens; a sign-in code is made by an administrator
-- for the account, who is named. A spent one stays until the next of its
-- kind replaces it.
CREATE TABLE handoff_codes (
  user_id    TEXT NOT NULL REFERENCES users (id),
  kind       TEXT NOT NULL CHECK (kind IN ('handoff', 'sign_in')),
  issued_by  TEXT REFERENCES users (id),
  id         TEXT NOT NULL UNIQUE,
  code_hash  TEXT NOT NULL UNIQUE,
  to_path    TEXT NOT NULL,
  expires_at INTEGER NOT NULL,
  spent      INTEGER NOT NULL CHECK (spent IN (0, 1)),
  PRIMARY KEY (user_id, kind),
  CHECK ((kind = 'sign_in') = (issued_by IS NOT NULL))
) STRICT;

-- One pending address change per user.
CREATE TABLE email_challenges (
  user_id       TEXT PRIMARY KEY REFERENCES users (id),
  id            TEXT NOT NULL UNIQUE,
  email         TEXT NOT NULL,
  password_hash TEXT NOT NULL,
  code_hash     TEXT NOT NULL,
  expires_at    INTEGER NOT NULL,
  sent_at       INTEGER NOT NULL,
  attempts      INTEGER NOT NULL
) STRICT;

-- Each user's passkeys, by credential id in base64url; a public key is no
-- secret, so it is stored as it is.
CREATE TABLE passkeys (
  id           TEXT PRIMARY KEY,
  user_id      TEXT NOT NULL REFERENCES users (id),
  name         TEXT NOT NULL,
  -- passkeys.credential: webauthn-rs 0.5's Passkey
  credential   TEXT NOT NULL,
  created_at   INTEGER NOT NULL,
  last_used_at INTEGER
) STRICT;
CREATE INDEX passkeys_of_user ON passkeys (user_id, created_at);

-- The challenges of passkey sign-ins and additions, until used or expired;
-- an addition's is the user's.
CREATE TABLE passkey_challenges (
  id         TEXT PRIMARY KEY,
  kind       TEXT NOT NULL CHECK (kind IN ('sign_in', 'registration')),
  user_id    TEXT REFERENCES users (id),
  -- passkey_challenges.state: webauthn-rs 0.5's DiscoverableAuthentication
  -- for a sign-in, PasskeyRegistration for a registration
  state      TEXT NOT NULL,
  expires_at INTEGER NOT NULL,
  CHECK ((kind = 'registration') = (user_id IS NOT NULL))
) STRICT;
CREATE INDEX passkey_challenges_expiry ON passkey_challenges (expires_at);

CREATE TABLE user_preferences (
  user_id     TEXT PRIMARY KEY REFERENCES users (id),
  preferences TEXT NOT NULL
) STRICT;

-- Each user's personal instructions (`instructions.md` § Personal
-- instructions): a user with no row has none.
CREATE TABLE user_instructions (
  user_id TEXT PRIMARY KEY REFERENCES users (id),
  text    TEXT NOT NULL
) STRICT;

-- Each user's Subagents switch and limits (`subagents.md` § Profiles,
-- § Limits): a user with no row has subagents on, at most 10 live and 1
-- level deep. Each limit is the number last set and whether it is lifted
-- (Unlimited), which keeps the number.
CREATE TABLE user_subagents (
  user_id             TEXT PRIMARY KEY REFERENCES users (id),
  enabled             INTEGER NOT NULL CHECK (enabled IN (0, 1)),
  max_live            INTEGER NOT NULL CHECK (max_live >= 1),
  max_live_unlimited  INTEGER NOT NULL CHECK (max_live_unlimited IN (0, 1)),
  max_depth           INTEGER NOT NULL CHECK (max_depth >= 1),
  max_depth_unlimited INTEGER NOT NULL CHECK (max_depth_unlimited IN (0, 1))
) STRICT;

-- Each user's subagent profiles: the model settings as JSON, null for the
-- parent's model, and the instructions added to a child's system prompt
-- after the product's, null for none. A name is unique among its user's
-- profiles.
CREATE TABLE subagent_profiles (
  id           TEXT PRIMARY KEY,
  user_id      TEXT NOT NULL REFERENCES users (id),
  name         TEXT NOT NULL,
  description  TEXT NOT NULL,
  model        TEXT,
  instructions TEXT,
  can_spawn    INTEGER NOT NULL CHECK (can_spawn IN (0, 1)),
  enabled      INTEGER NOT NULL CHECK (enabled IN (0, 1)),
  -- The profile's own Context in tokens, its model's full window or a
  -- limit; null for the user's limit, and always without a model.
  context_limit INTEGER CHECK (context_limit >= 1),
  UNIQUE (user_id, name)
) STRICT;

-- Devices and workspaces. The token hash is absent until the backend issues
-- a token; one managed device per user.
CREATE TABLE devices (
  id           TEXT PRIMARY KEY,
  user_id      TEXT NOT NULL REFERENCES users (id),
  kind         TEXT NOT NULL CHECK (kind IN ('user', 'managed')),
  name         TEXT NOT NULL,
  platform     TEXT NOT NULL,
  token_hash   TEXT UNIQUE,
  claimed_at   INTEGER NOT NULL,
  last_seen_at INTEGER,
  installed    TEXT NOT NULL DEFAULT '[]',
  -- The operating system its runner last reported, JSON, and the runner
  -- release it reported with it; none before its runner first connected.
  os           TEXT,
  runner_version TEXT,
  -- Whether the backend's last shutdown ended its runner's connection; the
  -- next start reads it and clears it.
  ended_by_shutdown INTEGER NOT NULL DEFAULT 0 CHECK (ended_by_shutdown IN (0, 1)),
  -- How pages reach it: Automatic, Prefer Direct or Server Only, which the
  -- user picks on the device's page.
  route        TEXT NOT NULL DEFAULT 'automatic' CHECK (route IN ('automatic', 'direct', 'server')),
  -- The instance its runner last named, a number each start of the runner
  -- draws; none before its runner first connected.
  runner_instance INTEGER,
  -- When a Cloud's latest boot completed, as its runner connected; the
  -- lifetime cap counts from it. None for a paired device and a Cloud that
  -- never booted.
  cloud_started_at INTEGER,
  -- A paired device's number among its user's devices of the same logo
  -- (conversations.md § Conversations and projects), given when its runner first
  -- reports its system and again when its logo changes; none before, and
  -- for a Cloud.
  mark_number INTEGER CHECK (mark_number > 0),
  -- Which managed machine a managed device is: the user's Cloud, or the
  -- Claude Code machine, which only the instance's master holds
  -- (managed-hosts.md § The Claude Code machine); none for a paired device.
  machine      TEXT CHECK (machine IN ('cloud', 'claude_code')),
  -- The home its runner last reported (sessions-and-targets.md § Resolve a
  -- target); none before its runner first reported one since the release
  -- that began keeping it.
  home TEXT
) STRICT;
CREATE UNIQUE INDEX devices_one_machine ON devices (user_id, machine) WHERE kind = 'managed';

CREATE TABLE workspaces (
  id         TEXT PRIMARY KEY,
  user_id    TEXT NOT NULL REFERENCES users (id),
  device_id  TEXT NOT NULL REFERENCES devices (id),
  path       TEXT NOT NULL,
  name       TEXT NOT NULL,
  sort_order INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  -- What the project's next conversation starts with (web-api.md § User
  -- preferences), as the user last chose it there: model settings as JSON,
  -- and a Workflow menu choice typed as a conversation's; each none before
  -- its first choice.
  last_model TEXT,
  last_workflow_kind TEXT
    CHECK (last_workflow_kind IN ('default', 'none', 'workflow')),
  last_workflow_id TEXT
    CHECK ((last_workflow_kind IS 'workflow') = (last_workflow_id IS NOT NULL))
) STRICT;
CREATE INDEX workspaces_order ON workspaces (user_id, sort_order);
CREATE UNIQUE INDEX workspaces_directory ON workspaces (user_id, device_id, path);

-- Each choice a user made about a plugin (`plugins.md` § A user's plugins):
-- a plugin with no row is on.
CREATE TABLE user_plugins (
  user_id TEXT NOT NULL REFERENCES users (id),
  plugin  TEXT NOT NULL,
  enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
  PRIMARY KEY (user_id, plugin)
) STRICT, WITHOUT ROWID;

-- Each plugin's values for a user (`plugins.md` § The contract): a JSON
-- document its plugin decodes, the revision a conditional write names, and
-- the blobs the value names, a JSON array of their SHA-256s.
CREATE TABLE plugin_values (
  user_id  TEXT NOT NULL REFERENCES users (id),
  plugin   TEXT NOT NULL,
  key      TEXT NOT NULL,
  document TEXT NOT NULL,
  revision INTEGER NOT NULL CHECK (revision >= 1),
  blobs    TEXT NOT NULL,
  PRIMARY KEY (user_id, plugin, key)
) STRICT, WITHOUT ROWID;

-- Each plugin's Host directories for a user (`plugins.md` § Host
-- directories): the directory's name, its digest and its files, a JSON
-- array of each file's path, whether it is executable and its SHA-256.
CREATE TABLE plugin_directories (
  user_id TEXT NOT NULL REFERENCES users (id),
  plugin  TEXT NOT NULL,
  name    TEXT NOT NULL,
  digest  TEXT NOT NULL,
  files   TEXT NOT NULL,
  PRIMARY KEY (user_id, plugin, name)
) STRICT, WITHOUT ROWID;

-- Each plugin's secrets for a user (`plugins.md` § Secrets): sealed, with
-- the plugin, the user and the name as the additional data, and the
-- revision a conditional write names. The plugin host keeps a sign-in's
-- tokens here too, under a name no plugin can give.
CREATE TABLE plugin_secrets (
  user_id  TEXT NOT NULL REFERENCES users (id),
  plugin   TEXT NOT NULL,
  name     TEXT NOT NULL,
  sealed   BLOB NOT NULL,
  revision INTEGER NOT NULL CHECK (revision >= 1),
  PRIMARY KEY (user_id, plugin, name)
) STRICT, WITHOUT ROWID;

-- The public client each plugin registered with its vendor's authorization
-- server (`plugins.md` § OAuth sign-in), and the redirect URI it was
-- registered with: a client of another URI is registered again.
CREATE TABLE plugin_oauth_clients (
  plugin       TEXT NOT NULL,
  issuer       TEXT NOT NULL,
  redirect_uri TEXT NOT NULL,
  client_id    TEXT NOT NULL,
  PRIMARY KEY (plugin, issuer)
) STRICT, WITHOUT ROWID;

-- Each user's credentials (`credentials.md`): the fields as JSON, credential
-- fields format 1, `[{ name, label, kind }]`, never a value; the values, a
-- JSON object by field name, sealed with the user and the name as the
-- additional data, so a rename seals them again; when it is removed, null
-- while it is kept always; and when an agent last read it. A name is unique
-- among its user's credentials.
CREATE TABLE user_credentials (
  id           TEXT PRIMARY KEY,
  user_id      TEXT NOT NULL REFERENCES users (id),
  name         TEXT NOT NULL,
  note         TEXT NOT NULL,
  fields       TEXT NOT NULL,
  sealed       BLOB NOT NULL,
  keep_until   INTEGER,
  last_used_at INTEGER,
  created_at   INTEGER NOT NULL,
  updated_at   INTEGER NOT NULL,
  UNIQUE (user_id, name)
) STRICT;
CREATE INDEX user_credentials_by_expiry ON user_credentials (keep_until) WHERE keep_until IS NOT NULL;

-- The conversation index. The web app chooses a conversation's id and its
-- case is kept, but no two ids differ only in case: each names a database
-- file, and a file system may ignore case. The target is Cloud (optionally a
-- directory on it), a device directory or a workspace, checked per kind. The
-- target columns are not foreign keys: a conversation that targets a device
-- directly does not block revoking that device.
CREATE TABLE conversations (
  id                  TEXT PRIMARY KEY COLLATE NOCASE,
  user_id             TEXT NOT NULL REFERENCES users (id),
  -- title: empty exactly while title_origin is 'placeholder', a title that
  -- holds no words (conversations.md § Conversation titles).
  title               TEXT NOT NULL,
  title_origin        TEXT NOT NULL
                        CHECK (title_origin IN ('placeholder', 'message', 'generated', 'user')),
  archived            INTEGER NOT NULL CHECK (archived IN (0, 1)),
  pinned              INTEGER NOT NULL CHECK (pinned IN (0, 1)),
  sort_order          INTEGER NOT NULL,
  read_revision       INTEGER NOT NULL,
  -- target: where it runs (sessions-and-targets.md § Resolve a target): the
  -- Cloud's directory of a conversation, its own without a path; a Host's
  -- home, by its device; a workspace, with the device and directory it
  -- names; or a device's directory as a release before projects by
  -- directory kept it, until that device's runner first reports its home,
  -- and of a conversation whose device was revoked.
  target_kind         TEXT NOT NULL CHECK (target_kind IN ('cloud', 'home', 'workspace', 'device')),
  target_device_id    TEXT,
  target_path         TEXT,
  target_workspace_id TEXT,
  -- last_switch: the latest target switch, TargetSwitch as JSON; with
  -- last_switch_revision, the execution-context revision it made, both null
  -- before the first. A CHECK would tie them, but SQLite tests one an added
  -- column brings against the rows before the migration fills it.
  last_switch         TEXT,
  cloud_reset_id      TEXT,
  context_version     INTEGER NOT NULL,
  model               TEXT,
  created_at          INTEGER NOT NULL,
  updated_at          INTEGER NOT NULL,
  -- The revision of its attached hosts, raised by each change of them; 0
  -- before the first.
  hosts_revision      INTEGER NOT NULL DEFAULT 0 CHECK (hosts_revision >= 0),
  -- The pending move an agent asked for (sessions-and-targets.md § Switch
  -- the primary target): its id and its target, typed as the target is; all
  -- null while none waits.
  pending_id           TEXT,
  pending_kind         TEXT CHECK (pending_kind IN ('cloud', 'device', 'workspace')),
  pending_device_id    TEXT,
  pending_path         TEXT,
  pending_workspace_id TEXT CHECK (
    (pending_kind IS NULL
      AND pending_id IS NULL AND pending_device_id IS NULL AND pending_path IS NULL
      AND pending_workspace_id IS NULL)
    OR (pending_kind = 'cloud' AND pending_id IS NOT NULL
      AND pending_device_id IS NULL AND pending_workspace_id IS NULL)
    OR (pending_kind = 'device' AND pending_id IS NOT NULL
      AND pending_device_id IS NOT NULL AND pending_path IS NOT NULL AND pending_workspace_id IS NULL)
    OR (pending_kind = 'workspace' AND pending_id IS NOT NULL
      AND pending_workspace_id IS NOT NULL AND pending_device_id IS NULL AND pending_path IS NULL)
  ),
  -- Its Workflow menu choice (subagents.md § Workflows of a project): the
  -- user's settings, No Subagents, or the workflow whose file is
  -- .demi/workflows/<workflow_id>.yaml.
  workflow_kind TEXT NOT NULL DEFAULT 'default' CHECK (workflow_kind IN ('default', 'none', 'workflow')),
  workflow_id   TEXT CHECK ((workflow_kind = 'workflow') = (workflow_id IS NOT NULL)),
  last_switch_revision INTEGER,
  CHECK (
    (target_kind = 'cloud'
      AND target_device_id IS NULL AND target_workspace_id IS NULL)
    OR (target_kind = 'home'
      AND target_device_id IS NOT NULL AND target_path IS NULL AND target_workspace_id IS NULL)
    OR (target_kind = 'device'
      AND target_device_id IS NOT NULL AND target_path IS NOT NULL AND target_workspace_id IS NULL)
    OR (target_kind = 'workspace'
      AND target_workspace_id IS NOT NULL AND target_device_id IS NOT NULL AND target_path IS NOT NULL)
  )
) STRICT;
CREATE INDEX conversations_sidebar ON conversations (user_id, archived, pinned, sort_order);
CREATE INDEX conversations_workspace ON conversations (target_workspace_id);

-- Each job of a command that runs, with its device, whose runner's hello
-- names the job, and its conversation, whose tree takes the command up
-- again (`storage.md` § Command outputs). Written before the job starts and
-- deleted with the command's end; written before then too while the job's
-- start waits for its device, with sent 0, until it is sent.
CREATE TABLE running_jobs (
  job_id          TEXT PRIMARY KEY,
  device_id       TEXT NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  sent            INTEGER NOT NULL DEFAULT 1 CHECK (sent IN (0, 1))
) STRICT;
CREATE INDEX running_jobs_of_device ON running_jobs (device_id);

-- A device attached to a conversation, which goes by its device's name; the
-- row id keeps the order of hosts attached at one time.
CREATE TABLE conversation_hosts (
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id),
  device_id       TEXT NOT NULL REFERENCES devices (id),
  -- The directory its commands start in, fixed when it was attached; null
  -- for a device attached by name, whose commands start in its home.
  cwd             TEXT,
  attached_at     INTEGER NOT NULL,
  -- An agent's detach, which waits for the tree to be idle.
  detaching       INTEGER NOT NULL DEFAULT 0 CHECK (detaching IN (0, 1)),
  PRIMARY KEY (conversation_id, device_id)
) STRICT;
CREATE INDEX conversation_hosts_device ON conversation_hosts (device_id);

-- A conversation's work panel: its tabs and the ids it ever had, one JSON
-- document, with the revision that counts its changes.
CREATE TABLE conversation_panels (
  conversation_id TEXT PRIMARY KEY COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  revision        INTEGER NOT NULL CHECK (revision >= 1),
  document        TEXT NOT NULL,
  updated_at      INTEGER NOT NULL
) STRICT;

-- A conversation's draft: its text and files with the revision they were
-- written at, and the version a save replaced with the revision that version
-- had. The row stays when the draft is emptied, so a revision is never used
-- twice. It holds the revision of the conversation's messages saved for
-- later too, which every save, edit, send and removal of one raises.
CREATE TABLE conversation_drafts (
  conversation_id   TEXT PRIMARY KEY COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  revision          INTEGER NOT NULL CHECK (revision >= 1),
  document          TEXT NOT NULL,
  written           INTEGER NOT NULL CHECK (written BETWEEN 1 AND revision),
  replaced_revision INTEGER,
  replaced          TEXT,
  saved_revision    INTEGER NOT NULL CHECK (saved_revision >= 0),
  updated_at        INTEGER NOT NULL,
  CHECK ((replaced IS NULL) = (replaced_revision IS NULL))
) STRICT;

-- A conversation's messages saved for later: each one's text and files as
-- one JSON document, as a draft holds them.
CREATE TABLE saved_messages (
  id              TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  saved_at        INTEGER NOT NULL,
  document        TEXT NOT NULL
) STRICT;
CREATE INDEX saved_messages_of_conversation ON saved_messages (conversation_id, saved_at);

-- An edit of a message saved for later, by the page's id for it, with its
-- answer, kept a day so the same edit sent again answers as it first did.
-- answer: the edit's answer, `{ draft, revisions }` as `web-api.md`
-- § Messages saved for later names it, as JSON.
CREATE TABLE saved_edits (
  op              TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  answer          TEXT NOT NULL,
  made_at         INTEGER NOT NULL
) STRICT;
CREATE INDEX saved_edits_by_age ON saved_edits (made_at);

-- A message saved for later that a send took, with the turn the send became,
-- kept a day so the same send, or a save of it, sent again is known.
-- document: its text and files as saved_messages keeps them.
CREATE TABLE saved_sends (
  id              TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  turn_id         TEXT NOT NULL,
  saved_at        INTEGER NOT NULL,
  document        TEXT NOT NULL,
  sent_at         INTEGER NOT NULL
) STRICT;
CREATE INDEX saved_sends_by_age ON saved_sends (sent_at);

-- A conversation's requests (permissions.md § Requests), each of a kind: a
-- permission request is a command an agent ran without the grants of its
-- categories, a JSON array of their ids sorted so that the same categories
-- compare equal; a credential request asks the user for a credential of a
-- name, its ask a JSON document, credential request format 1, `{ reason,
-- fields: [{ name, label, kind }], keep }`, and is gone once answered, since
-- the command that asked waits for the answer; an allowance request asks to
-- use a saved credential in the conversation, which names it while it is
-- saved. A request is undecided until the user decides and then kept until
-- the decision's message is in the agent's checkpoint, and read once the
-- user has opened the conversation with the request waiting (the sidebar's
-- yellow mark settles then). The agent is the root when its number and
-- description are null. Each category the user allowed for a conversation,
-- and each credential, are the conversation's grants and allowances.
CREATE TABLE permission_requests (
  id                TEXT PRIMARY KEY,
  conversation_id   TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  kind              TEXT NOT NULL CHECK (kind IN ('permission', 'credential', 'allowance')),
  categories        TEXT,
  credential_name   TEXT,
  credential_id     TEXT REFERENCES user_credentials (id) ON DELETE SET NULL,
  ask               TEXT,
  command           TEXT NOT NULL,
  node_id           TEXT NOT NULL,
  agent_number      INTEGER,
  agent_description TEXT,
  created_at        INTEGER NOT NULL,
  decision          TEXT CHECK (decision IN ('allowed', 'denied')),
  decided_at        INTEGER,
  read              INTEGER NOT NULL DEFAULT 0 CHECK (read IN (0, 1)),
  -- The host a permission request asks to reach (permissions.md § Hosts):
  -- a device, or the pairing code of a computer `demi host pair` pairs; none
  -- for a request of categories alone, whose categories are then one or more.
  host_device_id    TEXT CHECK (host_device_id IS NULL OR kind = 'permission'),
  pairing_code      TEXT CHECK (pairing_code IS NULL OR (kind = 'permission' AND host_device_id IS NULL)),
  CHECK ((kind = 'permission') = (categories IS NOT NULL)),
  CHECK ((kind = 'permission') = (credential_name IS NULL)),
  CHECK ((kind = 'credential') = (ask IS NOT NULL)),
  CHECK (kind = 'allowance' OR credential_id IS NULL),
  CHECK (kind <> 'credential' OR decision IS NULL),
  CHECK ((agent_number IS NULL) = (agent_description IS NULL)),
  CHECK ((decision IS NULL) = (decided_at IS NULL))
) STRICT;
CREATE INDEX permission_requests_of_conversation ON permission_requests (conversation_id, created_at);

CREATE TABLE permission_grants (
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  category        TEXT NOT NULL,
  granted_at      INTEGER NOT NULL,
  PRIMARY KEY (conversation_id, category)
) STRICT;

-- The hosts a conversation may reach besides its primary one
-- (sessions-and-targets.md § Hosts a conversation may reach): each primary
-- target it left, each host attached to it, and each host the user allowed
-- when an agent asked; kept for the conversation's life, as a grant is.
CREATE TABLE reachable_hosts (
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  device_id       TEXT NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
  added_at        INTEGER NOT NULL,
  PRIMARY KEY (conversation_id, device_id)
) STRICT;
CREATE INDEX reachable_hosts_of_device ON reachable_hosts (device_id);

-- The pairing codes the user let a conversation pair (permissions.md
-- § Hosts): its next `demi host pair` of the code is dispatched, once.
CREATE TABLE pairing_passes (
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  code            TEXT NOT NULL,
  allowed_at      INTEGER NOT NULL,
  PRIMARY KEY (conversation_id, code)
) STRICT;

CREATE TABLE credential_allowances (
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  credential_id   TEXT NOT NULL REFERENCES user_credentials (id) ON DELETE CASCADE,
  allowed_at      INTEGER NOT NULL,
  PRIMARY KEY (conversation_id, credential_id)
) STRICT;
CREATE INDEX credential_allowances_of_credential ON credential_allowances (credential_id);

-- A conversation whose root's running turn a graceful shutdown began to
-- interrupt, marked before the turn was aborted, which the next start resumes
-- once (`failures-and-recovery.md` § The unfinished turn).
CREATE TABLE interrupted_turns (
  conversation_id TEXT PRIMARY KEY COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE
) STRICT;

-- Each share of a conversation or of one answer of it, behind its token,
-- the only key to it (`sharing.md` § Storage and routes): its owner, its
-- conversation, its kind, the answer's block for an answer's share, and when
-- its snapshot, in the conversation's database, was taken. A conversation
-- has at most one share of its own and one of each answer; its shares go
-- with it.
CREATE TABLE shares (
  token           TEXT PRIMARY KEY,
  user_id         TEXT NOT NULL REFERENCES users (id),
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  kind            TEXT NOT NULL CHECK (kind IN ('conversation', 'answer')),
  block_id        TEXT,
  -- The conversation's title when the snapshot was taken; null while it
  -- was the placeholder.
  title           TEXT,
  taken_at        INTEGER NOT NULL,
  CHECK ((kind = 'answer') = (block_id IS NOT NULL))
) STRICT;
CREATE UNIQUE INDEX shares_of_conversation ON shares (conversation_id) WHERE kind = 'conversation';
CREATE UNIQUE INDEX shares_of_answer ON shares (conversation_id, block_id) WHERE kind = 'answer';

-- A conversation's deletion that has not reached its last step: its record
-- is gone, and a start finishes what is left (`storage.md` § Deleting a
-- conversation). The id is compared as the index compares it.
CREATE TABLE conversation_deletions (
  id         TEXT PRIMARY KEY COLLATE NOCASE,
  user_id    TEXT NOT NULL REFERENCES users (id),
  deleted_at INTEGER NOT NULL
) STRICT;

-- Records that make interrupted multi-step work discoverable. A Fork
-- reserves its destination's conversation id, compared as the index
-- compares it.
-- conversation_fork_operations.metadata: Fork metadata format 3, ForkMetadata
-- as JSON, whose attached hosts are their devices and directories without
-- names and which holds the source's execution-context revision at the Fork
-- and its workflow (format 1 named the hosts and held no revision, format 2
-- held no workflow).
CREATE TABLE conversation_fork_operations (
  id        TEXT PRIMARY KEY COLLATE NOCASE,
  user_id   TEXT NOT NULL REFERENCES users (id),
  source_id TEXT NOT NULL,
  block_id  TEXT NOT NULL,
  metadata  TEXT NOT NULL
) STRICT;

CREATE TABLE managed_operations (
  device_id    TEXT NOT NULL REFERENCES devices (id),
  operation_id TEXT NOT NULL,
  base_version TEXT NOT NULL,
  phase        TEXT NOT NULL
                 CHECK (phase IN ('stopping', 'saving', 'rebuilding', 'booting', 'ready', 'failed')),
  error        TEXT,
  updated_at   INTEGER NOT NULL,
  PRIMARY KEY (device_id, operation_id)
) STRICT;

-- Providers: an API-key entry's sealed configuration, a subscription entry's
-- active account, a sealed secret per subscription account, and one
-- subscription entry per owner and family. The active account is not a
-- foreign key: removing an account clears it in the same transaction.
-- providers.config: provider configuration format 2, sealed.
CREATE TABLE providers (
  id                   TEXT PRIMARY KEY,
  owner_user_id        TEXT NOT NULL REFERENCES users (id),
  provider_type        TEXT NOT NULL,
  credential_kind      TEXT NOT NULL CHECK (credential_kind IN ('api_key', 'subscription')),
  label                TEXT NOT NULL,
  config               BLOB,
  active_credential_id TEXT,
  created_at           INTEGER NOT NULL,
  -- Which account each agent uses (providers.md § Account rotation); an
  -- API-key entry's is always manual.
  rotation             TEXT NOT NULL DEFAULT 'manual'
    CHECK (rotation IN ('manual', 'in_order', 'balanced')
      AND (credential_kind = 'subscription' OR rotation = 'manual')),
  CHECK ((credential_kind = 'api_key') = (config IS NOT NULL)),
  CHECK (credential_kind = 'subscription' OR active_credential_id IS NULL)
) STRICT;
CREATE INDEX providers_owner ON providers (owner_user_id, created_at);
CREATE UNIQUE INDEX providers_one_subscription ON providers (owner_user_id, provider_type)
  WHERE credential_kind = 'subscription';

CREATE TABLE provider_credentials (
  provider_id  TEXT NOT NULL REFERENCES providers (id) ON DELETE CASCADE,
  id           TEXT NOT NULL,
  identity_key TEXT,
  label        TEXT NOT NULL,
  detail       TEXT,
  source       TEXT NOT NULL,
  -- The sealed secret document; none for a Claude Code account, whose
  -- sign-in its CLI keeps (providers.md § Subscription secrets).
  secret       BLOB,
  version      INTEGER NOT NULL CHECK (version >= 1),
  -- A quota snapshot, format 2, whose windows have a kind (format 1 named
  -- each window in English).
  quota        TEXT,
  updated_at   INTEGER NOT NULL,
  -- The account's place in its entry's list, which In Order follows.
  position     INTEGER NOT NULL DEFAULT 0 CHECK (position >= 0),
  -- The vendor refused the account's sign-in; a login of it clears this.
  signed_out   INTEGER NOT NULL DEFAULT 0 CHECK (signed_out IN (0, 1)),
  PRIMARY KEY (provider_id, id)
) STRICT;
CREATE UNIQUE INDEX provider_credentials_identity ON provider_credentials (provider_id, identity_key)
  WHERE identity_key IS NOT NULL;

CREATE TABLE model_catalogs (
  provider_id TEXT PRIMARY KEY REFERENCES providers (id) ON DELETE CASCADE,
  record      TEXT NOT NULL
) STRICT;

-- Usage and attachments; neither holds attachment bytes.
-- One row per vendor call. A row written before the ledger kept a call's
-- agent, turn, purpose, account, ids, hour-long cache write, outcome, quota
-- and start keeps null for them; a call the vendor reported no counts for
-- has null counts.
-- usage_ledger.quota: [{ kind, periodSeconds, scope, usedPercent, resetsAt }], format 1
CREATE TABLE usage_ledger (
  id                    TEXT PRIMARY KEY,
  user_id               TEXT NOT NULL REFERENCES users (id),
  conversation_id       TEXT NOT NULL,
  -- Null for a request outside a session, such as a title.
  agent                 INTEGER CHECK (agent >= 0),
  turn_id               TEXT,
  purpose               TEXT CHECK (purpose IN ('turn', 'compaction', 'title', 'image')),
  provider_id           TEXT NOT NULL,
  -- Null for an API key.
  account_id            TEXT,
  model_id              TEXT NOT NULL,
  request_id            TEXT,
  vendor_id             TEXT,
  input_tokens          INTEGER,
  output_tokens         INTEGER,
  cache_read_tokens     INTEGER,
  cache_write_tokens    INTEGER,
  cache_write_1h_tokens INTEGER,
  -- 'answered', 'cancelled' or the failure code of a refused call.
  outcome               TEXT,
  quota                 TEXT,
  started_at            INTEGER,
  ended_at              INTEGER NOT NULL
) STRICT;
CREATE INDEX usage_ledger_user ON usage_ledger (user_id, ended_at);

CREATE TABLE attachments (
  id         TEXT PRIMARY KEY,
  user_id    TEXT NOT NULL REFERENCES users (id),
  media_type TEXT NOT NULL,
  size_bytes INTEGER NOT NULL,
  sha256     TEXT NOT NULL,
  snippet    TEXT,
  created_at INTEGER NOT NULL
) STRICT;
";

/// One agent tree (`storage.md` § Conversation state and transactions): the
/// root node's id is the conversation's, and its descendants are subagents.
/// A node row carries identity and relationship; its checkpoint is the state
/// row's JSON and its block rows.
const CONVERSATION_V1: &str = r"
CREATE TABLE nodes (
  id               TEXT PRIMARY KEY,
  number           INTEGER NOT NULL UNIQUE CHECK (number >= 0),
  parent_id        TEXT REFERENCES nodes (id) ON DELETE CASCADE,
  description      TEXT NOT NULL,
  profile          TEXT,
  -- The instructions the node's profile added to its system prompt; null
  -- when it added none, or the node has its parent's prompt.
  instructions     TEXT,
  round            INTEGER NOT NULL CHECK (round >= 1),
  started_at       INTEGER NOT NULL,
  can_spawn        INTEGER NOT NULL CHECK (can_spawn IN (0, 1)),
  closed_phase     TEXT CHECK (closed_phase IN ('completed', 'aborted', 'error')),
  closed_at        INTEGER,
  result           TEXT,
  failure          TEXT,
  delivered        INTEGER NOT NULL CHECK (delivered IN (0, 1)),
  -- The session's checkpoint state as JSON, with whether a Stop holds its
  -- waiting input until the user's next action.
  -- nodes.state: checkpoint state format 4
  state            TEXT NOT NULL,
  output_revision  INTEGER NOT NULL CHECK (output_revision >= 0),
  -- The number of the agent the node was forked from; null for one that
  -- was not forked.
  forked_from      INTEGER CHECK (forked_from >= 1),
  -- A child's own working directory on the primary Host, and where the
  -- conversation worked when it was set: the primary Host's device, null
  -- for a Cloud not made yet, and the conversation's directory then. The
  -- child works there while the conversation still does, and in the
  -- conversation's directory otherwise. All null for the root, which works
  -- in the conversation's. Their reader refuses a row that sets some of
  -- them only, since a migration adds no table CHECK.
  cwd              TEXT CHECK (cwd <> ''),
  cwd_device       TEXT,
  cwd_under        TEXT CHECK (cwd_under <> ''),
  -- The Context its profile chose, in tokens, which its sessions use in
  -- place of the user's limit; null for the user's limit.
  context_limit    INTEGER CHECK (context_limit >= 1),
  -- The row number of the current path's last block, the current leaf;
  -- null while the path is empty (message-editing.md § Versions).
  leaf             INTEGER CHECK (leaf >= 0),
  -- The row a cut of the path left off its end that no row has replaced
  -- yet: the next row written at the path's end replaces it. Null when the
  -- path's end is no cut, or an edit or Regenerate cut it, whose next row
  -- is a version of its own.
  cut              INTEGER CHECK (cut >= 0),
  -- The id of the parent's call block that started this child, so the
  -- child belongs to the branch that call is on; null for the root and for
  -- a child stored before versions existed, which is on every path.
  started_by       TEXT,
  CHECK ((closed_phase IS NULL) = (closed_at IS NULL)),
  CHECK (result IS NULL OR closed_phase = 'completed'),
  CHECK (failure IS NULL OR closed_phase = 'error')
) STRICT;
-- One root per tree.
CREATE UNIQUE INDEX nodes_root ON nodes ((parent_id IS NULL)) WHERE parent_id IS NULL;
CREATE INDEX nodes_children ON nodes (parent_id, number);

-- The next number of each sequence the model sees in the conversation: a
-- number is advanced past in its own transaction before it is given out,
-- so a crash leaves a gap and never gives a number twice.
CREATE TABLE sequences (
  name TEXT PRIMARY KEY CHECK (name IN ('command', 'agent', 'tab', 'attachment')),
  next INTEGER NOT NULL CHECK (next >= 1)
) STRICT;

-- blocks.block: transcript block format 10.
-- A node's transcript is a tree of rows (message-editing.md § Versions):
-- `idx` is a row's number, its place in the order the node's rows were
-- written, which no other row is ever given. A row is never deleted.
CREATE TABLE blocks (
  node_id  TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
  idx      INTEGER NOT NULL CHECK (idx >= 0),
  -- blocks.block: transcript block format 10
  block    TEXT NOT NULL,
  -- The row before it on its path; null for a path's first row.
  follows  INTEGER CHECK (follows < idx),
  -- The row it was written in place of, when the session's own rewrite (a
  -- retry, an unwind, a compaction inside a turn) began a branch with it;
  -- it is then of that row's version. Null for every other row.
  replaces INTEGER CHECK (replaces < idx),
  PRIMARY KEY (node_id, idx)
) STRICT;
-- A page of a transcript is read by its requests, which start at its user
-- blocks; a block is found by its id, and the call that started a command
-- by the command its view names (web-api-conversations.md § Conversation history).
CREATE INDEX blocks_by_type ON blocks (node_id, json_extract(block, '$.type'), idx);
CREATE INDEX blocks_by_id ON blocks (json_extract(block, '$.id'));
CREATE INDEX blocks_by_command ON blocks (json_extract(block, '$.view.commandId'));
-- The rows that follow a row: its versions and the branches the session's
-- rewrites left; and the rows that do not follow the row written before
-- them, where a place may hold versions.
CREATE INDEX blocks_by_follows ON blocks (node_id, follows);
CREATE INDEX blocks_branching ON blocks (node_id, follows) WHERE follows IS NOT idx - 1;
CREATE INDEX blocks_carrying ON blocks (node_id, replaces) WHERE replaces IS NOT NULL;
-- Each node's current path as runs of consecutive row numbers: the run at
-- path position `start` holds the rows `first` to `first + length - 1`.
-- Derived from the rows and the node's leaf, and written with the leaf in
-- the same transaction, so a page of the path reads an index range.
CREATE TABLE path_runs (
  node_id TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
  start   INTEGER NOT NULL CHECK (start >= 0),
  first   INTEGER NOT NULL CHECK (first >= 0),
  length  INTEGER NOT NULL CHECK (length >= 1),
  PRIMARY KEY (node_id, start)
) STRICT, WITHOUT ROWID;
-- Each blob a block names, as a medium or a side of an edit, by the block's
-- node and index; written with the block in the same checkpoint and gone
-- with it, so what names a blob is found without reading a block. A
-- share's check of a blob and the collection of the user's namespace read
-- it (`storage.md` § Retention).
CREATE TABLE block_blobs (
  node_id TEXT NOT NULL,
  idx     INTEGER NOT NULL,
  blob    TEXT NOT NULL,
  PRIMARY KEY (node_id, idx, blob),
  FOREIGN KEY (node_id, idx) REFERENCES blocks (node_id, idx) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;
CREATE INDEX block_blobs_by_blob ON block_blobs (blob);
-- Each attachment a block's text names by `attachment:a3`, by the block's
-- node and index; written with the block like block_blobs, so a share lists
-- the attachments it shows without reading a block (`storage.md`).
CREATE TABLE block_attachments (
  node_id    TEXT NOT NULL,
  idx        INTEGER NOT NULL,
  attachment INTEGER NOT NULL CHECK (attachment >= 1),
  PRIMARY KEY (node_id, idx, attachment),
  FOREIGN KEY (node_id, idx) REFERENCES blocks (node_id, idx) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;

-- Each share's list (`sharing.md` § Storage and routes): what it shows, by
-- the share's token and the position in it, from 0: a message or a text of
-- the root's transcript, by the row that holds it, its node and index, or,
-- with neither, by the share's copy in share_blocks at the same token and
-- position; or, with `steps`, a run of that many steps not shared. Every
-- read of a share goes through its list. The share's control record says
-- the token is valid; rows of a token it no longer has are never read.
CREATE TABLE share_entries (
  token    TEXT NOT NULL,
  position INTEGER NOT NULL CHECK (position >= 0),
  node_id  TEXT,
  idx      INTEGER CHECK (idx >= 0),
  steps    INTEGER CHECK (steps >= 1),
  CHECK ((node_id IS NULL) = (idx IS NULL)),
  CHECK (steps IS NULL OR node_id IS NULL),
  PRIMARY KEY (token, position)
) STRICT;
-- The entries that name a transcript row, for a block found by its id or
-- its type and for a write that replaces the row.
CREATE INDEX share_entries_by_row ON share_entries (node_id, idx, token);
-- A share's copies of the texts still being written when its list was
-- made, as they stood then, by the token and their position on the list.
CREATE TABLE share_blocks (
  token TEXT NOT NULL,
  idx   INTEGER NOT NULL CHECK (idx >= 0),
  -- share_blocks.block: transcript block format 10
  block TEXT NOT NULL,
  PRIMARY KEY (token, idx)
) STRICT;
-- Each blob a share's copies name, by the token.
CREATE TABLE share_blobs (
  token TEXT NOT NULL,
  blob  TEXT NOT NULL,
  PRIMARY KEY (token, blob)
) STRICT, WITHOUT ROWID;
-- Each attachment a share's text names that existed when its list was
-- made, copied from `attachments`; its blob stays shared with the
-- conversation's.
CREATE TABLE share_attachments (
  token      TEXT NOT NULL,
  number     INTEGER NOT NULL CHECK (number >= 1),
  name       TEXT NOT NULL,
  media_type TEXT NOT NULL,
  size       INTEGER NOT NULL CHECK (size >= 0),
  width      INTEGER CHECK (width >= 1),
  height     INTEGER CHECK (height >= 1),
  blob       TEXT NOT NULL,
  CHECK ((width IS NULL) = (height IS NULL)),
  PRIMARY KEY (token, number)
) STRICT;

-- Each ended command's whole output: stored as a blob, with the bytes at
-- its end the backend does not have and why; or not stored, and why; and
-- the title of the call that started it, empty for a row written before
-- rows kept it.
CREATE TABLE command_outputs (
  command_id     TEXT PRIMARY KEY,
  ended_at       INTEGER NOT NULL,
  -- How it ended, a JSON object: its exit code, stopped, or with its Host's
  -- connection; null for a row a release before 0.1.21 wrote.
  -- command_outputs.ending: command ending format 2
  ending         TEXT,
  blob           TEXT,
  missing_bytes  INTEGER CHECK (missing_bytes >= 0),
  missing_reason TEXT,
  not_stored     TEXT,
  title          TEXT NOT NULL DEFAULT '',
  CHECK ((blob IS NOT NULL) + (not_stored IS NOT NULL) = 1),
  CHECK ((missing_bytes IS NULL) = (missing_reason IS NULL)),
  CHECK (missing_bytes IS NULL OR blob IS NOT NULL)
) STRICT;

-- Each command that runs, until its end writes its command_outputs row in
-- the same transaction: the node that ran it, its device, its job's id on
-- that device, the call that started it, when it started, how far each
-- node that looked at it has seen its output, a JSON object of each node's
-- bytes of stdout and stderr by the node's id, the title of the call that
-- started it, empty for a command recorded before records kept it, and the
-- interval in milliseconds that call asked it to report at, null for one
-- that reports only its end.
CREATE TABLE running_commands (
  command_id  TEXT PRIMARY KEY,
  node_id     TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
  device_id   TEXT NOT NULL,
  job_id      TEXT NOT NULL,
  tool_use_id TEXT NOT NULL,
  started_at  INTEGER NOT NULL,
  -- running_commands.places: command places format 1
  places      TEXT NOT NULL,
  title       TEXT NOT NULL DEFAULT '',
  interval_ms INTEGER CHECK (interval_ms >= 1),
  -- The turn whose shell call started it; null for a command recorded
  -- before records kept it.
  turn_id     TEXT,
  -- What it starts once its Host is back, while its start waits for it and
  -- was never sent; null once sent. running_commands.unsent: unsent start
  -- format 1.
  unsent      TEXT
) STRICT;
CREATE INDEX running_commands_of_node ON running_commands (node_id);

-- Each attachment the agent uploaded, by its number in the conversation's
-- attachment sequence: the file's name, the media type read from its bytes,
-- its size, an image's or a video's width and height in pixels when its
-- header gives them, and its blob in the owner's namespace. A row never
-- changes; a Fork's seed copies the rows, and the blobs stay shared.
CREATE TABLE attachments (
  number     INTEGER PRIMARY KEY CHECK (number >= 1),
  name       TEXT NOT NULL,
  media_type TEXT NOT NULL,
  size       INTEGER NOT NULL CHECK (size >= 0),
  width      INTEGER CHECK (width >= 1),
  height     INTEGER CHECK (height >= 1),
  blob       TEXT NOT NULL,
  CHECK ((width IS NULL) = (height IS NULL))
) STRICT;
";

#[cfg(test)]
mod tests {
    use super::*;

    /// The tables, columns, indexes and foreign keys of a database, as
    /// SQLite reports them, and each table's and index's definition with
    /// its constraints, whatever SQL made them: comments, spacing and
    /// quoted names aside, which a migration's `ALTER TABLE` changes.
    fn shape(connection: &Connection) -> Vec<String> {
        let mut shape = Vec::new();
        let mut definitions = connection
            .prepare("SELECT type, name, sql FROM sqlite_schema WHERE sql IS NOT NULL ORDER BY name")
            .unwrap();
        let definitions: Vec<String> = definitions
            .query_map([], |row| {
                let sql: String = row.get(2)?;
                Ok(format!("{} {} {}", row.get::<_, String>(0)?, row.get::<_, String>(1)?, definition(&sql)))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect();
        shape.extend(definitions);
        let mut tables = connection
            .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name")
            .unwrap();
        let names: Vec<String> = tables
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        for table in names {
            for pragma in ["table_xinfo", "index_list", "foreign_key_list"] {
                let mut rows = connection
                    .prepare(&format!("SELECT * FROM pragma_{pragma}(?1)"))
                    .unwrap();
                let width = rows.column_count();
                let mut found: Vec<String> = rows
                    .query_map([&table], |row| {
                        let values: Vec<String> = (0..width)
                            .map(|column| format!("{:?}", row.get_ref(column).unwrap()))
                            .collect();
                        Ok(format!("{table} {pragma} {}", values.join(" ")))
                    })
                    .unwrap()
                    .map(Result::unwrap)
                    .collect();
                found.sort();
                shape.extend(found);
            }
        }
        let mut indexes = connection
            .prepare("SELECT name, tbl_name FROM sqlite_schema WHERE type = 'index' AND sql IS NOT NULL ORDER BY name")
            .unwrap();
        let indexes: Vec<(String, String)> = indexes
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        for (index, table) in indexes {
            let mut columns = connection
                .prepare("SELECT seqno, name FROM pragma_index_info(?1) ORDER BY seqno")
                .unwrap();
            // An expression has no column name; the index's definition
            // above holds it.
            let columns: Vec<String> = columns
                .query_map([&index], |row| {
                    let name: Option<String> = row.get(1)?;
                    Ok(name.unwrap_or_else(|| "(expression)".to_owned()))
                })
                .unwrap()
                .map(Result::unwrap)
                .collect();
            shape.push(format!("{table} index {index} {}", columns.join(",")));
        }
        shape
    }

    /// `sql` without its comments, quotes and spacing.
    fn definition(sql: &str) -> String {
        let code: Vec<&str> = sql
            .lines()
            .map(|line| line.split_once("--").map_or(line, |(code, _)| code))
            .collect();
        code.join(" ")
            .replace('"', "")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .replace("( ", "(")
            .replace(" )", ")")
            .replace(" ,", ",")
    }

    /// A database file that holds nothing yet.
    fn empty() -> (tempfile::TempDir, std::path::PathBuf, Connection) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("database.sqlite");
        let connection = Connection::open(&path).unwrap();
        (directory, path, connection)
    }

    /// A database as a release of the schema `sql` made it.
    fn database(sql: &str) -> (tempfile::TempDir, std::path::PathBuf, Connection) {
        let (directory, path, connection) = empty();
        connection.execute_batch(sql).unwrap();
        connection
            .pragma_update(None, "user_version", version_of(sql))
            .unwrap();
        (directory, path, connection)
    }

    const SHIPPED: &str = "CREATE TABLE notes (id TEXT PRIMARY KEY) STRICT;";
    const CURRENT: &str =
        "CREATE TABLE notes (id TEXT PRIMARY KEY, title TEXT NOT NULL DEFAULT '') STRICT;";

    /// A database of a shipped schema becomes one of the current schema
    /// with its rows; one whose migration fails stays as it was; one of a
    /// schema the history does not hold is refused.
    #[test]
    fn a_shipped_schema_migrates_a_failed_migration_changes_nothing_and_another_is_refused() {
        let schema = Schema {
            sql: CURRENT,
            history: &[Shipped {
                sql: SHIPPED,
                migration: Migration::Sql("ALTER TABLE notes ADD COLUMN title TEXT NOT NULL DEFAULT '';"),
            }],
        };
        let (_directory, path, mut connection) = database(SHIPPED);
        connection.execute("INSERT INTO notes (id) VALUES ('a')", []).unwrap();
        assert_eq!(schema.current(version_of(SHIPPED)), Some(false));
        schema.apply(&mut connection, &path, &NoBlobs).unwrap();
        let version: i32 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, version_of(CURRENT));
        let title: String = connection
            .query_row("SELECT title FROM notes WHERE id = 'a'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(title, "");
        let (_fresh_directory, fresh_path, mut fresh) = empty();
        schema.apply(&mut fresh, &fresh_path, &NoBlobs).unwrap();
        assert_eq!(shape(&connection), shape(&fresh));

        let failing = Schema {
            sql: CURRENT,
            history: &[Shipped {
                sql: SHIPPED,
                migration: Migration::Code(|transaction, _| {
                    transaction.execute_batch("ALTER TABLE notes ADD COLUMN title TEXT NOT NULL DEFAULT '';")?;
                    transaction.execute_batch("INSERT INTO missing VALUES (1);")
                }),
            }],
        };
        let (_failing_directory, failing_path, mut failed) = database(SHIPPED);
        let refused = failing.apply(&mut failed, &failing_path, &NoBlobs);
        assert!(matches!(refused, Err(StorageError::Migration { .. })), "{refused:?}");
        let version: i32 = failed
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, version_of(SHIPPED));
        let columns: i64 = failed
            .query_row("SELECT count(*) FROM pragma_table_info('notes')", [], |row| row.get(0))
            .unwrap();
        assert_eq!(columns, 1);

        let (_other_directory, other_path, mut other) = database("CREATE TABLE other (id TEXT) STRICT;");
        let refused = schema.apply(&mut other, &other_path, &NoBlobs);
        assert!(matches!(refused, Err(StorageError::OtherSchema { .. })), "{refused:?}");
        assert_eq!(schema.current(version_of("CREATE TABLE other (id TEXT) STRICT;")), None);
    }

    /// Every schema a published release shipped migrates to the current
    /// one: the same tables, columns, indexes, constraints and foreign keys
    /// as a new database.
    /// The server runs 0.1.22-dev.20261016T095626Z, whose control schema
    /// its database records: an upgrade keeps its accounts and gives each
    /// the default size of its Cloud disks.
    #[test]
    fn a_control_database_of_0_1_22_dev_20261016t095626z_keeps_its_accounts_and_gets_the_default_cloud_storage() {
        let deployed = include_str!("schema/control-0.1.22-dev.20261016T095626Z.sql");
        assert_eq!(version_of(deployed), 690_719_127, "the server's recorded version");
        let (_directory, path, mut database) = database(deployed);
        database
            .execute_batch(
                "INSERT INTO users (id, email, nickname, password_hash, role, created_at)
                   VALUES ('u-1', 'ana@example.test', 'Ana', 'hash', 'master', 1),
                          ('u-2', 'bo@example.test', 'Bo', 'hash', 'user', 2);",
            )
            .unwrap();
        CONTROL.apply(&mut database, &path, &NoBlobs).unwrap();
        let rows: Vec<(String, String, i64, i64)> = database
            .prepare("SELECT id, nickname, cloud_system_gb, cloud_home_gb FROM users ORDER BY id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            rows,
            [("u-1".into(), "Ana".into(), 16, 16), ("u-2".into(), "Bo".into(), 16, 16)]
        );
        let recorded: i32 = database.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
        assert_eq!(recorded, version_of(CONTROL_V1));
    }

    #[test]
    fn each_shipped_schema_migrates_to_the_current_one() {
        for schema in [CONTROL, CONVERSATION] {
            let (_fresh_directory, fresh_path, mut fresh) = empty();
            schema.apply(&mut fresh, &fresh_path, &NoBlobs).unwrap();
            for shipped in schema.history {
                let (_directory, path, mut migrated) = database(shipped.sql);
                schema.apply(&mut migrated, &path, &NoBlobs).unwrap();
                assert_eq!(shape(&migrated), shape(&fresh));
            }
        }
    }

    /// 0.1.11's rows of a conversation keep their values through the
    /// rebuilt tables, and the attachment sequence counts in it; an output
    /// 0.1.11 removed, which has no state now, stops the migration and
    /// leaves the database as it was.
    #[test]
    fn a_conversation_of_0_1_11_keeps_its_rows_and_an_output_it_removed_stops_the_migration() {
        use demi_host_interface::Missing;
        use demi_shared_types::{BlobRef, CommandEnd, CommandId, Sequence, Timestamp};

        use crate::command_outputs::{self, CommandOutput, OutputRow};
        use crate::sequences;

        let shipped = CONVERSATION.history[0].sql;
        let blob = BlobRef::try_from("a".repeat(64)).unwrap();
        // The media 0.1.11 kept beside an output, which go with 0.1.21's
        // migration.
        let media = format!(
            r#"[{{"number":1,"mediaType":"image/png","size":12,"kept":{{"state":"stored","blob":"{blob}"}}}}]"#
        );
        let (_directory, path, mut connection) = database(shipped);
        connection
            .execute_batch("INSERT INTO sequences (name, next) VALUES ('command', 7), ('tab', 3);")
            .unwrap();
        connection
            .execute(
                "INSERT INTO command_outputs
                   (command_id, ended_at, blob, missing_bytes, missing_reason, media, not_stored, removed_at)
                 VALUES ('c1', 1000, ?1, 25, 'lost with the Host''s connection', ?2, NULL, NULL),
                        ('c2', 2000, NULL, NULL, NULL, NULL, 'the object store refused the write', NULL)",
                rusqlite::params![blob.as_str(), media],
            )
            .unwrap();

        CONVERSATION.apply(&mut connection, &path, &NoBlobs).unwrap();
        let stored = command_outputs::read(&connection, &CommandId::try_from("c1").unwrap()).unwrap();
        assert_eq!(
            stored,
            Some(CommandOutput {
                command: CommandId::try_from("c1").unwrap(),
                title: String::new(),
                ended: Timestamp::from_millisecond(1000).unwrap(),
                // Its release kept no end, and the migrations invent none.
                end: CommandEnd::Unrecorded,
                output: OutputRow::Stored {
                    blob,
                    missing: Some(Missing {
                        bytes: 25,
                        reason: "lost with the Host's connection".to_owned(),
                    }),
                },
            })
        );
        let not_stored = command_outputs::read(&connection, &CommandId::try_from("c2").unwrap()).unwrap();
        assert_eq!(
            not_stored.map(|row| row.output),
            Some(OutputRow::NotStored("the object store refused the write".to_owned()))
        );
        assert_eq!(
            sequences::all(&connection).unwrap(),
            vec![(Sequence::Command, 7), (Sequence::Tab, 3)]
        );
        assert_eq!(sequences::next(&connection, Sequence::Attachment).unwrap(), 1);

        let (_removed_directory, removed_path, mut removed) = database(shipped);
        removed
            .execute_batch(
                "INSERT INTO command_outputs (command_id, ended_at, removed_at) VALUES ('c3', 1000, 2000);",
            )
            .unwrap();
        let refused = CONVERSATION.apply(&mut removed, &removed_path, &NoBlobs);
        assert!(matches!(refused, Err(StorageError::Migration { .. })), "{refused:?}");
        let version: i32 = removed
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, version_of(shipped));
        let removed_at: i64 = removed
            .query_row("SELECT removed_at FROM command_outputs WHERE command_id = 'c3'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(removed_at, 2000);
    }

    /// A node's checkpoint state as 0.1.21 saved it, with the yield
    /// wakeups `wakeups`.
    fn state_of_0_1_21(wakeups: serde_json::Value) -> String {
        serde_json::json!({
            "phase": "idle",
            "queue": [],
            "agentInputs": [],
            "wakeups": wakeups,
            "cwd": "/w",
            "model": demi_agent_store::testing::test_model(),
            "edits": []
        })
        .to_string()
    }

    /// 0.1.21's checkpoint states lose their yield wakeups and hold no
    /// command report and no command's interval; a state that is not one of
    /// 0.1.21's stops the migration.
    /// Planted defect this catches: a stored catalog left with its models'
    /// attachment flags, which no longer decodes, so the entry would read
    /// as corrupt until its source answers again.
    #[test]
    fn a_catalog_of_0_1_21_keeps_its_models_without_their_attachment_flags() {
        let shipped = include_str!("schema/control-0.1.21.sql");
        let (_directory, path, mut connection) = database(shipped);
        connection.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
        let model = serde_json::json!({
            "id": "m", "displayName": "M", "description": null, "contextWindow": 1000,
            "outputLimit": null, "supportsTools": true, "supportsAttachments": true,
            "supportsVideo": null, "acceptedExtensions": ["png"], "supportsReasoning": null,
            "supportedThinkingEfforts": null, "canDisableThinking": null, "serviceTiers": [],
            "defaultServiceTierId": null, "cost": null
        });
        let record = serde_json::json!({
            "key": "k", "checkedAt": "2026-09-23T10:04:12.000Z",
            "catalog": {
                "models": [model], "defaultModelId": null, "warnings": [],
                "sourceFetchedAt": "2026-09-23T10:04:12.000Z", "stale": false
            }
        });
        connection
            .execute(
                "INSERT INTO model_catalogs (provider_id, record) VALUES ('p1', ?1)",
                [record.to_string()],
            )
            .unwrap();
        CONTROL.apply(&mut connection, &path, &NoBlobs).unwrap();
        let text: String = connection
            .query_row("SELECT record FROM model_catalogs WHERE provider_id = 'p1'", [], |row| row.get(0))
            .unwrap();
        let record: crate::providers::CatalogRecord = json("model_catalogs", "record", &text).unwrap();
        assert_eq!(
            record.catalog.models[0].accepted_extensions,
            Some(vec![demi_shared_types::FileExtension::Png])
        );
    }

    /// Every API-key entry's sealed configuration of the schema before
    /// provider configuration format 2 is sealed again by the vault's side,
    /// a subscription entry's left alone, in the one transaction that
    /// records the current version; one configuration it cannot read stops
    /// the migration and leaves every row and the version as they were.
    /// Planted defect this catches: an unreadable configuration skipped,
    /// which leaves a format 1 value that no read of the new format decodes,
    /// or a migration that commits the rows it resealed before it.
    /// One small database each, milliseconds.
    #[test]
    fn sealed_configurations_are_resealed_together_or_not_at_all() {
        struct Prefixed;
        impl ResealConfigs for Prefixed {
            fn provider_config(&self, id: &ProviderId, family: &str, sealed: &[u8]) -> Result<Vec<u8>, String> {
                if sealed == b"unreadable" {
                    return Err("the configuration does not open".to_owned());
                }
                Ok([format!("{id}/{family}:").as_bytes(), sealed].concat())
            }
        }
        let released = |configs: &[(&str, &[u8])]| {
            let (directory, path, connection) =
                database(include_str!("schema/control-0.1.22-dev.profile-contexts.sql"));
            connection.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
            for (id, config) in configs {
                connection
                    .execute(
                        "INSERT INTO providers (id, owner_user_id, provider_type, credential_kind, label, config, created_at)
                         VALUES (?1, 'u', 'anthropic', 'api_key', 'Anthropic', ?2, 1)",
                        rusqlite::params![id, config],
                    )
                    .unwrap();
            }
            connection
                .execute(
                    "INSERT INTO providers (id, owner_user_id, provider_type, credential_kind, label, created_at)
                     VALUES ('s', 'u', 'claude-code', 'subscription', 'Claude Code', 1)",
                    [],
                )
                .unwrap();
            (directory, path, connection)
        };
        let configs = |connection: &Connection| {
            connection
                .prepare("SELECT id, config FROM providers ORDER BY id")
                .unwrap()
                .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<Vec<u8>>>(1)?)))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        let version = |connection: &Connection| -> i32 {
            connection.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap()
        };

        let (_directory, path, mut connection) = released(&[("a", b"one"), ("b", b"two")]);
        CONTROL.apply_sealed(&mut connection, &path, &Prefixed).unwrap();
        assert_eq!(version(&connection), CONTROL.version());
        assert_eq!(
            configs(&connection),
            [
                ("a".to_owned(), Some(b"a/anthropic:one".to_vec())),
                ("b".to_owned(), Some(b"b/anthropic:two".to_vec())),
                ("s".to_owned(), None),
            ]
        );

        let (_directory, path, mut connection) = released(&[("a", b"one"), ("b", b"unreadable")]);
        let before = configs(&connection);
        let refused = CONTROL.apply_sealed(&mut connection, &path, &Prefixed);
        assert!(matches!(refused, Err(StorageError::Migration { .. })), "{refused:?}");
        assert_eq!(
            version(&connection),
            version_of(include_str!("schema/control-0.1.22-dev.profile-contexts.sql"))
        );
        assert_eq!(configs(&connection), before);
    }

    /// The attached hosts of a database that stored their names lose them
    /// and keep the order those names gave them, and a Fork's attached hosts
    /// read without theirs.
    /// Planted defect this catches: a Fork reserved before the upgrade whose
    /// metadata still names its hosts, which no longer decodes, so its
    /// destination would never be published.
    #[test]
    fn attached_hosts_lose_the_names_conversations_gave_them() {
        use crate::conversation_index::AttachedHostRecord;
        use crate::forks::ForkMetadata;
        use demi_web_api_protocol::conversations::ConversationTarget;
        use demi_web_api_protocol::ids::DeviceId;

        let shipped = include_str!("schema/control-0.1.22-dev.saved-edits.sql");
        let (_directory, path, mut connection) = database(shipped);
        connection.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
        // Attached at one time, the second written first: its name put it
        // second.
        connection
            .execute_batch(
                "INSERT INTO conversation_hosts (conversation_id, device_id, name, cwd, attached_at)
                   VALUES ('c1', 'd2', 'laptop-2', '/work', 5), ('c1', 'd1', 'laptop', NULL, 5);
                 INSERT INTO conversations (id, user_id, title, title_origin, archived, pinned, sort_order,
                   read_revision, target_kind, last_switch, context_version, created_at, updated_at)
                   VALUES ('c1', 'u1', 'T', 'user', 0, 0, 0, 0, 'cloud', NULL, 7, 0, 0);",
            )
            .unwrap();
        let metadata = ForkMetadata {
            title: "Fork".to_owned().into(),
            target: ConversationTarget::Cloud { path: None },
            model: None,
            // A Fork reserved before its workflow was kept starts with Global Workflow.
            workflow: demi_shared_types::WorkflowChoice::Default,
            created_at: "2026-10-14T08:00:00Z".parse().unwrap(),
            attached_hosts: vec![AttachedHostRecord {
                device: DeviceId::try_from("d1".to_owned()).unwrap(),
                cwd: None,
            }],
            // Its source's, which waits for it.
            context_version: 7,
        };
        let mut stored = serde_json::to_value(&metadata).unwrap();
        stored["attachedHosts"][0]["name"] = "laptop".into();
        stored.as_object_mut().unwrap().remove("contextVersion");
        stored.as_object_mut().unwrap().remove("workflow");
        connection
            .execute(
                "INSERT INTO conversation_fork_operations (id, user_id, source_id, block_id, metadata)
                 VALUES ('f1', 'u1', 'c1', 'b1', ?1)",
                [stored.to_string()],
            )
            .unwrap();
        CONTROL.apply(&mut connection, &path, &NoBlobs).unwrap();
        let order: Vec<String> = connection
            .prepare("SELECT device_id FROM conversation_hosts ORDER BY attached_at, rowid")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(order, ["d1", "d2"]);
        let text: String = connection
            .query_row("SELECT metadata FROM conversation_fork_operations", [], |row| row.get(0))
            .unwrap();
        let migrated: ForkMetadata = json("conversation_fork_operations", "metadata", &text).unwrap();
        assert_eq!(migrated, metadata);
    }

    /// A conversation's attached hosts seed the hosts it may reach, which
    /// replace Manage Devices: its grants go, and so does an undecided
    /// request for it, while a decided one waits for its message.
    /// Planted defect this catches: a migration that seeded nothing, after
    /// which a conversation could no longer attach again a device it had
    /// attached, or move back to the host it had left, without asking.
    #[test]
    fn attached_hosts_seed_the_hosts_a_conversation_may_reach_and_manage_devices_goes() {
        let shipped = include_str!("schema/control-0.1.22-dev.interrupted-turns.sql");
        let (_directory, path, mut connection) = database(shipped);
        connection.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
        connection
            .execute_batch(
                "INSERT INTO conversation_hosts (conversation_id, device_id, cwd, attached_at)
                   VALUES ('c1', 'd1', NULL, 5), ('c1', 'd2', '/work', 6), ('c2', 'd1', NULL, 7);
                 INSERT INTO permission_grants (conversation_id, category, granted_at)
                   VALUES ('c1', 'host.devices', 1), ('c1', 'conversation.organize', 1);
                 INSERT INTO permission_requests (id, conversation_id, kind, categories, command, node_id, created_at, decision, decided_at)
                   VALUES ('r1', 'c2', 'permission', '[\"conversation.organize\",\"host.devices\"]', 'demi conversation move a', 'c2', 1, NULL, NULL),
                          ('r2', 'c2', 'permission', '[\"host.devices\"]', 'demi host attach b', 'c2', 1, 'allowed', 2),
                          ('r3', 'c2', 'permission', '[\"skills.manage\"]', 'demi skills add c', 'c2', 1, NULL, NULL);",
            )
            .unwrap();
        CONTROL.apply(&mut connection, &path, &NoBlobs).unwrap();
        let rows = |sql: &str| -> Vec<String> {
            connection
                .prepare(sql)
                .unwrap()
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };
        assert_eq!(
            rows("SELECT conversation_id || ' ' || device_id || ' ' || added_at FROM reachable_hosts ORDER BY 1"),
            ["c1 d1 5", "c1 d2 6", "c2 d1 7"]
        );
        assert_eq!(rows("SELECT category FROM permission_grants"), ["conversation.organize"]);
        assert_eq!(rows("SELECT id FROM permission_requests ORDER BY id"), ["r2", "r3"]);
    }

    /// A switch announced before the upgrade is not announced again: a node
    /// that read it at revision 3, then idled while runner reports moved the
    /// conversation to 7, counts it as seen after the upgrade, since the
    /// switch's revision is one its latest block reached.
    /// Planted defect this catches: the switch recorded at the revision the
    /// conversation stood at, 7, which the node's block of 3 does not reach,
    /// so its next context announced the old switch again.
    #[tokio::test]
    async fn a_switch_announced_before_the_upgrade_is_not_announced_again() {
        use crate::conversation_index::{ExecutionTarget, TargetSwitch};
        use demi_web_api_protocol::ids::{ConversationId, DeviceId};

        const ID: &str = "0b7a1c2d-3e4f-4a5b-8c6d-7e8f9a0b1c2d";
        let shipped = include_str!("schema/control-0.1.22-dev.saved-edits.sql");
        let (directory, path, mut connection) = database(shipped);
        connection.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
        let device = |id: &str| DeviceId::try_from(id.to_owned()).unwrap();
        let switch = TargetSwitch {
            from: ExecutionTarget::Device {
                device_id: device("d1"),
                path: "/a".into(),
            },
            to: ExecutionTarget::Device {
                device_id: device("d2"),
                path: "/b".into(),
            },
        };
        connection
            .execute(
                "INSERT INTO conversations (id, user_id, title, title_origin, archived, pinned, sort_order,
                   read_revision, target_kind, target_device_id, target_path, last_switch, context_version,
                   created_at, updated_at)
                 VALUES (?1, 'u1', 'T', 'user', 0, 0, 0, 0, 'device', 'd2', '/b', ?2, 7, 0, 0)",
                [ID, &to_json(&switch)],
            )
            .unwrap();
        CONTROL.apply(&mut connection, &path, &NoBlobs).unwrap();
        drop(connection);

        let control = crate::control::ControlService::open(
            &path,
            std::sync::Arc::new(demi_shared_types::SystemClock),
            std::sync::Arc::new(crate::providers::NoSealedConfigs),
        )
        .await
        .unwrap();
        let (kept, revision) = control
            .last_switch(ConversationId::try_from(ID).unwrap())
            .await
            .unwrap()
            .expect("the switch is kept");
        assert_eq!(kept, switch);
        let announced_at = 3;
        assert!(revision <= announced_at, "{revision}");
        control.close().await.unwrap();
        drop(directory);
    }

    /// A Fork's metadata that is not of the format before, such as one
    /// without its attached hosts, stops the migration and leaves the
    /// database as it was, rather than being read as one without hosts.
    #[test]
    fn a_fork_record_of_no_format_stops_the_migration() {
        let shipped = include_str!("schema/control-0.1.22-dev.saved-edits.sql");
        let (_directory, path, mut connection) = database(shipped);
        connection.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
        let target = demi_web_api_protocol::conversations::ConversationTarget::Cloud { path: None };
        let stored = serde_json::json!({
            "title": "Fork", "target": target, "model": null,
            "createdAt": "2026-10-14T08:00:00.000Z"
        });
        connection
            .execute(
                "INSERT INTO conversation_fork_operations (id, user_id, source_id, block_id, metadata)
                 VALUES ('f1', 'u1', 'c1', 'b1', ?1)",
                [stored.to_string()],
            )
            .unwrap();
        assert!(CONTROL.apply(&mut connection, &path, &NoBlobs).is_err());
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, i64::from(version_of(shipped)));
    }

    #[test]
    fn a_conversation_of_0_1_21_loses_its_wakeups() {
        use crate::columns::json;

        let shipped = CONVERSATION.history[2].sql;
        let insert = "INSERT INTO nodes (id, number, parent_id, description, round, started_at, can_spawn,
                        delivered, state, block_count, output_revision, wakeup_at)
                      VALUES (?1, ?2, ?3, '', 1, 0, 1, 0, ?4, 0, 0, ?5)";
        let wakeup = serde_json::json!([{"id": "w1", "durationMs": 60000, "dueAt": null}]);
        let (_directory, path, mut connection) = database(shipped);
        connection
            .execute(
                insert,
                rusqlite::params!["root", 0, None::<String>, state_of_0_1_21(serde_json::json!([])), None::<i64>],
            )
            .unwrap();
        connection
            .execute(insert, rusqlite::params!["waiting", 1, "root", state_of_0_1_21(wakeup), 0])
            .unwrap();

        CONVERSATION.apply(&mut connection, &path, &NoBlobs).unwrap();
        for id in ["root", "waiting"] {
            let state: String = connection
                .query_row("SELECT state FROM nodes WHERE id = ?1", [id], |row| row.get(0))
                .unwrap();
            let state = json::<CheckpointState>("nodes", "state", &state).unwrap();
            assert!(state.reports.is_empty() && state.intervals.is_empty(), "{id}");
        }

        let (_corrupt_directory, corrupt_path, mut corrupt) = database(shipped);
        corrupt
            .execute(
                insert,
                rusqlite::params!["root", 0, None::<String>, "{\"phase\":\"idle\"}", None::<i64>],
            )
            .unwrap();
        let refused = CONVERSATION.apply(&mut corrupt, &corrupt_path, &NoBlobs);
        assert!(matches!(refused, Err(StorageError::Migration { .. })), "{refused:?}");
    }

    /// A child's working directory moves from its checkpoint state into its
    /// row, and the root's goes, since the root works in the conversation's
    /// directory; each state reads as the current format.
    #[test]
    fn a_childs_directory_moves_from_its_state_into_its_row() {
        use crate::columns::json;

        let shipped = include_str!("schema/conversation-0.1.22-dev.forked-from.sql");
        let insert = "INSERT INTO nodes (id, number, parent_id, description, round, started_at, can_spawn,
                        delivered, state, block_count, output_revision)
                      VALUES (?1, ?2, ?3, '', 1, 0, 1, 0, ?4, 0, 0)";
        let state = |cwd: &str| {
            serde_json::json!({
                "phase": "idle", "queue": [], "agentInputs": [], "reports": [], "intervals": [],
                "cwd": cwd, "model": demi_agent_store::testing::test_model(), "edits": []
            })
            .to_string()
        };
        let (_directory, path, mut connection) = database(shipped);
        connection
            .execute(insert, rusqlite::params!["root", 0, None::<String>, state("/work")])
            .unwrap();
        connection
            .execute(insert, rusqlite::params!["child", 1, "root", state("/work/app")])
            .unwrap();

        CONVERSATION.apply(&mut connection, &path, &NoBlobs).unwrap();
        for (id, cwd) in [("root", None), ("child", Some("/work/app"))] {
            let (state, kept): (String, Option<String>) = connection
                .query_row("SELECT state, cwd FROM nodes WHERE id = ?1", [id], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })
                .unwrap();
            assert_eq!(kept.as_deref(), cwd, "{id}");
            json::<CheckpointState>("nodes", "state", &state).unwrap();
        }
    }

    /// 0.1.21's shell sequence goes with its row; a stored `shell_exec`
    /// call is a `shell` call with its input, its shell view naming no
    /// shell and its repeat guard's view `repeated_shell`; a `yield` and a
    /// `shell_status` call keep their names, and the `yield` loses its
    /// view; a wakeup a command's end fired is that command's report, titled
    /// by the call that started it, and one its time fired a resume. Every
    /// other number keeps its next value.
    #[test]
    fn a_conversation_of_0_1_21_becomes_one_of_the_shell_tool() {
        use demi_shared_types::{Block, CommandId, CommandReport, ReportEvent, Sequence, ToolView};

        use crate::sequences;

        let shipped = include_str!("schema/conversation-0.1.21.sql");
        let model = serde_json::json!({
            "providerId": "anthropic",
            "model": {
                "id": "claude-sonnet-4-5",
                "name": "Claude Sonnet 4.5",
                "contextWindow": 200000,
                "outputLimit": 64000,
                "thinking": [{"type": "disabled"}],
                "acceptedExtensions": ["png"]
            },
            "thinking": {"type": "disabled"},
            "serviceTierId": null
        });
        let call = |id: &str, tool: &str, input: &str, view: serde_json::Value| {
            serde_json::json!({
                "type": "tool_call",
                "id": id,
                "createdAt": "2026-09-21T14:13:20.000Z",
                "model": model,
                "toolUseId": format!("toolu_{id}"),
                "toolName": tool,
                "input": input,
                "status": "completed",
                "output": [{"type": "text", "text": "done"}],
                "view": view
            })
        };
        let shell_view = serde_json::json!({
            "kind": "shell",
            "status": "running",
            "commandId": "17",
            "runningMs": 60000,
            "idleMs": 2000,
            "chunks": [],
            "viewTruncated": false,
            "shellId": "3"
        });
        let exec_input = r#"{"script":"npm test","description":"Run the tests","timeoutMs":600000}"#;
        let blocks = [
            call("c1", "shell_exec", exec_input, shell_view),
            call(
                "c2",
                "shell_exec",
                exec_input,
                serde_json::json!({"kind": "repeated_shell_exec", "script": "npm test", "count": 7}),
            ),
            call(
                "c3",
                "yield",
                r#"{"durationMs":600000,"commandIds":[17]}"#,
                serde_json::json!({"kind": "yield_wakeup", "wakeupId": "w1", "durationMs": 600000, "commandIds": ["17"]}),
            ),
            call("c4", "shell_status", r#"{"commandId":17}"#, serde_json::Value::Null),
            serde_json::json!({
                "type": "wakeup",
                "id": "w1",
                "turnId": "t1",
                "createdAt": "2026-09-21T14:13:20.000Z",
                "model": model,
                "placement": "new_turn",
                "command": {"commandId": "17", "end": {"kind": "exited", "exitCode": 1}}
            }),
            serde_json::json!({
                "type": "wakeup",
                "id": "w2",
                "turnId": "t2",
                "createdAt": "2026-09-21T14:13:20.000Z",
                "model": model,
                "placement": "steer"
            }),
        ];
        let (_directory, path, mut connection) = database(shipped);
        connection
            .execute_batch(
                "INSERT INTO sequences (name, next) VALUES ('command', 18), ('shell', 4), ('agent', 2);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO nodes
                   (id, number, description, round, started_at, can_spawn, delivered, state, block_count, output_revision)
                 VALUES ('root', 0, '', 1, 0, 1, 0, ?1, 6, 0)",
                [state_of_0_1_21(serde_json::json!([]))],
            )
            .unwrap();
        for (index, block) in blocks.iter().enumerate() {
            connection
                .execute(
                    "INSERT INTO blocks (node_id, idx, block) VALUES ('root', ?1, ?2)",
                    rusqlite::params![i64::try_from(index).unwrap(), block.to_string()],
                )
                .unwrap();
        }

        CONVERSATION.apply(&mut connection, &path, &NoBlobs).unwrap();
        let stored: Vec<Block> = connection
            .prepare("SELECT block FROM blocks WHERE node_id = 'root' ORDER BY idx")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|text| demi_shared_types::decode::<Block>(&text.unwrap()).unwrap())
            .collect();
        let calls: Vec<(&str, &str, Option<&ToolView>)> = stored
            .iter()
            .filter_map(|block| match block {
                Block::ToolCall(call) => {
                    Some((call.tool_name.as_str(), call.input.as_str(), call.view.as_ref()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(calls[0].0, "shell");
        assert_eq!(calls[0].1, exec_input);
        assert!(matches!(calls[0].2, Some(ToolView::Shell(view)) if view.command_id.as_str() == "17"));
        assert_eq!(
            calls[1].2,
            Some(&ToolView::RepeatedShell {
                script: "npm test".into(),
                count: 7
            })
        );
        assert_eq!((calls[2].0, calls[2].2), ("yield", None));
        assert_eq!((calls[3].0, calls[3].2), ("shell_status", None));
        // The wakeup command 17's end fired is its report, titled by the
        // call that started it; the one its time fired is a resume.
        match &stored[4] {
            Block::Wakeup(wakeup) => assert_eq!(
                wakeup.reports,
                [CommandReport {
                    command_id: CommandId::try_from("17").unwrap(),
                    title: "Run the tests".to_owned(),
                    event: ReportEvent::Ended { exit_code: Some(1) },
                    output: String::new(),
                    media: Vec::new(),
                }]
            ),
            other => panic!("{other:?}"),
        }
        assert!(matches!(&stored[5], Block::Resume(resume) if resume.turn_id.as_str() == "t2"));
        assert_eq!(
            sequences::all(&connection).unwrap(),
            vec![(Sequence::Agent, 2), (Sequence::Command, 18)]
        );
        assert!(
            connection
                .execute("INSERT INTO sequences (name, next) VALUES ('shell', 1)", [])
                .is_err()
        );
    }

    /// The edit copies of a conversation, by their blobs; a read of one it
    /// was told to fail on fails.
    struct Copies {
        texts: HashMap<BlobRef, Bytes>,
        failing: Option<BlobRef>,
    }

    impl ReadBlob for Copies {
        fn read(&self, blob: &BlobRef) -> Result<Option<Bytes>, String> {
            if self.failing.as_ref() == Some(blob) {
                return Err("the object store is unreachable".into());
            }
            Ok(self.texts.get(blob).cloned())
        }
    }

    /// The shell calls of 0.1.21 and of the development build a server
    /// keeps data of each get their run and its files, and the files of
    /// their request, counted from the edit copies in the owner's
    /// namespace: two calls of one run that wrote `a.txt` list it once,
    /// counted from before the first to after the second, and a file a call
    /// removed is listed deleted. A copy that cannot be read stops the
    /// migration, which leaves the database as it was for the next open to
    /// try again.
    // Cost: four databases on disk, a few milliseconds.
    #[test]
    fn stored_shell_calls_get_their_lists_from_their_edit_copies() {
        use demi_shared_types::{Block, ChangedFile, EditKind, LineCounts, ToolView};

        let model = serde_json::json!({
            "providerId": "anthropic",
            "model": {
                "id": "claude-sonnet-4-5",
                "name": "Claude Sonnet 4.5",
                "contextWindow": 200000,
                "outputLimit": 64000,
                "thinking": [{"type": "disabled"}],
                "acceptedExtensions": ["png"]
            },
            "thinking": {"type": "disabled"},
            "serviceTierId": null
        });
        let texts: HashMap<BlobRef, Bytes> = ["", "one\n", "one\ntwo\n", "old\nguide\n"]
            .into_iter()
            .map(|text| (BlobRef::of(text.as_bytes()), Bytes::from(text)))
            .collect();
        let blob = |text: &str| BlobRef::of(text.as_bytes()).to_string();
        // 0.1.21 named the tool `shell_exec` and its view a shell; the
        // development build's calls are `shell` calls, which list removals.
        let sql_of = |name: &str| CONVERSATION.history.iter().position(|shipped| shipped.sql == name).unwrap();
        let release = sql_of(include_str!("schema/conversation-0.1.21.sql"));
        let development = sql_of(include_str!("schema/conversation-0.1.22-dev.20261011T064359Z.sql"));
        for shipped in [release, development] {
            let dev = shipped == development;
            let call = |id: &str, command: &str, files: serde_json::Value, changes: serde_json::Value| {
                let mut view = serde_json::json!({
                    "kind": "shell",
                    "status": "exited",
                    "commandId": command,
                    "exitCode": 0,
                    "runningMs": 10,
                    "idleMs": 0,
                    "chunks": [],
                    "viewTruncated": false,
                    "files": files,
                    "filesTruncated": false
                });
                if dev {
                    view["pathChanges"] = changes;
                } else {
                    view["shellId"] = "3".into();
                }
                serde_json::json!({
                    "type": "tool_call",
                    "id": id,
                    "createdAt": "2026-09-21T14:13:20.000Z",
                    "model": model,
                    "toolUseId": format!("toolu_{id}"),
                    "toolName": if dev { "shell" } else { "shell_exec" },
                    "input": r#"{"script":"echo two >> a.txt","description":"Write the notes"}"#,
                    "status": "completed",
                    "output": [{"type": "text", "text": "done"}],
                    "view": view
                })
            };
            let wrote = |kind: &str, original: &str, modified: &str| {
                serde_json::json!([{
                    "path": "/w/a.txt",
                    "kind": kind,
                    "added": 1,
                    "removed": 0,
                    "edits": [{"copies": {"original": blob(original), "modified": blob(modified)}}]
                }])
            };
            let mut blocks = vec![
                serde_json::json!({
                    "type": "user",
                    "id": "u1",
                    "turnId": "t1",
                    "createdAt": "2026-09-21T14:13:20.000Z",
                    "model": model,
                    "content": [{"type": "text", "text": "Write the notes"}],
                    "preamble": null
                }),
                call("c1", "1", wrote("added", "", "one\n"), serde_json::json!([])),
                call("c2", "2", wrote("modified", "one\n", "one\ntwo\n"), serde_json::json!([])),
            ];
            if dev {
                blocks.push(call(
                    "c3",
                    "3",
                    serde_json::json!([]),
                    serde_json::json!([{"kind": "removed", "path": "/w/guide.md"}]),
                ));
            }
            let state = if dev {
                state_of_development()
            } else {
                state_of_0_1_21(serde_json::json!([]))
            };
            let sql = CONVERSATION.history[shipped].sql;
            let conversation = || {
                let (directory, path, connection) = database(sql);
                connection
                    .execute(
                        "INSERT INTO nodes
                           (id, number, description, round, started_at, can_spawn, delivered, state, block_count, output_revision)
                         VALUES ('root', 0, '', 1, 0, 1, 0, ?1, ?2, 0)",
                        rusqlite::params![state, i64::try_from(blocks.len()).unwrap()],
                    )
                    .unwrap();
                for (index, block) in blocks.iter().enumerate() {
                    connection
                        .execute(
                            "INSERT INTO blocks (node_id, idx, block) VALUES ('root', ?1, ?2)",
                            rusqlite::params![i64::try_from(index).unwrap(), block.to_string()],
                        )
                        .unwrap();
                }
                (directory, path, connection)
            };

            let (_directory, path, mut connection) = conversation();
            let copies = Copies { texts: texts.clone(), failing: None };
            CONVERSATION.apply(&mut connection, &path, &copies).unwrap();
            let stored: Vec<Block> = connection
                .prepare("SELECT block FROM blocks WHERE node_id = 'root' ORDER BY idx")
                .unwrap()
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .map(|text| demi_shared_types::decode::<Block>(&text.unwrap()).unwrap())
                .collect();
            let Block::ToolCall(latest) = stored.last().unwrap() else {
                panic!("{stored:?}");
            };
            assert_eq!(latest.run.as_ref().map(|run| run.as_str()), Some("c1"), "{sql:.40}");
            let Some(ToolView::Shell(view)) = &latest.view else {
                panic!("{latest:?}");
            };
            let listed = |files: &Option<Vec<ChangedFile>>| {
                files
                    .iter()
                    .flatten()
                    .map(|file| (file.path.clone(), file.kind, file.lines))
                    .collect::<Vec<_>>()
            };
            let mut expected = vec![(
                "/w/a.txt".to_owned(),
                EditKind::Added,
                Some(LineCounts { added: 2, removed: 0 }),
            )];
            if dev {
                expected.push(("/w/guide.md".to_owned(), EditKind::Deleted, None));
                let own: Vec<_> = view.files.iter().flatten().map(|file| (file.path.as_str(), file.kind)).collect();
                assert_eq!(own, [("/w/guide.md", EditKind::Deleted)]);
            }
            assert_eq!(listed(&view.run_files), expected, "{sql:.40}");
            assert_eq!(listed(&view.request_files), expected, "{sql:.40}");

            let (_failed_directory, failed_path, mut failed) = conversation();
            let unreachable = Copies {
                texts: texts.clone(),
                failing: Some(BlobRef::of(b"one\ntwo\n")),
            };
            let refused = CONVERSATION.apply(&mut failed, &failed_path, &unreachable);
            assert!(matches!(refused, Err(StorageError::Migration { .. })), "{refused:?}");
            let version: i32 = failed.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
            assert_eq!(version, version_of(sql));
        }
    }

    /// The checkpoint state of an idle agent, as the development builds
    /// store it: command reports and intervals in place of 0.1.21's wakeups.
    fn state_of_development() -> String {
        let mut state: serde_json::Value = serde_json::from_str(&state_of_0_1_21(serde_json::json!([]))).unwrap();
        let fields = state.as_object_mut().unwrap();
        fields.remove("wakeups");
        fields.insert("reports".into(), serde_json::json!([]));
        fields.insert("intervals".into(), serde_json::json!([]));
        state.to_string()
    }

    /// A conversation 0.1.22-dev.20261011T123543Z stored with its runs
    /// split at a `resume` block, as after a restart: a call wrote `a.txt`
    /// and was lost, the turn resumed, and the next call wrote it again.
    /// Migrated, the second call joins the first's run, and its run's files
    /// list `a.txt` once, counted from before the first call to after the
    /// second.
    // Cost: one database on disk, a few milliseconds.
    #[test]
    fn a_run_stored_split_at_a_resume_is_joined() {
        use demi_shared_types::{Block, ChangedFile, EditKind, LineCounts, ToolView};

        let model = serde_json::json!({
            "providerId": "anthropic",
            "model": {
                "id": "claude-sonnet-4-5",
                "name": "Claude Sonnet 4.5",
                "contextWindow": 200000,
                "outputLimit": 64000,
                "thinking": [{"type": "disabled"}],
                "acceptedExtensions": ["png"]
            },
            "thinking": {"type": "disabled"},
            "serviceTierId": null
        });
        let texts: HashMap<BlobRef, Bytes> = ["", "one\n", "one\ntwo\n"]
            .into_iter()
            .map(|text| (BlobRef::of(text.as_bytes()), Bytes::from(text)))
            .collect();
        let copies = |original: &str, modified: &str| {
            serde_json::json!({
                "original": BlobRef::of(original.as_bytes()).to_string(),
                "modified": BlobRef::of(modified.as_bytes()).to_string()
            })
        };
        let own = |kind: &str, original: &str, modified: &str| {
            serde_json::json!({
                "path": "/w/a.txt",
                "kind": kind,
                "added": 1,
                "removed": 0,
                "edits": [{"copies": copies(original, modified)}]
            })
        };
        // As the build listed each call's files: its run and its request
        // start at the call itself.
        let listed = |kind: &str, call: &str, original: &str, modified: &str| {
            serde_json::json!({
                "path": "/w/a.txt",
                "kind": kind,
                "lines": {"added": 1, "removed": 0},
                "edits": [{
                    "call": call,
                    "path": "/w/a.txt",
                    "segment": 0,
                    "created": kind == "added",
                    "copies": copies(original, modified)
                }]
            })
        };
        let call = |id: &str, command: &str, kind: &str, original: &str, modified: &str| {
            serde_json::json!({
                "type": "tool_call",
                "id": id,
                "createdAt": "2026-10-11T12:40:00.000Z",
                "model": model,
                "run": id,
                "toolUseId": format!("toolu_{id}"),
                "toolName": "shell",
                "input": r#"{"script":"echo more >> a.txt","description":"Write the notes"}"#,
                "status": "completed",
                "output": [{"type": "text", "text": "done"}],
                "view": {
                    "kind": "shell",
                    "status": "exited",
                    "commandId": command,
                    "exitCode": 0,
                    "runningMs": 10,
                    "idleMs": 0,
                    "chunks": [],
                    "viewTruncated": false,
                    "files": [own(kind, original, modified)],
                    "filesTruncated": false,
                    "pathChanges": [],
                    "runFiles": [listed(kind, command, original, modified)],
                    "requestFiles": [listed(kind, command, original, modified)]
                }
            })
        };
        let blocks = [
            serde_json::json!({
                "type": "user",
                "id": "u1",
                "turnId": "t1",
                "createdAt": "2026-10-11T12:40:00.000Z",
                "model": model,
                "content": [{"type": "text", "text": "Write the notes"}],
                "preamble": null
            }),
            call("c1", "1", "added", "", "one\n"),
            serde_json::json!({
                "type": "resume",
                "id": "r1",
                "turnId": "t1",
                "createdAt": "2026-10-11T12:41:00.000Z",
                "model": model,
                "reason": "The backend restarted."
            }),
            call("c2", "2", "modified", "one\n", "one\ntwo\n"),
        ];
        let sql = include_str!("schema/conversation-0.1.22-dev.20261011T123543Z.sql");
        let (_directory, path, mut connection) = database(sql);
        connection
            .execute(
                "INSERT INTO nodes
                   (id, number, description, round, started_at, can_spawn, delivered, state, block_count, output_revision)
                 VALUES ('root', 0, '', 1, 0, 1, 0, ?1, ?2, 0)",
                rusqlite::params![state_of_development(), i64::try_from(blocks.len()).unwrap()],
            )
            .unwrap();
        for (index, block) in blocks.iter().enumerate() {
            // Each is a block of format 3 as the build wrote it, which reads
            // as today's once the later formats' fields are added.
            let mut current = block.clone();
            with_its_end(&mut current).unwrap();
            demi_shared_types::decode_value::<Block>(current).unwrap();
            connection
                .execute(
                    "INSERT INTO blocks (node_id, idx, block) VALUES ('root', ?1, ?2)",
                    rusqlite::params![i64::try_from(index).unwrap(), block.to_string()],
                )
                .unwrap();
        }

        let copies = Copies { texts, failing: None };
        CONVERSATION.apply(&mut connection, &path, &copies).unwrap();
        let stored: Vec<Block> = connection
            .prepare("SELECT block FROM blocks WHERE node_id = 'root' ORDER BY idx")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|text| demi_shared_types::decode::<Block>(&text.unwrap()).unwrap())
            .collect();
        let Block::ToolCall(latest) = stored.last().unwrap() else {
            panic!("{stored:?}");
        };
        assert_eq!(latest.run.as_ref().map(|run| run.as_str()), Some("c1"));
        let Some(ToolView::Shell(view)) = &latest.view else {
            panic!("{latest:?}");
        };
        let listed = |files: &Option<Vec<ChangedFile>>| {
            files
                .iter()
                .flatten()
                .map(|file| (file.path.clone(), file.kind, file.lines))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            listed(&view.run_files),
            [("/w/a.txt".to_owned(), EditKind::Added, Some(LineCounts { added: 2, removed: 0 }))]
        );
    }

    /// A compaction stored before boundaries kept the user's messages, by
    /// the build the server runs (format 3) and by a development build of
    /// format 4, keeps none once migrated, and its conversation opens.
    // Cost: two databases on disk, a few milliseconds.
    #[test]
    fn a_boundary_stored_before_kept_messages_keeps_none() {
        use demi_shared_types::Block;

        let model = serde_json::json!({
            "providerId": "anthropic",
            "model": {
                "id": "claude-sonnet-4-5",
                "name": "Claude Sonnet 4.5",
                "contextWindow": 200000,
                "outputLimit": 64000,
                "thinking": [{"type": "disabled"}],
                "acceptedExtensions": ["png"]
            },
            "thinking": {"type": "disabled"},
            "serviceTierId": null
        });
        let blocks = [
            serde_json::json!({
                "type": "compaction_boundary",
                "id": "b1",
                "createdAt": "2026-10-11T12:40:00.000Z",
                "model": model,
                "summary": "The user asked for notes.",
                "summaryTokens": 7
            }),
            serde_json::json!({
                "type": "user",
                "id": "u1",
                "turnId": "t1",
                "createdAt": "2026-10-11T12:41:00.000Z",
                "model": model,
                "content": [{"type": "text", "text": "Go on"}],
                "preamble": null
            }),
            serde_json::json!({
                "type": "compaction_marker",
                "id": "m1",
                "createdAt": "2026-10-11T12:41:00.000Z",
                "model": model,
                "boundaryId": "b1",
                "compactedTokens": 900
            }),
        ];
        for sql in [
            include_str!("schema/conversation-0.1.22-dev.20261011T123543Z.sql"),
            include_str!("schema/conversation-0.1.22-dev.block-format-4.sql"),
        ] {
            let (_directory, path, mut connection) = database(sql);
            connection
                .execute(
                    "INSERT INTO nodes
                       (id, number, description, round, started_at, can_spawn, delivered, state, block_count, output_revision)
                     VALUES ('root', 0, '', 1, 0, 1, 0, ?1, ?2, 0)",
                    rusqlite::params![state_of_development(), i64::try_from(blocks.len()).unwrap()],
                )
                .unwrap();
            for (index, block) in blocks.iter().enumerate() {
                connection
                    .execute(
                        "INSERT INTO blocks (node_id, idx, block) VALUES ('root', ?1, ?2)",
                        rusqlite::params![i64::try_from(index).unwrap(), block.to_string()],
                    )
                    .unwrap();
            }

            CONVERSATION.apply(&mut connection, &path, &NoBlobs).unwrap();
            let stored: Vec<Block> = connection
                .prepare("SELECT block FROM blocks WHERE node_id = 'root' ORDER BY idx")
                .unwrap()
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .map(|text| demi_shared_types::decode::<Block>(&text.unwrap()).unwrap())
                .collect();
            let Block::CompactionBoundary(boundary) = &stored[0] else {
                panic!("{stored:?}");
            };
            assert_eq!(boundary.summary, "The user asked for notes.");
            assert!(boundary.kept.is_empty());
        }
    }

    /// Calls stored in transcript block format 8, by the schema that
    /// `0.1.22-dev.20261015T105826Z` deployed, kept no record of when they
    /// returned: migrated to format 9, a shell call ends the time its view
    /// says its command ran after it started, a call of another tool where
    /// it started, and a call still executing has not returned; a share's
    /// snapshot of a call is migrated the same way (`rendering.md` § Work
    /// groups, a run's time).
    // Cost: one database on disk, a few milliseconds.
    #[test]
    fn a_stored_call_gets_when_it_returned() {
        use demi_shared_types::Block;

        let model = serde_json::json!({
            "providerId": "anthropic",
            "model": {
                "id": "claude-opus-4-8",
                "name": "Claude Opus 4.8",
                "contextWindow": 200000,
                "outputLimit": 64000,
                "thinking": [{"type": "disabled"}],
                "acceptedExtensions": ["png"]
            },
            "thinking": {"type": "disabled"},
            "serviceTierId": null
        });
        let call = |id: &str, tool: &str, status: &str, view: serde_json::Value| {
            serde_json::json!({
                "type": "tool_call",
                "id": id,
                "run": "c1",
                "createdAt": "2026-10-15T05:00:01.000Z",
                "model": model,
                "toolUseId": format!("toolu_{id}"),
                "toolName": tool,
                "input": "{}",
                "status": status,
                "output": [],
                "view": view
            })
        };
        let shell = serde_json::json!({
            "kind": "shell",
            "status": "exited",
            "commandId": "1",
            "exitCode": 0,
            "runningMs": 1500,
            "idleMs": 0,
            "chunks": [],
            "viewTruncated": false
        });
        let blocks = [
            call("c1", "shell", "completed", shell.clone()),
            call("c2", "browse", "error", serde_json::Value::Null),
            call("c3", "shell", "executing", serde_json::Value::Null),
        ];
        let (_directory, path, mut connection) =
            database(include_str!("schema/conversation-0.1.22-dev.20261015T105826Z.sql"));
        connection
            .execute(
                "INSERT INTO nodes
                   (id, number, description, round, started_at, can_spawn, delivered, state, block_count, output_revision)
                 VALUES ('root', 0, '', 1, 0, 1, 0, ?1, ?2, 0)",
                rusqlite::params![state_of_development(), i64::try_from(blocks.len()).unwrap()],
            )
            .unwrap();
        for (index, block) in blocks.iter().enumerate() {
            connection
                .execute(
                    "INSERT INTO blocks (node_id, idx, block) VALUES ('root', ?1, ?2)",
                    rusqlite::params![i64::try_from(index).unwrap(), block.to_string()],
                )
                .unwrap();
        }
        CONVERSATION.apply(&mut connection, &path, &NoBlobs).unwrap();
        let ends = |sql: &str| -> Vec<Option<String>> {
            connection
                .prepare(sql)
                .unwrap()
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .map(|text| match demi_shared_types::decode::<Block>(&text.unwrap()).unwrap() {
                    Block::ToolCall(call) => call.ended_at.map(|at| at.to_string()),
                    other => panic!("{other:?}"),
                })
                .collect()
        };
        assert_eq!(
            ends("SELECT block FROM blocks WHERE node_id = 'root' ORDER BY idx"),
            [Some("2026-10-15T05:00:02.500Z".to_owned()), Some("2026-10-15T05:00:01.000Z".to_owned()), None]
        );
    }

    /// The quota snapshots of 0.1.22-dev.20261015T134426Z's control schema
    /// named each window in English: migrated, each window has the kind its
    /// vendor's old namer meant by the name, and keeps its period and its
    /// scope. A name that namer never made, such as another vendor's, a
    /// count Codex did not write or no name at all, fails the migration
    /// instead of becoming a model's name.
    // Cost: eight databases on disk, a few milliseconds each.
    #[test]
    fn a_stored_snapshot_gets_its_windows_kinds() {
        use demi_shared_types::{QuotaKind, QuotaSnapshot};

        let window = |label: Option<&str>| {
            let mut window = serde_json::json!({
                "id": "w", "usedPercent": 40.0, "used": null, "limit": null,
                "unit": "percent", "resetsAt": null, "periodSeconds": 90, "severity": null,
                "scope": null
            });
            if let Some(label) = label {
                window["label"] = label.into();
            }
            window
        };
        let migrated = |vendor: &str, labels: &[Option<&str>]| -> Result<Vec<QuotaKind>, StorageError> {
            let (_directory, path, mut connection) =
                database(include_str!("schema/control-0.1.22-dev.20261015T134426Z.sql"));
            let quota = serde_json::json!({
                "observedAt": "2026-10-15T13:00:00.000Z", "source": "probe", "plan": null,
                "accountLabel": null, "windows": labels.iter().map(|label| window(*label)).collect::<Vec<_>>()
            });
            connection
                .execute_batch(
                    "INSERT INTO users (id, email, nickname, password_hash, role, created_at)
                       VALUES ('u1', 'a@example.test', 'Ana', 'hash', 'master', 1);",
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO providers (id, owner_user_id, provider_type, credential_kind, label, created_at)
                       VALUES ('p1', 'u1', ?1, 'subscription', 'Subscription', 1)",
                    [vendor],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO provider_credentials (provider_id, id, label, source, secret, version, quota, updated_at)
                       VALUES ('p1', 'a1', 'Work', 'login', x'00', 1, ?1, 1)",
                    [quota.to_string()],
                )
                .unwrap();
            CONTROL.apply(&mut connection, &path, &NoBlobs)?;
            let text: String =
                connection.query_row("SELECT quota FROM provider_credentials", [], |row| row.get(0)).unwrap();
            let snapshot: QuotaSnapshot = demi_shared_types::decode(&text).unwrap();
            assert!(snapshot.windows.iter().all(|window| window.period_seconds == Some(90)));
            Ok(snapshot.windows.iter().map(|window| window.kind).collect())
        };

        assert_eq!(
            migrated("codex", &[Some("5-hour"), Some("1.5-minute"), Some("Weekly"), Some("Primary")]).unwrap(),
            [QuotaKind::Limit; 4]
        );
        assert_eq!(
            migrated("grok-build", &[Some("Weekly credits"), Some("On-demand cap"), Some("Tokens (short window)")])
                .unwrap(),
            [QuotaKind::Credits, QuotaKind::Cap, QuotaKind::Tokens]
        );
        assert_eq!(
            migrated("claude-code", &[Some("5h session"), Some("7d Fable"), Some("Opus"), Some("Other limit")])
                .unwrap(),
            [QuotaKind::Limit; 4]
        );
        for (vendor, label) in [
            ("codex", Some("1.50-minute")),
            ("codex", Some("Opus")),
            ("grok-build", Some("5-hour")),
            ("claude-code", Some("")),
            ("codex", None),
        ] {
            assert!(migrated(vendor, &[label]).is_err(), "{vendor} {label:?}");
        }
    }

    /// An `account_switch` block of transcript block format 9 named the
    /// window that was used up in English: migrated to format 10, it holds
    /// the kind, the period and the model that name said, and one that
    /// named none still names none.
    // Cost: one database on disk, a few milliseconds.
    #[test]
    fn a_linear_transcript_becomes_one_path() {
        use demi_shared_types::{Block, TextBlock};

        let said = |text: &str| {
            Block::Text(TextBlock {
                id: text.try_into().unwrap(),
                created_at: demi_shared_types::Timestamp::UNIX_EPOCH,
                model: demi_agent_store::testing::test_model(),
                text: text.to_owned(),
                forkable: true,
                entries: Vec::new(),
            })
        };
        let blocks = [said("first"), said("second"), said("third")];
        let (_directory, path, mut connection) =
            database(include_str!("schema/conversation-0.1.22-dev.20261015T134426Z.sql"));
        for (id, number, parent, count) in [("root", 0, None, 3), ("child", 1, Some("root"), 0)] {
            connection
                .execute(
                    "INSERT INTO nodes
                       (id, number, parent_id, description, round, started_at, can_spawn, delivered, state, block_count,
                        output_revision, cwd, cwd_under)
                     VALUES (?1, ?2, ?3, '', 1, 0, 1, 0, ?4, ?5, 0, ?6, ?6)",
                    rusqlite::params![id, number, parent, state_of_development(), count, parent.map(|_| "/work")],
                )
                .unwrap();
        }
        for (index, block) in blocks.iter().enumerate() {
            connection
                .execute(
                    "INSERT INTO blocks (node_id, idx, block) VALUES ('root', ?1, ?2)",
                    rusqlite::params![i64::try_from(index).unwrap(), to_json(block)],
                )
                .unwrap();
        }

        CONVERSATION.apply(&mut connection, &path, &NoBlobs).unwrap();

        // Each row follows the one before it, the last is the leaf, and the
        // path reads as the transcript did.
        let rows: Vec<(i64, Option<i64>, Option<i64>)> = connection
            .prepare("SELECT idx, follows, replaces FROM blocks WHERE node_id = 'root' ORDER BY idx")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(rows, [(0, None, None), (1, Some(0), None), (2, Some(1), None)]);
        let leaves: Vec<(String, Option<i64>, Option<i64>)> = connection
            .prepare("SELECT id, leaf, cut FROM nodes ORDER BY number")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(leaves, [("root".to_owned(), Some(2), None), ("child".to_owned(), None, None)]);
        assert_eq!(crate::tree::root_blocks(&connection).unwrap(), blocks);
    }

    /// A command that ran under 0.1.22-dev.20261016T043704Z keeps the
    /// interval its call asked for, as that build bounded it, or none for a
    /// call that asked for none: a restart that interrupted its call
    /// watches it at that interval.
    // Cost: one database on disk, a few milliseconds.
    #[test]
    fn a_running_command_keeps_the_interval_its_call_asked_for() {
        use demi_shared_types::{Block, ToolCallBlock, ToolCallStatus};

        let call = |id: &str, interval: serde_json::Value| {
            Block::ToolCall(ToolCallBlock {
                id: id.try_into().unwrap(),
                created_at: demi_shared_types::Timestamp::UNIX_EPOCH,
                model: demi_agent_store::testing::test_model(),
                tool_use_id: id.to_owned(),
                tool_name: "shell".to_owned(),
                input: serde_json::json!({"script": "make", "description": "Build", "intervalMs": interval}).to_string(),
                status: ToolCallStatus::Executing,
                output: Vec::new(),
                view: None,
                ended_at: None,
                run: None,
                entries: Vec::new(),
            })
        };
        let (_directory, path, mut connection) =
            database(include_str!("schema/conversation-0.1.22-dev.20261016T043704Z.sql"));
        connection
            .execute(
                "INSERT INTO nodes
                   (id, number, parent_id, description, round, started_at, can_spawn, delivered, state,
                    output_revision, cwd, cwd_under, leaf)
                 VALUES ('root', 0, NULL, '', 1, 0, 1, 0, ?1, 0, NULL, NULL, 2)",
                [state_of_development()],
            )
            .unwrap();
        for (index, (id, interval)) in [("c1", 30_000.into()), ("c2", serde_json::Value::Null), ("c3", 100.into())]
            .into_iter()
            .enumerate()
        {
            connection
                .execute(
                    "INSERT INTO blocks (node_id, idx, block, follows) VALUES ('root', ?1, ?2, nullif(?1 - 1, -1))",
                    rusqlite::params![i64::try_from(index).unwrap(), to_json(&call(id, interval))],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO running_commands (command_id, node_id, device_id, job_id, tool_use_id, started_at, places, title)
                     VALUES (?1, 'root', 'device', ?1, ?2, 0, '{}', 'Build')",
                    rusqlite::params![format!("{}", index + 1), id],
                )
                .unwrap();
        }
        CONVERSATION.apply(&mut connection, &path, &NoBlobs).unwrap();

        let intervals: Vec<(String, Option<i64>)> = connection
            .prepare("SELECT command_id, interval_ms FROM running_commands ORDER BY command_id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            intervals,
            [("1".to_owned(), Some(30_000)), ("2".to_owned(), None), ("3".to_owned(), Some(15_000))]
        );
    }

    /// A command running when 0.1.22 stopped keeps its record, its title
    /// and interval included, with no turn,
    /// and a command recorded afterwards keeps its turn, which a backend
    /// that starts again hands the job it takes up.
    /// Planted defect this catches: a turn the record drops, so a command
    /// taken up after a restart counts its calls under no turn.
    /// One small database, milliseconds.
    #[test]
    fn a_running_command_keeps_its_turn_and_one_from_before_keeps_none() {
        use crate::running;

        let (_directory, path, mut connection) =
            database(include_str!("schema/conversation-0.1.22.sql"));
        connection
            .execute(
                "INSERT INTO nodes
                   (id, number, parent_id, description, round, started_at, can_spawn, delivered, state,
                    output_revision, cwd, cwd_under, leaf)
                 VALUES ('root', 0, NULL, '', 1, 0, 1, 0, ?1, 0, NULL, NULL, NULL)",
                [state_of_development()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO running_commands
                   (command_id, node_id, device_id, job_id, tool_use_id, started_at, places, title, interval_ms)
                 VALUES ('1', 'root', 'device', 'job-1', 'call-1', 7, '{}', 'Build', 2000)",
                [],
            )
            .unwrap();
        CONVERSATION.apply(&mut connection, &path, &NoBlobs).unwrap();
        let root = demi_shared_types::NodeId::try_from("root").unwrap();
        let mut later = running::of_node(&connection, &root).unwrap().remove(0);
        assert_eq!(
            (
                later.command.as_str(),
                later.device.as_str(),
                later.job.as_str(),
                later.tool_use_id.as_str(),
                later.started.as_millisecond(),
                later.title.as_str(),
                later.interval_ms,
                later.turn.as_ref(),
            ),
            ("1", "device", "job-1", "call-1", 7, "Build", Some(2000), None)
        );
        later.command = demi_shared_types::CommandId::try_from("2").unwrap();
        later.turn = Some(demi_shared_types::TurnId::try_from("m4").unwrap());
        assert_eq!(later.unsent, None);
        running::record(&connection, &later).unwrap();
        let turns: Vec<Option<String>> = running::of_node(&connection, &root)
            .unwrap()
            .into_iter()
            .map(|command| command.turn.map(|turn| turn.to_string()))
            .collect();
        assert_eq!(turns, [None, Some("m4".to_owned())]);
    }

    #[test]
    fn a_stored_account_switch_gets_its_windows_name() {
        use demi_shared_types::{Block, QuotaKind, QuotaScope, QuotaWindowName};

        let switch = |id: &str, window: serde_json::Value| {
            serde_json::json!({
                "type": "account_switch",
                "id": id,
                "createdAt": "2026-10-15T13:00:00.000Z",
                "model": {
                    "providerId": "anthropic",
                    "model": {
                        "id": "claude-opus-4-8",
                        "name": "Claude Opus 4.8",
                        "contextWindow": 200000,
                        "outputLimit": 64000,
                        "thinking": [{"type": "disabled"}],
                        "acceptedExtensions": ["png"]
                    },
                    "thinking": {"type": "disabled"},
                    "serviceTierId": null
                },
                "from": "alex@example.com",
                "to": "sam@example.com",
                "window": window,
                "resetsAt": null
            })
        };
        let blocks = [
            switch("b1", "5h session".into()),
            switch("b2", "7d Fable".into()),
            switch("b3", "Opus".into()),
            switch("b4", "3-day".into()),
            switch("b5", serde_json::Value::Null),
        ];
        let (_directory, path, mut connection) =
            database(include_str!("schema/conversation-0.1.22-dev.20261015T134426Z.sql"));
        connection
            .execute(
                "INSERT INTO nodes
                   (id, number, description, round, started_at, can_spawn, delivered, state, block_count, output_revision)
                 VALUES ('root', 0, '', 1, 0, 1, 0, ?1, ?2, 0)",
                rusqlite::params![state_of_development(), i64::try_from(blocks.len()).unwrap()],
            )
            .unwrap();
        for (index, block) in blocks.iter().enumerate() {
            connection
                .execute(
                    "INSERT INTO blocks (node_id, idx, block) VALUES ('root', ?1, ?2)",
                    rusqlite::params![i64::try_from(index).unwrap(), block.to_string()],
                )
                .unwrap();
        }
        CONVERSATION.apply(&mut connection, &path, &NoBlobs).unwrap();
        let names = |sql: &str| -> Vec<Option<QuotaWindowName>> {
            connection
                .prepare(sql)
                .unwrap()
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .map(|text| match demi_shared_types::decode::<Block>(&text.unwrap()).unwrap() {
                    Block::AccountSwitch(switch) => switch.window,
                    other => panic!("{other:?}"),
                })
                .collect()
        };
        let limit = |period_seconds, model: Option<&str>| {
            Some(QuotaWindowName {
                kind: QuotaKind::Limit,
                period_seconds,
                scope: model.map(|model| QuotaScope { kind: "model".into(), label: Some(model.into()) }),
            })
        };
        let fable = limit(Some(604_800), Some("Fable"));
        assert_eq!(
            names("SELECT block FROM blocks WHERE node_id = 'root' ORDER BY idx"),
            [limit(Some(18_000), None), fable.clone(), limit(None, Some("Opus")), limit(Some(259_200), None), None]
        );
        assert_eq!(names("SELECT block FROM share_blocks"), []);

        // A name of a Codex length's shape is one exactly, a fractional
        // count of a minute included, or the migration fails; it never
        // becomes a model's name.
        let migrate = |window: &str| {
            let (_directory, path, mut connection) =
                database(include_str!("schema/conversation-0.1.22-dev.20261015T134426Z.sql"));
            let block = switch("b1", window.into()).to_string();
            connection
                .execute(
                    "INSERT INTO nodes
                       (id, number, description, round, started_at, can_spawn, delivered, state, block_count, output_revision)
                     VALUES ('root', 0, '', 1, 0, 1, 0, ?1, 1, 0)",
                    [state_of_development()],
                )
                .unwrap();
            connection
                .execute("INSERT INTO blocks (node_id, idx, block) VALUES ('root', 0, ?1)", [&block])
                .unwrap();
            CONVERSATION.apply(&mut connection, &path, &NoBlobs)?;
            let text: String = connection.query_row("SELECT block FROM blocks", [], |row| row.get(0)).unwrap();
            match demi_shared_types::decode::<Block>(&text).unwrap() {
                Block::AccountSwitch(switch) => Ok::<_, StorageError>(switch.window),
                other => panic!("{other:?}"),
            }
        };
        assert_eq!(migrate("1.5-minute").unwrap(), limit(Some(90), None));
        for label in ["1.5x-minute", "1.50-minute", "-hour", ""] {
            assert!(migrate(label).is_err(), "{label:?}");
        }
    }

    /// The shares made before a share held only what was said, each a
    /// snapshot of every block and its subagents, are removed by the
    /// migration: their copies, subagents and attachments go, and their
    /// tokens read nothing. Every blob a stored block names is listed, so
    /// the collection of the user's namespace keeps it, and every attachment
    /// a stored text names, so a new share of the migrated database lists
    /// both.
    // Cost: one database on disk, a few milliseconds.
    #[test]
    fn a_share_made_before_is_removed_and_each_blob_stays_named() {
        use demi_shared_types::{Block, BlobRef};
        use demi_web_api_protocol::ids::ShareToken;

        use crate::tree::tests::{reply, response, state, user};
        use crate::tree::{self, PageQuery, PageRead, Transcript};

        let blob = BlobRef::try_from("b".repeat(64)).unwrap();
        let mut message = serde_json::to_value(user("u1")).unwrap();
        message["content"] = serde_json::json!([
            { "type": "image", "source": { "type": "ref", "ref": blob.as_str(), "mediaType": "image/png" } }
        ]);
        let message: Block = serde_json::from_value(message).unwrap();
        let (_directory, path, mut connection) =
            database(include_str!("schema/conversation-0.1.22-dev.20261015T134426Z.sql"));
        connection
            .execute(
                "INSERT INTO nodes
                   (id, number, description, round, started_at, can_spawn, delivered, state, block_count, output_revision)
                 VALUES ('root', 0, '', 1, 0, 1, 0, ?1, 3, 0)",
                [to_json(&state())],
            )
            .unwrap();
        // The root's message names the blob, and its answer an attachment.
        let mut named = serde_json::to_value(reply("t0")).unwrap();
        named["text"] = serde_json::json!("See ![the shot](attachment:a1).");
        let named: Block = serde_json::from_value(named).unwrap();
        connection
            .execute(
                "INSERT INTO attachments (number, name, media_type, size, blob) VALUES (1, 'shot.png', 'image/png', 3, ?1)",
                [blob.as_str()],
            )
            .unwrap();
        for (index, block) in [&message, &named, &response("r1")].into_iter().enumerate() {
            connection
                .execute(
                    "INSERT INTO blocks (node_id, idx, block) VALUES ('root', ?1, ?2)",
                    rusqlite::params![i64::try_from(index).unwrap(), to_json(block)],
                )
                .unwrap();
        }
        // The old share's snapshot: the root's blocks, a subagent's and the
        // attachment.
        for (agent, index, block) in [(0, 0, &message), (0, 1, &named), (1, 0, &user("c0"))] {
            connection
                .execute(
                    "INSERT INTO share_blocks (token, agent, idx, block) VALUES ('old', ?1, ?2, ?3)",
                    rusqlite::params![agent, index, to_json(block)],
                )
                .unwrap();
        }
        connection
            .execute_batch(
                "INSERT INTO share_agents (token, number, description, profile) VALUES ('old', 1, 'Look', NULL);
                 INSERT INTO share_attachments (token, number, name, media_type, size, blob)
                   SELECT 'old', number, name, media_type, size, blob FROM attachments;",
            )
            .unwrap();

        CONVERSATION.apply(&mut connection, &path, &NoBlobs).unwrap();

        let old = ShareToken::try_from("old").unwrap();
        let read = tree::read_page(
            &connection,
            &Transcript::Shared { token: old.clone() },
            PageQuery { before: None, after: None, around: None, edge: None },
        )
        .unwrap();
        assert!(matches!(read, PageRead::Page { length: 0, .. }), "the old share reads nothing");
        assert!(!crate::shares::shows_blob(&connection, &old, &blob).unwrap());
        for table in ["share_entries", "share_blocks", "share_blobs", "share_attachments"] {
            let rows: i64 = connection.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0)).unwrap();
            assert_eq!(rows, 0, "{table}");
        }
        assert!(crate::references::conversation_blobs(&connection).unwrap().contains(&blob));

        let new = ShareToken::try_from("new").unwrap();
        let transaction = connection.transaction().unwrap();
        crate::shares::take_conversation(&transaction, &new).unwrap();
        transaction.commit().unwrap();
        assert!(crate::shares::shows_blob(&connection, &new, &blob).unwrap());
        let shot = crate::shares::shown_attachment(&connection, &new, crate::conversation_attachments::AttachmentNumber(1))
            .unwrap();
        assert_eq!(shot.map(|row| row.name), Some("shot.png".to_owned()));
    }

    /// The control database of 0.1.22-dev.20261015T134426Z held the
    /// records of shares made before a share held only what was said: the
    /// migration removes them, so their links answer as ones that never
    /// existed, and keeps the conversation.
    // Cost: one database on disk, a few milliseconds.
    #[test]
    fn a_share_record_made_before_is_removed() {
        let (_directory, path, mut connection) =
            database(include_str!("schema/control-0.1.22-dev.20261015T134426Z.sql"));
        connection
            .execute_batch(
                "INSERT INTO users (id, email, nickname, password_hash, role, created_at)
                   VALUES ('u1', 'a@example.test', 'Ana', 'hash', 'master', 1);
                 INSERT INTO conversations (id, user_id, title, title_origin, archived, pinned, sort_order,
                     read_revision, target_kind, context_version, created_at, updated_at)
                   VALUES ('c1', 'u1', 'T', 'user', 0, 0, 0, 0, 'cloud', 0, 1, 1);
                 INSERT INTO shares (token, user_id, conversation_id, kind, block_id, title, taken_at)
                   VALUES ('old', 'u1', 'c1', 'conversation', NULL, 'T', 1),
                          ('older', 'u1', 'c1', 'answer', 'b1', 'T', 1);",
            )
            .unwrap();

        CONTROL.apply(&mut connection, &path, &NoBlobs).unwrap();

        let count = |table: &str| -> i64 {
            connection.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0)).unwrap()
        };
        assert_eq!((count("shares"), count("conversations")), (0, 1));
    }

    /// A shell call stored in transcript block format 6, whose own files
    /// and renames marked the paths in a temporary directory `temporary`:
    /// migrated to format 7, a file marked temporary is marked `outside`,
    /// the others and the renames are marked nothing, and the stored
    /// request list stays as it was. A flag that is not a boolean fails the
    /// migration.
    // Cost: three databases on disk, a few milliseconds.
    #[test]
    fn a_call_stored_with_temporary_paths_marks_them_outside() {
        use demi_shared_types::{Block, ChangedFile, EditKind, PathChange, ToolView};

        let model = serde_json::json!({
            "providerId": "anthropic",
            "model": {
                "id": "claude-opus-4-8",
                "name": "Claude Opus 4.8",
                "contextWindow": 200000,
                "outputLimit": 64000,
                "thinking": [{"type": "disabled"}],
                "acceptedExtensions": ["png"]
            },
            "thinking": {"type": "disabled"},
            "serviceTierId": null
        });
        let listed = serde_json::json!([{"path": "/w/a.txt", "kind": "added", "edits": []}]);
        let blocks = [
            serde_json::json!({
                "type": "user",
                "id": "u1",
                "turnId": "t1",
                "createdAt": "2026-10-14T05:00:00.000Z",
                "model": model,
                "content": [{"type": "text", "text": "Write the notes"}],
                "preamble": null
            }),
            serde_json::json!({
                "type": "tool_call",
                "id": "c1",
                "run": "c1",
                "createdAt": "2026-10-14T05:00:01.000Z",
                "model": model,
                "toolUseId": "toolu_c1",
                "toolName": "shell",
                "input": r#"{"script":"edit","description":"Write the notes"}"#,
                "status": "completed",
                "output": [{"type": "text", "text": "done"}],
                "view": {
                    "kind": "shell",
                    "status": "exited",
                    "commandId": "1",
                    "exitCode": 0,
                    "runningMs": 10,
                    "idleMs": 0,
                    "chunks": [],
                    "viewTruncated": false,
                    "files": [
                        {"path": "/tmp/fix.py", "kind": "added", "added": 1, "removed": 0, "temporary": true, "edits": [{}]},
                        {"path": "/w/a.txt", "kind": "added", "added": 1, "removed": 0, "edits": [{}]},
                        {"path": "/w/b.txt", "kind": "renamed", "from": "/tmp/b", "added": 0, "removed": 0, "edits": []}
                    ],
                    "filesTruncated": false,
                    "pathChanges": [
                        {"kind": "renamed", "from": "/tmp/b", "to": "/w/b.txt"},
                        {"kind": "removed", "path": "/tmp/old", "temporary": true}
                    ],
                    "runFiles": listed,
                    "requestFiles": listed
                }
            }),
        ];
        let (_directory, path, mut connection) = database(include_str!("schema/conversation-0.1.22-dev.context-limits.sql"));
        connection
            .execute(
                "INSERT INTO nodes
                   (id, number, description, round, started_at, can_spawn, delivered, state, block_count, output_revision)
                 VALUES ('root', 0, '', 1, 0, 1, 0, ?1, ?2, 0)",
                rusqlite::params![state_of_development(), i64::try_from(blocks.len()).unwrap()],
            )
            .unwrap();
        for (index, block) in blocks.iter().enumerate() {
            connection
                .execute(
                    "INSERT INTO blocks (node_id, idx, block) VALUES ('root', ?1, ?2)",
                    rusqlite::params![i64::try_from(index).unwrap(), block.to_string()],
                )
                .unwrap();
        }

        CONVERSATION.apply(&mut connection, &path, &NoBlobs).unwrap();
        let stored: Vec<Block> = connection
            .prepare("SELECT block FROM blocks WHERE node_id = 'root' ORDER BY idx")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|text| demi_shared_types::decode::<Block>(&text.unwrap()).unwrap())
            .collect();
        let Block::ToolCall(call) = &stored[1] else {
            panic!("{stored:?}");
        };
        let Some(ToolView::Shell(view)) = &call.view else {
            panic!("{call:?}");
        };
        let marked: Vec<_> = view.files.iter().flatten().map(|file| (file.path.as_str(), file.outside)).collect();
        assert_eq!(marked, [("/tmp/fix.py", true), ("/w/a.txt", false), ("/w/b.txt", false)]);
        assert_eq!(
            view.path_changes,
            [
                PathChange::Renamed { from: "/tmp/b".into(), to: "/w/b.txt".into() },
                PathChange::Removed { path: "/tmp/old".into() }
            ]
        );
        let unchanged = vec![ChangedFile {
            path: "/w/a.txt".into(),
            kind: EditKind::Added,
            from: None,
            lines: None,
            edits: Vec::new(),
        }];
        assert_eq!(view.run_files.as_ref(), Some(&unchanged));
        assert_eq!(view.request_files.as_ref(), Some(&unchanged));

        // A flag that is not a boolean, in a file or a rename, fails the
        // migration, which leaves the database as it was.
        for (list, at) in [("files", 0), ("pathChanges", 1)] {
            let mut malformed = blocks[1].clone();
            malformed["view"][list][at]["temporary"] = serde_json::json!("yes");
            let (_directory, path, mut connection) =
                database(include_str!("schema/conversation-0.1.22-dev.context-limits.sql"));
            connection
                .execute(
                    "INSERT INTO nodes
                       (id, number, description, round, started_at, can_spawn, delivered, state, block_count, output_revision)
                     VALUES ('root', 0, '', 1, 0, 1, 0, ?1, 2, 0)",
                    rusqlite::params![state_of_development()],
                )
                .unwrap();
            for (index, block) in [&blocks[0], &malformed].into_iter().enumerate() {
                connection
                    .execute(
                        "INSERT INTO blocks (node_id, idx, block) VALUES ('root', ?1, ?2)",
                        rusqlite::params![i64::try_from(index).unwrap(), block.to_string()],
                    )
                    .unwrap();
            }
            let refused = CONVERSATION.apply(&mut connection, &path, &NoBlobs);
            assert!(matches!(refused, Err(StorageError::Migration { .. })), "{list}: {refused:?}");
            let stored: String = connection
                .query_row("SELECT block FROM blocks WHERE idx = 1", [], |row| row.get(0))
                .unwrap();
            assert!(stored.contains(r#""temporary":"yes""#), "{list}: {stored}");
        }
    }

    /// A conversation the server's build stored (transcript block format 5,
    /// which a node says it was forked from) whose turn opened with
    /// redacted reasoning: that block belonged to no run, and the thinking
    /// after it opened its own. Migrated to format 6, the redacted
    /// reasoning is the run's first step and the thinking joins its run.
    // Cost: one database on disk, a few milliseconds.
    #[test]
    fn redacted_reasoning_stored_outside_a_run_opens_its_run() {
        use demi_shared_types::Block;

        let model = serde_json::json!({
            "providerId": "anthropic",
            "model": {
                "id": "claude-opus-4-8",
                "name": "Claude Opus 4.8",
                "contextWindow": 200000,
                "outputLimit": 64000,
                "thinking": [{"type": "adaptive", "efforts": ["high"]}],
                "acceptedExtensions": ["png"]
            },
            "thinking": {"type": "adaptive", "effort": "high"},
            "serviceTierId": null
        });
        let blocks = [
            serde_json::json!({
                "type": "user",
                "id": "u1",
                "turnId": "t1",
                "createdAt": "2026-10-13T05:00:00.000Z",
                "model": model,
                "content": [{"type": "text", "text": "Go"}],
                "preamble": null
            }),
            serde_json::json!({
                "type": "redacted_thinking",
                "id": "r1",
                "createdAt": "2026-10-13T05:00:01.000Z",
                "model": model,
                "data": "opaque"
            }),
            serde_json::json!({
                "type": "thinking",
                "id": "k1",
                "createdAt": "2026-10-13T05:00:02.000Z",
                "model": model,
                "text": "Plan.",
                "signature": "sig",
                "run": "k1"
            }),
            serde_json::json!({
                "type": "text",
                "id": "a1",
                "createdAt": "2026-10-13T05:00:03.000Z",
                "model": model,
                "text": "Done.",
                "forkable": true
            }),
        ];
        let sql = include_str!("schema/conversation-0.1.22-dev.forked-from.sql");
        let (_directory, path, mut connection) = database(sql);
        connection
            .execute(
                "INSERT INTO nodes
                   (id, number, description, round, started_at, can_spawn, delivered, state, block_count, output_revision)
                 VALUES ('root', 0, '', 1, 0, 1, 0, ?1, ?2, 0)",
                rusqlite::params![state_of_development(), i64::try_from(blocks.len()).unwrap()],
            )
            .unwrap();
        for (index, block) in blocks.iter().enumerate() {
            connection
                .execute(
                    "INSERT INTO blocks (node_id, idx, block) VALUES ('root', ?1, ?2)",
                    rusqlite::params![i64::try_from(index).unwrap(), block.to_string()],
                )
                .unwrap();
        }

        CONVERSATION.apply(&mut connection, &path, &NoBlobs).unwrap();
        let runs: Vec<Option<String>> = connection
            .prepare("SELECT block FROM blocks WHERE node_id = 'root' ORDER BY idx")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|text| demi_shared_types::decode::<Block>(&text.unwrap()).unwrap())
            .map(|block| block.run().map(|run| run.as_str().to_owned()))
            .collect();
        assert_eq!(runs, [None, Some("r1".to_owned()), Some("r1".to_owned()), None]);
    }

    /// A subagent limit stored before the number was kept: a number stays
    /// the limit, and a null, Unlimited, stays lifted with the default
    /// number, so choosing Limited shows 10 or 1, never an empty field.
    #[test]
    fn a_lifted_limit_of_the_control_schema_without_detect_title_keeps_a_number() {
        let (_directory, path, mut connection) =
            database(include_str!("schema/control-0.1.22-dev.no-detect-title.sql"));
        connection
            .execute_batch(
                "INSERT INTO users (id, email, nickname, password_hash, role, created_at)
                   VALUES ('u1', 'a@example.test', 'a', 'h', 'master', 0),
                          ('u2', 'b@example.test', 'b', 'h', 'user', 0);
                 INSERT INTO user_subagents (user_id, enabled, max_live, max_depth)
                   VALUES ('u1', 1, NULL, 3), ('u2', 0, 4, NULL);",
            )
            .unwrap();

        CONTROL.apply(&mut connection, &path, &NoBlobs).unwrap();
        let rows: Vec<(String, bool, i64, bool, i64, bool)> = connection
            .prepare(
                "SELECT user_id, enabled, max_live, max_live_unlimited, max_depth, max_depth_unlimited
                 FROM user_subagents ORDER BY user_id",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            rows,
            [
                ("u1".to_owned(), true, 10, true, 3, false),
                ("u2".to_owned(), false, 4, false, 1, true),
            ]
        );
    }

    /// A permission request stored before requests had kinds keeps its
    /// values through the rebuilt table, as a permission request.
    #[test]
    fn a_request_of_the_control_schema_with_the_plugins_secrets_stays_a_permission_request() {
        let (_directory, path, mut connection) =
            database(include_str!("schema/control-0.1.22-dev.plugin-secrets.sql"));
        connection
            .execute_batch(
                "INSERT INTO users (id, email, nickname, password_hash, role, created_at)
                   VALUES ('u1', 'a@example.test', 'a', 'h', 'master', 0);
                 INSERT INTO conversations
                   (id, user_id, title, title_origin, archived, pinned, sort_order, read_revision,
                    target_kind, context_version, created_at, updated_at)
                   VALUES ('c1', 'u1', 't', 'user', 0, 0, 0, 0, 'cloud', 0, 0, 0);
                 INSERT INTO permission_requests
                   (id, conversation_id, categories, command, node_id, agent_number, agent_description,
                    created_at, decision, decided_at, read)
                   VALUES ('r1', 'c1', '[\"skills.manage\"]', 'demi skills add a/b', 'n1', 2, 'helper',
                           5, 'allowed', 6, 1);",
            )
            .unwrap();

        CONTROL.apply(&mut connection, &path, &NoBlobs).unwrap();
        let row: (String, String, String, String, i64, String, i64, i64) = connection
            .query_row(
                "SELECT kind, categories, command, node_id, agent_number, decision, decided_at, read
                 FROM permission_requests WHERE id = 'r1'",
                [],
                |row| {
                    Ok((
                        row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?,
                        row.get(5)?, row.get(6)?, row.get(7)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            row,
            (
                "permission".to_owned(),
                "[\"skills.manage\"]".to_owned(),
                "demi skills add a/b".to_owned(),
                "n1".to_owned(),
                2,
                "allowed".to_owned(),
                6,
                1
            )
        );
    }

    /// A database of the control schema with the plugins' secrets before a
    /// request had a read state, which a server keeps, migrates; a request
    /// that waits there keeps its values and is unread.
    #[test]
    fn a_waiting_request_of_the_control_schema_before_read_states_is_unread() {
        let (_directory, path, mut connection) =
            database(include_str!("schema/control-0.1.22-dev.plugin-secrets-unread.sql"));
        connection
            .execute_batch(
                "INSERT INTO users (id, email, nickname, password_hash, role, created_at)
                   VALUES ('u1', 'a@example.test', 'a', 'h', 'master', 0);
                 INSERT INTO conversations
                   (id, user_id, title, title_origin, archived, pinned, sort_order, read_revision,
                    target_kind, context_version, created_at, updated_at)
                   VALUES ('c1', 'u1', 't', 'user', 0, 0, 0, 0, 'cloud', 0, 0, 0);
                 INSERT INTO permission_requests
                   (id, conversation_id, categories, command, node_id, agent_number, agent_description,
                    created_at, decision, decided_at)
                   VALUES ('r1', 'c1', '[\"skills.manage\"]', 'demi skills add a/b', 'n1', NULL, NULL,
                           5, NULL, NULL);",
            )
            .unwrap();

        CONTROL.apply(&mut connection, &path, &NoBlobs).unwrap();
        let row: (String, String, String, String, i64, Option<String>, i64) = connection
            .query_row(
                "SELECT kind, categories, command, node_id, created_at, decision, read
                 FROM permission_requests WHERE id = 'r1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?)),
            )
            .unwrap();
        assert_eq!(
            row,
            (
                "permission".to_owned(),
                "[\"skills.manage\"]".to_owned(),
                "demi skills add a/b".to_owned(),
                "n1".to_owned(),
                5,
                None,
                0
            )
        );
    }

    /// A database of the control schema before passkeys keeps its accounts,
    /// sessions and credentials through the migration, and its accounts can
    /// have passkeys after it.
    #[test]
    fn the_control_schema_before_passkeys_keeps_its_accounts_and_takes_passkeys() {
        let (_directory, path, mut connection) =
            database(include_str!("schema/control-0.1.22-dev.credentials.sql"));
        connection
            .execute_batch(
                "INSERT INTO users (id, email, nickname, password_hash, role, created_at)
                   VALUES ('u1', 'a@example.test', 'Ana', 'h', 'master', 3);
                 INSERT INTO web_sessions (token_hash, user_id, expires_at) VALUES ('t1', 'u1', 9);
                 INSERT INTO user_credentials
                   (id, user_id, name, note, fields, sealed, keep_until, last_used_at, created_at, updated_at)
                   VALUES ('k1', 'u1', 'GitHub', '', '[]', x'00', NULL, NULL, 4, 4);",
            )
            .unwrap();

        CONTROL.apply(&mut connection, &path, &NoBlobs).unwrap();
        let kept: (String, String, i64, String, String) = connection
            .query_row(
                "SELECT u.email, u.nickname, s.expires_at, c.name, c.id
                 FROM users u JOIN web_sessions s ON s.user_id = u.id
                   JOIN user_credentials c ON c.user_id = u.id",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .unwrap();
        assert_eq!(
            kept,
            ("a@example.test".to_owned(), "Ana".to_owned(), 9, "GitHub".to_owned(), "k1".to_owned())
        );
        connection
            .execute_batch(
                "INSERT INTO passkeys (id, user_id, name, credential, created_at, last_used_at)
                   VALUES ('p1', 'u1', 'iCloud Keychain', '{}', 5, NULL);
                 INSERT INTO passkey_challenges (id, kind, user_id, state, expires_at)
                   VALUES ('c1', 'registration', 'u1', '{}', 6);",
            )
            .unwrap();
    }

    /// A database of the control schema before the signed-in browsers keeps
    /// its sessions, each with an id, named `Web Browser`, signed in and
    /// last active at its last renewal and not verified, and takes handoff
    /// codes after it.
    #[test]
    fn the_control_schema_before_the_signed_in_browsers_keeps_its_sessions() {
        let (_directory, path, mut connection) =
            database(include_str!("schema/control-0.1.22-dev.passkeys.sql"));
        connection
            .execute_batch(
                "INSERT INTO users (id, email, nickname, password_hash, role, created_at)
                   VALUES ('u1', 'a@example.test', 'Ana', 'hash', 'master', 1);
                 INSERT INTO web_sessions (token_hash, user_id, expires_at)
                   VALUES ('t1', 'u1', 2592000009), ('t2', 'u1', 2592000010);",
            )
            .unwrap();
        CONTROL.apply(&mut connection, &path, &NoBlobs).unwrap();
        let mut kept = connection
            .prepare(
                "SELECT token_hash, id, user_id, expires_at, name, created_at, last_active_at, verified_at, handoff_id
                 FROM web_sessions ORDER BY token_hash",
            )
            .unwrap();
        let kept: Vec<(String, String, String, i64, String, i64, i64, Option<i64>, Option<String>)> = kept
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                ))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(kept.len(), 2);
        assert_ne!(kept[0].1, kept[1].1, "each session has an id of its own");
        for ((token, id, user, expires_at, name, created_at, last_active_at, verified_at, handoff), expected) in
            kept.into_iter().zip([("t1", 2_592_000_009), ("t2", 2_592_000_010)])
        {
            assert!(!id.is_empty());
            assert_eq!(
                (token.as_str(), user.as_str(), expires_at, name.as_str(), verified_at, handoff),
                (expected.0, "u1", expected.1, "Web Browser", None, None)
            );
            assert_eq!((created_at, last_active_at), (expected.1 - 2_592_000_000, expected.1 - 2_592_000_000));
        }
        connection
            .execute(
                "INSERT INTO handoff_codes (user_id, kind, id, code_hash, to_path, expires_at, spent)
                   VALUES ('u1', 'handoff', 'h1', 'c1', '/chat', 5, 0)",
                [],
            )
            .unwrap();
    }

    /// A database of the control schema before administered accounts keeps
    /// its accounts enabled and with their own passwords, its sessions made
    /// by no administrator and each user's handoff code as a handoff code,
    /// and takes sign-in codes beside it.
    #[test]
    fn the_control_schema_before_administered_accounts_keeps_its_accounts_and_codes() {
        let (_directory, path, mut connection) =
            database(include_str!("schema/control-0.1.22-dev.saved-messages.sql"));
        connection
            .execute_batch(
                "INSERT INTO users (id, email, nickname, password_hash, role, created_at)
                   VALUES ('u1', 'a@example.test', 'Ana', 'hash', 'master', 1),
                          ('u2', 'b@example.test', 'Bo', 'hash', 'user', 2);
                 INSERT INTO web_sessions (token_hash, id, user_id, expires_at, name, created_at, last_active_at)
                   VALUES ('t1', 's1', 'u2', 9, 'Safari on iPhone', 3, 4);
                 INSERT INTO handoff_codes (user_id, id, code_hash, to_path, expires_at, spent)
                   VALUES ('u2', 'h1', 'c1', '/chat', 5, 0);",
            )
            .unwrap();
        CONTROL.apply(&mut connection, &path, &NoBlobs).unwrap();
        let users: Vec<(String, bool, bool)> = connection
            .prepare("SELECT id, disabled, chooses_password FROM users ORDER BY id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            users,
            [("u1".to_owned(), false, false), ("u2".to_owned(), false, false)]
        );
        let active: Vec<(String, Option<i64>)> = connection
            .prepare("SELECT id, last_active_at FROM users ORDER BY id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(active, [("u1".to_owned(), None), ("u2".to_owned(), Some(4))]);
        let session: (String, Option<String>) = connection
            .query_row("SELECT name, issued_by FROM web_sessions WHERE id = 's1'", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(session, ("Safari on iPhone".to_owned(), None));
        let code: (String, String, Option<String>, String) = connection
            .query_row(
                "SELECT user_id, kind, issued_by, to_path FROM handoff_codes WHERE id = 'h1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(code, ("u2".to_owned(), "handoff".to_owned(), None, "/chat".to_owned()));
        connection
            .execute(
                "INSERT INTO handoff_codes (user_id, kind, issued_by, id, code_hash, to_path, expires_at, spent)
                   VALUES ('u2', 'sign_in', 'u1', 'h2', 'c2', '/', 6, 0)",
                [],
            )
            .unwrap();
    }

    /// A placeholder title stored in English loses its words; a title of
    /// any other origin keeps them, the English words included.
    #[test]
    fn a_placeholder_title_of_the_schema_with_account_rotation_holds_no_words() {
        let (_directory, path, mut connection) =
            database(include_str!("schema/control-0.1.22-dev.account-rotation.sql"));
        connection
            .execute_batch(
                "INSERT INTO users (id, email, nickname, password_hash, role, created_at)
                   VALUES ('u1', 'a@example.test', 'Ana', 'hash', 'master', 1);
                 INSERT INTO conversations (id, user_id, title, title_origin, archived, pinned, sort_order,
                     read_revision, target_kind, context_version, created_at, updated_at)
                   VALUES ('c1', 'u1', 'New conversation', 'placeholder', 0, 0, 0, 0, 'cloud', 0, 1, 1),
                          ('c2', 'u1', 'New conversation', 'user', 0, 0, 1, 0, 'cloud', 0, 1, 1),
                          ('c3', 'u1', 'Fix the build', 'generated', 0, 0, 2, 0, 'cloud', 0, 1, 1);",
            )
            .unwrap();
        CONTROL.apply(&mut connection, &path, &NoBlobs).unwrap();
        let titles: Vec<(String, String)> = connection
            .prepare("SELECT id, title FROM conversations ORDER BY id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            titles,
            [
                ("c1".to_owned(), String::new()),
                ("c2".to_owned(), "New conversation".to_owned()),
                ("c3".to_owned(), "Fix the build".to_owned()),
            ]
        );
    }

    /// The development builds' schemas are the ones their server's
    /// databases record, the one with `forked_from` included, and formats
    /// 4 and 5's, the subagent limits' and
    /// the one without Detect Title the ones development databases record.
    /// A server records the one with the plugins' secrets before read
    /// states, and may record the one with the users' credentials before
    /// passkeys and the one with passkeys before the signed-in browsers.
    /// A ledger row of 0.1.22-dev.20261016T043704Z keeps its user,
    /// conversation, entry, model, counts and order, its `created_at` as its
    /// `ended_at`, and null for what it lacked.
    #[test]
    fn a_ledger_row_from_before_calls_keeps_its_counts_and_null_for_what_it_lacked() {
        let (_directory, path, mut connection) =
            database(include_str!("schema/control-0.1.22-dev.20261016T043704Z.sql"));
        connection
            .execute_batch(
                "INSERT INTO users (id, email, nickname, password_hash, role, created_at)
                   VALUES ('user-1', 'a@example.test', 'A', 'hash', 'master', 1);
                 INSERT INTO usage_ledger VALUES
                   ('row-2', 'user-1', 'conversation-1', 'entry-1', 'model-1', 5, 6, 7, 8, 30),
                   ('row-1', 'user-1', 'conversation-1', 'entry-1', 'model-1', 1, 2, 3, 4, 20);",
            )
            .unwrap();
        CONTROL.apply(&mut connection, &path, &NoBlobs).unwrap();
        let rows: Vec<String> = connection
            .prepare(
                "SELECT id, user_id, conversation_id, agent, turn_id, purpose, provider_id, account_id,
                   model_id, request_id, vendor_id, input_tokens, output_tokens, cache_read_tokens,
                   cache_write_tokens, cache_write_1h_tokens, outcome, quota, started_at, ended_at
                 FROM usage_ledger ORDER BY rowid",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((0..20)
                    .map(|index| match row.get_ref(index).unwrap() {
                        rusqlite::types::ValueRef::Null => "null".to_owned(),
                        rusqlite::types::ValueRef::Integer(value) => value.to_string(),
                        rusqlite::types::ValueRef::Text(text) => String::from_utf8_lossy(text).into_owned(),
                        other => format!("{other:?}"),
                    })
                    .collect::<Vec<_>>()
                    .join(" "))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            rows,
            [
                "row-2 user-1 conversation-1 null null null entry-1 null model-1 null null 5 6 7 8 null null null null 30",
                "row-1 user-1 conversation-1 null null null entry-1 null model-1 null null 1 2 3 4 null null null null 20",
            ]
        );
    }

    /// A ledger row of 0.1.22 keeps every column
    /// through the table made again, and the ledger then takes an image's
    /// row (`images.md` § Accounts and failures).
    /// Planted defect this catches: a migration that leaves the purpose's
    /// check as it was, or copies the columns out of order.
    /// One small database, milliseconds.
    #[test]
    fn a_ledger_row_of_0_1_22_stays_whole_and_the_ledger_takes_an_images_row() {
        let (_directory, path, mut connection) =
            database(include_str!("schema/control-0.1.22.sql"));
        let row = "'row-1', 'user-1', 'conversation-1', 2, 'turn-1', 'turn', 'entry-1', 'account-1', 'model-1', \
            'request-1', 'vendor-1', 1, 2, 3, 4, 5, 'answered', '[]', 10, 20";
        connection
            .execute_batch(&format!(
                "INSERT INTO users (id, email, nickname, password_hash, role, created_at)
                   VALUES ('user-1', 'a@example.test', 'A', 'hash', 'master', 1);
                 INSERT INTO usage_ledger VALUES ({row});"
            ))
            .unwrap();
        CONTROL.apply(&mut connection, &path, &NoBlobs).unwrap();
        connection
            .execute_batch(
                "INSERT INTO usage_ledger (id, user_id, conversation_id, purpose, provider_id, model_id, ended_at)
                   VALUES ('row-2', 'user-1', 'conversation-1', 'image', 'entry-1', 'gpt-image-2', 30);",
            )
            .unwrap();
        let kept: String = connection
            .query_row(
                "SELECT id || user_id || conversation_id || agent || turn_id || purpose || provider_id || account_id
                   || model_id || request_id || vendor_id || input_tokens || output_tokens || cache_read_tokens
                   || cache_write_tokens || cache_write_1h_tokens || outcome || quota || started_at || ended_at
                 FROM usage_ledger WHERE id = 'row-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(kept, "row-1user-1conversation-12turn-1turnentry-1account-1model-1request-1vendor-112345answered[]1020");
    }

    #[test]
    fn the_development_builds_in_the_history_are_the_ones_their_server_records() {
        for (schema, development, version) in [
            (&CONVERSATION, include_str!("schema/conversation-0.1.22-dev.20261011T064359Z.sql"), 1_563_671_332),
            (&CONVERSATION, include_str!("schema/conversation-0.1.22-dev.20261011T123543Z.sql"), 845_924_528),
            (&CONVERSATION, include_str!("schema/conversation-0.1.22-dev.block-format-4.sql"), 2_090_642_600),
            (&CONVERSATION, include_str!("schema/conversation-0.1.22-dev.block-format-5.sql"), 830_873_636),
            (&CONVERSATION, include_str!("schema/conversation-0.1.22-dev.forked-from.sql"), 712_991_662),
            (&CONTROL, include_str!("schema/control-0.1.22-dev.20261011T123543Z.sql"), 388_363_661),
            (&CONTROL, include_str!("schema/control-0.1.22-dev.subagent-limits.sql"), 491_070_162),
            (&CONTROL, include_str!("schema/control-0.1.22-dev.no-detect-title.sql"), 1_079_978_983),
            (&CONTROL, include_str!("schema/control-0.1.22-dev.plugin-secrets-unread.sql"), 1_293_042_998),
            (&CONTROL, include_str!("schema/control-0.1.22-dev.credentials.sql"), 235_949_009),
            (&CONTROL, include_str!("schema/control-0.1.22-dev.passkeys.sql"), 2_096_003_688),
            (&CONTROL, include_str!("schema/control-0.1.22-dev.20261016T043704Z.sql"), 1_043_494_885),
        ] {
            assert_eq!(version_of(development), version);
            assert!(schema.history.iter().any(|shipped| shipped.sql == development));
        }
    }

    /// The control migration to projects by directory (`upgrades.md`
    /// § What crosses releases): a Cloud conversation in a directory of its
    /// own choosing moves to the Cloud's home, a conversation's directory
    /// under `/home/demi/sessions` or the workspace of its directory, made
    /// when there is none; one in its own directory, a project's and a
    /// paired device's stay; workspaces of one directory, a Cloud's in two
    /// spellings included, become the oldest, with the others'
    /// conversations; a Cloud workspace at the home goes, its conversations
    /// running in the home; every device's home is unknown until its next
    /// hello.
    /// Planted defect this catches: a Cloud directory left outside
    /// projects, or a workspace selection that loses its directory.
    /// One small database, milliseconds.
    #[test]
    fn the_migration_puts_each_cloud_conversation_where_its_directory_says() {
        let (_directory, path, mut connection) =
            database(include_str!("schema/control-0.1.22-dev.20261016T043704Z.sql"));
        let own = "0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a01";
        connection
            .execute_batch(&format!(
                "
INSERT INTO users (id, email, nickname, password_hash, role, created_at)
  VALUES ('u', 'u@example.test', 'U', 'x', 'master', 1);
INSERT INTO devices (id, user_id, kind, name, platform, claimed_at)
  VALUES ('cloud', 'u', 'managed', 'Cloud', 'linux', 1), ('laptop', 'u', 'user', 'laptop', 'darwin', 1);
INSERT INTO workspaces (id, user_id, device_id, path, name, sort_order, created_at)
  VALUES ('w1', 'u', 'laptop', '/Users/u/site', 'site', 0, 1),
         ('w2', 'u', 'laptop', '/Users/u/site', 'site again', 1, 1),
         ('w3', 'u', 'cloud', '/home/demi/app', 'app', 2, 1),
         ('w4', 'u', 'cloud', '/home/demi/app/', 'app too', 3, 2),
         ('w5', 'u', 'cloud', '/home/demi/', 'home', 4, 1);
INSERT INTO conversations (id, user_id, title, title_origin, archived, pinned, sort_order, read_revision,
    target_kind, target_device_id, target_path, target_workspace_id, context_version, created_at, updated_at)
  VALUES ('{own}', 'u', '', 'placeholder', 0, 0, 0, 0, 'cloud', NULL, '/home/demi/sessions/{own}', NULL, 0, 1, 1),
         ('0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4b00', 'u', '', 'placeholder', 0, 0, 1, 0, 'cloud', NULL, NULL, NULL, 0, 1, 1),
         ('0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4b01', 'u', '', 'placeholder', 0, 0, 2, 0, 'cloud', NULL, '/home/demi', NULL, 0, 1, 1),
         ('0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4b02', 'u', '', 'placeholder', 0, 0, 3, 0, 'cloud', NULL, '/home/demi/sessions/{own}', NULL, 0, 1, 1),
         ('0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4b03', 'u', '', 'placeholder', 0, 0, 4, 0, 'cloud', NULL, '/home/demi/app', NULL, 0, 1, 1),
         ('0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4b04', 'u', '', 'placeholder', 0, 0, 5, 0, 'cloud', NULL, '/home/demi/docs/', NULL, 0, 1, 1),
         ('0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4b05', 'u', '', 'placeholder', 0, 0, 6, 0, 'workspace', NULL, NULL, 'w2', 0, 1, 1),
         ('0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4b06', 'u', '', 'placeholder', 0, 0, 7, 0, 'device', 'laptop', '/Users/u/notes', NULL, 0, 1, 1),
         ('0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4b07', 'u', '', 'placeholder', 0, 0, 8, 0, 'workspace', NULL, NULL, 'w4', 0, 1, 1),
         ('0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4b08', 'u', '', 'placeholder', 0, 0, 9, 0, 'workspace', NULL, NULL, 'w5', 0, 1, 1);
"
            ))
            .unwrap();
        CONTROL.apply(&mut connection, &path, &NoBlobs).unwrap();
        let target = |id: &str| -> (String, Option<String>, Option<String>, Option<String>) {
            connection
                .query_row(
                    "SELECT target_kind, target_device_id, target_path, target_workspace_id FROM conversations WHERE id = ?1",
                    [id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .unwrap()
        };
        let some = |text: &str| Some(text.to_owned());
        assert_eq!(target(own), ("cloud".into(), None, None, None));
        assert_eq!(target("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4b00"), ("cloud".into(), None, None, None));
        assert_eq!(target("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4b01"), ("home".into(), some("cloud"), None, None));
        assert_eq!(
            target("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4b02"),
            ("cloud".into(), None, some(&format!("/home/demi/sessions/{own}")), None)
        );
        assert_eq!(
            target("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4b03"),
            ("workspace".into(), some("cloud"), some("/home/demi/app"), some("w3"))
        );
        assert_eq!(
            target("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4b05"),
            ("workspace".into(), some("laptop"), some("/Users/u/site"), some("w1"))
        );
        assert_eq!(
            target("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4b06"),
            ("device".into(), some("laptop"), some("/Users/u/notes"), None)
        );
        // `/home/demi/app/` is `/home/demi/app`, whose oldest project keeps
        // its conversations; a Cloud project at the home goes.
        assert_eq!(
            target("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4b07"),
            ("workspace".into(), some("cloud"), some("/home/demi/app"), some("w3"))
        );
        assert_eq!(target("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4b08"), ("home".into(), some("cloud"), None, None));
        let (kind, device, path, workspace) = target("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4b04");
        assert_eq!((kind.as_str(), device.as_deref(), path.as_deref()), ("workspace", Some("cloud"), Some("/home/demi/docs")));
        let made: (String, String) = connection
            .query_row(
                "SELECT name, path FROM workspaces WHERE id = ?1",
                [workspace.unwrap()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(made, ("docs".to_owned(), "/home/demi/docs".to_owned()));
        let workspaces: Vec<String> = connection
            .prepare("SELECT id FROM workspaces WHERE id IN ('w1', 'w2', 'w3', 'w4', 'w5') ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(workspaces, ["w1", "w3"]);
        let homes: i64 = connection
            .query_row("SELECT count(*) FROM devices WHERE home IS NOT NULL", [], |row| row.get(0))
            .unwrap();
        assert_eq!(homes, 0);
    }
}
