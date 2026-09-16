mod browser_families;
#[path = "browser/server.rs"]
mod browser_server;
use browser_families::with_browser_fixture;
use serde_json::json;
use tokio_util::sync::CancellationToken;

#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn assets_export_observed_content_and_keep_partial_success() {
    with_browser_fixture(|fixture| async move {
        let tab=fixture.open("assets.html").await;
        let inventory=fixture.call("browser.assets.list",json!({"tab":tab})).await;
        assert!(!inventory["assets"].as_array().unwrap().is_empty());
        assert_eq!(inventory["inlineSvgs"].as_array().unwrap().len(),1);
        let result=fixture.call("browser.assets.export",json!({"tab":tab,"inventory":inventory["inventory"],"kind":["image"],"output-dir":"assets"})).await;
        assert_eq!(result["files"].as_array().unwrap().len(),2);
        for file in result["files"].as_array().unwrap() {
            assert!(tokio::fs::read_to_string(file["path"].as_str().unwrap()).await.unwrap().contains("<svg"));
        }
        let manifest:serde_json::Value=serde_json::from_slice(&tokio::fs::read(result["manifest"].as_str().unwrap()).await.unwrap()).unwrap();
        assert_eq!(manifest["failures"],json!([]));
        let partial=fixture.root.path().join("partial");
        tokio::fs::create_dir(&partial).await.unwrap();
        let id=inventory["assets"][0]["id"].as_str().unwrap();
        tokio::fs::write(partial.join(format!("{id}.svg")),"existing").await.unwrap();
        let (_,error)=fixture.result("browser.assets.export",json!({"tab":tab,"inventory":inventory["inventory"],"kind":["image"],"output-dir":"partial"}),CancellationToken::new()).await;
        assert_eq!(error["error"]["code"],"partial_failure","{error}");
        assert_eq!(error["error"]["details"]["files"].as_array().unwrap().len(),1);
        fixture.call("browser.reload",json!({"tab":tab})).await;
        let (_,error)=fixture.result("browser.assets.export",json!({"tab":tab,"inventory":inventory["inventory"],"kind":["image"],"output-dir":"stale"}),CancellationToken::new()).await;
        assert_eq!(error["error"]["code"],"stale_inventory","{error}");
        fixture
    }).await;
}

#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn asset_inventory_covers_cross_process_frames_and_expires_on_child_navigation() {
    let server = browser_server::Server::start(include_str!("browser/assets-frames.html")).await;
    let base = server.base.clone();
    with_browser_fixture(|fixture| async move {
        let tab = fixture.call("browser.open", json!({"url":base,"load":"load"})).await["tab"].clone();
        let list = fixture.call("browser.assets.list", json!({"tab":tab})).await;
        assert_eq!(list["inlineSvgs"].as_array().unwrap().len(), 3, "{list}");
        assert_eq!(list["assets"].as_array().unwrap().len(), 1, "{list}");
        let result = fixture.call("browser.assets.export", json!({"tab":tab,"inventory":list["inventory"],"kind":["image"],"output-dir":"frames"})).await;
        assert_eq!(result["files"].as_array().unwrap().len(), 4);
        fixture.call("browser.cdp.send", json!({"tab":tab,"method":"Runtime.evaluate","params":json!({"expression":"new Promise(resolve => { const frame = document.querySelector('iframe'); frame.onload = () => resolve(true); frame.src += '?next'; })","awaitPromise":true}).to_string()})).await;
        let (_, error) = fixture.result("browser.assets.export", json!({"tab":tab,"inventory":list["inventory"],"kind":["image"],"output-dir":"stale"}), CancellationToken::new()).await;
        assert_eq!(error["error"]["code"], "stale_inventory", "{error}");
        fixture
    }).await;
    server.close().await;
}
