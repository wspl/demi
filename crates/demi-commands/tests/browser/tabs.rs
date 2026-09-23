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
