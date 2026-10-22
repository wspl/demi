//! The Cloud through the backend (`crates/machine-manager/src/lib.rs`,
//! `crates/backend-database/src/selection.rs`,
//! `crates/backend-cloud/src/uses.rs`,
//! `crates/backend-user-shard/src/lifecycle/mod.rs`).
//!
//! Scenarios:
//!
//! - The first uses of two conversations boot one Cloud, which keeps nothing
//!   of an ended command.
//! - An idle Cloud stops, and the next operation wakes it.
//! - The model learns the home the Cloud's runner reports once, and not again
//!   at each wake.
//! - A conversation that idles on a Cloud another keeps running hears its
//!   release there.
//! - A conversation archived while the Cloud is stopped does not wake it.
//! - A Cloud the manager stopped without a word boots again.
//! - A conversation that left the Cloud still reaches it.
//! - A reset keeps home and identity and tells the model. A failed reset
//!   resumes. A reset holds the conversations that need the Cloud and leaves
//!   the ones that only have it attached.
//! - Capacity counts across users.
//! - A Cloud that keeps dying stops booting until a reset.
//! - A boot whose runner never connects fails.
//! - A shutdown saves the Cloud.
//! - Each wake names the size its account's Cloud disks have then.
//!
//! The machine manager is the scripted one; each Cloud's runner is real, and
//! the model is an Anthropic endpoint the test scripts.

use std::sync::Arc;
use std::time::Duration;

use demi_backend_user_shard::tuning::LifecycleTuning;
use demi_provider_common::testing::MockVendor;
use demi_shared_gates::Purpose;
use demi_web_api_protocol::cloud::{CloudResetAnswer, CloudState, CloudStatus, ResetPhase};
use demi_web_api_protocol::devices::{DeviceKind, DeviceState};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::files::{Directory, FileText};
use reqwest::StatusCode;
use serde_json::json;
use tokio::sync::Notify;

use crate::conversations::{anthropic_at, create};
use crate::files;
use crate::streams::{self, received};
use crate::support::{GNU_UTILITIES, Harness, PATIENCE, SCRATCH, Session, TestBackend, eventually};
use crate::work::{Driven, say, shell};

/// The conversation ids the scenarios create.
const FIRST: &str = "4e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a01";
const SECOND: &str = "4e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a02";
/// A reset's id, as the page names it.
const RESET: &str = "6e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a09";

pub(crate) async fn status(backend: &TestBackend, session: &Session) -> CloudStatus {
    let read = backend.get("/api/cloud", Some(session)).await;
    assert_eq!(
        read.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&read.body)
    );
    read.json()
}

/// The Cloud's status once `check` holds, asking every 20 ms for at most
/// 20 s.
pub(crate) async fn until_status(
    backend: &TestBackend,
    session: &Session,
    what: &str,
    check: impl Fn(&CloudStatus) -> bool,
) -> CloudStatus {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let status = status(backend, session).await;
        if check(&status) {
            return status;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "never came true: {what}: {status:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Whether the user's Cloud has a live runner, as the device list says.
pub(crate) async fn cloud_online(backend: &TestBackend, session: &Session) -> bool {
    backend
        .devices(session)
        .await
        .iter()
        .any(|device| device.kind == DeviceKind::Managed && device.state == DeviceState::Online)
}

/// The one device the manager made, the user's Cloud.
pub(crate) fn the_cloud(harness: &Harness) -> String {
    let devices = harness.manager.devices();
    assert_eq!(devices.len(), 1, "{devices:?}");
    devices[0].clone()
}

/// A query parameter's value, encoded.
fn query(path: &str) -> String {
    url::form_urlencoded::byte_serialize(path.as_bytes()).collect()
}

pub(crate) async fn reset(backend: &TestBackend, session: &Session, id: &str) -> CloudResetAnswer {
    let answer = backend
        .post(
            "/api/cloud/reset",
            Some(session),
            json!({ "operationId": id }),
        )
        .await;
    assert_eq!(
        answer.status,
        StatusCode::ACCEPTED,
        "{}",
        String::from_utf8_lossy(&answer.body)
    );
    answer.json()
}

/// Waits until the Cloud's runner holds no job directory: it keeps nothing
/// of a command once the backend has what it needs of it.
async fn keeps_no_job(harness: &Harness, device: &str) {
    let jobs = harness.manager.state(device).join("jobs");
    eventually("the Cloud keeps no job directory", || {
        let held = std::fs::read_dir(&jobs)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .any(|entry| entry.path().is_dir())
            })
            .unwrap_or(false);
        async move { !held }
    })
    .await;
}

/// Short idle windows, read often.
pub(crate) fn idle_after(window: Duration) -> LifecycleTuning {
    LifecycleTuning {
        idle_window: window,
        idle_poll: Duration::from_millis(50),
    }
}

// Several seconds: the Cloud boots for the first uses and installs the builtin
// package, stops after an idle window, and boots again.
#[tokio::test]
async fn the_first_uses_of_two_conversations_boot_one_cloud_which_stops_when_idle_and_wakes_for_the_next_operation()
 {
    let vendor = MockVendor::start().await;
    let mut harness = Harness::new().with_file_package();
    harness.lifecycle = idle_after(Duration::from_millis(600));
    // The conversations' own idle watches read their activity once an hour
    // here, so the Cloud's own idle watch is what stops it.
    harness.lifecycle.idle_poll = Duration::from_secs(3600);
    harness.cloud.sweep = Duration::from_millis(50);
    let (backend, master) = harness.start_set_up().await;
    let first_model = anthropic_at(&backend, &master, &vendor, "/a").await;
    let second_model = anthropic_at(&backend, &master, &vendor, "/b").await;
    for id in [FIRST, SECOND] {
        create(&backend, &master, id).await;
    }
    // Neither history nor an open conversation needs the Cloud.
    let mut a = Driven::open(&backend, &master, &vendor, FIRST, &first_model, "/a").await;
    let mut b = Driven::open(&backend, &master, &vendor, SECOND, &second_model, "/b").await;
    assert_eq!(
        status(&backend, &master).await.state,
        CloudState::Unallocated
    );
    assert_eq!(harness.manager.calls(), ["reconcile"]);

    // A lease of the first conversation's file gate is its work: the Cloud
    // stays awake until the test has looked, and idles once it ends.
    let working = backend
        .file_gate(&master, FIRST)
        .await
        .enter(Purpose::Demand)
        .await;
    let (first, second) = tokio::join!(
        a.turn(vec![shell("a1", "echo 0 > note"), say("a wrote")]),
        b.turn(vec![shell("b1", "echo 1 > note"), say("b wrote")]),
    );
    assert!(
        first.received[0].contains("exitCode: 0"),
        "{}",
        first.received[0]
    );
    assert!(
        second.received[0].contains("exitCode: 0"),
        "{}",
        second.received[0]
    );
    let device = the_cloud(&harness);
    assert_eq!(harness.manager.count(&format!("wake:{device}")), 1);
    let home = harness.manager.home(&device);
    assert_eq!(
        std::fs::read_to_string(format!("{home}/sessions/{FIRST}/note")).unwrap(),
        "0\n"
    );
    assert_eq!(
        std::fs::read_to_string(format!("{home}/sessions/{SECOND}/note")).unwrap(),
        "1\n"
    );
    keeps_no_job(&harness, &device).await;
    let running = status(&backend, &master).await;
    assert_eq!(running.state, CloudState::Running);
    assert_eq!(
        running.device.map(|device| device.id.as_str().to_owned()),
        Some(device.clone())
    );

    // The conversation's files are in its session directory on the Cloud.
    let listed: Directory = backend
        .get(&format!("/api/conversations/{FIRST}/fs"), Some(&master))
        .await
        .json();
    assert_eq!(listed.path, format!("{home}/sessions/{FIRST}"));
    assert!(
        listed.entries.iter().any(|entry| entry.name == "note"),
        "{listed:?}"
    );

    // Idle, the Cloud is saved and stops once.
    drop(working);
    let hibernate = format!("hibernate:{device}");
    eventually("the idle Cloud stops", || async {
        harness.manager.count(&hibernate) == 1 && !harness.manager.running(&device)
    })
    .await;
    until_status(&backend, &master, "the Cloud is off", |status| {
        status.state == CloudState::Off
    })
    .await;

    // A file read wakes it, and the next command runs on the awake Cloud.
    let path = format!("{home}/sessions/{FIRST}/note");
    let read = backend
        .get(
            &format!("/api/conversations/{FIRST}/fs/file?path={}", query(&path)),
            Some(&master),
        )
        .await;
    assert_eq!(
        read.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&read.body)
    );
    assert_eq!(read.json::<FileText>().text, "0\n");
    let next = a
        .turn(vec![shell("a2", "cat note"), say("awake")])
        .await;
    assert!(next.received[0].contains("0\n"), "{}", next.received[0]);
    assert_eq!(harness.manager.count(&format!("wake:{device}")), 2);
    // The runner woke with the home and the system it reported before,
    // which changes no execution context: the model read no new block.
    let blocks = |turn: &crate::work::Turn| {
        turn.requests.last().unwrap()["messages"]
            .to_string()
            .matches("[Execution context ")
            .count()
    };
    assert_eq!(blocks(&next), blocks(&first));
    backend.close().await;
}

// About three seconds: the Cloud boots and installs the fixture package, and
// the first conversation's idle window passes in real time while the second
// works.
#[tokio::test]
async fn a_conversation_that_idles_on_a_cloud_another_keeps_running_hears_its_release_there() {
    let vendor = MockVendor::start().await;
    let mut harness = Harness::new().with_native_fixture();
    harness.lifecycle = idle_after(Duration::from_millis(600));
    harness.cloud.sweep = Duration::from_millis(50);
    let (backend, master) = harness.start_set_up().await;
    let first_model = anthropic_at(&backend, &master, &vendor, "/a").await;
    let second_model = anthropic_at(&backend, &master, &vendor, "/b").await;
    for id in [FIRST, SECOND] {
        create(&backend, &master, id).await;
    }
    let mut a = Driven::open(&backend, &master, &vendor, FIRST, &first_model, "/a").await;
    let mut b = Driven::open(&backend, &master, &vendor, SECOND, &second_model, "/b").await;
    // The second conversation works throughout, which keeps the Cloud
    // running.
    let working = backend
        .file_gate(&master, SECOND)
        .await
        .enter(Purpose::Demand)
        .await;
    // Each runs a job, and the fixture service holds each conversation, as
    // the browser service holds a conversation's Chrome until its release.
    let (first, second) = tokio::join!(
        a.turn(vec![shell("a1", "echo ran"), say("ran")]),
        b.turn(vec![shell("b1", "echo ran"), say("ran")]),
    );
    assert!(
        first.received[0].contains("exitCode: 0"),
        "{}",
        first.received[0]
    );
    assert!(
        second.received[0].contains("exitCode: 0"),
        "{}",
        second.received[0]
    );
    for id in [FIRST, SECOND] {
        let (_, code, reason) =
            received(&mut streams::socket(&backend, &master, id, "retain").await).await;
        assert_eq!((code, reason.as_str()), (1000, "completed"));
    }
    let device = the_cloud(&harness);

    // A window after its last activity, the first conversation hears its
    // release on the running Cloud: the service lets it go, and keeps the
    // second.
    eventually(
        "the idle conversation's release reaches the service",
        || async {
            let (held, _, _) =
                received(&mut streams::socket(&backend, &master, SECOND, "held").await).await;
            serde_json::from_slice::<serde_json::Value>(&held).unwrap()
                == json!({ "conversations": [SECOND] })
        },
    )
    .await;
    // The machine runs on: it booted once and never stopped.
    assert_eq!(harness.manager.count(&format!("wake:{device}")), 1);
    assert_eq!(harness.manager.count(&format!("hibernate:{device}")), 0);
    assert_eq!(status(&backend, &master).await.state, CloudState::Running);
    drop(working);
    backend.close().await;
}

// About a second: the Cloud boots, and the backend restarts.
#[tokio::test]
async fn a_conversation_archived_while_its_cloud_is_stopped_does_not_wake_it() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic_at(&backend, &master, &vendor, "/a").await;
    create(&backend, &master, FIRST).await;
    let mut work = Driven::open(&backend, &master, &vendor, FIRST, &provider, "/a").await;
    let ran = work
        .turn(vec![shell("t1", "echo ran"), say("ran")])
        .await;
    assert!(
        ran.received[0].contains("exitCode: 0"),
        "{}",
        ran.received[0]
    );
    let device = the_cloud(&harness);

    // The backend stops, and the manager with it, whose stop stops the
    // Cloud, as an upgrade does. Archived while the Cloud is stopped, the
    // conversation's release does not wake it: the stop ended what a
    // release would.
    let address = backend.address();
    backend.close().await;
    harness.manager.stop_quietly(&device).await;
    let backend = harness.start_at(address).await;
    let archived = backend
        .patch(
            &format!("/api/conversations/{FIRST}"),
            &master,
            json!({ "archived": true }),
        )
        .await;
    assert_eq!(
        archived.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&archived.body)
    );
    assert_eq!(harness.manager.count(&format!("wake:{device}")), 1);
    assert_eq!(status(&backend, &master).await.state, CloudState::Off);
    backend.close().await;
}

// Over a second: the Cloud boots twice, and installs the builtin package.
#[tokio::test]
async fn a_cloud_the_manager_stopped_without_a_word_boots_again_for_the_operations_that_need_it() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new().with_file_package();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic_at(&backend, &master, &vendor, "/a").await;
    create(&backend, &master, FIRST).await;
    let mut work = Driven::open(&backend, &master, &vendor, FIRST, &provider, "/a").await;
    let wrote = work
        .turn(vec![
            shell("t1", "echo retained > note"),
            say("saved"),
        ])
        .await;
    assert!(
        wrote.received[0].contains("exitCode: 0"),
        "{}",
        wrote.received[0]
    );
    let device = the_cloud(&harness);
    let home = harness.manager.home(&device);

    // A manager restart stops the sandbox before it can report the stop.
    harness.manager.stop_quietly(&device).await;
    eventually("the backend sees the runner go", || async {
        !cloud_online(&backend, &master).await
    })
    .await;
    assert_eq!(status(&backend, &master).await.state, CloudState::Running);

    // Two reads at once join one recovery, which boots the Cloud again.
    let path = format!(
        "/api/conversations/{FIRST}/fs/file?path={}",
        query(&format!("{home}/sessions/{FIRST}/note"))
    );
    let (one, two) = tokio::join!(
        backend.get(&path, Some(&master)),
        backend.get(&path, Some(&master))
    );
    for read in [one, two] {
        assert_eq!(
            read.status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&read.body)
        );
        assert_eq!(read.json::<FileText>().text, "retained\n");
    }
    assert_eq!(harness.manager.count(&format!("wake:{device}")), 2);
    backend.close().await;
}

// Several seconds: the Cloud boots three times, for its first work and after
// two idle stops, and stops a third time; it and a paired device each install
// the builtin package.
#[tokio::test]
async fn a_conversation_that_left_the_cloud_reaches_it_as_an_attached_host_which_its_work_wakes() {
    let vendor = MockVendor::start().await;
    let mut harness = Harness::new().with_file_package();
    harness.lifecycle = idle_after(Duration::from_millis(800));
    // The conversation's own idle watch reads its activity once an hour
    // while it works here, so the Cloud's own idle watch is what stops it.
    harness.lifecycle.idle_poll = Duration::from_secs(3600);
    harness.cloud.sweep = Duration::from_millis(50);
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    std::fs::write(alpha.runner.home_dir().join("notes.txt"), "on alpha\n").unwrap();
    let provider = anthropic_at(&backend, &master, &vendor, "/work").await;
    create(&backend, &master, FIRST).await;
    let mut work = Driven::open(&backend, &master, &vendor, FIRST, &provider, "/work").await;

    // On the Cloud, with alpha attached: the Cloud's job reaches alpha.
    harness.attach(FIRST, alpha.id());
    let script = "echo report > report.txt && demi host shell --host alpha 'cat notes.txt'";
    let from_cloud = work
        .turn(vec![shell("t1", script), say("reached alpha")])
        .await;
    assert!(
        from_cloud.received[0].contains("on alpha"),
        "{}",
        from_cloud.received[0]
    );
    let device = the_cloud(&harness);
    let session = format!("{}/sessions/{FIRST}", harness.manager.home(&device));

    // Leaving the Cloud keeps it attached where the conversation worked.
    crate::work::switch(&backend, &master, FIRST, &alpha, alpha.runner.home_dir()).await;
    let hosts = crate::conversations::metadata(&backend, &master, FIRST).await.hosts;
    let cloud = hosts
        .hosts
        .iter()
        .find(|host| host.device_id.as_str() == device)
        .expect("the Cloud stays attached");
    assert_eq!(
        (cloud.name.as_str(), cloud.cwd.as_deref()),
        ("Cloud", Some(session.as_str()))
    );

    // The Cloud stops once no conversation uses it; browsing it wakes it,
    // and so does a command sent to it.
    eventually("the idle Cloud stops", || async {
        !harness.manager.running(&device)
    })
    .await;
    let browsed = backend
        .get(
            &format!(
                "/api/conversations/{FIRST}/hosts/{device}/fs?path={}",
                query(&session)
            ),
            Some(&master),
        )
        .await;
    assert_eq!(
        browsed.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&browsed.body)
    );
    assert!(
        browsed
            .json::<Directory>()
            .entries
            .iter()
            .any(|entry| entry.name == "report.txt")
    );
    assert_eq!(harness.manager.count(&format!("wake:{device}")), 2);
    eventually("the idle Cloud stops again", || async {
        !harness.manager.running(&device)
    })
    .await;
    let back = work
        .turn(vec![
            shell(
                "t2",
                "demi host shell --host Cloud 'cat report.txt'",
            ),
            say("read the Cloud"),
        ])
        .await;
    assert!(
        back.first_request().contains("[Execution target switched]"),
        "{}",
        back.first_request()
    );
    assert!(back.received[0].contains("report"), "{}", back.received[0]);
    assert_eq!(harness.manager.count(&format!("wake:{device}")), 3);
    eventually("the idle Cloud stops a third time", || async {
        !harness.manager.running(&device)
    })
    .await;
    backend.close().await;
}

// Several seconds: the Cloud installs the builtin package, and boots again
// after the reset.
#[tokio::test]
async fn a_reset_keeps_the_clouds_files_and_identity_tells_the_model_and_is_the_same_reset_when_asked_again()
 {
    let vendor = MockVendor::start().await;
    let harness = Harness::new().with_file_package();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic_at(&backend, &master, &vendor, "/a").await;
    create(&backend, &master, FIRST).await;
    let mut work = Driven::open(&backend, &master, &vendor, FIRST, &provider, "/a").await;
    let wrote = work
        .turn(vec![
            shell("t1", "echo retained > note"),
            say("written"),
        ])
        .await;
    assert!(
        wrote.received[0].contains("exitCode: 0"),
        "{}",
        wrote.received[0]
    );
    // The model learned its Host before its first request; a Cloud whose
    // runner never connected is named without its system.
    let unmade = format!(
        "[Execution context 0]\\nPrimary host: Cloud. Shells start in /home/demi/sessions/{FIRST}. {SCRATCH} {GNU_UTILITIES}\\nAttached hosts: none."
    );
    assert!(
        wrote.first_request().contains(&unmade),
        "{}",
        wrote.first_request()
    );
    // The command woke the Cloud, whose runner reported its home, a
    // temporary directory here: the model's next request names the
    // directory its shells really start in, as the Host now is, with no
    // label of a change.
    let home = harness.manager.home(&the_cloud(&harness));
    let reported = wrote.requests.last().unwrap()["messages"].to_string();
    assert!(
        reported.contains("[Execution context 1]\\nPrimary host: Cloud, ")
            && reported.contains(&format!(
                "Shells start in {home}/sessions/{FIRST}. {SCRATCH} {GNU_UTILITIES}\\nAttached hosts: none."
            )),
        "{reported}"
    );
    let before = status(&backend, &master)
        .await
        .device
        .expect("the Cloud was made");
    // The server moved to a newer image since the Cloud was made, which a
    // reset moves its system to.
    assert!(!status(&backend, &master).await.newer_image);
    harness
        .manager
        .script(|script| script.base = Some("test-base-2".into()));
    assert!(status(&backend, &master).await.newer_image);

    let (one, two, three) = tokio::join!(
        reset(&backend, &master, RESET),
        reset(&backend, &master, RESET),
        reset(&backend, &master, RESET),
    );
    for answer in [one, two, three] {
        assert_eq!(answer.operation.id.as_str(), RESET);
    }
    let ready = until_status(&backend, &master, "the reset is ready", |status| {
        status
            .operation
            .as_ref()
            .is_some_and(|operation| operation.phase == ResetPhase::Ready)
    })
    .await;
    assert_eq!(ready.state, CloudState::Running);
    assert_eq!(ready.device, Some(before));
    assert!(!ready.newer_image);

    // Home stays; the model hears of the reset before its next request.
    let next = work
        .turn(vec![shell("t2", "cat note"), say("read")])
        .await;
    assert!(
        next.received[0].contains("retained"),
        "{}",
        next.received[0]
    );
    assert!(
        next.first_request()
            .contains(&format!("[Cloud reset {RESET}]")),
        "{}",
        next.first_request()
    );
    // The block names the Cloud with the system its runner reported now.
    let named = format!(
        " ({}). Shells start in {home}/sessions/{FIRST}. {SCRATCH} {GNU_UTILITIES}\\n[Cloud reset {RESET}]",
        std::env::consts::ARCH
    );
    assert!(
        next.first_request().contains("Primary host: Cloud, ")
            && next.first_request().contains(&named),
        "{}",
        next.first_request()
    );
    let again = reset(&backend, &master, RESET).await;
    assert_eq!(again.operation.phase, ResetPhase::Ready);
    let device = the_cloud(&harness);
    let resets = harness
        .manager
        .calls()
        .iter()
        .filter(|call| call.starts_with("reset:"))
        .count();
    assert_eq!(resets, 1);
    assert!(
        harness
            .manager
            .calls()
            .contains(&format!("reset:{device}:{RESET}:test-base-2"))
    );

    // Another reset while one runs is refused.
    let (held, proceed) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    harness
        .manager
        .script(|script| script.hold_reset = Some((held.clone(), proceed.clone())));
    let second = "7e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a0a";
    reset(&backend, &master, second).await;
    held.notified().await;
    let third = backend
        .post(
            "/api/cloud/reset",
            Some(&master),
            json!({ "operationId": "8e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a0b" }),
        )
        .await;
    assert_eq!(
        third.refusal(),
        (StatusCode::CONFLICT, ErrorCode::CloudResetting)
    );
    proceed.notify_one();
    until_status(&backend, &master, "the second reset is ready", |status| {
        status.operation.as_ref().is_some_and(|operation| {
            operation.id.as_str() == second && operation.phase == ResetPhase::Ready
        })
    })
    .await;
    backend.close().await;
}

// Several seconds: the Cloud installs the builtin package, and the resumed
// reset boots it again.
#[tokio::test]
async fn a_failed_reset_reports_its_failure_and_the_same_operation_resumes_without_losing_home() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new().with_file_package();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic_at(&backend, &master, &vendor, "/a").await;
    create(&backend, &master, FIRST).await;
    let mut work = Driven::open(&backend, &master, &vendor, FIRST, &provider, "/a").await;
    let wrote = work
        .turn(vec![
            shell("t1", "printf retained > note"),
            say("written"),
        ])
        .await;
    assert!(
        wrote.received[0].contains("exitCode: 0"),
        "{}",
        wrote.received[0]
    );
    let device = the_cloud(&harness);

    harness
        .manager
        .script(|script| script.fail_reset = Some("image publication unavailable".into()));
    reset(&backend, &master, RESET).await;
    let failed = until_status(&backend, &master, "the reset fails", |status| {
        status
            .operation
            .as_ref()
            .is_some_and(|operation| operation.phase == ResetPhase::Failed)
    })
    .await;
    let operation = failed.operation.unwrap();
    assert!(
        operation
            .error
            .unwrap()
            .contains("image publication unavailable")
    );
    assert_eq!(failed.state, CloudState::Off);

    reset(&backend, &master, RESET).await;
    until_status(&backend, &master, "the resumed reset is ready", |status| {
        status
            .operation
            .as_ref()
            .is_some_and(|operation| operation.phase == ResetPhase::Ready)
    })
    .await;
    let path = format!("{}/sessions/{FIRST}/note", harness.manager.home(&device));
    let read = backend
        .get(
            &format!("/api/conversations/{FIRST}/fs/file?path={}", query(&path)),
            Some(&master),
        )
        .await;
    assert_eq!(read.json::<FileText>().text, "retained");
    // The retry reset on the base the first attempt selected.
    let resets: Vec<String> = harness
        .manager
        .calls()
        .into_iter()
        .filter(|call| call.starts_with("reset:"))
        .collect();
    assert_eq!(
        resets,
        [
            format!("reset:{device}:{RESET}:test-base"),
            format!("reset:{device}:{RESET}:test-base")
        ]
    );
    backend.close().await;
}

// Several seconds: the Cloud and a paired device each install the builtin
// package, and the Cloud boots again after the reset and installs it again.
#[tokio::test]
async fn a_reset_holds_a_cloud_conversation_until_it_ends_and_leaves_one_that_only_has_the_cloud_attached_running()
 {
    let vendor = MockVendor::start().await;
    let harness = Harness::new().with_file_package();
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    let cloud_model = anthropic_at(&backend, &master, &vendor, "/cloud").await;
    let local_model = anthropic_at(&backend, &master, &vendor, "/local").await;
    create(&backend, &master, FIRST).await;
    create(&backend, &master, SECOND).await;
    let mut cloud = Driven::open(&backend, &master, &vendor, FIRST, &cloud_model, "/cloud").await;
    let wrote = cloud
        .turn(vec![
            shell("c1", "echo kept > note"),
            say("written"),
        ])
        .await;
    assert!(
        wrote.received[0].contains("exitCode: 0"),
        "{}",
        wrote.received[0]
    );
    let device = the_cloud(&harness);
    // The second conversation leaves the Cloud for alpha, which keeps the
    // Cloud attached.
    crate::work::switch(&backend, &master, SECOND, &alpha, alpha.runner.home_dir()).await;
    let hosts = crate::conversations::metadata(&backend, &master, SECOND).await.hosts;
    assert_eq!(
        hosts
            .hosts
            .iter()
            .map(|host| host.device_id.as_str())
            .collect::<Vec<_>>(),
        [device.as_str()]
    );
    let mut local = Driven::open(&backend, &master, &vendor, SECOND, &local_model, "/local").await;

    let (held, proceed) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    harness
        .manager
        .script(|script| script.hold_reset = Some((held.clone(), proceed.clone())));
    reset(&backend, &master, RESET).await;
    held.notified().await;

    // The conversation on alpha is opened, written to and answered.
    local
        .reconnect(&backend, &master, SECOND, &local_model)
        .await;
    let during = local
        .turn(vec![shell("l1", "echo on-alpha"), say("done")])
        .await;
    assert!(
        during.received[0].contains("on-alpha"),
        "{}",
        during.received[0]
    );

    // The Cloud conversation waits instead of failing, at its file gate,
    // which the reset holds, and opens by itself once the reset ends.
    let mut waiting = backend.file_gate(&master, FIRST).await.waiting();
    {
        let opening = cloud.reconnect(&backend, &master, FIRST, &cloud_model);
        tokio::pin!(opening);
        tokio::select! {
            () = &mut opening => panic!("the open did not wait for the reset"),
            waits = waiting.wait_for(|count| *count > 0) => assert!(waits.is_ok()),
        }
        proceed.notify_one();
        opening.await;
    }
    let after = cloud
        .turn(vec![shell("c2", "cat note"), say("read")])
        .await;
    assert!(after.received[0].contains("kept"), "{}", after.received[0]);
    backend.close().await;
}

#[tokio::test]
async fn a_cloud_that_keeps_dying_stops_booting_by_itself_until_a_reset_starts_it() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    create(&backend, &master, FIRST).await;
    let listing = format!("/api/conversations/{FIRST}/fs");
    for death in 1..=3 {
        let listed = backend.get(&listing, Some(&master)).await;
        assert_eq!(
            listed.status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&listed.body)
        );
        let device = the_cloud(&harness);
        harness.manager.kill(&device).await;
        until_status(
            &backend,
            &master,
            &format!("death {death} stops the Cloud"),
            |status| status.state == CloudState::Off,
        )
        .await;
    }
    let refused = backend.get(&listing, Some(&master)).await;
    assert_eq!(
        refused.refusal(),
        (StatusCode::SERVICE_UNAVAILABLE, ErrorCode::CloudCrashLoop)
    );
    let device = the_cloud(&harness);
    assert_eq!(harness.manager.count(&format!("wake:{device}")), 3);

    reset(&backend, &master, RESET).await;
    until_status(&backend, &master, "the reset is ready", |status| {
        status
            .operation
            .as_ref()
            .is_some_and(|operation| operation.phase == ResetPhase::Ready)
    })
    .await;
    let listed = backend.get(&listing, Some(&master)).await;
    assert_eq!(
        listed.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&listed.body)
    );
    backend.close().await;
}

// Over a second: the first Cloud idles for a window before the second can
// start.
#[tokio::test]
async fn capacity_counts_the_clouds_of_every_user_and_a_cloud_that_finds_none_fails_to_start() {
    let mut harness = Harness::new();
    harness.cloud.capacity = 1;
    harness.lifecycle = idle_after(Duration::from_millis(500));
    harness.cloud.sweep = Duration::from_millis(50);
    let (backend, master) = harness.start_set_up().await;
    harness.add_user(
        "ana@example.test",
        "ana-pass-1",
        demi_web_api_protocol::auth::Role::User,
    );
    let ana = backend.login("ana@example.test", "ana-pass-1").await;
    create(&backend, &master, FIRST).await;
    create(&backend, &ana, SECOND).await;

    let listed = backend
        .get(&format!("/api/conversations/{FIRST}/fs"), Some(&master))
        .await;
    assert_eq!(
        listed.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&listed.body)
    );
    let refused = backend
        .get(&format!("/api/conversations/{SECOND}/fs"), Some(&ana))
        .await;
    assert_eq!(
        refused.refusal(),
        (StatusCode::SERVICE_UNAVAILABLE, ErrorCode::CloudCapacity)
    );
    let refused = backend
        .post(
            "/api/cloud/reset",
            Some(&ana),
            json!({ "operationId": RESET }),
        )
        .await;
    assert_eq!(
        refused.refusal(),
        (StatusCode::CONFLICT, ErrorCode::CloudCapacity)
    );

    // Once the first Cloud stops, the second starts.
    until_status(&backend, &master, "the first Cloud stops", |status| {
        status.state == CloudState::Off
    })
    .await;
    let listed = backend
        .get(&format!("/api/conversations/{SECOND}/fs"), Some(&ana))
        .await;
    assert_eq!(
        listed.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&listed.body)
    );
    assert_eq!(harness.manager.devices().len(), 2);
    backend.close().await;
}

// Several seconds: the boot timeout below passes in real time.
#[tokio::test]
async fn a_boot_whose_runner_never_connects_fails_saves_what_it_started_and_says_why() {
    let mut harness = Harness::new();
    // Short for the runner that never connects, and long enough for the
    // one that does on a busy machine.
    harness.cloud.runner_connection = Duration::from_secs(3);
    let (backend, master) = harness.start_set_up().await;
    create(&backend, &master, FIRST).await;
    harness.manager.script(|script| script.silent_wake = true);
    let listing = format!("/api/conversations/{FIRST}/fs");
    let refused = backend.get(&listing, Some(&master)).await;
    assert_eq!(
        refused.refusal(),
        (StatusCode::SERVICE_UNAVAILABLE, ErrorCode::CloudUnavailable)
    );
    assert_eq!(
        refused.error().message,
        "Your Cloud didn’t start. Its runner didn’t connect in 3 seconds."
    );
    let device = the_cloud(&harness);
    assert_eq!(harness.manager.count(&format!("hibernate:{device}")), 1);
    let failed = status(&backend, &master).await;
    assert_eq!(failed.state, CloudState::Off);
    assert!(failed.error.unwrap().contains("didn’t start"));

    harness.manager.script(|script| script.silent_wake = false);
    let listed = backend.get(&listing, Some(&master)).await;
    assert_eq!(
        listed.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&listed.body)
    );
    assert_eq!(status(&backend, &master).await.error, None);
    backend.close().await;
}

/// A Cloud whose runner cannot start, here because its executable is
/// missing, fails its wake at once and says why: the manager answers the
/// wake with the reason rather than never, so the request that woke the
/// Cloud is refused instead of waiting for ever.
#[tokio::test]
async fn a_wake_whose_runner_cannot_start_fails_at_once_and_says_why() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    create(&backend, &master, FIRST).await;
    let nowhere = tempfile::tempdir().unwrap();
    let missing = nowhere.path().join("demi-runner");
    harness
        .manager
        .script(|script| script.runner = Some(missing.clone()));
    let listing = format!("/api/conversations/{FIRST}/fs");
    let refused = tokio::time::timeout(PATIENCE, backend.get(&listing, Some(&master)))
        .await
        .expect("the wake is answered");
    assert_eq!(
        refused.refusal(),
        (StatusCode::SERVICE_UNAVAILABLE, ErrorCode::CloudUnavailable)
    );
    let message = refused.error().message;
    assert!(message.contains(&missing.display().to_string()), "{message}");
    backend.close().await;
}

#[tokio::test]
async fn the_clouds_device_log_answers_while_it_runs_and_a_stopped_cloud_says_so_without_waking() {
    let mut harness = Harness::new();
    harness.lifecycle = idle_after(Duration::from_millis(400));
    harness.cloud.sweep = Duration::from_millis(50);
    let (backend, master) = harness.start_set_up().await;
    create(&backend, &master, FIRST).await;
    // A lease of the conversation's file gate is its work: the Cloud stays
    // awake while the test reads its log, and idles once it ends.
    let working = backend
        .file_gate(&master, FIRST)
        .await
        .enter(Purpose::Demand)
        .await;
    let listing = format!("/api/conversations/{FIRST}/fs");
    let listed = backend.get(&listing, Some(&master)).await;
    assert_eq!(
        listed.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&listed.body)
    );
    let device = the_cloud(&harness);
    let log = format!("/api/devices/{device}/log");
    // The runner writes a line to its files a moment after it connected.
    let onlines = || async {
        let read = backend.get(&log, Some(&master)).await;
        if read.status != StatusCode::OK {
            return 0;
        }
        let lines = read
            .json::<demi_web_api_protocol::devices::DeviceLog>()
            .lines;
        lines.iter().filter(|line| line.text == "online").count()
    };
    eventually("the running Cloud's log says it is online", || async {
        onlines().await == 1
    })
    .await;

    drop(working);
    until_status(&backend, &master, "the idle Cloud stops", |status| {
        status.state == CloudState::Off
    })
    .await;
    let stopped = backend.get(&log, Some(&master)).await;
    assert_eq!(
        stopped.refusal(),
        (StatusCode::CONFLICT, ErrorCode::DeviceOffline)
    );
    assert_eq!(harness.manager.count(&format!("wake:{device}")), 1);

    // Woken, it answers again, with what it wrote before it stopped.
    let listed = backend.get(&listing, Some(&master)).await;
    assert_eq!(
        listed.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&listed.body)
    );
    eventually("the woken Cloud's log keeps the earlier boot", || async {
        onlines().await == 2
    })
    .await;
    backend.close().await;
}

// A few seconds: the Cloud boots once, runs one shell job, and idles.
//
// Whether the file an attachment was uploaded from is there is a look, which a
// stopped Cloud refuses rather than wakes, so the page opens the copy and the
// Cloud stays stopped. Planted defect it catches: the check admitted as a file
// transfer is, which boots the stopped Cloud.
#[tokio::test]
async fn whether_an_attachments_file_is_there_never_wakes_a_stopped_cloud() {
    let vendor = MockVendor::start().await;
    let mut harness = Harness::new();
    harness.lifecycle = idle_after(Duration::from_millis(400));
    harness.cloud.sweep = Duration::from_millis(50);
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic_at(&backend, &master, &vendor, "/a").await;
    create(&backend, &master, FIRST).await;
    let mut work = Driven::open(&backend, &master, &vendor, FIRST, &provider, "/a").await;
    let script = "printf '# Notes\\n' > notes.md && demi attachment upload notes.md";
    let uploaded = work
        .turn(vec![shell("t1", script), say("uploaded")])
        .await;
    assert!(uploaded.received[0].contains("a1"), "{}", uploaded.received[0]);
    let check = format!("/api/conversations/{FIRST}/attachments/a1/file");
    let present = backend.get(&check, Some(&master)).await;
    assert_eq!(
        present.status,
        StatusCode::NO_CONTENT,
        "{}",
        String::from_utf8_lossy(&present.body)
    );

    until_status(&backend, &master, "the idle Cloud stops", |status| {
        status.state == CloudState::Off
    })
    .await;
    assert_eq!(
        backend.get(&check, Some(&master)).await.refusal(),
        (StatusCode::CONFLICT, ErrorCode::HostStopped)
    );
    let device = the_cloud(&harness);
    assert_eq!(harness.manager.count(&format!("wake:{device}")), 1);
    backend.close().await;
}

// Several seconds: the Cloud boots before and after the backend's restart, and
// installs the builtin package.
#[tokio::test]
async fn the_clouds_files_and_the_usage_ledger_survive_a_backend_restart() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new().with_file_package();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic_at(&backend, &master, &vendor, "/a").await;
    create(&backend, &master, FIRST).await;
    let mut work = Driven::open(&backend, &master, &vendor, FIRST, &provider, "/a").await;
    let script = "demi file edit <<'EOF'\nnotes.md\n<<<<<<< SEARCH\n=======\nkeep me\n>>>>>>> REPLACE\nEOF";
    let stored = work
        .turn(vec![shell("t1", script), say("stored")])
        .await;
    assert!(
        stored.received[0].contains("Created notes.md (1 line)"),
        "{}",
        stored.received[0]
    );
    let requests = |usage: demi_web_api_protocol::usage::UsageTotals| {
        usage.totals.iter().map(|group| group.requests).sum::<u64>()
    };
    let before = requests(backend.get("/api/usage", Some(&master)).await.json());

    // The backend's close leaves the Cloud running, and the next start takes
    // it over: it boots nothing again. It starts at the address it had: the
    // Cloud's runner keeps its state, which names that address as its
    // backend's.
    let address = backend.address();
    backend.close().await;
    let backend = harness.start_at(address).await;
    work.reconnect(&backend, &master, FIRST, &provider).await;
    let found = work
        .turn(vec![
            shell("t2", "cat notes.md"),
            say("found"),
        ])
        .await;
    assert!(
        found.received[0].contains("keep me"),
        "{}",
        found.received[0]
    );
    let after = requests(backend.get("/api/usage", Some(&master)).await.json());
    assert_eq!(after, before + 2);
    let device = the_cloud(&harness);
    assert_eq!(harness.manager.count(&format!("wake:{device}")), 1);
    assert_eq!(harness.manager.count(&format!("hibernate:{device}")), 0);
    backend.close().await;
}

// One backend and no runner; a broken shutdown reaches the 20-second test
// guard. The runner connection timeout stays at its production default.
#[tokio::test]
async fn shutdown_cancels_a_cloud_boot_waiting_for_its_runner_and_saves_it_once() {
    let harness = Harness::new().with_claude_package();
    harness.manager.script(|script| script.silent_wake = true);
    let (backend, master) = harness.start_set_up().await;
    // A Claude sign-in runs the CLI's login on the Claude Code machine,
    // which it wakes.
    let login = backend
        .post(
            "/api/providers/subscription-login",
            Some(&master),
            json!({ "providerType": "claude-code" }),
        )
        .await;
    assert_eq!(login.status, StatusCode::ACCEPTED);
    eventually("the sign-in wakes the machine", || async {
        harness
            .manager
            .calls()
            .iter()
            .any(|call| call.starts_with("wake:"))
    })
    .await;
    let device = harness.manager.claude_code_machine();

    tokio::time::timeout(PATIENCE, backend.close())
        .await
        .expect("shutdown must not wait out the runner connection timeout");
    assert_eq!(harness.manager.count(&format!("hibernate:{device}")), 1);
}

#[tokio::test]
async fn shutdown_ends_an_open_download_and_leaves_the_cloud_running() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    create(&backend, &master, FIRST).await;
    let listed = backend
        .get(&format!("/api/conversations/{FIRST}/fs"), Some(&master))
        .await;
    assert_eq!(
        listed.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&listed.body)
    );
    let device = the_cloud(&harness);
    let home = harness.manager.home(&device);
    let big = format!("{home}/sessions/{FIRST}/big.bin");
    std::fs::write(&big, vec![7u8; 8 << 20]).unwrap();
    // A download the user's browser stopped reading holds the Cloud.
    let download = backend
        .response(
            reqwest::Method::GET,
            &format!("/api/conversations/{FIRST}/fs/raw?path={}", query(&big)),
            &master,
            &[],
            None,
        )
        .await;
    assert_eq!(download.status(), StatusCode::OK);

    let url = backend.url.clone();
    backend.close_reporting().await.unwrap();
    // The download ended rather than held the shutdown, and the Cloud runs
    // on: the shutdown neither saves it nor asks the manager anything.
    drop(download);
    assert!(harness.manager.running(&device));
    let calls = harness.manager.calls();
    assert!(!calls.contains(&format!("hibernate:{device}")), "{calls:?}");
    assert_eq!(calls.iter().filter(|call| *call == "reconcile").count(), 1, "{calls:?}");
    // The listener is gone.
    assert!(reqwest::get(format!("{url}/api/setup")).await.is_err());
}

/// The Cloud and a command on it survive a backend restart of one release:
/// the manager keeps the sandbox, the next start takes the Cloud over, and
/// the command reports its end there (`crates/backend-remote-host/src/device_jobs.rs`). Several seconds: the Cloud boots once, its job is a
/// login shell, and the call waits for two quiet seconds.
#[tokio::test]
async fn a_backend_restart_keeps_the_cloud_and_a_command_on_it_reports_its_end() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic_at(&backend, &master, &vendor, "/a").await;
    create(&backend, &master, FIRST).await;
    let mut work = Driven::open(&backend, &master, &vendor, FIRST, &provider, "/a").await;
    let script = "printf 'started\\n'; while [ ! -f finish ]; do sleep 0.05; done; exit 4";
    let started = work
        .turn(vec![crate::work::background("t1", script), say("waiting")])
        .await;
    assert!(started.received[0].starts_with("status: running"), "{}", started.received[0]);
    let command = demi_agent_tools::testing::field(&started.received[0], "commandId").to_owned();
    let device = the_cloud(&harness);
    let home = harness.manager.home(&device);

    let address = backend.address();
    backend.close().await;
    work.script(vec![say("noted")]);
    let backend = harness.start_at(address).await;
    until_status(&backend, &master, "the Cloud runs", |status| {
        status.state == CloudState::Running
    })
    .await;
    std::fs::write(format!("{home}/sessions/{FIRST}/finish"), "").unwrap();
    let report = format!("Command {command} (t1) ended with exit code 4.");
    eventually("the command's end reaches the model", || {
        let held = vendor
            .requests()
            .iter()
            .any(|request| request.json().to_string().contains(&report));
        async move { held }
    })
    .await;
    assert_eq!(harness.manager.count(&format!("wake:{device}")), 1);
    assert_eq!(harness.manager.count(&format!("hibernate:{device}")), 0);
    backend.close().await;
}

/// A Cloud taken over at a backend start keeps its lifetime cap's clock: the cap
/// counts from the boot its record keeps, not from the start
/// (`crates/backend-cloud/src/lib.rs`).
///
/// Here the Cloud booted two hours before a start whose cap is one hour, so the
/// start stops it at once.
///
/// Several seconds: the Cloud boots once, and the call that leaves a command
/// holding it waits for two quiet seconds.
#[tokio::test]
async fn a_cloud_taken_over_at_a_backend_start_keeps_its_lifetime_cap_clock() {
    let vendor = MockVendor::start().await;
    let mut harness = Harness::new();
    harness.cloud.lifetime_cap = Duration::from_secs(60 * 60);
    harness.cloud.sweep = Duration::from_millis(100);
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic_at(&backend, &master, &vendor, "/a").await;
    create(&backend, &master, FIRST).await;
    let mut work = Driven::open(&backend, &master, &vendor, FIRST, &provider, "/a").await;
    // A command left running holds the Cloud: no idle stop comes.
    let script = "printf 'started\\n'; while :; do sleep 0.05; done";
    let started = work
        .turn(vec![crate::work::background("t1", script), say("waiting")])
        .await;
    assert!(started.received[0].starts_with("status: running"), "{}", started.received[0]);
    let device = the_cloud(&harness);

    let address = backend.address();
    backend.close().await;
    let moved = harness
        .control_database()
        .execute(
            "UPDATE devices SET cloud_started_at = cloud_started_at - 7200000 WHERE kind = 'managed' AND cloud_started_at IS NOT NULL",
            [],
        )
        .unwrap();
    assert_eq!(moved, 1, "the Cloud's boot is recorded");
    work.script(vec![say("noted")]);
    let backend = harness.start_at(address).await;
    eventually("the Cloud stops at its lifetime cap", || async {
        harness.manager.count(&format!("hibernate:{device}")) == 1
            && !harness.manager.running(&device)
    })
    .await;
    assert_eq!(harness.manager.count(&format!("wake:{device}")), 1);
    backend.close().await;
}

/// An upgrade stops the Cloud with the manager, so the command on it ends. The
/// next start takes its conversation up, which starts the Cloud with the new
/// release's programs, and the background command is reported lost to the upgrade at
/// the next turn (`upgrades.md`).
///
/// Several seconds: the Cloud boots twice.
#[tokio::test]
async fn an_upgrade_restarts_the_cloud_and_its_command_is_lost_to_the_upgrade() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic_at(&backend, &master, &vendor, "/a").await;
    create(&backend, &master, FIRST).await;
    let mut work = Driven::open(&backend, &master, &vendor, FIRST, &provider, "/a").await;
    let started = work
        .turn(vec![crate::work::background("t1", "while :; do sleep 1; done"), say("waiting")])
        .await;
    let command = demi_agent_tools::testing::field(&started.received[0], "commandId").to_owned();
    let device = the_cloud(&harness);

    // The upgrade stops the backend, then the manager, whose stop saves and
    // stops the Cloud; the Cloud last ran another release.
    let address = backend.address();
    backend.close().await;
    harness.manager.stop_quietly(&device).await;
    harness
        .control_database()
        .execute("UPDATE devices SET runner_version = '0.0.1'", [])
        .unwrap();
    let backend = harness.start_at(address).await;
    let provider = anthropic_at(&backend, &master, &vendor, "/a").await;
    work.reconnect(&backend, &master, FIRST, &provider).await;
    let report = format!(
        "Command {command} (t1) was lost: {}. Start it again if it is still needed.",
        demi_backend_remote_host::UPGRADED
    );
    crate::lifetime::told_at_the_next_turn(&mut work, &harness, FIRST, &vendor, &report).await;
    assert_eq!(harness.manager.count(&format!("wake:{device}")), 2);
    backend.close().await;
}

// Several seconds: the Cloud boots, and three idle windows pass in real
// time: the conversation works past the window its first file read started,
// so that work that did not count would let the Cloud stop meanwhile, and
// then rests for one.
#[tokio::test]
async fn an_attached_cloud_stays_awake_while_the_conversation_works_on_its_paired_target() {
    let window = Duration::from_millis(600);
    let mut harness = Harness::new();
    harness.lifecycle = idle_after(window);
    harness.cloud.sweep = Duration::from_millis(50);
    let (backend, master) = harness.start_set_up().await;
    let paired = backend.pair(&master, "paired").await;
    create(&backend, &master, FIRST).await;
    let listing = format!("/api/conversations/{FIRST}/fs");
    let listed = backend.get(&listing, Some(&master)).await;
    assert_eq!(
        listed.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&listed.body)
    );
    let device = the_cloud(&harness);
    crate::work::switch(&backend, &master, FIRST, &paired, paired.runner.home_dir()).await;

    // Work on the paired target, with the Cloud attached, keeps it awake past its
    // idle window: it stops a full window after the conversation rests, and not
    // before.
    //
    // The last activity ends after its request was sent, so the window is counted
    // from the sending.
    let working = tokio::time::Instant::now();
    let mut rested = working;
    while rested - working < window * 2 {
        rested = tokio::time::Instant::now();
        let listed = backend.get(&listing, Some(&master)).await;
        assert_eq!(
            listed.status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&listed.body)
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let stopped = harness
        .manager
        .arrival(&format!("hibernate:{device}"), PATIENCE)
        .await;
    assert!(
        stopped >= rested + window,
        "the Cloud stopped {:?} after the work began and the work rested after {:?}",
        stopped.saturating_duration_since(working),
        rested - working
    );
    backend.close().await;
}

#[tokio::test]
async fn a_backend_that_stopped_in_the_middle_of_a_reset_finishes_its_disk_step_when_it_starts_and_boots_nothing()
 {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    create(&backend, &master, FIRST).await;
    let listed = backend
        .get(&format!("/api/conversations/{FIRST}/fs"), Some(&master))
        .await;
    assert_eq!(
        listed.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&listed.body)
    );
    let device = the_cloud(&harness);
    backend.close().await;
    // The backend stopped once the reset had selected its base and was
    // rebuilding the disks.
    harness
        .control_database()
        .execute(
            "INSERT INTO managed_operations (device_id, operation_id, base_version, phase, error, updated_at)
             VALUES (?1, ?2, 'test-base', 'rebuilding', NULL, 0)",
            rusqlite::params![device, RESET],
        )
        .unwrap();
    let context_before: i64 = harness
        .control_database()
        .query_row(
            "SELECT context_version FROM conversations WHERE id = ?1",
            [FIRST],
            |row| row.get(0),
        )
        .unwrap();

    let backend = harness.start().await;
    let calls = harness.manager.calls();
    let started = calls.iter().rposition(|call| call == "reconcile").unwrap();
    assert_eq!(
        calls[started + 1..],
        [format!("reset:{device}:{RESET}:test-base")]
    );
    let recovered = status(&backend, &master).await;
    assert_eq!(recovered.state, CloudState::Off);
    let operation = recovered.operation.expect("the reset's record");
    assert_eq!(operation.phase, ResetPhase::Failed);
    assert_eq!(
        operation.error.as_deref(),
        Some("Reset disks recovered. Retry to start Cloud.")
    );
    let context = || -> i64 {
        harness
            .control_database()
            .query_row(
                "SELECT context_version FROM conversations WHERE id = ?1",
                [FIRST],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert_eq!(context(), context_before + 1);
    assert_eq!(harness.manager.count(&format!("wake:{device}")), 1);

    // A reset recorded as failed is not resumed when the backend starts
    // again: only a retry runs it.
    backend.close().await;
    let backend = harness.start().await;
    let calls = harness.manager.calls();
    let started = calls.iter().rposition(|call| call == "reconcile").unwrap();
    assert!(calls[started + 1..].is_empty(), "{calls:?}");

    // A retry of the same reset starts the Cloud, and its announcement,
    // made at the recovery, advances the conversation's context no further;
    // nor does the runner's report, of the home the backend kept, a
    // temporary directory here.
    reset(&backend, &master, RESET).await;
    until_status(&backend, &master, "the retried reset is ready", |status| {
        status
            .operation
            .as_ref()
            .is_some_and(|operation| operation.phase == ResetPhase::Ready)
    })
    .await;
    assert_eq!(harness.manager.count(&format!("wake:{device}")), 2);
    assert_eq!(context(), context_before + 1);
    backend.close().await;
}

// Several seconds: the Cloud boots and installs the builtin package, and its
// lifetime cap of two seconds passes in real time.
#[tokio::test]
async fn at_its_lifetime_cap_the_cloud_ends_the_jobs_nothing_attends_and_stops() {
    let vendor = MockVendor::start().await;
    let mut harness = Harness::new().with_file_package();
    harness.cloud.lifetime_cap = Duration::from_secs(2);
    harness.cloud.sweep = Duration::from_millis(100);
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic_at(&backend, &master, &vendor, "/a").await;
    create(&backend, &master, FIRST).await;
    let mut work = Driven::open(&backend, &master, &vendor, FIRST, &provider, "/a").await;
    // The model leaves a command running after its turn: it holds the Cloud,
    // and no idle stop would ever come.
    let left = work
        .turn(vec![
            shell("t1", "while :; do sleep 1; done; echo late"),
            say("left it running"),
        ])
        .await;
    assert!(
        left.received[0].starts_with("status: running"),
        "{}",
        left.received[0]
    );
    let device = the_cloud(&harness);
    eventually("the Cloud stops at its lifetime cap", || async {
        harness.manager.count(&format!("hibernate:{device}")) == 1
            && !harness.manager.running(&device)
    })
    .await;
    until_status(&backend, &master, "the Cloud is off", |status| {
        status.state == CloudState::Off
    })
    .await;
    backend.close().await;
}

// Several seconds: the Cloud boots and installs the builtin package, and three
// turns run a shell job each.
#[tokio::test]
async fn cloud_projects_share_the_users_one_machine_and_a_deleted_project_keeps_its_files() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new().with_file_package();
    let (backend, master) = harness.start_set_up().await;
    let project = |name: &str| {
        let body = json!({ "kind": "cloud", "name": name });
        let (backend, master) = (&backend, &master);
        async move {
            let created = backend.post("/api/workspaces", Some(master), body).await;
            assert_eq!(
                created.status,
                StatusCode::CREATED,
                "{}",
                String::from_utf8_lossy(&created.body)
            );
            created
                .json::<demi_web_api_protocol::workspaces::WorkspaceAnswer>()
                .workspace
        }
    };
    // Two projects made at once are directories of one Cloud, booted once.
    let (first, second) = tokio::join!(project("first"), project("second"));
    let device = the_cloud(&harness);
    assert_eq!(
        (first.device_id.as_str(), second.device_id.as_str()),
        (device.as_str(), device.as_str())
    );
    let home = harness.manager.home(&device);
    assert_eq!(first.path, format!("{home}/projects/{}", first.id));
    assert_ne!(first.path, second.path);
    assert!(std::path::Path::new(&second.path).is_dir());
    assert_eq!(harness.manager.count(&format!("wake:{device}")), 1);

    let first_model = anthropic_at(&backend, &master, &vendor, "/a").await;
    let second_model = anthropic_at(&backend, &master, &vendor, "/b").await;
    for (id, workspace) in [(FIRST, &first), (SECOND, &second)] {
        create(&backend, &master, id).await;
        let target = json!({ "target": { "kind": "workspace", "workspaceId": workspace.id } });
        let moved = backend
            .patch(&format!("/api/conversations/{id}"), &master, target)
            .await;
        assert_eq!(
            moved.status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&moved.body)
        );
    }
    let mut a = Driven::open(&backend, &master, &vendor, FIRST, &first_model, "/a").await;
    let mut b = Driven::open(&backend, &master, &vendor, SECOND, &second_model, "/b").await;
    let wrote = a
        .turn(vec![
            shell("a1", "printf shared > note"),
            say("written"),
        ])
        .await;
    assert!(
        wrote.received[0].contains("exitCode: 0"),
        "{}",
        wrote.received[0]
    );
    let read = format!("cat '{}/note'", first.path);
    let seen = b.turn(vec![shell("b1", &read), say("read")]).await;
    assert!(seen.received[0].contains("shared"), "{}", seen.received[0]);

    // Deleting a project archives its conversation and keeps its files.
    let deleted = backend
        .delete(&format!("/api/workspaces/{}", first.id), &master)
        .await;
    assert_eq!(
        deleted.status,
        StatusCode::NO_CONTENT,
        "{}",
        String::from_utf8_lossy(&deleted.body)
    );
    let archived: bool = harness
        .control_database()
        .query_row("SELECT archived FROM conversations WHERE id = ?1", [FIRST], |row| row.get(0))
        .unwrap();
    assert!(archived);
    let kept = b
        .turn(vec![shell("b2", &read), say("still there")])
        .await;
    assert!(kept.received[0].contains("shared"), "{}", kept.received[0]);
    backend.close().await;
}

// Over a second: the Cloud boots twice, and a paired device's runner starts.
//
// A page's watch of an attached Cloud does not wake it: stopped, it answers
// `offline` and closes, and once something else woke it, the page's next watch is
// live there.
#[tokio::test]
async fn a_watch_of_an_attached_cloud_answers_offline_while_it_is_stopped_without_waking_it() {
    let mut harness = Harness::new();
    harness.lifecycle = idle_after(Duration::from_millis(400));
    harness.cloud.sweep = Duration::from_millis(50);
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    create(&backend, &master, FIRST).await;
    let listed = backend.get(&format!("/api/conversations/{FIRST}/fs"), Some(&master)).await;
    assert_eq!(listed.status, StatusCode::OK, "{}", String::from_utf8_lossy(&listed.body));
    let device = the_cloud(&harness);
    let session = format!("{}/sessions/{FIRST}", harness.manager.home(&device));
    // Leaving the Cloud keeps it attached.
    crate::work::switch(&backend, &master, FIRST, &alpha, alpha.runner.home_dir()).await;
    until_status(&backend, &master, "the idle Cloud stops", |status| {
        status.state == CloudState::Off
    })
    .await;

    let route = format!("/api/conversations/{FIRST}/fs/watch");
    let subscribe = json!({ "type": "subscribe", "subscriptions": [
        { "key": "session", "kind": "folder_names", "path": session, "host": device },
    ] });
    let mut socket = files::watch_socket_of(&backend, &master, &route).await;
    files::send_watch(&mut socket, subscribe.clone()).await;
    assert_eq!(files::watch_message(&mut socket).await, json!({ "type": "offline" }));
    assert_eq!(files::watch_closed(&mut socket).await, (1011, "host_unreachable".to_owned()));
    assert_eq!(harness.manager.count(&format!("wake:{device}")), 1);

    // Browsing the Cloud wakes it, and a lease of the conversation's file
    // gate, its work, keeps it awake while the page watches again.
    let working = backend.file_gate(&master, FIRST).await.enter(Purpose::Demand).await;
    let browsed = backend
        .get(
            &format!("/api/conversations/{FIRST}/hosts/{device}/fs?path={}", query(&session)),
            Some(&master),
        )
        .await;
    assert_eq!(browsed.status, StatusCode::OK, "{}", String::from_utf8_lossy(&browsed.body));
    let mut socket = files::watch_socket_of(&backend, &master, &route).await;
    files::send_watch(&mut socket, subscribe).await;
    let live = files::watch_message(&mut socket).await;
    assert_eq!((&live["type"], &live["key"]), (&json!("live"), &json!("session")), "{live}");
    assert_eq!(harness.manager.count(&format!("wake:{device}")), 2);
    drop(working);
    backend.close().await;
}

/// Cost: one backend and one Cloud, which boots twice; about two seconds.
#[tokio::test]
async fn each_wake_names_the_cloud_storage_its_account_has_then() {
    let mut harness = Harness::new();
    harness.lifecycle = idle_after(Duration::from_millis(400));
    harness.cloud.sweep = Duration::from_millis(50);
    let (backend, master) = harness.start_set_up().await;
    let own = format!("/api/users/{}", master.user.id);
    let set = |system: u16, home: u16| json!({ "cloudStorage": { "systemGb": system, "homeGb": home } });
    assert_eq!(backend.patch(&own, &master, set(20, 40)).await.status, StatusCode::NO_CONTENT);
    create(&backend, &master, FIRST).await;
    let listing = format!("/api/conversations/{FIRST}/fs");
    let wake = || async {
        let working = backend
            .file_gate(&master, FIRST)
            .await
            .enter(Purpose::Demand)
            .await;
        let listed = backend.get(&listing, Some(&master)).await;
        assert_eq!(listed.status, StatusCode::OK, "{}", String::from_utf8_lossy(&listed.body));
        drop(working);
        until_status(&backend, &master, "the idle Cloud stops", |status| {
            status.state == CloudState::Off
        })
        .await;
    };
    wake().await;
    let device = the_cloud(&harness);
    assert_eq!(harness.manager.named_capacity(&device), (20 << 30, 40 << 30));

    // A change takes effect at the Cloud's next start.
    assert_eq!(backend.patch(&own, &master, set(8, 64)).await.status, StatusCode::NO_CONTENT);
    wake().await;
    assert_eq!(harness.manager.named_capacity(&device), (8 << 30, 64 << 30));
    backend.close().await;
}
