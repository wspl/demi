//! Request bodies and the web app build.

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

#[tokio::test]
async fn a_json_body_over_its_limit_is_refused_before_it_is_read() {
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;

    // With its length declared up front: the head alone, whose body the
    // backend never reads (`TestBackend::declaring`).
    let json = [("content-type", "application/json")];
    let declared = backend
        .declaring(
            Method::PATCH,
            "/api/auth/me",
            &[json[0], ("cookie", &master.cookie)],
            JSON_BODY_LIMIT + 1,
        )
        .await;
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
    let unsigned = backend
        .declaring(Method::PATCH, "/api/auth/me", &json, JSON_BODY_LIMIT + 1)
        .await;
    assert_eq!(
        unsigned.refusal(),
        (StatusCode::UNAUTHORIZED, ErrorCode::Unauthenticated)
    );
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

/// What the backend answers a request for `path` that sends `host` as its
/// `Host`, or none: written by hand, since an HTTP client always sends the
/// one its URL names. The answer's status line and headers, lowercased.
async fn raw_head(url: &str, path: &str, host: Option<&str>) -> String {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let address = url.trim_start_matches("http://");
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let host = host.map(|host| format!("Host: {host}\r\n")).unwrap_or_default();
    let request = format!("GET {path} HTTP/1.0\r\n{host}Accept: text/html\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut answer = Vec::new();
    stream.read_to_end(&mut answer).await.unwrap();
    let answer = String::from_utf8_lossy(&answer).into_owned();
    answer.split("\r\n\r\n").next().unwrap().to_lowercase()
}

#[tokio::test]
async fn a_shared_link_answers_its_own_entry_under_a_policy_that_reaches_only_its_share() {
    // Cost: one backend start on a temporary directory; under a second.
    let mut harness = Harness::new().with_web(&[
        ("index.html", "<html>product page</html>"),
        ("share.html", "<html>shared page</html>"),
        ("build.json", r#"{ "build": "b7f3" }"#),
    ]);
    harness.public_url = Some("https://demi.example.test/".parse().unwrap());
    let (backend, _master) = harness.start_set_up().await;
    let routes = "https://demi.example.test/api/shares/Qm9iZXhhbXBsZTEyMzQ https://demi.example.test/api/shares/Qm9iZXhhbXBsZTEyMzQ/";
    let expected = [
        "default-src 'none'".to_owned(),
        "script-src 'self'".to_owned(),
        "font-src 'self' data:".to_owned(),
        format!("connect-src {routes}"),
        format!("img-src {routes} blob: data:"),
        format!("media-src {routes} blob:"),
        "frame-ancestors 'none'".to_owned(),
    ];

    // Any token gets the page, which asks the share's routes whether it
    // exists; the policy names the backend's public URL whatever `Host`
    // the request sends, a name a proxy passes on, one it rejects, or none.
    for host in [Some("demi.example.test"), Some("bad_host.example"), None] {
        let head = raw_head(&backend.url, "/s/Qm9iZXhhbXBsZTEyMzQ", host).await;
        assert!(head.starts_with("http/1.0 200") || head.starts_with("http/1.1 200"), "{host:?}: {head}");
        let policy = head
            .lines()
            .find_map(|line| line.strip_prefix("content-security-policy: "))
            .unwrap_or_else(|| panic!("{host:?}: no policy in {head}"));
        for directive in &expected {
            assert!(
                policy.split("; ").any(|part| part == directive.to_lowercase()),
                "{host:?}: {directive:?} in {policy:?}"
            );
        }
        assert!(!policy.contains("connect-src 'self'"), "{host:?}: {policy:?}");
        assert!(head.contains("referrer-policy: no-referrer"), "{host:?}: {head}");
        assert!(head.contains("x-robots-tag: noindex, nofollow"), "{host:?}: {head}");
    }

    // An address cannot write the policy: what the token holds stays in its path.
    let forged = raw_head(&backend.url, "/s/x%3B%20connect-src%20*", None).await;
    assert!(
        forged.contains("connect-src https://demi.example.test/api/shares/x%3b%20connect%2dsrc%20%2a "),
        "{forged}"
    );
    assert!(!forged.contains("connect-src *"), "{forged}");

    // The product's pages stay the product's entry.
    let product = raw_head(&backend.url, "/chat/c-1", None).await;
    assert!(!product.contains("content-security-policy"), "{product}");
    backend.close().await;
}

// A browser reuses what a GET answered only after the server says it has not
// changed: each read carries a tag and `no-cache`, the same read again with
// the tag is a 304 without a body that keeps the answer's own headers, a
// changed answer is sent whole, and what carries a secret or a session's own
// state is never kept; signing out has the browser drop what it kept.
#[tokio::test]
async fn a_read_is_reused_only_once_the_server_says_it_has_not_changed() {
    // Cost: one backend start; under a second.
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;

    let first = backend.get("/api/settings/preferences", Some(&master)).await;
    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(first.headers["cache-control"], "private, no-cache");
    let tag = first.headers["etag"].to_str().unwrap().to_owned();

    let again = backend.get_with("/api/settings/preferences", &master, &[("if-none-match", &tag)]).await;
    assert_eq!(again.status, StatusCode::NOT_MODIFIED);
    assert!(again.body.is_empty());
    assert_eq!(again.headers["etag"], first.headers["etag"]);
    assert_eq!(again.headers["cache-control"], "private, no-cache");

    // A change is sent whole, under a new tag.
    let saved = backend
        .patch("/api/settings/preferences", &master, json!({ "resumeAfterRestart": false }))
        .await;
    assert_eq!(saved.status, StatusCode::OK, "{}", String::from_utf8_lossy(&saved.body));
    let changed = backend.get_with("/api/settings/preferences", &master, &[("if-none-match", &tag)]).await;
    assert_eq!(changed.status, StatusCode::OK);
    assert_ne!(changed.headers["etag"], first.headers["etag"]);
    assert_ne!(changed.body, first.body);

    // The session's own state and the secrets are not kept at all.
    for path in ["/api/auth/me", "/api/credentials", "/api/auth/sessions"] {
        let read = backend.get(path, Some(&master)).await;
        assert_eq!(read.status, StatusCode::OK, "{path}");
        assert_eq!(read.headers["cache-control"], "private, no-store", "{path}");
        assert!(!read.headers.contains_key("etag"), "{path}");
    }
    // A sign-in code is a secret, whether or not the sign-in exists.
    let sign_in = backend.get("/api/providers/subscription-login/none", Some(&master)).await;
    assert_eq!(sign_in.headers["cache-control"], "private, no-store");
    assert!(!sign_in.headers.contains_key("etag"));

    // Signing out has the browser drop what it kept.
    let out = backend.post("/api/auth/logout", Some(&master), json!({})).await;
    assert_eq!(out.status, StatusCode::NO_CONTENT);
    assert_eq!(out.headers["clear-site-data"], "\"cache\"");
    backend.close().await;
}
