//! Messages saved for later (`storage.md` § Control records, `web-api.md`
//! § Messages saved for later): each conversation's list, newest first, and
//! its revision, which the conversation's draft row keeps. Every change of
//! the list raises the revision in the transaction that makes it; a save and
//! an edit change the draft in the same transaction, so a message is never
//! both saved and in the draft.

use demi_shared_types::Timestamp;
use demi_web_api_protocol::drafts::ConversationDraft;
use demi_web_api_protocol::ids::{AttachmentId, ConversationId, UserId};
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

    /// Saves the composer's message, `text` and `files`, for later and
    /// empties the conversation's draft, as a save of an empty draft built on
    /// the revision `base` would, in one transaction. Each upload is
    /// presented with what its record holds, as a draft's is.
    pub async fn save_from_draft(
        &self,
        conversation: ConversationId,
        owner: UserId,
        base: u64,
        text: String,
        files: Vec<StagedFile>,
    ) -> Result<Result<(SavedMessage, ConversationDraft), SavedRefusal>, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let version = match staged(&transaction, &conversation, &owner, text, files)? {
                Ok(version) => version,
                Err(refusal) => return Ok(Err(refusal)),
            };
            let message = insert(&transaction, &conversation, version, now)?;
            let mut draft =
                Stored::saved_on(stored(&transaction, &conversation)?, base, Version::empty());
            draft.saved_revision += 1;
            write(&transaction, &conversation, &draft, now.as_millisecond())?;
            transaction.commit()?;
            Ok(Ok((message, draft.present())))
        })
        .await
    }

    /// Adds `text` and `files` as a message saved for later, leaving the
    /// draft as it is, as Undo after a removal does.
    pub async fn add_saved(
        &self,
        conversation: ConversationId,
        owner: UserId,
        text: String,
        files: Vec<StagedFile>,
    ) -> Result<Result<SavedMessage, SavedRefusal>, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let version = match staged(&transaction, &conversation, &owner, text, files)? {
                Ok(version) => version,
                Err(refusal) => return Ok(Err(refusal)),
            };
            let message = insert(&transaction, &conversation, version, now)?;
            raise(&transaction, &conversation, now)?;
            transaction.commit()?;
            Ok(Ok(message))
        })
        .await
    }

    /// Moves the saved message `id` into the conversation's draft, and the
    /// draft's text and files, when it holds any, into a message saved for
    /// later, in one transaction.
    pub async fn edit_saved(
        &self,
        conversation: ConversationId,
        id: String,
    ) -> Result<Result<ConversationDraft, SavedRefusal>, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            if archived(&transaction, &conversation)? {
                return Ok(Err(SavedRefusal::Archived));
            }
            let Some(document) = remove(&transaction, &conversation, &id)? else {
                return Ok(Err(SavedRefusal::NotFound));
            };
            let current = stored(&transaction, &conversation)?;
            if let Some(draft) = current.as_ref().filter(|draft| !draft.version.is_empty()) {
                insert(&transaction, &conversation, draft.version.clone(), now)?;
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

/// The message a save names, when the conversation may keep it: not
/// archived, every upload the caller's, within a draft's limit, and fewer
/// than [`SAVED_MAX`] kept already.
fn staged(
    connection: &Connection,
    conversation: &ConversationId,
    owner: &UserId,
    text: String,
    files: Vec<StagedFile>,
) -> Result<Result<Version, SavedRefusal>, StorageError> {
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
    let kept: i64 = connection.query_row(
        "SELECT count(*) FROM saved_messages WHERE conversation_id = ?1",
        [conversation.as_str()],
        |row| row.get(0),
    )?;
    if kept >= count(SAVED_MAX) {
        return Ok(Err(SavedRefusal::Full));
    }
    Ok(Ok(version))
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

/// Saves `version` as a new message of the conversation, saved `now`.
fn insert(
    connection: &Connection,
    conversation: &ConversationId,
    version: Version,
    now: Timestamp,
) -> Result<SavedMessage, StorageError> {
    let id = uuid::Uuid::new_v4().to_string();
    connection.execute(
        "INSERT INTO saved_messages (id, conversation_id, saved_at, document) VALUES (?1, ?2, ?3, ?4)",
        params![id, conversation.as_str(), now.as_millisecond(), to_json(&version)],
    )?;
    Ok(SavedMessage {
        id,
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
