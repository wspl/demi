//! A test build's control socket (`DEMI_TEST_CONTROL`) and tuning file
//! (`DEMI_TEST_TUNING`), which the Go API suite drives: the real
//! `demi-backend` process serves each operation of the control's table, the
//! clock it moves is the one the backend reads, and the process ends on
//! SIGTERM with the socket removed. The suite that uses the control tests
//! the backend; this scenario tests the control.

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use futures_util::StreamExt as _;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::process::{Child, Command};
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

use crate::support::{Harness, MASTER_EMAIL, MASTER_PASSWORD};

/// The conversation of the gate the scenario enters.
const CONVERSATION: &str = "5e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a01";

/// One connection to the control socket, whose requests the scenario sends
/// one at a time.
struct Control {
    read: tokio::io::Lines<BufReader<OwnedReadHalf>>,
    write: OwnedWriteHalf,
    next: u64,
}

impl Control {
    async fn connect(path: &Path, backend: &mut Child) -> Self {
        for _ in 0..1_000 {
            if let Ok(stream) = UnixStream::connect(path).await {
                let (read, write) = stream.into_split();
                return Self { read: BufReader::new(read).lines(), write, next: 0 };
            }
            assert!(backend.try_wait().unwrap().is_none(), "the backend ended before it served");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("the control socket never accepted a connection");
    }

    /// Sends the request and answers its id, without waiting for the reply.
    async fn start(&mut self, op: &str, params: Value) -> String {
        self.next += 1;
        let id = self.next.to_string();
        let request = json!({ "id": id, "op": op, "params": params });
        self.write.write_all(format!("{request}\n").as_bytes()).await.unwrap_or_else(|error| panic!("{op} cannot be sent: {error}"));
        id
    }

    /// The replies of the requests `ids`, whatever order they come in.
    async fn replies(&mut self, ids: &[&str]) -> HashMap<String, Result<Value, String>> {
        let mut replies = HashMap::new();
        while replies.len() < ids.len() {
            let line = tokio::time::timeout(Duration::from_secs(20), self.read.next_line())
                .await
                .expect("a reply never came")
                .unwrap()
                .expect("the control ended while a request waited");
            let reply: Value = serde_json::from_str(&line).unwrap();
            let id = reply["id"].as_str().unwrap().to_owned();
            assert!(ids.contains(&id.as_str()), "{line}");
            let result = match reply["type"].as_str() {
                Some("ok") => Ok(reply["result"].clone()),
                Some("error") => Err(reply["message"].as_str().unwrap().to_owned()),
                _ => panic!("a reply of no type: {line}"),
            };
            replies.insert(id, result);
        }
        replies
    }

    /// The operation's result, or the message the control refused it with.
    async fn call(&mut self, op: &str, params: Value) -> Result<Value, String> {
        let id = self.start(op, params).await;
        self.replies(&[&id]).await.remove(&id).unwrap()
    }

    async fn ok(&mut self, op: &str, params: Value) -> Value {
        self.call(op, params).await.unwrap_or_else(|message| panic!("{op} was refused: {message}"))
    }
}

/// The session cookie of an account that signs in.
async fn login_cookie(http: &reqwest::Client, url: &str, email: &str, password: &str) -> String {
    let login = http
        .post(format!("{url}/api/auth/login"))
        .header("content-type", "application/json")
        .body(json!({ "email": email, "password": password }).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(login.status(), 200);
    let set = login.headers().get("set-cookie").expect("a login sets the session cookie").to_str().unwrap();
    set.split(';').next().unwrap().to_owned()
}

#[tokio::test]
async fn the_control_serves_each_of_its_operations_from_the_backend_process_and_ends_with_it() {
    let harness = Harness::new();
    let directory = tempfile::Builder::new().prefix("demi-control-").tempdir().unwrap();
    let path = |name: &str| directory.path().join(name);
    std::fs::write(path("native.json"), r#"{ "releases": [], "store": { "provider": "local" } }"#).unwrap();
    std::fs::write(path("tuning.json"), r#"{ "runners": { "pingMs": 0 }, "mail": true }"#).unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let url = format!("http://127.0.0.1:{port}");
    let mut backend = Command::new(env!("CARGO_BIN_EXE_demi-backend"))
        .env_clear()
        .env("DEMI_BACKEND_DATA", path("data"))
        .env("DEMI_BACKEND_PORT", port.to_string())
        .env("DEMI_BACKEND_PUBLIC_URL", &url)
        .env("DEMI_INSTANCE_MODE", "shared")
        .env("DEMI_MACHINES_SOCKET", harness.manager.socket())
        .env("DEMI_NATIVE_CONFIG", path("native.json"))
        .env("DEMI_TEST_CONTROL", path("control.sock"))
        .env("DEMI_TEST_TUNING", path("tuning.json"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut control = Control::connect(&path("control.sock"), &mut backend).await;
    let http = reqwest::Client::builder().no_proxy().build().unwrap();

    // The clock stands still where the control puts it, and the backend reads
    // it: the account set up now is created at that moment.
    let start: jiff::Timestamp = "2026-09-24T08:00:00Z".parse().unwrap();
    let moved = control.ok("clock.advance", json!({ "byMs": 1_500 })).await;
    assert_eq!(moved["atMs"], json!(start.as_millisecond() + 1_500));
    let at: jiff::Timestamp = "2030-01-02T03:04:05Z".parse().unwrap();
    let set = control.ok("clock.set", json!({ "atMs": at.as_millisecond() })).await;
    assert_eq!(set["atMs"], json!(at.as_millisecond()));
    let setup = http
        .post(format!("{url}/api/setup"))
        .header("content-type", "application/json")
        .body(json!({ "email": MASTER_EMAIL, "password": MASTER_PASSWORD }).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(setup.status(), 201);
    let master: Value = serde_json::from_str(&setup.text().await.unwrap()).unwrap();
    assert_eq!(master["user"]["createdAt"], json!("2030-01-02T03:04:05.000Z"));
    let later = control.ok("clock.advance", json!({ "byMs": 60_000 })).await;
    assert_eq!(later["atMs"], json!(at.as_millisecond() + 60_000));

    // An account added without a route of the API signs in.
    let user = control
        .ok("users.add", json!({ "email": "added@example.test", "password": "added-pass-1", "role": "user" }))
        .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let login = http
        .post(format!("{url}/api/auth/login"))
        .header("content-type", "application/json")
        .body(json!({ "email": "added@example.test", "password": "added-pass-1" }).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(login.status(), 200);
    let signed_in: Value = serde_json::from_str(&login.text().await.unwrap()).unwrap();
    assert_eq!(signed_in["user"]["id"], json!(user));
    let taken = control
        .call("users.add", json!({ "email": "added@example.test", "password": "added-pass-1", "role": "user" }))
        .await;
    assert!(taken.is_err(), "an address taken is added twice");

    // A hold is named, waited at and released once; a lease of the file gate
    // is entered, waited behind and released.
    for target in ["commits", "hello:token_lookup", "hello:bind", "sync:snapshot", "sync:changes"] {
        let hold = control.ok("hold", json!({ "target": target })).await["id"].as_str().unwrap().to_owned();
        control.ok("hold.wait", json!({ "hold": hold, "count": 0 })).await;
        control.ok("release", json!({ "id": hold })).await;
        let released_twice = control.call("release", json!({ "id": hold })).await;
        assert!(released_twice.is_err(), "{target}: a hold is released twice");
    }
    let lease = control
        .ok("gate.enter", json!({ "user": user, "conversation": CONVERSATION, "purpose": "demand" }))
        .await["id"]
        .as_str()
        .unwrap()
        .to_owned();
    control.ok("gate.waiting", json!({ "user": user, "conversation": CONVERSATION, "count": 0 })).await;
    let waited_at_a_lease = control.call("hold.wait", json!({ "hold": lease, "count": 1 })).await;
    assert!(waited_at_a_lease.is_err(), "nothing waits at a lease");
    control.ok("release", json!({ "id": lease })).await;

    // Retention runs at once, the object store's tally and the mailbox answer,
    // and the mail transport fails on request.
    control.ok("retention.run", json!({ "user": user })).await;
    let tally = control.ok("objects.count", json!({})).await;
    for member in ["puts", "bytesPut", "gets", "heads", "mostGetsAtOnce", "lists", "deletes"] {
        assert!(tally[member].is_u64(), "{member} in {tally}");
    }
    assert_eq!(control.ok("mail.list", json!({})).await, json!({ "mail": [] }));
    control.ok("mail.fail", json!({ "failing": true })).await;
    control.ok("mail.fail", json!({ "failing": false })).await;

    // What is not a request ends the connection, and the control serves on.
    control.write.write_all(b"not a request\n").await.unwrap();
    let ended = tokio::time::timeout(Duration::from_secs(20), control.read.next_line()).await.unwrap();
    assert!(matches!(ended, Ok(None)), "{ended:?}");
    let mut again = Control::connect(&path("control.sock"), &mut backend).await;
    assert_eq!(again.ok("mail.list", json!({})).await, json!({ "mail": [] }));

    // What a connection holds ends with the connection. A hold on a page's
    // synchronization channel keeps the snapshot back, and a lease of a
    // conversation's file gate makes an archive refuse for running work; once
    // the connection that took them closes, the snapshot comes and the archive
    // goes through.
    let cookie = login_cookie(&http, &url, "added@example.test", "added-pass-1").await;
    let mut holding = Control::connect(&path("control.sock"), &mut backend).await;
    let hold = holding.ok("hold", json!({ "target": "sync:snapshot" })).await["id"].as_str().unwrap().to_owned();
    let mut request = format!("ws://127.0.0.1:{port}/api/sync").into_client_request().unwrap();
    request.headers_mut().insert("cookie", cookie.parse().unwrap());
    request.headers_mut().insert("origin", url.parse().unwrap());
    let (mut page, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    holding.ok("hold.wait", json!({ "hold": hold, "count": 1 })).await;
    drop(holding);
    let snapshot = tokio::time::timeout(Duration::from_secs(20), page.next())
        .await
        .expect("the snapshot never came after the hold's connection closed");
    assert!(matches!(snapshot, Some(Ok(_))), "{snapshot:?}");

    let created = http
        .post(format!("{url}/api/conversations"))
        .header("cookie", &cookie)
        .header("content-type", "application/json")
        .body(json!({ "id": CONVERSATION }).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 201);
    let archive = || async {
        http.patch(format!("{url}/api/conversations/{CONVERSATION}"))
            .header("cookie", &cookie)
            .header("content-type", "application/json")
            .body(json!({ "archived": true }).to_string())
            .send()
            .await
            .unwrap()
            .status()
    };
    let mut leasing = Control::connect(&path("control.sock"), &mut backend).await;
    leasing.ok("gate.enter", json!({ "user": user, "conversation": CONVERSATION, "purpose": "demand" })).await;
    assert_eq!(archive().await, 409, "an archive goes through while a lease holds the gate");
    drop(leasing);
    let mut archived = false;
    for _ in 0..1_000 {
        if archive().await == 200 {
            archived = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(archived, "the lease outlived its connection");

    // A release ends the waits at its hold, and frees the flow at once.
    let hold = again.ok("hold", json!({ "target": "sync:snapshot" })).await["id"].as_str().unwrap().to_owned();
    let mut request = format!("ws://127.0.0.1:{port}/api/sync").into_client_request().unwrap();
    request.headers_mut().insert("cookie", cookie.parse().unwrap());
    request.headers_mut().insert("origin", url.parse().unwrap());
    let (mut page, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    again.ok("hold.wait", json!({ "hold": hold, "count": 1 })).await;
    let waiting = again.start("hold.wait", json!({ "hold": hold, "count": 9 })).await;
    let releasing = again.start("release", json!({ "id": hold })).await;
    let mut replies = again.replies(&[&waiting, &releasing]).await;
    assert!(replies.remove(&waiting).unwrap().is_err(), "a wait for nine pages succeeded");
    assert!(replies.remove(&releasing).unwrap().is_ok());
    let snapshot = tokio::time::timeout(Duration::from_secs(20), page.next())
        .await
        .expect("the snapshot never came after the release");
    assert!(matches!(snapshot, Some(Ok(_))), "{snapshot:?}");

    // A stop signal ends the process cleanly and removes the socket.
    let pid = backend.id().unwrap().to_string();
    let signalled = Command::new("kill").args(["-TERM", &pid]).status().await.unwrap();
    assert!(signalled.success());
    let status = tokio::time::timeout(Duration::from_secs(20), backend.wait()).await.unwrap().unwrap();
    assert!(status.success(), "the backend ended with {status}");
    assert!(!path("control.sock").exists(), "the control's socket stays behind");
}
