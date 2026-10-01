//! Uploads (`web-api.md` § Uploads and media, `backend.md` § Media by
//! reference): a file's bytes go to the caller's blobs with its record, the
//! answer says what the backend read from them, and a message or a steer
//! that names the upload writes the file to the conversation's Host, gives
//! the model the file's native medium and record, and shows the page the
//! medium by reference. An edit that keeps the medium names it by that
//! reference, and the model reads its bytes again. A frame whose media
//! cannot be stored reaches the page as an error, and the frames after it
//! still arrive. No test calls a real model.

use std::sync::LazyLock;

use demi_backend_blobs::counting::ObjectCounts;
use demi_conversation_socket_protocol::{
    ClientContent, ClientFrame, EditOutcome, EditRequest, MediaRef, ServerFrame, SteerOutcome,
    TranscriptPatch,
};
use demi_provider_common::testing::MockVendor;
use demi_shared_types::{
    Block, BlockId, GoneCause, MediaSource, ModelMediaKind, SessionPhase, ToolResultContentBlock,
    TurnId, UserContentBlock,
};
use demi_web_api_protocol::attachments::{ATTACHMENT_MAX_BYTES, AttachmentAnswer, AttachmentDto};
use demi_web_api_protocol::auth::Role;
use demi_web_api_protocol::error::ErrorCode;
use reqwest::{Method, StatusCode};
use serde_json::json;
use sha2::{Digest as _, Sha256};

use crate::conversations::{
    FIRST, Socket, answer, anthropic, choose, create, on_device, send, tool_use, transcript,
};
use crate::support::{Answer, Harness, Session, TestBackend, answer as read};

/// A PNG image, whose type the backend reads from its bytes; a real one,
/// since an image is decoded as it enters a transcript.
pub(crate) static PNG: LazyLock<Vec<u8>> =
    LazyLock::new(|| demi_agent_store::testing::png(4, 3, 0).as_bytes().to_vec());

async fn post(
    backend: &TestBackend,
    session: &Session,
    query: &str,
    media_type: Option<&str>,
    bytes: Vec<u8>,
) -> Answer {
    let headers: Vec<(&str, &str)> = media_type
        .into_iter()
        .map(|media_type| ("content-type", media_type))
        .collect();
    let path = format!("/api/attachments{query}");
    read(
        backend
            .response(Method::POST, &path, session, &headers, Some(bytes.into()))
            .await,
    )
    .await
}

/// Uploads `bytes` sent as `media_type` under the name `name`.
pub(crate) async fn upload(
    backend: &TestBackend,
    session: &Session,
    name: &str,
    media_type: &str,
    bytes: &[u8],
) -> AttachmentDto {
    let uploaded = post(
        backend,
        session,
        &format!("?name={name}"),
        Some(media_type),
        bytes.to_vec(),
    )
    .await;
    assert_eq!(
        uploaded.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&uploaded.body)
    );
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

pub(crate) fn with_upload(id: &str, text: &str, uploads: &[(&AttachmentDto, &str)]) -> ClientFrame {
    ClientFrame::Send {
        message_id: TurnId::try_from(id).unwrap(),
        content: naming(text, uploads),
    }
}

#[tokio::test]
async fn a_repeated_upload_sends_the_object_store_no_bytes() {
    let counts = ObjectCounts::default();
    let harness = Harness::new().with_object_counts(&counts);
    let (backend, master) = harness.start_set_up().await;
    let before = counts.tally();
    let first = upload(&backend, &master, "shot.png", "image/png", &PNG).await;
    let stored = counts.tally().since(&before);
    assert_eq!(
        (stored.puts, stored.bytes_put),
        (1, PNG.len() as u64),
        "{stored:?}"
    );

    // The same bytes under another name are the same blob: the object store
    // is asked whether it holds it, and receives none of its bytes again.
    let before = counts.tally();
    let again = upload(&backend, &master, "copy.png", "image/png", &PNG).await;
    let repeated = counts.tally().since(&before);
    assert_eq!(again.sha256, first.sha256);
    assert_eq!(
        (repeated.puts, repeated.bytes_put, repeated.heads),
        (0, 0, 1),
        "{repeated:?}"
    );
    backend.close().await;
}

/// The model's shell call that waits until the file `go` appears where the
/// conversation works.
fn wait_for_go() -> demi_provider_common::testing::MockResponse {
    let script = "until [ -f go ]; do sleep 0.05; done";
    tool_use(
        "toolu_wait",
        "shell_exec",
        &json!({ "description": "Wait", "script": script, "timeoutMs": 60_000 }),
    )
}

// Over a second: the uploads reach a real device over four turns.
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
    let image = upload(
        &backend,
        &master,
        "shot.png",
        "application/octet-stream",
        &PNG,
    )
    .await;
    let sha256 = format!("{:x}", Sha256::digest(&*PNG));
    assert_eq!(
        (
            image.media_type.as_str(),
            image.size_bytes,
            image.sha256.as_str(),
            image.snippet.as_deref()
        ),
        ("image/png", PNG.len() as u64, sha256.as_str(), None)
    );
    let notes = upload(
        &backend,
        &master,
        "notes.log",
        "application/octet-stream",
        b"\r\n  first\r\nsecond",
    )
    .await;
    assert_eq!(
        (notes.media_type.as_str(), notes.snippet.as_deref()),
        ("application/octet-stream", Some("first\nsecond"))
    );
    let ana = backend.login("ana@example.test", "ana-pass-1").await;
    let hers = upload(&backend, &ana, "hers.txt", "text/plain", b"not yours").await;

    // What is no single file's bytes is refused.
    let refusals = [
        (
            "",
            Some("image/png"),
            PNG.to_vec(),
            (StatusCode::BAD_REQUEST, ErrorCode::InvalidQuery),
        ),
        (
            "?name=a.png",
            None,
            PNG.to_vec(),
            (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody),
        ),
        (
            "?name=a.png",
            Some("multipart/form-data; boundary=x"),
            PNG.to_vec(),
            (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody),
        ),
        (
            "?name=a.png",
            Some("image/png"),
            Vec::new(),
            (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody),
        ),
        (
            "?name=a.bin",
            Some("application/octet-stream"),
            vec![0; ATTACHMENT_MAX_BYTES + 1],
            (StatusCode::PAYLOAD_TOO_LARGE, ErrorCode::TooLarge),
        ),
    ];
    for (query, media_type, bytes, refusal) in refusals {
        let refused = post(&backend, &master, query, media_type, bytes).await;
        assert_eq!(refused.refusal(), refusal, "{query} {media_type:?}");
    }

    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    vendor.respond(answer(&["Seen."], 1, 1));
    socket
        .send(&with_upload(
            "m1",
            "Look",
            &[
                (&image, "shot.png"),
                (&notes, "notes.log"),
                (&hers, "hers.txt"),
            ],
        ))
        .await;
    let turn = socket.until_idle().await;

    // The files are on the conversation's Host, outside its working
    // directory; another user's upload is not.
    let directory = format!("{}/.demi/attachments/{FIRST}", paired.runner.home());
    assert_eq!(
        std::fs::read(format!("{directory}/shot.png")).unwrap(),
        *PNG
    );
    assert_eq!(
        std::fs::read_to_string(format!("{directory}/notes.log")).unwrap(),
        "\r\n  first\r\nsecond"
    );
    assert!(!std::path::Path::new(&format!("{directory}/hers.txt")).exists());
    // The model reads the image and each file's record, and learns the other
    // user's upload is not available.
    let sent = vendor.requests()[0].json()["messages"].to_string();
    let base64 = data_encoding::BASE64.encode(&PNG);
    assert!(sent.contains(&base64), "{sent}");
    assert!(
        sent.contains(&format!("{directory}/shot.png"))
            && sent.contains(&format!("{directory}/notes.log")),
        "{sent}"
    );
    assert!(
        sent.contains(&format!("[attachment {} is not available]", hers.id)),
        "{sent}"
    );
    // The page receives the image by reference, never its bytes, and reads
    // them from its blobs.
    let frames = serde_json::to_string(&turn).unwrap();
    assert!(
        !frames.contains(&base64) && frames.contains(&sha256),
        "{frames}"
    );
    let blocks = transcript(&backend, &master, FIRST).await.blocks;
    let Some(Block::User(user)) = blocks.first() else {
        panic!("{blocks:?}");
    };
    let kinds: Vec<String> = user
        .content
        .iter()
        .map(|part| {
            serde_json::to_value(part).unwrap()["type"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(kinds, ["text", "image", "attachment", "attachment", "text"]);
    assert!(matches!(
        &user.content[1],
        UserContentBlock::Image { source: MediaSource::Ref { r#ref, .. } } if r#ref.as_str() == sha256
    ));
    let served = backend
        .get(&format!("/api/blobs/{sha256}"), Some(&master))
        .await;
    assert_eq!(
        (served.status, served.body.as_slice()),
        (StatusCode::OK, &PNG[..])
    );

    // The same name again is the next free one; nothing is overwritten.
    vendor.respond(answer(&["Again."], 1, 1));
    socket
        .send(&with_upload("m2", "Once more", &[(&image, "shot.png")]))
        .await;
    socket.until_idle().await;
    assert_eq!(
        std::fs::read(format!("{directory}/shot-2.png")).unwrap(),
        *PNG
    );

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
    let steered = socket
        .until(|frame| matches!(frame, ServerFrame::SteerResult { .. }))
        .await;
    assert!(
        matches!(
            steered.last(),
            Some(ServerFrame::SteerResult {
                outcome: SteerOutcome::Accepted,
                ..
            })
        ),
        "{steered:?}"
    );
    std::fs::write(format!("{root}/go"), "").unwrap();
    // The turn was running before the steer's answer came.
    socket
        .until(|frame| {
            matches!(
                frame,
                ServerFrame::Phase {
                    phase: SessionPhase::Idle
                }
            )
        })
        .await;
    assert_eq!(
        std::fs::read(format!("{directory}/shot-3.png")).unwrap(),
        *PNG
    );
    let continued = vendor.requests()[3].json()["messages"].to_string();
    assert!(
        continued.contains("And this one") && continued.contains(&base64),
        "{continued}"
    );
    assert!(
        continued.contains(&format!("{directory}/shot-3.png")),
        "{continued}"
    );

    // An edit of the first message keeps its image by the reference the
    // page shows, and the model reads the image's bytes again.
    let UserContentBlock::Image {
        source: MediaSource::Ref { r#ref, media_type },
    } = &user.content[1]
    else {
        unreachable!()
    };
    socket.send(&ClientFrame::SyncTranscript {}).await;
    let synced = socket
        .until(|frame| matches!(frame, ServerFrame::TranscriptReset { .. }))
        .await;
    let Some(ServerFrame::TranscriptReset { version, .. }) = synced.into_iter().last() else {
        unreachable!()
    };
    let keep = EditRequest {
        operation_id: "keep-1".try_into().unwrap(),
        target_block_id: user.id.clone(),
        version,
        content: vec![
            ClientContent::Text {
                text: "Look again".into(),
            },
            ClientContent::Media {
                media: MediaRef::Image {
                    r#ref: r#ref.clone(),
                    media_type: media_type.clone(),
                },
            },
        ],
    };
    vendor.respond(answer(&["The same shot."], 1, 1));
    socket
        .send(&ClientFrame::EditAndSend { request: keep })
        .await;
    let edited = socket.until_idle().await;
    assert!(
        edited.iter().any(|frame| matches!(
            frame,
            ServerFrame::EditResult {
                outcome: EditOutcome::Accepted { .. },
                ..
            }
        )),
        "{edited:?}"
    );
    let requests = vendor.requests();
    assert_eq!(
        requests.len(),
        5,
        "the replacement asks the model: {edited:?}"
    );
    let kept = requests[4].json()["messages"].to_string();
    assert!(
        kept.contains("Look again") && kept.contains(&base64),
        "{kept}"
    );
    backend.close().await;
}

#[tokio::test]
async fn a_tool_medium_that_cannot_be_stored_is_gone_from_its_result_and_the_turn_goes_on() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    let (_paired, root) = on_device(&harness, &backend, &master, FIRST).await;
    std::fs::write(format!("{root}/shot.png"), &*PNG).unwrap();
    // The owner's blob namespace cannot be made, so nothing can be stored
    // there: the object store's directory `blobs/<user>` is a file.
    let blobs = harness.data_dir().join("blobs");
    std::fs::create_dir_all(&blobs).unwrap();
    std::fs::write(blobs.join(master.user.id.as_str()), "").unwrap();
    // The directory's model reads PNG.
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    // The shell's stdout is a picture, which the tool's result would carry
    // to a model that reads PNG.
    vendor.respond(tool_use(
        "toolu_1",
        "shell_exec",
        &json!({ "description": "Show it", "script": "cat shot.png", "timeoutMs": 60_000 }),
    ));
    vendor.respond(answer(&["No picture."], 1, 1));
    let turn = socket.chat("m1", "Show me the picture").await;

    // The picture was not stored, so in its place the page receives a part
    // that says it is gone and why, the model reads that as its text, the
    // turn goes on, and nothing carries the picture's bytes.
    let base64 = data_encoding::BASE64.encode(&PNG);
    let gone: Vec<(ModelMediaKind, String, GoneCause)> = turn
        .iter()
        .filter_map(|frame| match frame {
            ServerFrame::TranscriptPatch { patches, .. } => Some(patches),
            _ => None,
        })
        .flatten()
        .filter_map(|patch| match patch {
            TranscriptPatch::Add {
                value: Block::ToolCall(call),
                ..
            }
            | TranscriptPatch::ReplaceBlock {
                value: Block::ToolCall(call),
                ..
            } => Some(&call.output),
            _ => None,
        })
        .flatten()
        .filter_map(|part| match part {
            ToolResultContentBlock::Gone {
                kind,
                media_type,
                cause,
            } => Some((*kind, media_type.clone(), cause.clone())),
            _ => None,
        })
        .collect();
    let [(ModelMediaKind::Image, media_type, GoneCause::NotStored { error })] = gone.as_slice()
    else {
        panic!("{gone:?}");
    };
    assert_eq!(media_type, "image/png");
    let frames = serde_json::to_string(&turn).unwrap();
    assert!(
        frames.contains("No picture.") && !frames.contains(&base64),
        "{frames}"
    );
    let continued = vendor.requests()[1].json()["messages"].to_string();
    let text = serde_json::to_string(&format!("[image not stored: {error}]")).unwrap();
    assert!(
        continued.contains(&text) && !continued.contains(&base64),
        "{continued}"
    );
    backend.close().await;
}

/// The width and height a PNG's header states.
fn png_size(bytes: &[u8]) -> (u32, u32) {
    let side = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
    (side(16), side(20))
}

/// The base64 of the first image a Messages API request carries, in a
/// message or a tool result.
fn first_image(request: &serde_json::Value) -> String {
    let text = request["messages"].to_string();
    let start = text
        .find("\"data\":\"")
        .expect("the request carries an image")
        + 8;
    let end = start + text[start..].find('"').unwrap();
    text[start..end].to_owned()
}

// About a second: an upload and a shell's output reach a real device.
#[tokio::test]
async fn an_image_over_2000_px_enters_fitted_from_an_upload_and_a_tool_and_stays_whole_on_the_host()
{
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    let (paired, root) = on_device(&harness, &backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let wide = demi_agent_store::testing::png(2_400, 10, 1)
        .as_bytes()
        .to_vec();
    std::fs::write(format!("{root}/wide.png"), &wide).unwrap();
    let image = upload(&backend, &master, "wide.png", "image/png", &wide).await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    vendor.respond(tool_use(
        "toolu_1",
        "shell_exec",
        &json!({ "description": "Show it", "script": "cat wide.png", "timeoutMs": 60_000 }),
    ));
    vendor.respond(answer(&["Both are wide."], 1, 1));
    socket
        .send(&with_upload("m1", "Look", &[(&image, "wide.png")]))
        .await;
    socket.until_idle().await;

    // The upload's file on the Host is the original, and so is the shell's
    // output; the model reads the image fitted to 2,000 px, the same bytes
    // in every request and from either source.
    let directory = format!("{}/.demi/attachments/{FIRST}", paired.runner.home());
    assert_eq!(
        std::fs::read(format!("{directory}/wide.png")).unwrap(),
        wide
    );
    let requests: Vec<serde_json::Value> = vendor
        .requests()
        .iter()
        .map(|request| request.json())
        .collect();
    let fitted = first_image(&requests[0]);
    let bytes = data_encoding::BASE64.decode(fitted.as_bytes()).unwrap();
    assert_eq!(png_size(&bytes), (2_000, 8));
    assert!(
        !requests[0]["messages"]
            .to_string()
            .contains(&data_encoding::BASE64.encode(&wide))
    );
    assert_eq!(first_image(&requests[1]), fitted);
    let result = requests[1]["messages"].to_string();
    assert_eq!(
        result.matches(&fitted).count(),
        2,
        "the upload's and the tool's: {result}"
    );
    assert!(
        result.contains(
            "fitted to what every model accepts; to keep the original, save it: demi shell output "
        ),
        "{result}"
    );
    // The message's image is the fitted one's blob, which the page reads.
    let blocks = transcript(&backend, &master, FIRST).await.blocks;
    let Some(Block::User(user)) = blocks.first() else {
        panic!("{blocks:?}");
    };
    let UserContentBlock::Image {
        source: MediaSource::Ref { r#ref, .. },
    } = &user.content[1]
    else {
        panic!("{:?}", user.content);
    };
    assert_ne!(r#ref.as_str(), image.sha256.as_str());
    let served = backend
        .get(&format!("/api/blobs/{}", r#ref.as_str()), Some(&master))
        .await;
    assert_eq!((served.status, served.body), (StatusCode::OK, bytes));
    backend.close().await;
}

/// A PNG image that differs from the others by its pixels.
pub(crate) fn png(seed: u8) -> Vec<u8> {
    demi_agent_store::testing::png(4, 3, seed)
        .as_bytes()
        .to_vec()
}

/// The frames of a compaction pass, to the idle phase that ends it.
async fn compact(socket: &mut Socket) -> Vec<ServerFrame> {
    socket.send(&ClientFrame::Compact {}).await;
    socket.until_idle().await
}

#[tokio::test]
async fn opening_a_stored_conversation_with_images_on_two_pages_and_syncing_it_puts_no_blob() {
    let counts = ObjectCounts::default();
    let vendor = MockVendor::start().await;
    let harness = Harness::new().with_object_counts(&counts);
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    // The uploads are written to the conversation's Host.
    let _device = on_device(&harness, &backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let shots = [
        upload(&backend, &master, "a.png", "image/png", &png(1)).await,
        upload(&backend, &master, "b.png", "image/png", &png(2)).await,
    ];
    let mut first = Socket::connect(&backend, &master, FIRST).await;
    first.open().await;
    vendor.respond(answer(&["Two shots."], 1, 1));
    first
        .send(&with_upload(
            "m1",
            "Look",
            &[(&shots[0], "a.png"), (&shots[1], "b.png")],
        ))
        .await;
    first.until_idle().await;
    // Closing the tree leaves the conversation stored; the next open
    // restores it.
    first.send(&ClientFrame::Close {}).await;
    first.until(|frame| *frame == ServerFrame::Closed).await;

    let before = counts.tally();
    first.open().await;
    let mut second = Socket::connect(&backend, &master, FIRST).await;
    second.open().await;
    let synced = second.live().await;
    // Nothing is stored again, and the object store is not even asked
    // whether it holds the images.
    let after = counts.tally().since(&before);
    assert_eq!(
        (after.puts, after.bytes_put, after.heads),
        (0, 0, 0),
        "{after:?}"
    );
    // Every frame named the images by the references the rows hold.
    let Some(Block::User(user)) = synced.first() else {
        panic!("{synced:?}");
    };
    let images = user
        .content
        .iter()
        .filter(|part| {
            matches!(
                part,
                UserContentBlock::Image {
                    source: MediaSource::Ref { .. }
                }
            )
        })
        .count();
    assert_eq!(images, 2, "{user:?}");
    backend.close().await;
}

#[tokio::test]
async fn a_restored_conversation_reads_each_replayed_blob_once_and_none_before_its_last_compaction()
{
    let counts = ObjectCounts::default();
    let vendor = MockVendor::start().await;
    let harness = Harness::new().with_object_counts(&counts);
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    let _device = on_device(&harness, &backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let old = upload(&backend, &master, "old.png", "image/png", &png(0)).await;
    let mut shots = Vec::new();
    for shot in 1..=9 {
        shots.push(upload(&backend, &master, "shot.png", "image/png", &png(shot)).await);
    }
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    vendor.respond(answer(&["An old shot."], 1, 1));
    socket
        .send(&with_upload("m1", "Look", &[(&old, "old.png")]))
        .await;
    socket.until_idle().await;
    // The vendor refuses the nine shots' request, so the latest answered
    // request is the first: the pass summarizes what it carried, the old
    // shot, and keeps what came after it, the nine shots
    // (`compaction.md` § One pass).
    let refusal = json!({ "type": "error", "error": { "type": "invalid_request_error", "message": "try later" } });
    vendor.respond(
        demi_provider_common::testing::MockResponse::status(400).chunk(refusal.to_string()),
    );
    let named: Vec<(&AttachmentDto, &str)> = shots.iter().map(|shot| (shot, "shot.png")).collect();
    socket
        .send(&with_upload("m2", "Look at these", &named))
        .await;
    socket.until_idle().await;
    vendor.respond(answer(&["The user showed an old shot."], 1, 1));
    compact(&mut socket).await;
    socket.send(&ClientFrame::Close {}).await;
    socket.until(|frame| *frame == ServerFrame::Closed).await;

    // The restored conversation's turn asks the model twice: an unknown
    // tool's error goes back to it.
    let before = counts.tally();
    socket.open().await;
    assert_eq!(
        counts.tally().since(&before).gets,
        0,
        "an open reads no blob"
    );
    vendor.respond(tool_use("toolu_1", "no_such_tool", &json!({})));
    vendor.respond(answer(&["Still nine."], 1, 1));
    let requests = vendor.requests().len();
    socket.send(&send("m3", "And now?")).await;
    vendor.received(requests + 1).await;
    let first = counts.tally().since(&before);
    socket.until_idle().await;
    let both = counts.tally().since(&before);

    // The first request read each replayed shot once, a few at a time, and
    // the old shot not at all; the second read nothing.
    assert_eq!((first.gets, both.gets), (9, 9), "{both:?}");
    assert!(
        1 < both.most_gets_at_once && both.most_gets_at_once <= 8,
        "{both:?}"
    );
    for request in &vendor.requests()[requests..] {
        let sent = request.json()["messages"].to_string();
        for shot in 1..=9 {
            assert!(
                sent.contains(&data_encoding::BASE64.encode(&png(shot))),
                "{sent}"
            );
        }
        assert!(
            !sent.contains(&data_encoding::BASE64.encode(&png(0))),
            "{sent}"
        );
    }
    backend.close().await;
}
