//! Credentials.
//!
//! Scenarios:
//!
//! - An agent asks the user for a credential and waits. The user's answer
//!   saves it, allows it in the conversation and returns the command.
//! - The agent pipes a field's value into a file on the Host without the value
//!   reaching the transcript, the backend's log, a page's synchronization
//!   channel or what `list` and `show` print.
//! - A read of a credential the conversation may not use asks for an allowance
//!   and fails until the user allows it.
//! - `remove` needs Manage Credentials.
//! - Settings lists a visible field's value, never a secret one's, and a
//!   name once.
//! - A credential kept for a while is removed at its time and, after a backend
//!   that was down then, at start.
//!
//! The model is the scripted family of the subagent scenarios; the device is a
//! real runner. No test calls a real model.

use crate::agent_profiles::assert_shows;
use std::sync::Arc;
use std::time::Duration;

use demi_backend_user_shard::shard::ShardPlacement;
use demi_web_api_protocol::credentials::{CredentialAnswer, CredentialList, Keep};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::permissions::PermissionRequest;
use demi_web_api_protocol::state::SyncEvent;
use reqwest::StatusCode;
use serde_json::json;

use crate::conversations::{FIRST, Socket, send, summary, transcript};
use crate::permissions::{categories, permissions};
use crate::subagents::{Scripts, background, say, shell, tree_on};
use crate::support::{Harness, Session, TestBackend, elapse, eventually};

/// The secret the user types, which only the file on the Host may hold.
const SECRET: &str = "sk-live-7f3a9c";

/// What the root of `id` was asked last.
fn last_asked(scripts: &Scripts, id: &str) -> String {
    scripts
        .asked(|session| session == id)
        .pop()
        .expect("the root was asked")
}

async fn credentials(backend: &TestBackend, session: &Session) -> CredentialList {
    let read = backend.get("/api/credentials", Some(session)).await;
    assert_eq!(read.status, StatusCode::OK, "{}", String::from_utf8_lossy(&read.body));
    read.json()
}

/// Saves a credential through Settings' route and answers its id.
async fn saved(backend: &TestBackend, session: &Session, body: serde_json::Value) -> String {
    let created = backend.post("/api/credentials", Some(session), body).await;
    assert_eq!(
        created.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&created.body)
    );
    created.json::<CredentialAnswer>().credential.id.as_str().to_owned()
}

async fn decide(
    backend: &TestBackend,
    session: &Session,
    request: &str,
    body: serde_json::Value,
) -> crate::support::Answer {
    backend
        .post(
            &format!("/api/conversations/{FIRST}/permissions/requests/{request}"),
            Some(session),
            body,
        )
        .await
}

// Several seconds: a real device runs the shell jobs of two turns, one of
// them waiting for the user's answer.
#[tokio::test]
async fn an_agent_asks_for_a_credential_and_pipes_the_answer_into_a_file_and_the_value_shows_nowhere_else()
{
    let logs = tempfile::tempdir().unwrap();
    let log_path = logs.path().join("backend.log");
    let file = Arc::new(std::fs::File::create(&log_path).unwrap());
    // The backend's log, its debug lines included, as `main` writes it;
    // here it serves this test's thread and the shard threads its backend
    // starts.
    let _log = demi_log_capture::capture(
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(file)
            .finish(),
    );
    let scripts = Arc::new(Scripts::default());
    let (_harness, backend, master, _device, root, _provider) = tree_on(&scripts, Harness::new()).await;
    let mut page = backend.sync(&master).await;
    page.snapshot().await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    scripts.root(
        FIRST,
        vec![
            shell(
                "t1",
                "demi credentials request 'Deploy key' --field key:secret:'API key' --field user:visible:User --reason 'To deploy the site' --keep 1d && demi credentials read 'Deploy key' key > key.txt; echo exit=$?",
            ),
            say("saved"),
        ],
    );
    socket.send(&send("m1", "Deploy the site")).await;

    // The request waits on the conversation's one list, with its kind, and
    // the summary counts it as it counts a permission request.
    eventually("the credential request is listed", || async {
        !permissions(&backend, &master, FIRST).await.requests.is_empty()
    })
    .await;
    let request = permissions(&backend, &master, FIRST).await.requests.remove(0);
    let PermissionRequest::Credential {
        id,
        name,
        reason,
        fields,
        keep,
        renewal,
        command,
        agent,
        ..
    } = &request
    else {
        panic!("not a credential request: {request:?}");
    };
    assert_eq!(name, "Deploy key");
    assert_eq!(reason, "To deploy the site");
    assert_eq!(*keep, Keep::Day);
    assert!(!renewal, "no credential of the name is saved yet");
    assert_eq!(*agent, None);
    assert!(command.starts_with("demi credentials request 'Deploy key'"), "{command}");
    let listed: Vec<(&str, &str, &str)> = fields
        .iter()
        .map(|field| (field.name.as_str(), field.kind.as_str(), field.label.as_str()))
        .collect();
    assert_eq!(listed, [("key", "secret", "API key"), ("user", "visible", "User")]);
    assert_eq!(summary(&backend, &master, FIRST).await.permission_requests, 1);

    // An answer that does not fit is refused and changes nothing.
    let id = id.as_str();
    let bare = decide(&backend, &master, id, json!({ "decision": "allow" })).await;
    assert_eq!(bare.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidAnswer));
    let short = decide(
        &backend,
        &master,
        id,
        json!({ "decision": "allow", "credential": { "values": { "key": SECRET } } }),
    )
    .await;
    assert_eq!(short.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidAnswer));
    // The card shows the refusal as it is: a sentence naming the field by
    // its label.
    assert_eq!(short.error().message, "Enter a value for \u{201c}User\u{201d}.");

    // The user's answer, its values alone, saves it for the day the agent
    // chose, and the command returns and reads it.
    let answered = decide(
        &backend,
        &master,
        id,
        json!({
            "decision": "allow",
            "credential": { "values": { "key": SECRET, "user": "alex" } }
        }),
    )
    .await;
    assert_eq!(answered.status, StatusCode::NO_CONTENT, "{}", String::from_utf8_lossy(&answered.body));
    socket.until_idle().await;
    let returned = last_asked(&scripts, FIRST);
    // What was saved, as labelled lines: never a value.
    assert_shows(&returned, "Name: Deploy key\nKeep until: ");
    assert_shows(
        &returned,
        "Allowed: yes\nFields:\n  NAME  KIND     VALUE  LABEL\n  key   secret   -      API key\n  user  visible  -      User\n",
    );
    assert!(returned.contains("exit=0"), "{returned}");
    assert_eq!(std::fs::read_to_string(format!("{root}/key.txt")).unwrap(), SECRET);
    assert!(permissions(&backend, &master, FIRST).await.requests.is_empty());
    assert_eq!(summary(&backend, &master, FIRST).await.permission_requests, 0);
    let stored = credentials(&backend, &master).await.credentials;
    assert_eq!(stored.len(), 1, "{stored:?}");
    assert_eq!(stored[0].note, "To deploy the site");
    let kept = stored[0]
        .keep_until
        .expect("kept for a day")
        .to_jiff()
        .duration_since(stored[0].created_at.to_jiff());
    assert_eq!(kept, jiff::SignedDuration::from_hours(24));
    assert!(stored[0].last_used_at.is_some());

    // `list` and `show` print the visible value and never the secret one.
    scripts.root(
        FIRST,
        vec![
            shell(
                "t2",
                "demi credentials list; demi credentials show 'Deploy key'; demi credentials list --json; demi credentials show 'Deploy key' --json",
            ),
            say("listed"),
        ],
    );
    socket.chat("m2", "What credentials are there?").await;
    let shown = last_asked(&scripts, FIRST);
    // A row per credential; one credential as labelled lines, its fields
    // a table, the visible value shown and the secret one never.
    assert_shows(&shown, "NAME        FIELDS    KEEP UNTIL            ALLOWED\nDeploy key  key,user  ");
    assert_shows(
        &shown,
        "Allowed: yes\nFields:\n  NAME  KIND     VALUE  LABEL\n  key   secret   -      API key\n  user  visible  alex   User\nNote:\n  To deploy the site\n",
    );
    assert!(shown.contains(r#"\"value\":\"alex\""#), "{shown}");
    assert!(!shown.contains(SECRET), "{shown}");

    // Nothing else holds the value: the transcript, a page's channel and
    // the log name the credential and its field only.
    let history = transcript(&backend, &master, FIRST).await;
    let blocks = serde_json::to_string(&history.blocks).unwrap();
    assert!(blocks.contains("Deploy key"), "{blocks}");
    assert!(!blocks.contains(SECRET), "{blocks}");
    let frames = page
        .until(|event| matches!(event, SyncEvent::Credentials { credentials } if credentials.iter().any(|credential| credential.last_used_at.is_some())))
        .await;
    let frames = serde_json::to_string(&frames).unwrap();
    assert!(frames.contains("Deploy key"), "{frames}");
    assert!(frames.contains(r#""value":"alex""#), "the visible value reaches the page: {frames}");
    assert!(!frames.contains(SECRET), "{frames}");

    // A credential of the waiting request's name, whatever its case, saved
    // in Settings meanwhile: the conversation's requests change, and the
    // card now asks to renew it.
    scripts.root(
        FIRST,
        vec![
            shell(
                "t3",
                "demi credentials request 'Staging key' --field key:secret:'API key'; echo exit=$?",
            ),
            say("declined"),
        ],
    );
    socket.send(&send("m3", "Deploy to staging")).await;
    eventually("the request is listed", || async {
        !permissions(&backend, &master, FIRST).await.requests.is_empty()
    })
    .await;
    let request = permissions(&backend, &master, FIRST).await.requests.remove(0);
    let PermissionRequest::Credential { renewal, .. } = &request else {
        panic!("not a credential request: {request:?}");
    };
    assert!(!renewal, "{request:?}");
    let before = crate::conversations::metadata(&backend, &master, FIRST).await.permissions.revision;
    saved(
        &backend,
        &master,
        json!({
            "name": "staging KEY", "keep": "1h",
            "fields": [{ "name": "key", "label": "API key", "kind": "secret", "value": "stg_123" }]
        }),
    )
    .await;
    assert!(crate::conversations::metadata(&backend, &master, FIRST).await.permissions.revision > before);
    let request = permissions(&backend, &master, FIRST).await.requests.remove(0);
    let PermissionRequest::Credential { id, renewal, .. } = &request else {
        panic!("not a credential request: {request:?}");
    };
    assert!(*renewal, "{request:?}");
    let declined = decide(&backend, &master, id.as_str(), json!({ "decision": "deny" })).await;
    assert_eq!(declined.status, StatusCode::NO_CONTENT, "{}", String::from_utf8_lossy(&declined.body));
    socket.until_idle().await;
    backend.close().await;
    let log = std::fs::read_to_string(&log_path).unwrap();
    assert!(!log.is_empty(), "the log captured the backend");
    assert!(!log.contains(SECRET));
}

// Several seconds: a real device runs the shell jobs of five turns, two of
// them woken by the user's decisions.
#[tokio::test]
async fn a_read_without_the_allowance_asks_the_user_and_a_removal_needs_the_users_permission() {
    let scripts = Arc::new(Scripts::default());
    let (harness, backend, master, _device, _root, _provider) = tree_on(&scripts, Harness::new()).await;
    let token = saved(
        &backend,
        &master,
        json!({
            "name": "GitHub token", "keep": "always",
            "fields": [{ "name": "token", "label": "Token", "kind": "secret", "value": "ghp_123456" }]
        }),
    )
    .await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    // Saved in Settings, not from this conversation: the read asks first.
    scripts.root(
        FIRST,
        vec![
            shell("t1", "demi credentials read 'github token' token; echo exit=$?"),
            say("I asked the user"),
        ],
    );
    socket.chat("m1", "Push the branch").await;
    let refused = last_asked(&scripts, FIRST);
    assert!(
        refused.contains("demi credentials read: this conversation needs the user's permission to use the credential \u{201c}GitHub token\u{201d}; the request was sent to the user, and you will be told when the user decides"),
        "{refused}"
    );
    assert!(refused.contains("exit=1"), "{refused}");
    assert!(!refused.contains("ghp_123456"), "{refused}");
    let asked = permissions(&backend, &master, FIRST).await;
    assert_eq!(asked.requests.len(), 1, "{asked:?}");
    let PermissionRequest::Allowance { id, name, removed, .. } = &asked.requests[0] else {
        panic!("not an allowance request: {asked:?}");
    };
    assert_eq!((name.as_str(), *removed), ("GitHub token", false));

    // Allowed: the root is told, and its read now passes the value on.
    scripts.root(
        FIRST,
        vec![
            shell("t2", "demi credentials read 'github token' token | wc -c"),
            say("read"),
        ],
    );
    let allowed = decide(&backend, &master, id.as_str(), json!({ "decision": "allow" })).await;
    assert_eq!(allowed.status, StatusCode::NO_CONTENT, "{}", String::from_utf8_lossy(&allowed.body));
    socket.until_idle().await;
    let woken = scripts.asked(|session| session == FIRST);
    assert!(
        woken[woken.len() - 2].contains(
            "The user allowed this conversation to use the credential \u{201c}GitHub token\u{201d}; the command `demi credentials read 'github token' token` can now run."
        ),
        "{}",
        woken[woken.len() - 2]
    );
    assert!(woken.last().unwrap().contains("10"), "{}", woken.last().unwrap());
    let allowances: i64 = harness
        .control_database()
        .query_row(
            "SELECT count(*) FROM credential_allowances WHERE credential_id = ?1",
            [&token],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(allowances, 1);

    // A removal needs Manage Credentials, as any category does, and waits
    // for the user's answer.
    scripts.root(
        FIRST,
        vec![
            background("t3", "demi credentials remove 'GITHUB TOKEN'; echo exit=$?"),
            say("I asked the user"),
            say("removed"),
            say("removed"),
        ],
    );
    socket.chat("m2", "Remove the token").await;
    let waiting = last_asked(&scripts, FIRST);
    assert!(
        waiting.contains("waiting for the user's permission to manage credentials; its end will report to you, so do not run it again"),
        "{waiting}"
    );
    assert_eq!(credentials(&backend, &master).await.credentials.len(), 1);
    let asked = permissions(&backend, &master, FIRST).await;
    assert_eq!(asked.requests.len(), 1, "{asked:?}");
    assert_eq!(categories(&asked.requests[0])[0].id, "credentials.manage");
    let allowed = decide(
        &backend,
        &master,
        asked.requests[0].id().as_str(),
        json!({ "decision": "allow" }),
    )
    .await;
    assert_eq!(allowed.status, StatusCode::NO_CONTENT);
    let removed = crate::permissions::settled(
        &scripts,
        &backend,
        &master,
        FIRST,
        &["The user allowed this conversation to manage credentials.", "(t3) ended with exit code 0"],
    )
    .await;
    assert!(removed.contains("Removed the credential \u{201c}GitHub token\u{201d}."), "{removed}");
    assert!(credentials(&backend, &master).await.credentials.is_empty());
    backend.close().await;
}

/// Settings lists a visible field's value and never a secret one's, a
/// broken rule is refused in the editor's words, and a name another
/// credential has, in any case, is refused.
#[tokio::test]
async fn a_new_credential_lists_its_visible_values_and_takes_a_name_once() {
    let (backend, master) = Harness::new().start_set_up().await;
    saved(
        &backend,
        &master,
        json!({
            "name": "Linear key", "note": "For the tracker", "keep": "always",
            "fields": [
                { "name": "key", "label": "API key", "kind": "secret", "value": "lin_api_1" },
                { "name": "team", "label": "Team", "kind": "visible", "value": "ENG" }
            ]
        }),
    )
    .await;
    // The list carries a visible field's value and never a secret one's.
    let listed = credentials(&backend, &master).await.credentials;
    let values: Vec<(&str, Option<&str>)> = listed[0]
        .fields
        .iter()
        .map(|field| (field.name.as_str(), field.value.as_deref()))
        .collect();
    assert_eq!(values, [("key", None), ("team", Some("ENG"))]);
    // A broken rule is refused in a sentence the editor shows as it is.
    let long = backend
        .post(
            "/api/credentials",
            Some(&master),
            json!({
                "name": "Long", "keep": "always",
                "fields": [{ "name": "key", "label": "L".repeat(81), "kind": "secret", "value": "x" }]
            }),
        )
        .await;
    assert_eq!(long.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody));
    assert_eq!(long.error().message, "Enter a label of up to 80 characters for each field.");
    saved(
        &backend,
        &master,
        json!({
            "name": "Other", "keep": "1d",
            "fields": [{ "name": "key", "label": "Key", "kind": "secret", "value": "x" }]
        }),
    )
    .await;

    // A name is one whatever its case.
    let again = backend
        .post(
            "/api/credentials",
            Some(&master),
            json!({
                "name": "LINEAR key", "keep": "always",
                "fields": [{ "name": "key", "label": "Key", "kind": "secret", "value": "y" }]
            }),
        )
        .await;
    assert_eq!(again.refusal(), (StatusCode::CONFLICT, ErrorCode::CredentialNameTaken));
    let names: Vec<String> = credentials(&backend, &master)
        .await
        .credentials
        .into_iter()
        .map(|credential| credential.name)
        .collect();
    assert_eq!(names, ["Linear key", "Other"]);
    backend.close().await;
}

/// A credential kept for an hour is removed when the hour has passed, by
/// the shard's timer on the paused clock, and every page is told; one whose
/// time came while the backend was down is gone when it starts again.
#[tokio::test(flavor = "local")]
async fn a_credential_kept_for_a_while_is_removed_at_its_time_and_at_start() {
    let mut harness = Harness::new();
    harness.shards = Some(ShardPlacement::Inline);
    let (backend, master) = harness.start_set_up().await;
    let mut page = backend.sync(&master).await;
    page.snapshot().await;
    let body = |name: &str| {
        json!({
            "name": name, "keep": "1h",
            "fields": [{ "name": "key", "label": "Key", "kind": "secret", "value": "v" }]
        })
    };
    saved(&backend, &master, body("Hourly")).await;
    saved(&backend, &master, json!({
        "name": "Kept", "keep": "always",
        "fields": [{ "name": "key", "label": "Key", "kind": "secret", "value": "v" }]
    }))
    .await;
    page.until(|event| matches!(event, SyncEvent::Credentials { credentials } if credentials.len() == 2))
        .await;

    // A minute short of the hour, nothing goes.
    harness.clock.advance(jiff::SignedDuration::from_mins(59));
    elapse(Duration::from_secs(59 * 60)).await;
    assert_eq!(credentials(&backend, &master).await.credentials.len(), 2);

    harness.clock.advance(jiff::SignedDuration::from_mins(1));
    elapse(Duration::from_secs(60)).await;
    page.until(|event| matches!(event, SyncEvent::Credentials { credentials } if credentials.len() == 1))
        .await;
    let left = credentials(&backend, &master).await.credentials;
    assert_eq!(left[0].name, "Kept");

    // Down when the next one's hour passes: the start removes it, before
    // any page asks.
    saved(&backend, &master, body("Later")).await;
    backend.close().await;
    harness.clock.advance(jiff::SignedDuration::from_hours(2));
    let backend = harness.start().await;
    let stored: Vec<String> = harness
        .control_database()
        .prepare("SELECT name FROM user_credentials ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(stored, ["Kept"]);
    backend.close().await;
}
