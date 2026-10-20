//! How long a device stays open (`crates/command-package-mobile/src/devices.rs`): one Demi booted
//! shuts down after an hour nobody watched or used it, or when its
//! conversation closes; one the user booted is only let go.

use std::time::Duration;

use demi_command_package_mobile_protocol::mobile::DeviceId;
use demi_mobile::devices::IDLE;
use serde_json::json;

use demi_mobile::testing::FakeDriver;

use crate::{CHAT, Mobile, OTHER, simulators};

const MINUTE: Duration = Duration::from_secs(60);

fn shut_down(mobile: &Mobile, native_id: &str) -> bool {
    mobile.log().contains(&format!("shutdown {native_id}"))
}

/// An hour with no command shuts a booted device down; each command starts the hour again.
#[tokio::test(start_paused = true)]
async fn a_device_demi_booted_shuts_down_after_an_hour_unused() {
    let mobile = Mobile::start().await;
    let device = mobile.open(CHAT, "iPhone 17").await;
    tokio::time::sleep(IDLE - MINUTE).await;
    mobile.run(CHAT, "key", json!({ "device": device, "key": "home" })).await.json();
    tokio::time::sleep(IDLE - MINUTE).await;
    assert!(!shut_down(&mobile, "UDID-1"), "a command started the hour again");
    tokio::time::sleep(2 * MINUTE).await;
    assert!(shut_down(&mobile, "UDID-1"));
    let gone = mobile.run(CHAT, "screenshot", json!({ "device": device })).await;
    assert_eq!(gone.failure().0, "device_not_found");
}

/// While the user watches it, a booted device stays; the hour starts when they leave.
#[tokio::test(start_paused = true)]
async fn a_watched_device_stays_open() {
    let mobile = Mobile::start().await;
    let device = mobile.open(CHAT, "iPhone 17").await;
    let open = mobile
        .service
        .devices()
        .get(CHAT, &DeviceId::numbered(device[1..].parse().unwrap()))
        .unwrap();
    let watching = open.watched();
    tokio::time::sleep(3 * IDLE).await;
    assert!(!shut_down(&mobile, "UDID-1"));
    drop(watching);
    tokio::time::sleep(IDLE - MINUTE).await;
    assert!(!shut_down(&mobile, "UDID-1"));
    tokio::time::sleep(2 * MINUTE).await;
    assert!(shut_down(&mobile, "UDID-1"));
}

/// A simulator the user booted is never shut down: not idle, not when the conversation closes.
#[tokio::test(start_paused = true)]
async fn a_device_the_user_booted_is_only_let_go() {
    let mobile = Mobile::start().await;
    let device = mobile.open(CHAT, "iPhone 17 Pro").await;
    let booted = mobile.open(CHAT, "iPhone 17").await;
    tokio::time::sleep(3 * IDLE).await;
    assert!(shut_down(&mobile, "UDID-1"), "{booted} shut down idle");
    // The user's simulator is still open here, however long it idled.
    mobile.run(CHAT, "key", json!({ "device": device, "key": "home" })).await.json();
    mobile.release(CHAT).await;
    assert!(mobile.log().contains(&"UDID-BOOTED: detach".to_owned()));
    assert!(!shut_down(&mobile, "UDID-BOOTED"));
    let gone = mobile.run(CHAT, "screenshot", json!({ "device": device })).await;
    assert_eq!(gone.failure().0, "device_not_found");
}

/// Closing the conversation shuts down the devices Demi booted for it.
#[tokio::test(start_paused = true)]
async fn closing_the_conversation_shuts_down_what_demi_booted() {
    let mobile = Mobile::start().await;
    mobile.open(CHAT, "iPhone 17").await;
    mobile.open(CHAT, "iPhone 16e").await;
    mobile.release(CHAT).await;
    assert!(shut_down(&mobile, "UDID-1"));
    assert!(shut_down(&mobile, "UDID-2"));
    let listed = mobile.run(CHAT, "list", json!({})).await.json();
    assert!(listed["devices"].as_array().unwrap().iter().all(|each| each.get("open").is_none()));
}

/// What the simulators record holds: each native id Demi booted.
fn recorded(mobile: &std::path::Path) -> Vec<String> {
    match std::fs::read(mobile.join("booted.json")) {
        Ok(bytes) => {
            let record: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            record["simulators"]
                .as_array()
                .unwrap()
                .iter()
                .map(|entry| entry["nativeId"].as_str().unwrap().to_owned())
                .collect()
        }
        Err(_) => Vec::new(),
    }
}

/// A killed program shuts nothing down; the next one gives the simulator
/// Demi booted the hour from its start, and shuts it down after it.
#[tokio::test(start_paused = true)]
async fn a_simulator_demi_booted_outlives_the_program_and_idles_out_after_its_restart() {
    let home = tempfile::tempdir().unwrap();
    let mobile = home.path().join("mobile");
    let driver = FakeDriver::ios(simulators());
    let first = Mobile::start_on(driver.clone(), &mobile, None).await;
    first.open(CHAT, "iPhone 17").await;
    assert_eq!(recorded(&mobile), ["UDID-1"]);
    first.kill();
    tokio::time::sleep(IDLE - MINUTE).await;
    assert!(!driver.log.lock().unwrap().contains(&"shutdown UDID-1".to_owned()), "the killed program shut it down");

    let second = Mobile::start_on(driver.clone(), &mobile, None).await;
    tokio::time::sleep(IDLE - MINUTE).await;
    assert!(!shut_down(&second, "UDID-1"), "the hour starts with the new program");
    tokio::time::sleep(2 * MINUTE).await;
    assert!(shut_down(&second, "UDID-1"));
    assert!(recorded(&mobile).is_empty());
}

/// A simulator the user shut down and booted again while no program ran is
/// the user's: the next program leaves it alone and forgets it.
#[tokio::test(start_paused = true)]
async fn a_simulator_the_user_booted_again_is_left_alone_after_a_restart() {
    let home = tempfile::tempdir().unwrap();
    let mobile = home.path().join("mobile");
    let driver = FakeDriver::ios(simulators());
    let first = Mobile::start_on(driver.clone(), &mobile, None).await;
    first.open(CHAT, "iPhone 17").await;
    first.kill();
    driver.reboot("UDID-1");

    let second = Mobile::start_on(driver.clone(), &mobile, None).await;
    // The new program forgets it as it starts.
    tokio::time::sleep(MINUTE).await;
    assert!(recorded(&mobile).is_empty());
    tokio::time::sleep(3 * IDLE).await;
    assert!(!shut_down(&second, "UDID-1"));
    // Opened again, it is the user's: its conversation's end only lets it go.
    second.open(CHAT, "iPhone 17").await;
    second.release(CHAT).await;
    assert!(!shut_down(&second, "UDID-1"));
}

/// Opened again within the hour, the simulator is still Demi's: its
/// conversation's end shuts it down.
#[tokio::test(start_paused = true)]
async fn a_simulator_demi_booted_is_still_demis_when_opened_after_a_restart() {
    let home = tempfile::tempdir().unwrap();
    let mobile = home.path().join("mobile");
    let driver = FakeDriver::ios(simulators());
    let first = Mobile::start_on(driver.clone(), &mobile, None).await;
    first.open(CHAT, "iPhone 17").await;
    first.kill();

    let second = Mobile::start_on(driver.clone(), &mobile, None).await;
    tokio::time::sleep(MINUTE).await;
    second.open(OTHER, "iPhone 17").await;
    // Its conversation watches over it now, not the restart's hour.
    tokio::time::sleep(IDLE - 2 * MINUTE).await;
    second.run(OTHER, "key", json!({ "device": "m1", "key": "home" })).await.json();
    tokio::time::sleep(2 * MINUTE).await;
    assert!(!shut_down(&second, "UDID-1"));
    second.release(OTHER).await;
    assert!(shut_down(&second, "UDID-1"));
    assert!(recorded(&mobile).is_empty());
}

/// The runner stopping the program, as an update does, lets its devices go
/// and shuts none down, so the app under test keeps running.
#[tokio::test(start_paused = true)]
async fn a_program_the_runner_stops_shuts_nothing_down() {
    let home = tempfile::tempdir().unwrap();
    let mobile = home.path().join("mobile");
    let driver = FakeDriver::ios(simulators());
    let first = Mobile::start_on(driver.clone(), &mobile, None).await;
    first.open(CHAT, "iPhone 17").await;
    first.stop().await;
    let log = driver.log.lock().unwrap().clone();
    assert!(log.contains(&"UDID-1: detach".to_owned()), "{log:?}");
    assert!(!log.contains(&"shutdown UDID-1".to_owned()), "{log:?}");
    assert_eq!(recorded(&mobile), ["UDID-1"]);
}

/// An open the agent interrupts while its simulator boots still ends: the
/// simulator Demi booted is then open, and shuts down with the
/// conversation, rather than left booted and nobody's. Planted defect this
/// catches: the open run in the request, whose cancellation drops it
/// halfway. One fake driver, milliseconds.
#[tokio::test]
async fn an_open_the_agent_interrupts_while_booting_still_ends() {
    let mobile = Mobile::start().await;
    let held = mobile.driver.hold_boots();
    let (mut input, output) = mobile
        .begin(CHAT, "open", json!({ "device": "iPhone 17" }), demi_command_protocol::CommandCaller::agent(1))
        .await;
    held.started.notified().await;
    input.cancel();
    drop(output);
    let mut revisions = mobile.service.devices().revisions();
    held.permits.add_permits(1);
    tokio::time::timeout(Duration::from_secs(10), revisions.changed())
        .await
        .expect("the interrupted open ends with the simulator open")
        .unwrap();
    mobile.release(CHAT).await;
    assert!(shut_down(&mobile, "UDID-1"), "the simulator Demi booted is left booted: {:?}", mobile.log());
}

/// A simulator Demi booted whose shutdown fails stays Demi's: `close`
/// says it did not shut down and why, the record keeps it, and Demi shuts
/// it down after the hour. Planted defect this catches: ownership dropped
/// with the failed shutdown, so the simulator is left booted for good. One
/// fake driver, a paused clock.
#[tokio::test(start_paused = true)]
async fn a_failed_shutdown_keeps_the_simulator_demis() {
    let mobile = Mobile::start().await;
    let device = mobile.open(CHAT, "iPhone 17").await;
    mobile.driver.fail_shutdowns(Some("the simulator is busy"));
    let closed = mobile.run(CHAT, "close", json!({ "device": device })).await.json();
    assert_eq!(closed["shutDown"], false, "the close says the simulator shut down");
    assert_eq!(recorded(&mobile.home().join("mobile")), ["UDID-1"]);
    mobile.driver.fail_shutdowns(None);
    tokio::time::sleep(IDLE + MINUTE).await;
    assert!(shut_down(&mobile, "UDID-1"));
    assert!(recorded(&mobile.home().join("mobile")).is_empty());
}

/// A device that went away while its simulator stays booted, as when its
/// helper ends, is still Demi's to shut down: after the hour it is.
/// Planted defect this catches: ownership dropped with the device. One
/// fake driver, a paused clock.
#[tokio::test(start_paused = true)]
async fn a_device_lost_while_booted_stays_demis() {
    let mobile = Mobile::start().await;
    mobile.open(CHAT, "iPhone 17").await;
    let mut revisions = mobile.service.devices().revisions();
    revisions.borrow_and_update();
    mobile.driver.device("UDID-1").lose();
    revisions.changed().await.unwrap();
    assert_eq!(recorded(&mobile.home().join("mobile")), ["UDID-1"]);
    tokio::time::sleep(IDLE + MINUTE).await;
    assert!(shut_down(&mobile, "UDID-1"));
}
