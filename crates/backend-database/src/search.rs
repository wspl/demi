//! Each user's search index (`storage.md` § Search index):
//! `search/<userId>.sqlite`, an FTS5 table over one row per searchable text,
//! a conversation's title or the text of one of its root's messages, with
//! the saved version of each conversation's transcript it was built from.
//!
//! The index is derived from the conversations, so it has no history and no
//! migration: an index of another schema, or one that fails to open, is
//! deleted and built again empty, and the backend's indexer fills it. Every
//! operation takes its user's turn and runs on a short-lived connection on
//! the blocking pool, so a rebuild never removes a file another operation
//! has open, and no index holds a thread while nobody searches.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use demi_shared_gates::KeyedSerialGate;
use demi_shared_types::BlockId;
use demi_web_api_protocol::ids::{ConversationId, UserId};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params, params_from_iter};

use super::columns::{count, decode};
use super::tree::SavedVersion;
use super::{StorageError, schema, sqlite};

/// The schema. The `texts` table holds the texts once; `texts_fts` indexes
/// them as its external content, kept in step by the triggers. FTS5's
/// `trigram` tokenizer matches any piece of three characters or more, case
/// ignored, in every language, without splitting words.
const SCHEMA: &str = r"
-- Each indexed conversation, with the saved version of its root's
-- transcript the rows were built from.
CREATE TABLE conversations (
  id       TEXT PRIMARY KEY COLLATE NOCASE,
  revision INTEGER NOT NULL CHECK (revision >= 0),
  blocks   INTEGER NOT NULL CHECK (blocks >= 0)
) STRICT;

-- One row per searchable text: a conversation's title, which has no block,
-- or the text of one of its root's messages, at the block's index.
CREATE TABLE texts (
  id           INTEGER PRIMARY KEY,
  conversation TEXT NOT NULL COLLATE NOCASE,
  block        TEXT,
  position     INTEGER CHECK (position >= 0),
  text         TEXT NOT NULL,
  CHECK ((block IS NULL) = (position IS NULL))
) STRICT;
CREATE INDEX texts_conversation ON texts (conversation);
CREATE UNIQUE INDEX texts_title ON texts (conversation) WHERE block IS NULL;

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
}

/// What the index holds of one conversation: the saved version of its
/// transcript and the title it was indexed with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Indexed {
    pub version: SavedVersion,
    pub title: String,
}

/// A conversation as the indexer gives it to the index.
#[derive(Debug, Clone)]
pub struct IndexedConversation {
    pub version: SavedVersion,
    pub title: String,
    /// The searchable text of each of the root's messages, oldest first.
    pub messages: Vec<IndexedMessage>,
}

/// The searchable text of one message: its block and the block's index.
#[derive(Debug, Clone)]
pub struct IndexedMessage {
    pub block: BlockId,
    pub position: usize,
    pub text: String,
}

/// A conversation a query found: whether its title holds every word, and
/// the newest of its messages that does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub conversation: ConversationId,
    pub title: bool,
    pub message: Option<FoundMessage>,
}

/// A message that holds every word, with the row its text is read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundMessage {
    pub block: BlockId,
    pub text: TextRow,
    position: i64,
}

/// A text's row in the index, from which `texts` reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextRow(i64);

impl SearchIndexes {
    /// The indexes in `directory`, which is created if it is missing.
    pub async fn open(directory: PathBuf) -> Result<Self, StorageError> {
        tokio::fs::create_dir_all(&directory).await?;
        Ok(Self(Arc::new(Indexes {
            directory,
            turns: KeyedSerialGate::new(),
        })))
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
        tokio::task::spawn_blocking(move || work(&mut open_index(&path)?)).await?
    }

    /// What the index holds of the conversation; none when it holds nothing.
    pub async fn indexed(
        &self,
        user: &UserId,
        conversation: &ConversationId,
    ) -> Result<Option<Indexed>, StorageError> {
        let conversation = conversation.clone();
        self.with(user, move |connection| {
            let row: Option<(i64, i64, String)> = connection
                .query_row(
                    "SELECT c.revision, c.blocks, t.text FROM conversations c
                     JOIN texts t ON t.conversation = c.id AND t.block IS NULL
                     WHERE c.id = ?1",
                    [conversation.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            row.map(|(revision, blocks, title)| {
                Ok(Indexed {
                    version: SavedVersion {
                        revision: decode("conversations", "revision", u64::try_from(revision))?,
                        blocks: decode("conversations", "blocks", u64::try_from(blocks))?,
                    },
                    title,
                })
            })
            .transpose()
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
        self.with(user, move |connection| {
            let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            remove_rows(&transaction, &conversation)?;
            transaction.execute(
                "INSERT INTO conversations (id, revision, blocks) VALUES (?1, ?2, ?3)",
                params![
                    conversation.as_str(),
                    integer(indexed.version.revision),
                    integer(indexed.version.blocks)
                ],
            )?;
            let mut insert = transaction.prepare_cached(
                "INSERT INTO texts (conversation, block, position, text) VALUES (?1, ?2, ?3, ?4)",
            )?;
            insert.execute(params![
                conversation.as_str(),
                None::<&str>,
                None::<i64>,
                indexed.title
            ])?;
            for message in &indexed.messages {
                insert.execute(params![
                    conversation.as_str(),
                    message.block.as_str(),
                    count(message.position),
                    message.text
                ])?;
            }
            drop(insert);
            transaction.commit()?;
            Ok(())
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

    /// The conversations with a text that holds every one of `words`, case
    /// ignored: whether the title does, and the newest message that does. A
    /// word of three characters or more is found through the trigram index,
    /// and a shorter one by a scan of the texts the longer ones found, or of
    /// all of them. SQLite's `LIKE` ignores the case of ASCII letters alone,
    /// so a word of one or two letters of another script matches in its own
    /// case; Chinese, which such words are most often, has none.
    pub async fn find(&self, user: &UserId, words: Vec<String>) -> Result<Vec<Found>, StorageError> {
        self.with(user, move |connection| {
            let (long, short): (Vec<&String>, Vec<&String>) = words
                .iter()
                .partition(|word| word.chars().count() >= TRIGRAM);
            let mut conditions = Vec::new();
            let mut values = Vec::new();
            let from = if long.is_empty() {
                "texts t"
            } else {
                conditions.push("texts_fts MATCH ?".to_owned());
                // Each word a phrase, so FTS5 reads none of it as an operator;
                // phrases side by side must all appear.
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
            let mut statement = connection.prepare(&format!(
                "SELECT t.id, t.conversation, t.block, t.position FROM {from} WHERE {}",
                conditions.join(" AND ")
            ))?;
            let mut rows = statement.query(params_from_iter(values))?;
            let mut found: HashMap<ConversationId, Found> = HashMap::new();
            while let Some(row) = rows.next()? {
                let conversation = decode(
                    "texts",
                    "conversation",
                    ConversationId::try_from(row.get::<_, String>(1)?),
                )?;
                let entry = found.entry(conversation.clone()).or_insert(Found {
                    conversation,
                    title: false,
                    message: None,
                });
                let block: Option<String> = row.get(2)?;
                let Some(block) = block else {
                    entry.title = true;
                    continue;
                };
                let position: i64 = row.get(3)?;
                if entry
                    .message
                    .as_ref()
                    .is_none_or(|newest| newest.position < position)
                {
                    entry.message = Some(FoundMessage {
                        block: decode("texts", "block", BlockId::try_from(block))?,
                        text: TextRow(row.get(0)?),
                        position,
                    });
                }
            }
            Ok(found.into_values().collect())
        })
        .await
    }

    /// The texts of `rows`; a row gone since it was found is left out.
    pub async fn texts(
        &self,
        user: &UserId,
        rows: Vec<TextRow>,
    ) -> Result<HashMap<TextRow, String>, StorageError> {
        self.with(user, move |connection| {
            let mut statement = connection.prepare_cached("SELECT text FROM texts WHERE id = ?1")?;
            let mut texts = HashMap::new();
            for row in rows {
                let text: Option<String> = statement
                    .query_row([row.0], |found| found.get(0))
                    .optional()?;
                if let Some(text) = text {
                    texts.insert(row, text);
                }
            }
            Ok(texts)
        })
        .await
    }
}

/// Removes the conversation's texts and its record.
fn remove_rows(
    transaction: &rusqlite::Transaction<'_>,
    conversation: &ConversationId,
) -> Result<(), StorageError> {
    transaction.execute(
        "DELETE FROM texts WHERE conversation = ?1",
        [conversation.as_str()],
    )?;
    transaction.execute(
        "DELETE FROM conversations WHERE id = ?1",
        [conversation.as_str()],
    )?;
    Ok(())
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
/// another schema, or one that does not open, is deleted and made again.
fn open_index(path: &Path) -> Result<Connection, StorageError> {
    match open_current(path) {
        Ok(Some(connection)) => return Ok(connection),
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
    open_current(path)?.ok_or_else(|| StorageError::OtherSchema {
        path: path.to_owned(),
    })
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
            version: SavedVersion::default(),
            title: title.to_owned(),
            messages: messages
                .iter()
                .enumerate()
                .map(|(position, (block, text))| IndexedMessage {
                    block: BlockId::try_from((*block).to_owned()).unwrap(),
                    position,
                    text: (*text).to_owned(),
                })
                .collect(),
        }
    }

    /// The conversations a query finds, with whether the title matched and
    /// the block of the newest message that did, in conversation order.
    async fn found(indexes: &SearchIndexes, query: &str) -> Vec<(ConversationId, bool, Option<String>)> {
        let words = query.split_whitespace().map(str::to_owned).collect();
        let mut found: Vec<_> = indexes
            .find(&user(), words)
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
}
