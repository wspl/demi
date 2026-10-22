//! The `mobile` stream as the page speaks it
//! (`crates/plugin-mobile/command/src/live.rs`): the view reports the agent's
//! activity on the device it watches,
//! and keeps reading the page meanwhile.

use std::time::Duration;

use demi_command_package_mobile_protocol::live::{LiveModuleMessage, LiveViewerMessage};
use demi_command_package_mobile_protocol::mobile::DeviceId;
use demi_command_protocol::{CommandCaller, Record};
use demi_live_view_protocol::LiveActivity;
use serde_json::json;

use crate::{CHAT, Mobile};

/// The page's message as the stream carries it.
fn framed(message: serde_json::Value) -> bytes::Bytes {
    let message: LiveViewerMessage = serde_json::from_value(message).unwrap();
    demi_live_view::frames::control(&message)
}

/// The page's side of the stream, which answers the view's demands for
/// input as the runner does: one at a time, so a second demand while one
/// is open is the runner's refusal (`crates/command-protocol/src/lib.rs`).
pub(crate) struct Page {
    input: demi_command_sdk::CommandInput,
    output: demi_command_sdk::CommandOutput,
    buffered: Vec<u8>,
    /// The pictures that came while the test waited for a control message:
    /// a key frame may come before the frame message, and a still screen
    /// sends no other (`crates/plugin-mobile/command/src/live.rs`).
    pictures: std::collections::VecDeque<Vec<u8>>,
    /// Those pictures' headers.
    headers: std::collections::VecDeque<demi_live_view_protocol::VideoHeader>,
    demanded: bool,
}

/// What the view sent: a control message, or a picture's header.
pub(crate) enum Said {
    Message(LiveModuleMessage),
    Picture(demi_live_view_protocol::VideoHeader),
}

impl Page {
    /// Opens `conversation`'s view as a page does.
    pub(crate) async fn open(mobile: &Mobile, conversation: &str) -> Self {
        let (input, output) = mobile.live(conversation).await;
        Self {
            input,
            output,
            buffered: Vec::new(),
            pictures: Default::default(),
            headers: Default::default(),
            demanded: false,
        }
    }

    /// The data of the view's next picture, skipping control messages.
    pub(crate) async fn video(&mut self) -> Vec<u8> {
        if let Some(picture) = self.pictures.pop_front() {
            self.headers.pop_front();
            return picture;
        }
        loop {
            while let Some(frame) = self.next_frame() {
                if frame[4] == demi_live_view_protocol::VIDEO_FRAME {
                    return picture_of(&frame);
                }
            }
            self.record().await;
        }
    }

    /// The next whole frame the view sent, its length included, if one is
    /// buffered.
    fn next_frame(&mut self) -> Option<Vec<u8>> {
        if self.buffered.len() < 5 {
            return None;
        }
        let length = u32::from_be_bytes(self.buffered[..4].try_into().unwrap()) as usize;
        if self.buffered.len() < 4 + length {
            return None;
        }
        Some(self.buffered.drain(..4 + length).collect())
    }

    /// Writes `message` once the view asks for input, as the runner does.
    pub(crate) async fn send(&mut self, message: serde_json::Value) {
        while !self.demanded {
            self.record().await;
        }
        self.input.write(framed(message)).await.unwrap();
        self.demanded = false;
    }

    /// Acknowledges `picture` as a page does on its arrival, so the view's
    /// window has room for the next.
    pub(crate) async fn ack(&mut self, picture: &demi_live_view_protocol::VideoHeader) {
        while !self.demanded {
            self.record().await;
        }
        let mut payload = Vec::new();
        let ack = demi_live_view_protocol::ChunkAck {
            generation: picture.generation,
            sequence: picture.sequence,
            chunk: picture.chunk,
            received: picture.timestamp,
        };
        ack.write(&mut payload);
        let frame = demi_live_view::frames::framed(demi_live_view_protocol::ACK_FRAME, payload.len(), |bytes| {
            bytes.extend_from_slice(&payload);
        });
        self.input.write(frame).await.unwrap();
        self.demanded = false;
    }

    /// Whether the view sends nothing for `long`, reading the page's
    /// records meanwhile; a test's paused clock passes it at once.
    pub(crate) async fn silent_for(&mut self, long: Duration) -> bool {
        let before = self.buffered.len();
        let _waited = tokio::time::timeout(long, async {
            loop {
                self.record().await;
            }
        })
        .await;
        self.buffered.len() == before
    }

    /// The view's next message or picture, in the order it sent them; the
    /// pictures [`Page::until`] kept come first.
    pub(crate) async fn said(&mut self) -> Said {
        if let Some(picture) = self.headers.pop_front() {
            self.pictures.pop_front();
            return Said::Picture(picture);
        }
        loop {
            if let Some(frame) = self.next_frame() {
                if frame[4] == demi_live_view_protocol::VIDEO_FRAME {
                    let (header, _) = demi_live_view_protocol::VideoHeader::split(&frame[5..]).unwrap();
                    return Said::Picture(header);
                }
                if frame[4] == demi_live_view_protocol::CONTROL_FRAME {
                    return Said::Message(serde_json::from_slice(&frame[5..]).unwrap());
                }
                continue;
            }
            self.record().await;
        }
    }

    /// The view's completion, once it ends; a `gone` message before it fails
    /// the test.
    pub(crate) async fn ends_without_gone(&mut self) -> demi_command_protocol::Completion {
        loop {
            while let Some(frame) = self.next_frame() {
                if frame[4] == demi_live_view_protocol::CONTROL_FRAME {
                    let message: LiveModuleMessage = serde_json::from_slice(&frame[5..]).unwrap();
                    assert_ne!(serde_json::to_value(&message).unwrap()["type"], "gone", "the view said the device is gone");
                }
            }
            match self.output.next().await.unwrap() {
                Some(Record::Stdout(bytes)) => self.buffered.extend_from_slice(&bytes),
                Some(Record::InputPull) => {}
                Some(Record::Completion(completion)) => return completion,
                Some(_) => {}
                None => panic!("the view ended without a completion"),
            }
        }
    }

    /// Takes the view's next record.
    async fn record(&mut self) {
        match self.output.next().await.unwrap() {
            Some(Record::Stdout(bytes)) => self.buffered.extend_from_slice(&bytes),
            Some(Record::InputPull) => {
                assert!(!self.demanded, "overlapping input demands");
                self.demanded = true;
            }
            Some(Record::Completion(completion)) => panic!("the view ended: {completion:?}"),
            Some(_) => {}
            None => panic!("the view ended without a completion"),
        }
    }

    /// The view's next `Frame` that carries the frame, skipping those sent
    /// while the device still read what it needs.
    pub(crate) async fn framed(&mut self) -> LiveModuleMessage {
        loop {
            let message = self.until("frame").await;
            if let LiveModuleMessage::Frame { frame: Some(_), .. } = &message {
                return message;
            }
        }
    }

    /// The view's next control message of `kind`, skipping the others and
    /// keeping the pictures for [`Page::video`].
    pub(crate) async fn until(&mut self, kind: &str) -> LiveModuleMessage {
        loop {
            while let Some(frame) = self.next_frame() {
                if frame[4] == demi_live_view_protocol::VIDEO_FRAME {
                    self.pictures.push_back(picture_of(&frame));
                    self.headers.push_back(demi_live_view_protocol::VideoHeader::split(&frame[5..]).unwrap().0);
                } else if frame[4] == demi_live_view_protocol::CONTROL_FRAME {
                    let message: LiveModuleMessage = serde_json::from_slice(&frame[5..]).unwrap();
                    if serde_json::to_value(&message).unwrap()["type"] == kind {
                        return message;
                    }
                }
            }
            self.record().await;
        }
    }
}

/// The H.264 data of a video `frame`, its length and kind included.
fn picture_of(frame: &[u8]) -> Vec<u8> {
    let (_, data) = demi_live_view_protocol::VideoHeader::split(&frame[5..]).unwrap();
    data.to_vec()
}

/// The agent's tap shows on the view while the view waits for the page,
/// and the page's next message still reaches it. Planted defect this
/// catches: a read of the page that the tap's event interrupts, whose
/// demand for input the next read overlaps, which ends the view. One fake
/// device, milliseconds.
#[tokio::test(start_paused = true)]
async fn the_view_reports_the_agents_activity_and_keeps_reading_the_page() {
    let mobile = Mobile::start().await;
    let device = mobile.open(CHAT, "iPhone 17").await;
    let (input, output) = mobile.live(CHAT).await;
    let mut page = Page { input, output, buffered: Vec::new(), pictures: Default::default(), headers: Default::default(), demanded: false };
    page.send(json!({ "type": "hello", "platform": "mac", "device": device })).await;
    page.send(json!({ "type": "looked", "looked": true })).await;
    page.until("state").await;

    mobile.run(CHAT, "tap", json!({ "device": device, "at": ["100", "200"] })).await.json();
    let activity = page.until("activity").await;
    assert_eq!(serde_json::to_value(&activity).unwrap()["activity"]["active"], true);

    // The page's next messages after the activity: not looked at, then
    // looked at again, which the view answers with the device's state.
    tokio::time::sleep(Duration::from_millis(10)).await;
    page.until("heartbeat").await;
    page.send(json!({ "type": "looked", "looked": false })).await;
    page.send(json!({ "type": "looked", "looked": true })).await;
    page.until("state").await;
}

/// A device's stream that nobody looks at sends nothing, not even a
/// heartbeat, from its hello and again once it is no longer looked at, and
/// says the device's state at once when it is looked at again
/// (`docs/decisions/0017-each-device-streams-on-its-own.md`).
///
/// Planted defects this catches: a view that speaks before it is looked
/// at, and a writer that keeps its heartbeat while nobody looks.
///
/// Cost: one fake device, a paused clock.
#[tokio::test(start_paused = true)]
async fn a_stream_nobody_looks_at_sends_nothing() {
    let mobile = Mobile::start().await;
    let device = mobile.open(CHAT, "iPhone 17").await;
    let mut page = Page::open(&mobile, CHAT).await;
    page.send(json!({ "type": "hello", "platform": "mac", "device": device })).await;
    assert!(page.silent_for(Duration::from_secs(5)).await, "the view spoke before it was looked at");
    page.send(json!({ "type": "looked", "looked": true })).await;
    page.until("state").await;
    page.until("heartbeat").await;
    page.send(json!({ "type": "looked", "looked": false })).await;
    // What the view queued before the page's word may still come; then nothing.
    page.silent_for(Duration::from_millis(100)).await;
    page.buffered.clear();
    assert!(page.silent_for(Duration::from_secs(5)).await, "the view spoke while nobody looked at it");
    page.send(json!({ "type": "looked", "looked": true })).await;
    page.until("state").await;
}

/// A device whose connection ended while its simulator runs is not gone: the
/// view ends without saying so, the page opens another stream, and that one
/// is answered once the device is attached again. Planted defect this
/// catches: the view told the device is gone, which the page never opens
/// again. One fake device, a paused clock.
#[tokio::test(start_paused = true)]
async fn a_stream_of_a_device_whose_connection_ended_ends_without_saying_gone() {
    let mobile = Mobile::start().await;
    let device = mobile.open(CHAT, "iPhone 17").await;
    let mut page = Page::open(&mobile, CHAT).await;
    page.send(json!({ "type": "hello", "platform": "mac", "device": device })).await;
    page.send(json!({ "type": "looked", "looked": true })).await;
    page.until("state").await;
    mobile.driver.device("UDID-1").lose();
    let completion = page.ends_without_gone().await;
    assert!(completion.error.is_some() || completion.exit_code != 0, "{completion:?}");

    let mut again = Page::open(&mobile, CHAT).await;
    again.send(json!({ "type": "hello", "platform": "mac", "device": device })).await;
    again.send(json!({ "type": "looked", "looked": true })).await;
    again.until("state").await;
}

/// The view says the agent uses a device from the start of its command on it
/// until three seconds after its last one ends, a running command keeping it
/// on, and marks where the agent taps. The panel's controls are the user's and
/// show neither (`crates/plugin-mobile/command/src/live.rs`).
///
/// Planted defects this catches:
///
/// - the note only while input is delivered;
/// - a panel control shown as the agent's;
/// - a linger the agent's next command does not renew.
///
/// Cost: one fake device, a paused clock.
#[tokio::test(start_paused = true)]
async fn the_agent_uses_a_device_until_three_seconds_after_its_last_command() {
    let mobile = Mobile::start().await;
    let device = mobile.open(CHAT, "iPhone 17").await;
    let open = mobile
        .service
        .devices()
        .get(CHAT, &DeviceId::numbered(device[1..].parse().unwrap()), &tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();
    let activity = || *open.activity.borrow();
    assert!(activity().active, "opening it is the agent's use");
    tokio::time::sleep(Duration::from_millis(3100)).await;
    assert!(!activity().active);

    let tap = json!({ "device": device, "at": ["100", "200"] });
    mobile.run_by(CHAT, "tap", tap.clone(), CommandCaller::User {}).await.json();
    mobile.run_by(CHAT, "key", json!({ "device": device, "key": "home" }), CommandCaller::User {}).await.json();
    assert_eq!(activity(), LiveActivity { active: false, point: None }, "the panel's controls are the user's");

    mobile.run(CHAT, "tap", tap).await.json();
    let marked = activity();
    assert!(marked.active && marked.point.is_some(), "the agent's tap: {marked:?}");
    tokio::time::sleep(Duration::from_secs(2)).await;
    // A wait that runs for five seconds keeps the note on while it runs.
    let waited = mobile.run(CHAT, "wait", json!({ "device": device, "text": "Nowhere", "timeout": 5 }));
    let during = async {
        tokio::time::sleep(Duration::from_millis(4500)).await;
        activity().active
    };
    let (_timed_out, during) = tokio::join!(waited, during);
    assert!(during, "a running command keeps the note on");
    tokio::time::sleep(Duration::from_millis(2900)).await;
    assert!(activity().active, "three seconds run from the last command's end");
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(activity(), LiveActivity { active: false, point: marked.point }, "the mark stays where the agent tapped");

    // A quick command within the three seconds starts them again.
    mobile.run(CHAT, "key", json!({ "device": device, "key": "home" })).await.json();
    tokio::time::sleep(Duration::from_secs(2)).await;
    mobile.run(CHAT, "key", json!({ "device": device, "key": "home" })).await.json();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(activity().active, "the second command's three seconds");
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(!activity().active);
}

/// The view gets the device's frame for the size it lays the device out at,
/// once for each size and orientation, and again when the device turns, its
/// buttons on the edges they turned to. A device without a frame installed gets
/// the frame the view draws.
///
/// Planted defects this catches:
///
/// - a frame drawn again for a size the view has;
/// - a frame that stays upright when the device turns.
///
/// Cost: one fake device, milliseconds.
#[tokio::test]
async fn the_view_gets_the_frame_for_its_size_and_again_when_the_device_turns() {
    let mobile = Mobile::start().await;
    let device = mobile.open(CHAT, "iPhone 17").await;
    let mut page = Page::open(&mobile, CHAT).await;
    page.send(json!({ "type": "hello", "platform": "mac", "device": device })).await;
    page.send(json!({ "type": "looked", "looked": true })).await;
    page.send(json!({ "type": "frame", "width": 800, "height": 1600, "scale": 2.0 })).await;
    let LiveModuleMessage::Frame { orientation, frame, .. } = page.until("frame").await else { unreachable!() };
    assert_eq!(orientation.to_string(), "portrait");
    let framed = serde_json::to_value(&frame).unwrap();
    assert_eq!(framed["kind"], "drawn");
    assert_eq!(framed["corners"]["topLeft"], serde_json::json!({ "kind": "round", "radius": 55.0 }));
    let sides = |framed: &serde_json::Value| -> Vec<String> {
        framed["buttons"].as_array().unwrap().iter().map(|mark| mark["side"].as_str().unwrap().to_owned()).collect()
    };
    assert_eq!(sides(&framed), ["left", "left", "left", "right"]);

    // A turn needs a new frame.
    mobile.run(CHAT, "rotate", json!({ "device": device, "orientation": "landscape" })).await.json();
    let turned = tokio::time::timeout(Duration::from_secs(10), page.until("frame")).await;
    let Ok(LiveModuleMessage::Frame { orientation, frame, .. }) = turned else { panic!("no frame came after the turn") };
    assert_eq!(orientation.to_string(), "landscape-left", "the frame sent after the turn is the turned one");
    assert_eq!(sides(&serde_json::to_value(&frame).unwrap()), ["bottom", "bottom", "bottom", "top"]);
    let drawn = mobile.log().iter().filter(|line| line.contains("frame 800x1600")).count();
    assert_eq!(drawn, 2, "a frame drawn once for each size and orientation");
}

/// The capture starts once the stream is looked at and the view said the
/// area it lays the device out in, before any frame is drawn or the view
/// says the screen's size, fitted to the most of the screen that area can
/// show. Planted defect this catches: a capture that waits for the view's
/// size of the screen, which the view knows only once the frame shows, so
/// a frame that never came left the device without pictures. One fake
/// device, milliseconds.
#[tokio::test]
async fn the_capture_starts_without_waiting_for_the_frame() {
    let mobile = Mobile::start().await;
    let device = mobile.open(CHAT, "iPhone 17").await;
    let mut page = Page::open(&mobile, CHAT).await;
    page.send(json!({ "type": "hello", "platform": "mac", "device": device })).await;
    page.send(json!({ "type": "looked", "looked": true })).await;
    let mut captures = mobile.driver.device("UDID-1").captures();
    // An area as wide as the screen and half as high: half the screen.
    page.send(json!({ "type": "frame", "width": 1178, "height": 1278, "scale": 2.0 })).await;
    let first = tokio::time::timeout(Duration::from_secs(10), captures.wait_for(|sizes| !sizes.is_empty()))
        .await
        .expect("a capture once the view said its area")
        .unwrap()[0];
    assert_eq!(first, (590, 1278), "the capture fits the area, as the screen half its size");
}

/// The user's input on a device reaches it while the agent's `wait` on the same
/// device runs, which holds nothing the input waits for
/// (`crates/plugin-mobile/command/src/live.rs`).
///
/// Planted defect this catches: one turn for every command on a device, which
/// kept the panel's Home waiting for the agent's five-minute wait.
///
/// Cost: one fake device, a paused clock.
#[tokio::test(start_paused = true)]
async fn the_users_input_does_not_wait_for_the_agents_wait() {
    let mobile = Mobile::start().await;
    let device = mobile.open(CHAT, "iPhone 17").await;
    let mut reads = mobile.driver.device("UDID-1").reads();
    reads.borrow_and_update();
    let (_input, waiting) = mobile
        .begin(CHAT, "wait", json!({ "device": device, "text": "Nowhere", "timeout": 300 }), CommandCaller::agent(1))
        .await;
    // The wait read the screen: it holds its turn.
    reads.changed().await.unwrap();
    let home = mobile.run_by(CHAT, "key", json!({ "device": device, "key": "home" }), CommandCaller::User {});
    let pressed = tokio::time::timeout(Duration::from_secs(10), home)
        .await
        .expect("the user's Home waited for the agent's wait");
    pressed.json();
    assert_eq!(mobile.log().last().unwrap(), "UDID-1: press Home");
    drop(waiting);
}

/// The pictures are captured at the size the panel shows the screen at,
/// not the screen's own 1179 × 2556, and again larger only when the panel
/// grows; a panel that shrinks keeps them. Planted defects this catches:
/// pictures at the native size whatever the panel, and a capture started
/// again at each shrink. One fake device, milliseconds.
#[tokio::test]
async fn pictures_are_captured_at_the_size_the_panel_shows() {
    let mobile = Mobile::start().await;
    let device = mobile.open(CHAT, "iPhone 17").await;
    let mut page = Page::open(&mobile, CHAT).await;
    page.send(json!({ "type": "hello", "platform": "mac", "device": device })).await;
    page.send(json!({ "type": "looked", "looked": true })).await;
    let mut captures = mobile.driver.device("UDID-1").captures();
    page.send(json!({ "type": "shown", "width": 590, "height": 1278 })).await;
    let first = tokio::time::timeout(Duration::from_secs(10), captures.wait_for(|sizes| !sizes.is_empty()))
        .await
        .expect("a capture once the panel said its size")
        .unwrap()[0];
    assert_eq!(first, (590, 1278), "the capture is at the panel's size");
    page.send(json!({ "type": "shown", "width": 300, "height": 650 })).await;
    page.send(json!({ "type": "shown", "width": 2000, "height": 4300 })).await;
    let sizes = tokio::time::timeout(Duration::from_secs(10), captures.wait_for(|sizes| sizes.contains(&(1178, 2556))))
        .await
        .expect("a capture at the native size once the panel grew past it")
        .unwrap()
        .clone();
    // A failed capture is tried again at its size; no other size comes.
    assert!(
        sizes.iter().all(|size| [(590, 1278), (1178, 2556)].contains(size)),
        "a smaller panel started another capture: {sizes:?}"
    );
}

/// The view's next word on the screen's state, within ten seconds.
pub(crate) async fn screen(page: &mut Page) -> LiveModuleMessage {
    tokio::time::timeout(Duration::from_secs(10), page.until("screen"))
        .await
        .expect("the view said nothing of the screen")
}

