use std::sync::{Arc, Mutex};

use axum::{Router, extract::State, http::HeaderMap, response::Html, routing::get};
use crate::families::with_browser_fixture;
use demi_command_service::protocol::CommandLocale;
use serde_json::{Value, json};
use tokio_util::{sync::CancellationToken, task::AbortOnDropHandle};

/// What a page learns about its browser, in its window and in a worker.
const REPORT: &str = r#"<!doctype html>
<style>body { margin: 0; height: 3000px }</style>
<p id="top">Fidelity</p>
<script>
const source = 'postMessage({ userAgent: navigator.userAgent, webdriver: navigator.webdriver ?? null, language: navigator.language, timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone })';
const worker = new Worker(URL.createObjectURL(new Blob([source], { type: 'text/javascript' })));
worker.onmessage = event => {
  window.report = {
    webdriver: navigator.webdriver,
    userAgent: navigator.userAgent,
    brands: navigator.userAgentData.brands.map(entry => entry.brand),
    language: navigator.language,
    timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
    defaultLocale: new Intl.DateTimeFormat().resolvedOptions().locale,
    scrollbar: innerWidth - document.documentElement.clientWidth,
    inner: [innerWidth, innerHeight],
    outer: [outerWidth, outerHeight],
    devicePixelRatio,
    hover: matchMedia('(hover: hover)').matches,
    finePointer: matchMedia('(pointer: fine)').matches,
    worker: event.data,
  };
  worker.terminate();
};
</script>"#;

struct Site {
    base: String,
    headers: Arc<Mutex<Vec<HeaderMap>>>,
    _task: AbortOnDropHandle<()>,
}

async fn site() -> Site {
    let headers = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route(
            "/",
            get(
                |State(seen): State<Arc<Mutex<Vec<HeaderMap>>>>, request: HeaderMap| async move {
                    seen.lock().unwrap().push(request);
                    Html(REPORT)
                },
            ),
        )
        .with_state(headers.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = AbortOnDropHandle::new(tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    }));
    Site {
        base,
        headers,
        _task: task,
    }
}

/// The page's report once its worker answered.
async fn page_report(fixture: &crate::families::BrowserFixture, tab: &str) -> Value {
    fixture
        .call(
            "browser.wait",
            json!({"tab": tab, "css": "#top", "state": "visible", "timeout": 30000}),
        )
        .await;
    for _ in 0..100 {
        let value = fixture
            .call(
                "browser.eval",
                json!({"tab": tab, "expression": "window.report ?? null"}),
            )
            .await["value"]
            .clone();
        if !value.is_null() {
            return value;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("the page never reported");
}

#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn pages_see_an_ordinary_chrome_in_the_users_time_zone_and_languages() {
    let site = site().await;
    let base = site.base.clone();
    with_browser_fixture(|fixture| async move {
        let mut fixture = fixture;
        fixture.locale = CommandLocale {
            time_zone: "America/Sao_Paulo".into(),
            languages: vec!["pt-BR".into(), "en".into()],
        };
        let tab = fixture
            .call(
                "browser.open",
                json!({"url": format!("{base}/"), "timeout": 120000}),
            )
            .await["tab"]
            .as_str()
            .unwrap()
            .to_owned();
        let report = page_report(&fixture, &tab).await;
        // No automation marker, in the window or in a worker.
        assert_eq!(report["webdriver"], false, "{report}");
        assert_ne!(report["worker"]["webdriver"], true, "{report}");
        let agent = report["userAgent"].as_str().unwrap();
        assert!(!agent.contains("Headless"), "{agent}");
        assert!(agent.contains(".0.0.0 Safari/537.36"), "{agent}");
        assert_eq!(report["worker"]["userAgent"], report["userAgent"]);
        assert!(
            report["brands"]
                .as_array()
                .unwrap()
                .iter()
                .any(|brand| brand == "Chromium")
        );
        // Scrollbars take their usual width, and the window holds the viewport.
        assert!(report["scrollbar"].as_f64().unwrap() > 0.0, "{report}");
        assert!(
            report["outer"][0].as_f64() >= report["inner"][0].as_f64(),
            "{report}"
        );
        assert!(
            report["outer"][1].as_f64() > report["inner"][1].as_f64(),
            "{report}"
        );
        // A mouse with hover, on Linux too.
        assert_eq!(report["hover"], true, "{report}");
        assert_eq!(report["finePointer"], true, "{report}");
        // The user's time zone and languages, whatever the Host's.
        assert_eq!(report["timeZone"], "America/Sao_Paulo");
        assert_eq!(report["worker"]["timeZone"], "America/Sao_Paulo");
        assert_eq!(report["language"], "pt-BR");
        assert_eq!(report["worker"]["language"], "pt-BR");
        assert_eq!(report["defaultLocale"], "pt-BR");
        let requests = site.headers.lock().unwrap().clone();
        let request = requests.last().unwrap();
        let header = |name: &str| request.get(name).unwrap().to_str().unwrap().to_owned();
        assert!(
            header("accept-language").starts_with("pt-BR"),
            "{requests:?}"
        );
        assert!(!header("user-agent").contains("Headless"), "{requests:?}");
        // The environment keeps the locale it started with.
        fixture.locale = CommandLocale {
            time_zone: "Asia/Tokyo".into(),
            languages: vec!["ja".into()],
        };
        let second = fixture
            .call(
                "browser.open",
                json!({"url": format!("{base}/"), "timeout": 120000}),
            )
            .await["tab"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(
            page_report(&fixture, &second).await["timeZone"],
            "America/Sao_Paulo"
        );
        fixture
    })
    .await;
}

#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn the_agents_viewport_sets_the_pixel_ratio_and_screenshots_stay_in_css_pixels() {
    let site = site().await;
    let base = site.base.clone();
    with_browser_fixture(|fixture| async move {
        let opened = fixture
            .call("browser.open", json!({"url": format!("{base}/"), "timeout": 120000}))
            .await;
        let tab = opened["tab"].as_str().unwrap().to_owned();
        assert_eq!(
            opened["viewport"],
            json!({"width": 1280, "height": 720, "devicePixelRatio": 1.0, "mode": "web"})
        );
        let set = fixture
            .call(
                "browser.viewport.set",
                json!({"tab": tab, "width": 800, "height": 600, "scale": 2}),
            )
            .await;
        let custom = json!({"width": 800, "height": 600, "devicePixelRatio": 2.0, "mode": "custom"});
        assert_eq!(set["viewport"], custom);
        assert_eq!(
            fixture.call("browser.info", json!({"tab": tab})).await["viewport"],
            custom
        );
        let ratio = fixture
            .call(
                "browser.eval",
                json!({"tab": tab, "expression": "[devicePixelRatio, innerWidth, innerHeight, outerHeight > innerHeight]"}),
            )
            .await;
        assert_eq!(ratio["value"], json!([2, 800, 600, true]));

        // A screenshot is in CSS pixels whatever the ratio, where the page
        // is scrolled to, and reports the ratio it rendered at.
        fixture
            .call("browser.scroll", json!({"tab": tab, "xy": "400,300", "dy": 500}))
            .await;
        let shot = fixture
            .call(
                "browser.screenshot",
                json!({"tab": tab, "output": "shot.png"}),
            )
            .await;
        assert_eq!((shot["width"].clone(), shot["height"].clone()), (json!(800), json!(600)));
        assert_eq!(shot["viewport"], custom);
        let clip = fixture
            .call(
                "browser.screenshot",
                json!({"tab": tab, "clip": "0,0,100,50", "output": "clip.png"}),
            )
            .await;
        assert_eq!((clip["width"].clone(), clip["height"].clone()), (json!(100), json!(50)));

        let reset = fixture
            .call("browser.viewport.reset", json!({"tab": tab}))
            .await;
        assert_eq!(
            reset["viewport"],
            json!({"width": 1280, "height": 720, "devicePixelRatio": 1.0, "mode": "web"})
        );
        let back = fixture
            .call(
                "browser.screenshot",
                json!({"tab": tab, "output": "back.png"}),
            )
            .await;
        assert_eq!((back["width"].clone(), back["height"].clone()), (json!(1280), json!(720)));
        let (code, _) = fixture
            .result(
                "browser.viewport.set",
                json!({"tab": tab, "width": 800, "height": 600, "scale": 8}),
                CancellationToken::new(),
            )
            .await;
        assert_eq!(code, 2);
        fixture
    })
    .await;
}
