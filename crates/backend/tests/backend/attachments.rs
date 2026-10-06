//! The files the agent gives the user (`commands.md` § Attachment commands,
//! `web-api.md` § Uploads and media): `demi attachment upload` copies files
//! beside the invocation into the conversation as `a1`, `a2`, …, and a file
//! it cannot copy fails with its own line while the others are stored; the
//! page reads an attachment by its number from the conversation's database,
//! and its bytes as a blob. A job `demi host shell` runs on an attached Host
//! uploads that Host's files, and a file whose blob the namespace holds
//! already is not read again. The model is an Anthropic endpoint the test
//! scripts; the devices are real runners.

use demi_agent_tools::testing::{field, shown_output};
use demi_provider_common::testing::MockVendor;
use demi_web_api_protocol::attachments::{ATTACHMENT_MAX_BYTES, ConversationAttachment};
use demi_web_api_protocol::error::ErrorCode;
use reqwest::StatusCode;

use crate::conversations::{anthropic_at, create, on_device, working_on};
use crate::holding_edge::HoldingEdge;
use crate::support::{Harness, eventually};
use crate::uploads::PNG;
use crate::work::{Driven, say, shell, switch};

const CONVERSATION: &str = "6a1b2c3d-8f3a-4c1e-9d2b-7a1c2e3f4a06";

// Several seconds: a real device runs two shell jobs, and one file is 25 MiB
// and a byte, which the upload refuses unread.
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
    std::fs::write(
        format!("{root}/huge.bin"),
        vec![0_u8; ATTACHMENT_MAX_BYTES + 1],
    )
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
            "a1  login.png    image/png      {png} bytes\na2  ../notes.md  text/markdown  8 bytes\n"
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
            "demi attachment upload: ../huge.bin: the file is {} bytes; an attachment is at most 25 MiB ({ATTACHMENT_MAX_BYTES} bytes)",
            ATTACHMENT_MAX_BYTES + 1
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
    let second: ConversationAttachment = read("a2").await.json();
    assert_eq!(
        (second.name.as_str(), second.media_type.as_str(), second.size),
        ("notes.md", "text/markdown", 8)
    );
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
    // next upload takes the next number, past the ones that failed. The
    // shell is still in `out`.
    std::fs::remove_file(format!("{root}/out/login.png")).unwrap();
    let again = work
        .turn(vec![
            shell("t2", "cd .. && demi attachment upload --json notes.md", 30_000),
            say("again"),
        ])
        .await;
    assert_eq!(
        shown_output(&again.received[0]),
        "{\"attachments\":[{\"id\":\"a3\",\"path\":\"notes.md\",\"mediaType\":\"text/markdown\",\"size\":8}]}\n"
    );
    let kept = backend
        .get(&format!("/api/blobs/{}", first.blob), Some(&master))
        .await;
    assert_eq!(kept.body, *PNG);
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
        format!("a1  shot.png  image/png  {} bytes\n", PNG.len()),
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

    let mut sent = Vec::new();
    for (turn, number) in [("t1", "a1"), ("t2", "a2")] {
        let before = edge.pipe_puts();
        let uploaded = work
            .turn(vec![shell(turn, "demi attachment upload shot.png", 30_000), say(turn)])
            .await;
        assert_eq!(
            shown_output(&uploaded.received[0]),
            format!("{number}  shot.png  image/png  {} bytes\n", PNG.len())
        );
        sent.push(edge.pipe_puts() - before);
    }
    // Each job sends the backend a pipe upload of its own; the file's bytes
    // went in one more, the first time only.
    assert_eq!(sent[0], sent[1] + 1, "{sent:?}");
    let blob = async |number: &str| {
        backend
            .get(
                &format!("/api/conversations/{CONVERSATION}/attachments/{number}"),
                Some(&master),
            )
            .await
            .json::<ConversationAttachment>()
            .blob
    };
    assert_eq!(blob("a1").await, blob("a2").await);
    backend.close().await;
}
