//! What the agent can change of its own conversation
//! (`sessions-and-targets.md` § What the agent can change, § Switch the
//! primary target): `demi host pair` pairs a computer whose installer shows
//! a code and attaches it, `demi host attach` attaches a device at once, which the
//! next context block announces; `demi host detach` and `demi conversation
//! move` wait until the conversation's work ends and are then made in one
//! transition; a move that fails then leaves the conversation where it was
//! and wakes the root with Demi's notice of the reason. The model is the
//! scripted family of the subagent scenarios, the devices real runners, and
//! the conversation holds both grants, as storage keeps them. No test calls
//! a real model.

use std::sync::Arc;

use demi_shared_types::{AgentMessageEvent, Block};
use reqwest::StatusCode;
use serde_json::json;

use demi_backend_remote_host::testing::{RunnerProcess, RunnerProcessOptions};

use crate::conversations::{FIRST, SECOND, Socket, choose, create, transcript, working_on};
use crate::subagents::{Scripts, WAIT, say, shell, tree_on};
use crate::support::{Harness, Paired, Session, TestBackend, eventually, stored_token};

/// Makes a project of a new directory of `device`'s home, named `name`, and
/// answers its id.
pub(crate) async fn project(
    backend: &TestBackend,
    session: &Session,
    device: &Paired,
    name: &str,
) -> String {
    let path = format!("{}/{name}", device.runner.home());
    std::fs::create_dir_all(&path).unwrap();
    let created = backend
        .post(
            "/api/workspaces",
            Some(session),
            json!({ "kind": "device", "deviceId": device.id(), "path": path, "name": name }),
        )
        .await;
    assert_eq!(
        created.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&created.body)
    );
    let answer: serde_json::Value = created.json();
    answer["workspace"]["id"].as_str().unwrap().to_owned()
}

/// The conversation's target as storage holds it: its kind, and the
/// workspace a project's names.
pub(crate) fn target_of(harness: &Harness, id: &str) -> (String, Option<String>) {
    harness
        .control_database()
        .query_row(
            "SELECT target_kind, target_workspace_id FROM conversations WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
}

/// The names of the devices attached to the conversation, as storage holds
/// them.
fn attached(harness: &Harness, id: &str) -> Vec<String> {
    harness
        .control_database()
        .prepare(
            "SELECT device.name FROM conversation_hosts host JOIN devices device ON device.id = host.device_id
             WHERE host.conversation_id = ?1 ORDER BY device.name",
        )
        .unwrap()
        .query_map([id], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// Whether a move waits for the conversation, as storage holds it.
fn pending(harness: &Harness, id: &str) -> bool {
    harness
        .control_database()
        .query_row(
            "SELECT pending_id IS NOT NULL FROM conversations WHERE id = ?1",
            [id],
            |row| row.get(0),
        )
        .unwrap()
}

/// Grants the conversation both categories, as an allow stores them.
fn grant_both(harness: &Harness, id: &str) {
    for category in ["conversation.organize", "host.devices"] {
        harness
            .control_database()
            .execute(
                "INSERT INTO permission_grants (conversation_id, category, granted_at) VALUES (?1, ?2, 1)",
                [id, category],
            )
            .unwrap();
    }
}

/// What the root of `id` was asked last.
fn last_asked(scripts: &Scripts, id: &str) -> String {
    scripts
        .asked(|session| session == id)
        .pop()
        .expect("the root was asked")
}

// Several seconds: two real devices, and one runs the shell jobs of a turn.
#[tokio::test]
async fn an_attach_applies_at_once_and_a_detach_and_a_move_wait_until_the_work_ends() {
    let scripts = Arc::new(Scripts::default());
    let (harness, backend, master, laptop, _, _) = tree_on(&scripts, Harness::new()).await;
    let _studio = backend.pair(&master, "studio").await;
    let notes = project(&backend, &master, &laptop, "notes").await;
    grant_both(&harness, FIRST);
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    scripts.root(
        FIRST,
        vec![
            shell("t1", "demi host attach studio"),
            shell("t2", "demi host detach studio && demi conversation move notes"),
            shell("t3", "demi host list && demi host current"),
            say("done"),
        ],
    );
    socket.chat("m1", "Use the studio, then tidy up").await;
    let asked = scripts.asked(|session| session == FIRST);
    // The attach applied at once, and the next request's context block
    // announced it.
    let after_attach = &asked[1];
    assert!(after_attach.contains("Attached studio as studio"), "{after_attach}");
    assert!(after_attach.contains("[Attached hosts changed]"), "{after_attach}");
    // The detach and the move waited: studio was still attached, and the
    // conversation still ran in its directory, for the turn's next command.
    let after_waits = &asked[3];
    assert!(
        after_waits.contains("studio is detached when this conversation's work ends"),
        "{after_waits}"
    );
    assert!(
        after_waits.contains("The move into notes (laptop, "),
        "{after_waits}"
    );
    assert!(after_waits.contains("(attached)"), "{after_waits}");
    // Nor did the turn's next request learn of a change: the attach's block
    // is the last it holds.
    assert!(after_waits.contains("[Execution context 1]"), "{after_waits}");
    assert!(!after_waits.contains("[Execution context 2]"), "{after_waits}");
    assert!(after_waits.contains("host: machine \\\"laptop\\\""), "{after_waits}");

    // Once the turn ended both were made, in one transition.
    eventually("the move and the detach are made", || async {
        target_of(&harness, FIRST) == ("workspace".to_owned(), Some(notes.clone()))
            && attached(&harness, FIRST).is_empty()
    })
    .await;
    assert!(!pending(&harness, FIRST));
    backend.close().await;
}

// Several seconds: two real runners, one of them paired by the agent.
#[tokio::test]
async fn an_agent_pairs_a_computer_from_its_installers_code_and_runs_commands_there() {
    let scripts = Arc::new(Scripts::default());
    let (harness, backend, master, _laptop, _, _) = tree_on(&scripts, Harness::new()).await;
    grant_both(&harness, FIRST);
    let state = backend.sync(&master).await.snapshot().await;
    // The computer the agent installed the runner on, which waits to be
    // paired under the code it shows.
    let computer = RunnerProcess::start(
        &backend.url,
        RunnerProcessOptions {
            name: "buildbox".into(),
            ..RunnerProcessOptions::default()
        },
    );
    let code = computer.pairing_code(0).await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    scripts.root(
        FIRST,
        vec![
            shell("t1", "demi host install-command --system linux"),
            shell("t2", "demi host pair 0000-0000-0000-0000-0000-0000-00"),
            shell("t3", &format!("demi host pair {code} --name build")),
            shell("t4", &format!("demi host pair {code}")),
            shell("t5", "demi host shell --host buildbox 'echo reached'"),
            say("done"),
        ],
    );
    socket.chat("m1", "Pair the build computer").await;
    let asked = scripts.asked(|session| session == FIRST);
    // The command the agent runs there is the one Add Device shows.
    assert!(asked[1].contains(&state.install_commands.linux), "{}", asked[1]);
    // A code no runner waits under says to read the newest one.
    assert!(
        asked[2].contains("read the newest one it shows and pass that"),
        "{}",
        asked[2]
    );
    // No agent names the host: `--name` is no option of the pair.
    assert!(asked[3].contains("unexpected argument '--name'"), "{}", asked[3]);
    // The live code pairs the computer and attaches it under its device's
    // name, the one the user sees.
    assert!(
        asked[4].contains("Paired as buildbox; attached as buildbox."),
        "{}",
        asked[4]
    );
    assert!(asked[5].contains("reached"), "{}", asked[5]);
    // It is the user's device, its runner holds its token, and the
    // conversation reaches it.
    let devices = backend.devices(&master).await;
    assert!(
        devices.iter().any(|device| device.name == "buildbox"),
        "{devices:?}"
    );
    assert!(!stored_token(&computer).await.is_empty());
    assert_eq!(attached(&harness, FIRST), ["buildbox"]);
    backend.close().await;
}

// Several seconds: a real device runs the shell jobs of two turns.
#[tokio::test]
async fn a_move_that_fails_leaves_the_conversation_in_place_and_wakes_the_root_with_the_reason() {
    let scripts = Arc::new(Scripts::default());
    let (harness, backend, master, laptop, root, _) = tree_on(&scripts, Harness::new()).await;
    let ledable = project(&backend, &master, &laptop, "ledable-app").await;
    grant_both(&harness, FIRST);
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    // The project is deleted while the turn that asked for the move runs.
    scripts.root(
        FIRST,
        vec![
            shell("t1", &format!("demi conversation move ledable-app; {WAIT}")),
            say("asked"),
        ],
    );
    scripts.root(FIRST, vec![say("noted")]);
    socket
        .send(&crate::conversations::send("m1", "Move into ledable-app"))
        .await;
    eventually("the move waits", || async { pending(&harness, FIRST) }).await;
    let deleted = backend
        .delete(&format!("/api/workspaces/{ledable}"), &master)
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    std::fs::write(format!("{root}/go"), "").unwrap();
    eventually("the root is told", || async {
        last_asked(&scripts, FIRST).contains("Demi could not make the move")
    })
    .await;
    let told = last_asked(&scripts, FIRST);
    assert!(
        told.contains(&format!(
            "Demi could not make the move this conversation's agent asked for: the project no longer exists. The conversation still runs on laptop ({root})."
        )),
        "{told}"
    );
    assert!(told.contains("\\\"sender\\\":\\\"demi\\\""), "{told}");
    socket.until_idle().await;
    assert_eq!(target_of(&harness, FIRST).0, "device");
    assert!(!pending(&harness, FIRST));
    let history = transcript(&backend, &master, FIRST).await;
    let notice = history
        .blocks
        .iter()
        .find_map(|block| match block {
            Block::AgentMessage(message) => Some(&message.message),
            _ => None,
        })
        .expect("the notice is in the transcript");
    assert!(notice.id.as_str().starts_with("move-failed:"), "{notice:?}");
    // The receipt row says why after Move failed, worded by the page from
    // the refusal's code; the model reads the reason in the message.
    assert!(
        matches!(
            &notice.event,
            AgentMessageEvent::MoveFailed { reason, code }
                if notice.content.contains(reason.as_str())
                    && code.as_deref() == Some("workspace_not_found")
        ),
        "{notice:?}"
    );
    assert_eq!(notice.sender, None);
    backend.close().await;
}

// Several seconds: the backend starts twice over the same data, and the
// project's device is a runner that has not reconnected.
#[tokio::test]
async fn a_move_a_restart_cut_off_is_made_at_the_next_start_even_to_an_offline_device() {
    let scripts = Arc::new(Scripts::default());
    let (harness, backend, master, _laptop, _, _) = tree_on(&scripts, Harness::new()).await;
    let mut studio = backend.pair(&master, "studio").await;
    let ledable = project(&backend, &master, &studio, "ledable-app").await;
    studio.runner.stop().await;
    let address = backend.address();
    backend.close().await;

    // The command recorded the move, and the backend stopped before the
    // tree was idle: the move as storage holds it.
    harness
        .control_database()
        .execute(
            "UPDATE conversations SET pending_id = 'm1', pending_kind = 'workspace', pending_workspace_id = ?2
             WHERE id = ?1",
            [FIRST, ledable.as_str()],
        )
        .unwrap();
    let backend = harness.start_at(address).await;
    eventually("the move is made", || async {
        target_of(&harness, FIRST) == ("workspace".to_owned(), Some(ledable.clone()))
    })
    .await;
    assert!(!pending(&harness, FIRST));
    backend.close().await;
}

// Several seconds: two real devices, a parent and its child on one, a second
// conversation on the other, and the child's shell waits across the rename.
#[tokio::test]
async fn a_device_rename_reaches_each_conversation_and_node_that_reaches_the_device_once() {
    let scripts = Arc::new(Scripts::default());
    let (harness, backend, master, _laptop, root, provider) = tree_on(&scripts, Harness::new()).await;
    let studio = backend.pair(&master, "studio").await;
    harness.attach(FIRST, studio.id());
    create(&backend, &master, SECOND).await;
    choose(&backend, &master, SECOND, &provider, "m").await;
    working_on(&harness, &studio, SECOND);
    let mut first = Socket::connect(&backend, &master, FIRST).await;
    first.open().await;
    let mut second = Socket::connect(&backend, &master, SECOND).await;
    second.open().await;

    // Each node reads the context once before the rename: the conversation
    // on the studio, the parent, and its child, whose shell then waits.
    scripts.root(SECOND, vec![say("hello")]);
    second.chat("s1", "Hello").await;
    scripts.child(vec![shell("c1", &format!("{WAIT}; demi host list")), say("listed")]);
    scripts.root(
        FIRST,
        vec![
            shell("t1", "demi agent spawn --description lister <<< 'List the hosts'"),
            say("dispatched"),
            // The child's completion wakes the idle parent.
            say("received"),
        ],
    );
    first.chat("m1", "Delegate the listing").await;
    let child = |session: &str| session != FIRST && session != SECOND;
    eventually("the child asked", || async { !scripts.asked(child).is_empty() }).await;

    let renamed = backend
        .patch(&format!("/api/devices/{}", studio.id()), &master, json!({ "name": "Build Box" }))
        .await;
    assert_eq!(renamed.status, StatusCode::OK);
    std::fs::write(format!("{root}/go"), "").unwrap();
    first.until_idle().await;
    scripts.root(FIRST, vec![say("again")]);
    first.chat("m2", "Anything new?").await;
    scripts.root(SECOND, vec![say("again")]);
    second.chat("s2", "Anything new?").await;
    scripts.root(SECOND, vec![say("still")]);
    second.chat("s3", "And now?").await;

    // Where the device is attached, each node's history holds one block
    // that names it anew and says the attached hosts changed, and no
    // switch: the child's for its request after its shell, the parent's
    // for its wake, which its next turn does not repeat.
    let attached = format!("Attached hosts: \\\"Build Box\\\" (host {}, ", studio.id());
    let parent = scripts.asked(|session| session == FIRST);
    let children = scripts.asked(child);
    for latest in [parent.last().unwrap(), children.last().unwrap()] {
        assert_eq!(latest.matches(&attached).count(), 1, "{latest}");
        assert_eq!(latest.matches("[Attached hosts changed]").count(), 1, "{latest}");
        assert!(!latest.contains("[Execution target switched]"), "{latest}");
    }
    assert!(!children[0].contains("Build Box"), "{}", children[0]);
    // Where it is the primary Host, the next block names it anew, once.
    let on_studio = scripts.asked(|session| session == SECOND);
    assert!(!on_studio[0].contains("Build Box"), "{}", on_studio[0]);
    let latest = on_studio.last().unwrap();
    assert_eq!(latest.matches("Primary host: Build Box").count(), 1, "{latest}");
    assert_eq!(latest.matches("[Execution context ").count(), 2, "{latest}");
    backend.close().await;
}
