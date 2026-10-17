//! Each command against the scripted iPhone: a 393 × 852 point screen at
//! 3 pixels a point, whose screenshot is 723 × 1568 (long side 1568), so a
//! screenshot pixel is 393/723 of a point.

use std::time::Duration;

use demi_command_package_mobile_protocol::mobile::PowerState;
use demi_mobile::driver::RawElement;
use serde_json::json;

use crate::{CHAT, Mobile, OTHER, element};

/// The screen the scripted device shows: a Settings button, a Name field
/// with the keyboard up.
fn form() -> Vec<RawElement> {
    let mut window = element("Window", "Form", (0.0, 0.0, 393.0, 852.0));
    let key = |label: &str, x: f64| RawElement {
        keyboard_key: true,
        ..element("Key", label, (x, 600.0, 39.0, 50.0))
    };
    window.children = vec![
        element("Button", "Settings", (100.0, 200.0, 80.0, 40.0)),
        element("TextField", "Name", (20.0, 300.0, 353.0, 44.0)),
        key("q", 0.0),
        key("w", 39.0),
    ];
    vec![window]
}

/// Every element's label, depth first.
fn labels(elements: &serde_json::Value) -> Vec<String> {
    let mut found = Vec::new();
    for each in elements.as_array().unwrap() {
        found.push(each["label"].as_str().unwrap_or_default().to_owned());
        if each.get("children").is_some() {
            found.extend(labels(&each["children"]));
        }
    }
    found
}

/// The ref of the element labelled `label`.
fn reference(elements: &serde_json::Value, label: &str) -> String {
    for each in elements.as_array().unwrap() {
        if each["label"] == label {
            return each["ref"].as_str().unwrap().to_owned();
        }
        if each.get("children").is_some() {
            let found = reference(&each["children"], label);
            if !found.is_empty() {
                return found;
            }
        }
    }
    String::new()
}

/// The thirteen commands, in an agent's order, each with what reaches the
/// device and what it prints, in screenshot pixels.
#[tokio::test(start_paused = true)]
async fn every_command_acts_in_screenshot_pixels() {
    let mobile = Mobile::start().await;

    // list: the Host's simulators, and why Android has none.
    let listed = mobile.run(CHAT, "list", json!({})).await.json();
    assert_eq!(listed["devices"].as_array().unwrap().len(), 5);
    assert_eq!(listed["unavailable"][0]["platform"], "android");
    assert_eq!(
        listed["unavailable"][0]["reason"],
        "no Android SDK; install Android Studio, or set ANDROID_HOME to the SDK's directory"
    );
    // As text, a row per simulator, its id first as open takes it, its name
    // last; why Android has none is a warning.
    let text = mobile.run_text(CHAT, "list", json!({})).await;
    let table = String::from_utf8(text.stdout).unwrap();
    assert_eq!(
        table,
        "ID           PLATFORM  OS        STATE      HANDLE  MODEL          NAME\n\
         UDID-BOOTED  ios       iOS 26.5  booted     -       iPhone 17 Pro  iPhone 17 Pro\n\
         UDID-1       ios       iOS 26.5  shut down  -       iPhone 17      iPhone 17\n\
         UDID-2       ios       iOS 26.5  shut down  -       iPhone 16e     iPhone 16e\n\
         UDID-3       ios       iOS 26.5  shut down  -       iPhone Air     iPhone Air\n\
         UDID-4       ios       iOS 26.5  shut down  -       iPad Air       iPad Air\n"
    );
    assert_eq!(text.stderr, "demi mobile list: warning: no Android SDK; install Android Studio, or set ANDROID_HOME to the SDK's directory\n");

    // open: boots a simulator that was shut down; the screenshot's long side is 1568.
    let opened = mobile.run(CHAT, "open", json!({ "device": "iPhone 17" })).await.json();
    let device = opened["device"].as_str().unwrap().to_owned();
    assert_eq!(device, "m1");
    assert_eq!((opened["width"].as_u64(), opened["height"].as_u64()), (Some(723), Some(1568)));
    assert_eq!(opened["booted"], true);
    assert!(mobile.log().contains(&"boot UDID-1".to_owned()));
    let fake = mobile.driver.device("UDID-1");
    fake.show(form());

    // screenshot: the PNG on stdout, or a file with its size.
    let png = mobile.run(CHAT, "screenshot", json!({ "device": device })).await;
    assert_eq!(png.exit_code, 0, "{}", png.stderr);
    assert!(png.stdout.starts_with(b"\x89PNG"));
    let saved = mobile
        .run(CHAT, "screenshot", json!({ "device": device, "output": "shot.png" }))
        .await
        .json();
    assert_eq!((saved["width"].as_u64(), saved["height"].as_u64(), saved["settled"].as_bool()), (Some(723), Some(1568), Some(true)));
    assert!(mobile.root.path().join("shot.png").exists());

    // inspect: frames in screenshot pixels, the keyboard as one element.
    let inspected = mobile.run(CHAT, "inspect", json!({ "device": device })).await.json();
    let elements = &inspected["elements"];
    assert_eq!(labels(elements), ["Form", "Settings", "Name", "Keyboard"]);
    let button = &elements[0]["children"][0];
    assert_eq!(button["frame"], json!({ "x": 184, "y": 368, "width": 147, "height": 74 }));
    let settings = reference(elements, "Settings");
    let keys = mobile
        .run(CHAT, "inspect", json!({ "device": device, "keyboard": true }))
        .await
        .json();
    assert_eq!(labels(&keys["elements"]), ["Form", "Settings", "Name", "q", "w"]);

    // tap: a ref from the last inspect taps its centre, a stale one is refused.
    let stale = mobile.run(CHAT, "tap", json!({ "device": device, "at": [settings] })).await;
    assert_eq!(stale.failure().0, "stale_ref");
    let settings = reference(&keys["elements"], "Settings");
    let tapped = mobile.run(CHAT, "tap", json!({ "device": device, "at": [settings] })).await.json();
    assert_eq!(tapped["point"], json!({ "x": 258, "y": 405 }));
    assert_eq!(tapped["element"]["label"], "Settings");
    mobile.run(CHAT, "tap", json!({ "device": device, "at": ["723", "1568"] })).await.json();
    let outside = mobile.run(CHAT, "tap", json!({ "device": device, "at": ["724", "10"] })).await;
    assert_eq!(outside.failure().0, "invalid_input");

    // swipe, type, key.
    mobile.run(CHAT, "swipe", json!({ "device": device, "at": ["0", "0", "723", "1568"] })).await.json();
    mobile.run(CHAT, "type", json!({ "device": device, "text": "hello" })).await.json();
    let name = reference(&keys["elements"], "Name");
    mobile
        .run(CHAT, "type", json!({ "device": device, "text": "Ann", "into": name, "replace": true }))
        .await
        .json();
    mobile.run(CHAT, "key", json!({ "device": device, "key": "home" })).await.json();

    // rotate: the screenshot turns, and pixels follow it.
    let rotated = mobile
        .run(CHAT, "rotate", json!({ "device": device, "orientation": "landscape" }))
        .await
        .json();
    assert_eq!((rotated["width"].as_u64(), rotated["height"].as_u64()), (Some(1568), Some(723)));
    mobile.run(CHAT, "tap", json!({ "device": device, "at": ["1568", "723"] })).await.json();

    // appearance: dark, then the other one.
    let dark = mobile.run(CHAT, "appearance", json!({ "device": device, "appearance": "dark" })).await.json();
    assert_eq!(dark["appearance"], "dark");
    let toggled = mobile.run(CHAT, "appearance", json!({ "device": device, "appearance": "toggle" })).await.json();
    assert_eq!(toggled["appearance"], "light");

    // wait: an element with the text is there; one that never leaves times out.
    let found = mobile.run(CHAT, "wait", json!({ "device": device, "text": "Sett" })).await.json();
    assert_eq!(found["element"]["label"], "Settings");
    let shown = mobile.run(CHAT, "inspect", json!({ "device": device })).await.json();
    let still = reference(&shown["elements"], "Settings");
    let waited = mobile.run(CHAT, "wait", json!({ "device": device, "gone": still, "timeout": 2 })).await;
    assert_eq!(waited.failure().0, "timeout");

    // record: start, then stop into a file.
    mobile.run(CHAT, "record", json!({ "device": device, "action": "start" })).await.json();
    let stopped = mobile
        .run(CHAT, "record", json!({ "device": device, "action": "stop", "output": "clip.mp4" }))
        .await
        .json();
    assert_eq!(stopped["durationMs"], 1500);
    assert_eq!(std::fs::read(mobile.root.path().join("clip.mp4")).unwrap(), b"fake mp4");
    let again = mobile.run(CHAT, "record", json!({ "device": device, "action": "stop" })).await;
    assert_eq!(again.failure().0, "not_recording");

    // close: Demi booted it, so it shuts down.
    let closed = mobile.run(CHAT, "close", json!({ "device": device })).await.json();
    assert_eq!(closed["shutDown"], true);

    let device_log: Vec<String> = mobile.log().into_iter().filter(|line| line.starts_with("UDID-1: ") && !line.ends_with("screenshot")).collect();
    assert_eq!(
        device_log,
        [
            "UDID-1: tap 140.0,220.0 for 0 s",
            "UDID-1: tap 393.0,852.0 for 0 s",
            "UDID-1: swipe 0.0,0.0 to 393.0,852.0",
            "UDID-1: text \"hello\"",
            "UDID-1: set 196.5,322.0 \"Ann\"",
            "UDID-1: press Home",
            "UDID-1: orient landscape-left",
            "UDID-1: tap 852.0,393.0 for 0 s",
            "UDID-1: appearance dark",
            "UDID-1: appearance light",
            "UDID-1: record start",
            "UDID-1: record stop",
            "UDID-1: detach",
        ]
    );
    assert_eq!(mobile.log().last().unwrap(), "shutdown UDID-1");
}

/// A screen that keeps changing is taken as it is after 1.5 s, and the
/// command says so.
#[tokio::test(start_paused = true)]
async fn a_screen_still_changing_is_taken_after_the_settle_limit() {
    let mobile = Mobile::start().await;
    let device = mobile.open(CHAT, "iPhone 17").await;
    let fake = mobile.driver.device("UDID-1");
    let drawing = tokio::spawn(async move {
        loop {
            fake.draw();
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
    let shot = mobile
        .run(CHAT, "screenshot", json!({ "device": device, "output": "busy.png" }))
        .await;
    drawing.abort();
    assert_eq!(shot.json()["settled"], false);
    assert!(
        shot.stderr.contains("demi mobile screenshot: the screen was still changing after 1.5 s"),
        "{}",
        shot.stderr
    );
}

/// A conversation has at most four devices open on a Host.
#[tokio::test(start_paused = true)]
async fn a_fifth_device_is_refused() {
    let mobile = Mobile::start().await;
    for name in ["iPhone 17 Pro", "iPhone 17", "iPhone 16e", "iPhone Air"] {
        mobile.open(CHAT, name).await;
    }
    let fifth = mobile.run(CHAT, "open", json!({ "device": "iPad Air" })).await;
    assert_eq!(
        fifth.failure(),
        (
            "too_many_devices".to_owned(),
            "this conversation has 4 devices open on this Host, the most it may; close one first".to_owned()
        )
    );
    // Another conversation still opens one.
    mobile.open(OTHER, "iPad Air").await;
}

/// A device open in one conversation is refused to another, which sees it
/// as open elsewhere.
#[tokio::test(start_paused = true)]
async fn a_device_open_elsewhere_is_refused() {
    let mobile = Mobile::start().await;
    mobile.open(CHAT, "iPhone 17 Pro").await;
    let refused = mobile.run(OTHER, "open", json!({ "device": "iPhone 17 Pro" })).await;
    assert_eq!(
        refused.failure(),
        (
            "device_busy".to_owned(),
            "iPhone 17 Pro is open in another conversation; close it there first".to_owned()
        )
    );
    let listed = mobile.run(OTHER, "list", json!({})).await.json();
    let pro = &listed["devices"][0];
    assert_eq!((pro["name"].as_str(), pro["elsewhere"].as_bool(), pro.get("open")), (Some("iPhone 17 Pro"), Some(true), None));
    // The other conversation's handles name none of this one's devices.
    let foreign = mobile.run(OTHER, "screenshot", json!({ "device": "m1" })).await;
    assert_eq!(foreign.failure().0, "device_not_found");
}

/// A name no simulator has says why the Host runs no Android emulators.
#[tokio::test(start_paused = true)]
async fn an_unknown_device_names_what_the_host_cannot_run() {
    let mobile = Mobile::start().await;
    let missing = mobile.run(CHAT, "open", json!({ "device": "Pixel 9" })).await;
    let (code, message) = missing.failure();
    assert_eq!(code, "device_not_found");
    assert!(message.contains("no Android SDK; install Android Studio, or set ANDROID_HOME to the SDK's directory"), "{message}");
}

/// A name two simulators share is refused, naming both by their ids, so
/// the agent opens the one it means; an id opens it. Planted defect this
/// catches: one of them chosen silently. One fake driver, milliseconds.
#[tokio::test]
async fn a_name_two_simulators_share_is_refused() {
    let home = tempfile::tempdir().unwrap();
    let driver = demi_mobile::testing::FakeDriver::ios(vec![
        demi_mobile::testing::FakeDriver::simulator("UDID-OLD", "iPhone 17", PowerState::ShutDown),
        demi_mobile::testing::FakeDriver::simulator("UDID-NEW", "iPhone 17", PowerState::Booted),
    ]);
    let mobile = Mobile::start_on(driver, &home.path().join("mobile"), None).await;
    let refused = mobile.run(CHAT, "open", json!({ "device": "iPhone 17" })).await;
    let (code, message) = refused.failure();
    assert_eq!(code, "ambiguous_device");
    assert!(message.contains("UDID-OLD") && message.contains("UDID-NEW"), "{message}");
    assert!(!mobile.log().iter().any(|line| line.starts_with("boot") || line.starts_with("attach")));
    mobile.open(CHAT, "UDID-OLD").await;
}
