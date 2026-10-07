//! The deployment's namespace at the preview domain (`preview.md` § The
//! preview domain service), against a scripted preview domain service: the
//! first start registers one with the origins the web app is served on,
//! keeps it and tells the pages; a day later it is renewed; changed origins
//! replace the registered ones; one that expired is replaced by a new one;
//! and a service that does not answer holds nothing up.

use std::time::Duration;

use demi_backend_user_shard::preview::PreviewSettings;
use demi_backend_user_shard::tuning::PreviewTuning;
use demi_provider_common::testing::{MockResponse, MockVendor};
use demi_shared_types::Timestamp;
use demi_web_api_protocol::state::{PreviewDomain, SyncEvent};
use jiff::SignedDuration;
use reqwest::Method;
use serde_json::{Value, json};

use crate::support::{Harness, MASTER_EMAIL, MASTER_PASSWORD, SyncChannel, eventually};

/// The URL the test backends serve on, whose origin the namespace admits.
const PUBLIC_URL: &str = "http://127.0.0.1:3271";
/// A development server's origin, which the namespace admits besides.
const WEB_DEV: &str = "http://127.0.0.1:18934";
const FIRST: &str = "jdoj0t2f";
const SECOND: &str = "8qcm009j";
/// The harness clock's start, plus the 90 days a namespace lives.
const EXPIRES: &str = "2026-12-23T08:00:00.000Z";

/// A harness whose backends keep their namespace at `service`, served on
/// [`PUBLIC_URL`] and the development server's `origins`, and try again
/// soon after a failure.
fn harness(service: &MockVendor, origins: &[&str]) -> Harness {
    let mut harness = Harness::new();
    harness.public_url = Some(PUBLIC_URL.parse().unwrap());
    let port = url::Url::parse(&service.url("/")).unwrap().port().unwrap();
    harness.preview = Some(PreviewSettings {
        domain: format!("demi-preview.localhost:{port}").parse().unwrap(),
        origins: origins.iter().map(|origin| origin.parse().unwrap()).collect(),
        tuning: PreviewTuning {
            check: Duration::from_millis(20),
            first_retry: Duration::from_millis(20),
            last_retry: Duration::from_millis(20),
        },
    });
    harness
}

fn answer(status: u16, body: Value) -> MockResponse {
    MockResponse::status(status)
        .header("content-type", "application/json")
        .chunk(body.to_string())
}

/// The service's answer to a creation at the harness clock's start.
fn created(namespace: &str, secret: &str) -> MockResponse {
    created_expiring(namespace, secret, EXPIRES)
}

fn created_expiring(namespace: &str, secret: &str, expires_at: &str) -> MockResponse {
    answer(
        201,
        json!({ "namespace": namespace, "secret": secret, "expiresAt": expires_at }),
    )
}

/// The service's answer to a renewal or a change of origins.
fn changed(namespace: &str, origins: &[&str], expires_at: &str) -> MockResponse {
    answer(
        200,
        json!({ "namespace": namespace, "origins": origins, "expiresAt": expires_at }),
    )
}

/// The preview the page is told: the snapshot's, or, while the backend
/// registers, the one the next `preview` message carries.
async fn preview_of(channel: &mut SyncChannel) -> PreviewDomain {
    if let Some(preview) = channel.snapshot().await.preview {
        return preview;
    }
    let events = channel
        .until(|event| matches!(event, SyncEvent::Preview { .. }))
        .await;
    match events.last() {
        Some(SyncEvent::Preview { preview }) => preview.clone(),
        other => panic!("not a preview message: {other:?}"),
    }
}

/// The namespace, expiry and origins the control database keeps, and
/// whether its secret column holds `secret` as it is.
fn record(harness: &Harness) -> (String, Timestamp, Value, bool) {
    harness
        .control_database()
        .query_row(
            "SELECT namespace, expires_at, origins, secret FROM preview_namespace",
            [],
            |row| {
                let origins: String = row.get(2)?;
                let secret: Vec<u8> = row.get(3)?;
                Ok((
                    row.get(0)?,
                    Timestamp::from_millisecond(row.get(1)?).unwrap(),
                    serde_json::from_str(&origins).unwrap(),
                    secret.windows(8).any(|window| window == b"secret-1"),
                ))
            },
        )
        .unwrap()
}

/// The first start registers a namespace with the public URL's origin and
/// the development server's, keeps it with its secret sealed, and the page
/// is told it; the next start keeps it and asks the service nothing.
#[tokio::test]
async fn the_first_start_registers_a_namespace_the_page_is_told_and_later_starts_keep_it() {
    let service = MockVendor::start().await;
    service.respond_at("/api/v1/namespaces", created(FIRST, "secret-1"));
    let harness = harness(&service, &[WEB_DEV]);
    let (backend, master) = harness.start_set_up().await;

    let mut channel = backend.sync(&master).await;
    let preview = preview_of(&mut channel).await;
    let port = url::Url::parse(&service.url("/")).unwrap().port().unwrap();
    assert_eq!(
        (preview.domain, preview.namespace.as_str()),
        (format!("demi-preview.localhost:{port}"), FIRST)
    );
    let requests = service.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, Method::POST);
    assert_eq!(
        requests[0].header("host"),
        Some(format!("demi-preview.localhost:{port}").as_str())
    );
    assert_eq!(
        requests[0].json(),
        json!({ "origins": [PUBLIC_URL, WEB_DEV] })
    );
    assert_eq!(
        record(&harness),
        (
            FIRST.to_owned(),
            EXPIRES.parse().unwrap(),
            json!([PUBLIC_URL, WEB_DEV]),
            false
        )
    );
    backend.close().await;

    let backend = harness.start().await;
    let master = backend.login(MASTER_EMAIL, MASTER_PASSWORD).await;
    let mut channel = backend.sync(&master).await;
    assert_eq!(preview_of(&mut channel).await.namespace.as_str(), FIRST);
    assert_eq!(service.requests().len(), 1, "a kept namespace is not registered again");
    backend.close().await;
}

/// A day after its registration the namespace is renewed with its secret,
/// and the record keeps the expiry the service answers.
#[tokio::test]
async fn the_namespace_is_renewed_a_day_later() {
    let service = MockVendor::start().await;
    service.respond_at("/api/v1/namespaces", created(FIRST, "secret-1"));
    let harness = harness(&service, &[]);
    let (backend, master) = harness.start_set_up().await;
    let mut channel = backend.sync(&master).await;
    preview_of(&mut channel).await;

    let renewal = format!("/api/v1/namespaces/{FIRST}/renew");
    let renewed = "2026-12-24T08:00:00.000Z";
    service.respond_at(&renewal, changed(FIRST, &[PUBLIC_URL], renewed));
    harness.clock.advance(SignedDuration::from_hours(24));
    service.received(2).await;
    let request = &service.requests()[1];
    assert_eq!(
        (&request.method, request.uri.path()),
        (&Method::POST, renewal.as_str())
    );
    assert_eq!(request.header("authorization"), Some("Bearer secret-1"));
    eventually("the record keeps the renewed expiry", || async {
        record(&harness).1 == renewed.parse().unwrap()
    })
    .await;
    backend.close().await;
}

/// A start whose origins differ from those registered replaces them, and
/// keeps the namespace.
#[tokio::test]
async fn changed_origins_replace_the_registered_ones() {
    let service = MockVendor::start().await;
    service.respond_at("/api/v1/namespaces", created(FIRST, "secret-1"));
    let mut harness = harness(&service, &[]);
    let (backend, master) = harness.start_set_up().await;
    let mut channel = backend.sync(&master).await;
    preview_of(&mut channel).await;
    backend.close().await;

    let origins = format!("/api/v1/namespaces/{FIRST}/origins");
    service.respond_at(&origins, changed(FIRST, &[PUBLIC_URL, WEB_DEV], EXPIRES));
    harness
        .preview
        .as_mut()
        .unwrap()
        .origins
        .push(WEB_DEV.parse().unwrap());
    let backend = harness.start().await;
    service.received(2).await;
    let request = &service.requests()[1];
    assert_eq!(
        (&request.method, request.uri.path()),
        (&Method::PUT, origins.as_str())
    );
    assert_eq!(request.header("authorization"), Some("Bearer secret-1"));
    assert_eq!(request.json(), json!({ "origins": [PUBLIC_URL, WEB_DEV] }));
    eventually("the record keeps the new origins", || async {
        record(&harness).2 == json!([PUBLIC_URL, WEB_DEV])
    })
    .await;
    assert_eq!(record(&harness).0, FIRST);
    backend.close().await;
}

/// A namespace the service says expired is replaced by a new one, which the
/// open pages are told.
#[tokio::test]
async fn an_expired_namespace_is_replaced_by_a_new_one() {
    let service = MockVendor::start().await;
    service.respond_at("/api/v1/namespaces", created(FIRST, "secret-1"));
    let harness = harness(&service, &[]);
    let (backend, master) = harness.start_set_up().await;
    let mut channel = backend.sync(&master).await;
    preview_of(&mut channel).await;

    service.respond_at(
        &format!("/api/v1/namespaces/{FIRST}/renew"),
        MockResponse::status(410),
    );
    // Created a day after the first, it lives until a day after it.
    service.respond_at(
        "/api/v1/namespaces",
        created_expiring(SECOND, "secret-2", "2026-12-24T08:00:00.000Z"),
    );
    harness.clock.advance(SignedDuration::from_hours(24));
    let events = channel
        .until(|event| matches!(event, SyncEvent::Preview { .. }))
        .await;
    let Some(SyncEvent::Preview { preview }) = events.last() else {
        unreachable!("until ends with the message it waits for");
    };
    assert_eq!(preview.namespace.as_str(), SECOND);
    assert_eq!(record(&harness).0, SECOND);
    let paths: Vec<String> = service
        .requests()
        .iter()
        .map(|request| request.uri.path().to_owned())
        .collect();
    assert_eq!(
        paths,
        [
            "/api/v1/namespaces".to_owned(),
            format!("/api/v1/namespaces/{FIRST}/renew"),
            "/api/v1/namespaces".to_owned(),
        ]
    );
    backend.close().await;
}

/// While the service fails, the backend serves and the page is told no
/// namespace; once it answers, the namespace is registered and the open page
/// is told.
#[tokio::test]
async fn a_failing_service_holds_nothing_up_and_the_namespace_comes_later() {
    // Unscripted, the service answers every request with a 500.
    let service = MockVendor::start().await;
    let harness = harness(&service, &[]);
    let (backend, master) = harness.start_set_up().await;
    let mut channel = backend.sync(&master).await;
    assert_eq!(channel.snapshot().await.preview, None);
    service.received(2).await;

    service.respond_at("/api/v1/namespaces", created(FIRST, "secret-1"));
    let events = channel
        .until(|event| matches!(event, SyncEvent::Preview { .. }))
        .await;
    let Some(SyncEvent::Preview { preview }) = events.last() else {
        unreachable!("until ends with the message it waits for");
    };
    assert_eq!(preview.namespace.as_str(), FIRST);
    backend.close().await;
}
