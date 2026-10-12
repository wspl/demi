//! A plugin's outbound requests (`plugins.md` § Outbound requests): only
//! `https`, through the plugins' client, within the deadline and the body
//! limit the plugin names, and ended by the request's cancellation.

use std::sync::Arc;

use axum::Router;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::get;
use demi_backend_plugins::Registry;
use demi_backend_plugins::testing::Authority;
use demi_plugin_interface::{PortRefusal, WebFailure};
use demi_provider_common::testing::MockVendor;
use serde_json::json;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::support::{Backend, PortProbe, call, refusal, within};

/// A vendor over HTTPS: `/echo` answers the key the request sent, `/large`
/// 2048 bytes with their length and `/stream` without it, `/slow` never,
/// and `/away` redirects to `away`.
fn vendor(away: String, slow: Arc<Notify>) -> Router {
    Router::new()
        .route(
            "/echo",
            get(|headers: HeaderMap| async move {
                let key = headers["x-api-key"].to_str().unwrap().to_owned();
                format!("hello {key}")
            }),
        )
        .route("/large", get(|| async { vec![b'x'; 2048] }))
        .route(
            "/stream",
            get(|| async {
                let chunks = (0..4).map(|_| Ok::<_, std::io::Error>(vec![b'x'; 512]));
                axum::body::Body::from_stream(futures_util::stream::iter(chunks))
            }),
        )
        .route(
            "/slow",
            get(move || async move {
                slow.notify_waiters();
                std::future::pending::<()>().await;
            }),
        )
        .route(
            "/away",
            get(move || async move { (StatusCode::FOUND, [(header::LOCATION, away)]).into_response() }),
        )
}

fn failure(answer: Result<serde_json::Value, demi_backend_plugins::PageCallError>) -> WebFailure {
    match refusal(answer) {
        PortRefusal::Web { failure } => failure,
        refusal => panic!("expected a failed request, got {refusal:?}"),
    }
}

// About half a second: one request waits for its 300 ms deadline.
#[tokio::test(flavor = "local")]
async fn a_request_is_sent_only_over_https_within_its_deadline_and_its_body_limit() {
    within(async {
        let authority = Authority::new();
        let plain = MockVendor::start().await;
        let slow = Arc::new(Notify::new());
        let (vendor, _server) = authority
            .serve(vendor(plain.url("/ok"), slow.clone()))
            .await;
        let registry = Registry::new(vec![PortProbe::new("probe", None).boxed()], |_| true).unwrap();
        let backend = Backend::start(registry, vec![authority.root()]).await;
        let alice = backend.user("alice").await;
        let plugins = backend.plugins(&alice);
        let send = |path: &str, deadline_ms: u64, limit_bytes: u64| {
            let url = if path.starts_with("http") {
                path.to_owned()
            } else {
                vendor.join(path).unwrap().to_string()
            };
            call(
                &plugins,
                "probe",
                "send",
                json!({ "url": url, "deadlineMs": deadline_ms, "limitBytes": limit_bytes, "key": "sk-1" }),
            )
        };

        assert_eq!(
            send("/echo", 10_000, 1024).await.unwrap(),
            json!({ "status": 200, "body": "hello sk-1" })
        );

        // Never over plain HTTP, not even through a redirect.
        assert_eq!(failure(send(&plain.url("/ok"), 10_000, 1024).await), WebFailure::NotHttps);
        assert_eq!(failure(send("/away", 10_000, 1024).await), WebFailure::NotHttps);
        assert!(plain.requests().is_empty());

        // A body over the limit, announced or not.
        assert_eq!(failure(send("/large", 10_000, 2047).await), WebFailure::TooLarge);
        assert_eq!(failure(send("/stream", 10_000, 2047).await), WebFailure::TooLarge);
        assert_eq!(
            send("/large", 10_000, 2048).await.unwrap()["status"],
            json!(200)
        );

        // The deadline.
        assert_eq!(failure(send("/slow", 300, 1024).await), WebFailure::TimedOut);

        // A deadline over a minute, or a limit over 16 MiB, is the plugin's
        // mistake.
        assert!(matches!(
            failure(send("/echo", 60_001, 1024).await),
            WebFailure::Invalid { .. }
        ));
        assert!(matches!(
            failure(send("/echo", 10_000, 16 * 1024 * 1024 + 1).await),
            WebFailure::Invalid { .. }
        ));

        // The request's cancellation ends it.
        let cancel = CancellationToken::new();
        let call = demi_backend_plugins::PageCall {
            plugin: "probe".into(),
            method: "send".into(),
            params: json!({
                "url": vendor.join("/slow").unwrap().to_string(),
                "deadlineMs": 60_000,
                "limitBytes": 1024,
            }),
            conversation: None,
        };
        let sent = plugins.page_call(call, cancel.clone());
        let cancelled = async {
            slow.notified().await;
            cancel.cancel();
        };
        let (answer, ()) = tokio::join!(sent, cancelled);
        assert!(matches!(failure(answer), WebFailure::Unreachable { .. }));
    })
    .await;
}
