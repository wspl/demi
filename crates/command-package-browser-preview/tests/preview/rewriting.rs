//! What the engine rewrites, and the labels it lists for the relay
//! (`preview.md` § Rewriting, § Addresses and labels).

use base64::Engine as _;
use demi_command_package_browser_protocol::preview::{PreviewEnvironment, PreviewMode};
use sha2::Digest as _;

use crate::support::{CHROME_154, Fetched, Relay, Site, embedded, encoded, engine, opening, preview_origin, request, top};

/// The label of a preview origin.
fn label(origin: &str) -> &str {
    origin.strip_prefix("https://k3f9a2ab--").unwrap().strip_suffix(".preview.test").unwrap()
}

/// That the answer maps addresses to `environment`'s label, and lists it.
fn lists(fetched: &Fetched, environment: &PreviewEnvironment) -> String {
    let origin = preview_origin(environment);
    assert_eq!(fetched.labels.get(label(&origin)), Some(environment), "{:?} {}", fetched.labels, fetched.text());
    origin
}

fn content(site: &Site, host: &str, kind: &str, body: &str) -> String {
    site.https(host, &format!("/content?type={}&body={}", encoded(kind), encoded(body)))
}

#[tokio::test]
async fn a_documents_addresses_map_to_the_labels_its_answer_lists() {
    let directory = tempfile::tempdir().unwrap();
    let site = Site::start().await;
    let mut relay = Relay::open(engine(&directory));
    let other = site.origin("other.test");
    let html = format!(r#"<a href="{other}/next">next</a><img src="{other}/i.png"><img src="/own.png">"#);
    let www = top(&site.origin("www.site.test"));
    let opened = relay
        .fetch(www.clone(), opening(&content(&site, "www.site.test", "text/html", &html)))
        .await
        .unwrap();
    let body = opened.text();
    // A link navigates the top: another site's own top-level environment,
    // through its boot page. An image is a subresource of this document.
    let linked = lists(&opened, &top(&other));
    assert!(body.contains(&format!(r#"href="{linked}/__demi/v1/boot.html#to=/next""#)), "{body}");
    let embedded_image = lists(&opened, &embedded(&other, &www.top, true));
    assert!(body.contains(&format!(r#"src="{embedded_image}/i.png""#)), "{body}");
    assert!(body.contains(r#"src="/own.png""#), "{body}");
    // The document starts with the client script and the runtime of the
    // engine's release, and its boot data says what device it runs on.
    assert!(body.contains("/__demi/v1/client.js"), "{body}");
    assert!(body.contains(&format!("/__demi/page/runtime/{}.js", demi_shared_artifacts::WORKSPACE_VERSION)), "{body}");
    assert!(body.contains(CHROME_154), "{body}");
    assert_eq!(opened.header("content-type"), Some("text/html; charset=utf-8"));
    // A redirect to another site goes through its boot page too.
    let moved = site.https(
        "www.site.test",
        &format!("/content?status=302&h={}", encoded(&format!("Location:{other}/landed"))),
    );
    let redirected = relay.fetch(www, opening(&moved)).await.unwrap();
    assert_eq!(redirected.status, 302);
    assert_eq!(redirected.header("location"), Some(format!("{linked}/__demi/v1/boot.html#to=/landed").as_str()));
}

#[tokio::test]
async fn stylesheets_and_scripts_map_their_addresses() {
    let directory = tempfile::tempdir().unwrap();
    let site = Site::start().await;
    let mut relay = Relay::open(engine(&directory));
    let other = site.origin("other.test");
    let www = top(&site.origin("www.site.test"));
    let cross = embedded(&other, &www.top, true);
    let css = content(&site, "www.site.test", "text/css", &format!("body{{background:url({other}/bg.png)}}"));
    let sheet = relay
        .fetch(www.clone(), request(&css, PreviewMode::NoCors, "style", Some(www.clone())))
        .await
        .unwrap();
    let mapped = lists(&sheet, &cross);
    assert_eq!(sheet.text(), format!("body{{background:url({mapped}/bg.png)}}"));
    let source = format!("import \"{other}/m.js\";\nlocation.href;\n");
    let js = content(&site, "www.site.test", "text/javascript", &source);
    let script = relay
        .fetch(www.clone(), request(&js, PreviewMode::NoCors, "script", Some(www.clone())))
        .await
        .unwrap();
    let module = lists(&script, &cross);
    assert!(script.text().contains(&format!("\"{module}/m.js\"")), "{}", script.text());
    assert!(script.text().contains("__proxyLocation"), "{}", script.text());
    // Its map is served on the document's own preview origin.
    let own = preview_origin(&www);
    assert!(
        script.text().contains(&format!("sourceMappingURL={own}/__demi/host/source-map?url={}", encoded(&js))),
        "{}",
        script.text()
    );
    // A worker's own script starts the worker's runtime first.
    let worker = relay
        .fetch(
            www.clone(),
            request(&format!("{js}&__demi_worker=classic"), PreviewMode::SameOrigin, "worker", Some(www.clone())),
        )
        .await
        .unwrap();
    assert!(worker.text().starts_with("globalThis.__proxyBoot="), "{}", worker.text());
    // The same text fetched as data stays as it is.
    let data = relay
        .fetch(www.clone(), request(&js, PreviewMode::Cors, "", Some(www.clone())))
        .await
        .unwrap();
    assert_eq!(data.text(), source);
    // A debugger asks for the map.
    let map = relay
        .fetch(
            www.clone(),
            request(&format!("{}/__demi/host/source-map?url={}", www.origin, encoded(&js)), PreviewMode::Cors, "", Some(www)),
        )
        .await
        .unwrap();
    let map: serde_json::Value = serde_json::from_slice(&map.body).unwrap();
    assert!(map["mappings"].as_str().is_some_and(|mappings| !mappings.is_empty()), "{map}");
}

#[tokio::test]
async fn integrity_is_checked_against_the_upstream_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let site = Site::start().await;
    let mut relay = Relay::open(engine(&directory));
    let www = top(&site.origin("www.site.test"));
    let source = "location.reload();";
    let js = content(&site, "www.site.test", "text/javascript", source);
    let digest = base64::engine::general_purpose::STANDARD.encode(sha2::Sha384::digest(source.as_bytes()));
    let script = |integrity: &str| {
        request(
            &format!("{js}&__demi_integrity={}", encoded(integrity)),
            PreviewMode::Cors,
            "script",
            Some(www.clone()),
        )
    };
    // The rewritten script differs from the upstream bytes; the check is on
    // those.
    let checked = relay.fetch(www.clone(), script(&format!("sha384-{digest}"))).await.unwrap();
    assert!(checked.text().contains("__proxyLocation"), "{}", checked.text());
    let refused = relay
        .fetch(www.clone(), script(&format!("sha256-{digest} sha384-AAAA")))
        .await;
    assert_eq!(refused.unwrap_err(), "refused: integrity");
}
