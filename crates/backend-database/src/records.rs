//! A block as a record: how the store splits a block into its record and
//! the contents the record names, and joins them into the block again.
//!
//! A block is one JSON document, and written as one it held every form of
//! what the model read in full: the result text in the call's output, again
//! in the vendor's record of the session, a piece of it in the view, and the
//! request's changed files once more on every call. A record is the same
//! document with each content replaced by a marker that names the content by
//! the SHA-256 of its bytes, and the conversation keeps each content once
//! (`contents.rs`). Which values are contents:
//!
//! - a JSON value at one of [`JSON_VALUES`] whose text is [`INLINE`] bytes or
//!   more, as its text: the model selection, and the shell view's file
//!   lists, which repeat from call to call until a call changes a file;
//! - every other string of [`INLINE`] bytes or more: as the bytes it encodes
//!   when it is canonical base64, as a piece of a longer string of the block
//!   followed by at most [`TAIL`] bytes when it is one, or as its text.
//!
//! A marker is a string that begins with the control character U+0001; a
//! string of the block that begins with U+0001 has another put before it, so
//! no string reads as a marker:
//!
//! ```text
//! U+0001 c <sha256>                          a string
//! U+0001 b <sha256>                          base64, of the bytes kept
//! U+0001 j <sha256>                          a JSON value
//! U+0001 s <sha256> : <from> : <to> : <tail> a piece of a string, and a tail
//! ```
//!
//! The split loses nothing: joining a record gives back the block written.
//! The columns beside a record's body are what queries read, taken from the
//! document, and the body is compressed with the conversation's dictionary,
//! trained on its own records once it has [`DICTIONARY_AFTER`] of them.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use demi_shared_types::Block;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use serde_json::Value;

use super::StorageError;
use super::columns::{json, to_json};
use super::contents::{self, Hash, LEVEL};

/// The format of the blocks records hold (`blocks.body: transcript block
/// format 18` in the schema). A release reads each format it knows:
///
/// - format 17 is format 18 with entries that name no owner, all the Claude
///   Code CLI's, and redacted reasoning as a block of its own
///   ([`entries_owned`]);
/// - format 16 is format 17 without the times a block's parts started and
///   ended, which read as null ([`times_unkept`]).
pub(crate) const FORMAT: i64 = 18;

/// A value of at least this many bytes is a content. A shorter one costs
/// less inline than its marker and the lookup of its bytes.
pub(crate) const INLINE: usize = 256;

/// The values kept as their JSON, by their JSON pointer in the block.
const JSON_VALUES: [&str; 4] = ["/model", "/view/files", "/view/runFiles", "/view/requestFiles"];

/// The most bytes a piece of another string may add after it: a shell
/// view's output is its call's result without the header lines and with
/// its last newline.
const TAIL: usize = 2;

/// How many records a conversation holds before its dictionary is trained.
pub(crate) const DICTIONARY_AFTER: i64 = 400;

/// How large a trained dictionary is: the size zstd's trainer recommends.
const DICTIONARY_BYTES: usize = 112_640;

/// The control character a marker begins with.
const MARK: char = '\u{1}';

/// What queries read of a record, by column.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Columns {
    pub kind: String,
    pub id: String,
    pub turn_id: Option<String>,
    pub command: Option<String>,
    pub source: Option<String>,
    pub status: Option<String>,
    pub forkable: bool,
    pub placement: Option<String>,
    pub created_at: Option<String>,
    pub outside_turn: bool,
    pub resumed: bool,
    pub summary_tokens: Option<i64>,
}

impl Columns {
    fn of(block: &Value) -> Result<Self, StorageError> {
        let text = |value: &Value| value.as_str().map(str::to_owned);
        let required = |key: &str| text(&block[key]).ok_or_else(|| corrupt(format!("a block has no `{key}`")));
        Ok(Self {
            kind: required("type")?,
            id: required("id")?,
            turn_id: text(&block["turnId"]),
            command: text(&block["view"]["commandId"]),
            source: text(&block["source"]),
            status: text(&block["status"]),
            forkable: block["forkable"].as_bool().unwrap_or_default(),
            placement: text(&block["placement"]),
            created_at: text(&block["createdAt"]),
            outside_turn: block["outsideTurn"].as_bool().unwrap_or_default(),
            resumed: block["isResumed"].as_bool().unwrap_or_default(),
            summary_tokens: block["summaryTokens"].as_i64(),
        })
    }
}

/// A block split into its record's body, before compression, and the
/// contents the body names, each once.
#[derive(Debug)]
pub(crate) struct Split {
    pub columns: Columns,
    pub body: Vec<u8>,
    pub contents: Vec<(Hash, Vec<u8>)>,
}

/// The JSON document of `block`. Its entries are raw JSON text, which only
/// text carries across, so the document is parsed from the block's text.
pub(crate) fn document(block: &Block) -> Value {
    serde_json::from_str(&to_json(block)).expect("JSON just written parses")
}

/// Splits `block` into its record and contents.
pub(crate) fn split(block: &Block) -> Result<Split, StorageError> {
    split_document(document(block))
}

/// How a string of the block is kept.
enum Kept {
    Text(Hash),
    Base64(Hash),
    Piece { of: Hash, from: usize, to: usize, tail: String },
}

/// Splits the JSON document of a block into its record and contents.
pub(crate) fn split_document(mut value: Value) -> Result<Split, StorageError> {
    let columns = Columns::of(&value)?;
    let mut contents: Vec<(Hash, Vec<u8>)> = Vec::new();
    let mut add = |bytes: Vec<u8>| {
        let hash = Hash::of(&bytes);
        if !contents.iter().any(|(held, _)| *held == hash) {
            contents.push((hash, bytes));
        }
        hash
    };
    // The JSON values leave first, so the pass over the strings does not
    // reach into them, and come back as markers last, so it does not mark
    // their markers.
    let mut values = Vec::new();
    for pointer in JSON_VALUES {
        if let Some(found) = value.pointer_mut(pointer).filter(|found| found.is_object() || found.is_array()) {
            let text = found.to_string();
            if text.len() >= INLINE {
                *found = Value::Null;
                values.push((pointer, add(text.into_bytes())));
            }
        }
    }
    let mut long = Vec::new();
    collect_long(&value, &mut long);
    let kept = keep(&long, &mut add);
    let mut next = kept.iter();
    mark(&mut value, &mut next);
    for (pointer, hash) in values {
        *value.pointer_mut(pointer).expect("the value was taken from here") = Value::String(marker('j', &hash));
    }
    Ok(Split {
        columns,
        body: serde_json::to_vec(&value).expect("a JSON value serializes"),
        contents,
    })
}

/// The strings of at least [`INLINE`] bytes in `value`, in document order.
fn collect_long<'v>(value: &'v Value, long: &mut Vec<&'v str>) {
    match value {
        Value::String(text) if text.len() >= INLINE => long.push(text),
        Value::Array(items) => items.iter().for_each(|item| collect_long(item, long)),
        Value::Object(fields) => fields.values().for_each(|field| collect_long(field, long)),
        _ => {}
    }
}

/// How each of `long` is kept, in its order. A piece names a string kept as
/// text, so the longest strings are judged first.
fn keep(long: &[&str], add: &mut impl FnMut(Vec<u8>) -> Hash) -> Vec<Kept> {
    let mut order: Vec<usize> = (0..long.len()).collect();
    order.sort_by_key(|&at| std::cmp::Reverse(long[at].len()));
    let mut kept: Vec<Option<Kept>> = (0..long.len()).map(|_| None).collect();
    // The strings kept as text so far, the longest first, with their hashes.
    let mut texts: Vec<(&str, Hash)> = Vec::new();
    for at in order {
        let text = long[at];
        if let Some(bytes) = canonical_base64(text) {
            kept[at] = Some(Kept::Base64(add(bytes)));
            continue;
        }
        let piece = (0..=TAIL.min(text.len() - INLINE)).find_map(|cut| {
            let end = text.len() - cut;
            if !text.is_char_boundary(end) {
                return None;
            }
            let needle = &text[..end];
            texts
                .iter()
                .filter(|(whole, _)| whole.len() > needle.len())
                .find_map(|(whole, hash)| whole.find(needle).map(|from| (*hash, from, from + end, &text[end..])))
        });
        kept[at] = Some(match piece {
            Some((of, from, to, tail)) => Kept::Piece {
                of,
                from,
                to,
                tail: tail.to_owned(),
            },
            None => {
                let hash = add(text.as_bytes().to_vec());
                texts.push((text, hash));
                Kept::Text(hash)
            }
        });
    }
    kept.into_iter().map(|kept| kept.expect("every string was judged")).collect()
}

/// The bytes `text` encodes when it is base64 in the canonical form, the
/// one that encodes them again as `text`.
fn canonical_base64(text: &str) -> Option<Vec<u8>> {
    if text.len() % 4 != 0 {
        return None;
    }
    let bytes = STANDARD.decode(text).ok()?;
    (STANDARD.encode(&bytes) == text).then_some(bytes)
}

fn marker(kind: char, hash: &Hash) -> String {
    format!("{MARK}{kind}{}", hash.hex())
}

/// Replaces each long string of `value` by its marker from `kept`, in the
/// order [`collect_long`] found them, and marks a short string that would
/// read as a marker.
fn mark<'k>(value: &mut Value, kept: &mut impl Iterator<Item = &'k Kept>) {
    match value {
        Value::String(text) if text.len() >= INLINE => {
            *text = match kept.next().expect("a string for each judged") {
                Kept::Text(hash) => marker('c', hash),
                Kept::Base64(hash) => marker('b', hash),
                Kept::Piece { of, from, to, tail } => format!("{}:{from}:{to}:{tail}", marker('s', of)),
            };
        }
        Value::String(text) if text.starts_with(MARK) => text.insert(0, MARK),
        Value::Array(items) => items.iter_mut().for_each(|item| mark(item, kept)),
        Value::Object(fields) => fields.values_mut().for_each(|field| mark(field, kept)),
        _ => {}
    }
}

/// Compresses and decompresses records with the conversation's dictionary,
/// over one connection.
pub(crate) struct Codec<'c> {
    connection: &'c Connection,
    /// The dictionary new records take, with its compressor; loaded on
    /// first use.
    compressor: RefCell<Option<(Option<i64>, zstd::bulk::Compressor<'static>)>>,
    decompressors: RefCell<HashMap<Option<i64>, zstd::bulk::Decompressor<'static>>>,
}

/// A record as a query read it: its format, dictionary and body.
pub(crate) struct Stored {
    pub format: i64,
    pub dictionary: Option<i64>,
    pub body: Vec<u8>,
}

impl Stored {
    /// The record of a row that selected `format, dictionary, body` from its
    /// column `first` on.
    pub(crate) fn of(row: &Row<'_>, first: usize) -> rusqlite::Result<Self> {
        Ok(Self {
            format: row.get(first)?,
            dictionary: row.get(first + 1)?,
            body: row.get(first + 2)?,
        })
    }
}

/// The columns a record is read from, of the table `table`, for
/// [`Stored::of`].
pub(crate) fn record_of(table: &str) -> String {
    format!("{table}.format, {table}.dictionary, {table}.body")
}

impl<'c> Codec<'c> {
    pub(crate) fn new(connection: &'c Connection) -> Self {
        Self {
            connection,
            compressor: RefCell::new(None),
            decompressors: RefCell::new(HashMap::new()),
        }
    }

    /// `body` compressed with the dictionary new records take, and that
    /// dictionary.
    pub(crate) fn compress(&self, body: &[u8]) -> Result<(Option<i64>, Vec<u8>), StorageError> {
        let mut compressor = self.compressor.borrow_mut();
        if compressor.is_none() {
            let newest: Option<(i64, Vec<u8>)> = self
                .connection
                .query_row("SELECT id, bytes FROM dictionaries ORDER BY id DESC LIMIT 1", [], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })
                .optional()?;
            *compressor = Some(match newest {
                Some((id, bytes)) => (Some(id), zstd::bulk::Compressor::with_dictionary(LEVEL, &bytes)?),
                None => (None, zstd::bulk::Compressor::new(LEVEL)?),
            });
        }
        let (dictionary, compressor) = compressor.as_mut().expect("it was just loaded");
        Ok((*dictionary, compressor.compress(body)?))
    }

    /// The block `stored` holds, joined with its contents.
    pub(crate) fn block(&self, table: &'static str, stored: &Stored) -> Result<Block, StorageError> {
        let value = self.document(stored)?;
        json(table, "body", &value.to_string())
    }

    /// The JSON document `stored` holds, joined with its contents.
    pub(crate) fn document(&self, stored: &Stored) -> Result<Value, StorageError> {
        if !(16..=FORMAT).contains(&stored.format) {
            return Err(corrupt(format!("a record of block format {} is not one this release reads", stored.format)));
        }
        let plain = {
            let mut decompressors = self.decompressors.borrow_mut();
            if !decompressors.contains_key(&stored.dictionary) {
                let bytes: Vec<u8> = match stored.dictionary {
                    Some(id) => self
                        .connection
                        .prepare_cached("SELECT bytes FROM dictionaries WHERE id = ?1")?
                        .query_row([id], |row| row.get(0))
                        .optional()?
                        .ok_or_else(|| corrupt(format!("the dictionary {id} is missing")))?,
                    None => Vec::new(),
                };
                decompressors.insert(stored.dictionary, zstd::bulk::Decompressor::with_dictionary(&bytes)?);
            }
            let decompressor = decompressors.get_mut(&stored.dictionary).expect("it was just put");
            contents::decompress(&stored.body, Some(decompressor))?
        };
        let mut value: Value =
            serde_json::from_slice(&plain).map_err(|error| corrupt(format!("a record's body is not JSON: {error}")))?;
        self.join(&mut value)?;
        if stored.format == 16 {
            times_unkept(&mut value);
        }
        if stored.format <= 17 {
            entries_owned(&mut value);
        }
        Ok(value)
    }

    fn join(&self, value: &mut Value) -> Result<(), StorageError> {
        join_with(value, &mut |hash| contents::read(self.connection, hash))
    }
}

/// A block of format 16 as format 17 reads it: the times format 16 did not
/// keep are null.
fn times_unkept(block: &mut Value) {
    let unkept: &[&str] = match block.get("type").and_then(Value::as_str) {
        Some("thinking" | "text") => &["endedAt"],
        Some("tool_call") => &["writingStartedAt"],
        Some("response") => &["sentAt", "firstSentAt", "firstOutputAt"],
        Some("compaction_marker") => &["startedAt"],
        _ => &[],
    };
    if let Value::Object(fields) = block {
        for field in unkept {
            fields.insert((*field).to_owned(), Value::Null);
        }
    }
}

/// A block of format 17 as format 18 reads it.
///
/// - Each entry is owned by `claude-code:`, the only provider that wrote
///   entries.
/// - A `redacted_thinking` block is an `opaque` one with the `reasoning` view.
///   Its data becomes an entry of the Anthropic API provider's when it carries
///   that provider's tag; any other is the Claude Code CLI's, whose own lines
///   on the block keep it.
fn entries_owned(block: &mut Value) {
    let Value::Object(fields) = block else {
        return;
    };
    let mut entries: Vec<Value> = match fields.remove("entries") {
        Some(Value::Array(entries)) => entries
            .into_iter()
            .map(|entry| serde_json::json!({ "owner": "claude-code:", "value": entry }))
            .collect(),
        _ => Vec::new(),
    };
    if fields.get("type").and_then(Value::as_str) == Some("redacted_thinking") {
        let data = fields.remove("data");
        if let Some(data) = data.as_ref().and_then(Value::as_str).and_then(|data| data.strip_prefix("anthropic:")) {
            entries.push(serde_json::json!({
                "owner": "anthropic:",
                "value": { "type": "redacted_thinking", "data": data },
            }));
        }
        fields.insert("type".to_owned(), Value::String("opaque".to_owned()));
        fields.insert("callId".to_owned(), Value::Null);
        fields.insert("status".to_owned(), Value::String("completed".to_owned()));
        fields.insert("view".to_owned(), serde_json::json!({ "kind": "reasoning" }));
        fields.insert("endedAt".to_owned(), Value::Null);
    }
    if !entries.is_empty() {
        fields.insert("entries".to_owned(), Value::Array(entries));
    }
}

/// `value` with each marker replaced by what it stands for, whose content
/// `read` gives by its hash.
pub(crate) fn join_with(
    value: &mut Value,
    read: &mut impl FnMut(&Hash) -> Result<Vec<u8>, StorageError>,
) -> Result<(), StorageError> {
    match value {
        Value::String(text) if text.starts_with(MARK) => *value = marked(text, read)?,
        Value::Array(items) => items.iter_mut().try_for_each(|item| join_with(item, read))?,
        Value::Object(fields) => fields.values_mut().try_for_each(|field| join_with(field, read))?,
        _ => {}
    }
    Ok(())
}

/// The value a marker stands for, or the string a marked one is.
fn marked(
    marker: &str,
    read: &mut impl FnMut(&Hash) -> Result<Vec<u8>, StorageError>,
) -> Result<Value, StorageError> {
    let rest = &marker[MARK.len_utf8()..];
    let mut kind = rest.chars();
    let Some(kind) = kind.next() else {
        return Err(corrupt("a marker of no kind".into()));
    };
    if kind == MARK {
        return Ok(Value::String(rest.to_owned()));
    }
    let rest = &rest[kind.len_utf8()..];
    let hash = rest
        .get(..64)
        .and_then(Hash::from_hex)
        .ok_or_else(|| corrupt("a marker names no hash".into()))?;
    let bytes = read(&hash)?;
    let text = |bytes: Vec<u8>| String::from_utf8(bytes).map_err(|_| corrupt(format!("the content {} is not text", hash.hex())));
    match kind {
        'c' => Ok(Value::String(text(bytes)?)),
        'b' => Ok(Value::String(STANDARD.encode(bytes))),
        'j' => serde_json::from_slice(&bytes).map_err(|error| corrupt(format!("the content {} is not JSON: {error}", hash.hex()))),
        's' => {
            let whole = text(bytes)?;
            let mut parts = rest.get(65..).unwrap_or_default().splitn(3, ':');
            let range = parts
                .next()
                .and_then(|from| from.parse::<usize>().ok())
                .zip(parts.next().and_then(|to| to.parse::<usize>().ok()));
            let piece = range
                .and_then(|(from, to)| whole.get(from..to))
                .ok_or_else(|| corrupt("a piece lies outside its content".into()))?;
            Ok(Value::String(format!("{piece}{}", parts.next().unwrap_or_default())))
        }
        _ => Err(corrupt(format!("a marker of the kind {kind:?}, which this release does not know"))),
    }
}

/// Publishes `split`'s contents and answers their ids, each once.
fn publish(transaction: &Connection, split: &Split) -> Result<BTreeSet<i64>, StorageError> {
    split
        .contents
        .iter()
        .map(|(hash, bytes)| contents::put(transaction, hash, bytes))
        .collect()
}

fn name_contents(transaction: &Connection, table: &str, owner: &str, record: i64, named: &BTreeSet<i64>) -> Result<(), StorageError> {
    let mut insert = transaction.prepare_cached(&format!("INSERT INTO {table} ({owner}, content) VALUES (?1, ?2)"))?;
    for content in named {
        insert.execute(params![record, content])?;
    }
    Ok(())
}

/// Writes the row `idx` of `node` for `block`, which follows the row
/// `follows` and replaces `replaces`, with the contents it names.
pub(crate) fn insert(
    transaction: &Transaction<'_>,
    codec: &Codec<'_>,
    node: &str,
    idx: i64,
    follows: Option<i64>,
    replaces: Option<i64>,
    block: &Block,
) -> Result<(), StorageError> {
    let split = split(block)?;
    insert_split(transaction, codec, node, idx, follows, replaces, &split)
}

/// [`insert`] of a block already split.
pub(crate) fn insert_split(
    transaction: &Transaction<'_>,
    codec: &Codec<'_>,
    node: &str,
    idx: i64,
    follows: Option<i64>,
    replaces: Option<i64>,
    split: &Split,
) -> Result<(), StorageError> {
    let named = publish(transaction, split)?;
    let (dictionary, body) = codec.compress(&split.body)?;
    let c = &split.columns;
    let record: i64 = transaction
        .prepare_cached(
            "INSERT INTO blocks (node_id, idx, follows, replaces, format, type, id, turn_id, command, source,
               status, forkable, placement, created_at, outside_turn, resumed, summary_tokens, dictionary, body)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)
             RETURNING record",
        )?
        .query_row(
            params![
                node,
                idx,
                follows,
                replaces,
                FORMAT,
                c.kind,
                c.id,
                c.turn_id,
                c.command,
                c.source,
                c.status,
                c.forkable,
                c.placement,
                c.created_at,
                c.outside_turn,
                c.resumed,
                c.summary_tokens,
                dictionary,
                body
            ],
            |row| row.get(0),
        )?;
    name_contents(transaction, "block_contents", "record", record, &named)
}

/// Replaces the block of the row `idx` of `node` by `block`; the contents
/// only the block it replaces named go with it.
pub(crate) fn update(transaction: &Transaction<'_>, codec: &Codec<'_>, node: &str, idx: i64, block: &Block) -> Result<(), StorageError> {
    let split = split(block)?;
    let named = publish(transaction, &split)?;
    let record: i64 = transaction
        .prepare_cached("SELECT record FROM blocks WHERE node_id = ?1 AND idx = ?2")?
        .query_row(params![node, idx], |row| row.get(0))?;
    let before = named_by(transaction, "block_contents", "record", record)?;
    let (dictionary, body) = codec.compress(&split.body)?;
    let c = &split.columns;
    transaction
        .prepare_cached(
            "UPDATE blocks SET format = ?2, type = ?3, id = ?4, turn_id = ?5, command = ?6, source = ?7,
               status = ?8, forkable = ?9, placement = ?10, created_at = ?11, outside_turn = ?12, resumed = ?13,
               summary_tokens = ?14, dictionary = ?15, body = ?16
             WHERE record = ?1",
        )?
        .execute(params![
            record,
            FORMAT,
            c.kind,
            c.id,
            c.turn_id,
            c.command,
            c.source,
            c.status,
            c.forkable,
            c.placement,
            c.created_at,
            c.outside_turn,
            c.resumed,
            c.summary_tokens,
            dictionary,
            body
        ])?;
    let mut remove = transaction.prepare_cached("DELETE FROM block_contents WHERE record = ?1 AND content = ?2")?;
    for content in before.difference(&named) {
        remove.execute(params![record, content])?;
    }
    let added: BTreeSet<i64> = named.difference(&before).copied().collect();
    name_contents(transaction, "block_contents", "record", record, &added)?;
    contents::remove_unnamed(transaction, before.difference(&named).copied())
}

/// The contents the record `record` of `table` names.
fn named_by(connection: &Connection, table: &str, owner: &str, record: i64) -> Result<BTreeSet<i64>, StorageError> {
    let mut statement = connection.prepare_cached(&format!("SELECT content FROM {table} WHERE {owner} = ?1"))?;
    let named = statement.query_map([record], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?;
    Ok(named)
}

/// Writes a share's copy of `block` at `idx` of the share `token`.
pub(crate) fn insert_copy(transaction: &Connection, codec: &Codec<'_>, token: &str, idx: i64, block: &Block) -> Result<(), StorageError> {
    let split = split(block)?;
    let named = publish(transaction, &split)?;
    let (dictionary, body) = codec.compress(&split.body)?;
    let copy: i64 = transaction
        .prepare_cached(
            "INSERT INTO share_blocks (token, idx, format, type, id, dictionary, body)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) RETURNING copy",
        )?
        .query_row(
            params![token, idx, FORMAT, split.columns.kind, split.columns.id, dictionary, body],
            |row| row.get(0),
        )?;
    name_contents(transaction, "share_contents", "copy", copy, &named)
}

/// Removes the copies of the share `token`, and the contents only they
/// named.
pub(crate) fn remove_copies(transaction: &Connection, token: &str) -> Result<(), StorageError> {
    let named: Vec<i64> = transaction
        .prepare_cached(
            "SELECT DISTINCT named.content FROM share_blocks AS copy JOIN share_contents AS named ON named.copy = copy.copy
             WHERE copy.token = ?1",
        )?
        .query_map([token], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    transaction.prepare_cached("DELETE FROM share_blocks WHERE token = ?1")?.execute([token])?;
    contents::remove_unnamed(transaction, named)
}

/// Trains the conversation's dictionary on its first records once it has
/// [`DICTIONARY_AFTER`] and none yet. The records written before keep
/// theirs: none.
pub(crate) fn train_when_due(transaction: &Transaction<'_>) -> Result<(), StorageError> {
    let has: bool = transaction.query_row("SELECT EXISTS (SELECT 1 FROM dictionaries)", [], |row| row.get(0))?;
    if has {
        return Ok(());
    }
    let records: Option<i64> = transaction.query_row("SELECT max(record) FROM blocks", [], |row| row.get(0))?;
    if records.unwrap_or_default() < DICTIONARY_AFTER {
        return Ok(());
    }
    let mut samples = Vec::new();
    {
        let mut rows = transaction.prepare("SELECT body FROM blocks WHERE dictionary IS NULL ORDER BY record LIMIT ?1")?;
        let mut rows = rows.query([DICTIONARY_AFTER])?;
        while let Some(row) = rows.next()? {
            let body: Vec<u8> = row.get(0)?;
            samples.push(contents::decompress(&body, None)?);
        }
    }
    let Some(dictionary) = train(&samples) else {
        return Ok(());
    };
    transaction.execute("INSERT INTO dictionaries (bytes) VALUES (?1)", [dictionary])?;
    Ok(())
}

/// A dictionary trained on `samples`; none when they are too few or too
/// alike for one, and the records go on without.
pub(crate) fn train(samples: &[Vec<u8>]) -> Option<Vec<u8>> {
    zstd::dict::from_samples(samples, DICTIONARY_BYTES).ok()
}

fn corrupt(reason: String) -> StorageError {
    StorageError::Corrupt {
        table: "blocks",
        column: "body",
        reason,
    }
}
