//! `demi mobile record` through the service as the runner calls it
//! (`crates/plugin-mobile/command/src/operations.rs`): a device's pictures
//! become a recording whose frames and images the commands read, and the
//! recording's limits and ends.

use std::time::Duration;

use serde_json::{Value, json};

use demi_command_protocol::CommandCaller;
use demi_mobile::testing::Drawn;

use crate::{CHAT, Mobile};

const WIDTH: u32 = 96;
const HEIGHT: u32 = 64;

/// A grey screen with a red square at `at`, if any.
fn screen(at: Option<(u32, u32)>, millis: u64) -> Drawn {
    let mut pixels = vec![90u8; (WIDTH * HEIGHT * 4) as usize];
    for pixel in pixels.chunks_exact_mut(4) {
        pixel[3] = 255;
    }
    if let Some((x, y)) = at {
        for row in y..y + 8 {
            for column in x..x + 8 {
                let at = ((row * WIDTH + column) * 4) as usize;
                pixels[at..at + 4].copy_from_slice(&[230, 20, 20, 255]);
            }
        }
    }
    Drawn {
        width: WIDTH,
        height: HEIGHT,
        pixels,
        at_us: millis * 1000,
    }
}

async fn start(mobile: &Mobile, device: &str) -> String {
    let started = mobile.run(CHAT, "record", json!({ "target": device, "action": "start" })).await.json();
    assert_eq!(started["recording"]["running"], true);
    started["recording"]["id"].as_str().unwrap().to_owned()
}

/// Only the frames that differ are listed, a change of one frame is among
/// them and exported with a label that names its number and time, and a
/// recording stopped says why and how many frames changed.
/// Planted defects this catches: a recording that keeps a frame equal to the
/// one before, one that loses the frame before the stop, and an export
/// without its label strip.
/// Cost: one fake device, an encoder at 96×64; milliseconds.
#[tokio::test]
async fn a_recording_lists_the_frames_that_differ_and_exports_them() {
    let mobile = Mobile::start().await;
    let device = mobile.open(CHAT, "UDID-1").await;
    let id = start(&mobile, &device).await;
    assert_eq!(id, "r1");
    let fake = mobile.driver.device("UDID-1");
    // The second frame is the first drawn again, which a source does not code.
    for (index, (frame, kept)) in [
        (screen(None, 0), true),
        (screen(None, 16), false),
        (screen(Some((40, 20)), 32), true),
        (screen(None, 48), true),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(fake.draws(frame).await, kept, "frame {index}");
    }
    let stopped = mobile.run(CHAT, "record", json!({ "target": id, "action": "stop" })).await.json();
    assert_eq!(stopped["recording"]["running"], false);
    assert_eq!(stopped["recording"]["ended"], "stopped");
    assert_eq!(stopped["recording"]["frames"], 3);

    let listed = mobile.run(CHAT, "record", json!({ "target": id, "action": "frames" })).await.json();
    let times: Vec<u64> = listed["frames"].as_array().unwrap().iter().map(|row| row["timeUs"].as_u64().unwrap()).collect();
    assert_eq!(times, [0, 32_000, 48_000]);
    assert_eq!(listed["frames"][1]["changed"], json!({ "x": 40, "y": 20, "width": 8, "height": 8 }));

    let text = mobile.run_text(CHAT, "record", json!({ "target": id, "action": "frames" })).await;
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(text.contains("f002") && text.contains("0.032"), "{text}");

    let exported = mobile
        .run(CHAT, "record", json!({ "target": id, "action": "export", "frames": "f002" }))
        .await
        .json();
    // Without --json the output is the paths and nothing else, which xargs
    // hands to `demi file view`.
    let text = mobile.run_text(CHAT, "record", json!({ "target": id, "action": "export", "frames": "f002" })).await;
    let text = String::from_utf8(text.stdout).unwrap();
    assert_eq!(text.lines().count(), 1, "{text}");
    assert!(std::path::Path::new(text.trim()).is_file(), "{text}");
    let image = exported["images"][0]["path"].as_str().unwrap();
    assert!(image.ends_with("recording-r1/f002.png"), "{image}");
    let png = image::open(image).unwrap().to_rgba8();
    // The label's strip sits above the 8x8 area that changed.
    assert!(png.width() >= 8);
    assert!(png.height() > 8, "the label has a strip of its own above the picture");
}

/// A recording with a duration ends by itself and says so; one longer than
/// two minutes is refused.
/// Cost: a 300 ms recording; under a second.
#[tokio::test]
async fn a_duration_ends_the_recording_and_two_minutes_is_the_most() {
    let mobile = Mobile::start().await;
    let device = mobile.open(CHAT, "UDID-1").await;
    let refused = mobile
        .run(CHAT, "record", json!({ "target": device, "action": "start", "duration": "121s" }))
        .await;
    assert_eq!(refused.failure().0, "invalid_input");

    let started = mobile
        .run(CHAT, "record", json!({ "target": device, "action": "start", "duration": "300ms" }))
        .await
        .json();
    let id = started["recording"]["id"].as_str().unwrap().to_owned();
    mobile.driver.device("UDID-1").draws(screen(None, 0)).await;
    tokio::time::sleep(Duration::from_millis(700)).await;
    let stopped = mobile.run(CHAT, "record", json!({ "target": id, "action": "stop" })).await.json();
    assert_eq!(stopped["recording"]["ended"], "duration");
    assert_eq!(stopped["recording"]["frames"], 1);
    // The device is free to record again.
    start(&mobile, &device).await;
}

/// The device records once at a time, a conversation keeps its five newest
/// recordings, and a recording that does not exist is told apart.
/// Cost: six recordings without frames; milliseconds.
#[tokio::test]
async fn a_conversation_keeps_five_recordings() {
    let mobile = Mobile::start().await;
    let device = mobile.open(CHAT, "UDID-1").await;
    let first = start(&mobile, &device).await;
    let twice = mobile.run(CHAT, "record", json!({ "target": device, "action": "start" })).await;
    assert_eq!(twice.failure().0, "recording");
    for _ in 0..5 {
        mobile.run(CHAT, "record", json!({ "target": device, "action": "stop" })).await.json();
        start(&mobile, &device).await;
    }
    let gone = mobile.run(CHAT, "record", json!({ "target": first, "action": "frames" })).await;
    assert_eq!(gone.failure().0, "recording_not_found");
    let kept = mobile.run(CHAT, "record", json!({ "target": "r6", "action": "stop" })).await.json();
    assert_eq!(kept["recording"]["id"], "r6");
    let missing = mobile.run(CHAT, "record", json!({ "target": "r99", "action": "frames" })).await;
    assert_eq!(missing.failure().0, "recording_not_found");
}

/// Closing the device ends its recording, which says it closed and is
/// complete when the close is answered: the service may end then. Releasing
/// the conversation keeps the recordings; they go when the conversation is
/// archived or deleted, which the runner sees to
/// (`crates/runner-jobs/src/scratch.rs`).
///
/// Planted defects this catches: a close that answers while the recording's
/// thread still writes its last frames and its end, and a release that
/// removes the recordings of a conversation that is only idle.
/// Cost: two fake devices' recordings; milliseconds.
#[tokio::test]
async fn closing_the_device_ends_the_recording_complete_and_releasing_keeps_it() {
    let mobile = Mobile::start().await;
    let device = mobile.open(CHAT, "UDID-1").await;
    let id = start(&mobile, &device).await;
    mobile.driver.device("UDID-1").draws(screen(None, 0)).await;
    mobile.driver.device("UDID-1").draws(screen(Some((10, 10)), 40)).await;
    mobile.run(CHAT, "close", json!({ "device": device })).await.json();
    // Read from the files, with no stop to wait for its end.
    let directory = mobile.home().join("scratch").join("chata").join(".recordings");
    let meta: Value = serde_json::from_slice(&std::fs::read(directory.join("r1").join("recording.json")).unwrap()).unwrap();
    assert_eq!(meta["ended"]["reason"], "closed", "{meta}");
    let index = std::fs::read_to_string(directory.join("r1").join("frames.jsonl")).unwrap();
    assert_eq!(index.lines().count(), 2, "both frames are kept");
    let stopped = mobile.run(CHAT, "record", json!({ "target": id, "action": "stop" })).await.json();
    assert_eq!(stopped["recording"]["ended"], "closed");
    assert_eq!(stopped["recording"]["frames"], 2);

    mobile.release(CHAT).await;
    assert!(directory.join("r1").is_dir(), "an idle conversation keeps its recordings");
}

/// The panel's Stop saves the same video a command's `export --mp4` gives,
/// and the panel's device view says the device records while it does.
/// Cost: one recording, one MP4 of two frames; milliseconds.
#[tokio::test]
async fn the_panels_record_saves_the_recordings_video() {
    let mobile = Mobile::start().await;
    let device = mobile.open(CHAT, "UDID-1").await;
    let user = CommandCaller::User {};
    let started = mobile
        .run_by(CHAT, "record", json!({ "target": device, "action": "start" }), user.clone())
        .await
        .json();
    assert_eq!(started["recording"]["running"], true);
    let fake = mobile.driver.device("UDID-1");
    fake.draws(screen(None, 0)).await;
    fake.draws(screen(Some((10, 10)), 40)).await;
    let stopped: Value = mobile
        .run_by(CHAT, "record", json!({ "target": device, "action": "stop" }), user)
        .await
        .json();
    let path = stopped["path"].as_str().expect("the panel's stop saves the video");
    let video = std::fs::read(path).unwrap();
    assert_eq!(&video[4..8], b"ftyp");
    let by_command = mobile
        .run(CHAT, "record", json!({ "target": "r1", "action": "export", "mp4": true }))
        .await;
    assert_eq!(by_command.exit_code, 0, "{}", by_command.stderr);
    assert_eq!(&by_command.stdout[4..8], b"ftyp");
    assert_eq!(by_command.stdout.len(), video.len());
}

/// A recording the panel started that ends by itself, at its limit, still
/// gives the panel its video at the panel's next Stop-call, which a
/// recording that ended some other way, with nothing started by the panel,
/// does not.
/// Cost: one recording ended by a fake limit; milliseconds.
#[tokio::test]
async fn a_recording_the_panel_started_gives_it_the_video_when_it_ended_by_itself() {
    use demi_shared_recording_protocol::EndReason;
    let mobile = Mobile::start().await;
    let device = mobile.open(CHAT, "UDID-1").await;
    let user = CommandCaller::User {};
    mobile
        .run_by(CHAT, "record", json!({ "target": device, "action": "start" }), user.clone())
        .await
        .json();
    let fake = mobile.driver.device("UDID-1");
    fake.draws(screen(None, 0)).await;
    fake.draws(screen(Some((10, 10)), 40)).await;
    fake.recording_ends(EndReason::SizeLimit);
    // Asking for the recording's end waits for it, and tells how it ended.
    let ended = mobile.run(CHAT, "record", json!({ "target": "r1", "action": "stop" })).await.json();
    assert_eq!(ended["recording"]["ended"], "size-limit");
    let saved: Value = mobile
        .run_by(CHAT, "record", json!({ "target": device, "action": "stop" }), user.clone())
        .await
        .json();
    assert_eq!(saved["recording"]["ended"], "size-limit");
    let video = std::fs::read(saved["path"].as_str().expect("the video is saved for the panel")).unwrap();
    assert_eq!(&video[4..8], b"ftyp");
    // It was handed over once.
    let again = mobile.run_by(CHAT, "record", json!({ "target": device, "action": "stop" }), user).await;
    assert_eq!(again.failure().0, "recording_not_found");
    // The agent's stop of a device that does not record is refused as it was.
    let agent = mobile.run(CHAT, "record", json!({ "target": device, "action": "stop" })).await;
    assert_eq!(agent.failure().0, "recording_not_found");
}
