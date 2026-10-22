
use crate::families::with_browser_fixture;
use axum::{Router, response::Html, routing::get};
use serde_json::json;
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
    _task: AbortOnDropHandle<()>,
}

async fn site() -> Site {
    let app = Router::new().route("/", get(|| async { Html(REPORT) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = AbortOnDropHandle::new(tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    }));
    Site { base, _task: task }
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
