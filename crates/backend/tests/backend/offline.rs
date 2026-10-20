//! A conversation whose Host goes offline
//! (`crates/backend-host-access/src/access.rs`,
//! `crates/backend-host-access/src/transition.rs`).
//!
//! Scenarios:
//!
//! - The agent's command on its offline primary device waits for the device
//!   and runs once it is back.
//! - A Stop ends every such wait, an earlier turn's too, and a move fails it,
//!   the command never sent.
//! - A command on an offline attached device fails at once with the device's
//!   offline error, worded for the model.
//! - A waiting command outlives a backend restart, still unsent, and a stop of
//!   it ends it.
//! - A move away from an offline device leaves the commands still running
//!   there to its runner, which reports their ends once it is back.
//! - A device whose connection drops and comes back within its wait shows
//!   online throughout, and what the work panel and the agent asked of it
//!   meanwhile goes on once its runner is back
//!   (`crates/backend-runners/src/devices.rs`). An agent's command that
//!   waited through it fails as offline once the wait ends without the
//!   runner, rather than waiting on for the device.
//!
//! The model is an Anthropic endpoint the test scripts; the devices are real
//! runners.

use std::time::Duration;

use demi_agent_tools::testing::field;
use demi_conversation_socket_protocol::{ClientFrame, ServerFrame, ShellStatus};
use demi_provider_common::testing::MockVendor;
use demi_shared_types::{CommandId, SessionPhase};
use demi_web_api_protocol::conversations::ConversationStatus;

use crate::conversations::{anthropic_at, create, summary};
use crate::lifetime::{Network, dropped, until_requested};
use crate::support::elapse;
use demi_backend_user_shard::shard::ShardPlacement;
use crate::support::{Harness, Paired, Session, TestBackend};
use demi_backend_user_shard::holds::HelloStep;
use demi_web_api_protocol::devices::DeviceState;
use demi_web_api_protocol::state::SyncEvent;
use crate::work::{Driven, resident, say, shell, switch};

const FIRST: &str = "7e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a07";

/// A conversation in `paired`'s home, opened.
async fn on<'a>(
    backend: &TestBackend,
    master: &Session,
    vendor: &'a MockVendor,
    paired: &Paired,
) -> Driven<'a> {
    let provider = anthropic_at(backend, master, vendor, "/offline").await;
    create(backend, master, FIRST).await;
    let home = paired.runner.home_dir().to_owned();
    switch(backend, master, FIRST, paired, &home).await;
    Driven::open(backend, master, vendor, FIRST, &provider, "/offline").await
}

/// Stops `paired`'s runner, waits until the backend shows it offline, and
/// lets 40 seconds pass on the backend's clock.
async fn offline_for_40s(harness: &Harness, backend: &TestBackend, master: &Session, paired: &mut Paired) {
    paired.runner.stop().await;
    backend.until_online(master, paired.id(), false).await;
    harness.clock.advance(jiff::SignedDuration::from_secs(40));
}

/// How long a helper below reads frames before it counts as hung.
const HANG: Duration = Duration::from_secs(30);

/// Reads the conversation's frames until a command shows waiting for its
/// Host, and answers the Host's name and the command's id.
async fn until_waiting(work: &mut Driven<'_>) -> (String, String) {
    let waiting = async {
        loop {
            if let ServerFrame::ShellOutput { status, .. } = work.socket.frame().await
                && let ShellStatus::Running {
                    waiting_for: Some(host),
                    ..
                } = &*status
            {
                return (host.clone(), status.command().command_id.to_string());
            }
        }
    };
    tokio::time::timeout(HANG, waiting).await.expect("a command shows waiting for its Host")
}

/// Reads the conversation's frames until its root is idle.
async fn until_root_idle(work: &mut Driven<'_>) {
    let idle = async {
        loop {
            if crate::conversations::root_phase(&work.socket.frame().await) == Some(SessionPhase::Idle) {
                return;
            }
        }
    };
    tokio::time::timeout(HANG, idle).await.expect("the root comes to rest");
}

// About two seconds: a real device pairs through a network the test cuts
// and opens again.
#[tokio::test]
async fn a_command_on_an_offline_primary_device_waits_for_it_and_runs_once_it_is_back() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let network = Network::start(backend.address()).await;
    let alpha = backend.pair_through(&master, "alpha", &network.url).await;
    let mut work = on(&backend, &master, &vendor, &alpha).await;
    network.cut();
    backend.until_online(&master, alpha.id(), false).await;

    let before = work
        .start(vec![shell("t1", "echo hello", 60_000), say("done")])
        .await;
    assert_eq!(until_waiting(&mut work).await.0, "alpha");
    network.open();
    until_root_idle(&mut work).await;

    // The model sees a command that ran, never the device's absence.
    let ran = work.observe(before);
    assert!(ran.received[0].contains("exitCode: 0") && ran.received[0].contains("hello"), "{}", ran.received[0]);
    assert!(!ran.received[0].contains("offline"), "{}", ran.received[0]);
    backend.close().await;
}

// About three seconds: a real device pairs through a network the test cuts
// and opens again, and a command runs once it is back.
#[tokio::test]
async fn a_stop_of_a_later_turn_ends_the_command_an_earlier_turn_left_waiting_and_it_never_runs() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let network = Network::start(backend.address()).await;
    let alpha = backend.pair_through(&master, "alpha", &network.url).await;
    let mut work = on(&backend, &master, &vendor, &alpha).await;
    let home = alpha.runner.home_dir().to_owned();
    network.cut();
    backend.until_online(&master, alpha.id(), false).await;

    // The first turn ends with its command waiting in the background.
    work.start(vec![shell("t1", "touch ran", 1_000), say("waiting")]).await;
    let (_, command) = until_waiting(&mut work).await;
    until_root_idle(&mut work).await;

    // The user stops the next turn while its request is under way; the
    // command's end is reported as any background command's.
    let (open, held) = tokio::sync::watch::channel(false);
    let before = work.start(vec![say("held").held(held)]).await;
    vendor.received(before + 1).await;
    work.script(vec![say("noted")]);
    work.socket.stop().await;
    drop(open);
    until_requested(&vendor, &format!("Command {command} (t1) was stopped")).await;
    // The stop's frames and the report's turn interleave on the socket; the
    // summary says when that turn is over.
    crate::support::eventually("the report's turn ends", || async {
        summary(&backend, &master, FIRST).await.status != ConversationStatus::Running
    })
    .await;
    network.open();
    backend.until_online(&master, alpha.id(), true).await;

    // A command the device runs after its return shows the stopped one was
    // never sent: the runner would have received it first.
    // The socket still holds the frames of the report's turn, so the
    // model's requests say when it ran.
    work.start(vec![shell("t2", "echo ran-$((1 + 1))", 10_000), say("done")]).await;
    until_requested(&vendor, "ran-2").await;
    assert!(!home.join("ran").exists(), "the stopped command never ran");
    backend.close().await;
}

// About two seconds: two real devices pair, one through a network the test
// cuts.
#[tokio::test]
async fn a_move_while_the_agent_waits_for_its_offline_device_fails_the_command_and_moves_the_conversation() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let network = Network::start(backend.address()).await;
    let alpha = backend.pair_through(&master, "alpha", &network.url).await;
    let beta = backend.pair(&master, "beta").await;
    let mut work = on(&backend, &master, &vendor, &alpha).await;
    network.cut();
    backend.until_online(&master, alpha.id(), false).await;

    let before = work
        .start(vec![shell("t1", "echo hello", 60_000), say("moved on")])
        .await;
    until_waiting(&mut work).await;
    // The backend says the agent only waits for the device, so the page's
    // move stops nothing.
    assert!(crate::conversations::metadata(&backend, &master, FIRST).await.only_waits_for_host);
    let beta_home = beta.runner.home_dir().to_owned();
    switch(&backend, &master, FIRST, &beta, &beta_home).await;
    until_root_idle(&mut work).await;

    let target = serde_json::to_value(summary(&backend, &master, FIRST).await.target).unwrap();
    assert_eq!(target["deviceId"], beta.id(), "{target}");
    let moved = work.observe(before);
    assert!(
        moved.received[0].contains("This conversation now runs on beta; this command did not run, since alpha was offline."),
        "{}",
        moved.received[0]
    );
    backend.close().await;
}

#[tokio::test]
async fn a_command_on_an_offline_attached_device_fails_at_once_with_its_offline_error() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    let mut build = backend.pair(&master, "build-box").await;
    let mut work = on(&backend, &master, &vendor, &alpha).await;
    harness.attach(FIRST, build.id());
    offline_for_40s(&harness, &backend, &master, &mut build).await;

    let tried = work
        .turn(vec![
            shell("t1", "demi host shell --host build-box 'echo hi'", 10_000),
            say("noted"),
        ])
        .await;
    // The command's error line names the command and the reason, without
    // the words of the pipe that carried it.
    assert!(
        tried.received[0].lines().any(|line| line
            == "demi host shell: build-box is offline: its runner has been disconnected for 40s, so nothing can run there now; commands already running there keep running and report when it is back."),
        "{}",
        tried.received[0]
    );
    backend.close().await;
}

// About three seconds: two real devices pair, one through a network the
// test cuts and opens again, and the background command's call waits for
// two quiet seconds.
#[tokio::test]
async fn a_move_away_from_an_offline_device_leaves_its_running_command_there_which_reports_its_end_when_it_is_back() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let network = Network::start(backend.address()).await;
    let alpha = backend.pair_through(&master, "alpha", &network.url).await;
    let beta = backend.pair(&master, "beta").await;
    let mut work = on(&backend, &master, &vendor, &alpha).await;
    let home = alpha.runner.home_dir().to_owned();
    let started = work
        .turn(vec![
            resident("t1", "printf 'started\\n'; while [ ! -f finish ]; do sleep 0.05; done; exit 3"),
            say("started"),
        ])
        .await;
    assert!(started.received[0].starts_with("status: running"), "{}", started.received[0]);
    let command = field(&started.received[0], "commandId").to_owned();

    // The device goes away with the command running there, and the user
    // moves the conversation once the agent's turn ended.
    network.cut();
    backend.until_online(&master, alpha.id(), false).await;
    let beta_home = beta.runner.home_dir().to_owned();
    switch(&backend, &master, FIRST, &beta, &beta_home).await;

    // The command ends while its runner is away, and reports its end once
    // the runner is back.
    work.script(vec![say("noted")]);
    std::fs::write(home.join("finish"), "").unwrap();
    network.open();
    until_requested(&vendor, &format!("Command {command} (t1) ended with exit code 3.")).await;
    backend.close().await;
}

// About three seconds: a real device pairs through a network the test cuts,
// and the backend restarts while the agent's command waits for the device.
#[tokio::test]
async fn a_command_waiting_for_its_device_outlives_a_backend_restart_and_runs_once_the_device_is_back() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let network = Network::start(backend.address()).await;
    let alpha = backend.pair_through(&master, "alpha", &network.url).await;
    let mut work = on(&backend, &master, &vendor, &alpha).await;
    let home = alpha.runner.home_dir().to_owned();
    network.cut();
    backend.until_online(&master, alpha.id(), false).await;

    // The call returns at its interval and the turn ends, the command
    // waiting on.
    work.start(vec![shell("t1", "touch ran", 1_000), say("waiting")]).await;
    let (_, command) = until_waiting(&mut work).await;
    until_root_idle(&mut work).await;
    work.script(vec![say("noted")]);

    // The shutdown waits for no device.
    let address = backend.address();
    tokio::time::timeout(HANG, backend.close())
        .await
        .expect("the backend shuts down while a command waits for its device");
    assert!(!home.join("ran").exists());

    // The backend that starts again sends it once the runner connects, and
    // the agent hears of its end as of any command's.
    let backend = harness.start_at(address).await;
    network.open();
    until_requested(&vendor, &format!("Command {command} (t1) ended with exit code 0.")).await;
    assert!(home.join("ran").exists());
    backend.close().await;
}

// About three seconds: a real device pairs through a network the test cuts
// and opens again, and a second command runs once it is back.
#[tokio::test]
async fn a_stop_of_a_command_waiting_for_its_device_ends_it_and_it_never_runs() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let network = Network::start(backend.address()).await;
    let alpha = backend.pair_through(&master, "alpha", &network.url).await;
    let mut work = on(&backend, &master, &vendor, &alpha).await;
    let home = alpha.runner.home_dir().to_owned();
    network.cut();
    backend.until_online(&master, alpha.id(), false).await;

    work.start(vec![shell("t1", "touch ran", 1_000), say("waiting")]).await;
    let (_, command) = until_waiting(&mut work).await;
    until_root_idle(&mut work).await;

    // The page's End Command stops it as `demi shell stop` does, through
    // its environment.
    work.script(vec![say("noted")]);
    work.socket
        .send(&ClientFrame::ShellAbort {
            command_id: CommandId::try_from(command.as_str()).unwrap(),
        })
        .await;
    until_requested(&vendor, &format!("Command {command} (t1) was stopped")).await;
    // The Stop's frames and the report's turn interleave on the socket; the
    // summary says when that turn is over.
    crate::support::eventually("the report's turn ends", || async {
        summary(&backend, &master, FIRST).await.status != ConversationStatus::Running
    })
    .await;
    network.open();
    backend.until_online(&master, alpha.id(), true).await;

    // A command the device runs after its return shows the stopped one was
    // never sent: the runner would have received it first.
    // The socket still holds the frames of the report's turn, so the
    // model's requests say when it ran.
    work.start(vec![shell("t2", "echo ran-$((1 + 1))", 10_000), say("done")]).await;
    until_requested(&vendor, "ran-2").await;
    assert!(!home.join("ran").exists(), "the stopped command never ran");
    backend.close().await;
}

/// Every device list in `events` shows `device` online.
fn shown_online(events: &[SyncEvent], device: &str) {
    for event in events {
        if let SyncEvent::Devices { devices } = event {
            let shown = devices.iter().find(|shown| shown.id.as_str() == device).unwrap();
            assert_eq!(shown.state, DeviceState::Online, "{shown:?}");
        }
    }
}

/// A connection that drops and whose runner comes back within the wait: the
/// page is told the device is online at the drop and after, and the work
/// panel's listing and the agent's command, asked for while the runner was
/// away, go on once it is back, the model never told of an absence.
// About two seconds: a real device pairs through a network the test cuts
// and opens again.
#[tokio::test]
async fn a_drop_the_runner_returns_from_shows_no_offline_change_and_what_was_asked_meanwhile_goes_on() {
    let vendor = MockVendor::start().await;
    let mut harness = Harness::new();
    // Longer than the test: the runner comes back first.
    harness.runners.returning = Duration::from_secs(10 * 60);
    let (backend, master) = harness.start_set_up().await;
    let network = Network::start(backend.address()).await;
    let alpha = backend.pair_through(&master, "alpha", &network.url).await;
    let mut work = on(&backend, &master, &vendor, &alpha).await;
    let mut page = backend.sync(&master).await;
    let seen = |devices: &[demi_web_api_protocol::devices::DeviceDto]| {
        devices.iter().find(|device| device.id.as_str() == alpha.id()).unwrap().last_seen_at
    };
    let connected = seen(&page.snapshot().await.devices);

    // The backend records when it last saw the runner as the connection
    // ends, which tells the page.
    harness.clock.advance(jiff::SignedDuration::from_mins(1));
    network.cut();
    let dropped = page
        .until(|event| matches!(event, SyncEvent::Devices { devices } if seen(devices) != connected))
        .await;
    shown_online(&dropped, alpha.id());

    let away = match dropped.last() {
        Some(SyncEvent::Devices { devices }) => seen(devices),
        _ => unreachable!("the wait ended at a device list"),
    };

    // Asked for while the runner is away, whose hello is held once it is
    // back; the backend records when it saw the runner again.
    let hello = backend.hold_hellos(HelloStep::Bind);
    harness.clock.advance(jiff::SignedDuration::from_mins(1));
    let listed_at = std::cell::Cell::new(false);
    let listing = async {
        let listed = backend.get(&format!("/api/conversations/{FIRST}/fs"), Some(&master)).await;
        listed_at.set(true);
        listed
    };
    let returning = async {
        let before = work
            .start(vec![shell("t1", "echo hello", 60_000), say("done")])
            .await;
        network.open();
        hello.until_arrived(1).await;
        assert!(!listed_at.get(), "the listing waits for the runner");
        hello.release();
        before
    };
    let (listed, before) = tokio::join!(listing, returning);
    assert_eq!(listed.status, reqwest::StatusCode::OK, "{}", String::from_utf8_lossy(&listed.body));
    until_root_idle(&mut work).await;
    let ran = work.observe(before);
    assert!(ran.received[0].contains("exitCode: 0") && ran.received[0].contains("hello"), "{}", ran.received[0]);
    assert!(!ran.received[0].contains("offline"), "{}", ran.received[0]);
    let back = page
        .until(|event| matches!(event, SyncEvent::Devices { devices } if seen(devices) != away))
        .await;
    shown_online(&back, alpha.id());
    assert!(backend.online(&master, alpha.id()).await);
    backend.close().await;
}

/// An agent's command on its primary device, started while the device's
/// runner is away after a drop, waits for it; when the wait ends without the
/// runner, the command fails with the device's offline error, though a
/// command started on a device already offline waits for it as long as it
/// takes.
// About two seconds: a real device pairs through a network the test cuts;
// the wait passes on the paused clock of the shards, which run on the test's
// thread.
#[tokio::test(flavor = "local")]
async fn a_command_that_waited_through_a_drop_fails_as_offline_when_the_wait_ends() {
    let vendor = MockVendor::start().await;
    let mut harness = Harness::new();
    harness.shards = Some(ShardPlacement::Inline);
    harness.runners.returning = Duration::from_secs(10);
    let (backend, master) = harness.start_set_up().await;
    let network = Network::start(backend.address()).await;
    let alpha = backend.pair_through(&master, "alpha", &network.url).await;
    let mut work = on(&backend, &master, &vendor, &alpha).await;
    // A first turn has the backend keep what the conversation's context
    // reads of the device, so the next one asks the model at once.
    work.turn(vec![say("ready")]).await;
    dropped(&harness, &backend, &master, &network, alpha.id()).await;

    let before = work
        .start(vec![shell("t1", "echo hello", 60_000), say("done")])
        .await;
    backend.until_waiting_for_return(&master, 1).await;
    elapse(Duration::from_secs(11)).await;
    until_root_idle(&mut work).await;
    let failed = work.observe(before);
    assert!(failed.received[0].contains("alpha is offline"), "{}", failed.received[0]);
    backend.close().await;
}
