//! Round-one regression specifications. These compile without launching Chrome.

use demi_commands::browser::{
    BrowserEnvironment, BrowserError, BrowserTab, LaunchOptions, Result, with_browser,
};
use serde_json::{Value, json};
use std::{
    future::Future,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::{
    sync::CancellationToken,
    task::{AbortOnDropHandle, TaskTracker},
};

const TIMEOUT: Duration = Duration::from_secs(10);

/// Serve deterministic browser fixtures, including failures before HTTP headers.
async fn with_fixture<F, W>(exercise: F)
where
    F: FnOnce(BrowserEnvironment, String) -> W,
    W: Future<Output = Result<()>>,
{
    let executable = PathBuf::from(std::env::var_os("DEMI_TEST_CHROME").expect("DEMI_TEST_CHROME"));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let stop = CancellationToken::new();
    let _stop_on_drop = stop.clone().drop_guard();
    let site_stop = stop.clone();
    let reloads = Arc::new(AtomicUsize::new(0));
    let server = AbortOnDropHandle::new(tokio::spawn(async move {
        let connections = TaskTracker::new();
        loop {
            let accepted = tokio::select! {
                _ = site_stop.cancelled() => break,
                result = listener.accept() => result,
            };
            let (mut socket, _) = accepted.unwrap();
            let stopped = site_stop.clone();
            let reloads = reloads.clone();
            connections.spawn(async move {
                let work = async {
                    let mut request = Vec::new();
                    loop {
                        let mut bytes = [0; 1024];
                        let size = socket.read(&mut bytes).await?;
                        if size == 0 {
                            return Ok::<_, std::io::Error>(());
                        }
                        request.extend_from_slice(&bytes[..size]);
                        if request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                            break;
                        }
                        assert!(request.len() < 16 * 1024);
                    }
                    let request = String::from_utf8(request).unwrap();
                    let path = request.split_whitespace().nth(1).unwrap();
                    if path == "/drop" || (path == "/reload-drop" && reloads.fetch_add(1, Ordering::SeqCst) > 0) {
                        return socket.shutdown().await;
                    }
                    if path == "/stall" {
                        stopped.cancelled().await;
                        return Ok(());
                    }
                    let (status, headers, body) = match path {
                        "/500" => ("500 Internal Server Error", "", "<!doctype html><title>HTTP error document</title><h1>Received 500</h1>"),
                        "/redirect" => ("302 Found", "Location: /destination\r\n", ""),
                        "/destination" => ("200 OK", "", "<!doctype html><title>Destination</title><h1>Arrived</h1>"),
                        _ => ("200 OK", "", include_str!("browser/repairs.html")),
                    };
                    let response = format!("HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}", body.len());
                    socket.write_all(response.as_bytes()).await?;
                    socket.shutdown().await
                };
                tokio::select! {
                    _ = stopped.cancelled() => {},
                    result = work => {
                        // Browser cancellation may close a fixture connection first.
                        if let Err(error) = result {
                            assert!(matches!(error.kind(), std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset), "{error}");
                        }
                    }
                }
            });
        }
        connections.close();
        connections.wait().await;
    }));
    let result = with_browser(
        LaunchOptions { executable },
        CancellationToken::new(),
        |browser| exercise(browser, base),
    )
    .await;
    stop.cancel();
    server.await.unwrap();
    assert!(result.is_ok(), "{result:?}");
}

/// Invoke the production command parser, tab gate, algorithms and result schema.
async fn command(tab: &BrowserTab, operation: &str, mut args: Value) -> Result<Value> {
    args["tab"] = json!(tab.id());
    if args.get("timeout").is_none() {
        args["timeout"] = json!(10_000);
    }
    tab.execute(operation, args, &CancellationToken::new())
        .await
}

async fn value(tab: &BrowserTab, css: &str) -> Result<Value> {
    Ok(command(tab, "read", json!({"css": css, "property": "value"})).await?["value"].clone())
}

async fn click(tab: &BrowserTab, css: &str) -> Result<()> {
    command(tab, "click", json!({"css": css})).await?;
    Ok(())
}

fn error(result: Result<Value>, code: &str, action: &str) -> BrowserError {
    let error = result.expect_err(code);
    assert_eq!(error.code(), code, "{error:?}");
    assert_eq!(error.details()["action"], action, "{error:?}");
    error
}

#[tokio::test]
#[ignore = "round two: requires DEMI_TEST_CHROME; launches a real browser"]
async fn native_fill_and_text_replacement() {
    with_fixture(|browser, base| async move {
        let tab = browser
            .open(&base, &CancellationToken::new(), TIMEOUT)
            .await?;
        for (css, text) in [
            ("#date", "2026-09-28"),
            ("#time", "14:35"),
            ("#month", "2026-09"),
            ("#week", "2026-W39"),
            ("#color", "#123456"),
            ("#range", "42"),
            ("#number", "123.5"),
        ] {
            command(&tab, "fill", json!({"css": css, "text": text})).await?;
            assert_eq!(value(&tab, css).await?, text);
        }
        for (css, text) in [
            ("#date", "2026-13-28"),
            ("#number", "not a number"),
            ("#time", "99:99"),
        ] {
            error(
                command(&tab, "fill", json!({"css": css, "text": text})).await,
                "invalid_input",
                "not_started",
            );
        }
        assert_eq!(value(&tab, "#date").await?, "2026-09-28");
        for css in ["#text", "#textarea", "#editable"] {
            command(&tab, "fill", json!({"css": css, "text": "replacement"})).await?;
            let property = if css == "#editable" { "text" } else { "value" };
            assert_eq!(
                command(&tab, "read", json!({"css": css, "property": property})).await?["value"],
                "replacement"
            );
            command(&tab, "fill", json!({"css": css, "text": ""})).await?;
            assert_eq!(
                command(&tab, "read", json!({"css": css, "property": property})).await?["value"],
                ""
            );
        }
        let started = tokio::time::Instant::now();
        error(
            command(
                &tab,
                "fill",
                json!({"css": "#plain", "text": "x", "timeout": 10_000}),
            )
            .await,
            "not_actionable",
            "not_started",
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        error(
            command(&tab, "fill", json!({"css": "#file", "text": "x"})).await,
            "not_actionable",
            "not_started",
        );
        error(
            command(
                &tab,
                "fill",
                json!({"css": "#datetime", "text": "2026-09-28T14:35"}),
            )
            .await,
            "not_actionable",
            "not_started",
        );
        error(
            command(
                &tab,
                "fill",
                json!({"css": "#readonly", "text": "x", "timeout": 500}),
            )
            .await,
            "not_actionable",
            "not_started",
        );
        command(
            &tab,
            "fill",
            json!({"css": "#covered-input", "text": "under overlay"}),
        )
        .await?;
        assert_eq!(value(&tab, "#covered-input").await?, "under overlay");
        click(&tab, "#transform-text").await?;
        command(&tab, "fill", json!({"css": "#text", "text": "transformed"})).await?;
        assert_eq!(value(&tab, "#text").await?, "TRANSFORMED");
        click(&tab, "#prevent-text").await?;
        command(&tab, "fill", json!({"css": "#text", "text": ""})).await?;
        assert_eq!(value(&tab, "#text").await?, "TRANSFORMED");
        Ok(())
    })
    .await;
}

#[tokio::test]
#[ignore = "round two: requires DEMI_TEST_CHROME; launches a real browser"]
async fn targeted_keyboard_preserves_selection_and_stops_on_focus_loss() {
    with_fixture(|browser, base| async move {
        let tab = browser
            .open(&base, &CancellationToken::new(), TIMEOUT)
            .await?;
        for prepare in ["#select-middle", "#select-blurred"] {
            command(&tab, "fill", json!({"css": "#text", "text": "hello"})).await?;
            click(&tab, prepare).await?;
            command(&tab, "type", json!({"css": "#text", "text": "XY"})).await?;
            assert_eq!(value(&tab, "#text").await?, "heXYo");
        }
        click(&tab, "#focus-other").await?;
        command(&tab, "key", json!({"css": "#unchecked", "key": "Space"})).await?;
        assert_eq!(
            command(
                &tab,
                "read",
                json!({"css": "#unchecked", "property": "checked"})
            )
            .await?["value"],
            true
        );
        assert_eq!(value(&tab, "#other").await?, "elsewhere");
        command(
            &tab,
            "key",
            json!({"css": "#text", "key": "ControlOrMeta+A"}),
        )
        .await?;
        command(&tab, "type", json!({"css": "#text", "text": "AB"})).await?;
        assert_eq!(value(&tab, "#text").await?, "AB");
        command(&tab, "key", json!({"css": "#text", "key": "+"})).await?;
        assert_eq!(value(&tab, "#text").await?, "AB+");
        command(&tab, "key", json!({"css": "#text", "key": "Shift+a"})).await?;
        command(&tab, "key", json!({"css": "#text", "key": "Shift+Digit1"})).await?;
        assert_eq!(value(&tab, "#text").await?, "AB+A!");
        error(
            command(
                &tab,
                "key",
                json!({"css": "#text", "key": "Control+NotAKey"}),
            )
            .await,
            "invalid_input",
            "not_started",
        );
        let events = tab
            .read_only("keys", &CancellationToken::new(), TIMEOUT)
            .await?;
        let events = events.as_array().unwrap();
        assert!(events.windows(2).any(|pair| pair[0]["key"] == "X"
            && pair[0]["type"] == "keydown"
            && pair[1]["key"] == "X"
            && pair[1]["type"] == "keyup"));
        assert_eq!(
            events
                .iter()
                .filter(|event| event["type"] == "keydown")
                .count(),
            events
                .iter()
                .filter(|event| event["type"] == "keyup")
                .count()
        );
        click(&tab, "#lose-focus").await?;
        let failed = error(
            command(&tab, "type", json!({"css": "#text", "text": "XYZ"})).await,
            "not_actionable",
            "completed",
        );
        assert_eq!(failed.details()["delivered"], 1);
        assert_eq!(value(&tab, "#other").await?, "elsewhere");
        command(&tab, "goto", json!({"url": base})).await?;
        click(&tab, "#replace-target").await?;
        let failed = error(
            command(&tab, "type", json!({"css": "#text", "text": "XYZ"})).await,
            "not_actionable",
            "completed",
        );
        assert_eq!(failed.details()["delivered"], 1);
        Ok(())
    })
    .await;
}

#[tokio::test]
#[ignore = "round two: requires DEMI_TEST_CHROME; launches a real browser"]
async fn check_verifies_state_and_select_waits_for_ordered_options() {
    with_fixture(|browser, base| async move {
        let tab = browser
            .open(&base, &CancellationToken::new(), TIMEOUT)
            .await?;
        command(&tab, "check", json!({"css": "#checked", "value": true})).await?;
        assert_eq!(
            command(
                &tab,
                "read",
                json!({"css": "#checked", "property": "checked"})
            )
            .await?["value"],
            true
        );
        error(
            command(&tab, "check", json!({"css": "#prevented", "value": true})).await,
            "not_actionable",
            "completed",
        );
        error(
            command(&tab, "check", json!({"css": "#radio", "value": false})).await,
            "invalid_input",
            "not_started",
        );
        error(
            command(&tab, "check", json!({"css": "#plain", "value": true})).await,
            "not_actionable",
            "not_started",
        );
        for css in ["#aria-check", "#aria-switch"] {
            command(&tab, "check", json!({"css": css, "value": true})).await?;
            assert_eq!(
                command(&tab, "read", json!({"css": css, "property": "checked"})).await?["value"],
                true
            );
        }
        click(&tab, "#add-late").await?;
        assert_eq!(
            command(&tab, "select", json!({"css": "#late", "value": ["new"]})).await?["result"],
            json!(["new"])
        );
        for candidate in ["bad", "group"] {
            error(
                command(
                    &tab,
                    "select",
                    json!({"css": "#disabled-option", "value": [candidate], "timeout": 500}),
                )
                .await,
                "not_actionable",
                "not_started",
            );
        }
        error(
            command(
                &tab,
                "select",
                json!({"css": "#single", "value": ["missing"], "timeout": 500}),
            )
            .await,
            "target_not_found",
            "not_started",
        );
        assert_eq!(
            command(
                &tab,
                "select",
                json!({"css": "#single", "value": ["two", "one"]})
            )
            .await?["result"],
            json!(["one"])
        );
        assert_eq!(
            command(
                &tab,
                "select",
                json!({"css": "#single", "value": ["missing", "two"]})
            )
            .await?["result"],
            json!(["two"])
        );
        assert_eq!(
            command(
                &tab,
                "select",
                json!({"css": "#multi", "value": ["two", "one"]})
            )
            .await?["result"],
            json!(["one", "two"])
        );
        assert_eq!(
            command(
                &tab,
                "select",
                json!({"css": "#single", "option-label": ["Second"]})
            )
            .await?["result"],
            json!(["two"])
        );
        assert_eq!(
            command(
                &tab,
                "select",
                json!({"css": "#hidden-select", "option-index": [1]})
            )
            .await?["result"],
            json!(["two"])
        );
        error(
            command(
                &tab,
                "select",
                json!({"css": "#single", "value": ["one"], "option-index": [0]}),
            )
            .await,
            "invalid_input",
            "not_started",
        );
        Ok(())
    })
    .await;
}

#[tokio::test]
#[ignore = "round two: requires DEMI_TEST_CHROME; launches a real browser"]
async fn action_conditions_shadow_hits_and_shared_wait_states() {
    with_fixture(|browser, base| async move {
        let tab = browser
            .open(&base, &CancellationToken::new(), TIMEOUT)
            .await?;
        let covered = error(
            command(
                &tab,
                "click",
                json!({"css": "#covered-button", "timeout": 500}),
            )
            .await,
            "not_actionable",
            "not_started",
        );
        assert!(
            covered.details()["interceptor"]
                .as_str()
                .unwrap()
                .contains("button-overlay")
        );
        for css in ["#fieldset-button", "#pointer-none", "#moving"] {
            error(
                command(&tab, "click", json!({"css": css, "timeout": 500})).await,
                "not_actionable",
                "not_started",
            );
        }
        command(&tab, "move", json!({"css": "#disabled-button"})).await?;
        assert_eq!(
            tab.read_only("hovered", &CancellationToken::new(), TIMEOUT)
                .await?,
            true
        );
        command(&tab, "scroll", json!({"css": "#disabled-button", "dy": 50})).await?;
        error(
            command(&tab, "click", json!({"xy": "99999,99999"})).await,
            "invalid_input",
            "not_started",
        );
        error(
            command(&tab, "click", json!({"css": "#no-nav", "xy": "10,10"})).await,
            "invalid_input",
            "not_started",
        );
        error(
            command(
                &tab,
                "wait",
                json!({"url": "**", "css": "#no-nav", "state": "visible"}),
            )
            .await,
            "invalid_input",
            "not_started",
        );
        error(
            command(&tab, "scroll", json!({"dy": 0})).await,
            "invalid_input",
            "not_started",
        );
        command(
            &tab,
            "click",
            json!({"role": "button", "name": "Shadow button", "exact": true}),
        )
        .await?;
        assert_eq!(
            command(
                &tab,
                "find",
                json!({"role": "button", "name": "Shadow clicked", "exact": true})
            )
            .await?["count"],
            1
        );
        command(
            &tab,
            "click",
            json!({"role": "button", "name": "Frame button", "exact": true}),
        )
        .await?;
        assert_eq!(
            command(
                &tab,
                "find",
                json!({"role": "button", "name": "Clicked frame", "exact": true})
            )
            .await?["count"],
            1
        );
        let year = command(
            &tab,
            "find",
            json!({"role": "spinbutton", "name": "Year", "exact": true}),
        )
        .await?;
        let reference = year["matches"][0]["ref"]
            .as_str()
            .expect("native date year reference");
        command(&tab, "click", json!({"ref": reference})).await?;
        error(
            command(&tab, "fill", json!({"ref": reference, "text": "2027"})).await,
            "not_actionable",
            "not_started",
        );
        error(
            command(
                &tab,
                "wait",
                json!({"css": "#hidden", "state": "visible", "timeout": 500}),
            )
            .await,
            "timeout",
            "not_started",
        );
        command(&tab, "wait", json!({"css": "#hidden", "state": "hidden"})).await?;
        command(&tab, "wait", json!({"css": "#hidden", "state": "attached"})).await?;
        for css in ["#fieldset-input", "#aria-disabled"] {
            error(
                command(
                    &tab,
                    "wait",
                    json!({"css": css, "state": "enabled", "timeout": 500}),
                )
                .await,
                "timeout",
                "not_started",
            );
            error(
                command(
                    &tab,
                    "fill",
                    json!({"css": css, "text": "x", "timeout": 500}),
                )
                .await,
                "not_actionable",
                "not_started",
            );
        }
        click(&tab, "#show-later").await?;
        command(&tab, "wait", json!({"css": "#hidden", "state": "visible"})).await?;
        click(&tab, "#enable-later").await?;
        command(
            &tab,
            "wait",
            json!({"css": "#aria-disabled", "state": "enabled"}),
        )
        .await?;
        let removable =
            command(&tab, "find", json!({"css": "#removable"})).await?["matches"][0]["ref"].clone();
        click(&tab, "#remove-later").await?;
        command(&tab, "wait", json!({"ref": removable, "state": "detached"})).await?;
        command(&tab, "wait", json!({"ref": removable, "state": "hidden"})).await?;
        error(
            command(&tab, "click", json!({"ref": removable})).await,
            "stale_ref",
            "not_started",
        );
        Ok(())
    })
    .await;
}

#[tokio::test]
#[ignore = "round two: requires DEMI_TEST_CHROME; launches a real browser"]
async fn label_text_and_accessible_name_are_distinct_and_ambiguity_is_explicit() {
    with_fixture(|browser, base| async move {
        let tab = browser
            .open(&base, &CancellationToken::new(), TIMEOUT)
            .await?;
        for (label, css) in [
            ("Appointment date", "#date"),
            ("Wrapped date", "#wrapped"),
            ("Related date", "#related"),
        ] {
            command(
                &tab,
                "fill",
                json!({"label": label, "exact": true, "text": "2026-10-01"}),
            )
            .await?;
            assert_eq!(value(&tab, css).await?, "2026-10-01");
        }
        assert_eq!(
            command(&tab, "find", json!({"label": "File label", "exact": true})).await?["count"],
            1
        );
        assert_eq!(
            command(&tab, "find", json!({"label": "Text field", "exact": true})).await?["count"],
            0
        );
        assert_eq!(
            command(
                &tab,
                "find",
                json!({"text-match": "Accessible label", "exact": true})
            )
            .await?["count"],
            0
        );
        assert_eq!(
            command(
                &tab,
                "find",
                json!({"text-match": "Visible words", "exact": true})
            )
            .await?["count"],
            1
        );
        assert_eq!(
            command(
                &tab,
                "find",
                json!({"role": "button", "name": "Accessible label", "exact": true})
            )
            .await?["count"],
            1
        );
        assert_eq!(
            command(
                &tab,
                "find",
                json!({"role": "button", "name": "Visible words", "exact": true})
            )
            .await?["count"],
            0
        );
        assert_eq!(
            command(
                &tab,
                "find",
                json!({"role": "button", "name": "Fallback", "exact": true})
            )
            .await?["count"],
            2
        );
        command(
            &tab,
            "click",
            json!({"role": "button", "name": "Fallback", "exact": true}),
        )
        .await?;
        assert_eq!(
            tab.read_only("fallbackClicks", &CancellationToken::new(), TIMEOUT)
                .await?,
            1
        );
        for name in ["Ambiguous", "Hidden duplicate"] {
            error(
                command(
                    &tab,
                    "click",
                    json!({"role": "button", "name": name, "exact": true}),
                )
                .await,
                "ambiguous_target",
                "not_started",
            );
        }
        error(
            command(
                &tab,
                "click",
                json!({"css": ".fallback", "nth": 0, "timeout": 500}),
            )
            .await,
            "not_actionable",
            "not_started",
        );
        error(
            command(&tab, "read", json!({"css": "#missing", "property": "text"})).await,
            "target_not_found",
            "not_started",
        );
        error(
            command(&tab, "click", json!({"css": "#missing", "timeout": 500})).await,
            "target_not_found",
            "not_started",
        );
        assert_eq!(
            command(
                &tab,
                "read",
                json!({"css": ".fallback", "property": "visible", "all": true})
            )
            .await?["values"],
            json!([false, true])
        );
        let scope =
            command(&tab, "find", json!({"css": "#scope"})).await?["matches"][0]["ref"].clone();
        assert_eq!(
            command(
                &tab,
                "find",
                json!({"role": "button", "name": "Scoped", "within": scope})
            )
            .await?["count"],
            1
        );
        Ok(())
    })
    .await;
}

#[tokio::test]
#[ignore = "round two: requires DEMI_TEST_CHROME; launches a real browser"]
async fn explicit_navigation_tracks_documents_failures_and_history_boundaries() {
    with_fixture(|browser, base| async move {
        let tab = browser
            .open(&base, &CancellationToken::new(), TIMEOUT)
            .await?;
        command(&tab, "goto", json!({"url": format!("{base}/500")})).await?;
        assert_eq!(
            tab.read_only("document.title", &CancellationToken::new(), TIMEOUT)
                .await?,
            "HTTP error document"
        );
        command(&tab, "goto", json!({"url": format!("{base}/redirect")})).await?;
        assert_eq!(
            tab.read_only("location.pathname", &CancellationToken::new(), TIMEOUT)
                .await?,
            "/destination"
        );
        command(
            &tab,
            "goto",
            json!({"url": format!("{base}/destination#changed")}),
        )
        .await?;
        command(&tab, "back", json!({})).await?;
        assert_eq!(
            tab.read_only("location.hash", &CancellationToken::new(), TIMEOUT)
                .await?,
            ""
        );
        command(&tab, "forward", json!({})).await?;
        assert_eq!(
            tab.read_only("location.hash", &CancellationToken::new(), TIMEOUT)
                .await?,
            "#changed"
        );
        for load in ["commit", "domcontentloaded", "load"] {
            command(&tab, "goto", json!({"url": base, "load": load})).await?;
            command(&tab, "reload", json!({"load": load})).await?;
        }
        let reserved = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let refused = format!("http://{}/", reserved.local_addr().unwrap());
        drop(reserved);
        let failed = error(
            command(&tab, "goto", json!({"url": refused})).await,
            "navigation_failed",
            "completed",
        );
        assert!(failed.to_string().contains("ERR_CONNECTION_REFUSED"));
        error(
            command(&tab, "goto", json!({"url": format!("{base}/drop")})).await,
            "navigation_failed",
            "completed",
        );
        command(&tab, "goto", json!({"url": format!("{base}/reload-drop")})).await?;
        error(
            command(&tab, "reload", json!({})).await,
            "navigation_failed",
            "completed",
        );
        let boundary = browser
            .open("about:blank", &CancellationToken::new(), TIMEOUT)
            .await?;
        error(
            command(&boundary, "back", json!({})).await,
            "history_boundary",
            "not_started",
        );
        error(
            command(&boundary, "forward", json!({})).await,
            "history_boundary",
            "not_started",
        );
        assert_eq!(
            command(&boundary, "history", json!({})).await?["entries"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let failed_open = browser
            .open(
                &format!("{base}/stall"),
                &CancellationToken::new(),
                Duration::from_millis(700),
            )
            .await;
        let failed_open = match failed_open {
            Ok(_) => panic!("stalled open succeeded"),
            Err(error) => error,
        };
        assert_eq!(failed_open.code(), "timeout");
        assert!(failed_open.details()["tab"].as_str().is_some());
        Ok(())
    })
    .await;
}

#[tokio::test]
#[ignore = "round two: requires DEMI_TEST_CHROME; launches a real browser"]
async fn url_observation_delivers_input_and_observes_transient_matches() {
    with_fixture(|browser, base| async move {
        let tab = browser
            .open(&base, &CancellationToken::new(), TIMEOUT)
            .await?;
        command(&tab, "wait", json!({"url": format!("{base}/")})).await?;
        command(
            &tab,
            "click",
            json!({"css": "#no-nav", "wait-url": format!("{base}/")}),
        )
        .await?;
        assert_eq!(
            tab.read_only("noNavClicks", &CancellationToken::new(), TIMEOUT)
                .await?,
            1
        );
        click(&tab, "#no-nav").await?;
        assert_eq!(
            tab.read_only("noNavClicks", &CancellationToken::new(), TIMEOUT)
                .await?,
            2
        );
        for css in ["#immediate", "#delayed"] {
            command(
                &tab,
                "click",
                json!({"css": css, "wait-url": "**/destination"}),
            )
            .await?;
            command(&tab, "goto", json!({"url": base})).await?;
        }
        command(
            &tab,
            "click",
            json!({"css": "#transient", "wait-url": "**/transient"}),
        )
        .await?;
        assert_eq!(
            tab.read_only("location.pathname", &CancellationToken::new(), TIMEOUT)
                .await?,
            "/returned"
        );
        command(&tab, "goto", json!({"url": base})).await?;
        let failed = error(
            command(
                &tab,
                "click",
                json!({"css": "#no-nav", "wait-url": "**/never", "timeout": 700}),
            )
            .await,
            "timeout",
            "completed",
        );
        assert_eq!(failed.details()["tab"], tab.id());
        assert!(failed.details()["url"].as_str().unwrap().starts_with(&base));
        command(
            &tab,
            "key",
            json!({"css": "#hash", "key": "Enter", "wait-url": "**/#changed"}),
        )
        .await?;
        command(&tab, "goto", json!({"url": format!("{base}/a/b/c")})).await?;
        error(
            command(
                &tab,
                "wait",
                json!({"url": format!("{base}/a/*"), "timeout": 500}),
            )
            .await,
            "timeout",
            "not_started",
        );
        command(&tab, "wait", json!({"url": format!("{base}/a/**")})).await?;
        command(&tab, "goto", json!({"url": format!("{base}/a/?")})).await?;
        command(&tab, "wait", json!({"url": format!("{base}/a/?")})).await?;
        command(&tab, "goto", json!({"url": format!("{base}/a/x")})).await?;
        error(
            command(
                &tab,
                "wait",
                json!({"url": format!("{base}/a/?"), "timeout": 500}),
            )
            .await,
            "timeout",
            "not_started",
        );
        command(&tab, "goto", json!({"url": base})).await?;
        error(
            command(&tab, "click", json!({"css": "#stall", "timeout": 700})).await,
            "timeout",
            "completed",
        );
        Ok(())
    })
    .await;
}

#[tokio::test]
#[ignore = "round two: requires DEMI_TEST_CHROME; launches a real browser"]
async fn inspect_keeps_false_values_and_protects_passwords_and_handles_expire() {
    let retained = Arc::new(std::sync::Mutex::new(None));
    let saved = retained.clone();
    with_fixture(|browser, base| async move {
        let tab = browser
            .open(&base, &CancellationToken::new(), TIMEOUT)
            .await?;
        assert!(
            regex::Regex::new(r"^t_[A-Za-z0-9_-]{22}$")
                .unwrap()
                .is_match(&tab.id())
        );
        let tree = command(&tab, "inspect", json!({"limit": 1000})).await?;
        let nodes = tree["tree"].as_array().unwrap();
        let text = nodes
            .iter()
            .find(|node| node["name"] == "Text field")
            .unwrap();
        assert_eq!(text["value"], "hello");
        let password = nodes
            .iter()
            .find(|node| node["name"] == "Password")
            .unwrap();
        assert!(password.get("value").is_none());
        assert!(
            password["states"]
                .as_array()
                .unwrap()
                .contains(&json!("protected"))
        );
        assert!(nodes.iter().any(|node| {
            node["states"]
                .as_array()
                .is_some_and(|states| states.contains(&json!("checked=false")))
        }));
        assert!(!tree.to_string().contains("never-print-this-password"));
        error(
            command(
                &tab,
                "read",
                json!({"css": "#password", "property": "value"}),
            )
            .await,
            "protected_value",
            "not_started",
        );
        let reference = text["ref"].as_str().unwrap().to_owned();
        assert!(
            regex::Regex::new(r"^e_[A-Za-z0-9_-]{22}$")
                .unwrap()
                .is_match(&reference)
        );
        let repeated = command(&tab, "find", json!({"css": "#text"})).await?;
        assert_eq!(repeated["matches"][0]["ref"], reference);
        click(&tab, "#no-nav").await?;
        command(
            &tab,
            "fill",
            json!({"ref": reference, "text": "survives DOM update"}),
        )
        .await?;
        command(&tab, "reload", json!({})).await?;
        error(
            command(&tab, "fill", json!({"ref": reference, "text": "stale"})).await,
            "stale_ref",
            "not_started",
        );
        error(
            command(&tab, "wait", json!({"ref": reference, "state": "hidden"})).await,
            "stale_ref",
            "not_started",
        );
        *saved.lock().unwrap() = Some((tab.id(), reference));
        Ok(())
    })
    .await;
    let (old_tab, old_ref) = retained.lock().unwrap().take().unwrap();
    with_fixture(|browser, base| async move {
        let tab = browser
            .open(&base, &CancellationToken::new(), TIMEOUT)
            .await?;
        assert_ne!(tab.id(), old_tab);
        assert!(
            browser
                .tabs(&CancellationToken::new(), TIMEOUT)
                .await?
                .iter()
                .all(|tab| tab.id() != old_tab)
        );
        error(
            command(&tab, "fill", json!({"ref": old_ref, "text": "stale"})).await,
            "stale_ref",
            "not_started",
        );
        let failed = tab
            .execute("info", json!({"tab": old_tab}), &CancellationToken::new())
            .await
            .unwrap_err();
        assert_eq!(failed.code(), "tab_not_found");
        Ok(())
    })
    .await;
}

#[tokio::test]
#[ignore = "round two: requires DEMI_TEST_CHROME; launches a real browser"]
async fn input_and_animation_probes_clean_up_on_cancellation_and_dialogs() {
    with_fixture(|browser, base| async move {
        let tab = browser.open(&base, &CancellationToken::new(), TIMEOUT).await?;
        let cancel = CancellationToken::new();
        let (typing, ()) = tokio::join!(
            tab.execute("type", json!({"tab": tab.id(), "css": "#text", "text": "x".repeat(10_000), "timeout": 10_000}), &cancel),
            async {
                tokio::time::sleep(Duration::from_secs(1)).await;
                cancel.cancel();
            }
        );
        assert_eq!(typing.unwrap_err().code(), "cancelled");
        let events = tab.read_only("keys", &CancellationToken::new(), TIMEOUT).await?;
        let events = events.as_array().unwrap();
        assert!(!events.is_empty());
        assert_eq!(events.iter().filter(|event| event["type"] == "keydown").count(), events.iter().filter(|event| event["type"] == "keyup").count());
        let dialog = command(&tab, "key", json!({"css": "#key-dialog", "key": "Control+x"})).await.unwrap_err();
        assert_eq!(dialog.code(), "dialog_blocked");
        command(&tab, "dialog.dismiss", json!({})).await?;
        let events = tab.read_only("keys", &CancellationToken::new(), TIMEOUT).await?;
        let events = events.as_array().unwrap();
        assert_eq!(events.iter().filter(|event| event["type"] == "keydown" && event["key"] == "Control").count(), events.iter().filter(|event| event["type"] == "keyup" && event["key"] == "Control").count());
        error(command(&tab, "click", json!({"css": "#moving", "timeout": 500})).await, "not_actionable", "not_started");
        assert_eq!(tab.read_only("Object.getOwnPropertyNames(document.querySelector('#moving')).filter(name => name.startsWith('probe_'))", &CancellationToken::new(), TIMEOUT).await?, json!([]));
        Ok(())
    }).await;
}
