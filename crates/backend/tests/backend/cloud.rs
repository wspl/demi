//! The Cloud through the backend (`managed-hosts.md`,
//! `sessions-and-targets.md` § Resolve a target, § How a conversation uses a
//! device, `resource-lifecycle.md`): the first uses of two conversations
//! boot one Cloud, which keeps nothing of an ended command, an idle Cloud
//! stops, and the next operation wakes it, a conversation that idles on a
//! Cloud another keeps running hears its release there, one archived while
//! the Cloud is stopped does not wake it, a Cloud the manager stopped
//! without a word boots again, a conversation that left the Cloud still
//! reaches it, a reset keeps home and identity and tells the model, a failed
//! reset resumes, a reset holds the conversations that need the Cloud and
//! leaves the ones that only have it attached, capacity counts across users,
//! a Cloud that keeps dying stops booting until a reset, a boot whose runner
//! never connects fails, and a shutdown saves the Cloud. The machine manager
//! is the scripted one; each Cloud's runner is real, and the model is an
//! Anthropic endpoint the test scripts.

use std::sync::Arc;
use std::time::Duration;

use demi_backend_user_shard::tuning::LifecycleTuning;
use demi_provider_common::testing::MockVendor;
use demi_shared_gates::Purpose;
use demi_web_api_protocol::cloud::{CloudResetAnswer, CloudState, CloudStatus, ResetPhase};
use demi_web_api_protocol::devices::{DeviceKind, DeviceState};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::files::{Directory, FileText};
use demi_web_api_protocol::hosts::AttachedHosts;
use reqwest::StatusCode;
use serde_json::json;
use tokio::sync::Notify;

use crate::conversations::{Socket, anthropic_at, choose, create};
use crate::families::{self, ScriptedKey};
use crate::streams::{self, received};
use crate::support::{Harness, PATIENCE, Session, TestBackend, eventually};
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

/// `harness` with the family `process` too, whose provider runs a process
/// on a Host, which the user's Cloud then is, and answers every run with
/// `ok`.
fn with_process_family(harness: Harness) -> Harness {
    let process = ScriptedKey {
        directory: Arc::new(families::Directory::default()),
        wires: &[],
        process_host: true,
    };
    harness.with_families(demi_backend::families::builtin().with("process", process))
}

/// The master's entry of the family `process`, with its one model `m`.
async fn process_entry(backend: &TestBackend, master: &Session) -> String {
    let created = backend
        .post(
            "/api/providers",
            Some(master),
            json!({
                "source": "custom", "providerType": "process", "label": "Process", "apiKey": "k",
                "models": [{
                    "id": "m", "displayName": "M", "contextWindow": 100000, "outputLimit": null,
                    "thinkingEfforts": [], "acceptedExtensions": null, "fastTier": null
                }]
            }),
        )
        .await;
    assert_eq!(
        created.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&created.body)
    );
    created
        .json::<demi_web_api_protocol::providers::ProviderAnswer>()
        .provider
        .id
        .as_str()
        .to_owned()
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
        a.turn(vec![shell("a1", "echo 0 > note", 20_000), say("a wrote")]),
        b.turn(vec![shell("b1", "echo 1 > note", 20_000), say("b wrote")]),
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
        .turn(vec![shell("a2", "cat note", 20_000), say("awake")])
        .await;
    assert!(next.received[0].contains("0\n"), "{}", next.received[0]);
    assert_eq!(harness.manager.count(&format!("wake:{device}")), 2);
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
        a.turn(vec![shell("a1", "echo ran", 20_000), say("ran")]),
        b.turn(vec![shell("b1", "echo ran", 20_000), say("ran")]),
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
        .turn(vec![shell("t1", "echo ran", 20_000), say("ran")])
        .await;
    assert!(
        ran.received[0].contains("exitCode: 0"),
        "{}",
        ran.received[0]
    );
    let device = the_cloud(&harness);

    // The backend's close stops the Cloud. Archived while the Cloud is
    // stopped, the conversation's release does not wake it: the stop ended
    // what a release would.
    let address = backend.address();
    backend.close().await;
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
            shell("t1", "echo retained > note", 20_000),
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
    harness.attach(FIRST, alpha.id(), "alpha");
    let script = "echo report > report.txt && demi host shell --host alpha 'cat notes.txt'";
    let from_cloud = work
        .turn(vec![shell("t1", script, 30_000), say("reached alpha")])
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
    let hosts: AttachedHosts = backend
        .get(&format!("/api/conversations/{FIRST}/hosts"), Some(&master))
        .await
        .json();
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
                30_000,
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
            shell("t1", "echo retained > note", 20_000),
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
        "[Execution context 0]\\nPrimary host: Cloud. Shells start in /home/demi/sessions/{FIRST}.\\nAttached hosts: none."
    );
    assert!(
        wrote.first_request().contains(&unmade),
        "{}",
        wrote.first_request()
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
        .turn(vec![shell("t2", "cat note", 20_000), say("read")])
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
    let home = harness.manager.home(&the_cloud(&harness));
    let named = format!(
        " ({}). Shells start in {home}/sessions/{FIRST}.\\n[Cloud reset {RESET}]",
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
            shell("t1", "printf retained > note", 20_000),
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
            shell("c1", "echo kept > note", 20_000),
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
    let hosts: AttachedHosts = backend
        .get(&format!("/api/conversations/{SECOND}/hosts"), Some(&master))
        .await
        .json();
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
        .turn(vec![shell("l1", "echo on-alpha", 20_000), say("done")])
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
        .turn(vec![shell("c2", "cat note", 20_000), say("read")])
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
    assert!(
        refused.error().message.contains("boot timeout"),
        "{}",
        refused.error().message
    );
    let device = the_cloud(&harness);
    assert_eq!(harness.manager.count(&format!("hibernate:{device}")), 1);
    let failed = status(&backend, &master).await;
    assert_eq!(failed.state, CloudState::Off);
    assert!(failed.error.unwrap().contains("boot timeout"));

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
        .turn(vec![shell("t1", script, 20_000), say("stored")])
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

    // The backend's close saves the Cloud; the next start boots nothing
    // until a command needs it. It starts at the address it had: the Cloud's
    // runner keeps its state, which names that address as its backend's.
    let address = backend.address();
    backend.close().await;
    let backend = harness.start_at(address).await;
    work.reconnect(&backend, &master, FIRST, &provider).await;
    let found = work
        .turn(vec![
            shell("t2", "cat notes.md", 20_000),
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
    assert_eq!(harness.manager.count(&format!("wake:{device}")), 2);
    backend.close().await;
}

// One backend and no runner; a broken shutdown reaches the 20-second test
// guard. The runner connection timeout stays at its production default.
#[tokio::test]
async fn shutdown_cancels_a_cloud_boot_waiting_for_its_runner_and_saves_it_once() {
    let harness = Harness::new().with_claude_package();
    harness.manager.script(|script| script.silent_wake = true);
    let (backend, master) = harness.start_set_up().await;
    // A Claude sign-in runs the CLI's login on the Cloud, which it wakes.
    let login = backend
        .post(
            "/api/providers/subscription-login",
            Some(&master),
            json!({ "providerType": "claude-code" }),
        )
        .await;
    assert_eq!(login.status, StatusCode::ACCEPTED);
    eventually("the sign-in wakes the Cloud", || async {
        harness
            .manager
            .calls()
            .iter()
            .any(|call| call.starts_with("wake:"))
    })
    .await;
    let device = the_cloud(&harness);

    tokio::time::timeout(PATIENCE, backend.close())
        .await
        .expect("shutdown must not wait out the runner connection timeout");
    assert_eq!(harness.manager.count(&format!("hibernate:{device}")), 1);
}

#[tokio::test]
async fn shutdown_ends_an_open_download_saves_the_cloud_and_reports_a_save_that_failed() {
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

    harness
        .manager
        .script(|script| script.fail_hibernate = Some("the disk is full".into()));
    let url = backend.url.clone();
    let closed = backend.close_reporting().await.unwrap_err().to_string();
    assert!(
        closed.contains("a Cloud was not saved") && closed.contains("the disk is full"),
        "{closed}"
    );
    // The download ended rather than kept the Cloud from its save.
    drop(download);
    let calls = harness.manager.calls();
    let saved = calls
        .iter()
        .position(|call| *call == format!("hibernate:{device}"));
    let reconciled = calls.iter().rposition(|call| call == "reconcile");
    assert!(saved.is_some() && saved < reconciled, "{calls:?}");
    // The listener is gone.
    assert!(reqwest::get(format!("{url}/api/setup")).await.is_err());
}

#[tokio::test]
async fn a_reset_holds_a_conversation_on_a_paired_device_whose_provider_runs_on_the_cloud() {
    let harness = with_process_family(Harness::new());
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    let provider = process_entry(&backend, &master).await;
    // Its files and commands are on alpha; only its provider uses the Cloud.
    create(&backend, &master, FIRST).await;
    crate::work::switch(&backend, &master, FIRST, &alpha, alpha.runner.home_dir()).await;
    choose(&backend, &master, FIRST, &provider, "m").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    drop(socket);

    let (held, proceed) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    harness
        .manager
        .script(|script| script.hold_reset = Some((held.clone(), proceed.clone())));
    reset(&backend, &master, RESET).await;
    held.notified().await;
    // It waits instead of failing, at its file gate, which the reset holds,
    // and opens by itself when the reset ends.
    let mut waiting = backend.file_gate(&master, FIRST).await.waiting();
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    {
        let opening = socket.open();
        tokio::pin!(opening);
        tokio::select! {
            _ = &mut opening => panic!("the open did not wait for the reset"),
            waits = waiting.wait_for(|count| *count > 0) => assert!(waits.is_ok()),
        }
        proceed.notify_one();
        opening.await;
    }
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

    // Work on the paired target, with the Cloud attached, keeps it awake
    // past its idle window: it stops a full window after the conversation
    // rests, and not before.
    // The last activity ends after its request was sent, so the window is
    // counted from the sending.
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
        Some("Reset disks recovered; retry to start Cloud")
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
    // made at the recovery, advances the conversation's context no further.
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
            shell("t1", "sleep 30; echo late", 1_000),
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
            shell("a1", "printf shared > note", 20_000),
            say("written"),
        ])
        .await;
    assert!(
        wrote.received[0].contains("exitCode: 0"),
        "{}",
        wrote.received[0]
    );
    let read = format!("cat '{}/note'", first.path);
    let seen = b.turn(vec![shell("b1", &read, 20_000), say("read")]).await;
    assert!(seen.received[0].contains("shared"), "{}", seen.received[0]);

    // A project a conversation targets stays; once none does, it goes and
    // its files stay.
    let in_use = backend
        .delete(&format!("/api/workspaces/{}", first.id), &master)
        .await;
    assert_eq!(
        in_use.refusal(),
        (StatusCode::CONFLICT, ErrorCode::WorkspaceInUse)
    );
    let away = backend
        .patch(
            &format!("/api/conversations/{FIRST}"),
            &master,
            json!({ "target": { "kind": "cloud" } }),
        )
        .await;
    assert_eq!(
        away.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&away.body)
    );
    let deleted = backend
        .delete(&format!("/api/workspaces/{}", first.id), &master)
        .await;
    assert_eq!(
        deleted.status,
        StatusCode::NO_CONTENT,
        "{}",
        String::from_utf8_lossy(&deleted.body)
    );
    let kept = b
        .turn(vec![shell("b2", &read, 20_000), say("still there")])
        .await;
    assert!(kept.received[0].contains("shared"), "{}", kept.received[0]);
    backend.close().await;
}
