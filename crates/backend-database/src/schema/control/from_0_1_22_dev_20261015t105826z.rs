//! From 0.1.22-dev.20261015T105826Z's control schema, which a server keeps data
//! of, to the one in which:
//!
//! - A workspace keeps what its project's next conversation starts with, none
//!   before the first choice.
//! - A Fork's metadata holds the workflow its destination keeps (format 3). A
//!   Fork reserved before was reserved to start with Global Workflow, which its
//!   record keeps.

use rusqlite::Transaction;

use super::super::{Migration, ReadBlob, corrupt};
use super::shared::{fork_format_2};

pub(super) const MIGRATION: Migration = Migration::Code(control_from_0_1_22_dev_20261015t105826z);

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
        let earlier: fork_format_2::ForkMetadata = serde_json::from_str(&text).map_err(corrupt)?;
        let metadata = fork_format_3::ForkMetadata {
            title: earlier.title,
            target: earlier.target,
            model: earlier.model,
            workflow: serde_json::json!({ "type": "default" }),
            created_at: earlier.created_at,
            attached_hosts: earlier.attached_hosts,
            context_version: earlier.context_version,
        };
        update.execute(rusqlite::params![id, serde_json::to_string(&metadata).map_err(corrupt)?])?;
    }
    Ok(())
}

/// Fork metadata format 3, with the destination's workflow, as the control
/// schema from [`control_from_0_1_22_dev_20261015t105826z`] stored it.
mod fork_format_3 {
    use serde::Serialize;

    use super::fork_format_2::AttachedHost;

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub(super) struct ForkMetadata {
        pub title: Option<String>,
        pub target: serde_json::Value,
        pub model: Option<serde_json::Value>,
        pub workflow: serde_json::Value,
        pub created_at: serde_json::Value,
        pub attached_hosts: Vec<AttachedHost>,
        pub context_version: u64,
    }
}
