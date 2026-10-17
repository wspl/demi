//! A conversation whose Host goes offline (`sessions-and-targets.md` § Host
//! operations, § Switch the primary target): the agent's operation on an
//! offline device, primary or attached, fails at once with the device's
//! offline error, worded for the model so it can decide what to do; and a
//! move away from an offline device leaves the commands still running there
//! to its runner, which reports their ends once it is back; and a turn the
//! device left resumes once by itself when the device's runner connects
//! again (`failures-and-recovery.md` § The unfinished turn). The model is an
//! Anthropic endpoint the test scripts; the devices are real runners.

use demi_agent_tools::testing::field;
use demi_conversation_socket_protocol::ClientFrame;
use demi_provider_common::testing::MockVendor;

use demi_backend_user_shard::holds::HelloStep;
use demi_provider_common::testing::MockResponse;
use demi_shared_types::Block;
use demi_web_api_protocol::conversations::ConversationStatus;
use serde_json::json;

use crate::conversations::{anthropic_at, create, summary, transcript};
use crate::lifetime::{Network, until_requested};
use crate::support::{Harness, Paired, Session, TestBackend, eventually};
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

/// Whether a request to the model carried the call `call`'s result as an
/// error.
fn errored(vendor: &MockVendor, call: &str) -> bool {
    vendor.requests().iter().any(|request| {
        request.json()["messages"].as_array().unwrap().iter().any(|message| {
            message["content"].as_array().into_iter().flatten().any(|block| {
                block["type"] == "tool_result" && block["tool_use_id"] == call && block["is_error"] == true
            })
        })
    })
}

/// Resumes the unfinished turn, as the page's Resume does, and answers the
/// last message of the model's first request after it: the resume.
async fn resume(work: &mut Driven<'_>, vendor: &MockVendor) -> String {
    work.script(vec![say("done")]);
    let before = vendor.requests().len();
    work.socket.send(&ClientFrame::Resume {}).await;
    work.socket.until_idle().await;
    let resumed = work.observe(before);
    resumed.requests[0]["messages"].as_array().unwrap().last().unwrap().to_string()
}

#[tokio::test]
async fn a_shell_call_on_an_offline_primary_device_says_it_is_offline_and_what_becomes_of_its_commands() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let mut alpha = backend.pair(&master, "alpha").await;
    let mut work = on(&backend, &master, &vendor, &alpha).await;
    offline_for_40s(&harness, &backend, &master, &mut alpha).await;

    let tried = work.turn(vec![shell("t1", "echo hello", 10_000), say("offline")]).await;
    assert_eq!(
        tried.received[0],
        "alpha is offline: its runner has been disconnected for 40s, so nothing can run there now; commands already running there are kept for up to 10m and report when it is back; the user can resume this turn once it is back."
    );
    assert!(errored(&vendor, "t1"), "the result is an error to the model");
    // The turn ends unfinished, with Demi's record of why after the
    // agent's last words, which the page offers Resume for.
    let blocks = transcript(&backend, &master, FIRST).await.blocks;
    let Some(Block::Error(error)) = blocks.last() else {
        panic!("the turn ends with {:?}", blocks.last());
    };
    assert_eq!(
        (error.code.as_deref(), error.message.as_str(), error.device.as_ref().map(|device| device.id.as_str())),
        (Some("host_offline"), "alpha went offline, so this turn could not finish its work.", Some(alpha.id()))
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
            == "demi host shell: build-box is offline: its runner has been disconnected for 40s, so nothing can run there now; commands already running there are kept for up to 10m and report when it is back."),
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

/// A conversation on `alpha`, paired through `network`, whose turn alpha
/// left: the network is cut, the backend shows alpha offline, and the turn's
/// shell call fails for it.
async fn left_by_alpha<'a>(
    backend: &TestBackend,
    master: &Session,
    vendor: &'a MockVendor,
    network: &Network,
) -> (Paired, Driven<'a>) {
    let alpha = backend.pair_through(master, "alpha", &network.url).await;
    let mut work = on(backend, master, vendor, &alpha).await;
    network.cut();
    backend.until_online(master, alpha.id(), false).await;
    work.turn(vec![shell("t1", "echo hello", 10_000), say("offline")]).await;
    (alpha, work)
}

/// Brings `alpha` back through `network` and answers once the backend has
/// decided what its coming back resumes: whatever it resumed has started,
/// and a turn it left that still waits is at rest. The next answer the
/// model is scripted with never comes, so a turn resumed by mistake keeps
/// running.
async fn back_without_resuming(backend: &TestBackend, master: &Session, work: &Driven<'_>, network: &Network) {
    let (_never, never) = tokio::sync::watch::channel(false);
    work.script(vec![say("resumed").held(never)]);
    let back = backend.hold_hellos(HelloStep::HostBack);
    network.open();
    back.until_arrived(1).await;
    assert_eq!(
        summary(backend, master, FIRST).await.status,
        ConversationStatus::Error,
        "the turn waits for the user's Resume"
    );
    back.release();
}

/// The `host_offline` record the conversation's last turn ended with, by
/// the device it names.
async fn left_record(backend: &TestBackend, master: &Session) -> (String, Option<String>) {
    let blocks = transcript(backend, master, FIRST).await.blocks;
    let Some(Block::Error(error)) = blocks.last() else {
        panic!("the turn ends with {:?}", blocks.last());
    };
    assert_eq!(error.code.as_deref(), Some("host_offline"));
    (error.id.to_string(), error.device.as_ref().map(|device| device.id.clone()))
}

// About two seconds: a real device pairs through a network the test cuts
// and opens again.
#[tokio::test]
async fn a_device_that_comes_back_resumes_the_turn_it_left_saying_it_is_back_online() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let network = Network::start(backend.address()).await;
    let (_alpha, mut work) = left_by_alpha(&backend, &master, &vendor, &network).await;

    work.script(vec![say("done")]);
    let before = vendor.requests().len();
    network.open();
    work.socket.until_idle().await;
    let resumed = work.observe(before);
    assert_eq!(resumed.requests.len(), 1, "one resumed turn, one request");
    let last = resumed.requests[0]["messages"].as_array().unwrap().last().unwrap().to_string();
    assert!(last.contains("alpha is back online. Continue from where you left off."), "{last}");
    backend.close().await;
}

// About two seconds: a real device pairs through a network the test cuts
// and opens again.
#[tokio::test]
async fn a_device_back_before_the_turn_it_left_ends_resumes_that_turn_as_it_ends() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let network = Network::start(backend.address()).await;
    let alpha = backend.pair_through(&master, "alpha", &network.url).await;
    let mut work = on(&backend, &master, &vendor, &alpha).await;
    network.cut();
    backend.until_online(&master, alpha.id(), false).await;

    // The call fails while alpha is away; the model's last words wait until
    // alpha is back, so the turn ends with alpha online.
    let (back, words) = tokio::sync::watch::channel(false);
    work.start(vec![shell("t1", "echo hello", 10_000), say("offline").held(words), say("done")])
        .await;
    until_requested(&vendor, "alpha is offline").await;
    network.open();
    backend.until_online(&master, alpha.id(), true).await;
    back.send_replace(true);
    until_requested(&vendor, "alpha is back online. Continue from where you left off.").await;
    backend.close().await;
}

// About three seconds: a real device pairs through a network the test cuts
// and opens three times.
#[tokio::test]
async fn a_turn_the_same_device_leaves_again_after_coming_back_waits_for_the_user() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let network = Network::start(backend.address()).await;
    let (alpha, work) = left_by_alpha(&backend, &master, &vendor, &network).await;
    let (first, _) = left_record(&backend, &master).await;

    // The resumed turn's call waits until the device is away again.
    let (gone, away) = tokio::sync::watch::channel(false);
    work.script(vec![shell("t2", "echo again", 10_000).held(away), say("offline again")]);
    network.open();
    until_requested(&vendor, "alpha is back online. Continue from where you left off.").await;
    network.cut();
    backend.until_online(&master, alpha.id(), false).await;
    gone.send_replace(true);
    eventually("the resumed turn ends left by alpha again", || async {
        let blocks = transcript(&backend, &master, FIRST).await.blocks;
        matches!(blocks.last(), Some(Block::Error(error)) if error.code.as_deref() == Some("host_offline") && error.id.to_string() != first)
    })
    .await;
    assert_eq!(left_record(&backend, &master).await.1.as_deref(), Some(alpha.id()));

    back_without_resuming(&backend, &master, &work, &network).await;
    backend.close().await;
}

// About two seconds: a real device pairs through a network the test cuts
// and opens again.
#[tokio::test]
async fn with_resume_when_host_back_off_a_device_that_comes_back_leaves_its_turn_waiting() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let patched = backend
        .patch("/api/settings/preferences", &master, json!({ "resumeWhenHostBack": false }))
        .await;
    assert!(patched.status.is_success(), "{}", String::from_utf8_lossy(&patched.body));
    let network = Network::start(backend.address()).await;
    let (_alpha, work) = left_by_alpha(&backend, &master, &vendor, &network).await;

    back_without_resuming(&backend, &master, &work, &network).await;
    backend.close().await;
}

// About two seconds: two real devices pair, one through a network the test
// cuts and opens again.
#[tokio::test]
async fn a_device_that_comes_back_leaves_the_turn_of_a_conversation_moved_away_waiting() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let network = Network::start(backend.address()).await;
    let (_alpha, work) = left_by_alpha(&backend, &master, &vendor, &network).await;
    let beta = backend.pair(&master, "beta").await;
    let beta_home = beta.runner.home_dir().to_owned();
    switch(&backend, &master, FIRST, &beta, &beta_home).await;

    back_without_resuming(&backend, &master, &work, &network).await;
    backend.close().await;
}

// About two seconds: a real device pairs through a network the test cuts
// and opens again.
#[tokio::test]
async fn a_device_that_comes_back_leaves_a_turn_that_failed_otherwise_waiting() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let network = Network::start(backend.address()).await;
    let alpha = backend.pair_through(&master, "alpha", &network.url).await;
    let mut work = on(&backend, &master, &vendor, &alpha).await;
    work.turn(vec![
        MockResponse::status(401)
            .chunk(r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#),
    ])
    .await;
    network.cut();
    backend.until_online(&master, alpha.id(), false).await;

    back_without_resuming(&backend, &master, &work, &network).await;
    backend.close().await;
}

// About two seconds: two real devices pair.
#[tokio::test]
async fn a_resume_after_a_move_says_where_the_conversation_now_runs() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let mut alpha = backend.pair(&master, "alpha").await;
    let beta = backend.pair(&master, "beta").await;
    let mut work = on(&backend, &master, &vendor, &alpha).await;
    offline_for_40s(&harness, &backend, &master, &mut alpha).await;
    work.turn(vec![shell("t1", "echo hello", 10_000), say("offline")]).await;

    // The user moves the conversation while the device is still away.
    let beta_home = beta.runner.home_dir().to_owned();
    switch(&backend, &master, FIRST, &beta, &beta_home).await;
    let resumed = resume(&mut work, &vendor).await;
    assert!(
        resumed.contains("This conversation now runs on beta. Continue from where you left off."),
        "{resumed}"
    );
    backend.close().await;
}
