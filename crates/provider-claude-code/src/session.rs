//! The session a new CLI process resumes (`claude-code.md` § The session a
//! process resumes): written from the request's replayed blocks, in their
//! order, into the process's configuration directory before it starts. A
//! block gives the entries the CLI wrote for it, as they were; what no entry
//! of a block stands for, such as another provider's turn or input Demi
//! wrote, is written in the CLI's format. The file opens with the CLI's
//! `environment` entry, written by the provider: the snapshot of the CLI's
//! machine, which the CLI compares with its own and so describes no other,
//! and no text. Each entry names the one before it, so the chain is whole
//! wherever blocks were cut out, and every entry carries the file's
//! session. A medium an entry holds is kept on its block as a reference to
//! the block's own medium (`demiBlob`), and its bytes return here.

use std::collections::HashMap;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use demi_provider_common::{
    InferenceItem, MediaBytes, Medium, RequestBlock, ResultPart, UserPart,
};
use demi_shared_types::{B64Bytes, BlobRef};
use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use crate::input::{MCP_SERVER, Message};
use crate::mcp;
use crate::placement::CliSystem;

/// The directory of the process's sessions in its configuration directory,
/// which `CLAUDE_CODE_PROJECT_DIR_NAME` names.
pub(crate) const PROJECT_DIR: &str = "demi";

/// The key of a reference to a block's medium in an entry kept on the
/// block, in place of the medium's base64.
const BLOB: &str = "demiBlob";

/// What the `environment` entry renders: the CLI refuses an empty text and
/// then renders the snapshot itself, and leaves a blank one out of its
/// request (Claude Code 2.1.294).
const NO_TEXT: &str = " ";

/// A session file, all but its first line built before the process's
/// machine is known.
pub(crate) struct SessionFile {
    /// The session the process resumes, which every entry carries.
    pub(crate) id: Uuid,
    environment: Uuid,
    timestamp: String,
    /// Every line after the `environment` entry.
    history: Vec<u8>,
}

impl SessionFile {
    /// The session of `session` holding `blocks` of `items` (each item a
    /// block of its own when the request names none), written at
    /// `timestamp`. It serializes the whole history, so it runs off the
    /// shard.
    pub(crate) fn new(
        session: &str,
        items: &[InferenceItem],
        blocks: &[RequestBlock],
        timestamp: String,
    ) -> Self {
        let id = session_id(session);
        let environment = Uuid::new_v4();
        let mut history = Vec::new();
        let mut parent = environment.to_string();
        for mut entry in entries(items, blocks, &id) {
            if let Value::Object(fields) = &mut entry {
                fields.insert("parentUuid".into(), Value::String(parent));
                fields.insert("sessionId".into(), Value::String(id.to_string()));
                fields
                    .entry("timestamp")
                    .or_insert_with(|| Value::String(timestamp.clone()));
                parent = fields
                    .get("uuid")
                    .and_then(Value::as_str)
                    .map_or_else(|| Uuid::new_v4().to_string(), str::to_owned);
            }
            // A JSON value always serializes.
            serde_json::to_writer(&mut history, &entry).expect("an entry serializes");
            history.push(b'\n');
        }
        Self {
            id,
            environment,
            timestamp,
            history,
        }
    }

    /// The file's path in the configuration directory.
    pub(crate) fn path(&self) -> String {
        format!("projects/{PROJECT_DIR}/{}.jsonl", self.id)
    }

    /// The file's bytes for a process on a machine of `system`.
    pub(crate) fn bytes(&self, system: &CliSystem) -> Bytes {
        let environment = json!({
            "type": "attachment",
            "uuid": self.environment.to_string(),
            "parentUuid": null,
            "isSidechain": false,
            "sessionId": self.id.to_string(),
            "timestamp": self.timestamp,
            "attachment": { "type": "environment", "snapshot": snapshot(system) },
            "rendered": [{ "content": NO_TEXT }],
            "renderedRole": "system",
        });
        let mut bytes = serde_json::to_vec(&environment).expect("an entry serializes");
        bytes.push(b'\n');
        bytes.extend_from_slice(&self.history);
        Bytes::from(bytes)
    }
}

/// The CLI's session id for Demi's session `session`: the same id when it
/// is a UUID, which the CLI requires, else one derived from it.
pub(crate) fn session_id(session: &str) -> Uuid {
    Uuid::parse_str(session).unwrap_or_else(|_| derived(session.as_bytes()))
}

/// A UUID that `name` alone decides.
fn derived(name: &[u8]) -> Uuid {
    let digest = Sha256::digest(name);
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest[..16]);
    uuid::Builder::from_random_bytes(bytes).into_uuid()
}

/// The environment as the CLI describes its machine: the run directory,
/// neither a git repository nor a worktree, with no other directories, and
/// the platform, shell and kernel as the CLI names them.
fn snapshot(system: &CliSystem) -> Value {
    let platform = system.kernel.to_lowercase();
    let shell = match system.shell.as_deref() {
        None | Some("") => "unknown",
        Some(shell) if shell.contains("zsh") => "zsh",
        Some(shell) if shell.contains("bash") => "bash",
        Some(shell) => shell,
    };
    json!({
        "workingDirectory": system.working_directory,
        "isWorktree": false,
        "isGitRepo": false,
        "additionalWorkingDirectories": [],
        "platform": platform,
        "shell": shell,
        "osVersion": format!("{} {}", system.kernel, system.release),
    })
}

/// The session's entries, unchained: each block's, with the calls of a run
/// of tool-call blocks before their results, as the CLI writes a batch;
/// then each entry of another session, as a fork's blocks hold, under an
/// id of this one's.
fn entries(items: &[InferenceItem], blocks: &[RequestBlock], session: &Uuid) -> Vec<Value> {
    let alone: Vec<RequestBlock>;
    let blocks = if blocks.is_empty() {
        alone = (0..items.len())
            .map(|index| RequestBlock {
                items: index..index + 1,
                entries: Vec::new(),
            })
            .collect();
        alone.as_slice()
    } else {
        blocks
    };
    let mut written = Vec::new();
    let mut index = 0;
    while index < blocks.len() {
        let calls = blocks[index..]
            .iter()
            .take_while(|block| is_call(items, block))
            .count();
        if calls == 0 {
            let (entries, results) = block_entries(items, &blocks[index], &written);
            written.extend(entries);
            written.extend(results);
            index += 1;
            continue;
        }
        let mut results = Vec::new();
        for block in &blocks[index..index + calls] {
            let (calls, block_results) = block_entries(items, block, &written);
            written.extend(calls);
            results.extend(block_results);
        }
        written.extend(results);
        index += calls;
    }
    renamed(written, session)
}

/// Whether `block` is a tool call's: its first item is the call.
fn is_call(items: &[InferenceItem], block: &RequestBlock) -> bool {
    matches!(
        items.get(block.items.start),
        Some(InferenceItem::ToolUse { .. })
    )
}

/// The id of the assistant message an assistant entry belongs to that
/// follows `written`, then `pending`: the last entry's, when it is an
/// assistant's, else one of its own.
fn message_id(written: &[Value], pending: &[Value]) -> String {
    let last = pending.last().or(written.last());
    match last.filter(|entry| entry["type"] == "assistant") {
        Some(entry) => entry["message"]["id"].as_str().map(str::to_owned),
        None => None,
    }
    .unwrap_or_else(|| format!("msg_demi_{}", written.len() + pending.len()))
}

/// What `block` gives the session: its entries with their media restored,
/// then an entry in the CLI's format for each of its items none of them
/// stands for. A tool call's results come apart from the rest, for the
/// batch's order. Entries whose medium the request no longer holds are
/// dropped, and the block's items are written instead; reasoning no entry
/// stands for is left out, since only its own vendor checks it, and so is
/// reasoning kept past a summary, since that vendor checks it against the
/// history the summary replaced (`providers.md` § Per vendor).
fn block_entries(
    items: &[InferenceItem],
    block: &RequestBlock,
    written: &[Value],
) -> (Vec<Value>, Vec<Value>) {
    let block_items = &items[block.items.clone()];
    let mut kept = restored(&block.entries, &media(block_items)).unwrap_or_default();
    let past_summary = block_items.iter().any(|item| {
        matches!(
            item,
            InferenceItem::AssistantThinking {
                kept_past_summary: true,
                ..
            } | InferenceItem::AssistantRedactedThinking {
                kept_past_summary: true,
                ..
            }
        )
    });
    if past_summary {
        kept.retain(|entry| !is_reasoning(entry));
    }
    let split = kept.iter().position(is_result).unwrap_or(kept.len());
    let results = kept.split_off(split);
    let (mut calls, mut results) = (kept, results);
    for item in block_items {
        if covered(item, calls.iter().chain(&results)) {
            continue;
        }
        if matches!(item, InferenceItem::ToolResult { .. }) {
            if let Some(entry) = written_as(item, written, &calls) {
                results.push(entry);
            }
        } else if let Some(entry) = written_as(item, written, &calls) {
            calls.push(entry);
        }
    }
    (calls, results)
}

/// Whether an entry is the model's reasoning.
fn is_reasoning(entry: &Value) -> bool {
    entry["type"] == "assistant"
        && matches!(
            entry["message"]["content"][0]["type"].as_str(),
            Some("thinking" | "redacted_thinking")
        )
}

/// Whether an entry is the result of a tool call.
fn is_result(entry: &Value) -> bool {
    entry["type"] == "user" && result_ids(entry).next().is_some()
}

/// The tool-use ids of the results an entry holds.
fn result_ids(entry: &Value) -> impl Iterator<Item = &str> {
    entry["message"]["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|block| block["type"] == "tool_result")
        .filter_map(|block| block["tool_use_id"].as_str())
}

/// The kind of the first block of an assistant entry's message.
fn assistant_kind(entry: &Value) -> Option<&str> {
    (entry["type"] == "assistant")
        .then(|| entry["message"]["content"][0]["type"].as_str())
        .flatten()
}

/// Whether one of `entries` stands for `item`.
fn covered<'a>(item: &InferenceItem, mut entries: impl Iterator<Item = &'a Value>) -> bool {
    match item {
        // A message, or one the CLI folded into a running turn.
        InferenceItem::UserMessage { .. } | InferenceItem::UserSteer { .. } => entries.any(|entry| {
            (entry["type"] == "user" && entry["isMeta"] != true && !is_result(entry))
                || entry["attachment"]["type"] == "queued_command"
        }),
        InferenceItem::AssistantText { .. } => {
            entries.any(|entry| assistant_kind(entry) == Some("text"))
        }
        InferenceItem::AssistantThinking { .. } => {
            entries.any(|entry| assistant_kind(entry) == Some("thinking"))
        }
        InferenceItem::AssistantRedactedThinking { .. } => {
            entries.any(|entry| assistant_kind(entry) == Some("redacted_thinking"))
        }
        InferenceItem::ToolUse { tool_use_id, .. } => entries.any(|entry| {
            assistant_kind(entry) == Some("tool_use")
                && entry["message"]["content"][0]["id"] == tool_use_id.as_str()
        }),
        InferenceItem::ToolResult { tool_use_id, .. } => {
            entries.any(|entry| result_ids(entry).any(|id| id == tool_use_id))
        }
    }
}

/// `item` as an entry in the CLI's format, as the CLI would have written it
/// had the item come from it: input as a `user` entry with the message Demi
/// writes to a process, text and tool calls as `assistant` entries of one
/// message, with the tool's name as the CLI gives it, and a result as a
/// `user` entry. Reasoning and empty text give none. `written` are the
/// session's entries before the block, and `pending` the block's before it.
fn written_as(item: &InferenceItem, written: &[Value], pending: &[Value]) -> Option<Value> {
    let assistant = |model: &str, content: Value| {
        json!({
            "type": "assistant",
            "uuid": Uuid::new_v4().to_string(),
            "isSidechain": false,
            "message": {
                "id": message_id(written, pending),
                "type": "message",
                "role": "assistant",
                "model": model,
                "content": [content],
            },
        })
    };
    let user = |message: Value| {
        json!({
            "type": "user",
            "uuid": Uuid::new_v4().to_string(),
            "isSidechain": false,
            "message": message,
        })
    };
    match item {
        InferenceItem::UserMessage { content } | InferenceItem::UserSteer { content } => {
            let message = serde_json::to_value(Message::user(content))
                .expect("a message serializes");
            Some(user(message))
        }
        InferenceItem::AssistantText { model_id, text } if !text.is_empty() => Some(assistant(
            model_id,
            json!({ "type": "text", "text": text }),
        )),
        InferenceItem::ToolUse {
            model_id,
            tool_use_id,
            tool_name,
            input,
        } => {
            let input = match input {
                Value::Null => json!({}),
                input => input.clone(),
            };
            Some(assistant(
                model_id,
                json!({
                    "type": "tool_use",
                    "id": tool_use_id,
                    "name": format!("mcp__{MCP_SERVER}__{tool_name}"),
                    "input": input,
                }),
            ))
        }
        InferenceItem::ToolResult {
            tool_use_id,
            output,
            is_error,
        } => {
            let mut result = json!({
                "type": "tool_result",
                "tool_use_id": tool_use_id,
                "content": result_content(output),
            });
            if *is_error {
                result["is_error"] = Value::Bool(true);
            }
            Some(user(json!({ "role": "user", "content": [result] })))
        }
        InferenceItem::AssistantText { .. }
        | InferenceItem::AssistantThinking { .. }
        | InferenceItem::AssistantRedactedThinking { .. } => None,
    }
}

/// A tool's result as the CLI gives the vendor one Demi sent it over MCP:
/// text, images with their base64, and a video or a document named in text.
fn result_content(output: &[ResultPart]) -> Vec<Value> {
    output
        .iter()
        .map(|part| match part {
            ResultPart::Text(text) => json!({ "type": "text", "text": text }),
            ResultPart::Image(bytes) => json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": bytes.media_type,
                    "data": STANDARD.encode(&bytes.data),
                },
            }),
            ResultPart::Video(bytes) | ResultPart::Document { bytes, .. } => {
                json!({ "type": "text", "text": mcp::unsent_text(bytes) })
            }
        })
        .collect()
}

/// `entries` with the ids of another session's renamed for `session`: a
/// fork's blocks hold its parent's entries, and the fork's file is a
/// session of its own (`claude-code.md` § The session a process resumes).
/// A field that names a renamed entry names its new id.
fn renamed(mut entries: Vec<Value>, session: &Uuid) -> Vec<Value> {
    let ours = session.to_string();
    let mut renames: HashMap<String, String> = HashMap::new();
    for entry in &entries {
        let foreign = entry["sessionId"]
            .as_str()
            .is_some_and(|other| other != ours);
        if let (true, Some(uuid)) = (foreign, entry["uuid"].as_str()) {
            let new = derived(format!("{ours}/{uuid}").as_bytes()).to_string();
            renames.insert(uuid.to_owned(), new);
        }
    }
    if renames.is_empty() {
        return entries;
    }
    for entry in &mut entries {
        let Value::Object(fields) = entry else {
            continue;
        };
        for value in fields.values_mut() {
            if let Some(new) = value.as_str().and_then(|old| renames.get(old)) {
                *value = Value::String(new.clone());
            }
        }
    }
    entries
}

/// The media of `items`, by the name of their bytes' blob.
pub(crate) fn media<'a>(items: &'a [InferenceItem]) -> HashMap<String, &'a B64Bytes> {
    let mut media = HashMap::new();
    let mut add = |bytes: &'a MediaBytes| {
        media.insert(BlobRef::of(&bytes.data).as_str().to_owned(), &bytes.data);
    };
    for item in items {
        match item {
            InferenceItem::UserMessage { content } | InferenceItem::UserSteer { content } => {
                for part in content {
                    match part {
                        UserPart::Image(Medium::Bytes(bytes))
                        | UserPart::Video(Medium::Bytes(bytes))
                        | UserPart::Document { bytes, .. } => add(bytes),
                        UserPart::Text(_) | UserPart::Image(_) | UserPart::Video(_) => {}
                    }
                }
            }
            InferenceItem::ToolResult { output, .. } => {
                for part in output {
                    match part {
                        ResultPart::Image(bytes) | ResultPart::Video(bytes) => add(bytes),
                        ResultPart::Text(_) | ResultPart::Document { .. } => {}
                    }
                }
            }
            _ => {}
        }
    }
    media
}

/// `entries` with each base64 text that holds one of `media` replaced by a
/// reference to it, as they are kept on their block.
pub(crate) fn referred(mut entries: Vec<Value>, media: &HashMap<String, &B64Bytes>) -> Vec<Value> {
    if media.is_empty() {
        return entries;
    }
    for entry in &mut entries {
        refer(entry, media);
    }
    entries
}

fn refer(value: &mut Value, media: &HashMap<String, &B64Bytes>) {
    match value {
        Value::String(text) => {
            // Only base64 can hold a medium's bytes; other text fails to
            // decode at its first character that is not.
            let Ok(bytes) = STANDARD.decode(text.as_bytes()) else {
                return;
            };
            let name = BlobRef::of(&bytes);
            if media.contains_key(name.as_str()) {
                *value = json!({ BLOB: name.as_str() });
            }
        }
        Value::Array(values) => values.iter_mut().for_each(|value| refer(value, media)),
        Value::Object(fields) => fields.values_mut().for_each(|value| refer(value, media)),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// `entries` with each reference to a medium replaced by its base64; none
/// when one of them names a medium `media` does not hold.
fn restored(entries: &[Value], media: &HashMap<String, &B64Bytes>) -> Option<Vec<Value>> {
    entries
        .iter()
        .map(|entry| {
            let mut entry = entry.clone();
            restore(&mut entry, media).then_some(entry)
        })
        .collect()
}

fn restore(value: &mut Value, media: &HashMap<String, &B64Bytes>) -> bool {
    match value {
        Value::Object(fields) if is_reference(fields) => {
            let Some(bytes) = fields[BLOB].as_str().and_then(|name| media.get(name)) else {
                return false;
            };
            *value = Value::String(STANDARD.encode(bytes.as_bytes()));
            true
        }
        Value::Object(fields) => fields.values_mut().all(|value| restore(value, media)),
        Value::Array(values) => values.iter_mut().all(|value| restore(value, media)),
        Value::String(_) | Value::Null | Value::Bool(_) | Value::Number(_) => true,
    }
}

fn is_reference(fields: &Map<String, Value>) -> bool {
    fields.len() == 1 && fields.get(BLOB).is_some_and(Value::is_string)
}
