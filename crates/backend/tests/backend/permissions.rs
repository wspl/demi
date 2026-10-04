//! Conversation permissions (`permissions.md` § Acceptance): a `demi skills`
//! command whose category the conversation has not been granted fails at
//! once and raises a request the pages list; the user's Allow grants the
//! category for the whole tree and tells the agent that asked, waking it,
//! even after the user stopped its turn; a Deny tells it and remembers
//! nothing; a newer command replaces an undecided request; a revocation
//! makes the next command ask again; an archive withdraws the requests and
//! keeps the grants; a Fork copies none; and a decision a restart cut off
//! reaches the agent at the next start. The model is the scripted family of
//! the subagent scenarios; the device is a real runner. No test calls a real
//! model.

use std::sync::Arc;

use demi_plugin_skills::testing::{Repos, skill_md};
use demi_shared_types::Block;
use demi_web_api_protocol::error::{ErrorBody, ErrorCode};
use demi_web_api_protocol::permissions::{ConversationPermissions, RequestingAgent};
use reqwest::StatusCode;
use serde_json::json;

use crate::conversations::{FIRST, SECOND, Socket, choose, create, summary};
use crate::subagents::{Scripts, WAIT, say, shell, tree_on};
use crate::support::{Harness, Session, TestBackend, eventually};

/// What a refused command prints, which the agent reads.
const REFUSED: &str = "demi: this conversation needs the user's permission to manage skills; the request was sent to the user, and you will be told when the user decides";

/// The repository `acme/tools` with the skills `review` and `lint`.
fn repos() -> Arc<Repos> {
    let repos = Arc::new(Repos::new());
    repos.commit(
        "acme/tools",
        &[
            (
                "review/SKILL.md",
                &skill_md("name: review\ndescription: Review a change."),
                false,
            ),
            (
                "lint/SKILL.md",
                &skill_md("name: lint\ndescription: Lint a change."),
                false,
            ),
        ],
    );
    repos
}

/// A backend of the scripted family with the skill repositories, the
/// conversation `FIRST` open on a socket, and the device it works on.
struct World {
    harness: Harness,
    backend: TestBackend,
    master: Session,
    socket: Socket,
    /// Held for its runner process.
    _device: crate::support::Paired,
    provider: String,
}

async fn opened(scripts: &Arc<Scripts>) -> World {
    let harness = Harness::new().with_skill_repos(&repos());
    let (harness, backend, master, device, _, provider) = tree_on(scripts, harness).await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    World {
        harness,
        backend,
        master,
        socket,
        _device: device,
        provider,
    }
}

async fn permissions(backend: &TestBackend, session: &Session, id: &str) -> ConversationPermissions {
    let read = backend
        .get(&format!("/api/conversations/{id}/permissions"), Some(session))
        .await;
    assert_eq!(
        read.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&read.body)
    );
    read.json()
}

/// The only undecided request of the conversation: its id.
async fn the_request(backend: &TestBackend, session: &Session, id: &str) -> String {
    let read = permissions(backend, session, id).await;
    assert_eq!(read.requests.len(), 1, "{read:?}");
    read.requests[0].id.as_str().to_owned()
}

async fn decide(
    backend: &TestBackend,
    session: &Session,
    id: &str,
    request: &str,
    decision: &str,
) -> crate::support::Answer {
    backend
        .post(
            &format!("/api/conversations/{id}/permissions/requests/{request}"),
            Some(session),
            json!({ "decision": decision }),
        )
        .await
}

/// How many skill sources the user has: the plugin's values.
fn sources(harness: &Harness) -> i64 {
    harness
        .control_database()
        .query_row(
            "SELECT count(*) FROM plugin_values WHERE plugin = 'skills'",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

/// What the root of `id` was asked last.
fn last_asked(scripts: &Scripts, id: &str) -> String {
    scripts
        .asked(|session| session == id)
        .pop()
        .expect("the root was asked")
}

fn granted(read: &ConversationPermissions) -> Vec<&str> {
    read.grants
        .iter()
        .map(|grant| grant.category.id.as_str())
        .collect()
}

// Several seconds: a real device runs the shell jobs of five turns, two of
// them woken by the user's decisions.
#[tokio::test]
async fn a_command_without_the_grant_asks_and_the_users_allow_grants_the_category_and_wakes_the_agent()
{
    let scripts = Arc::new(Scripts::default());
    let World {
        harness,
        backend,
        master,
        mut socket,
        _device,
        provider,
    } = opened(&scripts).await;

    scripts.root(
        FIRST,
        vec![
            shell("t1", "demi skills add acme/tools; echo exit=$?"),
            say("I asked the user"),
        ],
    );
    socket.chat("m1", "Install acme/tools").await;
    let refused = last_asked(&scripts, FIRST);
    assert!(refused.contains(REFUSED), "{refused}");
    assert!(refused.contains("exit=1"), "{refused}");
    // The plugin never received the command: nothing was fetched or added.
    assert_eq!(sources(&harness), 0);
    let asked = permissions(&backend, &master, FIRST).await;
    assert_eq!(asked.requests.len(), 1, "{asked:?}");
    let request = &asked.requests[0];
    assert_eq!(request.command, "demi skills add acme/tools");
    assert_eq!(request.agent, None);
    assert_eq!(request.category.id, "skills.manage");
    assert_eq!(request.category.action.as_deref(), Some("manage skills"));
    assert!(asked.grants.is_empty());
    assert_eq!(summary(&backend, &master, FIRST).await.permission_requests, 1);

    // Allow: the idle root starts a turn whose input is the allowed message,
    // and its command now runs.
    scripts.root(
        FIRST,
        vec![shell("t2", "demi skills add acme/tools"), say("added")],
    );
    let allowed = decide(&backend, &master, FIRST, request.id.as_str(), "allow").await;
    assert_eq!(allowed.status, StatusCode::NO_CONTENT);
    socket.until_idle().await;
    let woken = scripts.asked(|session| session == FIRST);
    let opened_with = &woken[woken.len() - 2];
    assert!(
        opened_with.contains(
            "The user allowed this conversation to manage skills; the command `demi skills add acme/tools` can now run."
        ),
        "{opened_with}"
    );
    let ran = woken.last().unwrap();
    assert!(ran.contains("Added acme/tools at"), "{ran}");
    assert!(ran.contains("Turned on: lint, review"), "{ran}");
    assert_eq!(sources(&harness), 1);
    let read = permissions(&backend, &master, FIRST).await;
    assert!(read.requests.is_empty(), "{read:?}");
    assert_eq!(granted(&read), ["skills.manage"]);
    assert_eq!(summary(&backend, &master, FIRST).await.permission_requests, 0);

    // The grant is the conversation's alone: another conversation on the
    // same device asks again.
    create(&backend, &master, SECOND).await;
    choose(&backend, &master, SECOND, &provider, "m").await;
    harness
        .control_database()
        .execute(
            "UPDATE conversations SET target_kind = 'device', target_device_id = \
             (SELECT target_device_id FROM conversations WHERE id = ?1), \
             target_path = (SELECT target_path FROM conversations WHERE id = ?1) WHERE id = ?2",
            rusqlite::params![FIRST, SECOND],
        )
        .unwrap();
    let mut second = Socket::connect(&backend, &master, SECOND).await;
    second.open().await;
    scripts.root(
        SECOND,
        vec![
            shell("s1", "demi skills disable acme/tools --skill lint"),
            say("asked"),
        ],
    );
    second.chat("n1", "Turn lint off").await;
    assert!(last_asked(&scripts, SECOND).contains(REFUSED));
    let other = permissions(&backend, &master, SECOND).await;
    assert_eq!(other.requests.len(), 1, "{other:?}");
    assert!(other.grants.is_empty());
    backend.close().await;
}

// Several seconds: a real device runs the shell jobs of seven turns.
#[tokio::test]
async fn a_newer_command_replaces_the_request_a_deny_remembers_nothing_and_a_revocation_asks_again() {
    let scripts = Arc::new(Scripts::default());
    // The harness holds the backend's data, which dropping it removes.
    let World {
        harness: _harness,
        backend,
        master,
        mut socket,
        _device,
        ..
    } = opened(&scripts).await;

    // Twice before a decision: one request, with the newer command line.
    scripts.root(
        FIRST,
        vec![
            shell("t1", "demi skills add acme/tools"),
            shell("t2", "demi skills add acme/tools --skill review"),
            say("asked"),
        ],
    );
    socket.chat("m1", "Install review").await;
    let read = permissions(&backend, &master, FIRST).await;
    assert_eq!(read.requests.len(), 1, "{read:?}");
    assert_eq!(
        read.requests[0].command,
        "demi skills add acme/tools --skill review"
    );
    let newer = read.requests[0].id.as_str().to_owned();

    // Deny: the agent is told, and nothing is remembered.
    scripts.root(FIRST, vec![say("understood")]);
    let denied = decide(&backend, &master, FIRST, &newer, "deny").await;
    assert_eq!(denied.status, StatusCode::NO_CONTENT);
    socket.until_idle().await;
    let told = last_asked(&scripts, FIRST);
    assert!(
        told.contains(
            "The user denied this conversation permission to manage skills; the command `demi skills add acme/tools --skill review` was not run."
        ),
        "{told}"
    );
    let read = permissions(&backend, &master, FIRST).await;
    assert!(read.requests.is_empty() && read.grants.is_empty(), "{read:?}");
    // A request decided once is decided: a second page's answer is 404.
    let again = decide(&backend, &master, FIRST, &newer, "allow").await;
    assert_eq!(again.status, StatusCode::NOT_FOUND);
    assert_eq!(
        again.json::<ErrorBody>().code,
        ErrorCode::PermissionRequestNotFound
    );

    // The next attempt asks again; this time the user allows it.
    scripts.root(
        FIRST,
        vec![shell("t3", "demi skills add acme/tools"), say("asked")],
    );
    socket.chat("m2", "Try again").await;
    assert!(last_asked(&scripts, FIRST).contains(REFUSED));
    let request = the_request(&backend, &master, FIRST).await;
    scripts.root(FIRST, vec![say("thanks")]);
    let allowed = decide(&backend, &master, FIRST, &request, "allow").await;
    assert_eq!(allowed.status, StatusCode::NO_CONTENT);
    socket.until_idle().await;
    assert_eq!(
        granted(&permissions(&backend, &master, FIRST).await),
        ["skills.manage"]
    );

    // Revoked: the next command of the category asks again.
    let revoked = backend
        .delete(
            &format!("/api/conversations/{FIRST}/permissions/grants/skills.manage"),
            &master,
        )
        .await;
    assert_eq!(revoked.status, StatusCode::NO_CONTENT);
    scripts.root(
        FIRST,
        vec![shell("t4", "demi skills add acme/tools"), say("asked")],
    );
    socket.chat("m3", "Once more").await;
    assert!(last_asked(&scripts, FIRST).contains(REFUSED));
    let read = permissions(&backend, &master, FIRST).await;
    assert_eq!(read.requests.len(), 1, "{read:?}");
    assert!(read.grants.is_empty());
    backend.close().await;
}

// Several seconds: two subagents and their parent run shell jobs on a real
// device.
#[tokio::test]
async fn a_closed_subagents_request_is_answered_to_its_parent_and_the_grant_covers_later_subagents() {
    let scripts = Arc::new(Scripts::default());
    let World {
        harness,
        backend,
        master,
        mut socket,
        _device,
        ..
    } = opened(&scripts).await;

    // The child asks, reports the refusal and closes.
    scripts.child(vec![
        shell("c1", "demi skills add acme/tools"),
        say("the user must allow managing skills"),
    ]);
    scripts.root(
        FIRST,
        vec![
            shell(
                "t1",
                "demi agent spawn --description installer <<< 'Install acme/tools'",
            ),
            say("dispatched"),
            // The child's completion wakes the parent.
            say("waiting for the user"),
        ],
    );
    socket.chat("m1", "Delegate the install").await;
    socket.until_idle().await;
    let read = permissions(&backend, &master, FIRST).await;
    assert_eq!(read.requests.len(), 1, "{read:?}");
    assert_eq!(
        read.requests[0].agent,
        Some(RequestingAgent {
            number: 1,
            description: "installer".into(),
        })
    );

    // The child is closed: its parent receives the message, naming it.
    scripts.root(
        FIRST,
        vec![shell("t2", "demi skills add acme/tools"), say("added")],
    );
    let allowed = decide(
        &backend,
        &master,
        FIRST,
        read.requests[0].id.as_str(),
        "allow",
    )
    .await;
    assert_eq!(allowed.status, StatusCode::NO_CONTENT);
    socket.until_idle().await;
    let woken = scripts.asked(|session| session == FIRST);
    let told = &woken[woken.len() - 2];
    assert!(
        told.contains("The user allowed this conversation to manage skills")
            && told.contains("Agent 1 (installer) ran it and has closed since."),
        "{told}"
    );
    assert_eq!(sources(&harness), 1);

    // A subagent spawned later runs the category's commands without asking.
    scripts.child(vec![
        shell("c2", "demi skills disable acme/tools --skill lint"),
        say("lint is off"),
    ]);
    scripts.root(
        FIRST,
        vec![
            shell(
                "t3",
                "demi agent spawn --description switcher <<< 'Turn lint off'",
            ),
            say("dispatched"),
            say("noted"),
        ],
    );
    socket.chat("m2", "Turn lint off").await;
    socket.until_idle().await;
    let child = scripts.asked(|session| session != FIRST).pop().unwrap();
    assert!(child.contains("Turned off: lint"), "{child}");
    assert!(
        permissions(&backend, &master, FIRST)
            .await
            .requests
            .is_empty()
    );
    backend.close().await;
}

// Over two seconds: a shell job on a real device runs until the user stops
// its turn.
#[tokio::test]
async fn an_allow_wakes_an_agent_whose_turn_the_user_stopped() {
    let scripts = Arc::new(Scripts::default());
    // The harness holds the backend's data, which dropping it removes.
    let World {
        harness: _harness,
        backend,
        master,
        mut socket,
        _device,
        ..
    } = opened(&scripts).await;
    scripts.root(
        FIRST,
        vec![shell("t1", &format!("demi skills add acme/tools; {WAIT}"))],
    );
    socket
        .send(&crate::conversations::send("m1", "Install, then wait"))
        .await;
    eventually("the request is listed", || async {
        !permissions(&backend, &master, FIRST)
            .await
            .requests
            .is_empty()
    })
    .await;
    socket.stop().await;

    // Deciding is the user's own action: the stopped root takes the message
    // in a turn of its own.
    let request = the_request(&backend, &master, FIRST).await;
    scripts.root(FIRST, vec![say("resumed")]);
    let allowed = decide(&backend, &master, FIRST, &request, "allow").await;
    assert_eq!(allowed.status, StatusCode::NO_CONTENT);
    socket.until_idle().await;
    let told = last_asked(&scripts, FIRST);
    assert!(
        told.contains("The user allowed this conversation to manage skills"),
        "{told}"
    );
    backend.close().await;
}

// Several seconds: a real device runs the shell jobs of three turns.
#[tokio::test]
async fn an_archive_withdraws_the_requests_and_keeps_the_grants_and_a_fork_has_neither() {
    let scripts = Arc::new(Scripts::default());
    // The harness holds the backend's data, which dropping it removes.
    let World {
        harness: _harness,
        backend,
        master,
        mut socket,
        _device,
        ..
    } = opened(&scripts).await;
    let archive = |archived: bool| {
        let backend = &backend;
        let master = &master;
        async move {
            let patched = backend
                .patch(
                    &format!("/api/conversations/{FIRST}"),
                    master,
                    json!({ "archived": archived }),
                )
                .await;
            assert_eq!(
                patched.status,
                StatusCode::OK,
                "{}",
                String::from_utf8_lossy(&patched.body)
            );
        }
    };

    scripts.root(
        FIRST,
        vec![shell("t1", "demi skills add acme/tools"), say("asked")],
    );
    socket.chat("m1", "Install").await;
    let withdrawn = the_request(&backend, &master, FIRST).await;
    archive(true).await;
    assert!(
        permissions(&backend, &master, FIRST)
            .await
            .requests
            .is_empty()
    );
    let refused = decide(&backend, &master, FIRST, &withdrawn, "allow").await;
    assert_eq!(refused.status, StatusCode::CONFLICT);
    archive(false).await;
    assert!(
        permissions(&backend, &master, FIRST)
            .await
            .requests
            .is_empty()
    );

    // A grant outlives an archive.
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    scripts.root(
        FIRST,
        vec![shell("t2", "demi skills add acme/tools"), say("asked")],
    );
    socket.chat("m2", "Install again").await;
    let request = the_request(&backend, &master, FIRST).await;
    scripts.root(FIRST, vec![say("granted")]);
    let allowed = decide(&backend, &master, FIRST, &request, "allow").await;
    assert_eq!(allowed.status, StatusCode::NO_CONTENT);
    socket.until_idle().await;
    archive(true).await;
    archive(false).await;
    assert_eq!(
        granted(&permissions(&backend, &master, FIRST).await),
        ["skills.manage"]
    );

    // A Fork is a new conversation: no grant, no request.
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    let text = socket
        .live()
        .await
        .iter()
        .rev()
        .find(|block| matches!(block, Block::Text(_)))
        .map(|block| block.id().clone())
        .expect("the root answered");
    let forked = backend
        .post(
            &format!("/api/conversations/{FIRST}/fork"),
            Some(&master),
            json!({ "id": SECOND, "blockId": text }),
        )
        .await;
    assert_eq!(
        forked.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&forked.body)
    );
    let fork = permissions(&backend, &master, SECOND).await;
    assert!(fork.requests.is_empty() && fork.grants.is_empty(), "{fork:?}");
    backend.close().await;
}

// Several seconds: a real device runs a shell job, and the backend starts
// twice over the same data.
#[tokio::test]
async fn a_decision_whose_message_a_restart_cut_off_reaches_the_agent_once_at_the_next_start() {
    let scripts = Arc::new(Scripts::default());
    let World {
        harness,
        backend,
        master,
        mut socket,
        _device,
        ..
    } = opened(&scripts).await;
    scripts.root(
        FIRST,
        vec![shell("t1", "demi skills add acme/tools"), say("asked")],
    );
    socket.chat("m1", "Install").await;
    let request = the_request(&backend, &master, FIRST).await;
    drop(socket);
    backend.close().await;

    // The backend stopped after the decision was stored and before its
    // message was admitted: the allow's one transaction, as storage holds it.
    let control = harness.control_database();
    control
        .execute(
            "INSERT INTO permission_grants (conversation_id, category, granted_at) VALUES (?1, 'skills.manage', 1)",
            [FIRST],
        )
        .unwrap();
    control
        .execute(
            "UPDATE permission_requests SET decision = 'allowed', decided_at = 1 WHERE id = ?1",
            [&request],
        )
        .unwrap();

    scripts.root(FIRST, vec![say("resumed")]);
    let backend = harness.start().await;
    eventually("the message woke the root", || async {
        scripts
            .asked(|session| session == FIRST)
            .last()
            .is_some_and(|asked| asked.contains("The user allowed this conversation to manage skills"))
    })
    .await;
    eventually("the delivered request is forgotten", || async {
        let left: i64 = harness
            .control_database()
            .query_row("SELECT count(*) FROM permission_requests", [], |row| row.get(0))
            .unwrap();
        left == 0
    })
    .await;
    let master = backend
        .login(crate::support::MASTER_EMAIL, crate::support::MASTER_PASSWORD)
        .await;
    let history = crate::conversations::transcript(&backend, &master, FIRST).await;
    let messages = history
        .blocks
        .iter()
        .filter(|block| matches!(block, Block::AgentMessage(_)))
        .count();
    assert_eq!(messages, 1);
    backend.close().await;
}
