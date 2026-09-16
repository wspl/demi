mod browser_families;

use browser_families::with_browser_fixture;
use serde_json::json;
use tokio_util::sync::CancellationToken;

#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn native_webmcp_validates_calls_and_invalidates_changed_declarations() {
    with_browser_fixture(|fixture| async move {
        let tab = fixture.open("webmcp.html").await;
        let list = fixture.call("browser.webmcp.list", json!({"tab":tab})).await;
        assert!(list["entries"].as_array().unwrap().iter().any(|entry| entry["name"] == "echo"));
        assert_eq!(fixture.call("browser.webmcp.list", json!({"tab":tab})).await["tools"], list["tools"]);
        let tools = list["tools"].clone();
        let result = fixture.call("browser.webmcp.call", json!({"tab":tab,"tools":tools,"tool":"echo","arguments":"{\"text\":\"hello\"}"})).await;
        assert_eq!(result["result"], json!({"echo":"hello"}));
        let (code, error) = fixture.result("browser.webmcp.call", json!({"tab":tab,"tools":tools,"tool":"echo","arguments":"{\"text\":42}"}), CancellationToken::new()).await;
        assert_eq!(code, 2, "{error}");
        assert_eq!(fixture.call("browser.read", json!({"tab":tab,"css":"#calls","property":"text"})).await["value"], "1");
        fixture.call("browser.click", json!({"tab":tab,"css":"#replace"})).await;
        let (_, error) = fixture.result("browser.webmcp.call", json!({"tab":tab,"tools":tools,"tool":"echo","arguments":"{\"text\":\"again\"}"}), CancellationToken::new()).await;
        assert_eq!(error["error"]["code"], "stale_tools", "{error}");
        let list = fixture.call("browser.webmcp.list", json!({"tab":tab})).await;
        let tools = list["tools"].clone();
        fixture.call("browser.reload", json!({"tab":tab})).await;
        let (_, error) = fixture.result("browser.webmcp.call", json!({"tab":tab,"tools":tools,"tool":"echo","arguments":"{\"text\":\"after navigation\"}"}), CancellationToken::new()).await;
        assert_eq!(error["error"]["code"], "stale_tools", "{error}");
        let list = fixture.call("browser.webmcp.list", json!({"tab":tab})).await;
        let cancel = CancellationToken::new();
        let stop = cancel.clone();
        let ((_, error), ()) = tokio::join!(
            fixture.result("browser.webmcp.call", json!({"tab":tab,"tools":list["tools"],"tool":"wait","arguments":"{}"}), cancel),
            async move { tokio::time::sleep(std::time::Duration::from_millis(300)).await; stop.cancel(); }
        );
        assert_eq!(error["error"]["code"], "cancelled", "{error}");
        assert_eq!(fixture.call("browser.read", json!({"tab":tab,"css":"#cancelled","property":"text"})).await["value"], "true");
        fixture
    }).await;
}
