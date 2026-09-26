//! Uploads (`web-api.md` § Uploads and media, `backend.md` § Media by
//! reference): a file's bytes go to the caller's blobs with its record, the
//! answer says what the backend read from them, and a message or a steer
//! that names the upload writes the file to the conversation's Host, gives
//! the model the file's native medium and record, and shows the page the
//! medium by reference. A frame whose media cannot be stored reaches the
//! page as an error, and the frames after it still arrive. No test calls a
//! real model.

use demi_agent::testing::model_of;
use demi_agent_protocol::{ClientContent, ClientFrame, ServerFrame, SteerOutcome};
use demi_core::{Block, BlockId, FileExtension, MediaSource, SessionPhase, TurnId, UserContentBlock};
use demi_provider::testing::MockVendor;
use demi_web_api::attachments::{ATTACHMENT_MAX_BYTES, AttachmentAnswer, AttachmentDto};
use demi_web_api::auth::Role;
use demi_web_api::error::ErrorCode;
use reqwest::{Method, StatusCode};
use serde_json::json;
use sha2::{Digest as _, Sha256};

use crate::conversations::{FIRST, Socket, anthropic, answer, create, on_device, send, tool_use, transcript};
use crate::support::{Answer, Harness, Session, TestBackend, answer as read};

/// A PNG image's first bytes, from which the backend reads its type.
const PNG: [u8; 12] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0xff, 0xfe, 0x01];

async fn post(backend: &TestBackend, session: &Session, query: &str, media_type: Option<&str>, bytes: Vec<u8>) -> Answer {
    let headers: Vec<(&str, &str)> = media_type.into_iter().map(|media_type| ("content-type", media_type)).collect();
    let path = format!("/api/attachments{query}");
    read(backend.response(Method::POST, &path, session, &headers, Some(bytes.into())).await).await
}

/// Uploads `bytes` sent as `media_type` under the name `name`.
async fn upload(backend: &TestBackend, session: &Session, name: &str, media_type: &str, bytes: &[u8]) -> AttachmentDto {
    let uploaded = post(backend, session, &format!("?name={name}"), Some(media_type), bytes.to_vec()).await;
    assert_eq!(uploaded.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&uploaded.body));
    uploaded.json::<AttachmentAnswer>().attachment
}

/// A text and the uploads it names under their file names, as the
/// composer sends them.
fn naming(text: &str, uploads: &[(&AttachmentDto, &str)]) -> Vec<ClientContent> {
    let mut content = vec![ClientContent::Text { text: text.into() }];
    for (attachment, file_name) in uploads {
        content.push(ClientContent::Upload {
            r#ref: attachment.id.as_str().to_owned(),
            file_name: (*file_name).to_owned(),
        });
    }
    content
}

fn with_upload(id: &str, text: &str, uploads: &[(&AttachmentDto, &str)]) -> ClientFrame {
    ClientFrame::Send {
        message_id: TurnId::try_from(id).unwrap(),
        content: naming(text, uploads),
    }
}

/// The model's shell call that waits until the file `go` appears where the
/// conversation works.
fn wait_for_go() -> demi_provider::testing::MockResponse {
    let script = "until [ -f go ]; do sleep 0.05; done";
    tool_use("toolu_wait", "shell_exec", &json!({ "description": "Wait", "script": script, "timeoutMs": 60_000 }))
}

#[tokio::test]
async fn an_upload_reaches_the_model_through_the_conversations_host_and_the_page_by_reference() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    harness.add_user("ana@example.test", "ana-pass-1", Role::User);
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    let (paired, root) = on_device(&harness, &backend, &master, FIRST).await;

    // The answer says what the backend read from the bytes: a type it
    // recognizes, and a text file's opening, which the name decides.
    let image = upload(&backend, &master, "shot.png", "application/octet-stream", &PNG).await;
    let sha256 = format!("{:x}", Sha256::digest(PNG));
    assert_eq!(
        (image.media_type.as_str(), image.size_bytes, image.sha256.as_str(), image.snippet.as_deref()),
        ("image/png", 12, sha256.as_str(), None)
    );
    let notes = upload(&backend, &master, "notes.log", "application/octet-stream", b"\r\n  first\r\nsecond").await;
    assert_eq!(
        (notes.media_type.as_str(), notes.snippet.as_deref()),
        ("application/octet-stream", Some("first\nsecond"))
    );
    let ana = backend.login("ana@example.test", "ana-pass-1").await;
    let hers = upload(&backend, &ana, "hers.txt", "text/plain", b"not yours").await;

    // What is no single file's bytes is refused.
    let refusals = [
        ("", Some("image/png"), PNG.to_vec(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidQuery)),
        ("?name=a.png", None, PNG.to_vec(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody)),
        ("?name=a.png", Some("multipart/form-data; boundary=x"), PNG.to_vec(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody)),
        ("?name=a.png", Some("image/png"), Vec::new(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody)),
        ("?name=a.bin", Some("application/octet-stream"), vec![0; ATTACHMENT_MAX_BYTES + 1], (StatusCode::PAYLOAD_TOO_LARGE, ErrorCode::TooLarge)),
    ];
    for (query, media_type, bytes, refusal) in refusals {
        let refused = post(&backend, &master, query, media_type, bytes).await;
        assert_eq!(refused.refusal(), refusal, "{query} {media_type:?}");
    }

    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open(&model_of(&provider, "claude-opus-4-8")).await;
    vendor.respond(answer(&["Seen."], 1, 1));
    socket
        .send(&with_upload("m1", "Look", &[(&image, "shot.png"), (&notes, "notes.log"), (&hers, "hers.txt")]))
        .await;
    let turn = socket.until_idle().await;

    // The files are on the conversation's Host, outside its working
    // directory; another user's upload is not.
    let directory = format!("{}/.demi/attachments/{FIRST}", paired.runner.home());
    assert_eq!(std::fs::read(format!("{directory}/shot.png")).unwrap(), PNG);
    assert_eq!(std::fs::read_to_string(format!("{directory}/notes.log")).unwrap(), "\r\n  first\r\nsecond");
    assert!(!std::path::Path::new(&format!("{directory}/hers.txt")).exists());
    // The model reads the image and each file's record, and learns the other
    // user's upload is not available.
    let sent = vendor.requests()[0].json()["messages"].to_string();
    let base64 = data_encoding::BASE64.encode(&PNG);
    assert!(sent.contains(&base64), "{sent}");
    assert!(sent.contains(&format!("{directory}/shot.png")) && sent.contains(&format!("{directory}/notes.log")), "{sent}");
    assert!(sent.contains(&format!("[attachment {} is not available]", hers.id)), "{sent}");
    // The page receives the image by reference, never its bytes, and reads
    // them from its blobs.
    let frames = serde_json::to_string(&turn).unwrap();
    assert!(!frames.contains(&base64) && frames.contains(&sha256), "{frames}");
    let blocks = transcript(&backend, &master, FIRST).await.blocks;
    let Some(Block::User(user)) = blocks.first() else {
        panic!("{blocks:?}");
    };
    let kinds: Vec<String> = user
        .content
        .iter()
        .map(|part| serde_json::to_value(part).unwrap()["type"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(kinds, ["text", "image", "attachment", "attachment", "text"]);
    assert!(matches!(
        &user.content[1],
        UserContentBlock::Image { source: MediaSource::Ref { r#ref, .. } } if r#ref.as_str() == sha256
    ));
    let served = backend.get(&format!("/api/blobs/{sha256}"), Some(&master)).await;
    assert_eq!((served.status, served.body.as_slice()), (StatusCode::OK, &PNG[..]));

    // The same name again is the next free one; nothing is overwritten.
    vendor.respond(answer(&["Again."], 1, 1));
    socket.send(&with_upload("m2", "Once more", &[(&image, "shot.png")])).await;
    socket.until_idle().await;
    assert_eq!(std::fs::read(format!("{directory}/shot-2.png")).unwrap(), PNG);

    // A steer names an upload as a message does: the file goes to the Host,
    // and the running turn's next request reads its image and its record.
    vendor.respond(wait_for_go());
    vendor.respond(answer(&["Steered."], 1, 1));
    socket.send(&send("m3", "Wait for the file")).await;
    vendor.received(3).await;
    let steer = ClientFrame::Steer {
        steer_id: BlockId::try_from("s1").unwrap(),
        content: naming("And this one", &[(&image, "shot.png")]),
    };
    socket.send(&steer).await;
    let steered = socket.until(|frame| matches!(frame, ServerFrame::SteerResult { .. })).await;
    assert!(
        matches!(steered.last(), Some(ServerFrame::SteerResult { outcome: SteerOutcome::Accepted, .. })),
        "{steered:?}"
    );
    std::fs::write(format!("{root}/go"), "").unwrap();
    // The turn was running before the steer's answer came.
    socket.until(|frame| matches!(frame, ServerFrame::Phase { phase: SessionPhase::Idle })).await;
    assert_eq!(std::fs::read(format!("{directory}/shot-3.png")).unwrap(), PNG);
    let continued = vendor.requests()[3].json()["messages"].to_string();
    assert!(continued.contains("And this one") && continued.contains(&base64), "{continued}");
    assert!(continued.contains(&format!("{directory}/shot-3.png")), "{continued}");
    backend.close().await;
}

#[tokio::test]
async fn a_frame_whose_media_cannot_be_stored_reaches_the_page_as_an_error_and_the_turn_goes_on() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    let (_paired, root) = on_device(&harness, &backend, &master, FIRST).await;
    std::fs::write(format!("{root}/shot.png"), PNG).unwrap();
    // The owner's blob namespace cannot be made, so nothing can be stored
    // there: the object store's directory `blobs/<user>` is a file.
    let blobs = harness.data_dir().join("blobs");
    std::fs::create_dir_all(&blobs).unwrap();
    std::fs::write(blobs.join(master.user.id.as_str()), "").unwrap();
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    let mut model = model_of(&provider, "claude-opus-4-8");
    model.model.accepted_extensions = Some(vec![FileExtension::Png]);
    socket.open(&model).await;

    // The shell's stdout is a picture, which the tool's result carries to a
    // model that reads PNG.
    vendor.respond(tool_use(
        "toolu_1",
        "shell_exec",
        &json!({ "description": "Show it", "script": "cat shot.png", "timeoutMs": 60_000 }),
    ));
    vendor.respond(answer(&["A picture."], 1, 1));
    let turn = socket.chat("m1", "Show me the picture").await;

    // The frame that brought the picture is an error, since no frame carries
    // media bytes; the frames after it arrive, and the model read the
    // picture all the same.
    let failed = turn
        .iter()
        .position(|frame| matches!(frame, ServerFrame::Error { code, .. } if code.as_deref() == Some("frame_send_failed")))
        .unwrap_or_else(|| panic!("{turn:?}"));
    let later = serde_json::to_string(&turn[failed + 1..]).unwrap();
    assert!(later.contains("A picture."), "{later}");
    let base64 = data_encoding::BASE64.encode(&PNG);
    assert!(!serde_json::to_string(&turn).unwrap().contains(&base64));
    let continued = vendor.requests()[1].json()["messages"].to_string();
    assert!(continued.contains(&base64), "{continued}");
    backend.close().await;
}
