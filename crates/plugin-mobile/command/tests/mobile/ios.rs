//! The iOS driver against a helper the test plays
//! (`crates/plugin-mobile/command/src/ios/mod.rs`): the connection
//! `demi-ios-sim serve` speaks, its end with the conversation's last device,
//! its captures' pictures, and a recording too large for one frame of it.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use bytes::{Buf, BufMut, BytesMut};
use futures_util::future::BoxFuture;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, DuplexStream},
    sync::{mpsc, watch},
};

use demi_command_package_mobile_protocol::live::{LiveCorners, LiveFrame, LiveModuleMessage};
use demi_mobile::{
    driver::Result,
    ios::{BLOB, CONTROL, Connection, Launch, VIDEO},
};

use crate::{CHAT, Mobile, live::{Page, screen}};

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
    /// The number of requests, which a test waits on.
    pub count: watch::Sender<usize>,
}

/// A helper the test plays: two booted iPhones, each of whose captures
/// sends one key frame naming its simulator, and a recording of
/// `recording` bytes.
pub struct FakeHelper {
    pub heard: Arc<Mutex<Heard>>,
    /// Counts the connections that ended, which the driver ends by closing
    /// its side.
    pub ended: watch::Sender<usize>,
    /// While true, the helper answers no side button press, nor anything
    /// after it, as a busy simulator does.
    pub held: watch::Sender<bool>,
    /// The orientations the app in front refuses, as the helper names them.
    refused: Arc<Mutex<Vec<String>>>,
    /// Where the helper's unasked events go, once it serves.
    events: Mutex<Option<mpsc::UnboundedSender<Value>>>,
    recording: usize,
}

impl FakeHelper {
    pub fn new(recording: usize) -> Arc<Self> {
        Arc::new(Self {
            heard: Arc::default(),
            ended: watch::Sender::new(0),
            held: watch::Sender::new(false),
            refused: Arc::default(),
            events: Mutex::default(),
            recording,
        })
    }

    /// The app in front refuses `orientation`, as the helper names it: a
    /// turn to it leaves the screen as it is.
    fn refuses(&self, orientation: &str) {
        self.refused.lock().unwrap().push(orientation.to_owned());
    }

    /// The display of `udid` turned to `orientation` by itself, as when an
    /// app that shows only across opens: the helper says so unasked.
    fn turns(&self, udid: &str, orientation: &str) {
        let events = self.events.lock().unwrap();
        let events = events.as_ref().expect("the helper serves");
        events.send(json!({ "event": "turned", "udid": udid, "screen": screen_json(orientation) })).unwrap();
    }

    /// Waits until the helper heard `count` requests of `method`.
    async fn heard(&self, method: &str, count: usize) {
        let mut heard = self.heard.lock().unwrap().count.subscribe();
        loop {
            if self.requests(method).len() >= count {
                return;
            }
            heard.changed().await.unwrap();
        }
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
            let (events, unasked) = mpsc::unbounded_channel();
            *self.events.lock().unwrap() = Some(events);
            tokio::spawn(serve(
                helper_input,
                helper_output,
                self.heard.clone(),
                self.ended.clone(),
                self.held.subscribe(),
                Helped { recording: self.recording, refused: self.refused.clone(), unasked },
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

/// The screen of an iPhone 17 turned to `orientation`, as the helper
/// answers it.
fn screen_json(orientation: &str) -> Value {
    let across = orientation.starts_with("landscape");
    let (points, pixels) = if across { ((874, 402), (2622, 1206)) } else { ((402, 874), (1206, 2622)) };
    json!({
        "orientation": orientation,
        "points": { "width": points.0, "height": points.1 },
        "pixels": { "width": pixels.0, "height": pixels.1 },
    })
}

/// What the helper does besides answering: its recording's size, the
/// orientations the app refuses and the events it sends unasked.
struct Helped {
    recording: usize,
    refused: Arc<Mutex<Vec<String>>>,
    unasked: mpsc::UnboundedReceiver<Value>,
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
    mut held: watch::Receiver<bool>,
    mut helped: Helped,
) {
    let recording = helped.recording;
    let mut buffer = BytesMut::new();
    // Each device's orientation, portrait once attached.
    let mut orientations: HashMap<String, String> = HashMap::new();
    // The field's text, and whether it has the focus, which a tap gives it.
    let mut field = String::new();
    let mut focused = false;
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
            {
                let mut heard = heard.lock().unwrap();
                heard.requests.push((method.clone(), params.clone()));
                heard.count.send_modify(|count| *count += 1);
            }
            if method == "button" {
                // The test's hold ends before the helper does.
                held.wait_for(|held| !*held).await.unwrap();
            }
            let udid = params["udid"].as_str().unwrap_or_default().to_owned();
            let result = match method.as_str() {
                "attach" => {
                    orientations.insert(udid.clone(), "portrait".into());
                    screen_json("portrait")
                }
                "orient" => {
                    let asked = params["orientation"].as_str().unwrap().to_owned();
                    if !helped.refused.lock().unwrap().contains(&asked) {
                        orientations.insert(udid.clone(), asked);
                    }
                    screen_json(&orientations[&udid])
                }
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
                    blob.extend_from_slice(b"PNGSIL");
                    if output.write_all(&frame(BLOB, &blob)).await.is_err() {
                        break 'reading;
                    }
                    json!({
                        "for": screen_json(&orientations[&udid]),
                        "width": 400, "height": 800,
                        "screen": { "x": 20.0, "y": 30.0, "width": 360.0, "height": 740.0 },
                        "pngBytes": 3,
                    })
                }
                "frame" => json!({ "for": screen_json(&orientations[&udid]), "corner": [] }),
                "tap" => {
                    focused = true;
                    json!({})
                }
                "setValue" => {
                    field = params["text"].as_str().unwrap_or_default().to_owned();
                    json!({})
                }
                "tree" => json!({ "elements": [{
                    "role": "text field", "value": field, "focused": focused,
                    "frame": { "x": 20.0, "y": 100.0, "width": 360.0, "height": 40.0 },
                }] }),
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
        tokio::select! {
            read = input.read_buf(&mut buffer) => match read {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            },
            Some(event) = helped.unasked.recv() => {
                if let Some(turned) = event["screen"]["orientation"].as_str() {
                    orientations.insert(event["udid"].as_str().unwrap().to_owned(), turned.to_owned());
                }
                if output.write_all(&frame(CONTROL, &serde_json::to_vec(&event).unwrap())).await.is_err() {
                    break;
                }
            }
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

/// Two devices of one conversation, each on its own stream and looked at
/// together, each show their own pictures, and one that is no longer looked
/// at leaves the other streaming
/// (`docs/decisions/0017-each-device-streams-on-its-own.md`).
///
/// - The helper they share tells captures apart by number alone, so the numbers
///   are the helper's, not each device's.
/// - The encoder's key frame reaches the page with an SPS that says the stream
///   has no reordering, so a still screen's only picture paints.
///
/// Planted defects this catches:
///
/// - captures numbered per device, whose second takes the first one's pictures;
/// - a key frame passed as the encoder made it;
/// - a stream no longer looked at that keeps its device's capture, or stops
///   or restarts the other device's.
///
/// Cost: one fake helper, two views, milliseconds.
#[tokio::test]
async fn devices_sharing_a_helper_each_show_their_own_pictures_at_once() {
    let helper = FakeHelper::new(0);
    let mobile = Mobile::start_ios(helper.clone()).await;
    let first = mobile.open(CHAT, "UDID-A").await;
    let second = mobile.open(CHAT, "UDID-B").await;
    let mut pages = Vec::new();
    for device in [&first, &second] {
        let mut page = Page::open(&mobile, CHAT).await;
        page.send(json!({ "type": "hello", "platform": "mac", "device": device })).await;
        page.send(json!({ "type": "looked", "looked": true })).await;
        page.send(json!({ "type": "shown", "width": 1206, "height": 2622 })).await;
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

    // The first device is no longer looked at: its capture stops, and the
    // second's goes on; looked at again, the first shows its own pictures.
    pages[0].send(json!({ "type": "looked", "looked": false })).await;
    tokio::time::timeout(std::time::Duration::from_secs(10), helper.heard("captureStop", 1))
        .await
        .expect("the first device's capture stops");
    pages[0].send(json!({ "type": "looked", "looked": true })).await;
    pages[0].send(json!({ "type": "shown", "width": 1206, "height": 2622 })).await;
    let data = tokio::time::timeout(std::time::Duration::from_secs(10), pages[0].video())
        .await
        .expect("the first device's picture arrives again");
    assert!(data.ends_with(b"UDID-A"), "the first device's view shows another device's picture");
    let started = |udid: &str| helper.requests("captureStart").iter().filter(|params| params["udid"] == udid).count();
    assert_eq!((started("UDID-A"), started("UDID-B")), (2, 1), "the second device's capture was restarted");
    assert_eq!(helper.requests("captureStop").len(), 1);
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
    assert!(file.starts_with(mobile.home().join("scratch").join("chata")), "{} is outside the conversation's scratch", file.display());
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
/// installed, at the page's pixel ratio, with the screen's place and corners.
/// For a model without one, it gets the drawn frame with the screen's corners.
///
/// Planted defects this catches:
///
/// - the helper's camelCase answer read with snake_case fields, which loses
///   every frame and gives the drawn frame square corners;
/// - the pixel ratio not passed to the helper.
///
/// Cost: one fake helper, one view, milliseconds.
#[tokio::test]
async fn the_view_gets_the_frame_the_helper_draws_or_draws_its_own() {
    let helper = FakeHelper::new(0);
    let mobile = Mobile::start_ios(helper.clone()).await;
    let framed = mobile.open(CHAT, "UDID-A").await;
    let bare = mobile.open(CHAT, "UDID-B").await;
    let mut frames = Vec::new();
    for device in [&framed, &bare] {
        let mut page = Page::open(&mobile, CHAT).await;
        page.send(json!({ "type": "hello", "platform": "mac", "device": device })).await;
        page.send(json!({ "type": "looked", "looked": true })).await;
        page.send(json!({ "type": "frame", "width": 800, "height": 1200, "scale": 2.0 })).await;
        let LiveModuleMessage::Frame { frame, .. } =
            tokio::time::timeout(std::time::Duration::from_secs(10), page.until("frame")).await.expect("the frame arrives")
        else {
            unreachable!()
        };
        frames.push(frame.expect("Xcode's frame is known at once"));
    }
    // The helper draws at the page's pixel ratio.
    assert_eq!(helper.requests("frame")[0], json!({ "udid": "UDID-A", "width": 800, "height": 1200, "scale": 2.0 }));
    match &frames[0] {
        LiveFrame::Image { png, width, height, screen, silhouette } => {
            assert_eq!((png.as_str(), *width, *height), ("UE5H", 400, 800));
            assert_eq!((screen.x, screen.y, screen.width, screen.height), (20.0, 30.0, 360.0, 740.0));
            // The blob's rest after the frame's PNG: the device's silhouette.
            assert_eq!(silhouette.as_deref(), Some("U0lM"));
        }
        other => panic!("the installed frame came as {other:?}"),
    }
    match &frames[1] {
        LiveFrame::Drawn { corners, buttons } => {
            // A mask that leaves its corners square.
            assert_eq!(*corners, LiveCorners::round(0.0));
            assert!(!buttons.is_empty(), "the drawn frame shows the iPhone's buttons");
        }
        other => panic!("the drawn frame came as {other:?}"),
    }
}

/// The viewer's typing reaches an iPhone's focused field as the characters it
/// is, in any layout and from any input method.
///
/// A German keyboard's ü and a Chinese input method's 東京 are added to the end
/// of the field's value through accessibility, as a simulator takes no text
/// into its pasteboard. What a US keyboard types is typed.
///
/// Planted defects this catches:
///
/// - the key's US character typed instead of ü;
/// - the composed text refused;
/// - the field's value replaced rather than added to.
///
/// Cost: one fake helper, milliseconds.
#[tokio::test]
async fn the_viewers_typing_in_any_layout_or_input_method_reaches_an_iphones_field() {
    let helper = FakeHelper::new(0);
    let mobile = Mobile::start_ios(helper.clone()).await;
    let device = mobile.open(CHAT, "UDID-A").await;
    mobile.run(CHAT, "tap", json!({ "device": device, "at": ["200", "120"] })).await.json();
    let mut page = Page::open(&mobile, CHAT).await;
    page.send(json!({ "type": "hello", "platform": "mac", "device": device })).await;
    page.send(json!({ "type": "looked", "looked": true })).await;
    let key = |action: &str, key: &str, code: &str| {
        let mut input = json!({ "type": "key", "action": action, "key": key, "code": code, "keyCode": 0, "modifiers": 0, "repeat": false, "location": 0, "altGraph": false });
        if action == "down" {
            input["text"] = json!(key);
        }
        json!({ "type": "input", "input": input })
    };
    page.send(key("down", "ü", "BracketLeft")).await;
    page.send(key("up", "ü", "BracketLeft")).await;
    page.send(key("down", "z", "KeyY")).await;
    page.send(key("up", "z", "KeyY")).await;
    page.send(json!({ "type": "input", "input": { "type": "text", "text": "東京" } })).await;
    let mut asked = helper.heard.lock().unwrap().count.subscribe();
    let entered = async {
        loop {
            let set = helper.requests("setValue");
            if set.len() == 2 {
                return set;
            }
            asked.changed().await.unwrap();
        }
    };
    let set = tokio::time::timeout(std::time::Duration::from_secs(10), entered).await.expect("the text is added");
    let texts: Vec<&str> = set.iter().map(|params| params["text"].as_str().unwrap()).collect();
    assert_eq!(texts, ["ü", "ü東京"]);
    // At the focused field's centre.
    assert_eq!((set[0]["x"].as_f64(), set[0]["y"].as_f64()), (Some(200.0), Some(120.0)));
    // z, on the key a US keyboard has y on, is typed as z.
    assert_eq!(helper.requests("type"), [json!({ "udid": "UDID-A", "keys": [[0x1d, false]] })]);
    assert!(helper.requests("key").is_empty(), "a key's release went to the simulator: {:?}", helper.requests("key"));
}

/// The viewer's text that finds no focused field on an iPhone is told to
/// the view as that reason, quoting the viewer's own text, so the panel
/// says why in its own words rather than that the input did not arrive.
/// Planted defect this catches: the refusal sent as the generic
/// input_failed. One fake helper, milliseconds.
#[tokio::test]
async fn the_view_says_when_its_text_has_no_focused_field() {
    use demi_command_package_mobile_protocol::live::NO_FOCUSED_FIELD;
    let helper = FakeHelper::new(0);
    let mobile = Mobile::start_ios(helper.clone()).await;
    let device = mobile.open(CHAT, "UDID-A").await;
    let mut page = Page::open(&mobile, CHAT).await;
    page.send(json!({ "type": "hello", "platform": "mac", "device": device })).await;
    page.send(json!({ "type": "looked", "looked": true })).await;
    page.send(json!({ "type": "input", "input": { "type": "text", "text": "東京" } })).await;
    let notice = tokio::time::timeout(std::time::Duration::from_secs(10), page.until("notice"))
        .await
        .expect("the view is told");
    let LiveModuleMessage::Notice(notice) = notice else { unreachable!() };
    assert_eq!((notice.code.as_str(), notice.message.as_str()), (NO_FOCUSED_FIELD, "東京"));
    assert!(helper.requests("setValue").is_empty());
}

/// Text a US keyboard cannot type needs a focused field on an iPhone, as
/// it is added to that field's value: without one, `type` fails saying so
/// and changes nothing. Planted defect this catches: the text set on
/// whatever element comes first, or dropped without a word. One fake
/// helper, milliseconds.
#[tokio::test]
async fn text_without_us_keys_needs_a_focused_field_on_an_iphone() {
    let helper = FakeHelper::new(0);
    let mobile = Mobile::start_ios(helper.clone()).await;
    let device = mobile.open(CHAT, "UDID-A").await;
    let (_, message) = mobile.run(CHAT, "type", json!({ "device": device, "text": "東京" })).await.failure();
    assert!(message.contains("no focused text field to add it to"), "{message}");
    assert!(helper.requests("setValue").is_empty());
}

/// A simulator, which reports nothing of its screen, is off from the Lock
/// the Host pressed until its next input, and the view says so as it
/// changes: Lock turns it off, the next tap turns it on. Planted defects
/// this catches: a Lock the view does not hear of, and a screen still off
/// after the user's tap woke it. One fake helper, milliseconds.
#[tokio::test]
async fn a_simulator_is_off_from_its_lock_until_the_next_input() {
    let helper = FakeHelper::new(0);
    let mobile = Mobile::start_ios(helper.clone()).await;
    let device = mobile.open(CHAT, "iPhone 17").await;
    let mut page = Page::open(&mobile, CHAT).await;
    page.send(json!({ "type": "hello", "platform": "mac", "device": device })).await;
    page.send(json!({ "type": "looked", "looked": true })).await;
    let on = |message: LiveModuleMessage| serde_json::to_value(message).unwrap()["on"].as_bool().unwrap();
    assert!(on(screen(&mut page).await));
    mobile.run(CHAT, "key", json!({ "device": device, "key": "lock" })).await.json();
    assert!(!on(screen(&mut page).await), "Lock turned the screen off");
    page.send(json!({ "type": "input", "input": { "type": "touch", "action": "start", "contacts": [{ "id": 0, "x": 100.0, "y": 200.0 }] } })).await;
    assert!(on(screen(&mut page).await), "the tap woke it");
}

/// Wake pressed twice in a row on a simulator that is off turns it on and
/// leaves it on: the second Wake waits for the first's press and finds the
/// screen on, so one side button is pressed, not two.
///
/// Planted defect this catches: each Wake reading the screen's state while the
/// other's press is on its way, which turns the screen on and off again.
///
/// Cost: one fake helper, milliseconds.
#[tokio::test]
async fn a_double_wake_on_a_simulator_presses_the_side_button_once() {
    let helper = FakeHelper::new(0);
    let mobile = Mobile::start_ios(helper.clone()).await;
    let device = mobile.open(CHAT, "iPhone 17").await;
    let mut page = Page::open(&mobile, CHAT).await;
    page.send(json!({ "type": "hello", "platform": "mac", "device": device })).await;
    page.send(json!({ "type": "looked", "looked": true })).await;
    let on = |message: LiveModuleMessage| serde_json::to_value(message).unwrap()["on"].as_bool().unwrap();
    assert!(on(screen(&mut page).await));
    mobile.run(CHAT, "key", json!({ "device": device, "key": "lock" })).await.json();
    assert!(!on(screen(&mut page).await), "Lock turned the screen off");
    // The first Wake's press is still on its way when the second comes.
    helper.held.send_replace(true);
    page.send(json!({ "type": "wake" })).await;
    page.send(json!({ "type": "wake" })).await;
    // The view takes this once it has taken the second Wake.
    page.send(json!({ "type": "release" })).await;
    helper.held.send_replace(false);
    assert!(on(screen(&mut page).await), "the Wakes leave the screen on");
    // The helper answers in order, so an input the agent sends after the
    // Wakes is answered after any press they made.
    mobile.run(CHAT, "key", json!({ "device": device, "key": "home" })).await.json();
    let presses = helper.requests("button").iter().filter(|params| params["button"] == "lock").count();
    assert_eq!(presses, 2, "the agent's Lock and one Wake press the side button");
}

/// The screen shows the orientation its display took, whoever turned it: a
/// turn the app in front refuses leaves it as it was, and Rotate says so,
/// the next press turning on from how the device is held, and a turn the
/// app makes itself shows on the device's stream
/// (`crates/plugin-mobile/command/ios-sim/Sources/DemiIosSim/DeviceSession.swift`).
/// Planted defect this catches: the orientation Demi asked for taken as the
/// screen's, which showed an app that stayed across as turned upright. One
/// fake helper, milliseconds.
#[tokio::test]
async fn the_screen_shows_the_orientation_its_display_took() {
    let helper = FakeHelper::new(0);
    let mobile = Mobile::start_ios(helper.clone()).await;
    let device = mobile.open(CHAT, "UDID-A").await;
    let mut page = Page::open(&mobile, CHAT).await;
    page.send(json!({ "type": "hello", "platform": "mac", "device": device })).await;
    page.send(json!({ "type": "looked", "looked": true })).await;
    page.until("state").await;
    // The app turns but for upside down, as most iPhone apps do.
    helper.refuses("portraitUpsideDown");
    let mut turns = Vec::new();
    for _ in 0..3 {
        let turned = mobile.run(CHAT, "rotate", json!({ "device": device, "orientation": "right" })).await.json();
        turns.push(turned["orientation"].as_str().unwrap().to_owned());
    }
    // The refused turn leaves the screen as it was, and the next press
    // turns on from how the device is held, as Simulator.app does.
    assert_eq!(turns, ["landscape-right", "landscape-right", "landscape-left"]);
    assert_eq!(
        helper.requests("orient").iter().map(|params| params["orientation"].as_str().unwrap().to_owned()).collect::<Vec<_>>(),
        ["landscapeRight", "portraitUpsideDown", "landscapeLeft"]
    );

    helper.turns("UDID-A", "portrait");
    let state = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let state = serde_json::to_value(page.until("state").await).unwrap();
            if state["device"]["orientation"] == "portrait" {
                return state;
            }
        }
    })
    .await
    .expect("the view says the screen turned");
    assert_eq!(state["device"]["orientation"], "portrait");
}
