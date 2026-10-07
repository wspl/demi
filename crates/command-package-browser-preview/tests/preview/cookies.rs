//! The Host's cookie jar (`preview.md` § Cookies, § Page state): which
//! cookies a request carries, `document.cookie`, the cookies page state
//! moves, and the jar's file.

use demi_command_package_browser_preview::{CookieSameSite, EngineError};
use demi_command_package_browser_protocol::preview::{PreviewMode, PreviewRequest};
use url::Url;

use crate::support::{Relay, Site, echoed_header, embedded, encoded, engine, jar, network, opening, request, top};

/// The echo route of `host`, answering with `cookies` set.
fn setting(site: &Site, host: &str, cookies: &[&str]) -> String {
    let headers: String = cookies.iter().map(|cookie| format!("&h={}", encoded(&format!("Set-Cookie:{cookie}")))).collect();
    site.https(host, &format!("/echo?set{headers}"))
}

#[tokio::test]
async fn samesite_decides_which_cookies_a_request_carries() {
    let directory = tempfile::tempdir().unwrap();
    let site = Site::start().await;
    let mut relay = Relay::open(engine(&directory));
    let www = top(&site.origin("www.site.test"));
    let other = top(&site.origin("other.test"));
    let set = setting(&site, "www.site.test", &["strict=1; SameSite=Strict; Secure", "lax=1", "none=1; SameSite=None; Secure"]);
    relay.fetch(www.clone(), opening(&set)).await.unwrap();
    let echo = site.https("www.site.test", "/echo");
    let cookie = |fetched: &crate::support::Fetched| echoed_header(&fetched.echoed(), "cookie").map(ToOwned::to_owned);

    // The site's own image: same-site with the top, every cookie.
    let own = relay
        .fetch(www.clone(), request(&echo, PreviewMode::NoCors, "image", Some(www.clone())))
        .await
        .unwrap();
    assert_eq!(cookie(&own).as_deref(), Some("strict=1; lax=1; none=1"));

    // A link from another site: Lax for a GET, None-only for a POST.
    let linked = relay
        .fetch(www.clone(), request(&echo, PreviewMode::Navigate, "document", Some(other.clone())))
        .await
        .unwrap();
    assert_eq!(cookie(&linked).as_deref(), Some("lax=1; none=1"));
    let posted = PreviewRequest {
        method: "POST".into(),
        ..request(&echo, PreviewMode::Navigate, "document", Some(other.clone()))
    };
    let posted = relay.fetch_with_body(www.clone(), posted, b"a=1").await.unwrap();
    assert_eq!(cookie(&posted).as_deref(), Some("none=1"));

    // A credentialed CORS request of another site's page: None only.
    let allowed = format!(
        "{echo}?h={}&h={}",
        encoded(&format!("Access-Control-Allow-Origin:{}", other.origin)),
        encoded("Access-Control-Allow-Credentials:true")
    );
    let cors = relay
        .fetch(other.clone(), request(&allowed, PreviewMode::Cors, "", Some(other.clone())))
        .await
        .unwrap();
    assert_eq!(cookie(&cors).as_deref(), Some("none=1"));
}

#[tokio::test]
async fn a_cross_site_no_cors_request_carries_no_cookie() {
    let directory = tempfile::tempdir().unwrap();
    let site = Site::start().await;
    let mut relay = Relay::open(engine(&directory));
    let www = top(&site.origin("www.site.test"));
    let set = setting(&site, "www.site.test", &["none=1; SameSite=None; Secure"]);
    relay.fetch(www, opening(&set)).await.unwrap();
    // Another site's page reads the image it gets (`preview.md` § The
    // forwarder and the relay): it must not be the user's.
    let other = top(&site.origin("other.test"));
    let image = relay
        .fetch(
            other.clone(),
            request(&site.https("www.site.test", "/echo"), PreviewMode::NoCors, "image", Some(other)),
        )
        .await
        .unwrap();
    assert_eq!(echoed_header(&image.echoed(), "cookie"), None);
    // A frame of that site embedded in the other is its own document:
    // its navigation carries the cookie.
    let frame = embedded(&site.origin("www.site.test"), "https://other.test", true);
    let navigated = relay
        .fetch(
            frame.clone(),
            request(&site.https("www.site.test", "/echo"), PreviewMode::Navigate, "iframe", Some(top(&site.origin("other.test")))),
        )
        .await
        .unwrap();
    assert_eq!(echoed_header(&navigated.echoed(), "cookie"), Some("none=1"));
}

#[tokio::test]
async fn document_cookie_reads_and_writes_the_jar_but_never_an_httponly_cookie() {
    let directory = tempfile::tempdir().unwrap();
    let site = Site::start().await;
    let mut relay = Relay::open(engine(&directory));
    let www = top(&site.origin("www.site.test"));
    let page = site.https("www.site.test", "/app");
    relay
        .fetch(www.clone(), opening(&setting(&site, "www.site.test", &["session=s; HttpOnly; Secure"])))
        .await
        .unwrap();
    let endpoint = format!("{}/__demi/host/cookie?url={}", www.origin, encoded(&page));
    let write = || PreviewRequest {
        method: "POST".into(),
        ..request(&endpoint, PreviewMode::Cors, "", Some(www.clone()))
    };
    let written = relay.fetch_with_body(www.clone(), write(), b"theme=dark").await.unwrap();
    assert_eq!(written.text(), "theme=dark");
    // A script can neither read nor replace an HttpOnly cookie.
    let refused = relay.fetch_with_body(www.clone(), write(), b"session=evil").await.unwrap();
    assert_eq!(refused.text(), "theme=dark");
    let echo = relay
        .fetch(www.clone(), request(&site.https("www.site.test", "/echo"), PreviewMode::Cors, "", Some(www.clone())))
        .await
        .unwrap();
    assert_eq!(echoed_header(&echo.echoed(), "cookie"), Some("session=s; theme=dark"));
    // An answer that changed cookies tells the runtime to read them again.
    let changed = relay
        .fetch(
            www.clone(),
            request(&setting(&site, "www.site.test", &["seen=1"]), PreviewMode::Cors, "", Some(www.clone())),
        )
        .await
        .unwrap();
    assert_eq!(changed.header("x-demi-cookie-changed"), Some("1"));
    assert_eq!(echo.header("x-demi-cookie-changed"), None);
    // Another origin's address is not the document's.
    let foreign = format!("{}/__demi/host/cookie?url={}", www.origin, encoded(&site.https("other.test", "/")));
    assert!(relay.fetch(www.clone(), request(&foreign, PreviewMode::Cors, "", Some(www))).await.is_err());
}

#[tokio::test]
async fn page_state_moves_cookies_with_their_attributes() {
    let directory = tempfile::tempdir().unwrap();
    let site = Site::start().await;
    let engine = engine(&directory);
    let www = Url::parse(&site.https("www.site.test", "/")).unwrap();
    let other = Url::parse(&site.https("other.test", "/")).unwrap();
    // As the conversation browser's cookies arrive: Set-Cookie values with
    // every attribute, HttpOnly included; one a browser refuses.
    let kept = engine.take_cookies([
        (&www, "id=7; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=3600"),
        (&www, "pref=a; Domain=site.test"),
        (&www, "loose=1; SameSite=None"),
        (&other, "elsewhere=1"),
    ]);
    assert_eq!(kept, 3);
    let listed = engine.cookies(std::slice::from_ref(&www));
    let names: Vec<&str> = listed.iter().map(|cookie| cookie.name.as_str()).collect();
    assert_eq!(names, ["id", "pref"]);
    let id = &listed[0];
    assert!(id.http_only && id.secure && id.host_only);
    assert_eq!((id.domain.as_str(), id.path.as_str()), ("www.site.test", "/"));
    assert_eq!(id.same_site, Some(CookieSameSite::Strict));
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    assert!(id.expires.is_some_and(|expires| (now + 3500..=now + 3700).contains(&expires)), "{id:?}");
    let pref = &listed[1];
    assert!(!pref.host_only && !pref.http_only);
    assert_eq!((pref.domain.as_str(), pref.same_site, pref.expires), ("site.test", None, None));
    // The jar sends what it took.
    let mut relay = Relay::open(engine);
    let opened = relay
        .fetch(crate::support::top(&site.origin("www.site.test")), opening(&site.https("www.site.test", "/echo")))
        .await
        .unwrap();
    assert_eq!(echoed_header(&opened.echoed(), "cookie"), Some("id=7; pref=a"));
}

#[tokio::test]
async fn the_jar_is_written_on_change_and_read_at_start() {
    let directory = tempfile::tempdir().unwrap();
    let site = Site::start().await;
    let www = top(&site.origin("www.site.test"));
    {
        let engine = engine(&directory);
        let mut relay = Relay::open(engine.clone());
        relay
            .fetch(www.clone(), opening(&setting(&site, "www.site.test", &["kept=1; Max-Age=600", "session=2"])))
            .await
            .unwrap();
        // Written a moment after the change, without a close.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        while !std::fs::read_to_string(jar(&directory)).is_ok_and(|text| text.contains("session=2")) {
            assert!(tokio::time::Instant::now() < deadline, "the jar was not written");
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        // A change just before the program ends is written by its close.
        relay
            .fetch(www.clone(), opening(&setting(&site, "www.site.test", &["late=3"])))
            .await
            .unwrap();
        relay.end().await.unwrap();
        engine.close().await.unwrap();
    }
    let mut relay = Relay::open(engine(&directory));
    let opened = relay.fetch(www, opening(&site.https("www.site.test", "/echo"))).await.unwrap();
    // Session cookies too: a preview tab outlives the engine's restart.
    assert_eq!(echoed_header(&opened.echoed(), "cookie"), Some("kept=1; session=2; late=3"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(jar(&directory)).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "only its owner reads the jar");
    }
}

#[tokio::test]
async fn a_jar_file_the_engine_did_not_write_is_refused_and_kept() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(jar(&directory).parent().unwrap()).unwrap();
    std::fs::write(jar(&directory), "not a jar").unwrap();
    let opened = demi_command_package_browser_preview::Engine::open_for_tests(jar(&directory), network());
    assert!(matches!(opened, Err(EngineError::CorruptJar(..))));
    assert_eq!(std::fs::read_to_string(jar(&directory)).unwrap(), "not a jar");
}

