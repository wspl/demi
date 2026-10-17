//! The `mobile` stream as the page speaks it (`mobile.md` § The live
//! view): the view reports the agent's activity on the device it watches,
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
/// is open is the runner's refusal (`native-runtime.md` § Request body and
/// input demand).
pub(crate) struct Page {
    input: demi_command_sdk::CommandInput,
    output: demi_command_sdk::CommandOutput,
    buffered: Vec<u8>,
    demanded: bool,
}

impl Page {
    /// Opens `conversation`'s view as a page does.
    pub(crate) async fn open(mobile: &Mobile, conversation: &str) -> Self {
        let (input, output) = mobile.live(conversation).await;
        Self { input, output, buffered: Vec::new(), demanded: false }
    }

    /// The data of the view's next picture.
    pub(crate) async fn video(&mut self) -> Vec<u8> {
        loop {
            while self.buffered.len() >= 5 {
                let length = u32::from_be_bytes(self.buffered[..4].try_into().unwrap()) as usize;
                if self.buffered.len() < 4 + length {
                    break;
                }
                let frame: Vec<u8> = self.buffered.drain(..4 + length).collect();
                if frame[4] == demi_live_view_protocol::VIDEO_FRAME {
                    let (_, data) = demi_live_view_protocol::VideoHeader::split(&frame[5..]).unwrap();
                    return data.to_vec();
                }
            }
            self.record().await;
        }
    }

    /// Writes `message` once the view asks for input, as the runner does.
    pub(crate) async fn send(&mut self, message: serde_json::Value) {
        while !self.demanded {
            self.record().await;
        }
        self.input.write(framed(message)).await.unwrap();
        self.demanded = false;
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

    /// The view's next control message of `kind`, skipping the others.
    pub(crate) async fn until(&mut self, kind: &str) -> LiveModuleMessage {
        loop {
            while self.buffered.len() >= 5 {
                let length = u32::from_be_bytes(self.buffered[..4].try_into().unwrap()) as usize;
                if self.buffered.len() < 4 + length {
                    break;
                }
                let frame: Vec<u8> = self.buffered.drain(..4 + length).collect();
                if frame[4] == demi_live_view_protocol::CONTROL_FRAME {
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
    let mut page = Page { input, output, buffered: Vec::new(), demanded: false };
    page.send(json!({ "type": "hello", "platform": "mac" })).await;
    page.send(json!({ "type": "watch", "device": device })).await;
    page.until("state").await;

    mobile.run(CHAT, "tap", json!({ "device": device, "at": ["100", "200"] })).await.json();
    let activity = page.until("activity").await;
    assert_eq!(serde_json::to_value(&activity).unwrap()["device"], device);

    // The page's next message after the activity: watching nothing.
    tokio::time::sleep(Duration::from_millis(10)).await;
    page.until("heartbeat").await;
    page.send(json!({ "type": "watch", "device": null })).await;
    // The view answers it with a state that watches nothing; the states
    // before it are the tap's own.
    loop {
        let state = page.until("state").await;
        if serde_json::to_value(&state).unwrap()["watched"].is_null() {
            break;
        }
    }
}

/// The view says the agent uses a device from the start of its command on
/// it until three seconds after its last one ends, a running command
/// keeping it on, and marks where the agent taps; the panel's controls are
/// the user's and show neither (`mobile.md` § The live view). Planted
/// defects this catches: the note only while input is delivered, a panel
/// control shown as the agent's, and a linger the agent's next command does
/// not renew. One fake device, a paused clock.
#[tokio::test(start_paused = true)]
async fn the_agent_uses_a_device_until_three_seconds_after_its_last_command() {
    let mobile = Mobile::start().await;
    let device = mobile.open(CHAT, "iPhone 17").await;
    let open = mobile
        .service
        .devices()
        .get(CHAT, &DeviceId::numbered(device[1..].parse().unwrap()))
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

/// The view gets the device's frame for the size it lays the device out
/// at, once for each size and orientation, and again when the device
/// turns, its buttons on the edges they turned to; a device without a
/// frame installed gets the outline. Planted defects this catches: a
/// frame drawn again for a size the view has, and one that stays upright
/// when the device turns. One fake device, milliseconds.
#[tokio::test]
async fn the_view_gets_the_frame_for_its_size_and_again_when_the_device_turns() {
    let mobile = Mobile::start().await;
    let device = mobile.open(CHAT, "iPhone 17").await;
    let mut page = Page::open(&mobile, CHAT).await;
    page.send(json!({ "type": "hello", "platform": "mac" })).await;
    page.send(json!({ "type": "watch", "device": device })).await;
    page.send(json!({ "type": "frame", "device": device, "width": 800, "height": 1600, "scale": 2.0 })).await;
    let LiveModuleMessage::Frame { orientation, frame, .. } = page.until("frame").await else { unreachable!() };
    assert_eq!(orientation.to_string(), "portrait");
    let framed = serde_json::to_value(&frame).unwrap();
    assert_eq!(framed["kind"], "outline");
    assert_eq!(framed["cornerRadius"], 55.0);
    let sides = |framed: &serde_json::Value| -> Vec<String> {
        framed["buttons"].as_array().unwrap().iter().map(|mark| mark["side"].as_str().unwrap().to_owned()).collect()
    };
    assert_eq!(sides(&framed), ["left", "left", "left", "right"]);

    // The same size again needs no new frame; a turn does.
    page.send(json!({ "type": "frame", "device": device, "width": 800, "height": 1600, "scale": 2.0 })).await;
    mobile.run(CHAT, "rotate", json!({ "device": device, "orientation": "landscape" })).await.json();
    let turned = tokio::time::timeout(Duration::from_secs(10), page.until("frame")).await;
    let Ok(LiveModuleMessage::Frame { orientation, frame, .. }) = turned else { panic!("no frame came after the turn") };
    assert_eq!(orientation.to_string(), "landscape-left", "the frame sent after the turn is the turned one");
    assert_eq!(sides(&serde_json::to_value(&frame).unwrap()), ["bottom", "bottom", "bottom", "top"]);
    let drawn = mobile.log().iter().filter(|line| line.contains("frame 800x1600")).count();
    assert_eq!(drawn, 2, "a frame drawn once for each size and orientation");
}

/// A recording shows on every view of the device, whoever started it: the
/// agent's command starts one, and the view lists the device as
/// recording until it stops. Planted defect this catches: a recording the
/// view does not hear of. One fake device, milliseconds.
#[tokio::test]
async fn a_recording_the_agent_starts_shows_on_the_view() {
    let mobile = Mobile::start().await;
    let device = mobile.open(CHAT, "iPhone 17").await;
    let mut page = Page::open(&mobile, CHAT).await;
    page.send(json!({ "type": "hello", "platform": "mac" })).await;
    page.send(json!({ "type": "watch", "device": device })).await;
    let first = serde_json::to_value(page.until("state").await).unwrap();
    assert_eq!(first["devices"][0]["recording"], false);
    mobile.run(CHAT, "record", json!({ "device": device, "action": "start" })).await.json();
    tokio::time::timeout(Duration::from_secs(10), recording_is(&mut page, true))
        .await
        .expect("the view says the device records");
    mobile.run(CHAT, "record", json!({ "device": device, "action": "stop", "output": "clip.mp4" })).await.json();
    tokio::time::timeout(Duration::from_secs(10), recording_is(&mut page, false))
        .await
        .expect("the view says the recording stopped");
}

/// Waits for a state that lists the device's recording as `wanted`.
async fn recording_is(page: &mut Page, wanted: bool) {
    loop {
        let state = serde_json::to_value(page.until("state").await).unwrap();
        if state["devices"][0]["recording"] == wanted {
            return;
        }
    }
}
