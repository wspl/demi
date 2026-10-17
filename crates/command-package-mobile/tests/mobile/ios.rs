//! The iOS driver against a helper the test plays (`mobile.md` § iOS): the
//! connection `demi-ios-sim serve` speaks, its end with the conversation's
//! last device, its captures' pictures, and a recording too large for one
//! frame of it.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use bytes::{Buf, BufMut, BytesMut};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, DuplexStream},
    sync::watch,
};

use demi_command_package_mobile_protocol::live::{LiveFrame, LiveModuleMessage};
use demi_mobile::{
    driver::Result,
    ios::{BLOB, CONTROL, Connection, Launch, VIDEO},
};

use crate::{CHAT, Mobile, live::Page};

/// The SPS VideoToolbox writes for an iPhone 17's 1206×2622 screen, H.264
/// High at level 5.1 with frame reordering off: no VUI.
const VIDEOTOOLBOX_SPS: [u8; 12] = [0x27, 0x64, 0x00, 0x33, 0xac, 0x56, 0x80, 0x4c, 0x01, 0x49, 0xe6, 0xa4];
/// That SPS with the VUI that tells a decoder the stream has no
/// reordering, which ffmpeg's `trace_headers` reads as
/// `bitstream_restriction_flag` 1, `max_num_reorder_frames` 0 and
/// `max_dec_frame_buffering` 1, and with which the encoder's IDR decodes.
const NO_REORDERING_SPS: [u8; 17] = [
    0x27, 0x64, 0x00, 0x33, 0xac, 0x56, 0x80, 0x4c, 0x01, 0x49, 0xe6, 0xa8, 0x06, 0xd0, 0x44, 0x23, 0x50,
];
const PPS: [u8; 4] = [0x28, 0xee, 0x3c, 0xb0];

/// What the test's helper heard and did.
#[derive(Default)]
pub struct Heard {
    /// Each request, its method and its parameters, in order.
    pub requests: Vec<(String, Value)>,
}

/// A helper the test plays: two booted iPhones, each of whose captures
/// sends one key frame naming its simulator, and a recording of
/// `recording` bytes.
pub struct FakeHelper {
    pub heard: Arc<Mutex<Heard>>,
    /// Counts the connections that ended, which the driver ends by closing
    /// its side.
    pub ended: watch::Sender<usize>,
    recording: usize,
}

impl FakeHelper {
    pub fn new(recording: usize) -> Arc<Self> {
        Arc::new(Self {
            heard: Arc::default(),
            ended: watch::Sender::new(0),
            recording,
        })
    }

    fn requests(&self, method: &str) -> Vec<Value> {
        let heard = self.heard.lock().unwrap();
        heard
            .requests
            .iter()
            .filter(|(each, _)| each == method)
            .map(|(_, params)| params.clone())
            .collect()
    }
}

impl Launch for FakeHelper {
    fn unavailable(&self) -> Option<String> {
        None
    }

    fn once<'a>(&'a self, args: &'a [&'a str]) -> BoxFuture<'a, Result<Vec<u8>>> {
        Box::pin(async move {
            Ok(match args {
                ["list"] => serde_json::to_vec(&json!([
                    { "udid": "UDID-A", "name": "iPhone 17", "model": "iPhone 17", "os": "iOS 26.5", "state": "Booted" },
                    { "udid": "UDID-B", "name": "iPhone Air", "model": "iPhone Air", "os": "iOS 26.5", "state": "Booted" },
                ]))
                .unwrap(),
                _ => b"{}".to_vec(),
            })
        })
    }

    fn serve(&self) -> BoxFuture<'_, Result<Connection>> {
        Box::pin(async move {
            let (driver_input, helper_input) = tokio::io::duplex(64 * 1024);
            let (helper_output, driver_output) = tokio::io::duplex(64 * 1024);
            tokio::spawn(serve(
                helper_input,
                helper_output,
                self.heard.clone(),
                self.ended.clone(),
                self.recording,
            ));
            Ok(Connection {
                input: Box::new(driver_input),
                output: Box::new(driver_output),
                process: None,
            })
        })
    }

    fn boots(&self) -> HashMap<String, u64> {
        HashMap::from([("UDID-A".into(), 1), ("UDID-B".into(), 1)])
    }
}

fn frame(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut frame = BytesMut::with_capacity(5 + payload.len());
    frame.put_u32(u32::try_from(1 + payload.len()).unwrap());
    frame.put_u8(kind);
    frame.put_slice(payload);
    frame.to_vec()
}

/// A key frame as the encoder makes it, its picture's slice holding the
/// simulator's id so the test tells whose it is.
fn key_frame(capture: u32, udid: &str) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&capture.to_be_bytes());
    payload.extend_from_slice(&1u32.to_be_bytes());
    payload.extend_from_slice(&[1, 0, 0, 0]);
    payload.extend_from_slice(&1000.0f64.to_be_bytes());
    payload.extend_from_slice(&1206u16.to_be_bytes());
    payload.extend_from_slice(&2622u16.to_be_bytes());
    for unit in [&VIDEOTOOLBOX_SPS[..], &PPS[..], &[&[0x25u8][..], udid.as_bytes()].concat()[..]] {
        payload.extend_from_slice(&[0, 0, 0, 1]);
        payload.extend_from_slice(unit);
    }
    frame(VIDEO, &payload)
}

/// The helper's side: answers each request, until the driver closes its
/// input, which ends the helper as it does `demi-ios-sim serve`.
async fn serve(
    mut input: DuplexStream,
    mut output: DuplexStream,
    heard: Arc<Mutex<Heard>>,
    ended: watch::Sender<usize>,
    recording: usize,
) {
    let mut buffer = BytesMut::new();
    'reading: loop {
        while buffer.len() >= 4 {
            let length = u32::from_be_bytes(buffer[..4].try_into().unwrap()) as usize;
            if buffer.len() < 4 + length {
                break;
            }
            buffer.advance(4);
            let mut payload = buffer.split_to(length);
            assert_eq!(payload.get_u8(), CONTROL, "the driver sends requests only");
            let request: Value = serde_json::from_slice(&payload).unwrap();
            let id = request["id"].clone();
            let method = request["method"].as_str().unwrap().to_owned();
            let params = request["params"].clone();
            heard.lock().unwrap().requests.push((method.clone(), params.clone()));
            let udid = params["udid"].as_str().unwrap_or_default().to_owned();
            let result = match method.as_str() {
                "attach" => json!({ "points": { "width": 402, "height": 874 }, "pixels": { "width": 1206, "height": 2622 } }),
                "recordStart" => {
                    let path = params["path"].as_str().unwrap().to_owned();
                    let mut file = tokio::fs::File::create(&path).await.unwrap();
                    let chunk = vec![7u8; 1024 * 1024];
                    let mut left = recording;
                    while left > 0 {
                        let part = left.min(chunk.len());
                        file.write_all(&chunk[..part]).await.unwrap();
                        left -= part;
                    }
                    json!({})
                }
                "recordStop" => json!({ "durationMs": 2000 }),
                // UDID-A's model has a frame installed, UDID-B's has none.
                "frame" if udid == "UDID-A" => {
                    let mut blob = u32::try_from(id.as_u64().unwrap()).unwrap().to_be_bytes().to_vec();
                    blob.extend_from_slice(b"PNG");
                    if output.write_all(&frame(BLOB, &blob)).await.is_err() {
                        break 'reading;
                    }
                    json!({
                        "width": 400, "height": 800,
                        "screen": { "x": 20.0, "y": 30.0, "width": 360.0, "height": 740.0 },
                        "cornerRadius": 40.0,
                    })
                }
                "frame" => json!({ "cornerRadius": 55.0 }),
                _ => json!({}),
            };
            let reply = serde_json::to_vec(&json!({ "id": id, "result": result })).unwrap();
            if output.write_all(&frame(CONTROL, &reply)).await.is_err() {
                break 'reading;
            }
            if method == "captureStart" {
                let capture = u32::try_from(params["capture"].as_u64().unwrap()).unwrap();
                if output.write_all(&key_frame(capture, &udid)).await.is_err() {
                    break 'reading;
                }
            }
        }
        match input.read_buf(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
    }
    ended.send_modify(|count| *count += 1);
}

/// Closing the conversation's last device ends its helper: the driver
/// closes the helper's input, which the helper takes as its end, as
/// `demi-ios-sim serve` does. Planted defect this catches: the task that
/// reads the helper holding the helper, so it never ends. One fake
/// helper, milliseconds.
#[tokio::test]
async fn the_helper_ends_when_the_conversations_last_device_closes() {
    let helper = FakeHelper::new(0);
    let mobile = Mobile::start_ios(helper.clone()).await;
    let first = mobile.open(CHAT, "UDID-A").await;
    let second = mobile.open(CHAT, "UDID-B").await;
    let mut ended = helper.ended.subscribe();
    mobile.run(CHAT, "close", json!({ "device": first })).await.json();
    assert_eq!(*ended.borrow_and_update(), 0, "a device is still open");
    mobile.run(CHAT, "close", json!({ "device": second })).await.json();
    tokio::time::timeout(std::time::Duration::from_secs(10), ended.wait_for(|count| *count == 1))
        .await
        .expect("the helper ends with the last device")
        .unwrap();
}

/// Two devices of one conversation, watched at once, each show their own
/// pictures: the helper they share tells captures apart by number alone,
/// so the numbers are the helper's, not each device's; and the encoder's
/// key frame reaches the page with an SPS that says the stream has no
/// reordering, so a still screen's only picture paints. Planted defects
/// this catches: captures numbered per device, whose second takes the
/// first one's pictures, and a key frame passed as the encoder made it.
/// One fake helper, two views, milliseconds.
#[tokio::test]
async fn devices_sharing_a_helper_each_show_their_own_pictures() {
    let helper = FakeHelper::new(0);
    let mobile = Mobile::start_ios(helper.clone()).await;
    let first = mobile.open(CHAT, "UDID-A").await;
    let second = mobile.open(CHAT, "UDID-B").await;
    let mut pages = Vec::new();
    for device in [&first, &second] {
        let mut page = Page::open(&mobile, CHAT).await;
        page.send(json!({ "type": "hello", "platform": "mac" })).await;
        page.send(json!({ "type": "watch", "device": device })).await;
        pages.push(page);
    }
    for (page, udid) in pages.iter_mut().zip(["UDID-A", "UDID-B"]) {
        let data = tokio::time::timeout(std::time::Duration::from_secs(10), page.video())
            .await
            .expect("the device's first picture arrives");
        assert!(data.ends_with(udid.as_bytes()), "{udid}'s view shows another device's picture");
        let sps = &data[4..4 + NO_REORDERING_SPS.len()];
        assert_eq!(sps, NO_REORDERING_SPS, "the key frame's SPS says the stream has no reordering");
        assert_eq!(&data[4 + NO_REORDERING_SPS.len()..][..8], [&[0, 0, 0, 1][..], &PPS[..]].concat(), "the PPS passes as it was");
    }
    let captures: Vec<u64> = helper
        .requests("captureStart")
        .iter()
        .map(|params| params["capture"].as_u64().unwrap())
        .collect();
    assert_eq!(captures.len(), 2);
    assert_ne!(captures[0], captures[1], "the helper's captures share a number");
}

/// A recording larger than any frame of the helper's connection, 70 MiB,
/// is saved whole and streamed whole, and its file in the conversation's
/// scratch directory goes once it was taken; the device answers on after
/// it. Planted defect this catches: a recording left behind in the scratch
/// directory. One fake helper, 140 MiB of files, a second or two.
#[tokio::test]
async fn a_recording_larger_than_a_frame_is_saved_whole() {
    const SIZE: usize = 70 * 1024 * 1024;
    let helper = FakeHelper::new(SIZE);
    let mobile = Mobile::start_ios(helper.clone()).await;
    let device = mobile.open(CHAT, "UDID-A").await;

    mobile.run(CHAT, "record", json!({ "device": device, "action": "start" })).await.json();
    let file = std::path::PathBuf::from(helper.requests("recordStart")[0]["path"].as_str().unwrap());
    assert!(file.starts_with(mobile.home().join("scratch").join(CHAT)), "{} is outside the conversation's scratch", file.display());
    let saved = mobile
        .run(CHAT, "record", json!({ "device": device, "action": "stop", "output": "clip.mp4" }))
        .await
        .json();
    assert_eq!(std::fs::metadata(mobile.root.path().join("clip.mp4")).unwrap().len(), SIZE as u64);
    assert_eq!(saved["durationMs"], 2000);
    assert!(!file.exists(), "the recording's file stays in the scratch directory");

    mobile.run(CHAT, "record", json!({ "device": device, "action": "start" })).await.json();
    let mut args = json!({ "device": device, "action": "stop" });
    args.as_object_mut().unwrap().remove("output");
    let streamed = mobile.run(CHAT, "record", args).await;
    assert_eq!(streamed.exit_code, 0, "{}", streamed.stderr);
    assert_eq!(streamed.stdout.len(), SIZE);
    let second = std::path::PathBuf::from(helper.requests("recordStart")[1]["path"].as_str().unwrap());
    assert!(!second.exists(), "the streamed recording's file stays in the scratch directory");

    mobile.run(CHAT, "key", json!({ "device": device, "key": "home" })).await.json();
}

/// The view gets the frame the helper draws for a model whose frame Xcode
/// installed, with the screen's place and corners, and the outline with
/// the screen's corners for one without. Planted defect this catches: the
/// helper's camelCase answer read with snake_case fields, which loses
/// every frame and gives the outline square corners. One fake helper, one
/// view, milliseconds.
#[tokio::test]
async fn the_view_gets_the_frame_the_helper_draws_or_the_outline() {
    let helper = FakeHelper::new(0);
    let mobile = Mobile::start_ios(helper.clone()).await;
    let framed = mobile.open(CHAT, "UDID-A").await;
    let bare = mobile.open(CHAT, "UDID-B").await;
    let mut page = Page::open(&mobile, CHAT).await;
    page.send(json!({ "type": "hello", "platform": "mac" })).await;
    let mut frames = Vec::new();
    for device in [&framed, &bare] {
        page.send(json!({ "type": "watch", "device": device })).await;
        page.send(json!({ "type": "frame", "device": device, "width": 800, "height": 1200 })).await;
        let LiveModuleMessage::Frame { frame, .. } =
            tokio::time::timeout(std::time::Duration::from_secs(10), page.until("frame")).await.expect("the frame arrives")
        else {
            unreachable!()
        };
        frames.push(frame);
    }
    match &frames[0] {
        LiveFrame::Image { png, width, height, screen, corner_radius } => {
            assert_eq!((png.as_str(), *width, *height), ("UE5H", 400, 800));
            assert_eq!((screen.x, screen.y, screen.width, screen.height), (20.0, 30.0, 360.0, 740.0));
            assert_eq!(*corner_radius, 40.0);
        }
        other => panic!("the installed frame came as {other:?}"),
    }
    match &frames[1] {
        LiveFrame::Outline { corner_radius, buttons } => {
            assert_eq!(*corner_radius, 55.0);
            assert!(!buttons.is_empty(), "the outline marks the iPhone's buttons");
        }
        other => panic!("the outline came as {other:?}"),
    }
}
