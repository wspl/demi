use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{
    Router,
    response::Html,
    routing::{any, get},
};
use demi_commands::browser::{
    BrowserEnvironment, BrowserError, BrowserTab, LaunchOptions, Result, with_browser,
};
use serde_json::json;
use tokio::sync::Mutex;
use tokio_util::{sync::CancellationToken, task::AbortOnDropHandle};

const DEADLINE: Duration = Duration::from_secs(5);

/// Exercise the native driver without a model, runner grant, or product browser UI.
#[tokio::test]
#[ignore = "requires DEMI_TEST_CHROME pointing to an installed Chrome for Testing release"]
async fn browser_contract_and_cleanup() {
    let executable = PathBuf::from(std::env::var_os("DEMI_TEST_CHROME").expect("DEMI_TEST_CHROME"));
    let requests = Arc::new(AtomicUsize::new(0));
    let effects = requests.clone();
    let app = Router::new()
        .route(
            "/",
            get(|| async { Html(include_str!("browser/fixture.html")) }),
        )
        .route(
            "/side-effect",
            any(move || {
                let effects = effects.clone();
                async move {
                    effects.fetch_add(1, Ordering::SeqCst);
                    "effect"
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let stop_site = CancellationToken::new();
    let site_cancelled = stop_site.clone();
    let site = AbortOnDropHandle::new(tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(site_cancelled.cancelled_owned())
            .await
    }));
    let retained_tab = Arc::new(Mutex::new(None));
    let save_tab = retained_tab.clone();
    let result = with_browser(
        LaunchOptions::pinned(
            executable.clone(),
            demi_command_service::protocol::CommandLocale {
                time_zone: "UTC".into(),
                languages: vec!["en-US".into()],
            },
        )
        .unwrap(),
        CancellationToken::new(),
        |browser| exercise_browser(browser, url, requests, save_tab),
    )
    .await;
    assert!(result.is_ok(), "{result:?}");
    let tab = retained_tab.lock().await.take().unwrap();
    assert!(matches!(
        tab.read_only("1", &CancellationToken::new(), DEADLINE)
            .await,
        Err(BrowserError::Closed)
    ));

    let result = with_browser(
        LaunchOptions::pinned(
            executable.clone(),
            demi_command_service::protocol::CommandLocale {
                time_zone: "UTC".into(),
                languages: vec!["en-US".into()],
            },
        )
        .unwrap(),
        CancellationToken::new(),
        |browser| async move {
            browser
                .open("about:blank", &CancellationToken::new(), DEADLINE)
                .await?;
            Err::<(), _>(BrowserError::Configuration("injected failure".into()))
        },
    )
    .await;
    assert!(
        matches!(result, Err(BrowserError::Configuration(_))),
        "{result:?}"
    );
    let stop = CancellationToken::new();
    let end = stop.clone();
    let result = with_browser(
        LaunchOptions::pinned(
            executable,
            demi_command_service::protocol::CommandLocale {
                time_zone: "UTC".into(),
                languages: vec!["en-US".into()],
            },
        )
        .unwrap(),
        stop,
        |browser| async move {
            browser
                .open("about:blank", &CancellationToken::new(), DEADLINE)
                .await?;
            end.cancel();
            std::future::pending::<demi_commands::browser::Result<()>>().await
        },
    )
    .await;
    assert!(matches!(result, Err(BrowserError::Cancelled)), "{result:?}");
    stop_site.cancel();
    site.await.unwrap().unwrap();
}

/// Check browser behavior with deterministic page state and no provider calls.
async fn exercise_browser(
    browser: BrowserEnvironment,
    url: String,
    requests: Arc<AtomicUsize>,
    save_tab: Arc<Mutex<Option<BrowserTab>>>,
) -> Result<()> {
    let live = CancellationToken::new();
    assert!(browser.tabs(&live, DEADLINE).await?.is_empty());
    let tab = browser.open(&url, &live, DEADLINE).await?;
    assert_eq!(
        tab.read_only("document.title", &live, DEADLINE).await?,
        json!("Native browser test")
    );
    tab.fill_css("#email", "hello@example.test", &live, DEADLINE)
        .await?;
    tab.click_css("#normal", &live, DEADLINE).await?;
    assert_eq!(
        tab.read_only(
            "({email:document.querySelector('#email').value,clicks:normalClicks})",
            &live,
            DEADLINE
        )
        .await?,
        json!({"email":"hello@example.test","clicks":1})
    );
    for selector in ["#covered", "#disabled", "#missing"] {
        let result = tab
            .click_css(selector, &live, Duration::from_millis(250))
            .await;
        assert!(
            matches!(
                result,
                Err(BrowserError::NotActionable { .. } | BrowserError::TargetNotFound)
            ),
            "{selector}: {result:?}"
        );
    }
    assert!(matches!(
        tab.click_css(".duplicate", &live, DEADLINE).await,
        Err(BrowserError::Ambiguous(2))
    ));
    assert_eq!(
        tab.read_only(
            "[coveredClicks,overlayClicks,disabledClicks]",
            &live,
            DEADLINE
        )
        .await?,
        json!([0, 0, 0])
    );
    for expression in [
        "document.body.setAttribute('data-mutated','yes')",
        "localStorage.setItem('probe','yes')",
        "fetch('/side-effect')",
        "navigator.sendBeacon('/side-effect','x')",
        "window.evil",
        "setTimeout(()=>window.sideEffects++,0)",
        "window.sideEffects++",
        "({get x(){window.sideEffects++;return 1}})",
        "({x:undefined})",
        "({x:NaN})",
        "document.body",
        "(()=>{const a={};a.self=a;return a})()",
        "[1,,3]",
    ] {
        assert!(
            tab.read_only(expression, &live, DEADLINE).await.is_err(),
            "accepted {expression}"
        );
    }
    assert_eq!(
        tab.read_only("(()=>{const a={x:1};return [a,a]})()", &live, DEADLINE)
            .await?,
        json!([{"x":1},{"x":1}])
    );
    // Storage getters are conservatively rejected by Chrome's debug evaluator.
    // Read the fixture's storage through an ordinary page action instead.
    tab.click_css("#check-storage", &live, DEADLINE).await?;
    assert_eq!(tab.read_only("({mutated:document.body.hasAttribute('data-mutated'),stored:window.storageValue,sideEffects})", &live, DEADLINE).await?, json!({"mutated":false,"stored":null,"sideEffects":0}));
    assert_eq!(requests.load(Ordering::SeqCst), 0);
    tab.click_css("#arm", &live, DEADLINE).await?;
    tab.click_css("#late", &live, DEADLINE).await?;
    assert_eq!(
        tab.read_only("lateClicks", &live, DEADLINE).await?,
        json!(1)
    );
    let png = tab.screenshot(&live, DEADLINE).await?;
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));

    let other = browser.open("about:blank", &live, DEADLINE).await?;
    let cancel = CancellationToken::new();
    let (waiting, independent) =
        tokio::join!(tab.click_css("#disabled", &cancel, DEADLINE), async {
            let result = other.read_only("1 + 1", &live, DEADLINE).await;
            cancel.cancel();
            result
        },);
    assert!(matches!(waiting, Err(BrowserError::Cancelled)));
    assert_eq!(independent?, json!(2));
    let (waiting, busy) = tokio::join!(
        tab.click_css("#disabled", &live, Duration::from_millis(100)),
        tab.read_only("1", &live, DEADLINE),
    );
    assert!(matches!(waiting, Err(BrowserError::NotActionable { .. })));
    assert!(matches!(busy, Err(BrowserError::Busy)));

    tab.close(&live, DEADLINE).await?;
    assert!(
        !browser
            .tabs(&live, DEADLINE)
            .await?
            .iter()
            .any(|candidate| candidate.target_id() == tab.target_id())
    );
    *save_tab.lock().await = Some(other);
    Ok(())
}
