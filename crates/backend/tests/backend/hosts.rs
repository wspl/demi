//! Where a conversation's work runs, changed through the API
//! (`sessions-and-targets.md` § Switch the primary target, § Attached hosts;
//! `web-api.md` § Workspaces, devices, and attached hosts): a target switch
//! moves the work and attaches the device it leaves, a switch ends the
//! conversation's open transfers instead of waiting for them, attached
//! hosts are detached, each change of them raises the revision a page
//! reads them by, and an attached host's commands start where it was
//! attached, whatever an earlier one did. The
//! conversation's agents attach devices, which these tests write to
//! storage. The devices are real runners.

use std::path::PathBuf;

use demi_provider_common::testing::MockVendor;
use demi_web_api_protocol::auth::Role;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::state::SyncEvent;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use crate::conversations::anthropic_at;
use crate::support::{Answer, Harness, Paired, Session, SyncChannel, TestBackend, pattern};
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

/// The conversation as the list shows it.
async fn summary(backend: &TestBackend, session: &Session) -> Value {
    let listed: Value = backend
        .get("/api/conversations?archived=false", Some(session))
        .await
        .json();
    listed["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|conversation| conversation["id"] == CONVERSATION)
        .unwrap()
        .clone()
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
    assert!(after["contextVersion"].as_u64() > before["contextVersion"].as_u64());
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
    let unchanged = summary(&backend, &master).await;
    let same = switch(&backend, &master, target(&laptop, &on_laptop)).await;
    assert_eq!(same.status, StatusCode::OK);
    assert_eq!(
        summary(&backend, &master).await["contextVersion"],
        unchanged["contextVersion"]
    );

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
    harness.attach(CONVERSATION, ci.id(), "ci");
    harness.attach(CONVERSATION, spare.id(), "spare");
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

/// Waits until the page receives the conversation's summary at the hosts
/// revision `revision`.
async fn until_hosts_revision(page: &mut SyncChannel, revision: u64) {
    page.until(|event| {
        matches!(
            event,
            SyncEvent::Conversation { conversation }
                if conversation.id.as_str() == CONVERSATION && conversation.hosts_revision == revision
        )
    })
    .await;
}

// About a second: two real devices install the builtin package, and a turn
// runs `demi host shell` from one on the other. Before the fix, the second
// shell started in `sub` and the hosts list recorded it.
#[tokio::test]
async fn an_attached_hosts_commands_start_where_it_was_attached_and_each_change_of_the_hosts_raises_the_revision()
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
    harness.attach(CONVERSATION, ci.id(), "builder");
    let mut page = backend.sync(&master).await;
    let state = page.snapshot().await;
    assert_eq!(state.conversations[0].hosts_revision, 1);
    let route = format!("/api/conversations/{CONVERSATION}/hosts");

    // A device attached by name runs every command in its home: a `cd` in
    // one shell moves neither the next one nor the hosts list.
    let provider = anthropic_at(&backend, &master, &vendor, "/laptop").await;
    let mut driven = Driven::open(&backend, &master, &vendor, CONVERSATION, &provider, "/laptop").await;
    let moved = "demi host shell --host builder 'mkdir -p sub && cd sub' && demi host shell --host builder pwd";
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

    let detached = backend.delete(&format!("{route}/{}", ci.id()), &master).await;
    assert_eq!(detached.status, StatusCode::NO_CONTENT);
    until_hosts_revision(&mut page, 2).await;
    backend.close().await;
}

// About a second: a real device runs two turns of the scripted model, the
// second woken by the move the user told the agent of.
#[tokio::test]
async fn a_move_told_to_the_agent_wakes_the_root_with_one_message_and_a_move_alone_admits_none() {
    use demi_shared_types::{AgentMessageEvent, Block};

    use crate::conversations::{FIRST, Socket, summary as listed, transcript};
    use crate::subagents::{Scripts, say as answer, tree_on};

    let scripts = std::sync::Arc::new(Scripts::default());
    let (_harness, backend, master, paired, root, _provider) = tree_on(&scripts, Harness::new()).await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    scripts.root(FIRST, vec![answer("ready")]);
    socket.chat("m1", "Start").await;
    let moves = |path: &str, notify: bool| {
        let quiet = format!("{root}/{path}");
        std::fs::create_dir_all(&quiet).unwrap();
        let mut body = json!({
            "target": { "kind": "device", "deviceId": paired.id(), "path": quiet }
        });
        if notify {
            body["notifyAgent"] = json!(true);
        }
        let (backend, master) = (&backend, &master);
        async move {
            backend
                .patch(&format!("/api/conversations/{FIRST}"), master, body)
                .await
        }
    };
    let agent_messages = |blocks: &[Block]| -> Vec<demi_shared_types::AgentMessage> {
        blocks
            .iter()
            .filter_map(|block| match block {
                Block::AgentMessage(block) => Some(block.message.clone()),
                _ => None,
            })
            .collect()
    };

    // Telling the agent needs a switch to tell.
    let alone = backend
        .patch(
            &format!("/api/conversations/{FIRST}"),
            &master,
            json!({ "notifyAgent": true }),
        )
        .await;
    assert_eq!(
        alone.refusal(),
        (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody)
    );

    // A move alone: the answer comes once the switch is made, and nothing
    // reached the agent.
    let quiet = moves("quiet", false).await;
    assert_eq!(quiet.status, StatusCode::OK, "{}", String::from_utf8_lossy(&quiet.body));
    let blocks = transcript(&backend, &master, FIRST).await.blocks;
    assert!(agent_messages(&blocks).is_empty(), "{blocks:?}");
    let asked = scripts.asked(|session| session == FIRST).len();
    assert_eq!(asked, 1);

    // Told: one message from the user, under the revision the move made,
    // wakes the idle root, which reads where it was and where it is.
    scripts.root(FIRST, vec![answer("noted")]);
    let told = moves("told", true).await;
    assert_eq!(told.status, StatusCode::OK, "{}", String::from_utf8_lossy(&told.body));
    socket.until_idle().await;
    let woken = scripts.asked(|session| session == FIRST);
    assert_eq!(woken.len(), 2);
    let expected = format!(
        "The user moved this conversation from laptop ({root}/quiet) to laptop ({root}/told). Files did not move. Check what this means for the work so far, and tell the user."
    );
    assert!(woken[1].contains(&expected), "{}", woken[1]);
    let revision = listed(&backend, &master, FIRST).await.context_version;
    let messages = agent_messages(&transcript(&backend, &master, FIRST).await.blocks);
    assert_eq!(messages.len(), 1, "{messages:?}");
    let message = &messages[0];
    assert_eq!(message.id.as_str(), format!("moved:{revision}"));
    assert_eq!(message.sender, None);
    let AgentMessageEvent::Moved { host, path, home } = &message.event else {
        panic!("{message:?}");
    };
    assert_eq!((host.as_str(), path.as_str()), ("laptop", format!("{root}/told").as_str()));
    assert!(home.as_deref().is_some_and(|home| path.starts_with(home)), "{home:?}");
    backend.close().await;
}
