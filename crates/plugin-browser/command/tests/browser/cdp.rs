use crate::families::with_browser_fixture;
use serde_json::json;
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
