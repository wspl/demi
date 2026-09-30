//! Retention (`storage.md` § Retention, `runtime.md` § Retired tool media,
//! `resource-lifecycle.md` § A release that fails): a collection deletes only
//! a blob nothing references once it and its last use are past the grace,
//! and nothing while a reference source cannot be read; a tool result's
//! image goes after 30 days once no request can send it from a vendor's
//! cache, never while a page has its conversation open and never a
//! message's; a command's output is removed 30 days after the command ended;
//! and an archive succeeds though its device goes away in the middle of the
//! release. Times of a day and more pass on the test's clock; the objects are
//! counted at the object store. The devices are real runners, and the model
//! is an Anthropic endpoint the test scripts.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use demi_agent_protocol::{ClientFrame, ServerFrame};
use demi_agent_tools::testing::field;
use demi_backend_objects::counting::ObjectCounts;
use demi_core::{
    Block, GoneCause, ModelMediaKind, Timestamp, ToolMediaSource, ToolResultContentBlock, UserContentBlock,
};
use demi_provider::testing::MockVendor;
use jiff::SignedDuration;
use reqwest::StatusCode;
use serde_json::json;
use sha2::{Digest as _, Sha256};

use crate::conversations::{
    FIRST, SECOND, Socket, anthropic, answer, choose, create, on_device, tool_result, transcript,
};
use crate::streams::{self, received};
use crate::support::{Harness, MASTER_EMAIL, MASTER_PASSWORD, Paired, Session, TestBackend, eventually};
use crate::uploads::{png, upload, with_upload};
use crate::work::{say, shell, switch};

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
/// image's blob, or, where an image was retired, its media type and the UTC
/// day of its retirement.
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
            ToolResultContentBlock::Gone {
                kind: ModelMediaKind::Image,
                media_type,
                cause: GoneCause::Retired { at },
            } => Some(format!("{media_type} retired on {}", utc_day(*at))),
            _ => None,
        })
        .collect()
}

/// The UTC day of `at`.
fn utc_day(at: Timestamp) -> jiff::civil::Date {
    at.to_jiff().to_zoned(jiff::tz::TimeZone::UTC).date()
}

/// A PNG retired on the clock's day, as `result_media` shows it.
fn retired_on(harness: &Harness) -> String {
    format!("image/png retired on {}", utc_day(demi_core::Clock::now(&*harness.clock)))
}

#[tokio::test]
async fn a_backend_runs_its_first_retention_pass_once_it_serves() {
    let counts = ObjectCounts::default();
    let mut harness = Harness::new().with_object_counts(&counts);
    harness.clock.follow_system();
    harness.lifecycle.retention_interval = Some(Duration::from_secs(24 * 60 * 60));
    let (backend, master) = harness.start_set_up().await;
    backend.close().await;
    let left = orphan(&harness, &master, &png(4), DAY + SignedDuration::from_hours(1));

    let backend = harness.start().await;
    eventually("the first pass collects the blob", || async { !left.exists() }).await;
    assert_eq!(counts.tally().deletes, 1);
    backend.close().await;
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
    // A block references a tool's screenshot, and another the copies of a
    // file its command created: the empty file before and the note after.
    std::fs::write(format!("{root}/shot.png"), png(1)).unwrap();
    vendor.respond(shell("toolu_0", "printf 'noted\\n' > note.txt", 60_000));
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
    let kept: Vec<PathBuf> = [png(1), png(2), png(3), Vec::new(), b"noted\n".to_vec()]
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

    // The retirement used the blob; a day later nothing did, and it goes,
    // with the outputs of the two commands, which the same pass removed
    // 30 days after their commands ended.
    harness.clock.advance(DAY + SignedDuration::from_hours(1));
    let before = counts.tally();
    backend.run_retention(&master).await;
    assert_eq!(counts.tally().since(&before).deletes, 3);
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
    assert_eq!(result_media(&backend, &master, "toolu_1").await, [retired_on(&harness)]);

    // The resumed conversation's next request carries the text where the
    // image was.
    socket.open().await;
    vendor.respond(say("Still here."));
    socket.chat("m2", "Anything new?").await;
    let sent = vendor.requests().last().unwrap().json()["messages"].to_string();
    let day = utc_day(demi_core::Clock::now(&*harness.clock));
    let retired = format!(
        "[image:image/png, removed on {day}: a tool result's images and videos are kept for 30 days]"
    );
    assert!(sent.contains(&retired), "{sent}");
    assert!(!sent.contains(&data_encoding::BASE64.encode(&png(1))), "{sent}");
    backend.close().await;
}

// Under a second: a real device installs the fixture package, and is killed
// in the middle of a release.
#[tokio::test]
async fn an_archive_succeeds_though_its_device_goes_away_during_the_release() {
    let harness = Harness::new().with_native_fixture();
    let (backend, master) = harness.start_set_up().await;
    let mut laptop = backend.pair(&master, "laptop").await;
    for id in [FIRST, SECOND] {
        create(&backend, &master, id).await;
        switch(&backend, &master, id, &laptop, laptop.runner.home_dir()).await;
    }
    // The fixture service holds the conversation, and its release of it
    // never ends by itself.
    let (_, code, reason) = received(&mut streams::socket(&backend, &master, FIRST, "stall_release").await).await;
    assert_eq!((code, reason.as_str()), (1000, "completed"));

    // The archive's release reaches the device, which goes away before it
    // answers; the archive succeeds all the same.
    let path = format!("/api/conversations/{FIRST}");
    let archiving = backend.patch(&path, &master, json!({ "archived": true }));
    let going_away = async {
        let (_, code, reason) = received(&mut streams::socket(&backend, &master, SECOND, "stalled").await).await;
        assert_eq!((code, reason.as_str()), (1000, "completed"));
        laptop.runner.kill().await;
    };
    let (archived, ()) = tokio::join!(archiving, going_away);
    assert_eq!(archived.status, StatusCode::OK, "{}", String::from_utf8_lossy(&archived.body));
    backend.close().await;
}

// About two seconds: a real device runs three shell commands.
#[tokio::test]
async fn a_commands_output_is_removed_30_days_after_it_ended() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    harness.clock.follow_system();
    let (backend, master) = harness.start_set_up().await;
    let (mut socket, _root, _device) = open_on_device(&harness, &backend, &master, &vendor).await;
    vendor.respond(shell("toolu_1", "echo kept", 60_000));
    vendor.respond(say("Said."));
    socket.chat("m1", "Say it").await;
    let command = field(&tool_result(&vendor.requests()[1].json(), "toolu_1"), "commandId").to_owned();
    let read = |id: &str| shell(id, &format!("demi shell output {command} --raw"), 60_000);
    vendor.respond(read("toolu_2"));
    vendor.respond(say("Read."));
    socket.chat("m2", "Read it").await;
    let requests = vendor.requests().len();
    assert!(tool_result(&vendor.requests()[requests - 1].json(), "toolu_2").contains("kept"));

    // Thirty-one days later, the retention pass removes it, with the day.
    harness.clock.advance(DAY * 31);
    let master = backend.login(MASTER_EMAIL, MASTER_PASSWORD).await;
    backend.run_retention(&master).await;
    vendor.respond(read("toolu_3"));
    vendor.respond(say("Gone."));
    socket.chat("m3", "Read it again").await;
    let day = utc_day(demi_core::Clock::now(&*harness.clock));
    let gone = tool_result(&vendor.requests().last().unwrap().json(), "toolu_3");
    let removed = format!(
        "demi shell output: the output of {command} was removed on {day}, 30 days after the command ended"
    );
    assert!(gone.contains(&removed), "{gone}");
    backend.close().await;
}
