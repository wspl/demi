use crate::families::with_browser_fixture;
use serde_json::json;
use tokio_util::sync::CancellationToken;

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
