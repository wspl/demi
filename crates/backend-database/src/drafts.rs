//! Conversation drafts: each conversation's draft with its revision, the revision
//! its text and files were written at, and the version a save replaced.
//!
//! Every change reads the row and writes it in one transaction, so two saves
//! never build on the same revision.

use demi_agent_store::media::comment_references;
use demi_shared_types::{BlobRef, MessageComment};
use demi_web_api_protocol::drafts::{
    ConversationDraft, DRAFT_BYTES_MAX, DraftFile, DraftWritten, ReplacedAction, ReplacedDraft,
    Revisions,
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
    /// The draft as stored would be over its limit.
    TooLarge,
    /// The replaced version is not the one the request names.
    Changed,
}

/// A draft's text, files and waiting comments, as its row stores the draft
/// and the replaced version, and as a message saved for later stores its
/// text and files: a saved message holds no comments, which stay waiting in
/// the draft, so its stored version omits them.
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

    /// Nothing a restore could bring back: no text but blank space, no file
    /// and no comment.
    pub(super) fn is_empty(&self) -> bool {
        self.text.trim().is_empty() && self.files.is_empty() && self.comments.is_empty()
    }

    /// The version's text and files, as a message saved for later keeps
    /// them, and its comments apart.
    pub(super) fn into_message(self) -> (Self, Vec<MessageComment>) {
        let Self { text, files, comments } = self;
        (Self { text, files, comments: Vec::new() }, comments)
    }

    /// Whether the version as stored is over a draft's limit.
    pub(super) fn too_large(&self) -> bool {
        to_json(self).len() > DRAFT_BYTES_MAX
    }
}

/// A draft's row.
pub(super) struct Stored {
    pub(super) revision: u64,
    pub(super) version: Version,
    /// The revision `version` was written at: a dismissal changes the
    /// revision and not the text.
    pub(super) written: u64,
    /// The version a save replaced, with the revision it had.
    pub(super) replaced: Option<(u64, Version)>,
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
                written: 1,
                replaced: None,
                saved_revision: 0,
            },
            Some(current) => Self {
                revision: current.revision + 1,
                // The text a save repeats is not written again.
                written: if current.version == version {
                    current.written
                } else {
                    current.revision + 1
                },
                replaced: current.replaced,
                saved_revision: current.saved_revision,
                version,
            },
        }
    }

    /// The row after a save of `version` built on the revision `base`: a
    /// save built on a revision older than the draft's text replaces a text
    /// its page never showed, which the row keeps as the replaced version,
    /// unless it is empty or the one saved.
    pub(super) fn saved_on(current: Option<Self>, base: u64, version: Version) -> Self {
        let unseen = current
            .as_ref()
            .filter(|current| {
                base < current.written && !current.version.is_empty() && current.version != version
            })
            .map(|current| (current.revision, current.version.clone()));
        let mut next = Self::written(current, version);
        if unseen.is_some() {
            next.replaced = unseen;
        }
        next
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
            replaced: self.replaced.map(|(revision, version)| ReplacedDraft {
                revision,
                text: version.text,
                files: version.files,
                comments: version.comments,
            }),
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
    /// The save always takes effect. When `base` is not the current revision, the
    /// version it replaces is kept as the replaced one, unless that version is empty
    /// or the one saved. Each upload is presented with what its record holds.
    pub async fn save_draft(
        &self,
        conversation: ConversationId,
        owner: UserId,
        base: u64,
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
            if version.too_large() {
                return Ok(Err(DraftRefusal::TooLarge));
            }
            let next = Stored::saved_on(stored(&transaction, &conversation)?, base, version);
            write(&transaction, &conversation, &next, now.as_millisecond())?;
            transaction.commit()?;
            Ok(Ok(next.written_answer()))
        })
        .await
    }

    /// Restores or dismisses the replaced version of `revision`. A restore
    /// exchanges it with the draft, whose version becomes the replaced one
    /// unless it is empty.
    pub async fn change_replaced_draft(
        &self,
        conversation: ConversationId,
        action: ReplacedAction,
        revision: u64,
    ) -> Result<Result<DraftWritten, DraftRefusal>, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            if archived(&transaction, &conversation)? {
                return Ok(Err(DraftRefusal::Archived));
            }
            let Some(current) = stored(&transaction, &conversation)? else {
                return Ok(Err(DraftRefusal::Changed));
            };
            let Some((replaced_revision, replaced)) = current.replaced else {
                return Ok(Err(DraftRefusal::Changed));
            };
            if replaced_revision != revision {
                return Ok(Err(DraftRefusal::Changed));
            }
            let next = match action {
                ReplacedAction::Restore => Stored {
                    revision: current.revision + 1,
                    written: current.revision + 1,
                    replaced: (!current.version.is_empty())
                        .then_some((current.revision, current.version)),
                    version: replaced,
                    saved_revision: current.saved_revision,
                },
                ReplacedAction::Dismiss => Stored {
                    revision: current.revision + 1,
                    written: current.written,
                    version: current.version,
                    replaced: None,
                    saved_revision: current.saved_revision,
                },
            };
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

/// The blobs the drafts of `owner`'s conversations name, in their current
/// and their replaced versions, and the messages they keep saved for later:
/// each upload's bytes.
pub(crate) fn draft_blobs(
    connection: &Connection,
    owner: &UserId,
) -> Result<Vec<BlobRef>, StorageError> {
    let mut versions = Vec::new();
    {
        let mut statement = connection.prepare(
            "SELECT d.revision, d.document, d.written, d.replaced_revision, d.replaced, d.saved_revision
             FROM conversation_drafts d JOIN conversations c ON c.id = d.conversation_id
             WHERE c.user_id = ?1",
        )?;
        let mut rows = statement.query([owner.as_str()])?;
        while let Some(row) = rows.next()? {
            let draft = stored_row(row)?;
            versions.push(draft.version);
            versions.extend(draft.replaced.map(|(_, version)| version));
        }
    }
    let mut statement = connection.prepare(
        "SELECT s.document FROM saved_messages s JOIN conversations c ON c.id = s.conversation_id
         WHERE c.user_id = ?1",
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
            "SELECT revision, document, written, replaced_revision, replaced, saved_revision
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
    let written = decode(
        TABLE,
        "written",
        u64::try_from(row.get::<_, i64>("written")?),
    )?;
    let replaced_revision: Option<i64> = row.get("replaced_revision")?;
    let replaced_version: Option<String> = row.get("replaced")?;
    let replaced = match (replaced_revision, replaced_version) {
        (Some(revision), Some(document)) => Some((
            decode(TABLE, "replaced_revision", u64::try_from(revision))?,
            json(TABLE, "replaced", &document)?,
        )),
        (None, None) => None,
        _ => {
            return Err(StorageError::Corrupt {
                table: TABLE,
                column: "replaced",
                reason: "a replaced version and its revision are stored together or not at all"
                    .into(),
            });
        }
    };
    let saved_revision = decode(
        TABLE,
        "saved_revision",
        u64::try_from(row.get::<_, i64>("saved_revision")?),
    )?;
    Ok(Stored {
        revision,
        version,
        written,
        replaced,
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
    let (replaced_revision, replaced) = match &draft.replaced {
        Some((revision, version)) => (Some(column(*revision)), Some(to_json(version))),
        None => (None, None),
    };
    connection.execute(
        "INSERT INTO conversation_drafts (conversation_id, revision, document, written, replaced_revision, replaced,
           saved_revision, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT (conversation_id) DO UPDATE SET revision = excluded.revision, document = excluded.document,
           written = excluded.written, replaced_revision = excluded.replaced_revision, replaced = excluded.replaced,
           saved_revision = excluded.saved_revision, updated_at = excluded.updated_at",
        params![
            conversation.as_str(),
            column(draft.revision),
            to_json(&draft.version),
            column(draft.written),
            replaced_revision,
            replaced,
            column(draft.saved_revision),
            now
        ],
    )?;
    Ok(())
}
