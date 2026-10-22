//! Request IDs and the log's lines of a request, as `log.sqlite` holds them
//! (`crates/backend-http/src/request_id.rs`).

use demi_shared_types::LogLevel;
use demi_web_api_protocol::error::ErrorCode;
use reqwest::StatusCode;
use serde_json::json;

use crate::support::{Answer, Harness, ServerLogCapture, TestBackend, eventually};

/// The ID an answer's `X-Request-Id` names.
pub(crate) fn request_id(answer: &Answer) -> String {
    answer.headers["x-request-id"].to_str().unwrap().to_owned()
}

/// One line of the log as these scenarios read it: its event and message,
/// its fields with the ID columns among them.
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct ServerLogLine {
    pub level: LogLevel,
    pub target: String,
    pub message: String,
    pub fields: serde_json::Map<String, serde_json::Value>,
}

/// The lines of the log, oldest first.
pub(crate) struct ServerLog {
    pub lines: Vec<ServerLogLine>,
}

impl TestBackend {
    /// The newest 1,000 lines of the log, or the lines of `request`, read
    /// from `log.sqlite` as whoever diagnoses reads it.
    pub(crate) fn server_log(&self, request: Option<&str>) -> ServerLog {
        let path = self.data_dir.join("log.sqlite");
        let Ok(connection) = rusqlite::Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) else {
            return ServerLog { lines: Vec::new() };
        };
        let mut statement = connection
            .prepare(
                "SELECT level, event, request_id, user_id, conversation_id, turn_id, host_id, page_id, fields
                 FROM (SELECT * FROM events WHERE ?1 IS NULL OR request_id = ?1 ORDER BY id DESC LIMIT 1000)
                 ORDER BY id",
            )
            .unwrap();
        let lines = statement
            .query_map([request], |row| {
                let level: String = row.get(0)?;
                let mut fields: serde_json::Map<String, serde_json::Value> =
                    serde_json::from_str(&row.get::<_, String>(8)?).unwrap();
                for (index, name) in
                    [(2, "request_id"), (3, "user_id"), (4, "conversation_id"), (5, "turn_id"), (6, "host_id"), (7, "page_id")]
                {
                    if let Some(value) = row.get::<_, Option<String>>(index)? {
                        fields.insert(name.into(), value.into());
                    }
                }
                let message = fields
                    .get("message")
                    .and_then(|message| message.as_str())
                    .unwrap_or_default()
                    .to_owned();
                Ok(ServerLogLine {
                    level: serde_json::from_value(level.into()).unwrap(),
                    target: row.get(1)?,
                    message,
                    fields,
                })
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        ServerLog { lines }
    }
}

/// The lines the log keeps of `request`.
pub(crate) fn lines_of(backend: &TestBackend, request: &str) -> ServerLog {
    backend.server_log(Some(request))
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
/// request's ID with its caller, and the ID finds those lines. Polls the log,
/// which commits at most every 200 ms; well under a second.
#[tokio::test]
async fn the_log_finds_a_requests_lines_from_its_shard() {
    let harness = Harness::new();
    let log = ServerLogCapture::open(&harness);
    let (backend, master) = harness.start_set_up().await;
    let created = backend
        .post(
            "/api/automations",
            Some(&master),
            json!({
                "name": "Pull requests",
                "trigger": { "kind": "url" },
                "timeZone": "UTC",
                "target": { "kind": "new", "target": { "kind": "cloud" } },
                "prompt": "Review what the pull request changed.",
                "paused": true
            }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&created.body));
    let automation = created.json::<serde_json::Value>()["automation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    // The shard logs that a page showed the automation's secret.
    let revealed = backend
        .get(&format!("/api/automations/{automation}/secret"), Some(&master))
        .await;
    assert_eq!(revealed.status, StatusCode::OK);
    let secret = revealed.json::<serde_json::Value>()["secret"].as_str().unwrap().to_owned();
    let id = request_id(&revealed);

    let revealed_line = || async {
        lines_of(&backend, &id)
            .lines
            .into_iter()
            .find(|line| line.message == "a page revealed an automation's secret")
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
    // The line names the automation, never its secret.
    let text = serde_json::to_string(&line).unwrap();
    assert!(!text.contains(&secret), "{text}");

    // Every request logs one line as it is answered, a refused one
    // included, so any ID a user copies finds at least its request.
    let refused = backend.get("/api/no-such-route", Some(&master)).await;
    let refused_id = request_id(&refused);
    let answered_line = || async {
        lines_of(&backend, &refused_id)
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

    // The route that read the log is gone.
    let gone = backend
        .get(&format!("/api/server/log?request={id}"), Some(&master))
        .await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
    backend.close().await;
    log.close().await;
}

/// A request whose client goes away before its answer, as a fetch that
/// times out and retries does, logs its line too, under its ID and with
/// its user: a summary read that waits for a migrating conversation is
/// abandoned. A restart and a migration, under a second.
#[tokio::test]
async fn a_request_its_client_abandons_logs_its_line() {
    use tokio::io::AsyncWriteExt as _;

    use demi_backend_database::{DatabaseKind, schema_testing};
    use demi_conversation_socket_protocol::ClientFrame;

    use crate::conversations::{FIRST, Socket, create};
    use crate::support::PATIENCE;

    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    create(&backend, &master, FIRST).await;
    backend.close().await;
    schema_testing::store_as_shipped(
        &harness
            .data_dir()
            .join("conversations")
            .join(format!("{FIRST}.sqlite")),
        DatabaseKind::Conversation,
    );
    let log = ServerLogCapture::open(&harness);
    // The start's search catch-up migrates the database under the hold; the
    // page's open and its summary read come meanwhile.
    let (backend, hold) = harness.start_with_migrations_held().await;
    tokio::time::timeout(PATIENCE, hold.until_migrating(1))
        .await
        .expect("the start's search catch-up migrates the database");
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.send(&ClientFrame::Open { agents: None, draft: None }).await;

    // The page's summary read waits for the migration; its client goes.
    let mut client = tokio::net::TcpStream::connect(backend.address()).await.unwrap();
    let head = format!(
        "GET /api/conversations HTTP/1.1\r\nhost: {}\r\ncookie: {}\r\n\r\n",
        backend.address(),
        master.cookie
    );
    client.write_all(head.as_bytes()).await.unwrap();
    tokio::time::timeout(PATIENCE, hold.until_reads_wait(1))
        .await
        .expect("the summary read waits for the migration");
    drop(client);

    let abandoned = || async {
        backend
            .server_log(None)
            .lines
            .into_iter()
            .find(|line| line.message == "a request was abandoned")
    };
    eventually("the abandoned request's line is in the log", || async {
        abandoned().await.is_some()
    })
    .await;
    let line = abandoned().await.unwrap();
    assert_eq!(line.level, LogLevel::Info);
    for (field, value) in [
        ("method", json!("GET")),
        ("path", json!("/api/conversations")),
        ("user_id", json!(master.user.id.as_str())),
    ] {
        assert_eq!(line.fields[field], value, "{field} in {line:?}");
    }
    assert!(line.fields["duration_ms"].is_u64(), "{line:?}");
    // Its ID finds it as an answered request's would.
    let id = line.fields["request_id"].as_str().unwrap().to_owned();
    let found = lines_of(&backend, &id).lines;
    assert!(
        found.iter().any(|line| line.message == "a request was abandoned"),
        "{found:?}"
    );
    hold.release();
    drop(socket);
    backend.close().await;
    log.close().await;
}
