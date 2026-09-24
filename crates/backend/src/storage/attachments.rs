//! Attachment records (`storage.md` § Control records, § Attachment and
//! transcript media): an upload's owner, media type, size and content hash;
//! the bytes are in the owner's blobs.

use demi_core::{BlobRef, Timestamp};
use demi_web_api::ids::{AttachmentId, UserId};
use rusqlite::{OptionalExtension, Row, params};

use super::StorageError;
use super::columns::{decode, instant};
use super::control::ControlService;

/// An upload as its record holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AttachmentRecord {
    pub(crate) id: AttachmentId,
    pub(crate) owner: UserId,
    pub(crate) media_type: String,
    pub(crate) size_bytes: u64,
    pub(crate) sha256: BlobRef,
    pub(crate) created_at: Timestamp,
}

impl ControlService {
    /// Records an upload of `owner`'s whose bytes `sha256` names in their
    /// blobs.
    pub(crate) async fn create_attachment(
        &self,
        owner: UserId,
        media_type: String,
        size_bytes: u64,
        sha256: BlobRef,
    ) -> Result<AttachmentRecord, StorageError> {
        let id = AttachmentId::try_from(uuid::Uuid::new_v4().to_string()).expect("a UUID is not empty");
        self.call(move |connection, now| {
            // An upload is at most 25 MiB, far below the column's range.
            let size = i64::try_from(size_bytes).expect("an upload's size fits the column");
            connection.execute(
                "INSERT INTO attachments (id, user_id, media_type, size_bytes, sha256, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    id.as_str(),
                    owner.as_str(),
                    media_type,
                    size,
                    sha256.as_str(),
                    now.as_millisecond()
                ],
            )?;
            Ok(AttachmentRecord {
                id,
                owner,
                media_type,
                size_bytes,
                sha256,
                created_at: now,
            })
        })
        .await
    }

    /// The upload `id` names, whoever's it is.
    pub(crate) async fn attachment(&self, id: AttachmentId) -> Result<Option<AttachmentRecord>, StorageError> {
        self.call(move |connection, _| {
            connection
                .query_row(
                    "SELECT id, user_id, media_type, size_bytes, sha256, created_at FROM attachments WHERE id = ?1",
                    [id.as_str()],
                    |row| Ok(attachment_row(row)),
                )
                .optional()?
                .transpose()
        })
        .await
    }
}

/// An `attachments` row, read from its columns.
fn attachment_row(row: &Row<'_>) -> Result<AttachmentRecord, StorageError> {
    const TABLE: &str = "attachments";
    Ok(AttachmentRecord {
        id: decode(TABLE, "id", AttachmentId::try_from(row.get::<_, String>("id")?))?,
        owner: decode(TABLE, "user_id", UserId::try_from(row.get::<_, String>("user_id")?))?,
        media_type: row.get("media_type")?,
        size_bytes: decode(TABLE, "size_bytes", u64::try_from(row.get::<_, i64>("size_bytes")?))?,
        sha256: decode(TABLE, "sha256", BlobRef::try_from(row.get::<_, String>("sha256")?))?,
        created_at: instant(row, TABLE, "created_at")?,
    })
}
