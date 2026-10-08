//! What the agent can change of its own conversation
//! (`sessions-and-targets.md` § What the agent can change, § Switch the
//! primary target): `demi host attach` attaches a device at once, which the
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

use crate::conversations::{FIRST, Socket, transcript};
use crate::subagents::{Scripts, WAIT, say, shell, tree_on};
use crate::support::{Harness, Paired, Session, TestBackend, eventually};

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

/// The devices attached to the conversation, by name, as storage holds them.
fn attached(harness: &Harness, id: &str) -> Vec<String> {
    harness
        .control_database()
        .prepare("SELECT name FROM conversation_hosts WHERE conversation_id = ?1 ORDER BY name")
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
    assert!(
        matches!(notice.event, AgentMessageEvent::MoveFailed {}),
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
