use crate::families::with_browser_fixture;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn oversized_observation_ends_the_browser_and_allows_a_fresh_open() {
    with_browser_fixture(|fixture| async move {
        let tab = fixture.open("cdp.html").await;
        // A console message larger than a CDP message may be arrives as an
        // event the connection cannot read, which ends it (a larger answer
        // only fails its own request).
        // The connection reads messages of up to 64 MiB (chromiumoxide's `MAX_CDP_MESSAGE_BYTES`).
        let expression = format!("console.log('x'.repeat({})); undefined", 64 * 1024 * 1024);
        let (_, sent) = fixture
            .result(
                "browser.cdp.send",
                json!({"tab":tab,"method":"Runtime.evaluate","params":json!({"expression":expression}).to_string()}),
                CancellationToken::new(),
            )
            .await;
        let (_, error) = fixture
            .result("browser.info", json!({"tab":tab}), CancellationToken::new())
            .await;
        assert_eq!(error["error"]["code"], "browser_lost", "{sent} then {error}");
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let (code, opened) = fixture
                    .result(
                        "browser.open",
                        json!({"url":"about:blank"}),
                        CancellationToken::new(),
                    )
                    .await;
                if code == 0 {
                    assert_ne!(opened["tab"], tab);
                    break;
                }
                assert_eq!(opened["error"]["code"], "browser_lost", "{opened}");
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("an explicit open succeeds after browser-loss cleanup");
        fixture
    })
    .await;
}

#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn cdp_validates_methods_scopes_children_and_preserves_event_cursors() {
    with_browser_fixture(|fixture|async move {
        let tab=fixture.open("cdp.html").await;
        fixture.call("browser.wait", json!({"tab":tab,"css":"#worker-ready","state":"attached"})).await;
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
async fn cdp_wait_expiry_retains_subscriptions_and_invocation_cancellation_releases_connections() {
    with_browser_fixture(|fixture| async move {
        let tab = fixture.open("cdp.html").await;
        fixture.call("browser.cdp.send", json!({"tab":tab,"method":"Runtime.enable","params":"{}"})).await;
        let before = fixture.call("browser.cdp.events", json!({"tab":tab})).await;
        let empty = fixture.call("browser.cdp.events", json!({"tab":tab,"after":before["cursor"],"method":["Runtime.consoleAPICalled"],"timeout":100})).await;
        assert_eq!(empty["events"], json!([]));
        fixture.call("browser.cdp.send", json!({"tab":tab,"method":"Runtime.evaluate","params":"{\"expression\":\"console.log('retained')\"}"})).await;
        let events = fixture.call("browser.cdp.events", json!({"tab":tab,"after":before["cursor"],"method":["Runtime.consoleAPICalled"]})).await;
        assert_eq!(events["events"].as_array().unwrap().len(), 1);
        let cancel = CancellationToken::new();
        let wait_cancel = cancel.clone();
        let waiting = fixture.clone();
        let request = json!({"tab":tab,"after":events["cursor"],"method":["Runtime.consoleAPICalled"],"timeout":30000});
        let task = tokio::spawn(async move { waiting.result("browser.cdp.events", request, wait_cancel).await });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(!task.is_finished());
        cancel.cancel();
        let (_, error) = task.await.unwrap();
        assert_eq!(error["error"]["code"], "cancelled", "{error}");
        let (_, stale) = fixture.result("browser.cdp.events", json!({"tab":tab,"after":before["cursor"]}), CancellationToken::new()).await;
        assert_eq!(stale["error"]["code"], "stale_cursor", "{stale}");
        fixture.call("browser.cdp.send", json!({"tab":tab,"method":"Debugger.enable","params":"{}"})).await;
        fixture.call("browser.cdp.send", json!({"tab":tab,"method":"Fetch.enable","params":"{}"})).await;
        let cancel = CancellationToken::new();
        let call_cancel = cancel.clone();
        let debugging = fixture.clone();
        let request = json!({"tab":tab,"method":"Runtime.evaluate","params":"{\"expression\":\"debugger; 42\",\"returnByValue\":true}","timeout":30000});
        let task = tokio::spawn(async move { debugging.result("browser.cdp.send", request, call_cancel).await });
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(!task.is_finished());
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
        fixture.call("browser.wait", json!({"tab":tab,"css":"#worker-ready","state":"attached"})).await;
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

/// About 3.5 s here: Chrome starts and loads two local pages.
///
/// Planted defects this catches: a cookie expiry typed as a number that is
/// always there, which Chrome contradicts by writing null for an expiry JSON
/// cannot hold (±Inf, as the protocol says), so the browser connection could
/// not decode the event and lost the browser with every tab; and a catalog
/// that rejects that null, or requires the deprecated `sameParty` Chrome 153
/// no longer sends, either of which ended the debugging connection.
#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn a_cookie_expiry_chrome_writes_as_null_keeps_the_browser_and_reaches_cdp_events() {
    let server = crate::server::Server::start("<!doctype html><title>Before</title>").await;
    let url = format!("{}/unbounded-cookie", server.base);
    with_browser_fixture(|fixture| async move {
        let tab = fixture.open("cdp.html").await;
        fixture.call("browser.cdp.send", json!({"tab":tab,"method":"Network.enable","params":"{}"})).await;
        let before = fixture.call("browser.cdp.events", json!({"tab":tab})).await;
        fixture.call("browser.goto", json!({"tab":tab,"url":url})).await;

        // The browser's own connection read the event and goes on.
        let info = fixture.call("browser.info", json!({"tab":tab})).await;
        assert_eq!(info["title"], "Unbounded cookie", "{info}");

        // The debugging connection records it as Chrome wrote it.
        let events = fixture
            .call("browser.cdp.events", json!({"tab":tab,"after":before["cursor"],"method":["Network.responseReceivedExtraInfo"],"timeout":5000}))
            .await;
        let blocked = events["events"][0]["params"]["blockedCookies"][0].clone();
        assert_eq!(blocked["cookie"]["name"], "unbounded", "{events}");
        assert_eq!(blocked["cookie"]["expires"], Value::Null, "{events}");
        fixture
    })
    .await;
    server.close().await;
}

/// About 3 s here: Chrome starts, and a page sends 64 MiB to a binding.
///
/// Planted defect this catches: a debugging connection that ends and tells
/// its calls only that it ended, so the caller cannot tell a page that
/// overflowed it from a lost browser.
#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn an_ended_debugging_connection_says_why_to_its_calls() {
    with_browser_fixture(|fixture| async move {
        let tab = fixture.open("cdp.html").await;
        fixture.call("browser.cdp.send", json!({"tab":tab,"method":"Runtime.addBinding","params":"{\"name\":\"demiLarge\"}"})).await;
        // Only this debugging connection hears the binding; its event is over
        // the 64 MiB a CDP message may have, which ends the connection.
        let expression = format!("demiLarge('x'.repeat({})); 1", 64 * 1024 * 1024);
        let (code, pending) = fixture
            .result(
                "browser.cdp.send",
                json!({"tab":tab,"method":"Runtime.evaluate","params":json!({"expression":expression}).to_string()}),
                CancellationToken::new(),
            )
            .await;
        let reason = "and it answers no request";
        assert_eq!(code, 1, "{pending}");
        let message = pending["error"]["message"].as_str().unwrap_or_default();
        assert!(message.contains("tab debugging connection ended") && message.contains(reason), "{pending}");

        // A later call hears the same reason.
        let (_, later) = fixture
            .result("browser.cdp.events", json!({"tab":tab}), CancellationToken::new())
            .await;
        let message = later["error"]["message"].as_str().unwrap_or_default();
        assert!(message.contains("tab debugging connection ended") && message.contains(reason), "{later}");

        // The browser and its tab go on.
        let info = fixture.call("browser.info", json!({"tab":tab})).await;
        assert_eq!(info["tab"], tab, "{info}");
        fixture
    })
    .await;
}

#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn cdp_detach_releases_only_its_caller_and_timeouts_identify_other_debug_owners() {
    let server = crate::server::Server::start("<!doctype html><title>Unblocked</title>").await;
    let url = server.base.clone();
    with_browser_fixture(|mut first| async move {
        first.caller = 1;
        let mut second = first.clone();
        second.caller = 2;
        let tab = first.open("cdp.html").await;
        first.call("browser.cdp.send", json!({"tab":tab,"method":"Fetch.enable","params":"{}"})).await;
        second.call("browser.cdp.send", json!({"tab":tab,"method":"Runtime.enable","params":"{}"})).await;
        let before = second.call("browser.cdp.events", json!({"tab":tab})).await;
        let (_, timeout) = second.result("browser.goto", json!({"tab":tab,"url":url,"timeout":300}), CancellationToken::new()).await;
        assert_eq!(timeout["error"]["code"], "timeout", "{timeout}");
        assert_eq!(timeout["error"]["details"]["tab"], tab);
        assert_eq!(timeout["error"]["details"]["debuggingCallers"], json!([1]));
        let detached = first.call("browser.cdp.detach", json!({"tab":tab})).await;
        assert_eq!(detached["detached"], tab);
        assert_eq!(first.call("browser.cdp.detach", json!({"tab":tab})).await, detached);
        second.call("browser.goto", json!({"tab":tab,"url":url,"timeout":3000})).await;
        second.call("browser.cdp.send", json!({"tab":tab,"method":"Runtime.evaluate","params":"{\"expression\":\"console.log('still subscribed')\"}"})).await;
        let events = second.call("browser.cdp.events", json!({"tab":tab,"after":before["cursor"],"method":["Runtime.consoleAPICalled"]})).await;
        assert_eq!(events["events"].as_array().unwrap().len(), 1);
        // A timeout with no other owner does not accuse the invoking connection.
        let (_, timeout) = second.result("browser.goto", json!({"tab":tab,"url":format!("{url}/stall"),"timeout":300}), CancellationToken::new()).await;
        assert_eq!(timeout["error"]["code"], "timeout", "{timeout}");
        assert!(timeout["error"]["details"].get("debuggingCallers").is_none());
        first
    }).await;
    server.close().await;
}

#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn tab_close_joins_paused_debug_connections_and_preserves_other_tabs() {
    with_browser_fixture(|fixture| async move {
        let tab = fixture.open("cdp.html").await;
        let other = fixture.open("cdp.html").await;
        fixture.call("browser.cdp.send", json!({"tab":tab,"method":"Debugger.enable","params":"{}"})).await;
        let debugging = fixture.clone();
        let request = json!({"tab":tab,"method":"Runtime.evaluate","params":"{\"expression\":\"debugger; 42\"}","timeout":30000});
        let task = tokio::spawn(async move { debugging.result("browser.cdp.send", request, CancellationToken::new()).await });
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(!task.is_finished());
        fixture.call("browser.close", json!({"tab":tab})).await;
        let (code, error) = task.await.unwrap();
        assert_ne!(code, 0);
        assert!(matches!(error["error"]["code"].as_str(), Some("browser_lost" | "tab_not_found")), "{error}");
        let (_, error) = fixture.result("browser.cdp.detach", json!({"tab":tab}), CancellationToken::new()).await;
        assert_eq!(error["error"]["code"], "tab_not_found");
        fixture.call("browser.goto", json!({"tab":other,"url":"about:blank"})).await;
        fixture
    }).await;
}

#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn element_wait_and_input_resample_nodes_inserted_during_locator_resolution() {
    with_browser_fixture(|fixture| async move {
        let tab = fixture.open("cdp.html").await;
        for state in ["attached", "hidden", "click"] {
            let selector = format!("#inserted-{state}");
            let script = format!(
                r#"(() => {{
                    const original = Element.prototype.matches;
                    Element.prototype.matches = function(selector) {{
                        if (selector === {selector} && this === document.documentElement) {{
                            const node = document.createElement('button');
                            node.id = selector.slice(1);
                            node.textContent = 'Inserted during lookup';
                            node.onclick = () => document.body.dataset.insertedClicked = 'yes';
                            document.documentElement.append(node);
                            Element.prototype.matches = original;
                        }}
                        return original.call(this, selector);
                    }};
                }})()"#,
                selector = json!(selector),
            );
            fixture
                .call(
                    "browser.cdp.send",
                    json!({
                        "tab": tab,
                        "method": "Runtime.evaluate",
                        "params": json!({"expression": script}).to_string(),
                    }),
                )
                .await;
            if state == "click" {
                fixture
                    .call("browser.click", json!({"tab": tab, "css": selector}))
                    .await;
                let value = fixture
                    .call(
                        "browser.eval",
                        json!({"tab": tab, "expression": "document.body.dataset.insertedClicked"}),
                    )
                    .await;
                assert_eq!(value["value"], "yes", "{value}");
                continue;
            }
            let (code, result) = fixture
                .result(
                    "browser.wait",
                    json!({
                        "tab": tab, "css": selector, "state": state, "timeout": 500,
                    }),
                    CancellationToken::new(),
                )
                .await;
            if state == "attached" {
                assert_eq!(code, 0, "{result}");
                assert_eq!(result["matched"], true);
                assert!(result["target"]["ref"].is_string(), "{result}");
            } else {
                assert_ne!(code, 0);
                assert_eq!(result["error"]["code"], "timeout", "{result}");
            }
        }
        fixture
    })
    .await;
}
