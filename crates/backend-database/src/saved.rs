//! Messages saved for later (`storage.md` § Control records, `web-api.md`
//! § Messages saved for later): each conversation's list, newest first, and
//! its revision, which the conversation's draft row keeps. Every change of
//! the list raises the revision in the transaction that makes it; a save and
//! an edit change the draft in the same transaction, so a message is never
//! both saved and in the draft.

use demi_shared_types::Timestamp;
use demi_web_api_protocol::drafts::ConversationDraft;
use demi_web_api_protocol::ids::{AttachmentId, ConversationId, SavedMessageId, UserId};
use demi_web_api_protocol::saved::{SAVED_MAX, SavedList, SavedMessage};
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::StorageError;
use super::columns::{count, instant, json, to_json};
use super::control::ControlService;
use super::drafts::{StagedFile, Stored, Version, archived, staged_version, stored, write};

const TABLE: &str = "saved_messages";

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
    ) -> Result<Result<(SavedMessage, ConversationDraft), SavedRefusal>, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let version = match staged(&transaction, &conversation, &owner, &id, text, files)? {
                Ok(Staged::New(version)) => version,
                Ok(Staged::Again(message)) => {
                    let draft = stored(&transaction, &conversation)?
                        .unwrap_or_else(|| Stored::written(None, Version::empty()));
                    return Ok(Ok((message, draft.present())));
                }
                Err(refusal) => return Ok(Err(refusal)),
            };
            let message = insert(&transaction, &conversation, &id, version, now)?;
            let mut draft =
                Stored::saved_on(stored(&transaction, &conversation)?, base, Version::empty());
            draft.saved_revision += 1;
            write(&transaction, &conversation, &draft, now.as_millisecond())?;
            transaction.commit()?;
            Ok(Ok((message, draft.present())))
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
    ) -> Result<Result<SavedMessage, SavedRefusal>, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let version = match staged(&transaction, &conversation, &owner, &id, text, files)? {
                Ok(Staged::New(version)) => version,
                Ok(Staged::Again(message)) => return Ok(Ok(message)),
                Err(refusal) => return Ok(Err(refusal)),
            };
            let message = insert(&transaction, &conversation, &id, version, now)?;
            raise(&transaction, &conversation, now)?;
            transaction.commit()?;
            Ok(Ok(message))
        })
        .await
    }

    /// Moves the saved message `id` into the conversation's draft, and the
    /// draft's text and files, when it holds any, into a message saved for
    /// later under `becomes`, in one transaction. Sent again, when `id` is
    /// gone and the conversation keeps `becomes`, it answers the draft as it
    /// is; a `becomes` another message holds is taken.
    pub async fn edit_saved(
        &self,
        conversation: ConversationId,
        id: String,
        becomes: SavedMessageId,
    ) -> Result<Result<ConversationDraft, SavedRefusal>, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            if archived(&transaction, &conversation)? {
                return Ok(Err(SavedRefusal::Archived));
            }
            let kept_by: Option<String> = transaction
                .query_row(
                    "SELECT conversation_id FROM saved_messages WHERE id = ?1",
                    [becomes.as_str()],
                    |row| row.get(0),
                )
                .optional()?;
            let taken = kept_by.is_some();
            if kept_by.is_some_and(|kept| !kept.eq_ignore_ascii_case(conversation.as_str())) {
                return Ok(Err(SavedRefusal::IdTaken));
            }
            let Some(document) = remove(&transaction, &conversation, &id)? else {
                // The edit was made already: the message the draft became is kept.
                return Ok(if taken {
                    let draft = stored(&transaction, &conversation)?
                        .unwrap_or_else(|| Stored::written(None, Version::empty()));
                    Ok(draft.present())
                } else {
                    Err(SavedRefusal::NotFound)
                });
            };
            if taken {
                // Another message holds the id the draft would become under.
                return Ok(Err(SavedRefusal::IdTaken));
            }
            let current = stored(&transaction, &conversation)?;
            if let Some(draft) = current.as_ref().filter(|draft| !draft.version.is_empty()) {
                insert(&transaction, &conversation, &becomes, draft.version.clone(), now)?;
            }
            let mut draft = Stored::written(current, document);
            draft.saved_revision += 1;
            write(&transaction, &conversation, &draft, now.as_millisecond())?;
            transaction.commit()?;
            Ok(Ok(draft.present()))
        })
        .await
    }

    /// Removes the saved message `id`.
    pub async fn delete_saved(
        &self,
        conversation: ConversationId,
        id: String,
    ) -> Result<Result<(), SavedRefusal>, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            if archived(&transaction, &conversation)? {
                return Ok(Err(SavedRefusal::Archived));
            }
            if remove(&transaction, &conversation, &id)?.is_none() {
                return Ok(Err(SavedRefusal::NotFound));
            }
            raise(&transaction, &conversation, now)?;
            transaction.commit()?;
            Ok(Ok(()))
        })
        .await
    }

    /// Takes the saved message `id` off the list as it is sent: true when
    /// the list held it, so that two sends of one message find it once.
    pub async fn take_saved(
        &self,
        conversation: ConversationId,
        id: String,
    ) -> Result<bool, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            if remove(&transaction, &conversation, &id)?.is_none() {
                return Ok(false);
            }
            raise(&transaction, &conversation, now)?;
            transaction.commit()?;
            Ok(true)
        })
        .await
    }
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
            "SELECT conversation_id FROM saved_messages WHERE id = ?1",
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
    if let Some(kept) = saved(connection, conversation, id.as_str())? {
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
