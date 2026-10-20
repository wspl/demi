//! The workflows of each Host directory as the backend last read them
//! (`crates/backend-host-access/src/kept.rs`).
//!
//! A page reads them from here at once, with when they were read, never
//! waiting on the Host, and also after a restart of the backend while the
//! Host is offline. The backend writes each read, and raises the revision
//! when the list differs from what was kept: the pages that show a
//! conversation in the directory hear of it in its metadata.

use demi_shared_types::Timestamp;
use demi_web_api_protocol::ids::DeviceId;
use demi_web_api_protocol::workflows::ConversationWorkflows;
use rusqlite::{OptionalExtension, params};

use super::StorageError;
use super::columns::{decode, instant, to_json};
use super::control::ControlService;

const TABLE: &str = "host_workflows";

/// A directory's workflows as last read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostWorkflows {
    pub list: ConversationWorkflows,
    pub read_at: Timestamp,
    /// Raised by each read that differed from the one before, from 1.
    pub revision: u64,
}

impl ControlService {
    /// The workflows kept of `directory` on the Host of `device`; none
    /// before the backend first read them.
    pub async fn host_workflows(&self, device: DeviceId, directory: String) -> Result<Option<HostWorkflows>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection
                .prepare_cached("SELECT list, read_at, revision FROM host_workflows WHERE device_id = ?1 AND directory = ?2")?;
            let mut rows = statement.query(params![device.as_str(), directory])?;
            let Some(row) = rows.next()? else {
                return Ok(None);
            };
            Ok(Some(HostWorkflows {
                list: decode(TABLE, "list", serde_json::from_str(&row.get::<_, String>("list")?))?,
                read_at: instant(row, TABLE, "read_at")?,
                revision: decode(TABLE, "revision", u64::try_from(row.get::<_, i64>("revision")?))?,
            }))
        })
        .await
    }

    /// Keeps `list` as the workflows of `directory` on the Host of `device`,
    /// read now; answers whether it differs from what was kept, which raised
    /// the revision.
    pub async fn keep_host_workflows(
        &self,
        device: DeviceId,
        directory: String,
        list: ConversationWorkflows,
    ) -> Result<bool, StorageError> {
        self.call(move |connection, now| {
            let list = to_json(&list);
            let transaction = connection.transaction()?;
            let kept: Option<String> = transaction
                .prepare_cached("SELECT list FROM host_workflows WHERE device_id = ?1 AND directory = ?2")?
                .query_row(params![device.as_str(), directory], |row| row.get(0))
                .optional()?;
            let changed = kept.as_ref() != Some(&list);
            transaction.execute(
                "INSERT INTO host_workflows (device_id, directory, list, read_at, revision) VALUES (?1, ?2, ?3, ?4, 1)
                 ON CONFLICT (device_id, directory) DO UPDATE SET
                   list = excluded.list,
                   read_at = excluded.read_at,
                   revision = revision + (list IS NOT excluded.list)",
                params![device.as_str(), directory, list, now.as_millisecond()],
            )?;
            transaction.commit()?;
            Ok(changed)
        })
        .await
    }
}
