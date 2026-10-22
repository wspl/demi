use crate::families::with_browser_fixture;
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
