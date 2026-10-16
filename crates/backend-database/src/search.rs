//! Each user's search index (`storage.md` § Search index):
//! `search/<userId>.sqlite`, an FTS5 table over one row per searchable text,
//! a conversation's title or the text of a message, an answer or a
//! command's title of any of its agents, with the saved version of each
//! agent's transcript it was built from. The product's search finds titles
//! and the root's messages and answers; `demi conversation history search`
//! finds an agent's own rows.
//!
//! The index is derived from the conversations, so it has no history and no
//! migration: an index of another schema, or one that fails to open, is
//! deleted and built again empty, and announced, so the backend's indexer
//! fills it. Every operation takes its user's turn and runs on a short-lived
//! connection on the blocking pool, so a rebuild never removes a file
//! another operation has open, and no index holds a thread while nobody
//! searches. A row is known by its conversation and block; its integer id
//! is FTS5's handle on it within one read alone, since SQLite gives a
//! removed row's id to the next one.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use demi_shared_gates::KeyedSerialGate;
use demi_shared_types::{Block, BlockId, Timestamp, UserContentBlock};
use demi_web_api_protocol::ids::{ConversationId, UserId};
use demi_web_api_protocol::search::LINE_MAX;
use tokio::sync::broadcast;
use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params, params_from_iter};

use super::columns::{count, decode};
use super::tree::SavedVersion;
use super::{StorageError, schema, sqlite};

/// The schema. The `texts` table holds the texts once; `texts_fts` indexes
/// them as its external content, kept in step by the triggers. FTS5's
/// `trigram` tokenizer matches any piece of three characters or more, case
/// ignored, in every language, without splitting words.
const SCHEMA: &str = r"
-- Each indexed conversation.
CREATE TABLE conversations (
  id TEXT PRIMARY KEY COLLATE NOCASE
) STRICT;

-- Each indexed agent of a conversation, by its number, 0 for the root,
-- with the saved version of its transcript the rows were built from.
CREATE TABLE agents (
  conversation TEXT NOT NULL COLLATE NOCASE,
  agent        INTEGER NOT NULL CHECK (agent >= 0),
  revision     INTEGER NOT NULL CHECK (revision >= 0),
  blocks       INTEGER NOT NULL CHECK (blocks >= 0),
  PRIMARY KEY (conversation, agent)
) STRICT;

-- One row per searchable text: a conversation's title, which has no agent
-- and no block, or the text of an agent's message, answer or command's
-- title, at the block's index, with the block's time in milliseconds since
-- the Unix epoch.
CREATE TABLE texts (
  id           INTEGER PRIMARY KEY,
  conversation TEXT NOT NULL COLLATE NOCASE,
  agent        INTEGER CHECK (agent >= 0),
  kind         TEXT CHECK (kind IN ('message', 'answer', 'command')),
  block        TEXT,
  position     INTEGER CHECK (position >= 0),
  time         INTEGER,
  text         TEXT NOT NULL,
  CHECK ((block IS NULL) = (position IS NULL)),
  CHECK ((block IS NULL) = (agent IS NULL)),
  CHECK ((block IS NULL) = (kind IS NULL)),
  CHECK ((block IS NULL) = (time IS NULL))
) STRICT;
CREATE UNIQUE INDEX texts_block ON texts (conversation, agent, block) WHERE block IS NOT NULL;
CREATE UNIQUE INDEX texts_title ON texts (conversation) WHERE block IS NULL;
CREATE INDEX texts_position ON texts (conversation, agent, position);

CREATE VIRTUAL TABLE texts_fts USING fts5(
  text,
  content = 'texts',
  content_rowid = 'id',
  tokenize = 'trigram'
);
CREATE TRIGGER texts_added AFTER INSERT ON texts BEGIN
  INSERT INTO texts_fts (rowid, text) VALUES (new.id, new.text);
END;
CREATE TRIGGER texts_removed AFTER DELETE ON texts BEGIN
  INSERT INTO texts_fts (texts_fts, rowid, text) VALUES ('delete', old.id, old.text);
END;
";

/// The fewest characters FTS5's `trigram` tokenizer matches; a shorter
/// word is matched by a scan of the user's texts.
const TRIGRAM: usize = 3;

/// Every user's search index. Cloning it is cheap.
#[derive(Clone)]
pub struct SearchIndexes(Arc<Indexes>);

struct Indexes {
    directory: PathBuf,
    /// One operation at a time per user.
    turns: KeyedSerialGate<UserId>,
    /// Each user whose index was deleted and built again empty.
    resets: broadcast::Sender<UserId>,
    #[cfg(any(test, feature = "testing"))]
    work: std::sync::Mutex<SearchWork>,
}

/// How much work the indexes have done, which tests bound by the sizes of
/// what changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SearchWork {
    /// Conversations indexed whole.
    pub whole: u64,
    /// Rows added or removed, by a whole indexing or a save's changes.
    pub rows: u64,
    /// Texts a query read to answer with.
    pub texts: u64,
}

/// How many resets wait for a slow follower before it hears that it missed
/// some (`RecvError::Lagged`).
const RESETS_WAITING: usize = 64;

/// What the index holds of one conversation: the saved version of each
/// agent's transcript, by the agent's number, and the title it was indexed
/// with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Indexed {
    pub versions: BTreeMap<u64, SavedVersion>,
    pub title: String,
}

/// A conversation as the indexer gives it to the index.
#[derive(Debug, Clone)]
pub struct IndexedConversation {
    pub title: String,
    pub agents: Vec<IndexedAgent>,
}

/// One agent of a conversation as the indexer gives it to the index: its
/// number, the saved version of its transcript, and the searchable text of
/// its blocks, oldest first.
#[derive(Debug, Clone)]
pub struct IndexedAgent {
    pub agent: u64,
    pub version: SavedVersion,
    pub messages: Vec<IndexedMessage>,
}

/// The searchable text of one block: the block and its index.
#[derive(Debug, Clone)]
pub struct IndexedMessage {
    pub block: BlockId,
    pub position: usize,
    pub text: Searchable,
}

/// What a block holds that a search finds: which kind of text, the text,
/// and the block's time, which a history search may bound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Searchable {
    pub kind: SearchKind,
    pub text: String,
    pub time: Timestamp,
}

/// The kinds of a block's searchable text. The product's search finds a
/// root's messages and answers; an agent's history search finds all three.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchKind {
    /// A `user` or `steer` block's text.
    Message,
    /// An agent's text.
    Answer,
    /// The title of a command a `shell` call ran.
    Command,
}

impl SearchKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Message => "message",
            Self::Answer => "answer",
            Self::Command => "command",
        }
    }
}

/// What `demi conversation history search` looks for in an agent's rows
/// (`compaction.md` § Reading the earlier history).
#[derive(Debug, Clone)]
pub struct HistoryQuery {
    /// The words a text holds every one of, or any with `any`.
    pub words: Vec<String>,
    pub any: bool,
    /// The kinds of text searched; every kind when empty.
    pub kinds: Vec<SearchKind>,
    /// What a text lies after: a block, or a moment, from which on.
    pub after: Option<HistoryBound>,
    /// What a text lies before: a block, or a moment, up to which.
    pub before: Option<HistoryBound>,
    /// The oldest first, rather than the newest.
    pub oldest: bool,
}

/// One end of a history search: a block's index, which the texts lie
/// strictly after or before, or a moment, from which on or up to which they
/// lie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryBound {
    Index(usize),
    Time(Timestamp),
}

/// A conversation a query found: whether its title holds every word, and
/// the newest of its messages that does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub conversation: ConversationId,
    pub title: bool,
    pub message: Option<FoundMessage>,
}

/// A message that holds every word: its block, and the line around the
/// first place a word matches with the places of every word in it, read
/// together with the block (`match_line`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundMessage {
    pub block: BlockId,
    pub line: String,
    pub ranges: Vec<[u32; 2]>,
}

/// What one save changed of an agent's transcript (`storage.md` § Search
/// index): the agent's number, the saved version it follows, the one it
/// brought the transcript to, whose block count is where it cut the rest,
/// and each block it wrote.
#[derive(Debug, Clone)]
pub struct TranscriptChange {
    pub agent: u64,
    pub before: SavedVersion,
    pub version: SavedVersion,
    pub written: Vec<WrittenBlock>,
}

impl TranscriptChange {
    /// The change a save of the agent `agent` from `before` to `version`
    /// made by writing `blocks`, each at its index, with their searchable
    /// text alone.
    pub fn new(agent: u64, before: SavedVersion, version: SavedVersion, blocks: &[(usize, Block)]) -> Self {
        Self {
            agent,
            before,
            version,
            written: blocks
                .iter()
                .map(|(position, block)| WrittenBlock {
                    block: block.id().clone(),
                    position: *position,
                    text: searchable_text(block),
                })
                .collect(),
        }
    }
}

/// A block a save wrote, at its index, with its searchable text; none when
/// it has none, such as thinking.
#[derive(Debug, Clone)]
pub struct WrittenBlock {
    pub block: BlockId,
    pub position: usize,
    pub text: Option<Searchable>,
}

impl SearchIndexes {
    /// The indexes in `directory`, which is created if it is missing.
    pub async fn open(directory: PathBuf) -> Result<Self, StorageError> {
        tokio::fs::create_dir_all(&directory).await?;
        Ok(Self(Arc::new(Indexes {
            directory,
            turns: KeyedSerialGate::new(),
            resets: broadcast::channel(RESETS_WAITING).0,
            #[cfg(any(test, feature = "testing"))]
            work: std::sync::Mutex::default(),
        })))
    }

    /// How much work the indexes have done so far.
    #[cfg(any(test, feature = "testing"))]
    pub fn work(&self) -> SearchWork {
        *self.0.work.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Adds to the work done; nothing outside tests.
    #[cfg_attr(not(any(test, feature = "testing")), allow(unused_variables))]
    fn record(&self, add: impl FnOnce(&mut SearchWork)) {
        #[cfg(any(test, feature = "testing"))]
        add(&mut self.0.work.lock().unwrap_or_else(std::sync::PoisonError::into_inner));
    }

    /// Each user whose index is deleted and built again empty from now on,
    /// whose conversations the backend then indexes again.
    pub fn resets(&self) -> broadcast::Receiver<UserId> {
        self.0.resets.subscribe()
    }

    /// Runs `work` on a connection of `user`'s index, in the user's turn, on
    /// the blocking pool.
    async fn with<T: Send + 'static>(
        &self,
        user: &UserId,
        work: impl FnOnce(&mut Connection) -> Result<T, StorageError> + Send + 'static,
    ) -> Result<T, StorageError> {
        let _turn = self.0.turns.acquire(user.clone()).await;
        let path = self.0.directory.join(format!("{user}.sqlite"));
        let indexes = self.0.clone();
        let user = user.clone();
        tokio::task::spawn_blocking(move || {
            let (mut connection, reset) = open_index(&path)?;
            if reset {
                // No follower is the backend starting or a test without
                // one: the start indexes every conversation anyway.
                let _ = indexes.resets.send(user);
            }
            work(&mut connection)
        })
        .await?
    }

    /// What the index holds of the conversation; none when it holds nothing.
    pub async fn indexed(
        &self,
        user: &UserId,
        conversation: &ConversationId,
    ) -> Result<Option<Indexed>, StorageError> {
        let conversation = conversation.clone();
        self.with(user, move |connection| {
            let title: Option<String> = connection
                .query_row(
                    "SELECT t.text FROM conversations c
                     JOIN texts t ON t.conversation = c.id AND t.block IS NULL
                     WHERE c.id = ?1",
                    [conversation.as_str()],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(title) = title else {
                return Ok(None);
            };
            let mut statement =
                connection.prepare("SELECT agent, revision, blocks FROM agents WHERE conversation = ?1")?;
            let mut rows = statement.query([conversation.as_str()])?;
            let mut versions = BTreeMap::new();
            while let Some(row) = rows.next()? {
                versions.insert(
                    decode("agents", "agent", u64::try_from(row.get::<_, i64>(0)?))?,
                    version_of(row.get(1)?, row.get(2)?)?,
                );
            }
            Ok(Some(Indexed { versions, title }))
        })
        .await
    }

    /// Every conversation the index holds.
    pub async fn conversations(&self, user: &UserId) -> Result<Vec<ConversationId>, StorageError> {
        self.with(user, |connection| {
            let mut statement = connection.prepare("SELECT id FROM conversations")?;
            let mut rows = statement.query([])?;
            let mut ids = Vec::new();
            while let Some(row) = rows.next()? {
                ids.push(decode(
                    "conversations",
                    "id",
                    ConversationId::try_from(row.get::<_, String>(0)?),
                )?);
            }
            Ok(ids)
        })
        .await
    }

    /// Replaces the conversation's rows with `indexed`, in one transaction,
    /// so a search sees it either as it was or as it is.
    pub async fn index(
        &self,
        user: &UserId,
        conversation: &ConversationId,
        indexed: IndexedConversation,
    ) -> Result<(), StorageError> {
        let conversation = conversation.clone();
        let rows = self
            .with(user, move |connection| {
                let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let mut rows = remove_rows(&transaction, &conversation)?;
                transaction.execute("INSERT INTO conversations (id) VALUES (?1)", [conversation.as_str()])?;
                insert_title(&transaction, &conversation, &indexed.title)?;
                rows += 1;
                for agent in &indexed.agents {
                    set_version(&transaction, &conversation, agent.agent, agent.version)?;
                    for message in &agent.messages {
                        insert_text(&transaction, &conversation, agent.agent, message)?;
                    }
                    rows += agent.messages.len();
                }
                transaction.commit()?;
                Ok(rows)
            })
            .await?;
        self.record(|work| {
            work.whole += 1;
            work.rows += count_of(rows);
        });
        Ok(())
    }

    /// Applies one save's `change` to the rows of its agent, in one
    /// transaction, when the index holds the agent at exactly the version
    /// the change follows, an agent it holds nothing of at the empty one: each written block's row replaced when
    /// its text or its index changed and removed when it has no text, any
    /// other block's row at its index removed, the rows from the change's
    /// block count on removed, and the change's version recorded. Answers
    /// false, changing nothing, otherwise: the index lacks the conversation
    /// or a change before this one, and the conversation has to be read
    /// whole.
    pub async fn apply(
        &self,
        user: &UserId,
        conversation: &ConversationId,
        change: TranscriptChange,
    ) -> Result<bool, StorageError> {
        let conversation = conversation.clone();
        let applied = self
            .with(user, move |connection| {
                let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                if !holds(&transaction, &conversation)? {
                    return Ok(None);
                }
                let held = held_version(&transaction, &conversation, change.agent)?.unwrap_or_default();
                if held != change.before {
                    return Ok(None);
                }
                let (id, agent) = (conversation.as_str(), integer(change.agent));
                let mut rows = transaction
                    .prepare_cached("DELETE FROM texts WHERE conversation = ?1 AND agent = ?2 AND position >= ?3")?
                    .execute(params![id, agent, integer(change.version.blocks)])?;
                for written in &change.written {
                    let (block, position) = (written.block.as_str(), count(written.position));
                    rows += transaction
                        .prepare_cached(
                            "DELETE FROM texts WHERE conversation = ?1 AND agent = ?2 AND position = ?3 AND block <> ?4",
                        )?
                        .execute(params![id, agent, position, block])?;
                    let held: Option<(i64, String, String, i64)> = transaction
                        .prepare_cached(
                            "SELECT position, kind, text, time FROM texts WHERE conversation = ?1 AND agent = ?2 AND block = ?3",
                        )?
                        .query_row(params![id, agent, block], |row| {
                            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                        })
                        .optional()?;
                    let unchanged = match (&held, &written.text) {
                        (Some((at, kind, held, time)), Some(text)) => {
                            *at == position
                                && kind == text.kind.as_str()
                                && *held == text.text
                                && *time == text.time.as_millisecond()
                        }
                        (None, None) => true,
                        _ => false,
                    };
                    if unchanged {
                        continue;
                    }
                    if held.is_some() {
                        rows += transaction
                            .prepare_cached("DELETE FROM texts WHERE conversation = ?1 AND agent = ?2 AND block = ?3")?
                            .execute(params![id, agent, block])?;
                    }
                    if let Some(text) = &written.text {
                        let message = IndexedMessage {
                            block: written.block.clone(),
                            position: written.position,
                            text: text.clone(),
                        };
                        insert_text(&transaction, &conversation, change.agent, &message)?;
                        rows += 1;
                    }
                }
                set_version(&transaction, &conversation, change.agent, change.version)?;
                transaction.commit()?;
                Ok(Some(rows))
            })
            .await?;
        let Some(rows) = applied else {
            return Ok(false);
        };
        self.record(|work| work.rows += count_of(rows));
        Ok(true)
    }

    /// Replaces the conversation's title row alone. Answers false, changing
    /// nothing, when the index does not hold the conversation.
    pub async fn retitle(
        &self,
        user: &UserId,
        conversation: &ConversationId,
        title: String,
    ) -> Result<bool, StorageError> {
        let conversation = conversation.clone();
        self.with(user, move |connection| {
            let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            if !holds(&transaction, &conversation)? {
                return Ok(false);
            }
            transaction.execute(
                "DELETE FROM texts WHERE conversation = ?1 AND block IS NULL",
                [conversation.as_str()],
            )?;
            insert_title(&transaction, &conversation, &title)?;
            transaction.commit()?;
            Ok(true)
        })
        .await
    }

    /// Removes the conversation's rows, as its deletion does
    /// (`storage.md` § Deleting a conversation); one the index does not
    /// hold removes nothing.
    pub async fn remove_conversation(
        &self,
        user: &UserId,
        conversation: &ConversationId,
    ) -> Result<(), StorageError> {
        let conversation = conversation.clone();
        self.with(user, move |connection| {
            let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            remove_rows(&transaction, &conversation)?;
            transaction.commit()?;
            Ok(())
        })
        .await
    }

    /// The block indices of the texts of the conversation's agent `agent`
    /// that `query` finds: those of its kinds and between its bounds that
    /// hold every one of its words, or any, case ignored, newest first or
    /// oldest first as it says: what `demi conversation history search`
    /// finds (`compaction.md` § Reading the earlier history). A word is
    /// found as [`SearchIndexes::find`] finds it: through the trigram index,
    /// or by a scan when it is shorter.
    pub async fn find_in(
        &self,
        user: &UserId,
        conversation: &ConversationId,
        agent: u64,
        query: HistoryQuery,
    ) -> Result<Vec<usize>, StorageError> {
        let conversation = conversation.clone();
        self.with(user, move |connection| {
            let mut matches = Vec::new();
            // `?1` and `?2` come first, so each `?` after them is the next
            // condition's, in the order the conditions are written.
            let mut values = vec![Value::Text(conversation.as_str().to_owned()), Value::Integer(integer(agent))];
            for word in &query.words {
                if word.chars().count() >= TRIGRAM {
                    // The word a phrase, so FTS5 reads none of it as an
                    // operator.
                    matches.push("t.id IN (SELECT rowid FROM texts_fts WHERE texts_fts MATCH ?)");
                    values.push(Value::Text(format!("\"{}\"", word.replace('"', "\"\""))));
                } else {
                    matches.push(r"t.text LIKE ? ESCAPE '\'");
                    values.push(Value::Text(format!("%{}%", escape_like(word))));
                }
            }
            let mut conditions = vec![format!("({})", matches.join(if query.any { " OR " } else { " AND " }))];
            if !query.kinds.is_empty() {
                let marks = vec!["?"; query.kinds.len()].join(", ");
                conditions.push(format!("t.kind IN ({marks})"));
                values.extend(query.kinds.iter().map(|kind| Value::Text(kind.as_str().to_owned())));
            }
            for (bound, index, time) in [
                (query.after, "t.position > ?", "t.time >= ?"),
                (query.before, "t.position < ?", "t.time < ?"),
            ] {
                match bound {
                    Some(HistoryBound::Index(at)) => {
                        conditions.push(index.to_owned());
                        values.push(Value::Integer(count(at)));
                    }
                    Some(HistoryBound::Time(moment)) => {
                        conditions.push(time.to_owned());
                        values.push(Value::Integer(moment.as_millisecond()));
                    }
                    None => {}
                }
            }
            let order = if query.oldest { "ASC" } else { "DESC" };
            let mut statement = connection.prepare(&format!(
                "SELECT t.position FROM texts t
                 WHERE t.conversation = ?1 AND t.agent = ?2 AND {}
                 ORDER BY t.position {order}",
                conditions.join(" AND ")
            ))?;
            let mut rows = statement.query(params_from_iter(values))?;
            let mut found = Vec::new();
            while let Some(row) = rows.next()? {
                found.push(decode("texts", "position", usize::try_from(row.get::<_, i64>(0)?))?);
            }
            Ok(found)
        })
        .await
    }

    /// The conversations with a text that holds every one of `words`, case
    /// ignored, among `ranked`, which lists the conversations a result may
    /// name in the order results of a kind come in: at most `limit` of them,
    /// those whose title holds every word first, each with the newest of its
    /// messages that does, cut to its line around the first match. The
    /// matches and the lines come from one read of the index, and only the
    /// texts of the conversations answered with are read. A word of fewer
    /// than three characters is matched by a scan.
    pub async fn find(
        &self,
        user: &UserId,
        words: Vec<String>,
        ranked: Vec<ConversationId>,
        limit: usize,
    ) -> Result<Vec<Found>, StorageError> {
        let (found, texts) = self
            .with(user, move |connection| {
                let transaction = connection.transaction()?;
                let (long, short): (Vec<&String>, Vec<&String>) = words
                    .iter()
                    .partition(|word| word.chars().count() >= TRIGRAM);
                let mut conditions = Vec::new();
                let mut values = Vec::new();
                let from = if long.is_empty() {
                    "texts t"
                } else {
                    conditions.push("texts_fts MATCH ?".to_owned());
                    // Each word a phrase, so FTS5 reads none of it as an
                    // operator; phrases side by side must all appear.
                    let query: Vec<String> = long
                        .iter()
                        .map(|word| format!("\"{}\"", word.replace('"', "\"\"")))
                        .collect();
                    values.push(query.join(" "));
                    "texts_fts JOIN texts t ON t.id = texts_fts.rowid"
                };
                for word in short {
                    conditions.push(r"t.text LIKE ? ESCAPE '\'".to_owned());
                    values.push(format!("%{}%", escape_like(word)));
                }
                // Each conversation by its place in `ranked`; the index holds
                // each under its record's spelling.
                let ranks: HashMap<ConversationId, usize> =
                    ranked.into_iter().enumerate().map(|(rank, id)| (id, rank)).collect();
                // A title, or a message or an answer of the root: the
                // product's search finds no subagent's text and no command.
                conditions.push("(t.block IS NULL OR (t.agent = 0 AND t.kind IN ('message', 'answer')))".to_owned());
                let mut statement = transaction.prepare(&format!(
                    "SELECT t.id, t.conversation, t.position FROM {from} WHERE {}",
                    conditions.join(" AND ")
                ))?;
                let mut rows = statement.query(params_from_iter(values))?;
                // Each conversation found, by its rank, with whether its
                // title matched and the row and index of its newest matching
                // message.
                let mut found: HashMap<usize, (ConversationId, bool, Option<(i64, i64)>)> = HashMap::new();
                while let Some(row) = rows.next()? {
                    let conversation = decode(
                        "texts",
                        "conversation",
                        ConversationId::try_from(row.get::<_, String>(1)?),
                    )?;
                    let Some(&rank) = ranks.get(&conversation) else {
                        continue;
                    };
                    let (_, title, newest) = found.entry(rank).or_insert((conversation, false, None));
                    let position: Option<i64> = row.get(2)?;
                    let Some(position) = position else {
                        *title = true;
                        continue;
                    };
                    if newest.is_none_or(|(_, newest)| newest < position) {
                        *newest = Some((row.get(0)?, position));
                    }
                }
                drop(rows);
                drop(statement);
                let mut found: Vec<_> = found.into_iter().collect();
                found.sort_by_key(|(rank, (_, title, _))| (!*title, *rank));
                found.truncate(limit);
                let mut message = transaction.prepare_cached("SELECT block, text FROM texts WHERE id = ?1")?;
                let mut answer = Vec::with_capacity(found.len());
                let mut texts = 0;
                for (_, (conversation, title, newest)) in found {
                    let message = match newest {
                        Some((row, _)) => {
                            let (block, text): (String, String) =
                                message.query_row([row], |row| Ok((row.get(0)?, row.get(1)?)))?;
                            texts += 1;
                            let (line, ranges) = match_line(&text, &words);
                            Some(FoundMessage {
                                block: decode("texts", "block", BlockId::try_from(block))?,
                                line,
                                ranges,
                            })
                        }
                        None => None,
                    };
                    answer.push(Found {
                        conversation,
                        title,
                        message,
                    });
                }
                Ok((answer, texts))
            })
            .await?;
        self.record(|work| work.texts += texts);
        Ok(found)
    }
}

/// The saved version at which the index holds the agent's transcript;
/// none when it holds nothing of the agent.
fn held_version(
    transaction: &rusqlite::Transaction<'_>,
    conversation: &ConversationId,
    agent: u64,
) -> Result<Option<SavedVersion>, StorageError> {
    let held: Option<(i64, i64)> = transaction
        .prepare_cached("SELECT revision, blocks FROM agents WHERE conversation = ?1 AND agent = ?2")?
        .query_row(params![conversation.as_str(), integer(agent)], |row| Ok((row.get(0)?, row.get(1)?)))
        .optional()?;
    held.map(|(revision, blocks)| version_of(revision, blocks)).transpose()
}

/// Records the saved version at which the index holds the agent.
fn set_version(
    transaction: &rusqlite::Transaction<'_>,
    conversation: &ConversationId,
    agent: u64,
    version: SavedVersion,
) -> Result<(), StorageError> {
    transaction
        .prepare_cached(
            "INSERT INTO agents (conversation, agent, revision, blocks) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (conversation, agent) DO UPDATE SET revision = excluded.revision, blocks = excluded.blocks",
        )?
        .execute(params![
            conversation.as_str(),
            integer(agent),
            integer(version.revision),
            integer(version.blocks)
        ])?;
    Ok(())
}

/// A saved version as the columns hold it.
fn version_of(revision: i64, blocks: i64) -> Result<SavedVersion, StorageError> {
    Ok(SavedVersion {
        revision: decode("agents", "revision", u64::try_from(revision))?,
        blocks: decode("agents", "blocks", u64::try_from(blocks))?,
    })
}

/// A count of rows as the work counts it.
fn count_of(rows: usize) -> u64 {
    u64::try_from(rows).unwrap_or(u64::MAX)
}

/// Whether the index holds the conversation.
fn holds(transaction: &rusqlite::Transaction<'_>, conversation: &ConversationId) -> Result<bool, StorageError> {
    let held = transaction
        .prepare_cached("SELECT EXISTS (SELECT 1 FROM conversations WHERE id = ?1)")?
        .query_row([conversation.as_str()], |row| row.get(0))?;
    Ok(held)
}

/// Adds the conversation's title row.
fn insert_title(
    transaction: &rusqlite::Transaction<'_>,
    conversation: &ConversationId,
    title: &str,
) -> Result<(), StorageError> {
    transaction
        .prepare_cached("INSERT INTO texts (conversation, text) VALUES (?1, ?2)")?
        .execute(params![conversation.as_str(), title])?;
    Ok(())
}

/// Adds the row of an agent's block: its text at the block and its index.
fn insert_text(
    transaction: &rusqlite::Transaction<'_>,
    conversation: &ConversationId,
    agent: u64,
    message: &IndexedMessage,
) -> Result<(), StorageError> {
    transaction
        .prepare_cached(
            "INSERT INTO texts (conversation, agent, kind, block, position, time, text) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?
        .execute(params![
            conversation.as_str(),
            integer(agent),
            message.text.kind.as_str(),
            message.block.as_str(),
            count(message.position),
            message.text.time.as_millisecond(),
            message.text.text
        ])?;
    Ok(())
}

/// Removes the conversation's texts and its record.
fn remove_rows(
    transaction: &rusqlite::Transaction<'_>,
    conversation: &ConversationId,
) -> Result<usize, StorageError> {
    let rows = transaction.execute(
        "DELETE FROM texts WHERE conversation = ?1",
        [conversation.as_str()],
    )?;
    transaction.execute("DELETE FROM agents WHERE conversation = ?1", [conversation.as_str()])?;
    transaction.execute(
        "DELETE FROM conversations WHERE id = ?1",
        [conversation.as_str()],
    )?;
    Ok(rows)
}

/// A word as a `LIKE` pattern matches it literally.
fn escape_like(word: &str) -> String {
    let mut escaped = String::with_capacity(word.len());
    for character in word.chars() {
        if matches!(character, '%' | '_' | '\\') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

/// A count as the INTEGER column holds it; no revision or transcript comes
/// near `i64::MAX`.
fn integer(value: u64) -> i64 {
    i64::try_from(value).expect("a count fits the column")
}

/// The index at `path`, with its schema: a new one receives it, and one of
/// another schema, or one that does not open, is deleted and made again,
/// which the answer's flag tells.
fn open_index(path: &Path) -> Result<(Connection, bool), StorageError> {
    match open_current(path) {
        Ok(Some(connection)) => return Ok((connection, false)),
        Ok(None) => {}
        Err(error) => tracing::warn!(
            path = %path.display(),
            error = &error as &dyn std::error::Error,
            "a search index did not open; it is built again"
        ),
    }
    // Another schema, or a file SQLite cannot read: the index is derived, so
    // it goes with its journal, and the indexer fills the new one.
    for suffix in ["", "-wal", "-shm"] {
        let mut file = path.as_os_str().to_owned();
        file.push(suffix);
        match std::fs::remove_file(&file) {
            Ok(()) => {}
            // A file that is not there needs no removing.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    let connection = open_current(path)?.ok_or_else(|| StorageError::OtherSchema {
        path: path.to_owned(),
    })?;
    Ok((connection, true))
}

/// The index at `path` when it has this build's schema, giving a new one
/// the schema; none when it has another.
fn open_current(path: &Path) -> Result<Option<Connection>, StorageError> {
    let mut connection = Connection::open(path)?;
    sqlite::configure(&connection)?;
    let version = schema::version_of(SCHEMA);
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let recorded: i32 = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if recorded != version {
        let tables: i64 = transaction.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type = 'table'",
            [],
            |row| row.get(0),
        )?;
        if recorded != 0 || tables != 0 {
            return Ok(None);
        }
        transaction.execute_batch(SCHEMA)?;
        transaction.pragma_update(None, "user_version", version)?;
    }
    transaction.commit()?;
    Ok(Some(connection))
}

/// The text a search finds of a block: a user's message's or steer's, an
/// answer's, or the title of the command a `shell` call ran. Thinking,
/// other tools' calls and every call's result, commands' scripts and output,
/// files and the blocks the runtime writes have none, and neither does a
/// message without text.
pub fn searchable_text(block: &Block) -> Option<Searchable> {
    let (kind, text) = match block {
        Block::User(_) | Block::Steer(_) => (SearchKind::Message, message_text(block)?),
        Block::Text(answer) if !answer.text.trim().is_empty() => (SearchKind::Answer, answer.text.clone()),
        Block::ToolCall(call) => (SearchKind::Command, call.shell_command()?.title),
        _ => return None,
    };
    Some(Searchable {
        kind,
        text,
        time: block.created_at(),
    })
}

/// How many characters of a long line come before its first match.
const LEAD: usize = 40;

/// The line of `text` around the first place a word matches, at most
/// `LINE_MAX` characters, with an ellipsis where it is cut, and the places
/// of every word in it as UTF-16 offsets, which the page marks.
///
/// FTS5's `snippet` and `highlight` would mark only the words of its MATCH,
/// which leaves out the words of one or two characters the index finds by
/// a scan, and they cut by tokens, which the trigram tokenizer makes of
/// every character; neither gives UTF-16 offsets. So the line is cut and
/// the words found here, on the text the index matched, case ignored as the
/// index ignores it.
fn match_line(text: &str, words: &[String]) -> (String, Vec<[u32; 2]>) {
    let characters: Vec<char> = text.chars().collect();
    let found = occurrences(&characters, words);
    let first = found.first().map_or(0, |&(start, _)| start);
    let mut line_start = characters[..first]
        .iter()
        .rposition(|&character| character == '\n')
        .map_or(0, |newline| newline + 1);
    let mut line_end = characters[first..]
        .iter()
        .position(|&character| character == '\n')
        .map_or(characters.len(), |newline| first + newline);
    while line_start < line_end && characters[line_start].is_whitespace() {
        line_start += 1;
    }
    while line_end > line_start && characters[line_end - 1].is_whitespace() {
        line_end -= 1;
    }
    let (mut start, mut end) = (line_start, line_end);
    if end - start > LINE_MAX {
        start = first.saturating_sub(LEAD).max(line_start);
        end = (start + LINE_MAX).min(line_end);
        start = end.saturating_sub(LINE_MAX).max(line_start);
    }
    // The ellipses take the place of a character each, inside the bound.
    let cut_start = start > line_start;
    let cut_end = end < line_end;
    if cut_start {
        start += 1;
    }
    if cut_end && end > start {
        end -= 1;
    }
    let mut line = String::new();
    if cut_start {
        line.push('…');
    }
    line.extend(&characters[start..end]);
    if cut_end {
        line.push('…');
    }
    let ellipsis = if cut_start { '…'.len_utf16() } else { 0 };
    let ranges = utf16_ranges(&characters[start..end], ellipsis, &found, start);
    (line, ranges)
}

/// The places of every word in the whole title, as UTF-16 offsets, found
/// as a match line's are.
pub fn title_ranges(title: &str, words: &[String]) -> Vec<[u32; 2]> {
    let characters: Vec<char> = title.chars().collect();
    let found = occurrences(&characters, words);
    utf16_ranges(&characters, 0, &found, 0)
}

/// `found`, character positions in a text, as UTF-16 offsets in `shown`, the
/// piece of the text that starts at character `start` and follows `lead`
/// UTF-16 units of its own, such as an ellipsis; pieces outside it are cut.
fn utf16_ranges(shown: &[char], lead: usize, found: &[(usize, usize)], start: usize) -> Vec<[u32; 2]> {
    let end = start + shown.len();
    let offset = |position: usize| -> u32 {
        let before: usize = shown[..position - start]
            .iter()
            .map(|character| character.len_utf16())
            .sum();
        u32::try_from(lead + before).expect("a line of 256 characters fits u32")
    };
    found
        .iter()
        .filter(|&&(from, to)| from < end && to > start)
        .map(|&(from, to)| [offset(from.max(start)), offset(to.min(end))])
        .collect()
}

/// Where the words occur in `characters`, as character positions, end
/// exclusive, in order, overlapping ones joined.
pub fn occurrences(characters: &[char], words: &[String]) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    for word in words {
        let word: Vec<char> = word.chars().collect();
        if word.is_empty() || word.len() > characters.len() {
            continue;
        }
        for start in 0..=characters.len() - word.len() {
            let matches = word
                .iter()
                .zip(&characters[start..])
                .all(|(&wanted, &character)| same_letter(wanted, character));
            if matches {
                found.push((start, start + word.len()));
            }
        }
    }
    found.sort_unstable();
    let mut joined: Vec<(usize, usize)> = Vec::new();
    for (start, end) in found {
        match joined.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => joined.push((start, end)),
        }
    }
    joined
}

/// Whether two characters are the same letter, case ignored.
fn same_letter(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
}

/// The text of a message the user sent, without its files; none for any
/// other block, or a message without text.
fn message_text(block: &Block) -> Option<String> {
    let content = match block {
        Block::User(user) => &user.content,
        Block::Steer(steer) => &steer.content,
        _ => return None,
    };
    let texts: Vec<&str> = content
        .iter()
        .filter_map(|part| match part {
            UserContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    let text = texts.join("\n");
    (!text.trim().is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    //! The scenario of `GET /search` (the backend's `search` tests) covers
    //! matching, order, edits and a rebuild after another schema; these cover
    //! what it does not reach.

    use super::*;

    fn user() -> UserId {
        UserId::try_from("ana".to_owned()).unwrap()
    }

    fn conversation(number: u8) -> ConversationId {
        ConversationId::try_from(format!("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a{number:02x}")).unwrap()
    }

    fn indexed(title: &str, messages: &[(&str, &str)]) -> IndexedConversation {
        IndexedConversation {
            title: title.to_owned(),
            agents: vec![IndexedAgent {
                agent: 0,
                version: SavedVersion::default(),
                messages: messages
                    .iter()
                    .enumerate()
                    .map(|(position, (block, text))| IndexedMessage {
                        block: BlockId::try_from((*block).to_owned()).unwrap(),
                        position,
                        text: Searchable {
                            kind: SearchKind::Message,
                            text: (*text).to_owned(),
                            time: Timestamp::UNIX_EPOCH,
                        },
                    })
                    .collect(),
            }],
        }
    }

    /// The conversations a query finds, with whether the title matched and
    /// the block of the newest message that did, in conversation order.
    async fn found(indexes: &SearchIndexes, query: &str) -> Vec<(ConversationId, bool, Option<String>)> {
        let words = query.split_whitespace().map(str::to_owned).collect();
        let ranked = (1..=4).map(conversation).collect();
        let mut found: Vec<_> = indexes
            .find(&user(), words, ranked, usize::MAX)
            .await
            .unwrap()
            .into_iter()
            .map(|found| {
                (
                    found.conversation,
                    found.title,
                    found.message.map(|message| message.block.into_string()),
                )
            })
            .collect();
        found.sort();
        found
    }

    // About 40 ms: one index file and a handful of short transactions.
    #[tokio::test]
    async fn short_words_match_literally_beside_long_ones_and_a_removed_conversation_has_no_rows() {
        let data = tempfile::tempdir().unwrap();
        let indexes = SearchIndexes::open(data.path().join("search")).await.unwrap();
        let (first, second) = (conversation(1), conversation(2));
        indexes
            .index(&user(), &first, indexed("部署 notes", &[("b1", "100% sure"), ("b2", "部署 done")]))
            .await
            .unwrap();
        indexes
            .index(&user(), &second, indexed("Other notes", &[("c1", "a_b")]))
            .await
            .unwrap();

        // A word of two characters beside a longer one, both in one text.
        assert_eq!(found(&indexes, "部署 notes").await, vec![(first.clone(), true, None)]);
        assert_eq!(found(&indexes, "部署").await, vec![(first.clone(), true, Some("b2".into()))]);
        // LIKE's wildcards in a word are the characters themselves.
        assert_eq!(found(&indexes, "0%").await, vec![(first.clone(), false, Some("b1".into()))]);
        assert_eq!(found(&indexes, "_").await, vec![(second.clone(), false, Some("c1".into()))]);
        assert_eq!(found(&indexes, "%").await, vec![(first.clone(), false, Some("b1".into()))]);

        indexes.remove_conversation(&user(), &second).await.unwrap();
        assert_eq!(found(&indexes, "notes").await, vec![(first.clone(), true, None)]);
        assert_eq!(indexes.conversations(&user()).await.unwrap(), vec![first]);
    }

    // About 20 ms.
    #[tokio::test]
    async fn a_found_message_comes_with_its_own_text_while_its_conversation_is_indexed_again() {
        let data = tempfile::tempdir().unwrap();
        let indexes = SearchIndexes::open(data.path().join("search")).await.unwrap();
        indexes
            .index(&user(), &conversation(1), indexed("", &[("a", "needle in a")]))
            .await
            .unwrap();
        // The search asks first and the replacement waits for it; the new
        // rows take the numbers the removed ones had.
        let (ana, first) = (user(), conversation(1));
        let (found, replaced) = tokio::join!(
            indexes.find(&ana, vec!["needle".to_owned()], vec![first.clone()], 1),
            indexes.index(&ana, &first, indexed("", &[("b", "needle in b")])),
        );
        replaced.unwrap();
        let message = found.unwrap().remove(0).message.unwrap();
        assert_eq!((message.block.as_str(), message.line.as_str()), ("a", "needle in a"));
    }

    // About 100 ms: 60 conversations indexed, one query.
    #[tokio::test]
    async fn a_query_reads_the_texts_of_the_results_it_answers_with_alone() {
        let data = tempfile::tempdir().unwrap();
        let indexes = SearchIndexes::open(data.path().join("search")).await.unwrap();
        // Each conversation's newest message matches, in a long text; the
        // last one's title matches too.
        let long = format!("{} needle {}", "a".repeat(5_000), "b".repeat(5_000));
        let all: Vec<ConversationId> = (1..=60).map(conversation).collect();
        for (number, id) in all.iter().enumerate() {
            let title = if number == 59 { "needle" } else { "" };
            indexes.index(&user(), id, indexed(title, &[("m", &long)])).await.unwrap();
        }
        let before = indexes.work();
        let found = indexes
            .find(&user(), vec!["needle".to_owned()], all.clone(), 3)
            .await
            .unwrap();
        // The title match first, then the others in their given order.
        let answered: Vec<_> = found.iter().map(|found| found.conversation.clone()).collect();
        assert_eq!(answered, vec![all[59].clone(), all[0].clone(), all[1].clone()]);
        assert!(found.iter().all(|found| {
            found.message.as_ref().is_some_and(|message| message.line.chars().count() == LINE_MAX)
        }));
        assert_eq!(indexes.work().texts - before.texts, 3);
    }

    // About 20 ms. An agent's history search finds the agent's own rows
    // alone: a subagent's search whose index rows of the root came back
    // would name blocks of the root's transcript by indices of its own.
    #[tokio::test]
    async fn a_history_search_finds_its_own_agents_rows_alone() {
        let data = tempfile::tempdir().unwrap();
        let indexes = SearchIndexes::open(data.path().join("search")).await.unwrap();
        let mut conversation_rows = indexed("", &[("r1", "the release switch"), ("r2", "other")]);
        let mut child = indexed("", &[("c1", "find the release switch"), ("c2", "release notes")]).agents.remove(0);
        child.agent = 1;
        conversation_rows.agents.push(child);
        indexes.index(&user(), &conversation(1), conversation_rows).await.unwrap();
        let words = |text: &str| text.split_whitespace().map(str::to_owned).collect::<Vec<_>>();
        let (ana, first) = (user(), conversation(1));
        let find = |agent, query: &str, any| {
            let query = HistoryQuery {
                words: words(query),
                any,
                kinds: Vec::new(),
                after: None,
                before: None,
                oldest: false,
            };
            indexes.find_in(&ana, &first, agent, query)
        };
        assert_eq!(find(1, "release switch", false).await.unwrap(), vec![0]);
        assert_eq!(find(1, "switch notes", true).await.unwrap(), vec![1, 0]);
        assert_eq!(find(0, "release switch", false).await.unwrap(), vec![0]);
        assert_eq!(find(0, "notes", false).await.unwrap(), Vec::<usize>::new());
    }

    // About 30 ms.
    #[tokio::test]
    async fn an_index_that_does_not_open_is_built_again() {
        let data = tempfile::tempdir().unwrap();
        let directory = data.path().join("search");
        let indexes = SearchIndexes::open(directory.clone()).await.unwrap();
        indexes
            .index(&user(), &conversation(1), indexed("Kept", &[]))
            .await
            .unwrap();
        // Each operation's connection closed with its WAL checkpointed, so
        // the file holds the index alone; garbage in it does not open.
        let path = directory.join(format!("{}.sqlite", user()));
        std::fs::write(&path, b"not a database at all, and long enough to be read as a header").unwrap();
        assert_eq!(indexes.conversations(&user()).await.unwrap(), vec![]);
        indexes
            .index(&user(), &conversation(2), indexed("Rebuilt", &[]))
            .await
            .unwrap();
        assert_eq!(found(&indexes, "rebuilt").await, vec![(conversation(2), true, None)]);
    }

    fn line(text: &str, query: &str) -> (String, Vec<[u32; 2]>) {
        let words: Vec<String> = query.split_whitespace().map(str::to_owned).collect();
        match_line(text, &words)
    }

    #[test]
    fn a_match_line_is_the_line_around_the_first_match_with_its_words_in_utf16() {
        // The line that holds the first match, case ignored, every word
        // marked, overlapping ones as one.
        assert_eq!(
            line("intro\n  Fix TS2307 by fixing ts paths  \nmore", "ts2307 ts"),
            ("Fix TS2307 by fixing ts paths".into(), vec![[4, 10], [21, 23]])
        );
        // Offsets count UTF-16 units: an emoji takes two.
        assert_eq!(
            line("🙂 模块路径不对", "路径"),
            ("🙂 模块路径不对".into(), vec![[5, 7]])
        );
        // A long line is cut around the match, an ellipsis at each cut,
        // 160 characters in all.
        let long = format!("{}needle{}", "a".repeat(300), "b".repeat(300));
        let (cut, ranges) = line(&long, "NEEDLE");
        assert_eq!(cut.chars().count(), LINE_MAX);
        assert!(cut.starts_with('…') && cut.ends_with('…'));
        assert_eq!(ranges, vec![[40, 46]]);
        assert_eq!(&cut[ranges[0][0] as usize + 2..ranges[0][1] as usize + 2], "needle");
        // Near the start, the line keeps its start and is cut at the end.
        let (cut, ranges) = line(&format!("needle {}", "c".repeat(400)), "needle");
        assert!(cut.starts_with("needle") && cut.ends_with('…'));
        assert_eq!(ranges, vec![[0, 6]]);
    }
}
