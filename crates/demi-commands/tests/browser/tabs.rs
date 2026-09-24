//! The tab registry through the service (`browser.md` § One tab registry).

use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::families::with_browser_fixture;

/// A command holding one tab does not hold up commands on another tab, and a
/// second command on the held tab reports it busy (`browser.md` § Required
/// checks 5).
#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn commands_on_another_tab_run_while_one_tab_is_held() {
    with_browser_fixture(|fixture| async move {
        let held = fixture.open("fixture.html").await;
        let other = fixture.open("fixture.html").await;
        let cancel = CancellationToken::new();
        let waiting = fixture.result(
            "browser.wait",
            json!({"tab": held, "url": "**/never", "timeout": 30000}),
            cancel.clone(),
        );
        let alongside = async {
            fixture.wait_until_busy(&held).await;
            let info = fixture.call("browser.info", json!({"tab": other})).await;
            assert_eq!(info["tab"], other);
            fixture
                .call("browser.inspect", json!({"tab": other}))
                .await;
            let tabs = fixture.call("browser.tabs", json!({})).await;
            let ids: Vec<_> = tabs["tabs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|tab| tab["id"].clone())
                .collect();
            assert_eq!(ids, [held.clone(), other.clone()]);
            // The wait still holds its tab: the commands above ran beside it.
            let (_, busy) = fixture
                .result("browser.info", json!({"tab": held}), CancellationToken::new())
                .await;
            assert_eq!(busy["error"]["code"], "tab_busy", "{busy}");
            cancel.cancel();
        };
        let ((code, _), ()) = tokio::join!(waiting, alongside);
        assert_eq!(code, 130);
        fixture
    })
    .await;
}

/// A click that opens a tab names it once the registry holds it, and a click
/// that opens none names nothing (`browser.md` § One tab registry).
#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn an_action_names_the_tabs_it_opened() {
    with_browser_fixture(|fixture| async move {
        let tab = fixture.open("popups.html").await;
        let plain = fixture
            .call("browser.click", json!({"tab": tab, "css": "#stay"}))
            .await;
        assert!(plain.get("openedTabs").is_none(), "{plain}");
        let clicked = fixture
            .call("browser.click", json!({"tab": tab, "css": "#open"}))
            .await;
        let opened = clicked["openedTabs"]
            .as_array()
            .unwrap_or_else(|| panic!("the click names the tab it opened: {clicked}"))
            .clone();
        assert_eq!(opened.len(), 1, "{clicked}");
        // The named tab is registered: it can be operated at once, and the
        // list says who opened it.
        let info = fixture
            .call("browser.info", json!({"tab": opened[0]}))
            .await;
        assert_eq!(info["url"], "about:blank");
        let tabs = fixture.call("browser.tabs", json!({})).await;
        let popup = tabs["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|listed| listed["id"] == opened[0])
            .unwrap_or_else(|| panic!("the list holds the popup: {tabs}"));
        assert_eq!(popup["createdBy"], json!({"kind": "page", "opener": tab}));
        // A later action does not name the tab again; the popup stays in front.
        let again = fixture
            .call("browser.click", json!({"tab": tab, "css": "#stay"}))
            .await;
        assert!(again.get("openedTabs").is_none(), "{again}");
        fixture
    })
    .await;
}

/// An action works in a tab that is not the front tab of its window: its
/// hidden document runs no animation frames (`browser.md` § Actionability
/// and coordinates).
#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn an_action_works_in_the_older_of_two_open_tabs() {
    with_browser_fixture(|fixture| async move {
        let older = fixture.open("popups.html").await;
        let newer = fixture.open("popups.html").await;
        let visibility = |tab: &str| {
            fixture.call(
                "browser.eval",
                json!({"tab": tab, "expression": "document.visibilityState"}),
            )
        };
        assert_eq!(visibility(&older).await["value"], "hidden");
        assert_eq!(visibility(&newer).await["value"], "visible");
        fixture
            .call("browser.click", json!({"tab": older, "css": "#stay"}))
            .await;
        let clicks = fixture
            .call("browser.eval", json!({"tab": older, "expression": "window.stays"}))
            .await;
        assert_eq!(clicks["value"], 1);
        // Neither tab came to the front.
        assert_eq!(visibility(&older).await["value"], "hidden");
        fixture
    })
    .await;
}
