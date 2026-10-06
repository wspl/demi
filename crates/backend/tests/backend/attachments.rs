//! The files the agent gives the user (`commands.md` § Attachment commands,
//! `web-api.md` § Uploads and media): `demi attachment upload` copies files
//! beside the invocation into the conversation as `a1`, `a2`, …, and a file
//! it cannot copy fails with its own line while the others are stored; the
//! page reads an attachment by its number from the conversation's database,
//! and its bytes as a blob. The model is an Anthropic endpoint the test
//! scripts; the device is a real runner.

use demi_agent_tools::testing::{field, shown_output};
use demi_provider_common::testing::MockVendor;
use demi_web_api_protocol::attachments::{ATTACHMENT_MAX_BYTES, ConversationAttachment};
use demi_web_api_protocol::error::ErrorCode;
use reqwest::StatusCode;

use crate::conversations::{anthropic_at, create, on_device};
use crate::support::Harness;
use crate::uploads::PNG;
use crate::work::{Driven, say, shell};

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
            "a1  login.png    image/png      {png} B\na2  ../notes.md  text/markdown  8 B\n"
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
