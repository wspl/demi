//! Messages saved for later (`storage.md` § Control records): each conversation's list,
//! newest first, and its revision, which the conversation's draft row keeps. Every
//! change of the list raises the revision in the transaction that makes it; a save and
//! an edit change the draft in the same transaction, so a message is never both saved
//! and in the draft.

use demi_shared_types::{Timestamp, TurnId};
use demi_web_api_protocol::drafts::{DraftWritten, Revisions};
use demi_web_api_protocol::ids::{AttachmentId, ConversationId, SavedEditId, SavedMessageId, UserId};
use demi_web_api_protocol::saved::{FromDraftAnswer, SAVED_MAX, SavedAnswer, SavedList, SavedMessage};
use jiff::SignedDuration;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use super::StorageError;
use super::columns::{count, decode, instant, json, to_json};
use super::control::ControlService;
use super::drafts::{StagedFile, Stored, Version, archived, staged_version, stored, write};

const TABLE: &str = "saved_messages";

/// How long an edit's answer and a send of a saved message are kept, for a
/// request sent again.
const KEPT: SignedDuration = SignedDuration::from_hours(24);

/// Why the list was left as it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SavedRefusal {
    /// The conversation is archived.
    Archived,
    /// The caller has no upload of this id.
    UploadNotFound(AttachmentId),
    /// The message as stored would be over a draft's limit.
    TooLarge,
    /// The conversation keeps [`SAVED_MAX`] messages already.
    Full,
    /// The conversation has no saved message of that id.
    NotFound,
    /// A saved message has the id already, with other content or in another
    /// conversation.
    IdTaken,
}

impl ControlService {
    /// The conversation's messages saved for later, newest first, with the
    /// list's revision.
    pub async fn saved_messages(
        &self,
        conversation: ConversationId,
    ) -> Result<SavedList, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let revision = stored(&transaction, &conversation)?
                .map_or(0, |draft| draft.saved_revision);
            let mut statement = transaction.prepare(
                "SELECT id, saved_at, document FROM saved_messages WHERE conversation_id = ?1
                 ORDER BY saved_at DESC, rowid DESC",
            )?;
            let messages = statement
                .query_map([conversation.as_str()], |row| Ok(saved_row(row)))?
                .map(|row| row?)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(SavedList { revision, messages })
        })
        .await
    }

    /// One of the conversation's messages saved for later, when it keeps it.
    pub async fn saved_message(
        &self,
        conversation: ConversationId,
        id: String,
    ) -> Result<Option<SavedMessage>, StorageError> {
        self.call(move |connection, _| saved(connection, &conversation, &id))
            .await
    }

    /// Saves the composer's message, `text` and `files`, for later as the
    /// message `id` and empties the conversation's draft, as a save of an
    /// empty draft built on the revision `base` would, in one transaction.
    /// Each upload is presented with what its record holds, as a draft's is.
    /// A save sent again, with an `id` the conversation keeps with the same
    /// content, changes nothing and answers the message with the draft as it
    /// is.
    pub async fn save_from_draft(
        &self,
        conversation: ConversationId,
        owner: UserId,
        id: SavedMessageId,
        base: u64,
        text: String,
        files: Vec<StagedFile>,
    ) -> Result<Result<FromDraftAnswer, SavedRefusal>, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let version = match staged(&transaction, &conversation, &owner, &id, text, files)? {
                Ok(Staged::New(version)) => version,
                Ok(Staged::Again(message)) => {
                    let draft = stored(&transaction, &conversation)?
                        .unwrap_or_else(|| Stored::written(None, Version::empty()));
                    let DraftWritten { draft, revisions } = draft.written_answer();
                    return Ok(Ok(FromDraftAnswer { message, draft, revisions }));
                }
                Err(refusal) => return Ok(Err(refusal)),
            };
            let message = insert(&transaction, &conversation, &id, version, now)?;
            // The draft's comments stay waiting there: a saved message keeps
            // the text and the files alone.
            let current = stored(&transaction, &conversation)?;
            let comments = current
                .as_ref()
                .map(|draft| draft.version.comments.clone())
                .unwrap_or_default();
            let mut draft = Stored::saved_on(
                current,
                base,
                Version {
                    comments,
                    ..Version::empty()
                },
            );
            draft.saved_revision += 1;
            write(&transaction, &conversation, &draft, now.as_millisecond())?;
            transaction.commit()?;
            let DraftWritten { draft, revisions } = draft.written_answer();
            Ok(Ok(FromDraftAnswer { message, draft, revisions }))
        })
        .await
    }

    /// Adds `text` and `files` as the message `id` saved for later, leaving
    /// the draft as it is, as Undo after a removal does; sent again with the
    /// same content, it changes nothing.
    pub async fn add_saved(
        &self,
        conversation: ConversationId,
        owner: UserId,
        id: SavedMessageId,
        text: String,
        files: Vec<StagedFile>,
    ) -> Result<Result<SavedAnswer, SavedRefusal>, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let message = match staged(&transaction, &conversation, &owner, &id, text, files)? {
                Ok(Staged::New(version)) => {
                    let message = insert(&transaction, &conversation, &id, version, now)?;
                    raise(&transaction, &conversation, now)?;
                    message
                }
                Ok(Staged::Again(message)) => message,
                Err(refusal) => return Ok(Err(refusal)),
            };
            let revisions = revisions(&transaction, &conversation)?;
            transaction.commit()?;
            Ok(Ok(SavedAnswer { message, revisions }))
        })
        .await
    }

    /// The edit `op`: moves the saved message `id` into the conversation's
    /// draft, and the draft's text and files, when it holds any, into a
    /// message saved for later as `becomes`, in one transaction. The answer
    /// is kept by `op` for a day, so the same edit sent again answers as it
    /// first did, whatever happened since; a refusal is not kept.
    pub async fn edit_saved(
        &self,
        conversation: ConversationId,
        op: SavedEditId,
        id: String,
        becomes: SavedMessageId,
    ) -> Result<Result<DraftWritten, SavedRefusal>, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            forget_old(&transaction, now)?;
            let kept: Option<String> = transaction
                .query_row(
                    "SELECT answer FROM saved_edits WHERE op = ?1 AND conversation_id = ?2",
                    [op.as_str(), conversation.as_str()],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(answer) = kept {
                let kept: KeptEdit = json("saved_edits", "answer", &answer)?;
                return Ok(Ok(kept.present()));
            }
            if archived(&transaction, &conversation)? {
                return Ok(Err(SavedRefusal::Archived));
            }
            if held(&transaction, becomes.as_str())? {
                return Ok(Err(SavedRefusal::IdTaken));
            }
            let Some(document) = remove(&transaction, &conversation, &id)? else {
                return Ok(Err(SavedRefusal::NotFound));
            };
            // The draft's text and files go into the list and the saved
            // message's come in; the draft's comments stay waiting.
            let current = stored(&transaction, &conversation)?;
            let (moved, comments) = current
                .as_ref()
                .map_or_else(|| Version::empty().into_message(), |draft| draft.version.clone().into_message());
            if !moved.is_empty() {
                insert(&transaction, &conversation, &becomes, moved, now)?;
            }
            let mut draft = Stored::written(current, Version { comments, ..document });
            draft.saved_revision += 1;
            write(&transaction, &conversation, &draft, now.as_millisecond())?;
            let kept = KeptEdit {
                revision: draft.revision,
                written: draft.written,
                version: draft.version.clone(),
                replaced: draft.replaced.clone(),
                saved_revision: draft.saved_revision,
            };
            transaction.execute(
                "INSERT INTO saved_edits (op, conversation_id, answer, made_at) VALUES (?1, ?2, ?3, ?4)",
                params![op.as_str(), conversation.as_str(), to_json(&kept), now.as_millisecond()],
            )?;
            transaction.commit()?;
            Ok(Ok(draft.written_answer()))
        })
        .await
    }

    /// Removes the saved message `id`; one already gone, as another page
    /// deleted or sent it, answers the same. Answers the revisions after it.
    pub async fn delete_saved(
        &self,
        conversation: ConversationId,
        id: String,
    ) -> Result<Result<Revisions, SavedRefusal>, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            if archived(&transaction, &conversation)? {
                return Ok(Err(SavedRefusal::Archived));
            }
            if remove(&transaction, &conversation, &id)?.is_some() {
                raise(&transaction, &conversation, now)?;
            }
            let revisions = revisions(&transaction, &conversation)?;
            transaction.commit()?;
            Ok(Ok(revisions))
        })
        .await
    }

    /// Takes the saved message `id` off the list as the session takes it as
    /// the message `turn`: true when the list held it, so that two sends of
    /// one message find it once. The send is kept for a day, so the same
    /// send sent again is known (`sent_as`).
    pub async fn take_saved(
        &self,
        conversation: ConversationId,
        id: String,
        turn: TurnId,
    ) -> Result<bool, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            forget_old(&transaction, now)?;
            let Some(message) = saved(&transaction, &conversation, &id)? else {
                return Ok(false);
            };
            remove(&transaction, &conversation, &id)?;
            transaction.execute(
                "INSERT INTO saved_sends (id, conversation_id, turn_id, saved_at, document, sent_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    message.id,
                    conversation.as_str(),
                    turn.as_str(),
                    message.saved_at.as_millisecond(),
                    to_json(&Version { text: message.text, files: message.files, comments: Vec::new() }),
                    now.as_millisecond()
                ],
            )?;
            raise(&transaction, &conversation, now)?;
            transaction.commit()?;
            Ok(true)
        })
        .await
    }

    /// The turn a send of the saved message `id` became within the last
    /// day, when one took it.
    pub async fn sent_as(
        &self,
        conversation: ConversationId,
        id: String,
    ) -> Result<Option<TurnId>, StorageError> {
        self.call(move |connection, _| {
            let turn: Option<String> = connection
                .query_row(
                    "SELECT turn_id FROM saved_sends WHERE id = ?1 AND conversation_id = ?2",
                    [id.as_str(), conversation.as_str()],
                    |row| row.get(0),
                )
                .optional()?;
            turn.map(|turn| decode("saved_sends", "turn_id", TurnId::try_from(turn)))
                .transpose()
        })
        .await
    }
}

/// An edit's answer as `saved_edits` keeps it: the draft's row as the edit
/// left it.
#[derive(Debug, Clone, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct KeptEdit {
    #[garde(skip)]
    revision: u64,
    #[garde(skip)]
    written: u64,
    #[garde(dive)]
    version: Version,
    #[garde(skip)]
    replaced: Option<(u64, Version)>,
    #[garde(skip)]
    saved_revision: u64,
}

impl KeptEdit {
    fn present(self) -> DraftWritten {
        Stored {
            revision: self.revision,
            written: self.written,
            version: self.version,
            replaced: self.replaced,
            saved_revision: self.saved_revision,
        }
        .written_answer()
    }
}

/// Forgets the edits and the sends older than a day: their answers are kept
/// only for a request sent again soon after.
fn forget_old(connection: &Connection, now: Timestamp) -> Result<(), StorageError> {
    let day_ago = now.as_millisecond() - KEPT.as_secs() * 1000;
    connection.execute("DELETE FROM saved_edits WHERE made_at < ?1", [day_ago])?;
    connection.execute("DELETE FROM saved_sends WHERE sent_at < ?1", [day_ago])?;
    Ok(())
}

/// Whether any conversation's saved message, or a send of one, holds the id.
fn held(connection: &Connection, id: &str) -> Result<bool, StorageError> {
    Ok(connection
        .query_row(
            "SELECT 1 FROM saved_messages WHERE id = ?1 UNION ALL SELECT 1 FROM saved_sends WHERE id = ?1",
            [id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// The conversation's revisions as they stand: those of a conversation
/// without a draft's row are 0.
fn revisions(connection: &Connection, conversation: &ConversationId) -> Result<Revisions, StorageError> {
    Ok(stored(connection, conversation)?
        .map_or(Revisions { draft: 0, saved: 0 }, |draft| draft.revisions()))
}

/// What a save names.
enum Staged {
    /// A message the conversation may keep: not archived, every upload the
    /// caller's, within a draft's limit, and fewer than [`SAVED_MAX`] kept
    /// already.
    New(Version),
    /// The save was made already: the conversation keeps a message of that id
    /// with this content.
    Again(SavedMessage),
}

/// The message a save of `id` names, or the one the conversation kept when
/// that save was made before. An id the conversation keeps with other
/// content, or another conversation keeps, is taken.
fn staged(
    connection: &Connection,
    conversation: &ConversationId,
    owner: &UserId,
    id: &SavedMessageId,
    text: String,
    files: Vec<StagedFile>,
) -> Result<Result<Staged, SavedRefusal>, StorageError> {
    let kept_by: Option<String> = connection
        .query_row(
            "SELECT conversation_id FROM saved_messages WHERE id = ?1
             UNION ALL SELECT conversation_id FROM saved_sends WHERE id = ?1",
            [id.as_str()],
            |row| row.get(0),
        )
        .optional()?;
    if kept_by.is_some_and(|kept_by| !kept_by.eq_ignore_ascii_case(conversation.as_str())) {
        return Ok(Err(SavedRefusal::IdTaken));
    }
    if archived(connection, conversation)? {
        return Ok(Err(SavedRefusal::Archived));
    }
    let version = match staged_version(connection, owner, text, files)? {
        Ok(version) => version,
        Err(missing) => return Ok(Err(SavedRefusal::UploadNotFound(missing))),
    };
    if version.too_large() {
        return Ok(Err(SavedRefusal::TooLarge));
    }
    if let Some(kept) = saved(connection, conversation, id.as_str())?
        .map_or_else(|| sent(connection, conversation, id.as_str()), |kept| Ok(Some(kept)))?
    {
        return Ok(if kept.text == version.text && kept.files == version.files {
            Ok(Staged::Again(kept))
        } else {
            Err(SavedRefusal::IdTaken)
        });
    }
    let kept: i64 = connection.query_row(
        "SELECT count(*) FROM saved_messages WHERE conversation_id = ?1",
        [conversation.as_str()],
        |row| row.get(0),
    )?;
    if kept >= count(SAVED_MAX) {
        return Ok(Err(SavedRefusal::Full));
    }
    Ok(Ok(Staged::New(version)))
}

fn saved(
    connection: &Connection,
    conversation: &ConversationId,
    id: &str,
) -> Result<Option<SavedMessage>, StorageError> {
    connection
        .query_row(
            "SELECT id, saved_at, document FROM saved_messages WHERE conversation_id = ?1 AND id = ?2",
            [conversation.as_str(), id],
            |row| Ok(saved_row(row)),
        )
        .optional()?
        .transpose()
}

/// A message a send took within the last day, as it was saved.
fn sent(
    connection: &Connection,
    conversation: &ConversationId,
    id: &str,
) -> Result<Option<SavedMessage>, StorageError> {
    connection
        .query_row(
            "SELECT id, saved_at, document FROM saved_sends WHERE conversation_id = ?1 AND id = ?2",
            [conversation.as_str(), id],
            |row| Ok(saved_row(row)),
        )
        .optional()?
        .transpose()
}

fn saved_row(row: &Row<'_>) -> Result<SavedMessage, StorageError> {
    let version: Version = json(TABLE, "document", &row.get::<_, String>("document")?)?;
    Ok(SavedMessage {
        id: row.get("id")?,
        text: version.text,
        files: version.files,
        saved_at: instant(row, TABLE, "saved_at")?,
    })
}

/// Saves `version` as the message `id` of the conversation, saved `now`.
fn insert(
    connection: &Connection,
    conversation: &ConversationId,
    id: &SavedMessageId,
    version: Version,
    now: Timestamp,
) -> Result<SavedMessage, StorageError> {
    connection.execute(
        "INSERT INTO saved_messages (id, conversation_id, saved_at, document) VALUES (?1, ?2, ?3, ?4)",
        params![id.as_str(), conversation.as_str(), now.as_millisecond(), to_json(&version)],
    )?;
    Ok(SavedMessage {
        id: id.to_string(),
        text: version.text,
        files: version.files,
        saved_at: now,
    })
}

/// Removes the conversation's saved message `id`; its text and files when
/// the conversation kept it.
fn remove(
    connection: &Connection,
    conversation: &ConversationId,
    id: &str,
) -> Result<Option<Version>, StorageError> {
    connection
        .query_row(
            "DELETE FROM saved_messages WHERE conversation_id = ?1 AND id = ?2 RETURNING document",
            [conversation.as_str(), id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(|document| json(TABLE, "document", &document))
        .transpose()
}

/// Raises the list's revision, which the conversation's draft row keeps:
/// a conversation without one gets the row of an empty draft.
fn raise(
    connection: &Connection,
    conversation: &ConversationId,
    now: Timestamp,
) -> Result<(), StorageError> {
    let mut draft = stored(connection, conversation)?
        .unwrap_or_else(|| Stored::written(None, Version::empty()));
    draft.saved_revision += 1;
    write(connection, conversation, &draft, now.as_millisecond())
}
