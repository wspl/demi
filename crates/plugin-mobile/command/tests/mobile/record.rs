//! `demi mobile record` through the service as the runner calls it
//! (`crates/plugin-mobile/command/src/operations.rs`): a device's pictures
//! become a recording whose frames and images the commands read, and the
//! recording's limits and ends.

use serde_json::{Value, json};

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

