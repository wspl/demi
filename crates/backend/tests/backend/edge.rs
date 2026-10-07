//! Request bodies and the web app build (`web-api.md` § Request bodies,
//! § Serving the web app build).

use demi_web_api_protocol::auth::Identity;
use demi_web_api_protocol::error::ErrorCode;
use reqwest::{Body, Method, StatusCode};
use serde_json::json;

use crate::support::{Answer, Harness, MASTER_EMAIL, MASTER_PASSWORD, Session, answer};

const JSON_BODY_LIMIT: usize = 1024 * 1024;

fn nickname_over_the_limit() -> String {
    json!({ "nickname": "x".repeat(JSON_BODY_LIMIT) }).to_string()
}

async fn patch_me(url: &str, session: &Session, body: Body) -> Answer {
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .patch(format!("{url}/api/auth/me"))
        .header("cookie", &session.cookie)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    answer(response).await
}

/// A `PATCH /api/auth/me` without a session whose head declares a body over
/// the limit, sent without the body: the status and body of the answer.
async fn declared_over_the_limit(url: &str) -> (u16, String) {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let address = url.strip_prefix("http://").unwrap();
    let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
    let head = format!(
        "PATCH /api/auth/me HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        JSON_BODY_LIMIT + 1
    );
    socket.write_all(head.as_bytes()).await.unwrap();
    let mut answer = Vec::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        socket.read_to_end(&mut answer),
    )
    .await
    .expect("the backend answers and closes within the hang guard")
    .unwrap();
    let answer = String::from_utf8(answer).unwrap();
    let (head, body) = answer.split_once("\r\n\r\n").unwrap();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, body.to_owned())
}

#[tokio::test]
async fn a_json_body_over_its_limit_is_refused_before_it_is_read() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;

    // With its length declared up front.
    let declared = patch_me(&backend.url, &master, Body::from(nickname_over_the_limit())).await;
    assert_eq!(
        declared.refusal(),
        (StatusCode::PAYLOAD_TOO_LARGE, ErrorCode::TooLarge)
    );

    // Without one, the body is counted as it arrives.
    let chunks = futures_util::stream::iter([Ok::<_, std::io::Error>(nickname_over_the_limit())]);
    let streamed = patch_me(&backend.url, &master, Body::wrap_stream(chunks)).await;
    assert_eq!(
        streamed.refusal(),
        (StatusCode::PAYLOAD_TOO_LARGE, ErrorCode::TooLarge)
    );

    // The gate runs first: without a session a body over the limit is 401.
    // The request declares the length and sends no byte of the body, which
    // the backend never reads: a client still writing one when the refusal
    // comes can meet the connection's reset instead of the answer.
    let (status, body) = declared_over_the_limit(&backend.url).await;
    assert_eq!(status, 401, "{body}");
    let refusal: demi_web_api_protocol::error::ErrorBody = serde_json::from_str(&body).unwrap();
    assert_eq!(refusal.code, ErrorCode::Unauthenticated);
    backend.close().await;
}

#[tokio::test]
async fn a_json_body_is_read_whatever_its_content_type_and_must_match_its_type() {
    let harness = Harness::new();
    let (backend, _) = harness.start_set_up().await;
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let login = async |body: String| {
        let response = client
            .post(format!("{}/api/auth/login", backend.url))
            .body(body)
            .send()
            .await
            .unwrap();
        answer(response).await
    };

    let untyped =
        login(json!({ "email": MASTER_EMAIL, "password": MASTER_PASSWORD }).to_string()).await;
    assert_eq!(untyped.status, StatusCode::OK);
    assert_eq!(untyped.json::<Identity>().user.email.as_str(), MASTER_EMAIL);

    for (body, field) in [
        (
            json!({ "email": MASTER_EMAIL, "password": MASTER_PASSWORD, "remember": true })
                .to_string(),
            "remember",
        ),
        (json!({ "email": MASTER_EMAIL }).to_string(), "password"),
        (
            json!({ "email": MASTER_EMAIL, "password": "" }).to_string(),
            "password",
        ),
        (
            json!({ "email": MASTER_EMAIL, "password": 5 }).to_string(),
            "password",
        ),
        (String::new(), ""),
        ("{".to_owned(), ""),
    ] {
        let refused = login(body.clone()).await;
        let error = refused.error();
        assert_eq!(
            (refused.status, error.code),
            (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody),
            "{body}"
        );
        assert!(error.message.contains(field), "{body}: {}", error.message);
    }
    backend.close().await;
}

#[tokio::test]
async fn the_web_app_build_is_served_with_deep_navigation_while_api_misses_stay_json() {
    let harness = Harness::new().with_web(&[
        ("index.html", "<html>fixture page</html>"),
        ("main.js", "export const fixture = true"),
        ("build.json", r#"{ "build": "b7f3" }"#),
    ]);
    let (backend, master) = harness.start_set_up().await;
    let page = |path: &'static str| backend.send(Method::GET, path, None, None);
    let html = async |path: &str| {
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(format!("{}{path}", backend.url))
            .header("accept", "text/html")
            .send()
            .await
            .unwrap();
        answer(response).await
    };

    let deep = html("/conversation/example").await;
    assert_eq!(deep.status, StatusCode::OK);
    assert!(String::from_utf8_lossy(&deep.body).contains("fixture page"));
    let script = page("/main.js").await;
    assert_eq!(script.status, StatusCode::OK);
    assert!(String::from_utf8_lossy(&script.body).contains("fixture = true"));

    let missing_asset = html("/missing.js").await;
    assert_eq!(
        missing_asset.refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::NotFound)
    );
    let not_a_page = page("/conversation/example").await;
    assert_eq!(
        not_a_page.refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::NotFound)
    );
    let api_miss = backend
        .send(
            Method::GET,
            "/api/no-such-resource",
            Some(&master.cookie),
            None,
        )
        .await;
    assert_eq!(
        api_miss.refusal(),
        (StatusCode::NOT_FOUND, ErrorCode::NotFound)
    );
    // Every page learns which build the backend serves, so a page of
    // another build can tell it is out of date.
    let state = backend.sync(&master).await.snapshot().await;
    assert_eq!(state.web_build.as_deref(), Some("b7f3"));
    backend.close().await;
}

/// A browser's request for `path`, sending back the validators an earlier
/// answer gave, as a reload does.
async fn reloaded(url: &str, path: &str, earlier: Option<&Answer>) -> Answer {
    let mut request = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .get(format!("{url}{path}"))
        .header("accept", "text/html");
    if let Some(earlier) = earlier {
        if let Some(etag) = earlier.headers.get("etag") {
            request = request.header("if-none-match", etag);
        }
        if let Some(modified) = earlier.headers.get("last-modified") {
            request = request.header("if-modified-since", modified);
        }
    }
    answer(request.send().await.unwrap()).await
}

#[tokio::test]
async fn a_new_build_whose_files_are_older_is_served_to_a_page_that_reloads() {
    // Cost: two backend starts on a temporary directory; under a second.
    let harness = Harness::new().with_web(&[
        ("index.html", "<html>build one</html>"),
        ("build.json", r#"{ "build": "one" }"#),
    ]);
    let (backend, _) = harness.start_set_up().await;
    let first = reloaded(&backend.url, "/chat/example", None).await;
    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(first.headers["cache-control"], "no-cache");
    let build = reloaded(&backend.url, "/build.json", None).await;
    assert_eq!(build.headers["cache-control"], "no-cache");
    // Unchanged, the page is not sent again.
    let again = reloaded(&backend.url, "/chat/example", Some(&first)).await;
    assert_eq!(again.status, StatusCode::NOT_MODIFIED);
    backend.close().await;

    // The server returns to a build made before the one the page holds, as
    // a rollback does: its files are older.
    let web = harness.web_dir();
    let older = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
    for (name, content) in [
        ("index.html", "<html>build zero</html>"),
        ("build.json", r#"{ "build": "zero" }"#),
    ] {
        std::fs::write(web.join(name), content).unwrap();
        std::fs::File::options()
            .write(true)
            .open(web.join(name))
            .unwrap()
            .set_modified(older)
            .unwrap();
    }
    std::fs::create_dir_all(web.join("assets")).unwrap();
    std::fs::write(web.join("assets/index-zero.js"), "export {}").unwrap();
    let backend = harness.start().await;
    // A hashed file never changes: its name does.
    let script = reloaded(&backend.url, "/assets/index-zero.js", None).await;
    assert_eq!(script.status, StatusCode::OK);
    assert!(script.headers["cache-control"].to_str().unwrap().contains("immutable"));
    let reload = reloaded(&backend.url, "/chat/example", Some(&first)).await;
    assert_eq!(reload.status, StatusCode::OK);
    assert_eq!(String::from_utf8_lossy(&reload.body), "<html>build zero</html>");
    let build = reloaded(&backend.url, "/build.json", Some(&build)).await;
    assert_eq!(build.status, StatusCode::OK);
    assert_eq!(String::from_utf8_lossy(&build.body), r#"{ "build": "zero" }"#);
    backend.close().await;
}

#[tokio::test]
async fn a_page_socket_message_over_the_limit_fails_the_socket() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    crate::conversations::create(&backend, &master, crate::conversations::FIRST).await;
    let mut socket =
        crate::conversations::Socket::connect(&backend, &master, crate::conversations::FIRST).await;
    // A frame the backend would read, were it within the limit.
    let frame = crate::conversations::send("m1", &"x".repeat(JSON_BODY_LIMIT));
    socket
        .send_refused_text(serde_json::to_string(&frame).unwrap())
        .await;
    assert_eq!(socket.closed().await, None);
    backend.close().await;
}
