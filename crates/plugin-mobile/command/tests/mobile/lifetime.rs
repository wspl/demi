//! How long a device stays open (`crates/plugin-mobile/command/src/devices.rs`): until
//! someone closes it, its conversation is released, or it is gone, across the
//! service's own restarts; one Demi booted then shuts down, one the user
//! booted is only let go.

use std::time::Duration;

use serde_json::json;

use demi_mobile::testing::FakeDriver;
use serde_json::Value;

use crate::{CHAT, Mobile, OTHER, simulators};

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

fn shut_down(mobile: &Mobile, native_id: &str) -> bool {
    mobile.log().contains(&format!("shutdown {native_id}"))
}

/// A device nobody watches and no command uses stays open, and its simulator
/// booted, however long it is left; its commands work after the wait.
#[tokio::test(start_paused = true)]
async fn nothing_closes_a_device_left_unwatched_and_unused() {
    let mobile = Mobile::start().await;
    let device = mobile.open(CHAT, "iPhone 17").await;
    let user = mobile.open(CHAT, "iPhone 17 Pro").await;
    tokio::time::sleep(DAY).await;
    assert!(!shut_down(&mobile, "UDID-1"), "{:?}", mobile.log());
    assert!(!shut_down(&mobile, "UDID-BOOTED"), "{:?}", mobile.log());
    mobile.run(CHAT, "key", json!({ "device": device, "key": "home" })).await.json();
    mobile.run(CHAT, "key", json!({ "device": user, "key": "home" })).await.json();
    let listed = mobile.run(CHAT, "list", json!({})).await.json();
    let open: Vec<&str> = listed["devices"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|each| each["open"].as_str())
        .collect();
    assert_eq!(open.len(), 2, "{listed}");
}

/// A simulator the user booted is never shut down: not when its conversation closes.
#[tokio::test(start_paused = true)]
async fn a_device_the_user_booted_is_only_let_go() {
    let mobile = Mobile::start().await;
    let device = mobile.open(CHAT, "iPhone 17 Pro").await;
    mobile.open(CHAT, "iPhone 17").await;
    mobile.run(CHAT, "key", json!({ "device": device, "key": "home" })).await.json();
    mobile.release(CHAT).await;
    assert!(mobile.log().contains(&"UDID-BOOTED: detach".to_owned()));
    assert!(shut_down(&mobile, "UDID-1"));
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

/// The record's content, as the file holds it.
fn kept_file(mobile: &std::path::Path) -> Value {
    match std::fs::read(mobile.join("kept.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap(),
        Err(_) => json!({ "simulators": [], "devices": [] }),
    }
}

/// What the record holds under `field`: each native id.
fn recorded_in(mobile: &std::path::Path, field: &str) -> Vec<String> {
    kept_file(mobile)[field]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["nativeId"].as_str().unwrap().to_owned())
        .collect()
}

/// The simulators Demi booted, as the record holds them.
fn recorded(mobile: &std::path::Path) -> Vec<String> {
    recorded_in(mobile, "simulators")
}

/// The devices open, as the record holds them.
fn kept_open(mobile: &std::path::Path) -> Vec<String> {
    recorded_in(mobile, "devices")
}

/// A field of the device the record holds for `native_id`.
fn kept_field(mobile: &std::path::Path, native_id: &str, field: &str) -> String {
    kept_file(mobile)["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["nativeId"] == native_id)
        .unwrap_or_else(|| panic!("the record holds no {native_id}"))[field]
        .as_str()
        .unwrap()
        .to_owned()
}

/// How often the driver attached `native_id` for `conversation`.
fn attaches(mobile: &Mobile, native_id: &str, conversation: &str) -> usize {
    let line = format!("attach {native_id} for {conversation}");
    mobile.log().iter().filter(|each| **each == line).count()
}

/// A service that is killed and starts again keeps every open device of
/// every conversation under its handle: once a conversation reaches the
/// service, the list says its devices are open, and the agent's command on
/// the old handle works, as do the others. A conversation that has not
/// reached the service holds none of it. Planted defect this catches: the
/// registry kept in memory only, which starts empty. One fake driver,
/// milliseconds.
#[tokio::test(start_paused = true)]
async fn a_restart_keeps_every_open_device_and_its_handle() {
    let home = tempfile::tempdir().unwrap();
    let mobile = home.path().join("mobile");
    let driver = FakeDriver::ios(simulators());
    let first = Mobile::start_on(driver.clone(), &mobile, None).await;
    let one = first.open(CHAT, "iPhone 17").await;
    let two = first.open(CHAT, "iPhone 16e").await;
    let other = first.open(OTHER, "iPhone Air").await;
    first.kill();

    let second = Mobile::start_on(driver.clone(), &mobile, None).await;
    let listed = second.run(CHAT, "list", json!({})).await.json();
    let device = |listed: &Value, native_id: &str| {
        listed["devices"]
            .as_array()
            .unwrap()
            .iter()
            .find(|each| each["nativeId"] == native_id)
            .unwrap()
            .clone()
    };
    assert_eq!(device(&listed, "UDID-1")["open"], json!(one), "{listed}");
    assert_eq!(device(&listed, "UDID-2")["open"], json!(two), "{listed}");
    // Another conversation's device is the service's once that conversation
    // reaches it, and not before.
    assert_eq!(device(&listed, "UDID-3").get("open"), None, "{listed}");

    second.run(CHAT, "key", json!({ "device": one, "key": "home" })).await.json();
    second.run(CHAT, "inspect", json!({ "device": two })).await.json();
    second.run(OTHER, "key", json!({ "device": other, "key": "home" })).await.json();
    let listed = second.run(CHAT, "list", json!({})).await.json();
    assert_eq!(device(&listed, "UDID-3")["elsewhere"], true, "{listed}");
    let log = second.log();
    assert!(!log.contains(&"shutdown UDID-1".to_owned()), "{log:?}");
    assert_eq!(kept_open(&mobile).len(), 3);
    // Still Demi's: its conversation's end shuts it down.
    second.release(CHAT).await;
    assert!(shut_down(&second, "UDID-1") && shut_down(&second, "UDID-2"));
    assert!(!shut_down(&second, "UDID-3"));
    assert_eq!(kept_open(&mobile), ["UDID-3"]);
    assert_eq!(recorded(&mobile), ["UDID-3"]);
}

/// A device whose simulator was deleted while the service was away closes as
/// lost: its handle is not open any more, the others are, and the record
/// forgets it. Planted defect this catches: every remembered device taken
/// back without asking whether it exists. One fake driver, milliseconds.
#[tokio::test(start_paused = true)]
async fn a_device_deleted_during_the_restart_closes_as_lost() {
    let home = tempfile::tempdir().unwrap();
    let mobile = home.path().join("mobile");
    let driver = FakeDriver::ios(simulators());
    let first = Mobile::start_on(driver.clone(), &mobile, None).await;
    let gone = first.open(CHAT, "iPhone 17").await;
    let kept = first.open(CHAT, "iPhone 16e").await;
    first.kill();
    driver.delete("UDID-1");

    let second = Mobile::start_on(driver.clone(), &mobile, None).await;
    let lost = second.run(CHAT, "screenshot", json!({ "device": gone })).await;
    assert_eq!(lost.failure().0, "device_not_found");
    second.run(CHAT, "key", json!({ "device": kept, "key": "home" })).await.json();
    assert_eq!(kept_open(&mobile), ["UDID-2"]);
}

/// A device whose simulator was shut down while the service was away is lost
/// too: nothing is left to show.
#[tokio::test(start_paused = true)]
async fn a_device_shut_down_during_the_restart_closes_as_lost() {
    let home = tempfile::tempdir().unwrap();
    let mobile = home.path().join("mobile");
    let driver = FakeDriver::ios(simulators());
    let first = Mobile::start_on(driver.clone(), &mobile, None).await;
    let device = first.open(CHAT, "iPhone 17").await;
    first.kill();
    driver.shutdown_now("UDID-1");

    let second = Mobile::start_on(driver.clone(), &mobile, None).await;
    let lost = second.run(CHAT, "screenshot", json!({ "device": device })).await;
    assert_eq!(lost.failure().0, "device_not_found");
    assert!(kept_open(&mobile).is_empty());
}

/// The restored device that the user booted again while the service was away
/// is the user's: it is taken back under its handle, and its conversation's
/// end only lets it go.
#[tokio::test(start_paused = true)]
async fn a_simulator_the_user_booted_again_is_the_users_after_a_restart() {
    let home = tempfile::tempdir().unwrap();
    let mobile = home.path().join("mobile");
    let driver = FakeDriver::ios(simulators());
    let first = Mobile::start_on(driver.clone(), &mobile, None).await;
    let device = first.open(CHAT, "iPhone 17").await;
    first.kill();
    driver.reboot("UDID-1");

    let second = Mobile::start_on(driver.clone(), &mobile, None).await;
    second.run(CHAT, "key", json!({ "device": device, "key": "home" })).await.json();
    second.release(CHAT).await;
    assert!(!shut_down(&second, "UDID-1"));
}

/// A conversation released before it reached the service again releases the
/// devices the record holds for it too.
#[tokio::test(start_paused = true)]
async fn a_release_after_a_restart_reaches_the_devices_the_record_holds() {
    let home = tempfile::tempdir().unwrap();
    let mobile = home.path().join("mobile");
    let driver = FakeDriver::ios(simulators());
    let first = Mobile::start_on(driver.clone(), &mobile, None).await;
    first.open(CHAT, "iPhone 17").await;
    first.kill();

    let second = Mobile::start_on(driver.clone(), &mobile, None).await;
    second.release(CHAT).await;
    assert!(shut_down(&second, "UDID-1"));
    assert!(kept_open(&mobile).is_empty());
    assert!(recorded(&mobile).is_empty());
    assert_eq!(attaches(&second, "UDID-1", CHAT), 1, "a released device was attached again");
}

/// The runner stopping the program, as an update does, lets its devices go
/// and shuts none down, so the app under test keeps running; the record
/// still holds them.
#[tokio::test(start_paused = true)]
async fn a_program_the_runner_stops_shuts_nothing_down() {
    let home = tempfile::tempdir().unwrap();
    let mobile = home.path().join("mobile");
    let driver = FakeDriver::ios(simulators());
    let first = Mobile::start_on(driver.clone(), &mobile, None).await;
    let device = first.open(CHAT, "iPhone 17").await;
    first.stop().await;
    let log = driver.log.lock().unwrap().clone();
    assert!(log.contains(&"UDID-1: detach".to_owned()), "{log:?}");
    assert!(!log.contains(&"shutdown UDID-1".to_owned()), "{log:?}");
    assert_eq!(recorded(&mobile), ["UDID-1"]);
    assert_eq!(kept_open(&mobile), ["UDID-1"]);

    let second = Mobile::start_on(driver.clone(), &mobile, None).await;
    second.run(CHAT, "key", json!({ "device": device, "key": "home" })).await.json();
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
/// says it did not shut down, and the record keeps it. Planted defect this
/// catches: ownership dropped with the failed shutdown. One fake driver.
#[tokio::test(start_paused = true)]
async fn a_failed_shutdown_keeps_the_simulator_demis() {
    let mobile = Mobile::start().await;
    let device = mobile.open(CHAT, "iPhone 17").await;
    mobile.driver.fail_shutdowns(Some("the simulator is busy"));
    let closed = mobile.run(CHAT, "close", json!({ "device": device })).await.json();
    assert_eq!(closed["shutDown"], false, "the close says the simulator shut down");
    assert_eq!(recorded(&mobile.home().join("mobile")), ["UDID-1"]);
    assert!(kept_open(&mobile.home().join("mobile")).is_empty());
}

/// A device whose helper or picture stream ends while its simulator runs
/// loses its connection, not its device: it keeps its handle, its place in
/// the record and its tab, is attached again, and its commands work. Planted
/// defect this catches: the device closed as lost with its handle. One fake
/// driver, a paused clock.
#[tokio::test(start_paused = true)]
async fn a_device_whose_connection_ended_keeps_its_handle_and_reconnects() {
    let mobile = Mobile::start().await;
    let device = mobile.open(CHAT, "iPhone 17").await;
    let mut revisions = mobile.service.devices().revisions();
    revisions.borrow_and_update();
    mobile.driver.device("UDID-1").lose();
    revisions.changed().await.unwrap();
    // While it reconnects it is still open, under its handle.
    let listed = mobile.run(CHAT, "list", json!({})).await.json();
    let listed_device = listed["devices"].as_array().unwrap().iter().find(|each| each["nativeId"] == "UDID-1").unwrap().clone();
    assert_eq!(listed_device["open"], json!(device), "{listed_device}");
    mobile.run(CHAT, "key", json!({ "device": device, "key": "home" })).await.json();
    assert_eq!(attaches(&mobile, "UDID-1", CHAT), 2, "{:?}", mobile.log());
    assert!(!shut_down(&mobile, "UDID-1"));
    assert_eq!(kept_open(&mobile.home().join("mobile")), ["UDID-1"]);
}

/// A device whose simulator went away closes as lost, whatever its helper
/// did: its handle is not open any more, and the simulator, which may still
/// be Demi's, stays in the record that says so. Planted defect this
/// catches: ownership dropped with the device. One fake driver.
#[tokio::test(start_paused = true)]
async fn a_device_whose_simulator_went_away_closes_as_lost() {
    let mobile = Mobile::start().await;
    let device = mobile.open(CHAT, "iPhone 17").await;
    let mut revisions = mobile.service.devices().revisions();
    revisions.borrow_and_update();
    mobile.driver.shutdown_now("UDID-1");
    mobile.driver.device("UDID-1").lose();
    revisions.changed().await.unwrap();
    let lost = mobile.run(CHAT, "screenshot", json!({ "device": device })).await;
    assert_eq!(lost.failure().0, "device_not_found");
    assert_eq!(recorded(&mobile.home().join("mobile")), ["UDID-1"]);
    assert!(kept_open(&mobile.home().join("mobile")).is_empty());
}

/// Two versions of the service run at once, as when a runner's update leaves
/// the older one holding the conversations it has. The record is shared
/// safely: a change reads the file fresh and writes only its own, so a
/// service that read the file long ago brings back nothing a release removed.
/// Planted defect this catches: each service writing the snapshot it read at
/// its start. Two services over one record, milliseconds.
#[tokio::test(start_paused = true)]
async fn a_service_writes_only_what_it_changed_to_the_shared_record() {
    let home = tempfile::tempdir().unwrap();
    let mobile = home.path().join("mobile");
    let driver = FakeDriver::ios(simulators());
    let older = Mobile::start_on(driver.clone(), &mobile, None).await;
    older.open(CHAT, "iPhone 17").await;
    // The newer version starts with the record holding the device.
    let newer = Mobile::start_on(driver.clone(), &mobile, None).await;
    older.release(CHAT).await;
    newer.open(OTHER, "iPhone 16e").await;
    assert_eq!(kept_open(&mobile), ["UDID-2"], "{}", kept_file(&mobile));
    assert_eq!(recorded(&mobile), ["UDID-2"], "{}", kept_file(&mobile));
}

/// Writers in many threads and processes lose no change: each takes the
/// record's lock, reads the file fresh and writes a file of its own. Planted
/// defects this catches: a snapshot written whole, and one fixed temporary
/// file that two writers rename in turn. Eight threads over two handles of
/// one record, milliseconds.
#[test]
fn concurrent_writers_of_the_record_lose_no_change() {
    use demi_command_package_mobile_protocol::mobile::{DeviceId, MobilePlatform};
    use demi_mobile::kept::{Kept, Opened};

    let home = tempfile::tempdir().unwrap();
    let mobile = home.path().join("mobile");
    let writers = [Kept::load(Some(&mobile)), Kept::load(Some(&mobile))];
    std::thread::scope(|scope| {
        for thread in 0..8usize {
            let kept = &writers[thread % 2];
            scope.spawn(move || {
                for index in 0..25usize {
                    kept.opened(Opened {
                        handle: DeviceId::numbered((thread * 100 + index + 1) as u64),
                        conversation: format!("chat-{thread}"),
                        platform: MobilePlatform::Ios,
                        native_id: format!("UDID-{thread}-{index}"),
                        owner: format!("service-{thread}"),
                    });
                }
            });
        }
    });
    let (_, devices) = Kept::load(Some(&mobile)).snapshot();
    assert_eq!(devices.len(), 200, "changes were lost");
    let leftovers: Vec<_> = std::fs::read_dir(&mobile)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".partial"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

/// A conversation that reaches a service takes over its own devices from the
/// record, and no other conversation's: the service that held them lets go
/// of them when it next notices, shuts nothing down, and nothing is held
/// twice. Planted defect this catches: every recorded device taken back at
/// the service's start, whoever's. Two services over one record, milliseconds.
#[tokio::test(start_paused = true)]
async fn a_conversation_reaching_a_service_takes_over_only_its_own_devices() {
    let home = tempfile::tempdir().unwrap();
    let mobile = home.path().join("mobile");
    let driver = FakeDriver::ios(simulators());
    let older = Mobile::start_on(driver.clone(), &mobile, None).await;
    let one = older.open(CHAT, "iPhone 17").await;
    let other = older.open(OTHER, "iPhone 16e").await;
    let owner_before = kept_field(&mobile, "UDID-1", "owner");

    let newer = Mobile::start_on(driver.clone(), &mobile, None).await;
    newer.run(CHAT, "key", json!({ "device": one, "key": "home" })).await.json();
    let owner_after = kept_field(&mobile, "UDID-1", "owner");
    assert_ne!(owner_after, owner_before, "the conversation that reached the service kept its devices with the other");
    assert_eq!(kept_field(&mobile, "UDID-2", "owner"), owner_before, "another conversation's device changed hands");
    assert_eq!(attaches(&newer, "UDID-2", OTHER), 1, "another conversation's device was taken back");

    // The older service lets go of what it lost, without shutting it down,
    // although nothing asks it anything.
    tokio::time::sleep(Duration::from_secs(5)).await;
    let detaches = newer.log().iter().filter(|line| *line == "UDID-1: detach").count();
    assert_eq!(detaches, 1, "the older service kept the device it lost: {:?}", newer.log());
    let refused = older.run(CHAT, "key", json!({ "device": one, "key": "home" })).await;
    assert_eq!(refused.failure().0, "device_not_found");
    let detaches = newer.log().iter().filter(|line| *line == "UDID-1: detach").count();
    assert_eq!(detaches, 1, "{:?}", newer.log());
    older.release(CHAT).await;
    assert!(!shut_down(&newer, "UDID-1"), "the older service shut down what the newer holds");
    older.run(OTHER, "key", json!({ "device": other, "key": "home" })).await.json();
    // The service that holds it ends it.
    newer.release(CHAT).await;
    assert!(shut_down(&newer, "UDID-1"));
}

/// The record's devices of a conversation that is no longer on this Host, as
/// one deleted while the Host was offline, are released as the service
/// starts: a simulator Demi booted for it shuts down, and the record forgets
/// it. Planted defect this catches: the device taken back, shown as held
/// elsewhere to everyone and closable by nobody. One fake driver,
/// milliseconds.
#[tokio::test(start_paused = true)]
async fn a_conversation_that_is_gone_is_released_as_the_service_starts() {
    let home = tempfile::tempdir().unwrap();
    let mobile = home.path().join("mobile");
    let driver = FakeDriver::ios(simulators());
    let first = Mobile::start_on(driver.clone(), &mobile, None).await;
    first.open(CHAT, "iPhone 17").await;
    first.kill();
    // The runner removed the conversation's scratch directory as it connected.
    std::fs::remove_dir_all(home.path().join("scratch").join(CHAT)).unwrap();

    let second = Mobile::start_on(driver.clone(), &mobile, None).await;
    tokio::time::timeout(Duration::from_secs(10), async {
        while !shut_down(&second, "UDID-1") {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the simulator Demi booted for the deleted conversation shuts down");
    assert!(kept_open(&mobile).is_empty());
    assert!(recorded(&mobile).is_empty());
    second.open(OTHER, "iPhone 17").await;
}

/// A conversation that exists but never reaches the service again, as one
/// archived while the Host was offline, holds no simulator: another
/// conversation opens it, and when the first reaches the service it finds
/// the device elsewhere. Planted defect this catches: the device shown as
/// held elsewhere and refused as busy, closable by nobody. One fake driver,
/// milliseconds.
#[tokio::test(start_paused = true)]
async fn a_conversation_that_never_reaches_the_service_holds_no_simulator() {
    let home = tempfile::tempdir().unwrap();
    let mobile = home.path().join("mobile");
    let driver = FakeDriver::ios(simulators());
    let first = Mobile::start_on(driver.clone(), &mobile, None).await;
    first.open(CHAT, "iPhone 17").await;
    first.kill();

    let second = Mobile::start_on(driver.clone(), &mobile, None).await;
    second.open(OTHER, "iPhone 17").await;
    assert_eq!(kept_field(&mobile, "UDID-1", "conversation"), OTHER);
    let listed = second.run(CHAT, "list", json!({})).await.json();
    let device = listed["devices"].as_array().unwrap().iter().find(|each| each["nativeId"] == "UDID-1").unwrap().clone();
    assert_eq!((device.get("open"), &device["elsewhere"]), (None, &json!(true)), "{device}");
}

/// A listing that fails or a driver that is unavailable is not proof the
/// device is gone: the device keeps its entry and its handle, the take-back
/// tries again with growing waits, and the command that waits for it goes
/// through once the Host answers. Planted defect this catches: every device of
/// the platform closed for good at the first failed list. One fake driver, a
/// paused clock.
#[tokio::test(start_paused = true)]
async fn a_listing_that_fails_keeps_the_device_and_tries_again() {
    let home = tempfile::tempdir().unwrap();
    let mobile = home.path().join("mobile");
    let driver = FakeDriver::ios(simulators());
    let first = Mobile::start_on(driver.clone(), &mobile, None).await;
    let device = first.open(CHAT, "iPhone 17").await;
    first.kill();
    driver.fail_lists(Some("simctl did not answer"));

    let second = Mobile::start_on(driver.clone(), &mobile, None).await;
    let command = second.run(CHAT, "key", json!({ "device": device, "key": "home" }));
    let heal = async {
        tokio::time::sleep(Duration::from_secs(10)).await;
        assert_eq!(kept_open(&mobile), ["UDID-1"], "the device was closed for a failed list");
        driver.fail_lists(None);
    };
    let (answer, ()) = tokio::join!(command, heal);
    answer.json();
    assert_eq!(kept_open(&mobile), ["UDID-1"]);
}

/// A simulator still booting when the service starts is attached after its
/// boot ends, not at once: the iOS helper refuses one that is not booted,
/// and an Android emulator has no connection yet. Planted defect this
/// catches: the attach at once, refused, and the device closed as lost. One
/// fake driver, a paused clock.
#[tokio::test(start_paused = true)]
async fn a_simulator_still_booting_is_attached_after_its_boot() {
    let home = tempfile::tempdir().unwrap();
    let mobile = home.path().join("mobile");
    let driver = FakeDriver::ios(simulators());
    let first = Mobile::start_on(driver.clone(), &mobile, None).await;
    let device = first.open(CHAT, "iPhone 17").await;
    first.kill();
    driver.start_booting("UDID-1");

    let second = Mobile::start_on(driver.clone(), &mobile, None).await;
    let command = second.run(CHAT, "key", json!({ "device": device, "key": "home" }));
    let finish = async {
        tokio::time::sleep(Duration::from_secs(5)).await;
        driver.finish_boot("UDID-1");
    };
    let (answer, ()) = tokio::join!(command, finish);
    answer.json();
    let log = second.log();
    let finished = log.iter().position(|line| line == "UDID-1 finishes booting").unwrap();
    let attached: Vec<usize> = log
        .iter()
        .enumerate()
        .filter(|(_, line)| **line == format!("attach UDID-1 for {CHAT}"))
        .map(|(index, _)| index)
        .collect();
    assert_eq!(attached.len(), 2, "{log:?}");
    assert!(attached[1] > finished, "the device was attached before its boot ended: {log:?}");
}

/// A device that cannot be attached yet is tried again, with its entry and
/// its handle kept: a failed attach is not a loss. Planted defect this
/// catches: the device closed at the first refusal. One fake driver, a paused
/// clock.
#[tokio::test(start_paused = true)]
async fn a_failed_attach_is_tried_again_and_is_not_a_loss() {
    let home = tempfile::tempdir().unwrap();
    let mobile = home.path().join("mobile");
    let driver = FakeDriver::ios(simulators());
    let first = Mobile::start_on(driver.clone(), &mobile, None).await;
    let device = first.open(CHAT, "iPhone 17").await;
    first.kill();
    driver.fail_attaches(Some("the helper is not ready"));

    let second = Mobile::start_on(driver.clone(), &mobile, None).await;
    let command = second.run(CHAT, "key", json!({ "device": device, "key": "home" }));
    let heal = async {
        tokio::time::sleep(Duration::from_secs(5)).await;
        assert_eq!(kept_open(&mobile), ["UDID-1"], "the device was closed for a refused attach");
        driver.fail_attaches(None);
    };
    let (answer, ()) = tokio::join!(command, heal);
    answer.json();
    assert!(attaches(&second, "UDID-1", CHAT) >= 4, "{:?}", second.log());
}

/// A command that waits for a device being taken back ends when it is
/// cancelled, and the take-back goes on: the SDK's deadline for a cancelled
/// handler never retires the service for everyone. Planted defect this
/// catches: the wait that ignores the cancellation. A held attach, a paused
/// clock.
#[tokio::test(start_paused = true)]
async fn cancelling_a_command_that_waits_for_a_take_back_ends_it() {
    let home = tempfile::tempdir().unwrap();
    let mobile = home.path().join("mobile");
    let driver = FakeDriver::ios(simulators());
    let first = Mobile::start_on(driver.clone(), &mobile, None).await;
    let device = first.open(CHAT, "iPhone 17").await;
    first.kill();
    let held = driver.hold_attaches();

    let second = Mobile::start_on(driver.clone(), &mobile, None).await;
    let (mut input, output) = second
        .begin(CHAT, "key", json!({ "device": device, "key": "home" }), demi_command_protocol::CommandCaller::agent(1))
        .await;
    held.started.notified().await;
    input.cancel();
    drop(output);
    // Past the SDK's deadline for a cancelled handler, which retires the
    // whole service when the handler is still running.
    tokio::time::sleep(Duration::from_secs(8)).await;
    // The service is still there for everyone, and the take-back goes on.
    second.run(CHAT, "list", json!({})).await.json();
    held.permits.add_permits(8);
    second.run(CHAT, "key", json!({ "device": device, "key": "home" })).await.json();
}

/// A device being taken back closes at once, without waiting for the
/// take-back, and the attach that finishes afterwards is detached.
#[tokio::test(start_paused = true)]
async fn a_device_being_taken_back_closes_at_once() {
    let home = tempfile::tempdir().unwrap();
    let mobile = home.path().join("mobile");
    let driver = FakeDriver::ios(simulators());
    let first = Mobile::start_on(driver.clone(), &mobile, None).await;
    let device = first.open(CHAT, "iPhone 17").await;
    first.kill();
    let held = driver.hold_attaches();

    let second = Mobile::start_on(driver.clone(), &mobile, None).await;
    second.run(CHAT, "list", json!({})).await.json();
    held.started.notified().await;
    let closed = second.run(CHAT, "close", json!({ "device": device })).await.json();
    assert_eq!(closed["shutDown"], true, "{closed}");
    held.permits.add_permits(8);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(second.log().contains(&"UDID-1: detach".to_owned()), "{:?}", second.log());
    assert!(kept_open(&mobile).is_empty());
    let gone = second.run(CHAT, "key", json!({ "device": device, "key": "home" })).await;
    assert_eq!(gone.failure().0, "device_not_found");
}

/// Stopping and taking back share one boundary: a device a take-back attaches
/// while the service stops is detached, not added after the stop took its
/// snapshot, and the record keeps it for the next service. Planted defect
/// this catches: the stop that snapshots the open devices, then waits while
/// the take-back adds one it never detaches. A held attach, a paused clock.
#[tokio::test(start_paused = true)]
async fn a_device_attached_while_the_service_stops_is_detached() {
    let home = tempfile::tempdir().unwrap();
    let mobile = home.path().join("mobile");
    let driver = FakeDriver::ios(simulators());
    let first = Mobile::start_on(driver.clone(), &mobile, None).await;
    first.open(CHAT, "iPhone 17").await;
    first.kill();
    let held = driver.hold_attaches();

    let second = Mobile::start_on(driver.clone(), &mobile, None).await;
    second.run(CHAT, "list", json!({})).await.json();
    held.started.notified().await;
    let log = second.driver.log.clone();
    let release = async {
        // After the stop began, while it waits for the take-back.
        tokio::time::sleep(Duration::from_secs(2)).await;
        held.permits.add_permits(8);
    };
    tokio::join!(second.stop(), release);
    tokio::time::sleep(Duration::from_secs(1)).await;
    let log = log.lock().unwrap().clone();
    assert!(log.contains(&"UDID-1: detach".to_owned()), "the device the take-back attached was never detached: {log:?}");
    assert!(!log.contains(&"shutdown UDID-1".to_owned()), "{log:?}");
    assert_eq!(kept_open(&mobile), ["UDID-1"]);
}
