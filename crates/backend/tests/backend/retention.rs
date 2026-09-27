//! Retention (`storage.md` § Retention, `runtime.md` § Retired tool media,
//! `resource-lifecycle.md` § A release the device missed): a collection
//! deletes only a blob nothing references once it and its last use are past
//! the grace, and nothing while a reference source cannot be read; a tool
//! result's image goes after 30 days once no request can send it from a
//! vendor's cache, never while a page has its conversation open and never a
//! message's; and a device that missed a release hears it when its runner
//! connects again. Times of a day and more pass on the test's clock; the
//! objects are counted at the object store. The devices are real runners,
//! and the model is an Anthropic endpoint the test scripts.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use demi_agent_protocol::{ClientFrame, ServerFrame};
use demi_backend::ObjectCounts;
use demi_core::{Block, ToolMediaSource, ToolResultContentBlock, UserContentBlock};
use demi_provider::testing::MockVendor;
use jiff::SignedDuration;
use reqwest::StatusCode;
use serde_json::json;
use sha2::{Digest as _, Sha256};

use crate::conversations::{FIRST, SECOND, Socket, anthropic, answer, choose, create, on_device, transcript};
use crate::support::{Harness, MASTER_EMAIL, MASTER_PASSWORD, Paired, Session, TestBackend, eventually};
use crate::uploads::{png, upload, with_upload};
use crate::work::{say, shell};

const DAY: SignedDuration = SignedDuration::from_hours(24);

/// The object `bytes` are stored as in `session`'s blob namespace.
fn blob_path(harness: &Harness, session: &Session, bytes: &[u8]) -> PathBuf {
    harness
        .data_dir()
        .join("blobs")
        .join(session.user.id.as_str())
        .join(hex::encode(Sha256::digest(bytes)))
}

/// Stores `bytes` in `session`'s namespace with its object written `age`
/// before the backend's clock, as a put whose save failed leaves it.
fn orphan(harness: &Harness, session: &Session, bytes: &[u8], age: SignedDuration) -> PathBuf {
    let path = blob_path(harness, session, bytes);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, bytes).unwrap();
    let now = demi_core::Clock::now(&*harness.clock).as_millisecond();
    let written = now - i64::try_from(age.as_millis()).unwrap();
    let written = SystemTime::UNIX_EPOCH + Duration::from_millis(u64::try_from(written).unwrap());
    std::fs::File::options().write(true).open(&path).unwrap().set_modified(written).unwrap();
    path
}

/// A conversation on a paired device whose model reads PNG, open on a page;
/// the device's runner runs while the answer is held.
async fn open_on_device(
    harness: &Harness,
    backend: &TestBackend,
    master: &Session,
    vendor: &MockVendor,
) -> (Socket, String, Paired) {
    let provider = anthropic(backend, master, vendor).await;
    create(backend, master, FIRST).await;
    let (paired, root) = on_device(harness, backend, master, FIRST).await;
    choose(backend, master, FIRST, &provider, "claude-opus-4-8").await;
    let mut socket = Socket::connect(backend, master, FIRST).await;
    socket.open().await;
    (socket, root, paired)
}

/// Closes the page's tree, which disposes it.
async fn close(socket: &mut Socket) {
    socket.send(&ClientFrame::Close {}).await;
    socket.until(|frame| *frame == ServerFrame::Closed).await;
}

/// The media of the result of the tool call `id` in the stored history: each
/// image's blob, or the text in its place.
async fn result_media(backend: &TestBackend, master: &Session, id: &str) -> Vec<String> {
    let blocks = transcript(backend, master, FIRST).await.blocks;
    let call = blocks
        .iter()
        .find_map(|block| match block {
            Block::ToolCall(call) if call.tool_use_id == id => Some(call.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no call {id}: {blocks:?}"));
    call.output
        .iter()
        .filter_map(|part| match part {
            ToolResultContentBlock::Image {
                source: ToolMediaSource::Ref { r#ref, .. },
            } => Some(r#ref.to_string()),
            ToolResultContentBlock::Text { text } if text.starts_with("[image:") => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// The line that takes a retired PNG's place, retired on the clock's day.
fn retired_on(harness: &Harness) -> String {
    let day = jiff::Timestamp::from_millisecond(demi_core::Clock::now(&*harness.clock).as_millisecond())
        .unwrap()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date();
    format!("[image:image/png, removed on {day}: a tool result's images and videos are kept for 30 days]")
}

// About two seconds: a real device runs the tool's shell command.
#[tokio::test]
async fn a_collection_deletes_an_unreferenced_blob_past_the_grace_and_nothing_while_a_database_cannot_be_read() {
    let counts = ObjectCounts::default();
    let vendor = MockVendor::start().await;
    let harness = Harness::new().with_object_counts(&counts);
    harness.clock.follow_system();
    let (backend, master) = harness.start_set_up().await;
    let (mut socket, root, _device) = open_on_device(&harness, &backend, &master, &vendor).await;
    // A block references a tool's screenshot.
    std::fs::write(format!("{root}/shot.png"), png(1)).unwrap();
    vendor.respond(shell("toolu_1", "cat shot.png", 60_000));
    vendor.respond(say("Seen."));
    socket.chat("m1", "Show me the shot").await;
    // An upload no message names, and one a draft of another conversation
    // stages.
    upload(&backend, &master, "kept.png", "image/png", &png(2)).await;
    let staged = upload(&backend, &master, "staged.png", "image/png", &png(3)).await;
    create(&backend, &master, SECOND).await;
    let files = json!([{ "type": "upload", "ref": staged.id, "fileName": "staged.png" }]);
    let body = json!({ "base": 0, "text": "\u{FFFC}", "files": files });
    let saved = backend.put(&format!("/api/conversations/{SECOND}/draft"), &master, body).await;
    assert_eq!(saved.status, StatusCode::OK, "{}", String::from_utf8_lossy(&saved.body));
    // What a failed save left a day ago, and what one left an hour ago.
    harness.clock.advance(DAY + SignedDuration::from_hours(1));
    let left = orphan(&harness, &master, &png(4), DAY + SignedDuration::from_hours(1));
    let recent = orphan(&harness, &master, &png(5), SignedDuration::from_hours(1));
    let kept: Vec<PathBuf> = [png(1), png(2), png(3)]
        .iter()
        .map(|bytes| blob_path(&harness, &master, bytes))
        .chain([recent])
        .collect();

    // The other conversation's database cannot be read, so the collection
    // deletes nothing.
    let database = harness.data_dir().join("conversations").join(format!("{SECOND}.sqlite"));
    std::fs::write(&database, "not a database").unwrap();
    let before = counts.tally();
    backend.run_retention(&master).await;
    assert_eq!(counts.tally().since(&before).deletes, 0);
    assert!(left.exists());

    // Once every source can be read, only the blob that nothing references
    // and that is past the grace goes.
    std::fs::remove_file(&database).unwrap();
    let before = counts.tally();
    backend.run_retention(&master).await;
    let collected = counts.tally().since(&before);
    assert_eq!((collected.lists, collected.deletes), (1, 1), "{collected:?}");
    assert!(!left.exists());
    for path in &kept {
        assert!(path.exists(), "{}", path.display());
    }
    backend.close().await;
}

// About three seconds: a real device runs two shell commands, and the
// conversation compacts.
#[tokio::test]
async fn a_tool_image_summarized_a_day_before_goes_after_30_days_once_its_page_lets_go_and_its_blob_a_day_later() {
    let counts = ObjectCounts::default();
    let vendor = MockVendor::start().await;
    let harness = Harness::new().with_object_counts(&counts);
    harness.clock.follow_system();
    let (backend, master) = harness.start_set_up().await;
    let (mut socket, root, _device) = open_on_device(&harness, &backend, &master, &vendor).await;
    std::fs::write(format!("{root}/shot.png"), png(1)).unwrap();
    std::fs::write(format!("{root}/later.png"), png(2)).unwrap();
    let pasted = upload(&backend, &master, "pasted.png", "image/png", &png(3)).await;
    vendor.respond(shell("toolu_1", "cat shot.png", 60_000));
    vendor.respond(say("Seen."));
    socket.send(&with_upload("m1", "Look", &[(&pasted, "pasted.png")])).await;
    socket.until_idle().await;
    // A long message fills the history compaction keeps, so the pass
    // summarizes the first turn: its images lie before the boundary.
    vendor.respond(say("Read."));
    socket.chat("m2", &"a long note ".repeat(2_000)).await;
    vendor.respond(answer(&["The user showed two images."], 1, 1));
    socket.send(&ClientFrame::Compact {}).await;
    socket.until_idle().await;
    vendor.respond(shell("toolu_2", "cat later.png", 60_000));
    vendor.respond(say("Seen again."));
    socket.chat("m3", "And the later one").await;
    let shot = hex::encode(Sha256::digest(png(1)));
    let later = hex::encode(Sha256::digest(png(2)));

    // Thirty-one days later, a page still has the conversation open: its
    // tree holds the history, so nothing is retired. The web session of a
    // month ago has expired.
    harness.clock.advance(DAY * 31);
    let master = backend.login(MASTER_EMAIL, MASTER_PASSWORD).await;
    backend.run_retention(&master).await;
    assert_eq!(result_media(&backend, &master, "toolu_1").await, [shot.clone()]);

    // Once the page lets go, the image before the day-old boundary goes. The
    // one in the replayed window stays, since the conversation was in use,
    // and so does the message's.
    close(&mut socket).await;
    let retired = retired_on(&harness);
    eventually("the summarized image is retired", || async {
        result_media(&backend, &master, "toolu_1").await == [retired.clone()]
    })
    .await;
    assert_eq!(result_media(&backend, &master, "toolu_2").await, [later.clone()]);
    let blocks = transcript(&backend, &master, FIRST).await.blocks;
    let Some(Block::User(message)) = blocks.first() else {
        panic!("{blocks:?}");
    };
    assert!(
        message.content.iter().any(|part| matches!(part, UserContentBlock::Image { .. })),
        "{message:?}"
    );

    // The retirement used the blob; a day later nothing did, and it goes.
    harness.clock.advance(DAY + SignedDuration::from_hours(1));
    let before = counts.tally();
    backend.run_retention(&master).await;
    assert_eq!(counts.tally().since(&before).deletes, 1);
    assert!(!blob_path(&harness, &master, &png(1)).exists());
    for kept in [png(2), png(3)] {
        assert!(blob_path(&harness, &master, &kept).exists());
    }
    backend.close().await;
}

// About two seconds: a real device runs the tool's shell command.
#[tokio::test]
async fn a_conversation_idle_for_30_days_loses_its_tool_images_and_its_next_request_carries_their_text() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    harness.clock.follow_system();
    let (backend, master) = harness.start_set_up().await;
    let (mut socket, root, _device) = open_on_device(&harness, &backend, &master, &vendor).await;
    std::fs::write(format!("{root}/shot.png"), png(1)).unwrap();
    vendor.respond(shell("toolu_1", "cat shot.png", 60_000));
    vendor.respond(say("Seen."));
    socket.chat("m1", "Show me the shot").await;
    close(&mut socket).await;

    harness.clock.advance(DAY * 31);
    let master = backend.login(MASTER_EMAIL, MASTER_PASSWORD).await;
    backend.run_retention(&master).await;
    let retired = retired_on(&harness);
    assert_eq!(result_media(&backend, &master, "toolu_1").await, [retired.clone()]);

    // The resumed conversation's next request carries the text where the
    // image was.
    socket.open().await;
    vendor.respond(say("Still here."));
    socket.chat("m2", "Anything new?").await;
    let sent = vendor.requests().last().unwrap().json()["messages"].to_string();
    assert!(sent.contains(&retired), "{sent}");
    assert!(!sent.contains(&data_encoding::BASE64.encode(&png(1))), "{sent}");
    backend.close().await;
}

/// The job directories a paired device's runner holds for `conversation`.
fn job_output(state: &Path, conversation: &str) -> PathBuf {
    state.join("jobs").join(conversation.to_ascii_lowercase())
}

// About two seconds: a real device runs a shell command and restarts.
#[tokio::test]
async fn a_device_offline_at_a_release_hears_it_when_its_runner_connects_again() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    let (mut paired, _root) = on_device(&harness, &backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    vendor.respond(shell("toolu_1", "printf done", 60_000));
    vendor.respond(say("Done."));
    socket.chat("m1", "Do it").await;
    let state = paired.runner.state_dir().to_owned();
    assert!(job_output(&state, FIRST).exists());
    // A leftover of a conversation the device's owner does not have.
    let stranger = "9f8e7d6c-5b4a-4c3d-8e2f-1a0b9c8d7e6f";
    std::fs::create_dir_all(job_output(&state, stranger).join("job-left")).unwrap();

    // The device is offline when the conversation is archived, so it hears
    // no release then.
    close(&mut socket).await;
    paired.runner.stop().await;
    eventually("the device is offline", || async { !backend.online(&master, paired.id()).await }).await;
    let archived = backend.patch(&format!("/api/conversations/{FIRST}"), &master, json!({ "archived": true })).await;
    assert_eq!(archived.status, StatusCode::OK, "{}", String::from_utf8_lossy(&archived.body));
    assert!(job_output(&state, FIRST).exists());

    // Its hello names both, and each is released at once.
    paired.runner.start_again();
    eventually("the released job output is gone", || async {
        !job_output(&state, FIRST).exists() && !job_output(&state, stranger).exists()
    })
    .await;
    backend.close().await;
}
