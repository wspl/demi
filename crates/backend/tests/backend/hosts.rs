//! Where a conversation's work runs, changed through the API
//! (`crates/backend-host-access/src/transition.rs`,
//! `crates/backend-database/src/conversation_index.rs`).
//!
//! Scenarios:
//!
//! - A target switch moves the work and attaches the device it leaves.
//! - A switch ends the conversation's open transfers instead of waiting for
//!   them.
//! - A switch while a command runs applies at once: the command finishes
//!   where it started and reports, and the agent's next request names the new
//!   place.
//! - Attached hosts are detached, and each change of them raises the revision a
//!   page reads them by.
//! - An attached host's commands start where it was attached, whatever an
//!   earlier one did.
//!
//! The conversation's agents attach devices, which these tests write to
//! storage. The devices are real runners.

use std::path::PathBuf;

use demi_provider_common::testing::MockVendor;
use demi_web_api_protocol::auth::Role;
use demi_web_api_protocol::error::ErrorCode;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use crate::conversations::{Socket, anthropic_at};
use crate::support::{Answer, Harness, Paired, Session, TestBackend, pattern};
use crate::work::{Driven, say, shell};

const CONVERSATION: &str = "3c2b1a0f-8f3a-4c1e-9d2b-7a1c2e3f4a01";

/// A signed-in master with a conversation on the Cloud.
async fn conversation(harness: &Harness) -> (TestBackend, Session) {
    let (backend, master) = harness.start_set_up().await;
    let created = backend
        .post(
            "/api/conversations",
            Some(&master),
            json!({ "id": CONVERSATION }),
        )
        .await;
    assert_eq!(
        created.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&created.body)
    );
    (backend, master)
}

/// A directory in the device's home.
fn directory(device: &Paired, name: &str) -> PathBuf {
    let path = device.runner.home_dir().join(name);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn target(device: &Paired, path: &std::path::Path) -> Value {
    json!({ "kind": "device", "deviceId": device.id(), "path": path.to_str().unwrap() })
}

async fn switch(backend: &TestBackend, session: &Session, target: Value) -> Answer {
    backend
        .patch(
            &format!("/api/conversations/{CONVERSATION}"),
            session,
            json!({ "target": target }),
        )
        .await
}

async fn hosts(backend: &TestBackend, session: &Session) -> Vec<Value> {
    let listed = backend
        .get(
            &format!("/api/conversations/{CONVERSATION}/hosts"),
            Some(session),
        )
        .await;
    assert_eq!(
        listed.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&listed.body)
    );
    listed.json::<Value>()["hosts"].as_array().unwrap().clone()
}

/// The version of what the agent was told of where the conversation
/// works, which names the message that tells it of a move; the page reads
/// none of it.
fn context_version(harness: &Harness, id: &str) -> u64 {
    harness
        .control_database()
        .query_row("SELECT context_version FROM conversations WHERE id = ?1", [id], |row| row.get::<_, i64>(0))
        .unwrap()
        .try_into()
        .unwrap()
}

/// The conversation's metadata, as a page that shows it reads it.
async fn summary(backend: &TestBackend, session: &Session) -> Value {
    let detail: Value = backend
        .get(&format!("/api/conversations/{CONVERSATION}"), Some(session))
        .await
        .json();
    detail["metadata"].clone()
}

#[tokio::test]
async fn a_switch_moves_the_work_and_attaches_the_device_it_leaves() {
    let harness = Harness::new();
    let (backend, master) = conversation(&harness).await;
    let laptop = backend.pair(&master, "laptop").await;
    let ci = backend.pair(&master, "ci").await;
    let on_laptop = directory(&laptop, "work");
    let on_ci = directory(&ci, "build");

    let moved = switch(&backend, &master, target(&laptop, &on_laptop)).await;
    assert_eq!(
        moved.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&moved.body)
    );
    assert_eq!(
        moved.json::<Value>()["results"],
        json!([{ "field": "target", "status": "applied" }])
    );
    let before = summary(&backend, &master).await;
    assert_eq!(before["cwd"], json!(on_laptop.to_str().unwrap()));
    let version_before = context_version(&harness, CONVERSATION);
    // The Cloud was never used, so no device is left behind.
    assert!(hosts(&backend, &master).await.is_empty());

    // The files follow the target; what stayed on the laptop stays there.
    std::fs::write(on_laptop.join("report.txt"), "on the laptop").unwrap();
    let moved = switch(&backend, &master, target(&ci, &on_ci)).await;
    assert_eq!(
        moved.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&moved.body)
    );
    let after = summary(&backend, &master).await;
    assert_eq!(after["cwd"], json!(on_ci.to_str().unwrap()));
    assert!(context_version(&harness, CONVERSATION) > version_before);
    let listing: Value = backend
        .get(
            &format!("/api/conversations/{CONVERSATION}/fs"),
            Some(&master),
        )
        .await
        .json();
    assert_eq!(listing["path"], json!(on_ci.to_str().unwrap()));
    let attached = hosts(&backend, &master).await;
    assert_eq!(attached.len(), 1);
    assert_eq!(
        (
            &attached[0]["deviceId"],
            &attached[0]["name"],
            &attached[0]["cwd"],
            &attached[0]["state"]
        ),
        (
            &json!(laptop.id()),
            &json!("laptop"),
            &json!(on_laptop.to_str().unwrap()),
            &json!("online")
        )
    );
    let left: Value = backend
        .get(
            &format!("/api/conversations/{CONVERSATION}/hosts/{}/fs", laptop.id()),
            Some(&master),
        )
        .await
        .json();
    let names: Vec<&str> = left["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["report.txt"]);

    // Back to the laptop: it is primary alone, and the device left is attached.
    let back = switch(&backend, &master, target(&laptop, &on_laptop)).await;
    assert_eq!(
        back.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&back.body)
    );
    let attached = hosts(&backend, &master).await;
    let devices: Vec<&Value> = attached.iter().map(|host| &host["deviceId"]).collect();
    assert_eq!(devices, [&json!(ci.id())]);
    // The conversation's own target is no change.
    let unchanged = context_version(&harness, CONVERSATION);
    let same = switch(&backend, &master, target(&laptop, &on_laptop)).await;
    assert_eq!(same.status, StatusCode::OK);
    assert_eq!(context_version(&harness, CONVERSATION), unchanged);

    // What the destination names must be the user's.
    let missing = switch(
        &backend,
        &master,
        json!({ "kind": "workspace", "workspaceId": "nowhere" }),
    )
    .await;
    assert_eq!(
        missing.refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::WorkspaceNotFound)
    );
    let unknown = switch(
        &backend,
        &master,
        json!({ "kind": "device", "deviceId": "nothing", "path": "/" }),
    )
    .await;
    assert_eq!(
        unknown.refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::DeviceNotFound)
    );
    harness.add_user("user@example.test", "user-pass-1", Role::User);
    let other = backend.login("user@example.test", "user-pass-1").await;
    let theirs = backend.pair(&other, "theirs").await;
    let foreign = switch(&backend, &master, target(&theirs, &directory(&theirs, "x"))).await;
    assert_eq!(
        foreign.refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::DeviceNotFound)
    );
    // An archived conversation takes no switch.
    let archived = backend
        .patch(
            &format!("/api/conversations/{CONVERSATION}"),
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
    let refused = switch(&backend, &master, target(&ci, &on_ci)).await;
    assert_eq!(
        refused.refusal(),
        (StatusCode::CONFLICT, ErrorCode::ConversationArchived)
    );
    backend.close().await;
}

#[tokio::test]
async fn a_switch_ends_the_open_download_instead_of_waiting_for_it() {
    let harness = Harness::new();
    let (backend, master) = conversation(&harness).await;
    let laptop = backend.pair(&master, "laptop").await;
    let on_laptop = directory(&laptop, "work");
    assert_eq!(
        switch(&backend, &master, target(&laptop, &on_laptop))
            .await
            .status,
        StatusCode::OK
    );
    let video = pattern(64 * 1024 * 1024, 0);
    std::fs::write(on_laptop.join("long.mp4"), &video).unwrap();
    let path = on_laptop.join("long.mp4");
    let route = format!(
        "/api/conversations/{CONVERSATION}/fs/raw?{}",
        url::form_urlencoded::Serializer::new(String::new())
            .append_pair("path", path.to_str().unwrap())
            .finish()
    );
    let mut playing = backend
        .response(Method::GET, &route, &master, &[], None)
        .await;
    assert_eq!(playing.status(), StatusCode::OK);
    assert!(playing.chunk().await.unwrap().is_some());

    // The page reads nothing more; the switch ends the download and goes on.
    let moved = switch(&backend, &master, json!({ "kind": "cloud" })).await;
    assert_eq!(
        moved.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&moved.body)
    );
    let ended = loop {
        match playing.chunk().await {
            Ok(Some(_)) => {}
            Ok(None) => break "complete",
            Err(_) => break "cut",
        }
    };
    assert_eq!(ended, "cut");
    backend.close().await;
}

#[tokio::test]
async fn a_detach_takes_the_host_from_the_conversation_and_a_device_not_attached_detaches_as_nothing() {
    let harness = Harness::new();
    let (backend, master) = conversation(&harness).await;
    let laptop = backend.pair(&master, "laptop").await;
    let ci = backend.pair(&master, "ci").await;
    let spare = backend.pair(&master, "spare").await;
    assert_eq!(
        switch(
            &backend,
            &master,
            target(&laptop, &directory(&laptop, "work"))
        )
        .await
        .status,
        StatusCode::OK
    );
    harness.attach(CONVERSATION, ci.id());
    harness.attach(CONVERSATION, spare.id());
    let listed = hosts(&backend, &master).await;
    assert_eq!(
        (&listed[0]["name"], &listed[0]["cwd"], &listed[0]["state"]),
        (&json!("ci"), &Value::Null, &json!("online"))
    );
    let route = format!("/api/conversations/{CONVERSATION}/hosts");

    // A detach is a transition; a device that is not attached detaches as
    // nothing.
    let detached = backend
        .delete(&format!("{route}/{}", ci.id()), &master)
        .await;
    assert_eq!(detached.status, StatusCode::NO_CONTENT);
    let left: Vec<Value> = hosts(&backend, &master)
        .await
        .iter()
        .map(|host| host["deviceId"].clone())
        .collect();
    assert_eq!(left, [json!(spare.id())]);
    assert_eq!(
        backend
            .delete(&format!("{route}/{}", ci.id()), &master)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    // A detached device is no Host of the conversation.
    let gone = backend
        .get(&format!("{route}/{}/fs", ci.id()), Some(&master))
        .await;
    assert_eq!(
        gone.refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::HostNotAttached)
    );
    backend.close().await;
}

/// Waits until the page receives the conversation's metadata at the hosts
/// revision `revision`.
async fn until_hosts_revision(page: &mut Socket, revision: u64) {
    page.metadata_until(|metadata| metadata.hosts_revision == revision).await;
}

// About a second: two real devices install the builtin package, and a turn
// runs `demi host shell` from one on the other. Before the fix, the second
// shell started in `sub` and the hosts list recorded it.
#[tokio::test]
async fn an_attached_hosts_commands_start_where_it_was_attached_and_each_change_of_the_hosts_or_their_names_raises_the_revision()
 {
    let vendor = MockVendor::start().await;
    let harness = Harness::new().with_file_package();
    let (backend, master) = conversation(&harness).await;
    let laptop = backend.pair(&master, "laptop").await;
    let ci = backend.pair(&master, "ci").await;
    let work = directory(&laptop, "work");
    assert_eq!(
        switch(&backend, &master, target(&laptop, &work)).await.status,
        StatusCode::OK
    );
    harness.attach(CONVERSATION, ci.id());
    assert_eq!(summary(&backend, &master).await["hostsRevision"], json!(1));
    let route = format!("/api/conversations/{CONVERSATION}/hosts");

    // A device attached by name runs every command in its home: a `cd` in
    // one shell moves neither the next one nor the hosts list.
    let provider = anthropic_at(&backend, &master, &vendor, "/laptop").await;
    let mut driven = Driven::open(&backend, &master, &vendor, CONVERSATION, &provider, "/laptop").await;
    let moved = "demi host shell --host ci 'mkdir -p sub && cd sub' && demi host shell --host ci pwd";
    let ran = driven
        .turn(vec![shell("t1", moved, 20_000), say("moved")])
        .await;
    // The shell reports its directory with links resolved.
    let home = std::fs::canonicalize(ci.runner.home_dir()).unwrap();
    assert!(
        ran.received[0]
            .lines()
            .any(|line| line == home.to_str().unwrap()),
        "{}",
        ran.received[0]
    );
    assert!(!ran.received[0].contains("/sub"), "{}", ran.received[0]);
    assert_eq!(hosts(&backend, &master).await[0]["cwd"], Value::Null);
    assert_eq!(summary(&backend, &master).await["hostsRevision"], json!(1));

    // The attached host goes by its device's name: renamed in Settings, the
    // conversation's page reads it again under the new one.
    let renamed = backend
        .patch(&format!("/api/devices/{}", ci.id()), &master, json!({ "name": "Build Box" }))
        .await;
    assert_eq!(renamed.status, StatusCode::OK);
    until_hosts_revision(&mut driven.socket, 2).await;
    assert_eq!(hosts(&backend, &master).await[0]["name"], json!("Build Box"));

    let detached = backend.delete(&format!("{route}/{}", ci.id()), &master).await;
    assert_eq!(detached.status, StatusCode::NO_CONTENT);
    until_hosts_revision(&mut driven.socket, 3).await;
    backend.close().await;
}

// Several seconds: two real devices, and one runs a turn's shell job across
// the move.
#[tokio::test]
async fn a_move_while_a_command_runs_applies_at_once_and_the_command_finishes_where_it_started_and_reports() {
    use crate::conversations::{FIRST, send};
    use crate::subagents::{Scripts, WAIT, say as answer, shell as call, tree_on};

    let scripts = std::sync::Arc::new(Scripts::default());
    let (harness, backend, master, _laptop, root, _provider) = tree_on(&scripts, Harness::new()).await;
    let studio = backend.pair(&master, "studio").await;
    let site = std::fs::canonicalize(directory(&studio, "site")).unwrap();
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    scripts.root(
        FIRST,
        vec![
            call("t1", &format!("touch started; {WAIT}; echo \"finished in $(pwd -P)\"")),
            call("t2", "echo \"next in $(pwd -P)\""),
            answer("done"),
        ],
    );
    socket.send(&send("m1", "Build, then look around")).await;
    crate::support::eventually("the command runs", || async {
        std::path::Path::new(&root).join("started").exists()
    })
    .await;

    // The move answers at once, with the command still running.
    let moved = backend
        .patch(&format!("/api/conversations/{FIRST}"), &master, json!({ "target": target(&studio, &site) }))
        .await;
    assert_eq!(moved.status, StatusCode::OK, "{}", String::from_utf8_lossy(&moved.body));
    let primary: (String, String) = harness
        .control_database()
        .query_row("SELECT target_device_id, target_path FROM conversations WHERE id = ?1", [FIRST], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    assert_eq!(primary, (studio.id().to_owned(), site.display().to_string()));
    assert!(!scripts.asked(|session| session == FIRST).iter().any(|asked| asked.contains("finished in")));

    // The command finishes where it started and reports as usual; the
    // agent's next request names the new place, where its next command runs.
    std::fs::write(format!("{root}/go"), "").unwrap();
    socket.until_idle().await;
    let asked = scripts.asked(|session| session == FIRST);
    let real_root = std::fs::canonicalize(&root).unwrap();
    assert!(asked[1].contains(&format!("finished in {}", real_root.display())), "{}", asked[1]);
    assert!(asked[1].contains("[Execution target switched]"), "{}", asked[1]);
    assert!(asked[1].contains(&site.display().to_string()), "{}", asked[1]);
    assert!(asked[2].contains(&format!("next in {}", site.display())), "{}", asked[2]);
    backend.close().await;
}
