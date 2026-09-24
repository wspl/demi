//! Uploads (`web-api.md` § Uploads and media, `backend.md` § Media by
//! reference): a file's bytes go to the caller's blobs with its record, the
//! answer says what the backend read from them, and a message that names the
//! upload writes the file to the conversation's Host, gives the model the
//! file's native medium and record, and shows the page the medium by
//! reference. No test calls a real model.

use demi_agent::testing::model_of;
use demi_agent_protocol::{ClientContent, ClientFrame};
use demi_core::{Block, MediaSource, TurnId, UserContentBlock};
use demi_provider::testing::MockVendor;
use demi_web_api::attachments::{ATTACHMENT_MAX_BYTES, AttachmentAnswer, AttachmentDto};
use demi_web_api::auth::Role;
use demi_web_api::error::ErrorCode;
use reqwest::{Method, StatusCode};
use sha2::{Digest as _, Sha256};

use crate::conversations::{FIRST, Socket, anthropic, answer, create, on_device, settled, transcript};
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

fn with_upload(id: &str, text: &str, uploads: &[(&AttachmentDto, &str)]) -> ClientFrame {
    let mut content = vec![ClientContent::Text { text: text.into() }];
    for (attachment, file_name) in uploads {
        content.push(ClientContent::Upload {
            r#ref: attachment.id.as_str().to_owned(),
            file_name: (*file_name).to_owned(),
        });
    }
    ClientFrame::Send {
        message_id: TurnId::try_from(id).unwrap(),
        content,
    }
}

#[tokio::test]
async fn an_upload_reaches_the_model_through_the_conversations_host_and_the_page_by_reference() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    harness.add_user("ana@example.test", "ana-pass-1", Role::User);
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    let (paired, _root) = on_device(&harness, &backend, &master, FIRST).await;

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
    settled(&backend, &master, FIRST).await;
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
    backend.close().await;
}
