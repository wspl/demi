//! Round-one regression specifications. These compile without launching Chrome.

use demi_commands::browser::{
    BrowserEnvironment, BrowserError, BrowserTab, LaunchOptions, Result, with_browser,
};
use serde_json::{Value, json};
use std::{future::Future, path::PathBuf, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

#[path = "browser/server.rs"]
mod browser_server;

const TIMEOUT: Duration = Duration::from_secs(10);

/// Serve deterministic browser fixtures, including failures before HTTP headers.
async fn with_fixture<F, W>(exercise: F)
where
    F: FnOnce(BrowserEnvironment, String) -> W,
    W: Future<Output = Result<()>>,
{
    let executable = PathBuf::from(std::env::var_os("DEMI_TEST_CHROME").expect("DEMI_TEST_CHROME"));
    let server = browser_server::Server::start(include_str!("browser/repairs.html")).await;
    let base = server.base.clone();
    let result = with_browser(
        LaunchOptions { executable },
        CancellationToken::new(),
        |browser| exercise(browser, base),
    )
    .await;
    server.close().await;
    assert!(result.is_ok(), "{result:?}");
}

/// Invoke the production command parser, tab gate, algorithms and result schema.
async fn command(tab: &BrowserTab, operation: &str, mut args: Value) -> Result<Value> {
    args["tab"] = json!(tab.id());
    if args.get("timeout").is_none() {
        args["timeout"] = json!(10_000);
    }
    let result = tab
        .execute(operation, args.clone(), &CancellationToken::new())
        .await;
    if let Err(error) = &result {
        eprintln!("browser {operation} {args}: {error:?}");
    }
    result
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
            ("#datetime", "2026-09-28T14:35"),
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
            ("#datetime", "2026-13-28T14:35"),
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
            let property = if css == "#editable" {
                "text-content"
            } else {
                "value"
            };
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
            .await
            .expect("read the keyboard event log expression: keys");
        let events = events.as_array().unwrap();
        assert_eq!(
            events.len(),
            28,
            "each delivered key has exactly one down/up pair"
        );
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
        for (css, condition) in [
            ("#fieldset-button", "enabled"),
            ("#pointer-none", "pointer-events"),
            ("#moving", "stable"),
        ] {
            let failure = error(
                command(&tab, "click", json!({"css": css, "timeout": 500})).await,
                "not_actionable",
                "not_started",
            );
            assert_eq!(failure.details()["condition"], condition);
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
        let date =
            command(&tab, "find", json!({"css": "#date"})).await?["matches"][0]["ref"].clone();
        let segments = command(&tab, "find", json!({"role": "spinbutton", "within": date})).await?;
        // Native segment names follow Chrome's locale. The fixture's year is
        // 2026, while its month/day are 9 and 15, so select that observed value.
        let reference = segments["matches"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["value"].as_f64() == Some(2026.0))
            .and_then(|node| node["ref"].as_str())
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
                json!({"role": "button", "name": "AX fallback", "exact": true})
            )
            .await?["count"],
            2
        );
        assert_eq!(
            command(
                &tab,
                "read",
                json!({"css": ".ax-fallback", "property": "visible", "all": true})
            )
            .await?["values"],
            json!([false, true])
        );
        command(
            &tab,
            "click",
            json!({"role": "button", "name": "AX fallback", "exact": true}),
        )
        .await?;
        assert_eq!(
            tab.read_only("fallbackClicks", &CancellationToken::new(), TIMEOUT)
                .await?,
            1
        );
        assert_eq!(
            command(
                &tab,
                "find",
                json!({"role":"button", "name":"Fallback", "exact":true})
            )
            .await?["count"],
            1
        );
        assert_eq!(
            command(
                &tab,
                "find",
                json!({"role":"button", "name":"Fallback", "exact":true, "nth":1})
            )
            .await?["count"],
            0
        );
        assert_eq!(
            command(
                &tab,
                "find",
                json!({"role":"button", "name":"Hidden duplicate", "exact":true})
            )
            .await?["count"],
            0
        );
        click(&tab, ".fallback").await?;
        assert_eq!(
            tab.read_only("fallbackClicks", &CancellationToken::new(), TIMEOUT)
                .await?,
            2
        );
        for css in [".ambiguous", ".all-hidden"] {
            error(
                command(&tab, "click", json!({"css": css})).await,
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
        let mut nodes: Vec<&Value> = tree["tree"].as_array().unwrap().iter().collect();
        let mut index = 0;
        while index < nodes.len() {
            if let Some(children) = nodes[index]["children"].as_array() {
                nodes.extend(children);
            }
            index += 1;
        }
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

#[tokio::test]
#[ignore = "round two: requires DEMI_TEST_CHROME; launches a real browser"]
async fn text_locators_scan_large_documents_shadow_roots_and_frames() {
    with_fixture(|browser, base| async move {
        let tab = browser
            .open(&format!("{base}/many"), &CancellationToken::new(), TIMEOUT)
            .await?;
        for label in [
            "Appointment date",
            "Open shadow label",
            "Frame review label",
        ] {
            let found = command(&tab, "find", json!({"label": label, "exact": true})).await?;
            assert_eq!(found["count"], 1);
            command(
                &tab,
                "fill",
                json!({"ref": found["matches"][0]["ref"], "text": "2026-10-01"}),
            )
            .await?;
        }
        for text in ["Visible words", "Open shadow text", "Frame button"] {
            let found = command(&tab, "find", json!({"text-match": text, "exact": true})).await?;
            assert_eq!(found["count"], 1);
        }
        let scope =
            command(&tab, "find", json!({"css": "#open-shadow"})).await?["matches"][0]["ref"]
                .clone();
        let scoped = command(
            &tab,
            "find",
            json!({"label": "Open shadow label", "within": scope}),
        )
        .await?;
        assert_eq!(scoped["count"], 1);
        Ok(())
    })
    .await;
}

#[tokio::test]
#[ignore = "round two: requires DEMI_TEST_CHROME; launches a real browser"]
async fn unregistered_popup_survives_its_opener_closing() {
    with_fixture(|browser, base| async move {
        let opener = browser
            .open(&base, &CancellationToken::new(), TIMEOUT)
            .await?;
        // Execute directly on the tab so the environment has not reconciled the popup yet.
        click(&opener, "#open-popup").await?;
        opener.close(&CancellationToken::new(), TIMEOUT).await?;
        let tabs = browser.tabs(&CancellationToken::new(), TIMEOUT).await?;
        assert_eq!(tabs.len(), 1);
        assert_ne!(tabs[0].id(), opener.id());
        assert_eq!(
            command(&tabs[0], "info", json!({})).await?["url"],
            "about:blank"
        );
        let repeated = browser.tabs(&CancellationToken::new(), TIMEOUT).await?;
        assert_eq!(repeated[0].id(), tabs[0].id());
        Ok(())
    })
    .await;
}

#[tokio::test]
#[ignore = "requires DEMI_TEST_CHROME; launches a real browser"]
async fn catalog_queries_patterns_pagination_and_read_only_elements() {
    with_fixture(|browser, base| async move {
        let tab = browser.open(&base, &CancellationToken::new(), TIMEOUT).await?;
        let matches = command(&tab, "find", json!({"role": "button", "name-pattern": "^Delete$", "offset": 1, "limit": 1})).await?;
        assert_eq!(matches["count"], 2);
        assert_eq!(matches["matches"].as_array().unwrap().len(), 1);
        assert_eq!(matches["truncated"], false);
        let query = json!({"within": {"match": {"css": ".catalog-row"}, "hasText": "Order A"}, "match": {"role": "button", "name": "Delete", "exact": true}, "visible": true});
        let found = command(&tab, "find", json!({"query": true, "body": query.to_string()})).await?;
        assert_eq!(found["count"], 1);
        let reference = found["matches"][0]["ref"].as_str().unwrap();
        let value = command(&tab, "eval", json!({"ref": reference, "expression": "element.parentElement.innerText"})).await?;
        assert!(value["value"].as_str().unwrap().contains("Order A"));
        let root = command(&tab, "inspect", json!({})).await?["tree"][0]["ref"].clone();
        assert_eq!(command(&tab, "eval", json!({"ref":root,"expression":"document === element"})).await?["value"], true);
        let all = command(&tab, "eval", json!({"css": ".catalog-delete", "all": true, "expression": "elements.map(element => element.textContent)"})).await?;
        assert_eq!(all["value"], json!(["Delete", "Delete"]));
        error(command(&tab, "eval", json!({"ref": reference, "expression": "element.textContent = 'mutated'"})).await, "side_effect_rejected", "not_started");
        for invalid in [json!({"match":{"css":"button"},"or":[{"match":{"css":"button"}}]}), json!({"match":{"css":"button"},"bogus":true}), json!({"or":[]}), json!({"match":{"css":"button","role":"button"}})] {
            error(command(&tab, "find", json!({"query":true,"body":invalid.to_string()})).await,"invalid_input","not_started");
        }
        for args in [json!({"role":"button","name-pattern":"["}), json!({"text-pattern":"["}), json!({"css":"["})] {
            error(command(&tab, "find", args).await, "invalid_input", "not_started");
        }
        let html = command(&tab,"read",json!({"css":"#named","property":"html"})).await?;
        assert!(html["value"].as_str().unwrap().starts_with("<button"));
        command(&tab,"wait",json!({"load":"load"})).await?;
        Ok(())
    }).await;
}

#[tokio::test]
#[ignore = "requires DEMI_TEST_CHROME; launches a real browser"]
async fn catalog_untargeted_keys_selection_drag_and_console_cursors() {
    with_fixture(|browser, base| async move {
        let tab = browser.open(&base, &CancellationToken::new(), TIMEOUT).await?;
        click(&tab,"#select-middle").await?;
        let typed = command(&tab,"type",json!({"text":"XY"})).await?;
        assert!(typed["target"].as_str().unwrap().starts_with("e_"));
        assert_eq!(value(&tab,"#text").await?, json!("heXYo"));
        command(&tab,"key",json!({"key":"ControlOrMeta+A"})).await?;
        command(&tab,"type",json!({"text":"hello"})).await?;
        command(&tab,"select-text",json!({"css":"#text","text":"ll"})).await?;
        command(&tab,"type",json!({"text":"XY"})).await?;
        assert_eq!(value(&tab,"#text").await?, json!("heXYo"));
        command(&tab,"select-text",json!({"css":"#text","text":"XY","cursor":"after"})).await?;
        command(&tab,"type",json!({"text":"!"})).await?;
        assert_eq!(value(&tab,"#text").await?, json!("heXY!o"));
        error(command(&tab,"select-text",json!({"css":"#repeated-text","text":"word"})).await,"ambiguous_target","not_started");
        command(&tab,"select-text",json!({"css":"#repeated-text","text":"word","prefix":"second ","suffix":"!"})).await?;
        assert_eq!(command(&tab,"eval",json!({"expression":"getSelection().toString()"})).await?["value"],"word");
        click(&tab,"#focus-document").await?;
        assert_eq!(command(&tab,"key",json!({"key":"Escape"})).await?["target"],"document");
        command(&tab, "key", json!({"css":"#key-navigate","key":"Shift"})).await?;
        let navigation = command(&tab, "key", json!({"key":"Enter","wait-url":"**/#key-focus"})).await?;
        assert!(navigation["url"].as_str().unwrap().ends_with("/#key-focus"));
        assert!(navigation["target"].as_str().unwrap().starts_with("e_"));
        command(&tab,"move",json!({"css":"#drag-area"})).await?;
        let rect = command(&tab,"eval",json!({"css":"#drag-area","expression":"({x:element.getBoundingClientRect().x,y:element.getBoundingClientRect().y})"})).await?["value"].clone();
        let x=rect["x"].as_f64().unwrap()+10.0;
        let y=rect["y"].as_f64().unwrap()+10.0;
        command(&tab,"drag",json!({"point":[format!("{x},{y}"),format!("{},{}",x+30.0,y+20.0)],"modifier":["Shift"]})).await?;
        let events=command(&tab,"eval",json!({"expression":"dragEvents"})).await?["value"].clone();
        assert_eq!(events[0]["type"],"mousedown");
        assert_eq!(events[0]["shift"],true);
        assert_eq!(events.as_array().unwrap().last().unwrap()["buttons"],0);
        assert_eq!(events.as_array().unwrap().last().unwrap()["shift"],true);
        click(&tab,"#console-burst").await?;
        let logs=command(&tab,"logs",json!({"limit":2,"filter":"catalog"})).await?;
        assert_eq!(logs["entries"].as_array().unwrap().len(),2);
        assert_eq!(logs["truncated"],true);
        assert_eq!(logs["entries"][1]["level"],"error");
        let again=command(&tab,"logs",json!({"limit":2,"filter":"catalog"})).await?;
        assert_eq!(logs,again);
        let empty=command(&tab,"logs",json!({"after":logs["cursor"]})).await?;
        assert_eq!(empty["entries"],json!([]));
        error(command(&tab,"logs",json!({"after":"foreign:0"})).await,"stale_cursor","not_started");
        Ok(())
    }).await;
}

#[tokio::test]
#[ignore = "requires DEMI_TEST_CHROME; launches a real browser"]
async fn catalog_cross_origin_frames_scope_focus_and_references() {
    with_fixture(|browser, base| async move {
        let tab = browser.open(&format!("{base}/catalog-frame"), &CancellationToken::new(), TIMEOUT).await?;
        command(&tab,"wait",json!({"load":"load"})).await?;
        let outer = command(&tab,"find",json!({"css":"#cross-frame"})).await?["matches"][0]["ref"].as_str().unwrap().to_owned();
        let input = command(&tab,"find",json!({"frame":[outer],"label":"Cross frame input"})).await?["matches"][0]["ref"].as_str().unwrap().to_owned();
        command(&tab,"fill",json!({"ref":input,"text":"cross"})).await?;
        assert_eq!(command(&tab,"read",json!({"ref":input,"property":"value"})).await?["value"],"cross");
        let typed=command(&tab,"type",json!({"text":"-focus"})).await?;
        assert_eq!(typed["target"],input);
        assert_eq!(command(&tab,"read",json!({"ref":input,"property":"value"})).await?["value"],"cross-focus");
        assert_eq!(command(&tab,"eval",json!({"ref":input,"expression":"element.value"})).await?["value"],"cross-focus");
        command(&tab,"click",json!({"frame":[outer],"css":"#cross-button"})).await?;
        assert_eq!(command(&tab,"read",json!({"frame":[outer],"css":"#cross-button","property":"text"})).await?["value"],"Cross clicked");
        let logs = command(&tab, "logs", json!({"filter":"cross-frame console","level":["info"]})).await?;
        assert_eq!(logs["entries"].as_array().unwrap().len(), 1);
        click(&tab, "#toggle-frame").await?;
        assert_eq!(command(&tab, "read", json!({"ref":input,"property":"visible"})).await?["value"], false);
        command(&tab, "wait", json!({"ref":input,"state":"hidden"})).await?;
        click(&tab, "#toggle-frame").await?;
        let nested=command(&tab,"find",json!({"frame":[outer],"css":"#nested"})).await?["matches"][0]["ref"].as_str().unwrap().to_owned();
        command(&tab,"fill",json!({"frame":[outer,nested],"label":"Nested input","text":"nested"})).await?;
        let query=json!({"frame":{"frame":{"match":{"css":"#cross-frame"}},"match":{"css":"#nested"}},"match":{"label":"Nested input"}});
        assert_eq!(command(&tab,"find",json!({"query":true,"body":query.to_string()})).await?["count"],1);
        let tree=command(&tab,"inspect",json!({"frame":[outer],"view":"dom"})).await?;
        assert!(tree.to_string().contains("cross-focus"));
        assert!(!tree.to_string().contains("Appointment date"));
        assert_eq!(browser.tabs(&CancellationToken::new(),TIMEOUT).await?.len(),1);
        click(&tab, "#replace-frame").await?;
        command(&tab, "wait", json!({"text-match":"Frame ready","exact":true})).await?;
        error(command(&tab,"read",json!({"ref":input,"property":"value"})).await,"stale_ref","not_started");
        let input = command(&tab,"find",json!({"frame":[outer],"label":"Cross frame input"})).await?["matches"][0]["ref"].as_str().unwrap().to_owned();
        command(&tab,"reload",json!({})).await?;
        error(command(&tab,"read",json!({"ref":input,"property":"value"})).await,"stale_ref","not_started");
        Ok(())
    }).await;
}

#[tokio::test]
#[ignore = "requires DEMI_TEST_CHROME; launches a real browser"]
async fn catalog_probe_and_server_recorded_native_form_state() {
    with_fixture(|browser, base| async move {
        let tab = browser.open(&base,&CancellationToken::new(),TIMEOUT).await?;
        command(&tab,"move",json!({"css":"#named"})).await?;
        let rect=command(&tab,"eval",json!({"css":"#named","expression":"({x:element.getBoundingClientRect().x+5,y:element.getBoundingClientRect().y+5})"})).await?["value"].clone();
        let xy=format!("{},{}",rect["x"],rect["y"]);
        let probe=command(&tab,"probe",json!({"xy":xy})).await?;
        assert!(probe["matches"].as_array().unwrap().iter().any(|node| node["name"]=="Accessible label" && node["bounds"]["width"].as_f64().unwrap()>0.0));
        let ordinary=command(&tab,"probe",json!({"xy":xy,"include-non-interactable":true})).await?;
        assert!(ordinary["matches"].as_array().unwrap().len()>probe["matches"].as_array().unwrap().len());
        error(command(&tab,"probe",json!({"xy":"-1,0"})).await,"invalid_input","not_started");
        for date in ["2026-10-01","2026-10-02"] {
            assert_eq!(command(&tab,"read",json!({"css":"[name=confirm]","property":"checked"})).await?["value"],false);
            command(&tab,"fill",json!({"label":"Submission date","text":date})).await?;
            command(&tab,"check",json!({"css":"[name=confirm]","value":true})).await?;
            click(&tab,"#submit-final").await?;
        }
        let state=reqwest::get(format!("{base}/submitted-state")).await.unwrap().text().await.unwrap();
        let submissions:Vec<String>=serde_json::from_str(&state).unwrap();
        assert_eq!(submissions,vec!["/submit?date=2026-10-01&confirm=on","/submit?date=2026-10-02&confirm=on"]);
        Ok(())
    }).await;
}

#[tokio::test]
#[ignore = "requires DEMI_TEST_CHROME; launches a real browser"]
async fn catalog_load_wait_tracks_only_the_current_document() {
    with_fixture(|browser, base| async move {
        let tab = browser
            .open(
                &format!("{base}/load-replaced"),
                &CancellationToken::new(),
                TIMEOUT,
            )
            .await?;
        command(&tab, "wait", json!({"load":"commit"})).await?;
        command(&tab, "wait", json!({"load":"domcontentloaded"})).await?;
        error(
            command(&tab, "wait", json!({"load":"load"})).await,
            "navigation_failed",
            "not_started",
        );
        command(&tab, "wait", json!({"load":"load"})).await?;
        Ok(())
    })
    .await;
}
