//! Round-one regression specifications. These compile without launching Chrome.

use demi_command_package_browser_chrome::driver::operation::{BrowserError, Result};
use demi_command_package_browser_chrome::page::evaluation;
use demi_command_package_browser_chrome::tabs::tab::BrowserTab;
use serde_json::{Value, json};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

use crate::fixture::with_fixture;

const TIMEOUT: Duration = Duration::from_secs(10);

/// Invoke the production command parser, tab gate, algorithms and result schema.
async fn command(tab: &BrowserTab, operation: &str, mut args: Value) -> Result<Value> {
    args["tab"] = json!(tab.id());
    if args.get("timeout").is_none() {
        args["timeout"] = json!(10_000);
    }
    let result = demi_command_package_browser_chrome::page::actions::execute(
        tab,
        operation,
        args.clone(),
        &CancellationToken::new(),
    )
    .await;
    if let Err(error) = &result {
        eprintln!("browser {operation} {args}: {error:?}");
    }
    result
}

async fn click(tab: &BrowserTab, css: &str) -> Result<()> {
    command(tab, "click", json!({"css": css})).await?;
    Ok(())
}

/// A failure's details as the JSON its output carries.
fn details(error: &BrowserError) -> Value {
    serde_json::to_value(error.details()).unwrap()
}

fn error(result: Result<Value>, code: &str, action: &str) -> BrowserError {
    let error = result.expect_err(code);
    assert_eq!(error.code().to_string(), code, "{error:?}");
    assert_eq!(details(&error)["action"], action, "{error:?}");
    error
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
                json!({"css": "#covered-button", "timeout": 5000}),
            )
            .await,
            "not_actionable",
            "not_started",
        );
        assert!(
            details(&covered)["interceptor"]
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
                command(&tab, "click", json!({"css": css, "timeout": 5000})).await,
                "not_actionable",
                "not_started",
            );
            assert_eq!(details(&failure)["condition"], condition);
        }
        command(&tab, "move", json!({"css": "#disabled-button"})).await?;
        assert_eq!(
            evaluation::evaluate(&tab, "hovered", &CancellationToken::new(), TIMEOUT).await?,
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
                    json!({"css": css, "text": "x", "timeout": 5000}),
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
async fn explicit_navigation_tracks_documents_failures_and_history_boundaries() {
    with_fixture(|browser, base| async move {
        let tab = browser
            .open(&base, &CancellationToken::new(), TIMEOUT)
            .await?;
        command(&tab, "goto", json!({"url": format!("{base}/500")})).await?;
        assert_eq!(
            evaluation::evaluate(&tab, "document.title", &CancellationToken::new(), TIMEOUT)
                .await?,
            "HTTP error document"
        );
        command(&tab, "goto", json!({"url": format!("{base}/redirect")})).await?;
        assert_eq!(
            evaluation::evaluate(
                &tab,
                "location.pathname",
                &CancellationToken::new(),
                TIMEOUT
            )
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
            evaluation::evaluate(&tab, "location.hash", &CancellationToken::new(), TIMEOUT).await?,
            ""
        );
        command(&tab, "forward", json!({})).await?;
        assert_eq!(
            evaluation::evaluate(&tab, "location.hash", &CancellationToken::new(), TIMEOUT).await?,
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
        assert_eq!(failed_open.code().to_string(), "timeout");
        assert!(details(&failed_open)["tab"].as_str().is_some());
        Ok(())
    })
    .await;
}

/// A click or a key press waits for the navigation it starts, as Playwright's
/// click and press do, and for no clock: a page that navigates once its click
/// handler was told to, after it told the test the click arrived, is the
/// next command's to wait for.
#[tokio::test]
#[ignore = "round two: requires DEMI_TEST_CHROME; launches a real browser"]
async fn a_click_or_a_key_waits_for_the_navigation_it_starts_and_no_other() {
    with_fixture(|browser, base| async move {
        let tab = browser
            .open(&base, &CancellationToken::new(), TIMEOUT)
            .await?;
        let clicked = command(&tab, "click", json!({"css": "#go-destination"})).await?;
        assert_eq!(clicked["url"], format!("{base}/destination"));
        assert_eq!(
            command(&tab, "read", json!({"css": "h1", "property": "text"})).await?["value"],
            "Arrived"
        );
        command(&tab, "goto", json!({"url": base})).await?;
        let pressed = command(
            &tab,
            "key",
            json!({"css": "#go-destination", "key": "Enter"}),
        )
        .await?;
        assert_eq!(pressed["url"], format!("{base}/destination"));
        assert_eq!(
            command(&tab, "read", json!({"css": "h1", "property": "text"})).await?["value"],
            "Arrived"
        );
        command(&tab, "goto", json!({"url": base})).await?;
        let (clicked, ()) = tokio::join!(
            command(&tab, "click", json!({"css": "#later-hash"})),
            async {
                reqwest::get(format!("{base}/wait-typing")).await.unwrap();
                reqwest::get(format!("{base}/release-load")).await.unwrap();
            }
        );
        assert_eq!(clicked?["url"], format!("{base}/"));
        command(&tab, "wait", json!({"url": "**#later"})).await?;
        Ok(())
    })
    .await;
}

/// A cover that appears over a button as the pointer reaches it, the way a
/// layout shift between the checks and the input does, takes none of the
/// click: the click's events are stopped before the page sees them, and the
/// click checks the button again and lands on it once the cover is gone.
#[tokio::test]
#[ignore = "round two: requires DEMI_TEST_CHROME; launches a real browser"]
async fn a_click_diverted_after_its_checks_is_checked_and_sent_again() {
    with_fixture(|browser, base| async move {
        let tab = browser
            .open(&base, &CancellationToken::new(), TIMEOUT)
            .await?;
        click(&tab, "#shifting").await?;
        assert_eq!(
            command(&tab, "read", json!({"css": "#shifting", "attribute": "data-clicks"})).await?
                ["value"],
            "1"
        );
        assert_eq!(
            evaluation::evaluate(&tab, "coverClicks", &CancellationToken::new(), TIMEOUT).await?,
            0
        );
        Ok(())
    })
    .await;
}

#[tokio::test]
#[ignore = "requires DEMI_TEST_CHROME; launches a real browser"]
async fn frame_pointer_actions_wait_for_composited_scroll() {
    with_fixture(|browser, base| async move {
        let tab = browser
            .open(
                &format!("{base}/catalog-frame"),
                &CancellationToken::new(),
                TIMEOUT,
            )
            .await?;
        command(&tab, "wait", json!({"load": "load"})).await?;
        let outer =
            command(&tab, "find", json!({"css": "#cross-frame"})).await?["matches"][0]["ref"]
                .clone();
        let viewport = evaluation::evaluate(
            &tab,
            "[innerWidth, innerHeight, devicePixelRatio]",
            &CancellationToken::new(),
            TIMEOUT,
        )
        .await?;
        for count in 1..=20 {
            // Centering these two controls moves their embedding page by 21px.
            // A stale browser-process transform presses the input, then releases
            // the button, even though each renderer reports stable DOM geometry.
            command(
                &tab,
                "fill",
                json!({"frame": [outer], "css": "#cross-input", "text": count.to_string()}),
            )
            .await?;
            command(
                &tab,
                "click",
                json!({"frame": [outer], "css": "#cross-button"}),
            )
            .await?;
            let clicked = command(
                &tab,
                "read",
                json!({"frame": [outer], "css": "#cross-button", "attribute": "data-clicks"}),
            )
            .await?;
            assert_eq!(
                clicked["value"],
                count.to_string(),
                "every press must click the button exactly once"
            );
        }
        assert_eq!(
            evaluation::evaluate(
                &tab,
                "[innerWidth, innerHeight, devicePixelRatio]",
                &CancellationToken::new(),
                TIMEOUT
            )
            .await?,
            viewport
        );
        Ok(())
    })
    .await;
}
