use axum::{
    Router,
    response::Html,
    routing::{any, get},
};
use chromiumoxide::{
    Browser, BrowserConfig,
    cdp::{
        browser_protocol::{
            accessibility::GetFullAxTreeParams,
            page::{
                CaptureScreenshotFormat, CaptureScreenshotParams, EventScreencastFrame,
                ScreencastFrameAckParams, StartScreencastParams, StopScreencastParams,
            },
        },
        js_protocol::runtime::{EvaluateParams, EventConsoleApiCalled},
    },
};
use futures_util::StreamExt;
use serde_json::json;
use std::{
    error::Error,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::time::{Instant, sleep, timeout};
type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

// Exercise the browser contract against a local, deterministic page.
async fn probe_browser_contract(browser: &Browser, url: &str, hits: &AtomicUsize) -> Result<()> {
    let page = browser.new_page(url).await?;
    println!("version={}", browser.version().await?.product);
    page.find_element("#email")
        .await?
        .click()
        .await?
        .type_str("probe@example.test")
        .await?;
    page.find_element("#normal").await?.click().await?;
    println!(
        "basic={}",
        page.evaluate(
            "JSON.stringify({email:document.querySelector('#email').value,clicks:normalClicks})"
        )
        .await?
        .into_value::<String>()?
    );
    let ax = page.execute(GetFullAxTreeParams::default()).await?;
    let names: Vec<_> = ax
        .nodes
        .iter()
        .filter_map(|n| n.name.as_ref().and_then(|v| v.value.as_ref()))
        .collect();
    println!("ax_names={}", serde_json::to_string(&names)?);
    let start = Instant::now();
    let missing = page.find_element("#missing").await;
    println!(
        "missing_error={} elapsed_ms={}",
        missing.is_err(),
        start.elapsed().as_millis()
    );
    println!(
        "duplicate_single_accepts={} duplicate_count={}",
        page.find_element(".duplicate").await.is_ok(),
        page.find_elements(".duplicate").await?.len()
    );
    let covered = page.find_element("#covered").await?.click().await.is_ok();
    let disabled = page.find_element("#disabled").await?.click().await.is_ok();
    let action_counts = page
        .evaluate("JSON.stringify({coveredClicks,overlayClicks,disabledClicks})")
        .await?
        .into_value::<String>()?;
    println!(
        "actionability={}",
        json!({
            "covered_returns_ok": covered,
            "disabled_returns_ok": disabled,
            "actual": action_counts,
        })
    );
    for expression in [
        "document.title",
        "document.querySelector('#email').value",
        "document.body.setAttribute('data-mutated','yes')",
        "localStorage.setItem('probe','yes')",
        "fetch('/side-effect')",
        "navigator.sendBeacon('/side-effect','x')",
        "window.evil",
        "setTimeout(()=>window.sideEffects++,0)",
        "window.sideEffects++",
    ] {
        let request = EvaluateParams::builder()
            .expression(expression)
            .throw_on_side_effect(true)
            .return_by_value(true)
            .await_promise(false)
            .build()?;
        let response = page.execute(request).await?;
        println!(
            "readonly={}",
            json!({
                "expression": expression,
                "rejected": response.exception_details.is_some(),
                "value": response.result.result.value,
            })
        );
    }
    sleep(Duration::from_millis(100)).await;
    let effects = page
        .evaluate("JSON.stringify({mutated:document.body.hasAttribute('data-mutated'),stored:localStorage.getItem('probe'),sideEffects})")
        .await?
        .into_value::<String>()?;
    println!(
        "readonly_effects={} network_hits={}",
        effects,
        hits.load(Ordering::SeqCst)
    );
    let png = page
        .screenshot(
            CaptureScreenshotParams::builder()
                .format(CaptureScreenshotFormat::Png)
                .build(),
        )
        .await?;
    if !png.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("invalid PNG signature".into());
    }
    println!("screenshot_bytes={}", png.len());
    let mut frames = page.event_listener::<EventScreencastFrame>().await?;
    page.execute(StartScreencastParams::default()).await?;
    for index in 0..3 {
        let frame = timeout(Duration::from_secs(5), frames.next())
            .await?
            .ok_or("frame stream closed")?;
        println!(
            "frame={} session={} width={} height={}",
            index, frame.session_id, frame.metadata.device_width, frame.metadata.device_height
        );
        page.execute(ScreencastFrameAckParams::new(frame.session_id))
            .await?;
    }
    page.execute(StopScreencastParams::default()).await?;
    drop(frames);
    println!("stream_stopped=true");
    let mut logs = page.event_listener::<EventConsoleApiCalled>().await?;
    page.evaluate("for(let i=0;i<1000;i++)console.log('probe-backlog',i)")
        .await?;
    // Intentionally leave the browser subscription unread during this bounded burst.
    sleep(Duration::from_millis(100)).await;
    let mut retained = 0;
    while let Ok(Some(_)) = timeout(Duration::from_millis(100), logs.next()).await {
        retained += 1;
    }
    drop(logs);
    println!("unread_console_events_retained={retained}");
    let other = browser.new_page(url).await?;
    let request = EvaluateParams::builder()
        .expression("new Promise(r=>setTimeout(()=>{window.sideEffects=99;r(99)},500))")
        .await_promise(true)
        .build()?;
    let canceled = timeout(Duration::from_millis(50), page.execute(request))
        .await
        .is_err();
    println!(
        "cancel_wait={} other_tab_usable={}",
        canceled,
        other.evaluate("1+1").await?.into_value::<u32>()? == 2
    );
    sleep(Duration::from_millis(600)).await;
    println!(
        "after_cancel_side_effects={}",
        page.evaluate("sideEffects").await?.into_value::<u32>()?
    );
    other.close().await?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let chrome = std::env::args()
        .nth(1)
        .ok_or("pass the Chrome for Testing executable")?;
    let scenario = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "normal".to_owned());
    if !["normal", "failure", "kill"].contains(&scenario.as_str()) {
        return Err("scenario must be normal, failure, or kill".into());
    }
    let profile = tempfile::tempdir()?;
    let config = BrowserConfig::builder()
        .chrome_executable(chrome)
        .user_data_dir(profile.path())
        .request_timeout(Duration::from_secs(5))
        .build()?;
    let hits = Arc::new(AtomicUsize::new(0));
    let route_hits = hits.clone();
    let app = Router::new()
        .route("/", get(|| async { Html(include_str!("fixture.html")) }))
        .route(
            "/side-effect",
            any(move || {
                let hits = route_hits.clone();
                async move {
                    hits.fetch_add(1, Ordering::SeqCst);
                    "recorded"
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}", listener.local_addr()?);
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                // Sender loss also requests shutdown; the server has no further work.
                let _ = stopped.await;
            })
            .await
    });
    let run = async {
        let (mut browser, mut handler) = Browser::launch(config).await?;
        let mut pump = tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                event?;
            }
            Ok::<(), chromiumoxide::error::CdpError>(())
        });
        let result = timeout(Duration::from_secs(45), async {
            if scenario == "failure" {
                browser.new_page(&url).await?;
                return Err("injected probe failure".into());
            }
            if scenario == "kill" {
                browser.new_page(&url).await?;
                return Ok(());
            }
            probe_browser_contract(&browser, &url, &hits).await
        })
        .await;
        let close = if scenario == "kill" {
            None
        } else {
            Some(timeout(Duration::from_secs(5), browser.close()).await)
        };
        println!("close_result={close:?}");
        let exited = if matches!(close, Some(Ok(Ok(_)))) {
            matches!(
                timeout(Duration::from_secs(5), browser.wait()).await,
                Ok(Ok(_))
            )
        } else {
            false
        };
        let killed = if !exited {
            browser.kill().await.transpose()
        } else {
            Ok(None)
        };
        let reaped = browser.try_wait();
        match timeout(Duration::from_secs(5), &mut pump).await {
            Ok(joined) => println!("pump_exit={joined:?}"),
            Err(_) => {
                pump.abort();
                println!("pump_abort_join={:?}", pump.await);
            }
        }
        killed?;
        println!("browser_reaped={}", reaped?.is_some());
        result??;
        Ok::<(), Box<dyn Error + Send + Sync>>(())
    }
    .await;
    // A closed receiver means the server already ended; joining reports its error.
    let _ = stop.send(());
    server.await??;
    profile.close()?;
    run
}
