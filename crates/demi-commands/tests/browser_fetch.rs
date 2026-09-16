mod browser_families;
#[path = "browser/server.rs"]
mod browser_server;
use browser_families::with_browser_fixture;
use serde_json::json;
use tokio_util::sync::CancellationToken;

#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn fetch_returns_input_order_and_releases_batch_tabs() {
    with_browser_fixture(|fixture| async move {
        let retained = fixture.open("upload.html").await;
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/browser");
        let urls: Vec<_> = ["assets.html", "download.html"]
            .into_iter()
            .map(|name| {
                url::Url::from_file_path(root.join(name))
                    .unwrap()
                    .to_string()
            })
            .collect();
        let result = fixture
            .call("browser.content.fetch", json!({"url":urls,"format":"html"}))
            .await;
        assert_eq!(result["pages"][0]["requestedUrl"], urls[0]);
        assert_eq!(result["pages"][1]["requestedUrl"], urls[1]);
        assert_eq!(result["pages"][0]["title"], "Assets fixture");
        assert!(
            result["pages"][1]["content"]
                .as_str()
                .unwrap()
                .contains("Delayed save")
        );
        let tabs = fixture.call("browser.tabs", json!({})).await;
        assert_eq!(tabs["tabs"].as_array().unwrap().len(), 1);
        assert_eq!(tabs["tabs"][0]["id"], retained);
        let result = fixture
            .call(
                "browser.content.fetch",
                json!({"url":[urls[0]],"format":"dom"}),
            )
            .await;
        assert!(!result["pages"][0]["content"].as_str().unwrap().is_empty());
        let (_, error) = fixture
            .result(
                "browser.content.fetch",
                json!({"url":[urls[0],"javascript:alert(1)"]}),
                CancellationToken::new(),
            )
            .await;
        assert_eq!(error["error"]["code"], "invalid_input", "{error}");
        assert_eq!(
            fixture.call("browser.tabs", json!({})).await["tabs"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        fixture
    })
    .await;
}

#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn fetch_closure_and_cancellation_release_registered_tabs() {
    let server = browser_server::Server::start(include_str!("browser/assets.html")).await;
    let base = server.base.clone();
    with_browser_fixture(|fixture| async move {
        let retained = fixture.open("upload.html").await;
        let collecting = fixture.clone();
        let urls = [format!("{base}/ready"), format!("{base}/stall")];
        let request = json!({"url":urls,"format":"text","timeout":30000});
        let task =
            tokio::spawn(async move { collecting.call("browser.content.fetch", request).await });
        let temporary = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let tabs = fixture.call("browser.tabs", json!({})).await;
                let rows = tabs["tabs"].as_array().unwrap();
                if rows.iter().any(|tab| tab["url"] == urls[0])
                    && let Some(tab) = rows
                        .iter()
                        .find(|tab| tab["id"] != retained && tab["url"] != urls[0])
                {
                    assert_eq!(tab["createdBy"]["kind"], "temporary");
                    break tab["id"].clone();
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("stalled fetch tab is registered and visible");
        fixture
            .call("browser.close", json!({"tab":temporary}))
            .await;
        let result = task.await.unwrap();
        assert_eq!(
            result["pages"][1]["error"]["code"], "tab_not_found",
            "{result}"
        );
        assert_eq!(result["pages"][0]["title"], "Assets fixture");
        let cancel = CancellationToken::new();
        let collecting = fixture.clone();
        let operation_cancel = cancel.clone();
        let task = tokio::spawn(async move {
            collecting
                .result(
                    "browser.content.fetch",
                    json!({"url":urls,"timeout":30000}),
                    operation_cancel,
                )
                .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        cancel.cancel();
        let (_, error) = task.await.unwrap();
        assert_eq!(error["error"]["code"], "cancelled", "{error}");
        let tabs = fixture.call("browser.tabs", json!({})).await;
        assert_eq!(tabs["tabs"].as_array().unwrap().len(), 1);
        assert_eq!(tabs["tabs"][0]["id"], retained);
        fixture
    })
    .await;
    server.close().await;
}
