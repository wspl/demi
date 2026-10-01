//! Conversation drafts (`storage.md` § Control records, `web-api.md`
//! § Conversation drafts): each conversation's draft with its revision, the
//! revision its text and files were written at, and the version a save
//! replaced. Every change reads the row and writes it in one transaction, so
//! two saves never build on the same revision.

use demi_web_api_protocol::drafts::{
    ConversationDraft, DRAFT_BYTES_MAX, DraftFile, ReplacedAction, ReplacedDraft,
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

/// A draft's text and files, as its row stores the draft and the replaced
/// version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Version {
    #[garde(skip)]
    text: String,
    #[garde(skip)]
    files: Vec<DraftFile>,
}

impl Version {
    /// Nothing a restore could bring back: no text but blank space, and no
    /// file.
    fn is_empty(&self) -> bool {
        self.text.trim().is_empty() && self.files.is_empty()
    }
}

/// A draft's row.
struct Stored {
    revision: u64,
    version: Version,
    /// The revision `version` was written at: a dismissal changes the
    /// revision and not the text.
    written: u64,
    /// The version a save replaced, with the revision it had.
    replaced: Option<(u64, Version)>,
}

impl Stored {
    fn present(self) -> ConversationDraft {
        ConversationDraft {
            revision: self.revision,
            text: self.version.text,
            files: self.version.files,
            replaced: self.replaced.map(|(revision, version)| ReplacedDraft {
                revision,
                text: version.text,
                files: version.files,
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

    /// Saves `text` and `files` as the draft of `owner`'s conversation. The
    /// save always takes effect; when `base` is not the current revision, the
    /// version it replaces is kept as the replaced one, unless that version
    /// is empty or the one saved. Each upload is presented with what its
    /// record holds.
    pub async fn save_draft(
        &self,
        conversation: ConversationId,
        owner: UserId,
        base: u64,
        text: String,
        files: Vec<StagedFile>,
    ) -> Result<Result<ConversationDraft, DraftRefusal>, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            if archived(&transaction, &conversation)? {
                return Ok(Err(DraftRefusal::Archived));
            }
            let mut presented = Vec::with_capacity(files.len());
            for file in files {
                presented.push(match file {
                    StagedFile::Upload { id, file_name } => {
                        let upload = attachment_by_id(&transaction, &id)?
                            .filter(|upload| upload.owner == owner);
                        let Some(upload) = upload else {
                            return Ok(Err(DraftRefusal::UploadNotFound(id)));
                        };
                        DraftFile::Upload {
                            r#ref: id,
                            file_name,
                            media_type: upload.media_type,
                            sha256: upload.sha256,
                            snippet: upload.snippet,
                        }
                    }
                    StagedFile::Remote { device_id, path } => {
                        DraftFile::RemoteFile { device_id, path }
                    }
                });
            }
            let version = Version {
                text,
                files: presented,
            };
            if to_json(&version).len() > DRAFT_BYTES_MAX {
                return Ok(Err(DraftRefusal::TooLarge));
            }
            let next = match stored(&transaction, &conversation)? {
                None => Stored {
                    revision: 1,
                    version,
                    written: 1,
                    replaced: None,
                },
                Some(current) => {
                    // A save built on a revision older than the draft's text
                    // replaces a text its page never showed.
                    let unseen = base < current.written
                        && !current.version.is_empty()
                        && current.version != version;
                    Stored {
                        revision: current.revision + 1,
                        // The text a save repeats is not written again.
                        written: if current.version == version {
                            current.written
                        } else {
                            current.revision + 1
                        },
                        replaced: if unseen {
                            Some((current.revision, current.version))
                        } else {
                            current.replaced
                        },
                        version,
                    }
                }
            };
            write(&transaction, &conversation, &next, now.as_millisecond())?;
            transaction.commit()?;
            Ok(Ok(next.present()))
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
    ) -> Result<Result<ConversationDraft, DraftRefusal>, StorageError> {
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
                },
                ReplacedAction::Dismiss => Stored {
                    revision: current.revision + 1,
                    written: current.written,
                    version: current.version,
                    replaced: None,
                },
            };
            write(&transaction, &conversation, &next, now.as_millisecond())?;
            transaction.commit()?;
            Ok(Ok(next.present()))
        })
        .await
    }
}

/// Whether the conversation is archived.
fn archived(connection: &Connection, conversation: &ConversationId) -> Result<bool, StorageError> {
    Ok(connection.query_row(
        "SELECT archived FROM conversations WHERE id = ?1",
        [conversation.as_str()],
        |row| row.get(0),
    )?)
}

/// The conversation's draft row, when it has one.
fn stored(
    connection: &Connection,
    conversation: &ConversationId,
) -> Result<Option<Stored>, StorageError> {
    connection
        .query_row(
            "SELECT revision, document, written, replaced_revision, replaced FROM conversation_drafts
             WHERE conversation_id = ?1",
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
    Ok(Stored {
        revision,
        version,
        written,
        replaced,
    })
}

fn write(
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
        "INSERT INTO conversation_drafts (conversation_id, revision, document, written, replaced_revision, replaced, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (conversation_id) DO UPDATE SET revision = excluded.revision, document = excluded.document,
           written = excluded.written, replaced_revision = excluded.replaced_revision, replaced = excluded.replaced,
           updated_at = excluded.updated_at",
        params![
            conversation.as_str(),
            column(draft.revision),
            to_json(&draft.version),
            column(draft.written),
            replaced_revision,
            replaced,
            now
        ],
    )?;
    Ok(())
}
