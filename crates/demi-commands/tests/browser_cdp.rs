mod browser_families;
use browser_families::with_browser_fixture;
use serde_json::json;
use tokio_util::sync::CancellationToken;

#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn cdp_validates_methods_scopes_children_and_preserves_event_cursors() {
    with_browser_fixture(|fixture|async move {
        let tab=fixture.open("cdp.html").await;
        for method in ["Browser.close","Target.createTarget","Page.close","SystemInfo.getInfo"] {
            let (_,error)=fixture.result("browser.cdp.send",json!({"tab":tab,"method":method,"params":"{}"}),CancellationToken::new()).await;
            assert_eq!(error["error"]["code"],"cdp_method_denied","{error}");
        }
        for (method,params) in [("Missing.command","{}"),("Runtime.evaluate","{}"),("Runtime.evaluate","{\"expression\":1}"),("Runtime.evaluate","{\"expression\":\"1\",\"sessionId\":\"external\"}")] {
            let (code,error)=fixture.result("browser.cdp.send",json!({"tab":tab,"method":method,"params":params}),CancellationToken::new()).await;
            assert_eq!(code,2,"{error}");
        }
        let targets=fixture.call("browser.cdp.targets",json!({"tab":tab})).await;
        assert!(targets["targets"].as_array().unwrap().iter().any(|target|target["id"]=="main"));
        let worker=targets["targets"].as_array().unwrap().iter().find(|target|target["kind"]=="worker").expect("worker belongs to tab")["id"].clone();
        let result=fixture.call("browser.cdp.send",json!({"tab":tab,"method":"Runtime.evaluate","params":"{\"expression\":\"self.answer\",\"returnByValue\":true}","target":worker})).await;
        assert_eq!(result["result"]["result"]["value"],42);
        fixture.call("browser.cdp.send",json!({"tab":tab,"method":"Runtime.enable","params":"{}"})).await;
        let initial=fixture.call("browser.cdp.events",json!({"tab":tab,"method":["Runtime.consoleAPICalled"]})).await;
        assert_eq!(initial["events"],json!([]));
        fixture.call("browser.cdp.send",json!({"tab":tab,"method":"Runtime.evaluate","params":"{\"expression\":\"console.log('first'); console.log('second')\"}"})).await;
        let first=fixture.call("browser.cdp.events",json!({"tab":tab,"method":["Runtime.consoleAPICalled"],"after":initial["cursor"],"limit":1})).await;
        assert_eq!(first["events"].as_array().unwrap().len(),1);
        assert_eq!(first["hasMore"],true);
        let second=fixture.call("browser.cdp.events",json!({"tab":tab,"method":["Runtime.consoleAPICalled"],"after":first["cursor"],"limit":1})).await;
        assert_eq!(second["events"].as_array().unwrap().len(),1);
        assert_eq!(second["hasMore"],false);
        assert!(second["events"][0]["sequence"].as_u64()>first["events"][0]["sequence"].as_u64());
        let (_,error)=fixture.result("browser.cdp.events",json!({"tab":tab,"after":"expired:0"}),CancellationToken::new()).await;
        assert_eq!(error["error"]["code"],"stale_cursor","{error}");
        let (_,error)=fixture.result("browser.cdp.send",json!({"tab":tab,"method":"Runtime.evaluate","params":"{\"expression\":\"1\"}","target":"external"}),CancellationToken::new()).await;
        assert_eq!(error["error"]["code"],"target_not_found","{error}");
        fixture
    }).await;
}

#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn cdp_wait_cancellation_retains_subscriptions_and_send_cancellation_releases_pause() {
    with_browser_fixture(|fixture| async move {
        let tab = fixture.open("cdp.html").await;
        fixture.call("browser.cdp.send", json!({"tab":tab,"method":"Runtime.enable","params":"{}"})).await;
        let before = fixture.call("browser.cdp.events", json!({"tab":tab})).await;
        let cancel = CancellationToken::new();
        let wait_cancel = cancel.clone();
        let waiting = fixture.clone();
        let request = json!({"tab":tab,"after":before["cursor"],"method":["Runtime.consoleAPICalled"],"timeout":30000});
        let task = tokio::spawn(async move { waiting.result("browser.cdp.events", request, wait_cancel).await });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        cancel.cancel();
        let (_, error) = task.await.unwrap();
        assert_eq!(error["error"]["code"], "cancelled", "{error}");
        fixture.call("browser.cdp.send", json!({"tab":tab,"method":"Runtime.evaluate","params":"{\"expression\":\"console.log('retained')\"}"})).await;
        let events = fixture.call("browser.cdp.events", json!({"tab":tab,"after":before["cursor"],"method":["Runtime.consoleAPICalled"]})).await;
        assert_eq!(events["events"].as_array().unwrap().len(), 1);
        fixture.call("browser.cdp.send", json!({"tab":tab,"method":"Debugger.enable","params":"{}"})).await;
        fixture.call("browser.cdp.send", json!({"tab":tab,"method":"Fetch.enable","params":"{}"})).await;
        let cancel = CancellationToken::new();
        let call_cancel = cancel.clone();
        let debugging = fixture.clone();
        let request = json!({"tab":tab,"method":"Runtime.evaluate","params":"{\"expression\":\"debugger; 42\",\"returnByValue\":true}","timeout":30000});
        let task = tokio::spawn(async move { debugging.result("browser.cdp.send", request, call_cancel).await });
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        cancel.cancel();
        let (_, error) = task.await.unwrap();
        assert_eq!(error["error"]["code"], "cancelled", "{error}");
        let value = fixture.call("browser.eval", json!({"tab":tab,"expression":"21*2","timeout":3000})).await;
        assert_eq!(value["value"], 42);
        let (_, stale) = fixture.result("browser.cdp.events", json!({"tab":tab,"after":before["cursor"]}), CancellationToken::new()).await;
        assert_eq!(stale["error"]["code"], "stale_cursor", "{stale}");
        fixture.call("browser.goto", json!({"tab":tab,"url":"about:blank","timeout":3000})).await;
        fixture
    }).await;
}

#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn cdp_eviction_marks_truncation_and_worker_handles_expire() {
    with_browser_fixture(|fixture| async move {
        let tab = fixture.open("cdp.html").await;
        let targets = fixture.call("browser.cdp.targets", json!({"tab":tab})).await;
        let worker = targets["targets"].as_array().unwrap().iter()
            .find(|target| target["kind"] == "worker").unwrap()["id"].clone();
        fixture.call("browser.cdp.send", json!({"tab":tab,"method":"Runtime.evaluate","params":json!({"expression":"window.open('about:blank'); true","userGesture":true}).to_string()})).await;
        let scoped = fixture.call("browser.cdp.targets", json!({"tab":tab})).await;
        assert_eq!(scoped["targets"].as_array().unwrap().len(), 2, "popup pages do not become child debug targets: {scoped}");
        let page = fixture.call("browser.cdp.targets", json!({"tab":tab,"offset":1,"limit":1})).await;
        assert_eq!(page["targets"].as_array().unwrap().len(), 1);
        assert_eq!(page["truncated"], false);
        fixture.call("browser.cdp.send", json!({"tab":tab,"method":"Runtime.enable","params":"{}"})).await;
        let initial = fixture.call("browser.cdp.events", json!({"tab":tab})).await;
        fixture.call("browser.cdp.send", json!({"tab":tab,"method":"Runtime.evaluate","params":json!({"expression":"for (let n=0; n<10020; n++) console.log(n); worker.terminate();"}).to_string()})).await;
        let events = fixture.call("browser.cdp.events", json!({"tab":tab,"after":initial["cursor"],"method":["Runtime.consoleAPICalled"],"limit":1})).await;
        assert_eq!(events["truncated"], true, "{events}");
        assert_eq!(events["hasMore"], true);
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let targets = fixture.call("browser.cdp.targets", json!({"tab":tab})).await;
                if !targets["targets"].as_array().unwrap().iter().any(|target| target["id"] == worker) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        }).await.unwrap();
        let (_, error) = fixture.result("browser.cdp.send", json!({"tab":tab,"target":worker,"method":"Runtime.evaluate","params":"{\"expression\":\"1\"}"}), CancellationToken::new()).await;
        assert_eq!(error["error"]["code"], "target_not_found", "{error}");
        fixture.call("browser.goto", json!({"tab":tab,"url":"about:blank"})).await;
        let targets = fixture.call("browser.cdp.targets", json!({"tab":tab})).await;
        assert_eq!(targets["targets"][0]["url"], "about:blank");
        fixture
    }).await;
}
