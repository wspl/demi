//! Messages saved for later: each conversation's list, newest first, and its
//! revision, which the conversation's draft row keeps.
//!
//! Every change of the list raises the revision in the transaction that makes it.
//! A save of the composer's message empties the draft in the same transaction,
//! so a message is never both saved and in the draft. An edit writes a message
//! in its place and never touches the draft.

use demi_shared_types::{Timestamp, TurnId};
use demi_web_api_protocol::drafts::{DraftWritten, Revisions};
use demi_web_api_protocol::ids::{AttachmentId, ConversationId, SavedMessageId, UserId};
use demi_web_api_protocol::saved::{
    FromDraftAnswer, ListedSavedMessage, SAVED_MAX, SAVED_OPENING_CHARS, SavedAnswer, SavedListing, SavedMessage,
};
use jiff::SignedDuration;
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::StorageError;
use super::columns::{count, decode, instant, json, to_json};
use super::control::ControlService;
use super::drafts::{StagedFile, Stored, Version, archived, staged_version, stored, write};

const TABLE: &str = "saved_messages";

/// [`SAVED_OPENING_CHARS`] as the query's parameter.
const OPENING_CHARS: i64 = SAVED_OPENING_CHARS as i64;

/// How long a send of a saved message is kept, for a request sent again.
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
    /// The conversation's messages saved for later as the composer lists
    /// them, newest first, with the list's revision: each with its text's
    /// opening and length, which the database cuts, so a long text never
    /// leaves it whole.
    pub async fn saved_listing(&self, conversation: ConversationId) -> Result<SavedListing, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let revision = stored(&transaction, &conversation)?
                .map_or(0, |draft| draft.saved_revision);
            let mut statement = transaction.prepare_cached(
                "SELECT id, saved_at,
                   substr(json_extract(document, '$.text'), 1, ?2) AS opening,
                   length(json_extract(document, '$.text')) AS length,
                   json_extract(document, '$.files') AS files
                 FROM saved_messages WHERE conversation_id = ?1
                 ORDER BY saved_at DESC, rowid DESC",
            )?;
            let messages = statement
                .query_map(params![conversation.as_str(), OPENING_CHARS], |row| Ok(listed_row(row)))?
                .map(|row| row?)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(SavedListing { revision, messages })
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

    /// Saves the composer's message, `text` and `files`, for later as the message
    /// `id` and empties the conversation's draft, as a save of an empty draft built
    /// on the revision `base` would, in one transaction.
    ///
    /// Each upload is presented with what its record holds, as a draft's is. A save
    /// sent again, with an `id` the conversation keeps with the same content,
    /// changes nothing and answers the message with the draft as it is.
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

    /// Writes `text` and `files` as the saved message `id`'s content, in its
    /// place in the list, leaving the draft as it is. Each upload is presented
    /// with what its record holds, as a save's is. The same content written
    /// again changes nothing.
    pub async fn write_saved(
        &self,
        conversation: ConversationId,
        owner: UserId,
        id: String,
        text: String,
        files: Vec<StagedFile>,
    ) -> Result<Result<SavedAnswer, SavedRefusal>, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            if archived(&transaction, &conversation)? {
                return Ok(Err(SavedRefusal::Archived));
            }
            let version = match staged_version(&transaction, &owner, text, files)? {
                Ok(version) => version,
                Err(missing) => return Ok(Err(SavedRefusal::UploadNotFound(missing))),
            };
            if version.too_large() {
                return Ok(Err(SavedRefusal::TooLarge));
            }
            let Some(kept) = saved(&transaction, &conversation, &id)? else {
                return Ok(Err(SavedRefusal::NotFound));
            };
            if kept.text == version.text && kept.files == version.files {
                let revisions = revisions(&transaction, &conversation)?;
                return Ok(Ok(SavedAnswer { message: kept, revisions }));
            }
            transaction.execute(
                "UPDATE saved_messages SET document = ?3 WHERE conversation_id = ?1 AND id = ?2",
                params![conversation.as_str(), id, to_json(&version)],
            )?;
            raise(&transaction, &conversation, now)?;
            let revisions = revisions(&transaction, &conversation)?;
            transaction.commit()?;
            Ok(Ok(SavedAnswer {
                message: SavedMessage { text: version.text, files: version.files, ..kept },
                revisions,
            }))
        })
        .await
    }

    /// Removes the saved message `id`; one already gone, as another page
    /// deleted or sent it, answers the same. Answers the revisions after it
    /// and the message as it was, whole, `None` when it was gone.
    pub async fn delete_saved(
        &self,
        conversation: ConversationId,
        id: String,
    ) -> Result<Result<(Revisions, Option<SavedMessage>), SavedRefusal>, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            if archived(&transaction, &conversation)? {
                return Ok(Err(SavedRefusal::Archived));
            }
            let removed = remove(&transaction, &conversation, &id)?;
            if removed.is_some() {
                raise(&transaction, &conversation, now)?;
            }
            let revisions = revisions(&transaction, &conversation)?;
            transaction.commit()?;
            Ok(Ok((revisions, removed)))
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

/// Forgets the sends older than a day: they are kept only for a request sent
/// again soon after.
fn forget_old(connection: &Connection, now: Timestamp) -> Result<(), StorageError> {
    let day_ago = now.as_millisecond() - KEPT.as_secs() * 1000;
    connection.execute("DELETE FROM saved_sends WHERE sent_at < ?1", [day_ago])?;
    Ok(())
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

fn listed_row(row: &Row<'_>) -> Result<ListedSavedMessage, StorageError> {
    Ok(ListedSavedMessage {
        id: row.get("id")?,
        opening: row.get("opening")?,
        length: decode(TABLE, "document", u64::try_from(row.get::<_, i64>("length")?))?,
        files: decode(TABLE, "document", serde_json::from_str(&row.get::<_, String>("files")?))?,
        saved_at: instant(row, TABLE, "saved_at")?,
    })
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

/// Removes the conversation's saved message `id`; the message as it was when
/// the conversation kept it.
fn remove(
    connection: &Connection,
    conversation: &ConversationId,
    id: &str,
) -> Result<Option<SavedMessage>, StorageError> {
    connection
        .query_row(
            "DELETE FROM saved_messages WHERE conversation_id = ?1 AND id = ?2
             RETURNING id, saved_at, document",
            [conversation.as_str(), id],
            |row| Ok(saved_row(row)),
        )
        .optional()?
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
