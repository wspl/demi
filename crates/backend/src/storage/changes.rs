//! The change store (`edit-tracking.md` § The change store): both sides of
//! every file segment a command edited, bound to the conversation in the
//! object store's `changes/` namespace:
//!
//! ```text
//! changes/<conversationId>/<commandId>/<file>/<edit>.original
//! changes/<conversationId>/<commandId>/<file>/<edit>.modified
//! ```
//!
//! A command's exit writes them from the Host's copies before its block is
//! saved; a copy that cannot be read or stored leaves its segment without
//! contents, never the list without the file. The browser reads a segment's
//! sides from here, without the Host, and a Fork copies the segments of its
//! blocks into its own namespace.

use std::sync::Arc;

use bytes::Bytes;
use demi_core::{Block, CommandId, EditedFile, ToolView};
use demi_host_remote::edited_file;
use demi_runner_protocol::wire::JobFileChange;
use demi_shell::HostError;
use demi_web_api::files::ChangeSides;
use demi_web_api::ids::ConversationId;
use object_store::path::Path;
use object_store::{ObjectStore, ObjectStoreExt as _, PutPayload};
use rusqlite::Connection;

use super::StorageError;
use super::columns::json;
use crate::runner::files::text_of;

/// Which side of a segment an object holds.
#[derive(Debug, Clone, Copy)]
enum Side {
    Original,
    Modified,
}

/// The conversations' retained edits in the object store.
#[derive(Clone)]
pub(crate) struct ChangeStore {
    objects: Arc<dyn ObjectStore>,
}

impl ChangeStore {
    pub(crate) fn new(objects: Arc<dyn ObjectStore>) -> Self {
        Self { objects }
    }

    /// Keeps both sides of each of the command's edit segments that the
    /// Host's copies hold as text, reading a copy with `read`, and answers
    /// the list the command's view shows: every file, with each segment's
    /// kept flag.
    pub(crate) async fn retain(
        &self,
        conversation: &ConversationId,
        command: &CommandId,
        read: impl AsyncFn(&str) -> Result<Bytes, HostError>,
        files: &[JobFileChange],
    ) -> Vec<EditedFile> {
        let mut retained = Vec::with_capacity(files.len());
        for (index, file) in files.iter().enumerate() {
            let mut kept = Vec::with_capacity(file.edits.len());
            for (segment, copies) in file.edits.iter().enumerate() {
                let Some(modified) = &copies.modified else {
                    kept.push(false);
                    continue;
                };
                let stored = self
                    .keep(conversation, command, index, segment, &read, copies.original.as_deref(), modified)
                    .await;
                if let Err(error) = &stored {
                    // The file's record stays in the list without this
                    // segment's contents.
                    tracing::warn!(%conversation, %command, path = %file.path, "an edit's contents were not kept: {error}");
                }
                kept.push(stored.is_ok());
            }
            retained.push(edited_file(file, |segment| kept[segment]));
        }
        retained
    }

    /// Stores one segment's two sides: the copy before it, empty for a
    /// segment that created the file, and the copy after it. Both must be
    /// text the browser can show.
    #[expect(clippy::too_many_arguments, reason = "a segment is named by all of them")]
    async fn keep(
        &self,
        conversation: &ConversationId,
        command: &CommandId,
        file: usize,
        segment: usize,
        read: &impl AsyncFn(&str) -> Result<Bytes, HostError>,
        original: Option<&str>,
        modified: &str,
    ) -> Result<(), String> {
        let before = match original {
            Some(path) => read(path).await.map_err(|error| error.to_string())?,
            None => Bytes::new(),
        };
        let after = read(modified).await.map_err(|error| error.to_string())?;
        text_of(before.clone()).map_err(|refusal| refusal.to_string())?;
        text_of(after.clone()).map_err(|refusal| refusal.to_string())?;
        for (side, bytes) in [(Side::Original, before), (Side::Modified, after)] {
            let path = key(conversation, command, file, segment, side).map_err(|error| error.to_string())?;
            self.objects
                .put(&path, PutPayload::from_bytes(bytes))
                .await
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    /// Copies the kept segments of `blocks`' shell calls from `source`'s
    /// namespace into `destination`'s, for a Fork of `source`. A source
    /// object that is gone stays unavailable in both conversations.
    pub(crate) async fn fork(
        &self,
        source: &ConversationId,
        destination: &ConversationId,
        blocks: &[Block],
    ) -> Result<(), StorageError> {
        let mut copied: Vec<&CommandId> = Vec::new();
        for block in blocks {
            let Block::ToolCall(call) = block else {
                continue;
            };
            let Some(ToolView::Shell(view)) = &call.view else {
                continue;
            };
            let Some(files) = &view.files else {
                continue;
            };
            if copied.contains(&&view.command_id) {
                continue;
            }
            copied.push(&view.command_id);
            for (index, file) in files.iter().enumerate() {
                for (segment, edit) in file.edits.iter().enumerate() {
                    if !edit.kept {
                        continue;
                    }
                    for side in [Side::Original, Side::Modified] {
                        let Some(bytes) = self.get(&key(source, &view.command_id, index, segment, side)?).await? else {
                            continue;
                        };
                        let path = key(destination, &view.command_id, index, segment, side)?;
                        self.objects.put(&path, PutPayload::from_bytes(bytes)).await?;
                    }
                }
            }
        }
        Ok(())
    }

    /// The two sides of segment `edit` of the file at `path` among `files`,
    /// the command's list; none when that segment was not kept or one of its
    /// objects is gone.
    pub(crate) async fn read(
        &self,
        conversation: &ConversationId,
        command: &CommandId,
        files: &[EditedFile],
        path: &str,
        edit: usize,
    ) -> Result<Option<ChangeSides>, StorageError> {
        let Some(index) = files.iter().position(|file| file.path == path) else {
            return Ok(None);
        };
        if !files[index].edits.get(edit).is_some_and(|segment| segment.kept) {
            return Ok(None);
        }
        let original = self.get(&key(conversation, command, index, edit, Side::Original)?).await?;
        let modified = self.get(&key(conversation, command, index, edit, Side::Modified)?).await?;
        let (Some(original), Some(modified)) = (original, modified) else {
            return Ok(None);
        };
        let text = |bytes: Bytes| {
            text_of(bytes).map_err(|refusal| StorageError::Corrupt {
                table: "changes",
                column: "contents",
                reason: refusal.to_string(),
            })
        };
        Ok(Some(ChangeSides {
            original: text(original)?,
            modified: text(modified)?,
        }))
    }

    /// The object at `path`; none when there is none.
    async fn get(&self, path: &Path) -> Result<Option<Bytes>, StorageError> {
        match self.objects.get(path).await {
            Ok(object) => Ok(Some(object.bytes().await?)),
            Err(object_store::Error::NotFound { .. }) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
}

/// The object of one side of a segment. The ids are the backend's own and
/// stay one path segment each.
fn key(conversation: &ConversationId, command: &CommandId, file: usize, segment: usize, side: Side) -> Result<Path, StorageError> {
    let identity = |value: &str| value.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'));
    if !identity(conversation.as_str()) || !identity(command.as_str()) {
        return Err(StorageError::Corrupt {
            table: "changes",
            column: "key",
            reason: format!("{conversation}/{command} is no edit history identity"),
        });
    }
    let side = match side {
        Side::Original => "original",
        Side::Modified => "modified",
    };
    Ok(Path::from(format!("changes/{conversation}/{command}/{file}/{segment}.{side}")))
}

/// The files the command `command` changed, as the conversation's saved
/// shell call of it lists them; none when no saved call ran it.
pub(crate) fn command_files(connection: &Connection, command: &CommandId) -> Result<Option<Vec<EditedFile>>, StorageError> {
    let mut statement = connection.prepare_cached(
        "SELECT block FROM blocks
         WHERE json_extract(block, '$.type') = 'tool_call' AND json_extract(block, '$.view.commandId') = ?1",
    )?;
    let mut rows = statement.query([command.as_str()])?;
    while let Some(row) = rows.next()? {
        let text: String = row.get(0)?;
        let block: Block = json("blocks", "block", &text)?;
        if let Block::ToolCall(call) = block
            && let Some(ToolView::Shell(view)) = call.view
            && let Some(files) = view.files
        {
            return Ok(Some(files));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use demi_command_service::protocol::{EditCopies, EditKind as JobEditKind};
    use object_store::memory::InMemory;

    use super::*;

    /// A finished shell call of `command` as its block saves it, listing
    /// `files` when it changed some.
    fn shell_call(command: &CommandId, files: Option<Vec<EditedFile>>) -> Block {
        Block::ToolCall(demi_core::ToolCallBlock {
            id: demi_core::BlockId::try_from(format!("block-{command}")).unwrap(),
            created_at: demi_core::Timestamp::UNIX_EPOCH,
            model: demi_agent::testing::test_model(),
            tool_use_id: "t".into(),
            tool_name: "shell_exec".into(),
            input: "{}".into(),
            status: demi_core::ToolCallStatus::Completed,
            output: Vec::new(),
            view: Some(ToolView::Shell(demi_core::ShellToolView {
                status: demi_core::ShellViewStatus::Exited,
                shell_id: demi_core::ShellId::try_from("shell-1").unwrap(),
                command_id: command.clone(),
                exit_code: Some(0),
                running_ms: 1,
                idle_ms: 0,
                chunks: Vec::new(),
                view_truncated: false,
                files_truncated: files.as_ref().map(|_| false),
                files,
            })),
        })
    }

    fn change(path: &str, kind: JobEditKind, edits: Vec<(Option<&str>, Option<&str>)>) -> JobFileChange {
        JobFileChange {
            path: path.into(),
            kind,
            edits: edits
                .into_iter()
                .map(|(original, modified)| EditCopies {
                    original: original.map(str::to_owned),
                    modified: modified.map(str::to_owned),
                })
                .collect(),
            added: 2,
            removed: 1,
        }
    }

    #[tokio::test(flavor = "local")]
    async fn a_retained_edit_reads_cold_survives_a_fork_and_a_missing_side_reads_as_none() {
        let objects: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        let store = ChangeStore::new(objects.clone());
        // The Host's copies; `missing` cannot be read.
        let copies = HashMap::from([
            ("before-A", b"one\n".as_slice()),
            ("after-A", b"one\ntwo\n".as_slice()),
            ("before-B", b"two\n".as_slice()),
            ("after-B", b"two\nthree\n".as_slice()),
            ("new", b"fresh\n".as_slice()),
            ("binary", b"\x00\x01".as_slice()),
        ]);
        let read = async |path: &str| {
            copies
                .get(path)
                .map(|bytes| Bytes::from_static(bytes))
                .ok_or_else(|| HostError::failed(Some("ENOENT".into()), format!("{path} is missing")))
        };
        let source = ConversationId::try_from("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a01").unwrap();
        let fork = ConversationId::try_from("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a02").unwrap();
        let command = CommandId::try_from("command-1").unwrap();
        let files = store
            .retain(
                &source,
                &command,
                read,
                &[
                    change(
                        "/work/file",
                        JobEditKind::Modified,
                        vec![(Some("before-A"), Some("after-A")), (Some("before-B"), Some("after-B"))],
                    ),
                    change("/work/new", JobEditKind::Added, vec![(None, Some("new"))]),
                    change(
                        "/work/binary",
                        JobEditKind::Modified,
                        vec![(Some("binary"), Some("binary")), (None, None), (Some("missing"), Some("new"))],
                    ),
                ],
            )
            .await;
        let kept: Vec<Vec<bool>> = files
            .iter()
            .map(|file| file.edits.iter().map(|edit| edit.kept).collect())
            .collect();
        assert_eq!(kept, [vec![true, true], vec![true], vec![false, false, false]]);

        let cold = ChangeStore::new(objects.clone());
        let sides = |original: &str, modified: &str| {
            Some(ChangeSides {
                original: original.into(),
                modified: modified.into(),
            })
        };
        assert_eq!(
            cold.read(&source, &command, &files, "/work/file", 1).await.unwrap(),
            sides("two\n", "two\nthree\n")
        );
        assert_eq!(cold.read(&source, &command, &files, "/work/new", 0).await.unwrap(), sides("", "fresh\n"));
        assert_eq!(cold.read(&source, &command, &files, "/work/binary", 0).await.unwrap(), None);
        assert_eq!(cold.read(&source, &command, &files, "/work/file", 9).await.unwrap(), None);
        assert_eq!(cold.read(&source, &command, &files, "/outside", 0).await.unwrap(), None);

        // A Fork owns its copies: the source's objects can go.
        let block = shell_call(&command, Some(files.clone()));
        store.fork(&source, &fork, &[block]).await.unwrap();
        for side in ["original", "modified"] {
            let path = Path::from(format!("changes/{source}/command-1/0/0.{side}"));
            objects.delete(&path).await.unwrap();
        }
        assert_eq!(
            cold.read(&fork, &command, &files, "/work/file", 0).await.unwrap(),
            sides("one\n", "one\ntwo\n")
        );
        assert_eq!(cold.read(&source, &command, &files, "/work/file", 0).await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_commands_files_are_the_ones_its_saved_shell_call_lists() {
        let data = tempfile::tempdir().unwrap();
        let stores = crate::storage::conversations::ConversationStores::open(
            data.path().to_owned(),
            std::num::NonZeroUsize::new(2).unwrap(),
        )
        .await
        .unwrap();
        let conversation = ConversationId::try_from("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a01").unwrap();
        let files = vec![EditedFile {
            path: "/work/notes.md".into(),
            kind: demi_core::EditKind::Added,
            added: 1,
            removed: 0,
            edits: vec![demi_core::KeptEdit { kept: true }],
        }];
        let first = CommandId::try_from("command-1").unwrap();
        let second = CommandId::try_from("command-2").unwrap();
        let rows = [
            serde_json::to_string(&shell_call(&first, Some(files.clone()))).unwrap(),
            serde_json::to_string(&shell_call(&second, None)).unwrap(),
        ];
        stores
            .db(&conversation)
            .call(move |connection| {
                connection.execute(
                    "INSERT INTO nodes (id, parent_id, description, profile, spawned_at, can_spawn, closed_phase,
                       closed_at, result, failure, delivered, state, block_count, command_revision, output_revision)
                     VALUES ('root', NULL, '', NULL, 0, 1, NULL, NULL, NULL, NULL, 0, '{}', 2, 0, 0)",
                    [],
                )?;
                for (index, row) in rows.iter().enumerate() {
                    connection.execute(
                        "INSERT INTO blocks (node_id, idx, block) VALUES ('root', ?1, ?2)",
                        rusqlite::params![index as i64, row],
                    )?;
                }
                Ok(())
            })
            .await
            .unwrap();
        let read = |command: &'static str| {
            let stores = stores.clone();
            let conversation = conversation.clone();
            async move {
                let command = CommandId::try_from(command).unwrap();
                stores
                    .read(&conversation, move |connection| command_files(connection, &command))
                    .await
                    .unwrap()
                    .flatten()
            }
        };
        assert_eq!(read("command-1").await, Some(files));
        assert_eq!(read("command-2").await, None);
        assert_eq!(read("command-3").await, None);
        stores.close().await;
    }
}
