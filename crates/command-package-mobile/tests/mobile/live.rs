//! The `mobile` stream as the page speaks it
//! (`crates/command-package-mobile/src/live.rs`): the view reports the agent's
//! activity on the device it watches,
//! and keeps reading the page meanwhile.

use std::time::Duration;

use demi_command_package_mobile_protocol::live::{LiveModuleMessage, LiveViewerMessage};
use demi_command_package_mobile_protocol::mobile::DeviceId;
use demi_command_protocol::{CommandCaller, Record};
use demi_live_view_protocol::LiveActivity;
use demi_mobile::driver::Points;
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
    /// sends no other (`crates/command-package-mobile/src/live.rs`).
    pictures: std::collections::VecDeque<Vec<u8>>,
    demanded: bool,
}

impl Page {
    /// Opens `conversation`'s view as a page does.
    pub(crate) async fn open(mobile: &Mobile, conversation: &str) -> Self {
        let (input, output) = mobile.live(conversation).await;
        Self { input, output, buffered: Vec::new(), pictures: Default::default(), demanded: false }
    }

    /// The data of the view's next picture, skipping control messages.
    pub(crate) async fn video(&mut self) -> Vec<u8> {
        if let Some(picture) = self.pictures.pop_front() {
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
    let mut page = Page { input, output, buffered: Vec::new(), pictures: Default::default(), demanded: false };
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

/// The view says the agent uses a device from the start of its command on it
/// until three seconds after its last one ends, a running command keeping it
/// on, and marks where the agent taps. The panel's controls are the user's and
/// show neither (`crates/command-package-mobile/src/live.rs`).
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
    page.send(json!({ "type": "hello", "platform": "mac" })).await;
    page.send(json!({ "type": "watch", "device": device })).await;
    page.send(json!({ "type": "frame", "device": device, "width": 800, "height": 1600, "scale": 2.0 })).await;
    let LiveModuleMessage::Frame { orientation, frame, .. } = page.until("frame").await else { unreachable!() };
    assert_eq!(orientation.to_string(), "portrait");
    let framed = serde_json::to_value(&frame).unwrap();
    assert_eq!(framed["kind"], "drawn");
    assert_eq!(framed["corners"]["topLeft"], serde_json::json!({ "kind": "round", "radius": 55.0 }));
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

/// The user's input on a device reaches it while the agent's `wait` on the same
/// device runs, which holds nothing the input waits for
/// (`crates/command-package-mobile/src/live.rs`).
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

/// The view tells the page whether the watched device's screen is on as
/// it watches it and as it turns off and on. Planted defect this catches:
/// a screen that went dark with the view showing its last picture as
/// though it were on. One fake device, milliseconds.
#[tokio::test]
async fn the_view_says_when_the_screen_turns_off_and_on() {
    let mobile = Mobile::start().await;
    let device = mobile.open(CHAT, "iPhone 17").await;
    let mut page = Page::open(&mobile, CHAT).await;
    page.send(json!({ "type": "hello", "platform": "mac" })).await;
    page.send(json!({ "type": "watch", "device": device })).await;
    let on = |message: LiveModuleMessage| serde_json::to_value(message).unwrap()["on"].as_bool().unwrap();
    assert!(on(screen(&mut page).await), "the screen is on as the view starts watching");
    let fake = mobile.driver.device("UDID-1");
    fake.power(false);
    assert!(!on(screen(&mut page).await), "the device slept");
    fake.power(true);
    assert!(on(screen(&mut page).await), "the device woke");
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
    page.send(json!({ "type": "hello", "platform": "mac" })).await;
    page.send(json!({ "type": "watch", "device": device })).await;
    let mut captures = mobile.driver.device("UDID-1").captures();
    page.send(json!({ "type": "shown", "device": device, "width": 590, "height": 1278 })).await;
    let first = tokio::time::timeout(Duration::from_secs(10), captures.wait_for(|sizes| !sizes.is_empty()))
        .await
        .expect("a capture once the panel said its size")
        .unwrap()[0];
    assert_eq!(first, (590, 1278), "the capture is at the panel's size");
    page.send(json!({ "type": "shown", "device": device, "width": 300, "height": 650 })).await;
    page.send(json!({ "type": "shown", "device": device, "width": 2000, "height": 4300 })).await;
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

/// The viewer's scroll wheel scrolls the device's screen: a turn is the
/// swipe that moves the screen as far, the finger going against the turn
/// from where the pointer is, and stopping at the screen's edge
/// (`crates/command-package-mobile/src/live.rs`). Planted defect this catches: a wheel the
/// view drops, as it did before. One fake device, milliseconds.
#[tokio::test]
async fn the_viewers_wheel_scrolls_the_screen() {
    let mobile = Mobile::start().await;
    let device = mobile.open(CHAT, "iPhone 17").await;
    let mut page = Page::open(&mobile, CHAT).await;
    page.send(json!({ "type": "hello", "platform": "mac" })).await;
    page.send(json!({ "type": "watch", "device": device })).await;
    page.until("state").await;
    let swipes = mobile.driver.device("UDID-1").swipes();
    let wheel = |y: f64, delta_y: f64| json!({ "type": "input", "device": device, "input": { "type": "wheel", "x": 200.0, "y": y, "deltaX": 0.0, "deltaY": delta_y, "modifiers": 0 } });
    let swiped = |swipes: &tokio::sync::watch::Receiver<Vec<(Points, Points)>>, count: usize| {
        let mut swipes = swipes.clone();
        async move {
            tokio::time::timeout(Duration::from_secs(10), swipes.wait_for(|swipes| swipes.len() == count))
                .await
                .expect("the wheel reached the device")
                .unwrap()
                .last()
                .copied()
                .unwrap()
        }
    };
    // Down the page: the finger goes up by the turn.
    page.send(wheel(400.0, 120.0)).await;
    assert_eq!(swiped(&swipes, 1).await, (Points { x: 200.0, y: 400.0 }, Points { x: 200.0, y: 280.0 }));
    // Far up the page: the finger stops at the screen's bottom edge.
    page.send(wheel(400.0, -10_000.0)).await;
    // The fake's screen is 393 × 852 points.
    assert_eq!(swiped(&swipes, 2).await, (Points { x: 200.0, y: 400.0 }, Points { x: 200.0, y: 852.0 }));
}
