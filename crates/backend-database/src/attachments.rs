//! Attachment records (`storage.md` § Control records, § Attachment and
//! transcript media): an upload's owner, media type, size, content hash and
//! a text file's snippet; the bytes are in the owner's blobs.

use demi_shared_types::{BlobRef, Timestamp};
use demi_web_api_protocol::ids::{AttachmentId, UserId};
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::StorageError;
use super::columns::{decode, instant};
use super::control::ControlService;

/// An upload as its record holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentRecord {
    pub id: AttachmentId,
    pub owner: UserId,
    pub media_type: String,
    pub size_bytes: u64,
    pub sha256: BlobRef,
    /// A text file's opening, as the upload's answer carried it.
    pub snippet: Option<String>,
    pub created_at: Timestamp,
}

impl ControlService {
    /// Records an upload of `owner`'s whose bytes `sha256` names in their
    /// blobs, with a text file's `snippet`.
    pub async fn create_attachment(
        &self,
        owner: UserId,
        media_type: String,
        size_bytes: u64,
        sha256: BlobRef,
        snippet: Option<String>,
    ) -> Result<AttachmentRecord, StorageError> {
        let id =
            AttachmentId::try_from(uuid::Uuid::new_v4().to_string()).expect("a UUID is not empty");
        self.call(move |connection, now| {
            // An upload is at most 25 MiB, far below the column's range.
            let size = i64::try_from(size_bytes).expect("an upload's size fits the column");
            connection.execute(
                "INSERT INTO attachments (id, user_id, media_type, size_bytes, sha256, snippet, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    id.as_str(),
                    owner.as_str(),
                    media_type,
                    size,
                    sha256.as_str(),
                    snippet,
                    now.as_millisecond()
                ],
            )?;
            Ok(AttachmentRecord {
                id,
                owner,
                media_type,
                size_bytes,
                sha256,
                snippet,
                created_at: now,
            })
        })
        .await
    }

    /// The upload `id` names, whoever's it is.
    pub async fn attachment(
        &self,
        id: AttachmentId,
    ) -> Result<Option<AttachmentRecord>, StorageError> {
        self.call(move |connection, _| attachment_by_id(connection, &id))
            .await
    }

    /// The blob of each of `owner`'s uploads, which stay for as long as the
    /// account (`storage.md` § Retention).
    pub async fn upload_blobs(&self, owner: UserId) -> Result<Vec<BlobRef>, StorageError> {
        self.call(move |connection, _| {
            let mut statement =
                connection.prepare("SELECT DISTINCT sha256 FROM attachments WHERE user_id = ?1")?;
            let mut rows = statement.query([owner.as_str()])?;
            let mut blobs = Vec::new();
            while let Some(row) = rows.next()? {
                blobs.push(decode(
                    "attachments",
                    "sha256",
                    BlobRef::try_from(row.get::<_, String>(0)?),
                )?);
            }
            Ok(blobs)
        })
        .await
    }
}

/// The upload `id` names, whoever's it is, read on `connection`.
pub(crate) fn attachment_by_id(
    connection: &Connection,
    id: &AttachmentId,
) -> Result<Option<AttachmentRecord>, StorageError> {
    connection
        .query_row(
            "SELECT id, user_id, media_type, size_bytes, sha256, snippet, created_at FROM attachments WHERE id = ?1",
            [id.as_str()],
            |row| Ok(attachment_row(row)),
        )
        .optional()?
        .transpose()
}

/// An `attachments` row, read from its columns.
fn attachment_row(row: &Row<'_>) -> Result<AttachmentRecord, StorageError> {
    const TABLE: &str = "attachments";
    Ok(AttachmentRecord {
        id: decode(
            TABLE,
            "id",
            AttachmentId::try_from(row.get::<_, String>("id")?),
        )?,
        owner: decode(
            TABLE,
            "user_id",
            UserId::try_from(row.get::<_, String>("user_id")?),
        )?,
        media_type: row.get("media_type")?,
        size_bytes: decode(
            TABLE,
            "size_bytes",
            u64::try_from(row.get::<_, i64>("size_bytes")?),
        )?,
        sha256: decode(
            TABLE,
            "sha256",
            BlobRef::try_from(row.get::<_, String>("sha256")?),
        )?,
        snippet: row.get("snippet")?,
        created_at: instant(row, TABLE, "created_at")?,
    })
}
