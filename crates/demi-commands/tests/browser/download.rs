use crate::families::with_browser_fixture;
use serde_json::json;
use tokio_util::sync::CancellationToken;

#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn downloads_publish_complete_files_and_support_media() {
    with_browser_fixture(|fixture| async move {
        let tab = fixture.open("download.html").await;
        let result = fixture.call("browser.download", json!({"tab":tab,"css":"#instant","output":"saved.txt"})).await;
        assert_eq!(tokio::fs::read(result["path"].as_str().unwrap()).await.unwrap(), b"download fixture\n");
        assert_eq!(result["bytes"], 17);
        let (_, error) = fixture.result("browser.download", json!({"tab":tab,"css":"#instant","output":"saved.txt"}), CancellationToken::new()).await;
        assert_eq!(error["error"]["code"], "output_exists", "{error}");
        fixture.call("browser.download", json!({"tab":tab,"css":"#instant","output":"saved.txt","overwrite":true})).await;
        let result = fixture.call("browser.download", json!({"tab":tab,"css":"#instant"})).await;
        let temporary = result["path"].as_str().unwrap();
        assert!(std::path::Path::new(temporary).is_absolute());
        assert_eq!(tokio::fs::read(temporary).await.unwrap(), b"download fixture\n");
        tokio::fs::remove_file(temporary).await.unwrap();
        let position = fixture.call("browser.eval", json!({"tab":tab,"expression":"(() => { const rect = document.querySelector('#media').getBoundingClientRect(); return [rect.x+10, rect.y+10]; })()"})).await;
        let xy = format!("{},{}",position["value"][0],position["value"][1]);
        let result = fixture.call("browser.download",json!({"tab":tab,"xy":xy,"output":"media.svg"})).await;
        assert!(tokio::fs::read_to_string(result["path"].as_str().unwrap()).await.unwrap().contains("<svg"));
        let (_, error) = fixture.result("browser.download",json!({"tab":tab,"css":"#delayed","output":"late.txt","timeout":100}),CancellationToken::new()).await;
        assert_eq!(error["error"]["code"],"timeout","{error}");
        assert!(!fixture.root.path().join("late.txt").exists());
        fixture
    }).await;
}

#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn cancelled_streaming_download_never_publishes_output() {
    let server = crate::server::Server::start(
        "<!doctype html><a id='stream' href='/stream-download'>Download stream</a>",
    )
    .await;
    let base = server.base.clone();
    with_browser_fixture(|fixture| async move {
        let tab = fixture.call("browser.open", json!({"url":base})).await["tab"].clone();
        let cancel = CancellationToken::new();
        let operation_cancel = cancel.clone();
        let downloading = fixture.clone();
        let task = tokio::spawn(async move {
            downloading
                .result(
                    "browser.download",
                    json!({"tab":tab,"css":"#stream","output":"stream.bin","timeout":30000}),
                    operation_cancel,
                )
                .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        assert!(!fixture.root.path().join("stream.bin").exists());
        cancel.cancel();
        let (_, error) = task.await.unwrap();
        assert_eq!(error["error"]["code"], "cancelled", "{error}");
        assert!(!fixture.root.path().join("stream.bin").exists());
        assert_eq!(std::fs::read_dir(fixture.root.path()).unwrap().count(), 0);
        fixture
    })
    .await;
    server.close().await;
}
