//! Messages saved for later: lists of them, newest first, each a
//! conversation's own or a project's, and the revision of the list each
//! conversation shows, which its draft row keeps.
//!
//! - A conversation shows its project's list while the project keeps its
//!   messages per project (`workspaces.saved_scope`), and its own otherwise.
//!   Every route names the conversation, and the list is resolved from it in
//!   the transaction that reads or changes it.
//! - Every change of a list raises the revision of each conversation that
//!   shows it, in the transaction that makes it, so a page's revisions of a
//!   conversation only grow, whichever list it shows.
//! - A message is in one list. A project switched to per project takes the
//!   lists of all its conversations; switched back, its list goes to the one
//!   conversation the user names. A conversation that moves into a project
//!   that keeps them per project brings its own; one that moves out takes
//!   none of the project's.
//! - A save of the composer's message empties the draft in the same
//!   transaction, so a message is never both saved and in the draft. An edit
//!   writes a message in its place and never touches the draft.

use demi_shared_types::{Timestamp, TurnId};
use demi_web_api_protocol::drafts::{DraftWritten, Revisions};
use demi_web_api_protocol::ids::{AttachmentId, ConversationId, SavedMessageId, UserId, WorkspaceId};
use demi_web_api_protocol::workspaces::SavedScope;
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
    /// The list keeps [`SAVED_MAX`] messages already.
    Full,
    /// The conversation's list has no saved message of that id.
    NotFound,
    /// A saved message has the id already, with other content or in another
    /// list.
    IdTaken,
}

/// The list of messages saved for later a conversation shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SavedList {
    /// The conversation's own.
    Conversation(ConversationId),
    /// Its project's, which every conversation of the project shows.
    Project(WorkspaceId),
}

impl SavedList {
    /// The column of `saved_messages` that names the list.
    fn column(&self) -> &'static str {
        match self {
            Self::Conversation(_) => "conversation_id",
            Self::Project(_) => "workspace_id",
        }
    }

    fn owner(&self) -> &str {
        match self {
            Self::Conversation(conversation) => conversation.as_str(),
            Self::Project(project) => project.as_str(),
        }
    }

    /// Whether a message of `saved_messages` whose columns are `conversation`
    /// and `project` is in this list.
    fn holds(&self, conversation: Option<&str>, project: Option<&str>) -> bool {
        match self {
            Self::Conversation(own) => conversation.is_some_and(|id| id.eq_ignore_ascii_case(own.as_str())),
            Self::Project(own) => project == Some(own.as_str()),
        }
    }
}

/// The conversations that show one list of saved messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedSharers {
    pub conversations: Vec<ConversationId>,
    /// Whether the list is a project's, whose count the project's record
    /// carries.
    pub project: bool,
    /// The list's key, one per list, which orders its sends.
    pub key: String,
}

/// What a switch of a project's way of keeping saved messages did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeSwitch {
    /// The project keeps them the new way; the conversations whose list
    /// changed, which their pages learn.
    Switched(Vec<ConversationId>),
    /// The user has no project of the id in the sidebar.
    Missing,
    /// The project's list holds messages and the switch named no
    /// conversation to take them.
    DestinationNeeded,
    /// The conversation named to take them is not one of the project's that
    /// is not archived.
    DestinationInvalid,
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
            let list = list_of(&transaction, &conversation)?;
            let mut statement = transaction.prepare_cached(&format!(
                "SELECT id, saved_at,
                   substr(json_extract(document, '$.text'), 1, ?2) AS opening,
                   length(json_extract(document, '$.text')) AS length,
                   json_extract(document, '$.files') AS files
                 FROM saved_messages WHERE {} = ?1
                 ORDER BY saved_at DESC, rowid DESC",
                list.column()
            ))?;
            let messages = statement
                .query_map(params![list.owner(), OPENING_CHARS], |row| Ok(listed_row(row)))?
                .map(|row| row?)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(SavedListing { revision, messages })
        })
        .await
    }

    /// One of the messages saved for later of the list the conversation
    /// shows, when it keeps it.
    pub async fn saved_message(
        &self,
        conversation: ConversationId,
        id: String,
    ) -> Result<Option<SavedMessage>, StorageError> {
        self.call(move |connection, _| saved(connection, &list_of(connection, &conversation)?, &id))
            .await
    }

    /// The conversations that show the list `conversation` shows: those of
    /// its project while the project keeps its messages per project, and
    /// itself otherwise.
    pub async fn saved_sharers(&self, conversation: ConversationId) -> Result<SavedSharers, StorageError> {
        self.call(move |connection, _| {
            let list = list_of(connection, &conversation)?;
            Ok(SavedSharers {
                conversations: sharers(connection, &list)?,
                project: matches!(list, SavedList::Project(_)),
                key: format!("{}:{}", list.column(), list.owner()),
            })
        })
        .await
    }

    /// Makes the user's project `project` keep its saved messages `scope`'s
    /// way, moving them in the same transaction: to per project, every
    /// conversation's list joins the project's; to per conversation, the
    /// project's list goes to `to`, a conversation of the project that is
    /// not archived, which a list with messages needs.
    pub async fn switch_saved_scope(
        &self,
        user: UserId,
        project: WorkspaceId,
        scope: SavedScope,
        to: Option<ConversationId>,
    ) -> Result<ScopeSwitch, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let current: Option<String> = transaction
                .query_row(
                    "SELECT saved_scope FROM workspaces WHERE id = ?1 AND user_id = ?2 AND deleted_at IS NULL",
                    params![project.as_str(), user.as_str()],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(current) = current else {
                return Ok(ScopeSwitch::Missing);
            };
            if current == scope_column(scope) {
                return Ok(ScopeSwitch::Switched(Vec::new()));
            }
            match scope {
                SavedScope::Project => {
                    transaction.execute(
                        "UPDATE saved_messages SET workspace_id = ?1, conversation_id = NULL
                         WHERE conversation_id IN (SELECT id FROM conversations WHERE target_workspace_id = ?1)",
                        [project.as_str()],
                    )?;
                }
                SavedScope::Conversation => {
                    let kept: i64 = transaction.query_row(
                        "SELECT count(*) FROM saved_messages WHERE workspace_id = ?1",
                        [project.as_str()],
                        |row| row.get(0),
                    )?;
                    if kept > 0 {
                        let Some(to) = to else {
                            return Ok(ScopeSwitch::DestinationNeeded);
                        };
                        let inside = transaction
                            .query_row(
                                "SELECT id FROM conversations
                                 WHERE id = ?1 AND user_id = ?2 AND target_workspace_id = ?3 AND archived = 0",
                                params![to.as_str(), user.as_str(), project.as_str()],
                                |row| row.get::<_, String>(0),
                            )
                            .optional()?;
                        let Some(inside) = inside else {
                            return Ok(ScopeSwitch::DestinationInvalid);
                        };
                        transaction.execute(
                            "UPDATE saved_messages SET conversation_id = ?2, workspace_id = NULL WHERE workspace_id = ?1",
                            params![project.as_str(), inside],
                        )?;
                    }
                }
            }
            transaction.execute(
                "UPDATE workspaces SET saved_scope = ?2 WHERE id = ?1",
                params![project.as_str(), scope_column(scope)],
            )?;
            let changed = sharers(&transaction, &SavedList::Project(project.clone()))?;
            for conversation in &changed {
                raise(&transaction, conversation, now)?;
            }
            transaction.commit()?;
            Ok(ScopeSwitch::Switched(changed))
        })
        .await
    }

    /// Saves the composer's message, `text` and `files`, for later as the message
    /// `id` and empties the conversation's draft, as a save of an empty draft
    /// would, in one transaction.
    ///
    /// Each upload is presented with what its record holds, as a draft's is. A save
    /// sent again, with an `id` the conversation keeps with the same content,
    /// changes nothing and answers the message with the draft as it is.
    pub async fn save_from_draft(
        &self,
        conversation: ConversationId,
        owner: UserId,
        id: SavedMessageId,
        text: String,
        files: Vec<StagedFile>,
    ) -> Result<Result<FromDraftAnswer, SavedRefusal>, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let list = list_of(&transaction, &conversation)?;
            let version = match staged(&transaction, &list, &conversation, &owner, &id, text, files)? {
                Ok(Staged::New(version)) => version,
                Ok(Staged::Again(message)) => {
                    let draft = stored(&transaction, &conversation)?
                        .unwrap_or_else(|| Stored::written(None, Version::empty()));
                    let DraftWritten { draft, revisions } = draft.written_answer();
                    return Ok(Ok(FromDraftAnswer { message, draft, revisions }));
                }
                Err(refusal) => return Ok(Err(refusal)),
            };
            let message = insert(&transaction, &list, &id, version, now)?;
            // The draft's comments stay waiting there: a saved message keeps
            // the text and the files alone.
            let current = stored(&transaction, &conversation)?;
            let comments = current
                .as_ref()
                .map(|draft| draft.version.comments.clone())
                .unwrap_or_default();
            let mut draft = Stored::written(
                current,
                Version {
                    comments,
                    ..Version::empty()
                },
            );
            draft.saved_revision += 1;
            write(&transaction, &conversation, &draft, now.as_millisecond())?;
            for sharer in sharers(&transaction, &list)? {
                if sharer != conversation {
                    raise(&transaction, &sharer, now)?;
                }
            }
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
            let list = list_of(&transaction, &conversation)?;
            let message = match staged(&transaction, &list, &conversation, &owner, &id, text, files)? {
                Ok(Staged::New(version)) => {
                    let message = insert(&transaction, &list, &id, version, now)?;
                    raise_list(&transaction, &list, now)?;
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
            let list = list_of(&transaction, &conversation)?;
            let Some(kept) = saved(&transaction, &list, &id)? else {
                return Ok(Err(SavedRefusal::NotFound));
            };
            if kept.text == version.text && kept.files == version.files {
                let revisions = revisions(&transaction, &conversation)?;
                return Ok(Ok(SavedAnswer { message: kept, revisions }));
            }
            transaction.execute(
                &format!("UPDATE saved_messages SET document = ?3 WHERE {} = ?1 AND id = ?2", list.column()),
                params![list.owner(), id, to_json(&version)],
            )?;
            raise_list(&transaction, &list, now)?;
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
            let list = list_of(&transaction, &conversation)?;
            let removed = remove(&transaction, &list, &id)?;
            if removed.is_some() {
                raise_list(&transaction, &list, now)?;
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
            let list = list_of(&transaction, &conversation)?;
            let Some(message) = saved(&transaction, &list, &id)? else {
                return Ok(false);
            };
            remove(&transaction, &list, &id)?;
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
            raise_list(&transaction, &list, now)?;
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
    /// A message the list may keep: its conversation not archived, every
    /// upload the caller's, and fewer than [`SAVED_MAX`] kept already.
    New(Version),
    /// The save was made already: the list keeps a message of that id with
    /// this content, or the conversation sent it.
    Again(SavedMessage),
}

/// The message a save of `id` into `list` from `conversation` names, or the
/// one kept when that save was made before. An id the list keeps with other
/// content, another list keeps, or another conversation sent, is taken.
fn staged(
    connection: &Connection,
    list: &SavedList,
    conversation: &ConversationId,
    owner: &UserId,
    id: &SavedMessageId,
    text: String,
    files: Vec<StagedFile>,
) -> Result<Result<Staged, SavedRefusal>, StorageError> {
    let kept_by: Option<(Option<String>, Option<String>)> = connection
        .query_row(
            "SELECT conversation_id, workspace_id FROM saved_messages WHERE id = ?1
             UNION ALL SELECT conversation_id, NULL FROM saved_sends WHERE id = ?1",
            [id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let elsewhere = kept_by.is_some_and(|(by_conversation, by_project)| {
        !list.holds(by_conversation.as_deref(), by_project.as_deref())
            && !by_conversation.is_some_and(|by| by_project.is_none() && by.eq_ignore_ascii_case(conversation.as_str()))
    });
    if elsewhere {
        return Ok(Err(SavedRefusal::IdTaken));
    }
    if archived(connection, conversation)? {
        return Ok(Err(SavedRefusal::Archived));
    }
    let version = match staged_version(connection, owner, text, files)? {
        Ok(version) => version,
        Err(missing) => return Ok(Err(SavedRefusal::UploadNotFound(missing))),
    };
    if let Some(kept) = saved(connection, list, id.as_str())?
        .map_or_else(|| sent(connection, conversation, id.as_str()), |kept| Ok(Some(kept)))?
    {
        return Ok(if kept.text == version.text && kept.files == version.files {
            Ok(Staged::Again(kept))
        } else {
            Err(SavedRefusal::IdTaken)
        });
    }
    let kept: i64 = connection.query_row(
        &format!("SELECT count(*) FROM saved_messages WHERE {} = ?1", list.column()),
        [list.owner()],
        |row| row.get(0),
    )?;
    if kept >= count(SAVED_MAX) {
        return Ok(Err(SavedRefusal::Full));
    }
    Ok(Ok(Staged::New(version)))
}

fn saved(
    connection: &Connection,
    list: &SavedList,
    id: &str,
) -> Result<Option<SavedMessage>, StorageError> {
    connection
        .query_row(
            &format!("SELECT id, saved_at, document FROM saved_messages WHERE {} = ?1 AND id = ?2", list.column()),
            [list.owner(), id],
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

/// Saves `version` as the message `id` of `list`, saved `now`.
fn insert(
    connection: &Connection,
    list: &SavedList,
    id: &SavedMessageId,
    version: Version,
    now: Timestamp,
) -> Result<SavedMessage, StorageError> {
    connection.execute(
        &format!("INSERT INTO saved_messages (id, {}, saved_at, document) VALUES (?1, ?2, ?3, ?4)", list.column()),
        params![id.as_str(), list.owner(), now.as_millisecond(), to_json(&version)],
    )?;
    Ok(SavedMessage {
        id: id.to_string(),
        text: version.text,
        files: version.files,
        saved_at: now,
    })
}

/// Removes the saved message `id` of `list`; the message as it was when the
/// list kept it.
fn remove(
    connection: &Connection,
    list: &SavedList,
    id: &str,
) -> Result<Option<SavedMessage>, StorageError> {
    connection
        .query_row(
            &format!(
                "DELETE FROM saved_messages WHERE {} = ?1 AND id = ?2 RETURNING id, saved_at, document",
                list.column()
            ),
            [list.owner(), id],
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

/// Raises the revision of each conversation that shows `list`; answers them.
fn raise_list(connection: &Connection, list: &SavedList, now: Timestamp) -> Result<Vec<ConversationId>, StorageError> {
    let conversations = sharers(connection, list)?;
    for conversation in &conversations {
        raise(connection, conversation, now)?;
    }
    Ok(conversations)
}

/// The list `conversation` shows: its project's while the project keeps its
/// messages per project, its own otherwise.
pub(crate) fn list_of(connection: &Connection, conversation: &ConversationId) -> Result<SavedList, StorageError> {
    let project: Option<String> = connection
        .prepare_cached(
            "SELECT workspace.id FROM conversations conversation
             JOIN workspaces workspace ON workspace.id = conversation.target_workspace_id
             WHERE conversation.id = ?1 AND workspace.saved_scope = 'project'",
        )?
        .query_row([conversation.as_str()], |row| row.get(0))
        .optional()?;
    Ok(match project {
        Some(project) => SavedList::Project(decode("workspaces", "id", WorkspaceId::try_from(project))?),
        None => SavedList::Conversation(conversation.clone()),
    })
}

/// The conversations that show `list`.
fn sharers(connection: &Connection, list: &SavedList) -> Result<Vec<ConversationId>, StorageError> {
    match list {
        SavedList::Conversation(conversation) => Ok(vec![conversation.clone()]),
        SavedList::Project(project) => members(connection, project),
    }
}

/// The conversations of the project `project`, archived ones included.
fn members(connection: &Connection, project: &WorkspaceId) -> Result<Vec<ConversationId>, StorageError> {
    connection
        .prepare_cached("SELECT id FROM conversations WHERE target_workspace_id = ?1")?
        .query_map([project.as_str()], |row| row.get::<_, String>(0))?
        .map(|id| decode("conversations", "id", ConversationId::try_from(id?.as_str())))
        .collect()
}

/// Whether the project `project` keeps its saved messages per project.
fn shared(connection: &Connection, project: &WorkspaceId) -> Result<bool, StorageError> {
    Ok(connection
        .prepare_cached("SELECT 1 FROM workspaces WHERE id = ?1 AND saved_scope = 'project'")?
        .query_row([project.as_str()], |_| Ok(()))
        .optional()?
        .is_some())
}

/// `scope` as `workspaces.saved_scope` stores it.
pub(crate) fn scope_column(scope: SavedScope) -> &'static str {
    match scope {
        SavedScope::Conversation => "conversation",
        SavedScope::Project => "project",
    }
}

/// What a move of `conversation` from the project `from` to the project `to`,
/// either none for outside projects, does to the saved messages, after the
/// move is written: into a project that keeps them per project, its own
/// join the project's; out of one, it shows its own again, which is empty.
pub(crate) fn moved(
    connection: &Connection,
    conversation: &ConversationId,
    from: Option<&WorkspaceId>,
    to: Option<&WorkspaceId>,
    now: Timestamp,
) -> Result<(), StorageError> {
    if from == to {
        return Ok(());
    }
    let into = match to {
        Some(to) if shared(connection, to)? => Some(to),
        _ => None,
    };
    let left = match from {
        Some(from) => shared(connection, from)?,
        None => false,
    };
    if let Some(into) = into {
        let joined = connection.execute(
            "UPDATE saved_messages SET workspace_id = ?2, conversation_id = NULL WHERE conversation_id = ?1",
            params![conversation.as_str(), into.as_str()],
        )?;
        if joined > 0 {
            raise_list(connection, &SavedList::Project(into.clone()), now)?;
            return Ok(());
        }
    }
    if into.is_some() || left {
        raise(connection, conversation, now)?;
    }
    Ok(())
}

/// What the end of the project `from`, whose conversations go into the
/// project `into` or out of projects, does to the saved messages, before its
/// conversations move: into a project that keeps them per project, its list
/// and each conversation's own join the receiving one's. Otherwise its list,
/// if it keeps one, goes to the conversation changed last of it, else of
/// `into`, else into `into`'s own list, which shows once `into` keeps its
/// messages per project; it goes with `from` only out of projects with no
/// conversation to take it, as when its device is removed. Answers the
/// conversations whose list changed, whose pages read it again.
pub(crate) fn project_ends(
    connection: &Connection,
    from: &WorkspaceId,
    into: Option<&WorkspaceId>,
    now: Timestamp,
) -> Result<Vec<ConversationId>, StorageError> {
    let mut changed = members(connection, from)?;
    match into {
        Some(into) if shared(connection, into)? => {
            connection.execute(
                "UPDATE saved_messages SET workspace_id = ?2, conversation_id = NULL
                 WHERE workspace_id = ?1
                    OR conversation_id IN (SELECT id FROM conversations WHERE target_workspace_id = ?1)",
                params![from.as_str(), into.as_str()],
            )?;
            changed.extend(raise_list(connection, &SavedList::Project(into.clone()), now)?);
        }
        _ => {
            let last_of = |project: &WorkspaceId| -> Result<Option<String>, StorageError> {
                Ok(connection
                    .query_row(
                        "SELECT id FROM conversations WHERE target_workspace_id = ?1
                         ORDER BY updated_at DESC, id LIMIT 1",
                        [project.as_str()],
                        |row| row.get(0),
                    )
                    .optional()?)
            };
            let taker = match last_of(from)? {
                Some(last) => Some(last),
                None => match into {
                    Some(into) => last_of(into)?,
                    None => None,
                },
            };
            match (taker, into) {
                (Some(taker), _) => {
                    connection.execute(
                        "UPDATE saved_messages SET conversation_id = ?2, workspace_id = NULL WHERE workspace_id = ?1",
                        params![from.as_str(), taker],
                    )?;
                    let taker = decode("conversations", "id", ConversationId::try_from(taker.as_str()))?;
                    raise(connection, &taker, now)?;
                    changed.push(taker);
                }
                (None, Some(into)) => {
                    connection.execute(
                        "UPDATE saved_messages SET workspace_id = ?2 WHERE workspace_id = ?1",
                        params![from.as_str(), into.as_str()],
                    )?;
                }
                (None, None) => {}
            }
        }
    }
    for conversation in &members(connection, from)? {
        raise(connection, conversation, now)?;
    }
    changed.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    changed.dedup();
    Ok(changed)
}
