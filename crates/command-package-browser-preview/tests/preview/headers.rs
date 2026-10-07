//! What the engine changes in an answer and in the request upstream sees
//! (`preview.md` § CORS, CORP and Referer, § Response headers).

use demi_command_package_browser_protocol::preview::{PreviewCredentials, PreviewHeader, PreviewMode, PreviewRequest, PreviewScheme};

use crate::support::{Relay, Site, echoed_header, embedded, encoded, engine, opening, preview_origin, request, top};

/// `path` of `host` answering with `headers`.
fn answering(site: &Site, host: &str, path: &str, headers: &[&str]) -> String {
    let separator = if path.contains('?') { '&' } else { '?' };
    let query: Vec<String> = headers.iter().map(|header| format!("h={}", encoded(header))).collect();
    site.https(host, &format!("{path}{separator}{}", query.join("&")))
}

#[tokio::test]
async fn a_sites_content_policies_give_way_to_the_previews() {
    let directory = tempfile::tempdir().unwrap();
    let site = Site::start().await;
    let mut relay = Relay::open(engine(&directory));
    let page = answering(
        &site,
        "www.site.test",
        &format!("/content?type=text/html&body={}", encoded("<p>hi</p>")),
        &[
            "Content-Security-Policy:script-src 'none'",
            "Content-Security-Policy-Report-Only:default-src 'none'",
            "X-Frame-Options:DENY",
        ],
    );
    let opened = relay.fetch(top(&site.origin("www.site.test")), opening(&page)).await.unwrap();
    let policies: Vec<&str> = opened
        .headers
        .iter()
        .filter(|header| header.name.starts_with("content-security-policy") || header.name == "x-frame-options")
        .map(|header| header.value.as_str())
        .collect();
    assert_eq!(policies, ["default-src https://*.preview.test data: blob: 'unsafe-inline' 'unsafe-eval'"]);
}

#[tokio::test]
async fn a_preview_domain_served_over_http_is_allowed_over_http_with_its_port() {
    let directory = tempfile::tempdir().unwrap();
    let site = Site::start().await;
    let mut relay = Relay::open_on(engine(&directory), PreviewScheme::Http, "demi-preview.localhost:5174");
    let other = site.origin("other.test");
    let page = format!("/content?type=text/html&body={}", encoded(&format!(r#"<img src="{other}/i.png">"#)));
    let www = top(&site.origin("www.site.test"));
    let opened = relay.fetch(www.clone(), opening(&site.https("www.site.test", &page))).await.unwrap();
    assert_eq!(
        opened.header("content-security-policy"),
        Some("default-src http://*.demi-preview.localhost:5174 http://*.demi-preview.localhost data: blob: 'unsafe-inline' 'unsafe-eval'")
    );
    let image = embedded(&other, &www.top, true);
    let origin = preview_origin(&image).replace("https://", "http://").replace(".preview.test", ".demi-preview.localhost:5174");
    assert!(opened.text().contains(&format!(r#"src="{origin}/i.png""#)), "{}", opened.text());
}

#[tokio::test]
async fn a_permissions_policy_reaches_the_browser_without_sync_xhr() {
    let directory = tempfile::tempdir().unwrap();
    let site = Site::start().await;
    let mut relay = Relay::open(engine(&directory));
    let www = top(&site.origin("www.site.test"));
    let both = answering(
        &site,
        "www.site.test",
        "/content",
        &["Permissions-Policy:sync-xhr=(), camera=()", "Feature-Policy:sync-xhr 'none'; geolocation 'none'"],
    );
    let opened = relay.fetch(www.clone(), opening(&both)).await.unwrap();
    assert_eq!(opened.header("permissions-policy"), Some("camera=()"));
    assert_eq!(opened.header("feature-policy"), Some("geolocation 'none'"));
    let only = answering(&site, "www.site.test", "/content", &["Permissions-Policy:sync-xhr=()"]);
    let opened = relay.fetch(www, opening(&only)).await.unwrap();
    assert_eq!(opened.header("permissions-policy"), None);
}

#[tokio::test]
async fn cors_is_decided_by_logical_origin_and_answered_for_the_label() {
    let directory = tempfile::tempdir().unwrap();
    let site = Site::start().await;
    let mut relay = Relay::open(engine(&directory));
    let page = top(&site.origin("other.test"));
    let cors = |url: String, credentials: PreviewCredentials| PreviewRequest {
        credentials,
        ..request(&url, PreviewMode::Cors, "", Some(page.clone()))
    };
    let allowed = answering(
        &site,
        "www.site.test",
        "/echo",
        &[&format!("Access-Control-Allow-Origin:{}", page.origin), "Access-Control-Expose-Headers:x-total"],
    );
    let fetched = relay.fetch(page.clone(), cors(allowed, PreviewCredentials::Omit)).await.unwrap();
    // The browser checks the label's origin, which the engine answers for.
    let label_origin = preview_origin(&page);
    assert_eq!(fetched.header("access-control-allow-origin"), Some(label_origin.as_str()));
    let label = label_origin.strip_prefix("https://k3f9a2ab--").unwrap().strip_suffix(".preview.test").unwrap();
    assert_eq!(fetched.labels.get(label), Some(&page));
    assert_eq!(fetched.header("access-control-expose-headers"), Some("x-total, x-demi-cookie-changed"));
    assert_eq!(echoed_header(&fetched.echoed(), "origin"), Some(page.origin.as_str()));
    // Another origin's permission, and a wildcard for a credentialed request,
    // are refusals.
    let elsewhere = answering(&site, "www.site.test", "/echo", &["Access-Control-Allow-Origin:https://elsewhere.test"]);
    let refused = relay.fetch(page.clone(), cors(elsewhere, PreviewCredentials::Omit)).await;
    assert_eq!(refused.unwrap_err(), "refused: cors");
    let wildcard = answering(&site, "www.site.test", "/echo", &["Access-Control-Allow-Origin:*"]);
    relay.fetch(page.clone(), cors(wildcard.clone(), PreviewCredentials::Omit)).await.unwrap();
    let refused = relay.fetch(page.clone(), cors(wildcard, PreviewCredentials::Include)).await;
    assert_eq!(refused.unwrap_err(), "refused: cors");
    // A request that is not simple asks first, and goes only if allowed.
    let put = |url: String| PreviewRequest {
        method: "PUT".into(),
        headers: vec![PreviewHeader {
            name: "x-custom".into(),
            value: "1".into(),
        }],
        ..cors(url, PreviewCredentials::Omit)
    };
    let unasked = answering(&site, "www.site.test", "/unasked", &[&format!("Access-Control-Allow-Origin:{}", page.origin)]);
    let refused = relay.fetch_with_body(page.clone(), put(unasked), b"x").await;
    assert_eq!(refused.unwrap_err(), "refused: cors-preflight");
    let asked = answering(
        &site,
        "www.site.test",
        "/asked",
        &[
            &format!("Access-Control-Allow-Origin:{}", page.origin),
            "Access-Control-Allow-Methods:PUT",
            "Access-Control-Allow-Headers:x-custom",
        ],
    );
    relay.fetch_with_body(page.clone(), put(asked), b"x").await.unwrap();
    let methods: Vec<(String, String)> = site
        .received()
        .into_iter()
        .filter(|received| received.path == "/unasked" || received.path == "/asked")
        .map(|received| (received.method, received.path))
        .collect();
    assert_eq!(
        methods,
        [
            ("OPTIONS".into(), "/unasked".into()),
            ("OPTIONS".into(), "/asked".into()),
            ("PUT".into(), "/asked".into())
        ]
    );
}

#[tokio::test]
async fn corp_is_judged_by_logical_origin() {
    let directory = tempfile::tempdir().unwrap();
    let site = Site::start().await;
    let mut relay = Relay::open(engine(&directory));
    let image = |page: &demi_command_package_browser_protocol::preview::PreviewEnvironment, policy: &str| {
        let url = answering(&site, "www.site.test", "/echo", &[&format!("Cross-Origin-Resource-Policy:{policy}")]);
        request(&url, PreviewMode::NoCors, "image", Some(page.clone()))
    };
    let other = top(&site.origin("other.test"));
    let refused = relay.fetch(other.clone(), image(&other, "same-site")).await;
    assert_eq!(refused.unwrap_err(), "refused: corp");
    let sibling = top(&site.origin("api.site.test"));
    relay.fetch(sibling.clone(), image(&sibling, "same-site")).await.unwrap();
    let refused = relay.fetch(sibling.clone(), image(&sibling, "same-origin")).await;
    assert_eq!(refused.unwrap_err(), "refused: corp");
    // Every answer may be embedded by the label's origin, which the browser
    // judges.
    let allowed = relay.fetch(other.clone(), image(&other, "cross-origin")).await.unwrap();
    assert_eq!(allowed.header("cross-origin-resource-policy"), Some("cross-origin"));
}

#[tokio::test]
async fn the_referer_upstream_is_the_real_one_under_the_pages_policy() {
    let directory = tempfile::tempdir().unwrap();
    let site = Site::start().await;
    let mut relay = Relay::open(engine(&directory));
    let page = top(&site.origin("www.site.test"));
    let from_page = site.https("www.site.test", "/page?q=1");
    let referred = |url: String, policy: &str| PreviewRequest {
        referrer: from_page.clone(),
        referrer_policy: policy.into(),
        ..request(&url, PreviewMode::NoCors, "image", Some(page.clone()))
    };
    let referer = |fetched: crate::support::Fetched| echoed_header(&fetched.echoed(), "referer").map(ToOwned::to_owned);
    let cross = relay.fetch(page.clone(), referred(site.https("other.test", "/echo"), "")).await.unwrap();
    assert_eq!(echoed_header(&cross.echoed(), "sec-fetch-site"), Some("cross-site"));
    assert_eq!(referer(cross), Some(format!("{}/", page.origin)));
    let own = relay.fetch(page.clone(), referred(site.https("www.site.test", "/echo"), "")).await.unwrap();
    assert_eq!(referer(own), Some(from_page.clone()));
    let none = relay
        .fetch(page.clone(), referred(site.https("www.site.test", "/echo"), "no-referrer"))
        .await
        .unwrap();
    assert_eq!(referer(none), None);
    // The site's own no-referrer reaches the browser as same-origin, so the
    // origin's navigations still name their initiator.
    let strict = answering(&site, "www.site.test", "/content", &["Referrer-Policy:no-referrer"]);
    let opened = relay.fetch(page.clone(), opening(&strict)).await.unwrap();
    assert_eq!(opened.header("referrer-policy"), Some("same-origin"));
    // A frame's navigation of another site carries the origin only.
    let frame = embedded(&site.origin("other.test"), &page.top, true);
    let navigation = PreviewRequest {
        referrer: from_page,
        ..request(&site.https("other.test", "/echo"), PreviewMode::Navigate, "iframe", Some(page.clone()))
    };
    let navigated = relay.fetch(frame, navigation).await.unwrap();
    assert_eq!(referer(navigated), Some(format!("{}/", page.origin)));
}

/// A tab of the user's browser in Mobile describes an Android Chrome
/// (`preview.md` § Mobile): its user agent and client hints reach the site,
/// and a document's boot data carries the device its runtime shows the
/// page's scripts.
#[tokio::test]
async fn an_android_client_is_the_phone_its_requests_and_documents_describe() {
    let directory = tempfile::tempdir().unwrap();
    let site = Site::start().await;
    let mut relay = Relay::open(engine(&directory));
    let android = "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/154.0.0.0 Mobile Safari/537.36";
    relay.client = demi_command_package_browser_protocol::preview::PreviewClient {
        user_agent: android.into(),
        mobile: true,
        platform: "Android".into(),
        ..relay.client.clone()
    };
    let page = top(&site.origin("www.site.test"));
    let echoed = relay.fetch(page.clone(), opening(&site.https("www.site.test", "/echo"))).await.unwrap().echoed();
    assert_eq!(echoed_header(&echoed, "user-agent"), Some(android));
    assert_eq!(echoed_header(&echoed, "sec-ch-ua-mobile"), Some("?1"));
    assert_eq!(echoed_header(&echoed, "sec-ch-ua-platform"), Some("\"Android\""));
    let document = relay
        .fetch(page, opening(&site.https("www.site.test", &format!("/content?type=text/html&body={}", encoded("<p>phone</p>")))))
        .await
        .unwrap();
    let boot = document.text();
    let device = &boot[boot.find("\"device\":").expect("the boot data's device")..];
    let device: serde_json::Value = serde_json::Deserializer::from_str(&device["\"device\":".len()..])
        .into_iter()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(device, serde_json::json!({ "userAgent": android, "mobile": true, "platform": "Android" }));
}
