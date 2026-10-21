//! Conversation permissions.
//!
//! Scenarios:
//!
//! - A `demi skills` command whose category the conversation has not been
//!   granted raises a request the pages list and waits for the user's answer.
//!   The agent reads that it waits, in its result and its progress reports.
//! - The user's Allow grants the category for the whole tree and runs each
//!   waiting command once, whose end reports to the agent as any command's
//!   does. The decision's message tells the agent, waking it even after the
//!   user stopped its turn.
//! - A Deny ends the command unrun and remembers nothing.
//! - A newer run replaces an undecided request and ends the older command.
//! - A stopped command leaves its request to the user.
//! - An archive waits for the answer and keeps the grants.
//! - A Fork copies none.
//! - A decision a restart cut off reaches the agent at the next start.
//!
//! The model is the scripted family of the subagent scenarios; the device is a
//! real runner. No test calls a real model.

use std::sync::Arc;

use demi_plugin_skills::testing::{Repos, skill_md};
use demi_shared_types::Block;
use demi_web_api_protocol::error::{ErrorBody, ErrorCode};
use demi_web_api_protocol::permissions::{
    ConversationPermissions, PermissionCategory, PermissionHost, PermissionRequest, RequestingAgent,
};
use reqwest::StatusCode;
use serde_json::json;

use crate::conversations::{FIRST, SECOND, Socket, choose, create, summary};
use crate::subagents::{Scripts, background, hang, say, shell, tree_on, watched};
use crate::support::{Harness, Session, TestBackend, eventually};

/// What the agent reads of a command that waits for the user's answer.
pub(crate) const WAITING: &str =
    "waiting for the user's permission to manage skills; its end will report to you, so do not run it again";

/// The decision's message an allow sends the agent that asked.
const ALLOWED: &str = "The user allowed this conversation to manage skills.";

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
    opened_on(scripts, Harness::new()).await
}

/// [`opened`] on `harness`, such as one whose shards run on the test's own
/// runtime.
async fn opened_on(scripts: &Arc<Scripts>, harness: Harness) -> World {
    let harness = harness.with_skill_repos(&repos());
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

pub(crate) async fn permissions(backend: &TestBackend, session: &Session, id: &str) -> ConversationPermissions {
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

/// The categories of a permission request.
pub(crate) fn categories(request: &PermissionRequest) -> &[PermissionCategory] {
    match request {
        PermissionRequest::Permission { categories, .. } => categories,
        other => panic!("not a permission request: {other:?}"),
    }
}

/// The host a permission request asks to reach.
pub(crate) fn host_of(request: &PermissionRequest) -> Option<&PermissionHost> {
    match request {
        PermissionRequest::Permission { host, .. } => host.as_ref(),
        other => panic!("not a permission request: {other:?}"),
    }
}

/// The only undecided request of the conversation: its id.
pub(crate) async fn the_request(backend: &TestBackend, session: &Session, id: &str) -> String {
    let read = permissions(backend, session, id).await;
    assert_eq!(read.requests.len(), 1, "{read:?}");
    read.requests[0].id().as_str().to_owned()
}

pub(crate) async fn decide(
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

/// The categories the conversation `id` was granted, as storage holds them:
/// no page lists them.
fn granted(harness: &Harness, id: &str) -> Vec<String> {
    harness
        .control_database()
        .prepare("SELECT category FROM permission_grants WHERE conversation_id = ?1 ORDER BY category")
        .unwrap()
        .query_map([id], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// Waits until the conversation `id` lists `count` requests, and answers
/// them.
pub(crate) async fn listed(backend: &TestBackend, session: &Session, id: &str, count: usize) -> ConversationPermissions {
    eventually("the requests are listed", || async {
        permissions(backend, session, id).await.requests.len() == count
    })
    .await;
    permissions(backend, session, id).await
}

/// Waits until a request of the sessions `which` picks carries `text`, and
/// answers every request of theirs.
async fn until_asked(scripts: &Scripts, which: impl Fn(&str) -> bool + Copy, text: &str) -> Vec<String> {
    eventually(&format!("an agent reads {text:?}"), || async {
        scripts.asked(which).iter().any(|asked| asked.contains(text))
    })
    .await;
    scripts.asked(which)
}

/// Waits until the root of `id` was last asked with every one of `texts` in
/// its history and its turn ended, and answers what it was asked last.
pub(crate) async fn settled(scripts: &Scripts, backend: &TestBackend, session: &Session, id: &str, texts: &[&str]) -> String {
    eventually(&format!("the root read {texts:?} and settled"), || async {
        let last = scripts.asked(|asked| asked == id).pop().unwrap_or_default();
        texts.iter().all(|text| last.contains(text))
            && summary(backend, session, id).await.status != demi_web_api_protocol::conversations::ConversationStatus::Running
    })
    .await;
    last_asked(scripts, id)
}

/// The command a result names, by the number after `commandId: `.
fn command_id(text: &str) -> String {
    let at = text.find("commandId: ").expect("a command's handle") + "commandId: ".len();
    text[at..].chars().take_while(char::is_ascii_digit).collect()
}

/// Waits until each of the subagent sessions `children` read the end of its
/// command and the allow's message, and answers the history each was last asked
/// with.
///
/// Each child's command is `demi skills add acme/tools`, which prints what the
/// plugin did, and the history of one session is its own.
async fn ran_after_allow(scripts: &Scripts, children: &[String]) -> Vec<String> {
    let last = |child: &String| scripts.asked(|session| session == child).pop().unwrap_or_default();
    eventually("each subagent's command ended and each was told", || async {
        children.iter().all(|child| {
            let history = last(child);
            history.contains("ended with exit code") && history.contains(ALLOWED)
        })
    })
    .await;
    let histories: Vec<String> = children.iter().map(last).collect();
    for history in &histories {
        // What the plugin printed, not the command line, which the
        // history carries too.
        assert!(
            history.contains("Added acme/tools at") || history.contains("acme/tools is added already"),
            "{history}"
        );
    }
    histories
}

/// `count` closing words, for the turns that reports and messages wake.
fn notes(count: usize) -> Vec<crate::subagents::Answer> {
    (0..count).map(|_| say("noted")).collect()
}

// Several seconds: a real device runs the shell jobs of two conversations;
// the first waits for the user through ten of its half-second intervals,
// which a jump of the shard's clock passes at once.
#[tokio::test(flavor = "local")]
async fn a_command_without_the_grant_waits_and_runs_once_the_user_allows_it() {
    let scripts = Arc::new(Scripts::default());
    let World {
        harness,
        backend,
        master,
        mut socket,
        _device,
        provider,
    } = opened_on(&scripts, {
        // Its shard's timers are the test's, whose clock the wait jumps.
        let mut harness = Harness::new();
        harness.shards = Some(demi_backend_user_shard::shard::ShardPlacement::Inline);
        harness
    })
    .await;

    let mut answers = vec![
        watched("t1", "demi skills add acme/tools; echo exit=$?", 500),
        say("I asked the user"),
    ];
    answers.extend(notes(40));
    scripts.root(FIRST, answers);
    socket.chat("m1", "Install acme/tools").await;
    // The call's window passed: the command still runs, and says why.
    let waiting = last_asked(&scripts, FIRST);
    assert!(waiting.contains("status: running") && waiting.contains(WAITING), "{waiting}");
    let asked_before = scripts.asked(|session| session == FIRST).len();
    // Ten of its intervals pass while it waits.
    crate::support::elapse(std::time::Duration::from_secs(5)).await;
    // The plugin received nothing: nothing was fetched or added.
    assert_eq!(sources(&harness), 0);
    let asked = permissions(&backend, &master, FIRST).await;
    assert_eq!(asked.requests.len(), 1, "{asked:?}");
    let request = &asked.requests[0];
    assert_eq!(request.command(), "demi skills add acme/tools");
    assert_eq!(request.agent(), None);
    assert_eq!(categories(request).len(), 1, "{request:?}");
    assert_eq!(categories(request)[0].id, "skills.manage");
    assert_eq!(categories(request)[0].action.as_deref(), Some("manage skills"));
    assert!(granted(&harness, FIRST).is_empty());
    assert_eq!(summary(&backend, &master, FIRST).await.permission_requests, 1);
    // A waiting command reports no progress: nothing woke the agent.
    assert_eq!(scripts.asked(|session| session == FIRST).len(), asked_before);

    // Allow: the waiting command runs once, its end reports to the agent,
    // and the decision's message says only what the user decided.
    let allowed = decide(&backend, &master, FIRST, request.id().as_str(), "allow").await;
    assert_eq!(allowed.status, StatusCode::NO_CONTENT);
    let last = settled(&scripts, &backend, &master, FIRST, &["exit=0", ALLOWED, "ended with exit code 0"]).await;
    assert!(!last.contains("is still running after"), "{last}");
    assert_eq!(last.matches("Added acme/tools at").count(), 1, "{last}");
    assert!(last.contains("Turned on: lint, review"), "{last}");
    assert!(!last.contains("can now run"), "{last}");
    assert_eq!(sources(&harness), 1);
    let read = permissions(&backend, &master, FIRST).await;
    assert!(read.requests.is_empty(), "{read:?}");
    assert_eq!(granted(&harness, FIRST), ["skills.manage"]);
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
            background("s1", "demi skills disable acme/tools --skill lint"),
            say("asked"),
        ],
    );
    second.chat("n1", "Turn lint off").await;
    assert!(last_asked(&scripts, SECOND).contains(WAITING));
    assert_eq!(permissions(&backend, &master, SECOND).await.requests.len(), 1);
    assert!(granted(&harness, SECOND).is_empty());
    backend.close().await;
}

// Several seconds: a real device runs the shell jobs of three turns.
#[tokio::test]
async fn a_newer_run_replaces_the_request_and_a_deny_ends_the_command_unrun() {
    let scripts = Arc::new(Scripts::default());
    let World {
        harness,
        backend,
        master,
        mut socket,
        _device,
        ..
    } = opened(&scripts).await;

    // Twice before a decision: one request, with the newer command line,
    // and the older command ends unrun.
    let mut answers = vec![
        background("t1", "demi skills add acme/tools; echo first=$?"),
        background("t2", "demi skills add acme/tools --skill review; echo second=$?"),
        say("asked"),
    ];
    answers.extend(notes(6));
    scripts.root(FIRST, answers);
    socket.send(&crate::conversations::send("m1", "Install review")).await;
    let replaced = until_asked(
        &scripts,
        |session| session == FIRST,
        "demi skills add: a newer run of this command asks the user instead; this one was not run",
    )
    .await;
    assert!(replaced.iter().any(|asked| asked.contains("first=1")), "{replaced:?}");
    let read = listed(&backend, &master, FIRST, 1).await;
    assert_eq!(
        read.requests[0].command(),
        "demi skills add acme/tools --skill review"
    );
    let newer = read.requests[0].id().as_str().to_owned();

    // Deny: the command ends unrun, the agent is told, and nothing is
    // remembered.
    let denied = decide(&backend, &master, FIRST, &newer, "deny").await;
    assert_eq!(denied.status, StatusCode::NO_CONTENT);
    let told = settled(
        &scripts,
        &backend,
        &master,
        FIRST,
        &["second=1", "The user denied this conversation permission to manage skills."],
    )
    .await;
    assert!(
        told.contains(
            "demi skills add: the user denied this conversation permission to manage skills; the command was not run"
        ),
        "{told}"
    );
    assert_eq!(sources(&harness), 0);
    let read = permissions(&backend, &master, FIRST).await;
    assert!(read.requests.is_empty(), "{read:?}");
    assert!(granted(&harness, FIRST).is_empty());
    // A request decided once is decided: a second page's answer is 404.
    let again = decide(&backend, &master, FIRST, &newer, "allow").await;
    assert_eq!(again.status, StatusCode::NOT_FOUND);
    assert_eq!(
        again.json::<ErrorBody>().code,
        ErrorCode::PermissionRequestNotFound
    );

    // The next attempt asks again.
    scripts.root_anew(
        FIRST,
        vec![background("t3", "demi skills add acme/tools"), say("asked")],
    );
    socket.send(&crate::conversations::send("m2", "Try again")).await;
    let read = listed(&backend, &master, FIRST, 1).await;
    assert_ne!(read.requests[0].id().as_str(), newer);
    backend.close().await;
}

// Several seconds: a real device runs the shell jobs of two turns.
#[tokio::test]
async fn a_request_is_unread_until_the_user_reads_the_conversation_and_a_later_request_is_unread_again() {
    let scripts = Arc::new(Scripts::default());
    let World {
        backend,
        master,
        mut socket,
        _device,
        ..
    } = opened(&scripts).await;

    scripts.root(FIRST, vec![background("t1", "demi skills add acme/tools"), say("asked")]);
    socket.chat("m1", "Install acme/tools").await;
    let asked = summary(&backend, &master, FIRST).await;
    assert_eq!((asked.permission_requests, asked.unread_permission_requests), (1, 1));

    // Reading the conversation settles the mark; the request still waits.
    let read = backend
        .post(
            &format!("/api/conversations/{FIRST}/read"),
            Some(&master),
            json!({ "revision": asked.revision }),
        )
        .await;
    assert_eq!(read.status, StatusCode::NO_CONTENT);
    let read = summary(&backend, &master, FIRST).await;
    assert_eq!((read.permission_requests, read.unread_permission_requests), (1, 0));
    assert_eq!(permissions(&backend, &master, FIRST).await.requests.len(), 1);

    // A request raised after the reading is new: the same command again
    // replaces the request, and the replacement is unread.
    let mut answers = vec![background("t2", "demi skills add acme/tools"), say("asked again")];
    answers.extend(notes(2));
    scripts.root(FIRST, answers);
    socket.send(&crate::conversations::send("m2", "Try again")).await;
    settled(&scripts, &backend, &master, FIRST, &["a newer run of this command asks the user instead"]).await;
    let again = summary(&backend, &master, FIRST).await;
    assert_eq!((again.permission_requests, again.unread_permission_requests), (1, 1));
    backend.close().await;
}

// Several seconds: two subagents and their parent run shell jobs on a real
// device.
#[tokio::test]
async fn an_allow_runs_every_waiting_command_of_the_category_and_leaves_the_other_request() {
    let scripts = Arc::new(Scripts::default());
    let World {
        harness,
        backend,
        master,
        mut socket,
        _device,
        ..
    } = opened(&scripts).await;

    // Two subagents ask for Manage Skills, each waiting with its command in
    // the background, and the root for another category.
    for _ in 0..2 {
        let mut answers = vec![
            background("c1", "demi skills add acme/tools; echo added=$?"),
            say("waiting for the user"),
        ];
        answers.extend(notes(4));
        scripts.child(answers);
    }
    let mut answers = vec![
        shell(
            "t1",
            "demi agent spawn --description installer <<< 'Install' && \
             demi agent spawn --description linter <<< 'Install too'",
        ),
        background("t2", "demi agent profiles delete nobody"),
        say("dispatched"),
    ];
    answers.extend(notes(8));
    scripts.root(FIRST, answers);
    socket.send(&crate::conversations::send("m1", "Delegate the installs")).await;
    let read = listed(&backend, &master, FIRST, 3).await;
    let installer = read
        .requests
        .iter()
        .find(|request| request.agent().is_some_and(|agent| agent.description == "installer"))
        .expect("the installer asked");
    assert_eq!(
        installer.agent(),
        Some(&RequestingAgent {
            number: 1,
            description: "installer".into(),
        })
    );

    // Allowing one request of the category runs both subagents' commands,
    // which waited, and tells both; the request of the other category stays.
    let allowed = decide(&backend, &master, FIRST, installer.id().as_str(), "allow").await;
    assert_eq!(allowed.status, StatusCode::NO_CONTENT);
    let children = scripts.sessions(|session| session != FIRST);
    assert_eq!(children.len(), 2, "{children:?}");
    let ended = ran_after_allow(&scripts, &children).await;
    // The plugin ran both: one added the source, and the other found it
    // added already.
    let mut outcomes: Vec<bool> = ended.iter().map(|history| history.contains("Added acme/tools at")).collect();
    outcomes.sort_unstable();
    assert_eq!(outcomes, [false, true], "{ended:?}");
    assert!(ended.iter().any(|history| history.contains("acme/tools is added already")), "{ended:?}");
    assert_eq!(sources(&harness), 1);
    let left = permissions(&backend, &master, FIRST).await;
    assert_eq!(left.requests.len(), 1, "{left:?}");
    assert_eq!(left.requests[0].command(), "demi agent profiles delete nobody");
    backend.close().await;
}

// Several seconds: a subagent and its parent run shell jobs on a real
// device.
#[tokio::test]
async fn an_allow_decided_as_another_call_of_its_category_asks_runs_that_call_too() {
    use demi_backend_user_shard::holds::PermissionStep;
    let scripts = Arc::new(Scripts::default());
    let World {
        harness,
        backend,
        master,
        mut socket,
        _device,
        ..
    } = opened(&scripts).await;

    // The root waits for Manage Skills.
    let mut answers = vec![
        background("t1", "demi skills add acme/tools; echo root=$?"),
        say("asked"),
    ];
    answers.extend(notes(2));
    scripts.root(FIRST, answers);
    socket.chat("m1", "Install acme/tools").await;
    let first = the_request(&backend, &master, FIRST).await;

    // A subagent asks for it too, and its call is held once its request is
    // stored, before it waits for the answer: the user allows the root's
    // request then.
    let held = backend.hold_permissions(PermissionStep::Raised);
    let mut answers = vec![
        background("c1", "demi skills add acme/tools; echo child=$?"),
        say("waiting for the user"),
    ];
    answers.extend(notes(3));
    scripts.child(answers);
    let mut answers = vec![
        shell("t2", "demi agent spawn --description installer <<< 'Install'"),
        say("dispatched"),
    ];
    answers.extend(notes(8));
    scripts.root_anew(FIRST, answers);
    socket.send(&crate::conversations::send("m2", "Delegate it")).await;
    held.until_arrived(1).await;
    listed(&backend, &master, FIRST, 2).await;
    let allowed = decide(&backend, &master, FIRST, &first, "allow").await;
    assert_eq!(allowed.status, StatusCode::NO_CONTENT);
    held.release();

    // The subagent's call goes on to its run, as one that waited would.
    let children = scripts.sessions(|session| session != FIRST);
    assert_eq!(children.len(), 1, "{children:?}");
    ran_after_allow(&scripts, &children).await;
    settled(&scripts, &backend, &master, FIRST, &["root=0", ALLOWED]).await;
    assert_eq!(sources(&harness), 1);
    let read = permissions(&backend, &master, FIRST).await;
    assert!(read.requests.is_empty(), "{read:?}");
    backend.close().await;
}

// Several seconds: a subagent's shell job on a real device waits until its
// parent stops it.
#[tokio::test]
async fn a_stopped_command_runs_nothing_and_its_request_is_answered_to_the_parent_of_its_closed_subagent() {
    let scripts = Arc::new(Scripts::default());
    let World {
        harness,
        backend,
        master,
        mut socket,
        _device,
        ..
    } = opened(&scripts).await;

    // The child asks and waits; its parent stops the command, and the
    // child, told of the stop, closes.
    scripts.child(vec![
        background("c1", "demi skills add acme/tools"),
        say("waiting for the user"),
        say("the install was stopped"),
    ]);
    let mut answers = vec![
        shell("t1", "demi agent spawn --description installer <<< 'Install acme/tools'"),
        say("dispatched"),
    ];
    answers.extend(notes(2));
    scripts.root(FIRST, answers);
    socket.send(&crate::conversations::send("m1", "Delegate the install")).await;
    socket.until_idle().await;
    let waiting = until_asked(&scripts, |session| session != FIRST, WAITING).await;
    let command = command_id(waiting.iter().find(|asked| asked.contains(WAITING)).unwrap());
    let read = listed(&backend, &master, FIRST, 1).await;

    let mut answers = vec![shell("t2", &format!("demi shell stop {command}")), say("stopped")];
    answers.extend(notes(4));
    scripts.root_anew(FIRST, answers);
    socket.send(&crate::conversations::send("m2", "Stop the install")).await;
    until_asked(&scripts, |session| session != FIRST, "was stopped by agent 0").await;
    // The parent is told only that the child finished.
    settled(&scripts, &backend, &master, FIRST, &["outcome", "completed"]).await;
    // The stop dispatched nothing and left the request to the user.
    assert_eq!(sources(&harness), 0);
    assert_eq!(permissions(&backend, &master, FIRST).await.requests, read.requests);

    // The child is closed: its parent receives the message, naming it.
    let allowed = decide(&backend, &master, FIRST, read.requests[0].id().as_str(), "allow").await;
    assert_eq!(allowed.status, StatusCode::NO_CONTENT);
    let told = settled(&scripts, &backend, &master, FIRST, &[ALLOWED]).await;
    assert!(told.contains("Agent 1 (installer) asked and has closed since."), "{told}");
    assert_eq!(sources(&harness), 0);
    assert_eq!(granted(&harness, FIRST), ["skills.manage"]);
    assert!(permissions(&backend, &master, FIRST).await.requests.is_empty());
    backend.close().await;
}

// Over two seconds: a shell job on a real device waits through the user's
// Stop of the turn.
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
        vec![background("t1", "demi skills add acme/tools; echo exit=$?"), hang()],
    );
    socket
        .send(&crate::conversations::send("m1", "Install, then think"))
        .await;
    until_asked(&scripts, |session| session == FIRST, WAITING).await;
    socket.stop().await;

    // Deciding is the user's own action: the stopped root takes the
    // message in a turn of its own, and the command's end with it.
    let request = the_request(&backend, &master, FIRST).await;
    let mut answers = vec![say("resumed")];
    answers.extend(notes(2));
    scripts.root(FIRST, answers);
    let allowed = decide(&backend, &master, FIRST, &request, "allow").await;
    assert_eq!(allowed.status, StatusCode::NO_CONTENT);
    settled(&scripts, &backend, &master, FIRST, &[ALLOWED, "exit=0"]).await;
    backend.close().await;
}

// Several seconds: a real device runs the shell jobs of two turns.
#[tokio::test]
async fn an_archive_waits_for_the_answer_and_keeps_the_grants_and_a_fork_has_neither() {
    let scripts = Arc::new(Scripts::default());
    let World {
        harness,
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
            backend
                .patch(
                    &format!("/api/conversations/{FIRST}"),
                    master,
                    json!({ "archived": archived }),
                )
                .await
        }
    };

    // A waiting command runs, so the conversation is not archived while
    // its request waits.
    let mut answers = vec![background("t1", "demi skills add acme/tools; echo exit=$?"), say("asked")];
    answers.extend(notes(4));
    scripts.root(FIRST, answers);
    socket.chat("m1", "Install").await;
    let request = the_request(&backend, &master, FIRST).await;
    let refused = archive(true).await;
    assert_eq!(refused.status, StatusCode::CONFLICT);
    assert_eq!(refused.json::<ErrorBody>().code, ErrorCode::TurnInFlight);
    assert_eq!(permissions(&backend, &master, FIRST).await.requests.len(), 1);

    // Once the user answered and the command ended, it archives, and a
    // grant outlives the archive.
    let allowed = decide(&backend, &master, FIRST, &request, "allow").await;
    assert_eq!(allowed.status, StatusCode::NO_CONTENT);
    settled(&scripts, &backend, &master, FIRST, &["exit=0", ALLOWED]).await;
    for archived in [true, false] {
        let patched = archive(archived).await;
        assert_eq!(patched.status, StatusCode::OK, "{}", String::from_utf8_lossy(&patched.body));
    }
    assert_eq!(granted(&harness, FIRST), ["skills.manage"]);

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
    assert!(fork.requests.is_empty(), "{fork:?}");
    assert!(granted(&harness, SECOND).is_empty());
    backend.close().await;
}

// Several seconds: a real device runs a shell job, and the backend starts
// twice over the same data.
#[tokio::test]
async fn a_request_outlives_a_restart_and_a_decision_whose_message_it_cut_off_reaches_the_agent_once() {
    let scripts = Arc::new(Scripts::default());
    let World {
        harness,
        backend,
        master,
        mut socket,
        _device,
        ..
    } = opened(&scripts).await;
    let mut answers = vec![background("t1", "demi skills add acme/tools"), say("asked")];
    answers.extend(notes(6));
    scripts.root(FIRST, answers);
    socket.chat("m1", "Install").await;
    let request = the_request(&backend, &master, FIRST).await;
    drop(socket);
    // The backend comes back at its address, where the device's runner
    // reconnects: the woken root's work waits for it after a restart.
    let address = backend.address();
    backend.close().await;

    // The request outlived the restart the call did not.
    let backend = harness.start_at(address).await;
    let master = backend
        .login(crate::support::MASTER_EMAIL, crate::support::MASTER_PASSWORD)
        .await;
    assert_eq!(the_request(&backend, &master, FIRST).await, request);
    let address = backend.address();
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

    let backend = harness.start_at(address).await;
    until_asked(&scripts, |session| session == FIRST, ALLOWED).await;
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
    // The lost call never ran its handler.
    assert_eq!(sources(&harness), 0);
    backend.close().await;
}
