//! The files the agent gives the user (`commands.md` § Attachment commands):
//! `demi attachment upload` copies files beside the invocation into the
//! conversation as `a1`, `a2`, …, and a file it cannot copy fails with its own
//! line while the others are stored; the page reads an attachment by its number
//! from the conversation's database, and its bytes as a blob. A job `demi host
//! shell` runs on an attached Host uploads that Host's files, and a file whose
//! blob the namespace holds already is not read again. The model is an Anthropic
//! endpoint the test scripts; the devices are real runners.

use demi_agent_tools::testing::{field, shown_output};
use demi_provider_common::testing::MockVendor;
use demi_web_api_protocol::attachments::{ATTACHMENT_MAX_BYTES, ConversationAttachment};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::state::SyncEvent;
use reqwest::StatusCode;

use crate::conversations::{anthropic_at, create, on_device, working_on};
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
// a command needs it (`commands.md` § Attachment commands): the runner
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

// Several seconds: a real device runs two shell jobs, and one file is 26 MiB,
// which the upload refuses unread.
//
// Planted defect the large file catches: the Host hashes a file before it
// looks at its size. The file may be written but not read, so a Host that
// opens it to read fails with another reason.
#[tokio::test]
async fn upload_stores_each_file_as_the_next_attachment_and_names_each_one_it_cannot() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic_at(&backend, &master, &vendor, "/work").await;
    create(&backend, &master, CONVERSATION).await;
    let (_device, root) = on_device(&harness, &backend, &master, CONVERSATION).await;
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
        .turn(vec![shell("t1", script, 30_000), say("uploaded")])
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

    // Each is a record of the conversation with its blob, read without a
    // live Host; a number it does not have is not found.
    let read = async |number: &str| {
        backend
            .get(
                &format!("/api/conversations/{CONVERSATION}/attachments/{number}"),
                Some(&master),
            )
            .await
    };
    let first = read("a1").await;
    assert_eq!(first.status, StatusCode::OK);
    let first: ConversationAttachment = first.json();
    assert_eq!(
        (first.id.as_str(), first.name.as_str(), first.media_type.as_str(), first.size),
        ("a1", "login.png", "image/png", png as u64)
    );
    // An image's record carries its size in pixels, read from its header.
    assert_eq!((first.width, first.height), (Some(4), Some(3)));
    let second: ConversationAttachment = read("a2").await.json();
    assert_eq!(
        (second.name.as_str(), second.media_type.as_str(), second.size),
        ("notes.md", "text/markdown", 8)
    );
    assert_eq!((second.width, second.height), (None, None));
    for missing in ["a3", "a0", "3", "a01"] {
        let answer = read(missing).await;
        assert_eq!(
            (answer.status, answer.error().code),
            (StatusCode::NOT_FOUND, ErrorCode::NotFound),
            "{missing}"
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
    backend.close().await;
}

// Under a second: a real device runs one shell job.
//
// Planted defect it catches: the upload stores the attachment and tells no
// page, so a page that showed a reply naming `a1` before it existed keeps
// showing it missing. The job waits on a pipe after the upload until the
// test has seen the summary, so no other change of the conversation, such as
// the turn's end, can carry the number first.
#[tokio::test]
async fn the_pages_learn_of_a_new_attachment_from_the_summary_as_it_is_stored() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic_at(&backend, &master, &vendor, "/work").await;
    create(&backend, &master, CONVERSATION).await;
    let (_device, root) = on_device(&harness, &backend, &master, CONVERSATION).await;
    std::fs::write(format!("{root}/shot.png"), &*PNG).unwrap();
    let gate = format!("{root}/gate");
    let mut work = Driven::open(&backend, &master, &vendor, CONVERSATION, &provider, "/work").await;
    let mut page = backend.sync(&master).await;

    let script = "mkfifo gate && demi attachment upload shot.png > /dev/null && read -r go < gate";
    let turn = work.turn(vec![shell("t1", script, 30_000), say("uploaded")]);
    let announced = async {
        page.until(|event| {
            matches!(event, SyncEvent::Conversation { conversation }
                if conversation.id.as_str() == CONVERSATION && conversation.last_attachment == 1)
        })
        .await;
        // Opening the pipe to write waits for the job to read it.
        tokio::fs::write(&gate, b"go\n").await.unwrap();
    };
    let (uploaded, ()) = tokio::join!(turn, announced);
    assert_eq!(field(&uploaded.received[0], "exitCode"), "0", "{}", uploaded.received[0]);
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
    let attachment = format!("/api/conversations/{CONVERSATION}/attachments/a1");
    let turn = work.turn(vec![shell("t1", &script, 30_000), say("uploaded")]);
    let stored = async {
        eventually("the far job stored a1", async || {
            backend.get(&attachment, Some(&master)).await.status == StatusCode::OK
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
    let attachment: ConversationAttachment = backend.get(&attachment, Some(&master)).await.json();
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
            .turn(vec![shell(turn, "demi attachment upload shot.png", 30_000), say(turn)])
            .await;
        assert_eq!(
            shown_output(&uploaded.received[0]),
            format!("ID  MEDIA TYPE  SIZE (BYTES)  PATH\n{number}  image/png   {:<12}  shot.png\n", PNG.len())
        );
    }
    // The file's bytes went to the backend once, the first time.
    assert_eq!(edge.pipe_uploads_of(&PNG), 1);
    let read = async |number: &str| {
        backend
            .get(
                &format!("/api/conversations/{CONVERSATION}/attachments/{number}"),
                Some(&master),
            )
            .await
            .json::<ConversationAttachment>()
    };
    let (first, again) = (read("a1").await, read("a2").await);
    assert_eq!(first.blob, again.blob);
    // The held blob's opening gives the record the image's size all the same.
    assert_eq!((again.width, again.height), (Some(4), Some(3)));
    backend.close().await;
}

// A few seconds: the root, its child, the child's fork and a side question
// each run shell jobs on a real device.
//
// A reply shows only what its own agent uploaded, as the page asks for it with
// `?node=`: the root its a1, the child its a2, the side question its a3, each
// none of another's; the child's fork, whose copied history the child wrote,
// the child's a2. Planted defects this catches: an upload recorded by the
// agent's number, which a side question shares with the root, so the root shows
// the side's file; and a fork that shows only its own uploads, so its copied
// replies lose theirs.
#[tokio::test]
async fn a_reply_shows_the_uploads_of_its_own_agent_and_of_the_one_it_was_forked_from() {
    use demi_conversation_socket_protocol::{ServerFrame, SubagentEvent};
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
    // Each round's child waits until the root's turn has ended, so that its
    // completion wakes the idle root, which answers it.
    let go = format!("{root}/go");
    let mut round = async |id: &str, command: &str, child: Vec<Answer>| {
        let _ = std::fs::remove_file(&go);
        scripts.child(child);
        scripts.root(FIRST, vec![call(id, command), say("dispatched"), say("received")]);
        socket.chat(id, "Go").await;
        std::fs::write(&go, "").unwrap();
        socket
            .until(|frame| matches!(frame, ServerFrame::Subagent { event: SubagentEvent::Closed, .. }))
            .await;
        socket.until_idle().await;
    };
    round(
        "t1",
        "demi attachment upload root.png && demi agent spawn <<< 'Upload'",
        vec![call("c1", &format!("{WAIT} && demi attachment upload child.png")), say("child")],
    )
    .await;
    round("t2", "demi agent fork 1 <<< 'Again'", vec![call("f1", WAIT), say("forked")]).await;
    let tree = transcript(&backend, &master, FIRST).await;
    let node = |number: u64| {
        tree.subagents
            .iter()
            .find(|job| job.child.number == number)
            .map(|job| job.child.subagent_id.as_str().to_owned())
            .expect("the agent is listed")
    };
    let (child, fork) = (node(1), node(2));
    let reply = tree
        .blocks
        .iter()
        .rev()
        .find(|block| matches!(block, Block::Text(text) if text.text == "received"))
        .expect("the root answered")
        .id()
        .to_string();

    // A side question about the root's reply uploads its own file.
    scripts.child(vec![call("s1", "demi attachment upload side.png"), say("side")]);
    let asked = backend
        .post(
            &format!("/api/conversations/{FIRST}/side-questions"),
            Some(&master),
            serde_json::json!({ "id": SIDE, "blockId": reply, "start": 0, "end": 8, "quote": "received", "question": "Show it" }),
        )
        .await;
    assert_eq!(asked.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&asked.body));
    crate::support::eventually("the side question answers", || async {
        last_text(&crate::conversations::agent_history(&backend, &master, FIRST, Some(SIDE)).await.0) == "side"
    })
    .await;

    for (node, shown) in [
        (FIRST, ["a1"].as_slice()),
        (child.as_str(), ["a2"].as_slice()),
        (fork.as_str(), ["a2"].as_slice()),
        (SIDE, ["a3"].as_slice()),
    ] {
        let mut answered = Vec::new();
        for number in ["a1", "a2", "a3"] {
            let answer = backend
                .get(&format!("/api/conversations/{FIRST}/attachments/{number}?node={node}"), Some(&master))
                .await;
            match answer.status {
                StatusCode::OK => answered.push(number),
                StatusCode::NOT_FOUND => {}
                status => panic!("{number} for {node}: {status}"),
            }
        }
        assert_eq!(answered, shown, "the replies of {node}");
    }
    backend.close().await;
}
