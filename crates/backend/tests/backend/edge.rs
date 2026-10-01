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
    tokio::time::timeout(std::time::Duration::from_secs(10), socket.read_to_end(&mut answer))
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
    assert_eq!(declared.refusal(), (StatusCode::PAYLOAD_TOO_LARGE, ErrorCode::TooLarge));

    // Without one, the body is counted as it arrives.
    let chunks = futures_util::stream::iter([Ok::<_, std::io::Error>(nickname_over_the_limit())]);
    let streamed = patch_me(&backend.url, &master, Body::wrap_stream(chunks)).await;
    assert_eq!(streamed.refusal(), (StatusCode::PAYLOAD_TOO_LARGE, ErrorCode::TooLarge));

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

    let untyped = login(json!({ "email": MASTER_EMAIL, "password": MASTER_PASSWORD }).to_string()).await;
    assert_eq!(untyped.status, StatusCode::OK);
    assert_eq!(untyped.json::<Identity>().user.email.as_str(), MASTER_EMAIL);

    for (body, field) in [
        (json!({ "email": MASTER_EMAIL, "password": MASTER_PASSWORD, "remember": true }).to_string(), "remember"),
        (json!({ "email": MASTER_EMAIL }).to_string(), "password"),
        (json!({ "email": MASTER_EMAIL, "password": "" }).to_string(), "password"),
        (json!({ "email": MASTER_EMAIL, "password": 5 }).to_string(), "password"),
        (String::new(), ""),
        ("{".to_owned(), ""),
    ] {
        let refused = login(body.clone()).await;
        let error = refused.error();
        assert_eq!((refused.status, error.code), (StatusCode::BAD_REQUEST, ErrorCode::InvalidBody), "{body}");
        assert!(error.message.contains(field), "{body}: {}", error.message);
    }
    backend.close().await;
}

#[tokio::test]
async fn the_web_app_build_is_served_with_deep_navigation_while_api_misses_stay_json() {
    let harness = Harness::new().with_web(&[
        ("index.html", "<html>fixture page</html>"),
        ("main.js", "export const fixture = true"),
    ]);
    let (backend, master) = harness.start_set_up().await;
    let page = |path: &'static str| {
        backend.send(Method::GET, path, None, None)
    };
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
    assert_eq!(missing_asset.refusal(), (StatusCode::NOT_FOUND, ErrorCode::NotFound));
    let not_a_page = page("/conversation/example").await;
    assert_eq!(not_a_page.refusal(), (StatusCode::NOT_FOUND, ErrorCode::NotFound));
    let api_miss = backend
        .send(Method::GET, "/api/no-such-resource", Some(&master.cookie), None)
        .await;
    assert_eq!(api_miss.refusal(), (StatusCode::NOT_FOUND, ErrorCode::NotFound));
    backend.close().await;
}
