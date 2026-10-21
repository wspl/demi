//! The files the agent gives the user
//! (`crates/backend-host-access/src/attachment_commands.rs`).
//!
//! Scenarios:
//!
//! - `demi attachment upload` copies files beside the invocation into the
//!   conversation as `a1`, `a2`, …. A file it cannot copy fails with its own
//!   line while the others are stored.
//! - A reply that names an attachment by its number carries its description
//!   from the conversation's database, and the page reads its bytes as a
//!   blob, with the file it was copied from: the device and the file's full
//!   path, which the page opens in the copy's place while it can read it.
//! - A page that shows the conversation is sent the description of each
//!   attachment as it is stored, since a reply may name it first.
//! - A job `demi host shell` runs on an attached Host uploads that Host's
//!   files. A file whose blob the namespace holds already is not read again.
//! - A complete reply that shows pictures of the Host keeps a copy of each as
//!   an attachment of the conversation, or why it could not be copied
//!   (`docs/decisions/0046-a-reply-keeps-the-pictures-it-shows.md`).
//!
//! The model is an Anthropic endpoint the test scripts; the devices are real
//! runners.

use demi_agent_tools::testing::{field, shown_output};
use demi_conversation_socket_protocol::{ServerFrame, TranscriptPatch};
use demi_provider_common::testing::{MockResponse, MockVendor};
use demi_shared_types::Block;
use serde_json::json;
use demi_web_api_protocol::attachments::{ATTACHMENT_MAX_BYTES, ConversationAttachment};
use demi_web_api_protocol::error::ErrorCode;
use reqwest::StatusCode;

use crate::conversations::{
    anthropic_at, create, delivering, events, message_start, metadata, named_attachments, on_device, text_block, working_on,
};
use crate::holding_edge::HoldingEdge;
use crate::support::{Harness, eventually};
use crate::uploads::PNG;
use crate::work::{Driven, say, shell, switch};

const CONVERSATION: &str = "6a1b2c3d-8f3a-4c1e-9d2b-7a1c2e3f4a06";

/// A file over the 25 MiB an attachment may have.
const HUGE: u64 = 26 * 1024 * 1024;

// About a second: a real device runs two shell jobs.
//
// `demi attachment download` puts an attachment, the agent's own here, where
// a command needs it (`crates/backend-host-access/src/attachment_commands.rs`): the runner
// fetches it from the backend and writes it under its name; run again, the
// same file already there is that file, and a number the conversation does
// not have fails with its own line while the others are written.
#[tokio::test]
async fn download_writes_each_attachment_under_its_name_and_the_same_file_again_is_that_file() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic_at(&backend, &master, &vendor, "/work").await;
    create(&backend, &master, CONVERSATION).await;
    let (_device, root) = on_device(&harness, &backend, &master, CONVERSATION).await;
    std::fs::write(format!("{root}/login.png"), &*PNG).unwrap();
    let mut work = Driven::open(&backend, &master, &vendor, CONVERSATION, &provider, "/work").await;
    work.turn(vec![shell("t1", "demi attachment upload login.png", 30_000), say("uploaded")])
        .await;

    let script = "mkdir in && demi attachment download a1 a9 --dir in; demi attachment download --json a1 --dir in";
    let fetched = work.turn(vec![shell("t2", script, 30_000), say("fetched")]).await;
    let output = shown_output(&fetched.received[0]);
    assert!(output.contains("ID  PATH\na1  in/login.png\n"), "{output}");
    assert!(
        output.contains("demi attachment download: a9: the conversation has no attachment a9"),
        "{output}"
    );
    assert!(output.contains(r#"[{"id":"a1","path":"in/login.png"}]"#), "{output}");
    assert_eq!(std::fs::read(format!("{root}/in/login.png")).unwrap(), *PNG);
    assert_eq!(std::fs::read_dir(format!("{root}/in")).unwrap().count(), 1);
    backend.close().await;
}

// Several seconds: a real device runs three shell jobs, and one file is 26 MiB,
// which the upload refuses unread.
//
// Planted defects it catches:
//
// - The Host hashes a file before it looks at its size, which the large file
//   catches. The file may be written but not read, so a Host that opens it to
//   read fails with another reason.
// - The record keeps the path as the command was given it, `../notes.md`,
//   which names no file once the page reads it against another directory.
// - The record keeps the path resolved by its text, which a symbolic link
//   makes another file than the one read: `link/../report.md` read through
//   the link is the `report.md` beside the link's target.
#[tokio::test]
async fn upload_stores_each_file_as_the_next_attachment_and_names_each_one_it_cannot() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic_at(&backend, &master, &vendor, "/work").await;
    create(&backend, &master, CONVERSATION).await;
    let (device, root) = on_device(&harness, &backend, &master, CONVERSATION).await;
    std::fs::create_dir_all(format!("{root}/out")).unwrap();
    std::fs::write(format!("{root}/out/login.png"), &*PNG).unwrap();
    std::fs::write(format!("{root}/notes.md"), "# Notes\n").unwrap();
    let huge = std::fs::File::create(format!("{root}/huge.bin")).unwrap();
    huge.set_len(HUGE).unwrap();
    huge.set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o200))
        .unwrap();
    let mut work = Driven::open(&backend, &master, &vendor, CONVERSATION, &provider, "/work").await;

    // Paths resolve against the directory the command runs in.
    let script = "cd out && demi attachment upload login.png ../notes.md missing.png ../huge.bin";
    let uploaded = work
        .turn(vec![shell("t1", script, 30_000), say(&delivering("uploaded", &["a1", "a2", "a3"]))])
        .await;
    let result = &uploaded.received[0];
    assert_eq!(field(result, "exitCode"), "1", "{result}");
    // Standard output and standard error arrive apart, so only each one's
    // own order is fixed.
    let output = shown_output(result);
    let png = PNG.len();
    assert!(
        output.contains(&format!(
            "ID  MEDIA TYPE     SIZE (BYTES)  PATH\na1  image/png      {png:<12}  login.png\na2  text/markdown  8             ../notes.md\n"
        )),
        "{output}"
    );
    let failures: Vec<&str> = output
        .lines()
        .filter(|line| line.starts_with("demi attachment upload: "))
        .collect();
    let [missing, huge] = failures.as_slice() else {
        panic!("a line for each path that failed: {output}");
    };
    assert!(missing.starts_with("demi attachment upload: missing.png: "), "{missing}");
    assert!(missing.len() > "demi attachment upload: missing.png: ".len(), "{missing}");
    assert_eq!(
        *huge,
        format!(
            "demi attachment upload: ../huge.bin: the file is {HUGE} bytes; an attachment is at most 25 MiB ({ATTACHMENT_MAX_BYTES} bytes)"
        )
    );

    // Each is a record of the conversation with its blob, which the reply
    // that names it carries, read without a live Host; a number it does not
    // have is named by nothing.
    let described = named_attachments(&backend, &master, CONVERSATION, CONVERSATION).await;
    // A page that opens the conversation is sent them with the reply, in
    // its handshake.
    let mut page = crate::conversations::Socket::connect(&backend, &master, CONVERSATION).await;
    let handshake = page.open().await;
    let reset = handshake.iter().find_map(|frame| match frame {
        ServerFrame::TranscriptReset { attachments, .. } => Some(attachments),
        _ => None,
    });
    assert_eq!(reset, Some(&described), "{handshake:?}");
    drop(page);
    let [first, second]: [ConversationAttachment; 2] = described.try_into().expect("a1 and a2 alone");
    assert_eq!(
        (first.id.as_str(), first.name.as_str(), first.media_type.as_str(), first.size),
        ("a1", "login.png", "image/png", png as u64)
    );
    // An image's record carries its size in pixels, read from its header.
    assert_eq!((first.width, first.height), (Some(4), Some(3)));
    assert_eq!(
        (second.name.as_str(), second.media_type.as_str(), second.size),
        ("notes.md", "text/markdown", 8)
    );
    assert_eq!((second.width, second.height), (None, None));
    // Each keeps the file it was copied from: the device the command ran on,
    // and the full path of the file read, as that Host writes it.
    for (attachment, file) in [(&first, format!("{root}/out/login.png")), (&second, format!("{root}/notes.md"))] {
        let source = attachment.source.as_ref().expect("an upload keeps its file");
        assert_eq!(source.host.as_str(), device.id());
        assert_eq!(
            std::fs::canonicalize(&source.path).unwrap(),
            std::fs::canonicalize(&file).unwrap(),
            "{}",
            source.path
        );
    }
    let bytes = backend
        .get(&format!("/api/blobs/{}", first.blob), Some(&master))
        .await;
    assert_eq!(bytes.body, *PNG);

    // An attachment is a copy: the file's later fate changes nothing, and the
    // next upload takes the next number, past the ones that failed.
    std::fs::remove_file(format!("{root}/out/login.png")).unwrap();
    let again = work
        .turn(vec![
            shell("t2", "demi attachment upload --json notes.md", 30_000),
            say("again"),
        ])
        .await;
    assert_eq!(
        shown_output(&again.received[0]),
        "[{\"id\":\"a3\",\"path\":\"notes.md\",\"mediaType\":\"text/markdown\",\"size\":8}]\n"
    );
    let kept = backend
        .get(&format!("/api/blobs/{}", first.blob), Some(&master))
        .await;
    assert_eq!(kept.body, *PNG);

    // A path through a symbolic link is recorded as the file the Host read:
    // `out/link` leads to `exports/subdir`, so `link/../report.md` is
    // `exports/report.md`, and `out/report.md` is no file at all.
    let exports = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(exports.path().join("subdir")).unwrap();
    std::fs::write(exports.path().join("report.md"), "# Report\n").unwrap();
    std::os::unix::fs::symlink(exports.path().join("subdir"), format!("{root}/out/link")).unwrap();
    let linked = work
        .turn(vec![
            shell("t3", "cd out && demi attachment upload link/../report.md", 30_000),
            say(&delivering("linked", &["a4"])),
        ])
        .await;
    assert_eq!(field(&linked.received[0], "exitCode"), "0", "{}", linked.received[0]);
    let report = named_attachments(&backend, &master, CONVERSATION, CONVERSATION)
        .await
        .into_iter()
        .find(|attachment| attachment.id == "a4")
        .expect("the reply carries a4");
    let read_file = std::fs::canonicalize(exports.path().join("report.md")).unwrap();
    assert_eq!(
        report.source.map(|source| source.path),
        Some(read_file.to_str().unwrap().to_owned())
    );
    backend.close().await;
}

// Under a second: a real device runs one shell job.
//
// Planted defect it catches: the upload stores the attachment and tells no
// page, so a page that showed a reply naming `a1` before it existed keeps
// showing it missing. The job waits on a pipe after the upload until the
// test has seen the description, so no other change of the conversation,
// such as the turn's end, can carry it first.
#[tokio::test]
async fn the_pages_are_sent_a_new_attachment_with_its_description_as_it_is_stored() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic_at(&backend, &master, &vendor, "/work").await;
    create(&backend, &master, CONVERSATION).await;
    let (_device, root) = on_device(&harness, &backend, &master, CONVERSATION).await;
    std::fs::write(format!("{root}/shot.png"), &*PNG).unwrap();
    let gate = format!("{root}/gate");
    let mut work = Driven::open(&backend, &master, &vendor, CONVERSATION, &provider, "/work").await;
    let (mut page, _) = crate::conversations::Socket::shown(&backend, &master, CONVERSATION, 0).await;

    let script = "mkfifo gate && demi attachment upload shot.png > /dev/null && read -r go < gate";
    let turn = work.turn(vec![shell("t1", script, 30_000), say("uploaded")]);
    let announced = async {
        let (node, attachments) = page.attachments_until().await;
        assert_eq!(node.as_str(), CONVERSATION);
        assert_eq!(
            attachments.iter().map(|attachment| (attachment.id.as_str(), attachment.name.as_str())).collect::<Vec<_>>(),
            [("a1", "shot.png")]
        );
        // Opening the pipe to write waits for the job to read it.
        tokio::fs::write(&gate, b"go\n").await.unwrap();
    };
    let (uploaded, ()) = tokio::join!(turn, announced);
    assert_eq!(field(&uploaded.received[0], "exitCode"), "0", "{}", uploaded.received[0]);
    backend.close().await;
}

// Under a second: a real device runs one shell job, and the reply that names
// its upload streams in two pieces.
//
// Planted defect it catches: a frame names attachments from its own delta
// alone, so the piece that completes a tag split between two deltas carries
// no description, and the page draws the reply's picture as unavailable until
// a later frame of the block, if any, brings it.
#[tokio::test]
async fn a_tag_split_across_deltas_carries_its_attachment_with_the_piece_that_completes_it() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic_at(&backend, &master, &vendor, "/work").await;
    create(&backend, &master, CONVERSATION).await;
    let (_device, root) = on_device(&harness, &backend, &master, CONVERSATION).await;
    std::fs::write(format!("{root}/shot.png"), &*PNG).unwrap();
    let mut work = Driven::open(&backend, &master, &vendor, CONVERSATION, &provider, "/work").await;

    // The reply's second piece waits until the test has seen the first.
    let (release, held) = tokio::sync::watch::channel(false);
    let usage = json!({ "input_tokens": 1, "output_tokens": 0 });
    let mut head = vec![message_start(usage)];
    head.extend(text_block(0, &["Here it is:\n\n<attachm"]));
    let tail = [
        json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "text_delta", "text": "ent id=\"a1\" />\n" } }),
        json!({ "type": "content_block_stop", "index": 0 }),
        json!({ "type": "message_delta", "delta": { "stop_reason": "end_turn" }, "usage": { "output_tokens": 1 } }),
        json!({ "type": "message_stop" }),
    ];
    let streamed = MockResponse::status(200)
        .header("content-type", "text/event-stream")
        .chunk(events(&head))
        .then_held(held)
        .chunk(events(&tail));
    work.start(vec![shell("t1", "demi attachment upload shot.png", 30_000), streamed]).await;

    // The first piece arrives with the block's first patch, the next as text
    // appended to it.
    let begun = |frame: &ServerFrame| {
        matches!(frame, ServerFrame::TranscriptPatch { patches, .. } if patches.iter().any(|patch| matches!(
            patch,
            TranscriptPatch::Add { value: Block::Text(text), .. } if text.text.contains("<attachm")
        )))
    };
    let appended = |frame: &ServerFrame, piece: &str| {
        matches!(frame, ServerFrame::TranscriptPatch { patches, .. } if patches.iter().any(|patch| matches!(
            patch,
            TranscriptPatch::AppendText { delta, .. } if delta.contains(piece)
        )))
    };
    let first = work.socket.until(|frame| begun(frame)).await;
    release.send(true).unwrap();
    let rest = work.socket.until_idle().await;

    let carried = |frame: &ServerFrame| match frame {
        ServerFrame::TranscriptPatch { attachments, .. } => {
            attachments.iter().map(|attachment| attachment.id.clone()).collect::<Vec<_>>()
        }
        _ => Vec::new(),
    };
    // The first piece names no attachment, and the piece that completes the
    // tag brings a1 in the same frame, before any later frame of the block.
    assert!(first.iter().all(|frame| carried(frame).is_empty()), "{first:?}");
    let completing = rest.iter().find(|frame| appended(frame, "ent id")).expect("the second piece reaches the page");
    assert_eq!(carried(completing), ["a1"], "{completing:?}");
    backend.close().await;
}

// Under a second: two real devices each run a job, one of them through `demi
// host shell`.
//
// Planted defect it catches: the upload reads on the conversation's primary
// Host whatever Host the invoking job runs on. The devices here share one
// filesystem, so the far job stops the primary's runner before it uploads:
// a read there never answers, and the attachment never comes.
#[tokio::test]
async fn a_job_on_an_attached_host_uploads_that_hosts_file() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    let beta = backend.pair(&master, "beta").await;
    let provider = anthropic_at(&backend, &master, &vendor, "/work").await;
    create(&backend, &master, CONVERSATION).await;
    // Working on alpha and then on beta leaves alpha attached.
    let on_alpha = alpha.runner.home_dir().to_owned();
    let on_beta = beta.runner.home_dir().to_owned();
    switch(&backend, &master, CONVERSATION, &alpha, &on_alpha).await;
    switch(&backend, &master, CONVERSATION, &beta, &on_beta).await;
    std::fs::create_dir_all(on_alpha.join("out")).unwrap();
    std::fs::write(on_alpha.join("out/shot.png"), &*PNG).unwrap();
    let mut work = Driven::open(&backend, &master, &vendor, CONVERSATION, &provider, "/work").await;

    let script = format!(
        "demi host shell --host alpha 'kill -STOP {} && cd out && demi attachment upload shot.png'",
        beta.runner.pid()
    );
    let turn = work.turn(vec![shell("t1", &script, 30_000), say(&delivering("uploaded", &["a1"]))]);
    let stored = async {
        eventually("the far job stored a1", async || {
            metadata(&backend, &master, CONVERSATION).await.last_attachment == 1
        })
        .await;
        beta.runner.resume();
    };
    let (uploaded, ()) = tokio::join!(turn, stored);
    let result = &uploaded.received[0];
    assert_eq!(
        shown_output(result),
        format!("ID  MEDIA TYPE  SIZE (BYTES)  PATH\na1  image/png   {:<12}  shot.png\n", PNG.len()),
        "{result}"
    );
    let [attachment]: [ConversationAttachment; 1] = named_attachments(&backend, &master, CONVERSATION, CONVERSATION)
        .await
        .try_into()
        .expect("the reply carries a1");
    let bytes = backend
        .get(&format!("/api/blobs/{}", attachment.blob), Some(&master))
        .await;
    assert_eq!(bytes.body, *PNG);
    backend.close().await;
}

// About a second: a real device runs two shell jobs through an edge that
// counts its pipe uploads.
#[tokio::test]
async fn a_file_uploaded_again_is_hashed_on_the_host_and_its_bytes_not_read_again() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic_at(&backend, &master, &vendor, "/work").await;
    create(&backend, &master, CONVERSATION).await;
    let edge = HoldingEdge::start(backend.address()).await;
    let device = backend.pair_through(&master, "laptop", &edge.url).await;
    let root = working_on(&harness, &device, CONVERSATION);
    std::fs::write(format!("{root}/shot.png"), &*PNG).unwrap();
    let mut work = Driven::open(&backend, &master, &vendor, CONVERSATION, &provider, "/work").await;

    for (turn, number) in [("t1", "a1"), ("t2", "a2")] {
        let uploaded = work
            .turn(vec![shell(turn, "demi attachment upload shot.png", 30_000), say(&delivering(turn, &[number]))])
            .await;
        assert_eq!(
            shown_output(&uploaded.received[0]),
            format!("ID  MEDIA TYPE  SIZE (BYTES)  PATH\n{number}  image/png   {:<12}  shot.png\n", PNG.len())
        );
    }
    // The file's bytes went to the backend once, the first time.
    assert_eq!(edge.pipe_uploads_of(&PNG), 1);
    let [first, again]: [ConversationAttachment; 2] = named_attachments(&backend, &master, CONVERSATION, CONVERSATION)
        .await
        .try_into()
        .expect("the replies carry a1 and a2");
    assert_eq!(first.blob, again.blob);
    // The held blob's opening gives the record the image's size all the same.
    assert_eq!((again.width, again.height), (Some(4), Some(3)));
    backend.close().await;
}

// A few seconds: the root, its child and a side question each run shell jobs
// on a real device.
//
// A reply shows only what its own agent uploaded, as each transcript page
// carries the attachments its replies name:
//
// - The root shows its a1, the child its a2, the side question its a3, each
//   none of another's.
//
// Planted defect this catches: an upload recorded by the agent's number, which
// a side question shares with the root, so the root shows the side's file.
#[tokio::test]
async fn a_reply_shows_the_uploads_of_its_own_agent() {
    use demi_shared_types::Block;

    use crate::conversations::{FIRST, last_text, transcript};
    use crate::subagents::Answer;
    use crate::subagents::{Scripts, WAIT, say, shell as call, tree_on};

    const SIDE: &str = "5b0e7c1a-2f3d-4e6a-9b8c-7d6e5f4a3b2d";
    let scripts = std::sync::Arc::new(Scripts::default());
    let (_harness, backend, master, _paired, root, _provider) = tree_on(&scripts, Harness::new()).await;
    for name in ["root.png", "child.png", "side.png"] {
        std::fs::write(format!("{root}/{name}"), &*PNG).unwrap();
    }
    let mut socket = crate::conversations::Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    // Every agent's last reply names all three; each shows its own.
    let named = |text: &str| delivering(text, &["a1", "a2", "a3"]);
    // Each round's child waits until the root's turn has ended, so that its
    // completion wakes the idle root, which answers it.
    let go = format!("{root}/go");
    let mut round = async |id: &str, command: &str, child: Vec<Answer>| {
        let _ = std::fs::remove_file(&go);
        scripts.child(child);
        scripts.root(FIRST, vec![call(id, command), say("dispatched"), say(&named("received"))]);
        socket.chat(id, "Go").await;
        std::fs::write(&go, "").unwrap();
        socket.until_child_closed().await;
        socket.until_idle().await;
    };
    round(
        "t1",
        "demi attachment upload root.png && demi agent spawn <<< 'Upload'",
        vec![call("c1", &format!("{WAIT} && demi attachment upload child.png")), say(&named("child"))],
    )
    .await;
    let tree = transcript(&backend, &master, FIRST).await;
    let node = |number: u64| {
        tree.subagents
            .iter()
            .find(|entry| crate::conversations::child_of(entry).number == number)
            .map(|entry| entry.id.as_str().to_owned())
            .expect("the agent is listed")
    };
    let child = node(1);
    let reply = tree
        .blocks
        .iter()
        .rev()
        .find(|block| matches!(block, Block::Text(text) if text.text.starts_with("received")))
        .expect("the root answered")
        .id()
        .to_string();

    // A side question about the root's reply uploads its own file.
    scripts.child(vec![call("s1", "demi attachment upload side.png"), say(&named("side"))]);
    let asked = backend
        .post(
            &format!("/api/conversations/{FIRST}/side-questions"),
            Some(&master),
            serde_json::json!({ "id": SIDE, "blockId": reply, "start": 0, "end": 8, "quote": "received", "question": "Show it" }),
        )
        .await;
    assert_eq!(asked.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&asked.body));
    crate::support::eventually("the side question answers", || async {
        last_text(&crate::conversations::agent_history(&backend, &master, FIRST, Some(SIDE)).await.0).starts_with("side")
    })
    .await;

    for (node, shown) in [
        (FIRST, ["a1"].as_slice()),
        (child.as_str(), ["a2"].as_slice()),
        (SIDE, ["a3"].as_slice()),
    ] {
        let answered: Vec<String> = named_attachments(&backend, &master, FIRST, node)
            .await
            .into_iter()
            .map(|attachment| attachment.id)
            .collect();
        assert_eq!(answered, shown, "the replies of {node}");
    }
    backend.close().await;
}

// About a second: a real device runs one shell job, and a second device takes
// the conversation.
//
// Each time the page opens a card of an attachment it asks whether the file it
// was uploaded from is there (`GET .../attachments/:n/file`): yes while the
// conversation's Host has it, no once it is gone, and no once the conversation
// runs on another Host, though that Host reads the same path.
//
// Planted defects it catches: the check made on whatever Host the conversation
// runs on, which finds the path on the second device, since the devices here
// share one file system; and an attachment of no file that answers yes.
#[tokio::test]
async fn whether_an_attachments_file_is_there_is_asked_of_the_host_it_came_from() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic_at(&backend, &master, &vendor, "/work").await;
    create(&backend, &master, CONVERSATION).await;
    let (_device, root) = on_device(&harness, &backend, &master, CONVERSATION).await;
    std::fs::write(format!("{root}/notes.md"), "# Notes\n").unwrap();
    std::fs::write(format!("{root}/login.png"), &*PNG).unwrap();
    let mut work = Driven::open(&backend, &master, &vendor, CONVERSATION, &provider, "/work").await;
    let uploaded = work
        .turn(vec![shell("t1", "demi attachment upload notes.md login.png", 30_000), say("uploaded")])
        .await;
    assert_eq!(field(&uploaded.received[0], "exitCode"), "0", "{}", uploaded.received[0]);
    let check = async |number: &str| {
        backend
            .get(
                &format!("/api/conversations/{CONVERSATION}/attachments/{number}/file"),
                Some(&master),
            )
            .await
    };
    let no_file = (StatusCode::NOT_FOUND, ErrorCode::NotFound);
    for number in ["a1", "a2"] {
        let answer = check(number).await;
        assert_eq!(answer.status, StatusCode::NO_CONTENT, "{}", String::from_utf8_lossy(&answer.body));
    }
    assert_eq!(check("a3").await.refusal(), no_file);

    std::fs::remove_file(format!("{root}/notes.md")).unwrap();
    assert_eq!(check("a1").await.refusal(), no_file);
    std::fs::write(format!("{root}/notes.md"), "# Notes\n").unwrap();
    assert_eq!(check("a1").await.status, StatusCode::NO_CONTENT);

    let beta = backend.pair(&master, "beta").await;
    let on_beta = beta.runner.home_dir().to_owned();
    switch(&backend, &master, CONVERSATION, &beta, &on_beta).await;
    assert_eq!(check("a1").await.refusal(), no_file);
    backend.close().await;
}

// About a second: a real device and one scripted turn.
//
// A complete reply that shows pictures of the Host keeps the copy of each as
// an attachment of the node that wrote it, beside the path the reply wrote, or
// why the file was not copied, and a web picture is
// no business of the backend. The frame that settles a copy carries its
// description, so the page draws it at its size with the reply and asks no
// Host. The copy stays what the file was when the reply ended, across the
// file's removal and a restart.
//
// Planted defects it catches: a reply that keeps no copy, a copy that is not
// the node's own attachment, and a failure that is not kept.
#[tokio::test]
async fn a_complete_reply_keeps_a_copy_of_each_picture_of_the_host_it_shows() {
    use crate::support::{MASTER_EMAIL, MASTER_PASSWORD};
    use crate::conversations::transcript;
    use demi_shared_types::{PictureCopy, PictureFailure, ReplyPicture};

    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic_at(&backend, &master, &vendor, "/work").await;
    create(&backend, &master, CONVERSATION).await;
    let (device, root) = on_device(&harness, &backend, &master, CONVERSATION).await;
    std::fs::create_dir_all(format!("{root}/out")).unwrap();
    std::fs::write(format!("{root}/out/chart.png"), &*PNG).unwrap();
    std::fs::write(format!("{root}/notes.md"), "# Notes\n").unwrap();
    let mut work = Driven::open(&backend, &master, &vendor, CONVERSATION, &provider, "/work").await;

    work.turn(vec![say(
        "![chart](out/chart.png) ![again](out/chart.png) ![gone](missing.png) ![notes](notes.md) ![web](https://example.com/web.png)",
    )])
    .await;
    // The frames up to the last picture settling.
    let settled = |frame: &ServerFrame| match frame {
        ServerFrame::TranscriptPatch { patches, .. } => patches.iter().any(|patch| {
            matches!(patch, TranscriptPatch::ReplaceBlock { value: Block::Text(text), .. }
                if text.pictures.as_ref().is_some_and(|pictures| {
                    pictures.len() == 3 && pictures.iter().all(|picture| picture.copy != PictureCopy::Copying)
                }))
        }),
        _ => false,
    };
    let frames = work.socket.until(settled).await;
    // The frame that settles the copy carries its description, once.
    let attachments: Vec<&ConversationAttachment> = frames
        .iter()
        .flat_map(|frame| match frame {
            ServerFrame::TranscriptPatch { attachments, .. } => attachments.iter().collect(),
            _ => Vec::new(),
        })
        .collect();
    let [copy]: [&ConversationAttachment; 1] = attachments.try_into().expect("a1 alone");
    assert_eq!((copy.id.as_str(), copy.name.as_str(), copy.media_type.as_str()), ("a1", "chart.png", "image/png"));
    assert_eq!((copy.width, copy.height), (Some(4), Some(3)));
    assert_eq!(copy.source.as_ref().map(|source| source.host.as_str()), Some(device.id()));

    // The copy is the file as it was, whatever becomes of the file, and the
    // reply keeps its answers across a restart.
    std::fs::remove_file(format!("{root}/out/chart.png")).unwrap();
    drop(work);
    backend.close().await;
    let backend = harness.start().await;
    let master = backend.login(MASTER_EMAIL, MASTER_PASSWORD).await;
    let blocks = transcript(&backend, &master, CONVERSATION).await.blocks;
    let Some(Block::Text(reply)) = blocks.iter().rev().find(|block| matches!(block, Block::Text(_))) else {
        panic!("{blocks:?}")
    };
    let pictures = reply.pictures.clone().expect("the reply keeps its pictures");
    let [chart, gone, notes]: [ReplyPicture; 3] = pictures.try_into().expect("a picture for each file path, once");
    assert_eq!(chart, ReplyPicture { target: "out/chart.png".into(), copy: PictureCopy::Copied { attachment: "a1".into() } });
    assert_eq!(gone, ReplyPicture { target: "missing.png".into(), copy: PictureCopy::Failed { reason: PictureFailure::Missing } });
    assert_eq!(
        notes,
        ReplyPicture {
            target: "notes.md".into(),
            copy: PictureCopy::Failed { reason: PictureFailure::NotAnImage { media_type: "text/markdown".into() } }
        }
    );
    let described = named_attachments(&backend, &master, CONVERSATION, CONVERSATION).await;
    let [stored]: [ConversationAttachment; 1] = described.try_into().expect("only the copy that was made");
    let bytes = backend.get(&format!("/api/blobs/{}", stored.blob), Some(&master)).await;
    assert_eq!(bytes.body, *PNG);
    backend.close().await;
}

// About a second: a real device and one scripted turn.
//
// A picture over the limit of an attachment is refused unread, by the size
// the Host names, and a path from the Host's home, `~/chart.png`, is read
// from the home the device reported, as the file beside the conversation's
// directory is not. Each is kept as the cause and nothing else.
//
// Planted defects it catches: a `~/` path read against the working directory
// (the file is not there), and a file read whole before its size is known.
#[tokio::test]
async fn a_picture_over_the_limit_is_refused_by_its_size_and_one_in_the_home_is_copied() {
    use demi_shared_types::{PictureCopy, PictureFailure, ReplyPicture};

    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic_at(&backend, &master, &vendor, "/work").await;
    create(&backend, &master, CONVERSATION).await;
    let (device, root) = on_device(&harness, &backend, &master, CONVERSATION).await;
    // The conversation works in `{home}/work`; the picture is in the home
    // itself, and a file of the same name is not beside the conversation.
    let home = device.runner.home().to_owned();
    std::fs::write(format!("{home}/chart.png"), &*PNG).unwrap();
    let huge = std::fs::File::create(format!("{root}/huge.png")).unwrap();
    huge.set_len(HUGE).unwrap();
    huge.set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o200))
        .unwrap();
    let mut work = Driven::open(&backend, &master, &vendor, CONVERSATION, &provider, "/work").await;

    work.turn(vec![say("![big](huge.png) ![home](~/chart.png)")]).await;
    let settled = |frame: &ServerFrame| match frame {
        ServerFrame::TranscriptPatch { patches, .. } => patches.iter().any(|patch| {
            matches!(patch, TranscriptPatch::ReplaceBlock { value: Block::Text(text), .. }
                if text.pictures.as_ref().is_some_and(|pictures| {
                    pictures.len() == 2 && pictures.iter().all(|picture| picture.copy != PictureCopy::Copying)
                }))
        }),
        _ => false,
    };
    let frames = work.socket.until(settled).await;
    let Some(ServerFrame::TranscriptPatch { patches, .. }) = frames.last() else {
        panic!("{frames:?}")
    };
    let Some(TranscriptPatch::ReplaceBlock { value: Block::Text(reply), .. }) = patches.last() else {
        panic!("{patches:?}")
    };
    assert_eq!(
        reply.pictures,
        Some(vec![
            ReplyPicture { target: "huge.png".into(), copy: PictureCopy::Failed { reason: PictureFailure::TooLarge { size: HUGE } } },
            ReplyPicture { target: "~/chart.png".into(), copy: PictureCopy::Copied { attachment: "a1".into() } },
        ])
    );
    let described: Vec<&ConversationAttachment> = frames
        .iter()
        .flat_map(|frame| match frame {
            ServerFrame::TranscriptPatch { attachments, .. } => attachments.iter().collect(),
            _ => Vec::new(),
        })
        .collect();
    let [stored]: [&ConversationAttachment; 1] = described.try_into().expect("only the picture in the home");
    assert_eq!((stored.name.as_str(), stored.media_type.as_str()), ("chart.png", "image/png"));
    backend.close().await;
}
