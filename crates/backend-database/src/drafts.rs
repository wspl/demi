//! Conversation drafts: each conversation's draft with its revision, which
//! every save raises.
//!
//! Every change reads the row and writes it in one transaction, so two saves
//! never build on the same revision.

use demi_agent_store::media::comment_references;
use demi_shared_types::{BlobRef, MessageComment};
use demi_web_api_protocol::drafts::{
    ConversationDraft, DraftFile, DraftWritten, Revisions,
};
use demi_web_api_protocol::ids::{AttachmentId, ConversationId, UserId};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use super::StorageError;
use super::attachments::attachment_by_id;
use super::columns::{decode, json, to_json};
use super::control::ControlService;

const TABLE: &str = "conversation_drafts";

/// A file a save names, as the page names it: an upload of the caller's, or
/// a file on a paired device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StagedFile {
    Upload { id: AttachmentId, file_name: String },
    Remote { device_id: String, path: String },
}

/// Why a draft was left as it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftRefusal {
    /// The conversation is archived.
    Archived,
    /// The caller has no upload of this id.
    UploadNotFound(AttachmentId),
}

/// A draft's text, files and waiting comments, as its row stores the draft,
/// and as a message saved for later stores its text and files: a saved
/// message holds no comments, which stay waiting in the draft, so its stored
/// version omits them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Version {
    #[garde(skip)]
    pub(super) text: String,
    #[garde(skip)]
    pub(super) files: Vec<DraftFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[garde(skip)]
    pub(super) comments: Vec<MessageComment>,
}

impl Version {
    pub(super) fn empty() -> Self {
        Self {
            text: String::new(),
            files: Vec::new(),
            comments: Vec::new(),
        }
    }
}

/// A draft's row.
pub(super) struct Stored {
    pub(super) revision: u64,
    pub(super) version: Version,
    /// The revision of the conversation's messages saved for later, which
    /// the row keeps beside the draft's.
    pub(super) saved_revision: u64,
}

impl Stored {
    /// The row with `version` as the draft, written now, and the list's
    /// revision as it was: the next revision of `current`, or the first.
    pub(super) fn written(current: Option<Self>, version: Version) -> Self {
        match current {
            None => Self {
                revision: 1,
                version,
                saved_revision: 0,
            },
            Some(current) => Self {
                revision: current.revision + 1,
                saved_revision: current.saved_revision,
                version,
            },
        }
    }

    /// The draft's revision and the list's, as a write's answer names them.
    pub(super) fn revisions(&self) -> Revisions {
        Revisions {
            draft: self.revision,
            saved: self.saved_revision,
        }
    }

    /// The draft as a write's answer carries it, with the revisions.
    pub(super) fn written_answer(self) -> DraftWritten {
        DraftWritten {
            revisions: self.revisions(),
            draft: self.present(),
        }
    }

    pub(super) fn present(self) -> ConversationDraft {
        ConversationDraft {
            revision: self.revision,
            text: self.version.text,
            files: self.version.files,
            comments: self.version.comments,
        }
    }
}

impl ControlService {
    /// The conversation's draft: the empty one at revision 0 before its first
    /// save.
    pub async fn draft(
        &self,
        conversation: ConversationId,
    ) -> Result<ConversationDraft, StorageError> {
        self.call(move |connection, _| {
            Ok(stored(connection, &conversation)?
                .map_or_else(ConversationDraft::empty, Stored::present))
        })
        .await
    }

    /// Saves `text`, `files` and `comments` as the draft of `owner`'s conversation.
    ///
    /// The save always takes effect, whichever page saved the draft before.
    /// Each upload is presented with what its record holds.
    pub async fn save_draft(
        &self,
        conversation: ConversationId,
        owner: UserId,
        text: String,
        files: Vec<StagedFile>,
        comments: Vec<MessageComment>,
    ) -> Result<Result<DraftWritten, DraftRefusal>, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            if archived(&transaction, &conversation)? {
                return Ok(Err(DraftRefusal::Archived));
            }
            let version = match staged_version(&transaction, &owner, text, files)? {
                Ok(version) => Version { comments, ..version },
                Err(missing) => return Ok(Err(DraftRefusal::UploadNotFound(missing))),
            };
            let next = Stored::written(stored(&transaction, &conversation)?, version);
            write(&transaction, &conversation, &next, now.as_millisecond())?;
            transaction.commit()?;
            Ok(Ok(next.written_answer()))
        })
        .await
    }
}

/// The text and the files a save names, each upload presented with what its
/// record holds; the id of an upload `owner` has none of.
pub(super) fn staged_version(
    connection: &Connection,
    owner: &UserId,
    text: String,
    files: Vec<StagedFile>,
) -> Result<Result<Version, AttachmentId>, StorageError> {
    let mut presented = Vec::with_capacity(files.len());
    for file in files {
        presented.push(match file {
            StagedFile::Upload { id, file_name } => {
                let upload =
                    attachment_by_id(connection, &id)?.filter(|upload| upload.owner == *owner);
                let Some(upload) = upload else {
                    return Ok(Err(id));
                };
                DraftFile::Upload {
                    r#ref: id,
                    file_name,
                    media_type: upload.media_type,
                    sha256: upload.sha256,
                    snippet: upload.snippet,
                }
            }
            StagedFile::Remote { device_id, path } => DraftFile::RemoteFile { device_id, path },
        });
    }
    Ok(Ok(Version {
        text,
        files: presented,
        comments: Vec::new(),
    }))
}

/// The blobs the drafts of `owner`'s conversations name, and the messages saved
/// for later of their lists and of their projects' lists, deleted projects'
/// included: each upload's bytes.
pub(crate) fn draft_blobs(
    connection: &Connection,
    owner: &UserId,
) -> Result<Vec<BlobRef>, StorageError> {
    let mut versions = Vec::new();
    {
        let mut statement = connection.prepare(
            "SELECT d.revision, d.document, d.saved_revision
             FROM conversation_drafts d JOIN conversations c ON c.id = d.conversation_id
             WHERE c.user_id = ?1",
        )?;
        let mut rows = statement.query([owner.as_str()])?;
        while let Some(row) = rows.next()? {
            let draft = stored_row(row)?;
            versions.push(draft.version);
        }
    }
    let mut statement = connection.prepare(
        "SELECT s.document FROM saved_messages s JOIN conversations c ON c.id = s.conversation_id
         WHERE c.user_id = ?1
         UNION ALL
         SELECT s.document FROM saved_messages s JOIN projects w ON w.id = s.project_id
         WHERE w.user_id = ?1",
    )?;
    let mut rows = statement.query([owner.as_str()])?;
    while let Some(row) = rows.next()? {
        versions.push(json::<Version>(
            "saved_messages",
            "document",
            &row.get::<_, String>(0)?,
        )?);
    }
    // A file the draft holds, or a picture one of its waiting comments quotes.
    Ok(versions
        .into_iter()
        .flat_map(|version| {
            let pictures: Vec<BlobRef> = comment_references(&version.comments).cloned().collect();
            version
                .files
                .into_iter()
                .filter_map(|file| match file {
                    DraftFile::Upload { sha256, .. } => Some(sha256),
                    DraftFile::RemoteFile { .. } => None,
                })
                .chain(pictures)
        })
        .collect())
}

/// Whether the conversation is archived.
pub(crate) fn archived(
    connection: &Connection,
    conversation: &ConversationId,
) -> Result<bool, StorageError> {
    Ok(connection.query_row(
        "SELECT archived FROM conversations WHERE id = ?1",
        [conversation.as_str()],
        |row| row.get(0),
    )?)
}

/// The conversation's draft row, when it has one.
pub(super) fn stored(
    connection: &Connection,
    conversation: &ConversationId,
) -> Result<Option<Stored>, StorageError> {
    connection
        .query_row(
            "SELECT revision, document, saved_revision
             FROM conversation_drafts WHERE conversation_id = ?1",
            [conversation.as_str()],
            |row| Ok(stored_row(row)),
        )
        .optional()?
        .transpose()
}

fn stored_row(row: &Row<'_>) -> Result<Stored, StorageError> {
    let revision = decode(
        TABLE,
        "revision",
        u64::try_from(row.get::<_, i64>("revision")?),
    )?;
    let version = json(TABLE, "document", &row.get::<_, String>("document")?)?;
    let saved_revision = decode(
        TABLE,
        "saved_revision",
        u64::try_from(row.get::<_, i64>("saved_revision")?),
    )?;
    Ok(Stored {
        revision,
        version,
        saved_revision,
    })
}

pub(super) fn write(
    connection: &Connection,
    conversation: &ConversationId,
    draft: &Stored,
    now: i64,
) -> Result<(), StorageError> {
    // A revision grows by one with each change, so it stays far below the
    // column's range.
    let column =
        |revision: u64| i64::try_from(revision).expect("a draft's revision fits its column");
    connection.execute(
        "INSERT INTO conversation_drafts (conversation_id, revision, document, saved_revision, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (conversation_id) DO UPDATE SET revision = excluded.revision, document = excluded.document,
           saved_revision = excluded.saved_revision, updated_at = excluded.updated_at",
        params![
            conversation.as_str(),
            column(draft.revision),
            to_json(&draft.version),
            column(draft.saved_revision),
            now
        ],
    )?;
    Ok(())
}
