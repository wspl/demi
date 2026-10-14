//! Request IDs, the page's own error reports and the server log
//! (`backend.md` § Request IDs and the server log).

use demi_web_api_protocol::auth::Role;
use demi_web_api_protocol::diagnostics::{LogLevel, ServerLog};
use demi_web_api_protocol::error::ErrorCode;
use reqwest::StatusCode;
use serde_json::json;

use crate::support::{Answer, Harness, ServerLogCapture, Session, TestBackend, eventually};

/// The ID an answer's `X-Request-Id` names.
pub(crate) fn request_id(answer: &Answer) -> String {
    answer.headers["x-request-id"].to_str().unwrap().to_owned()
}

/// The lines the server log keeps of `request`, as an administrator reads
/// them.
pub(crate) async fn lines_of(backend: &TestBackend, admin: &Session, request: &str) -> ServerLog {
    let read = backend
        .get(&format!("/api/server/log?request={request}"), Some(admin))
        .await;
    assert_eq!(read.status, StatusCode::OK, "{}", String::from_utf8_lossy(&read.body));
    read.json()
}

/// Every answer names its request, and an error carries the same ID in its
/// body, which the page shows: `req_` and 16 lowercase base-32 characters,
/// another for each request.
#[tokio::test]
async fn every_answer_names_its_request_and_an_error_carries_the_id() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;

    let answered = backend.get("/api/setup", None).await;
    assert_eq!(answered.status, StatusCode::OK);
    let first = request_id(&answered);
    let refused = backend.get("/api/no-such-route", Some(&master)).await;
    assert_eq!(refused.refusal(), (StatusCode::NOT_FOUND, ErrorCode::NotFound));
    let id = request_id(&refused);
    assert_eq!(refused.error().request_id.as_str(), id);
    let random = id.strip_prefix("req_").unwrap();
    assert_eq!(random.len(), 16, "{id}");
    assert!(
        random.chars().all(|c| c.is_ascii_digit() || c.is_ascii_lowercase()),
        "{id}"
    );
    assert_ne!(id, first);
    // A refusal before any route, by the session gate, carries it too.
    let signed_out = backend.get("/api/settings", None).await;
    assert_eq!(signed_out.error().request_id.as_str(), request_id(&signed_out));
    backend.close().await;
}

/// A request's work in the user's shard, on another thread, logs under the
/// request's ID with its caller, and an administrator finds those lines by
/// the ID; anyone else is refused. Polls the log, which commits at most every
/// 200 ms; well under a second.
#[tokio::test]
async fn the_server_log_finds_a_requests_lines_from_its_shard_for_an_administrator_alone() {
    let harness = Harness::new();
    let log = ServerLogCapture::open(&harness);
    let (backend, master) = harness.start_set_up().await;
    let created = backend
        .post(
            "/api/credentials",
            Some(&master),
            json!({
                "name": "Deploy key", "keep": "always",
                "fields": [{ "name": "key", "label": "Key", "kind": "secret", "value": "s3cret-value" }]
            }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&created.body));
    let credential = created.json::<serde_json::Value>()["credential"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    // The shard logs that a page read the values.
    let revealed = backend
        .get(&format!("/api/credentials/{credential}/values"), Some(&master))
        .await;
    assert_eq!(revealed.status, StatusCode::OK);
    let id = request_id(&revealed);

    let revealed_line = || async {
        lines_of(&backend, &master, &id)
            .await
            .lines
            .into_iter()
            .find(|line| line.message == "a page revealed a credential's values")
    };
    eventually("the shard's line is in the log", || async {
        revealed_line().await.is_some()
    })
    .await;
    let line = revealed_line().await.unwrap();
    assert_eq!(line.level, LogLevel::Info);
    assert!(line.target.starts_with("demi_backend_user_shard"), "{line:?}");
    assert_eq!(line.fields["request_id"], json!(id));
    assert_eq!(line.fields["user_id"], json!(master.user.id.as_str()));
    assert_eq!(line.fields["method"], json!("GET"));
    // The line names the credential, never its value.
    let text = serde_json::to_string(&line).unwrap();
    assert!(!text.contains("s3cret-value"), "{text}");

    // Every request logs one line as it is answered, a refused one
    // included, so any ID a user copies finds at least its request.
    let refused = backend.get("/api/no-such-route", Some(&master)).await;
    let refused_id = request_id(&refused);
    let answered_line = || async {
        lines_of(&backend, &master, &refused_id)
            .await
            .lines
            .into_iter()
            .find(|line| line.message == "a request was answered")
    };
    eventually("the refused request's line is in the log", || async {
        answered_line().await.is_some()
    })
    .await;
    let answered = answered_line().await.unwrap();
    assert_eq!(answered.level, LogLevel::Info);
    for (field, value) in [
        ("status", json!(404)),
        ("code", json!("not_found")),
        ("method", json!("GET")),
        ("path", json!("/api/no-such-route")),
        ("user_id", json!(master.user.id.as_str())),
    ] {
        assert_eq!(answered.fields[field], value, "{field} in {answered:?}");
    }
    assert!(answered.fields["duration_ms"].is_u64(), "{answered:?}");

    harness.add_user("ana@example.com", "correct horse battery", Role::User);
    let ana = backend.login("ana@example.com", "correct horse battery").await;
    let refused = backend
        .get(&format!("/api/server/log?request={id}"), Some(&ana))
        .await;
    assert_eq!(refused.refusal(), (StatusCode::FORBIDDEN, ErrorCode::Forbidden));
    backend.close().await;
    log.close().await;
}

/// A page's report of its own failure is logged as a warning under the
/// report's ID; a session sends thirty a minute, and the next is refused
/// with 429, which still names its request.
#[tokio::test]
async fn a_page_reports_its_own_errors_thirty_a_minute() {
    let harness = Harness::new();
    let log = ServerLogCapture::open(&harness);
    let (backend, master) = harness.start_set_up().await;
    let report = json!({
        "title": "Could Not Read the Plugin's Data",
        "message": "Invalid server response",
        "detail": "sections.0.title: expected string, received number",
    });
    let mut ids = Vec::new();
    for _ in 0..30 {
        let reported = backend.post("/api/page-errors", Some(&master), report.clone()).await;
        assert_eq!(reported.status, StatusCode::NO_CONTENT);
        ids.push(request_id(&reported));
    }
    let refused = backend.post("/api/page-errors", Some(&master), report.clone()).await;
    assert_eq!(refused.refusal(), (StatusCode::TOO_MANY_REQUESTS, ErrorCode::RateLimited));
    assert_eq!(refused.error().request_id.as_str(), request_id(&refused));

    let reported_line = || async {
        lines_of(&backend, &master, &ids[0])
            .await
            .lines
            .into_iter()
            .find(|line| line.message == "the page reported an error")
    };
    eventually("the report is in the log", || async {
        reported_line().await.is_some()
    })
    .await;
    let line = reported_line().await.unwrap();
    assert_eq!(line.level, LogLevel::Warn);
    assert_eq!(line.fields["title"], json!("Could Not Read the Plugin's Data"));
    assert_eq!(
        line.fields["detail"],
        json!("sections.0.title: expected string, received number")
    );
    backend.close().await;
    log.close().await;
}
