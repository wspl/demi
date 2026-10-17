//! The lists of changed files a session stores with each shell call
//! (`edit-tracking.md` § What the conversation shows): computed as each
//! call's result is recorded, from the calls before it in the transcript
//! whatever order they returned in, and the same as a stored transcript
//! listed again in order, as the conversation database's migration does.

use std::collections::HashMap;

use demi_agent_store::{changes::relist, media::BlobStore, testing::MemoryBlobs};
use demi_shared_types::{EditCopies, EditKind, EditSegment, EditedFile, PathChange};

use super::*;

/// A shell tool whose input says what its command did: the files it wrote,
/// each `[path, kind, before, after]`, and the paths it removed. The call
/// whose input says `slow` returns once `release` fires.
fn shell_tool(release: oneshot::Receiver<()>) -> (String, Invoke) {
    let release = Rc::new(RefCell::new(Some(release)));
    tool("shell", move |call| {
        let input = &call.input;
        let files: Vec<EditedFile> = input["writes"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|write| {
                let text = |at: usize| write[at].as_str().unwrap();
                let kind = if text(1) == "added" { EditKind::Added } else { EditKind::Modified };
                EditedFile {
                    path: text(0).into(),
                    kind,
                    from: None,
                    added: 1,
                    removed: 0,
                    outside: false,
                    edits: vec![EditSegment {
                        copies: Some(EditCopies {
                            original: BlobRef::of(text(2).as_bytes()),
                            modified: BlobRef::of(text(3).as_bytes()),
                        }),
                        binary: false,
                    }],
                }
            })
            .collect();
        let path_changes = input["removes"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|path| PathChange::Removed {
                path: path.as_str().unwrap().into(),
            })
            .collect();
        let view = ToolView::Shell(ShellToolView {
            status: ShellViewStatus::Exited,
            command_id: CommandId::try_from(input["command"].as_str().unwrap()).unwrap(),
            exit_code: Some(0),
            lost_reason: None,
            running_ms: 1,
            idle_ms: 0,
            chunks: Vec::new(),
            view_truncated: false,
            files_truncated: (!files.is_empty()).then_some(false),
            files: (!files.is_empty()).then_some(files),
            path_changes,
            run_files: None,
            request_files: None,
        });
        let waits = if input["slow"] == true { release.borrow_mut().take() } else { None };
        Box::pin(async move {
            if let Some(waits) = waits {
                let _ = waits.await;
            }
            Ok(ToolOutcome {
                view: Some(view),
                ..output("exit code: 0")
            })
        })
    })
}

fn listed(block: &Block, run: bool) -> Vec<(String, EditKind)> {
    let Block::ToolCall(call) = block else {
        panic!("not a call: {block:?}");
    };
    let Some(ToolView::Shell(view)) = &call.view else {
        panic!("not a shell call: {block:?}");
    };
    let files = if run { &view.run_files } else { &view.request_files };
    files
        .iter()
        .flatten()
        .map(|file| (file.path.clone(), file.kind))
        .collect()
}

/// `blocks` as a release before the lists stored them: no run, and of each
/// shell call's files only those its job wrote.
fn as_stored_before(blocks: &[Block]) -> Vec<Block> {
    let mut stored = blocks.to_vec();
    for block in &mut stored {
        block.set_run(None);
        if let Block::ToolCall(call) = block
            && let Some(ToolView::Shell(view)) = &mut call.view
        {
            let written: Vec<EditedFile> = view
                .files
                .take()
                .into_iter()
                .flatten()
                .filter(|file| matches!(file.kind, EditKind::Added | EditKind::Modified))
                .collect();
            view.files = (!written.is_empty()).then_some(written);
            view.run_files = None;
            view.request_files = None;
        }
    }
    stored
}

// Cost: one session's turn in memory, a few milliseconds.
#[tokio::test(flavor = "local")]
async fn each_call_lists_the_calls_before_it_and_a_stored_transcript_lists_the_same() {
    // Two calls of one step: the first creates `a.txt` and returns last,
    // the second changes `b.txt`. The next call removes `a.txt`, in the same
    // run.
    let texts = ["", "one\n", "x\n", "y\n"];
    let blobs = MemoryBlobs::new();
    let mut copies = HashMap::new();
    for text in texts {
        let bytes = B64Bytes::new(text.as_bytes().to_vec());
        let blob = blobs.put(bytes.clone()).await.unwrap();
        copies.insert(blob, bytes.into_bytes());
    }
    let provider = ScriptedRuntime::new([
        Turn::Events(vec![
            event::tool_call(
                "call-1",
                "shell",
                json!({ "command": "1", "slow": true, "writes": [["/w/a.txt", "added", "", "one\n"]] }),
            ),
            event::tool_call(
                "call-2",
                "shell",
                json!({ "command": "2", "writes": [["/w/b.txt", "modified", "x\n", "y\n"]] }),
            ),
            event::response(1, 1),
        ]),
        Turn::Events(vec![
            event::tool_call("call-3", "shell", json!({ "command": "3", "removes": ["/w/a.txt"] })),
            event::response(1, 1),
        ]),
        Turn::Events(vec![event::text("Done."), event::response(1, 1)]),
    ]);
    let store = MemoryTreeStore::with_blobs(blobs);
    let (release, released) = oneshot::channel();
    let runtime = TestRuntime {
        together: vec!["shell"],
        ..test_runtime(vec![shell_tool(released)])
    };
    let session = start_on(&provider, runtime, &store, SessionConfig::default()).await;
    let running = session.send(text("Tidy up"), turn("t1")).unwrap();
    until(|| kinds(&session.transcript().blocks).contains(&"tool_call:completed".to_owned())).await;
    // The second call returned first: up to it, its run had changed only
    // what it changed.
    let blocks = session.transcript().blocks;
    assert_eq!(listed(&blocks[2], true), [("/w/b.txt".to_owned(), EditKind::Modified)]);

    release.send(()).unwrap();
    running.await.unwrap();
    let blocks = session.transcript().blocks;
    let at = |id: &str| {
        blocks
            .iter()
            .position(|block| matches!(block, Block::ToolCall(call) if call.tool_use_id == id))
            .unwrap()
    };
    // Once the first returned, the second's lists hold both.
    let both = [
        ("/w/a.txt".to_owned(), EditKind::Added),
        ("/w/b.txt".to_owned(), EditKind::Modified),
    ];
    assert_eq!(listed(&blocks[at("call-2")], true), both);
    assert_eq!(listed(&blocks[at("call-2")], false), both);
    // A file the run created and removed is at neither of its ends.
    let left = [("/w/b.txt".to_owned(), EditKind::Modified)];
    assert_eq!(listed(&blocks[at("call-3")], true), left);
    assert_eq!(listed(&blocks[at("call-3")], false), left);

    let mut stored = as_stored_before(&blocks);
    relist(&mut stored, |blob| Ok::<_, ()>(copies.get(blob).cloned())).unwrap();
    assert_eq!(stored, blocks);
}
