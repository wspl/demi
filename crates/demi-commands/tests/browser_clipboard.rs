mod browser_families;
use browser_families::with_browser_fixture;
use serde_json::json;
use tokio_util::sync::CancellationToken;

#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn clipboard_roundtrips_raw_text_html_png_and_page_api() {
    with_browser_fixture(|fixture| async move {
        let tab = fixture.open("clipboard.html").await;
        let capabilities = fixture
            .call("browser.capabilities", json!({"tab":tab}))
            .await;
        assert!(
            capabilities["capabilities"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["id"] == "clipboard" && entry["available"] == true),
            "{capabilities}"
        );
        let (code, result) = fixture
            .result_with_input(
                "browser.clipboard.write",
                json!({"tab":tab}),
                CancellationToken::new(),
                b"hello\n\n".to_vec(),
            )
            .await;
        assert_eq!(code, 0, "{result}");
        assert_eq!(result["bytes"], 7);
        assert_eq!(
            fixture
                .call("browser.clipboard.read", json!({"tab":tab,"format":"text"}))
                .await["text"],
            "hello\n\n"
        );
        fixture
            .call("browser.click", json!({"tab":tab,"css":"#read"}))
            .await;
        assert_eq!(
            fixture
                .call(
                    "browser.read",
                    json!({"tab":tab,"css":"#text","property":"text-content"})
                )
                .await["value"],
            "hello\n\n"
        );
        let (code, error) = fixture
            .result_with_input(
                "browser.clipboard.write",
                json!({"tab":tab}),
                CancellationToken::new(),
                vec![255],
            )
            .await;
        assert_eq!(code, 2, "{error}");
        assert_eq!(
            fixture
                .call("browser.clipboard.read", json!({"tab":tab,"format":"text"}))
                .await["text"],
            "hello\n\n"
        );
        let (code, result) = fixture
            .result_with_input(
                "browser.clipboard.write",
                json!({"tab":tab,"mime":"text/html"}),
                CancellationToken::new(),
                b"<b>bold</b>".to_vec(),
            )
            .await;
        assert_eq!(code, 0, "{result}");
        let result = fixture
            .call(
                "browser.clipboard.read",
                json!({"tab":tab,"output-dir":"html"}),
            )
            .await;
        assert!(
            result["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["mimeType"] == "text/html")
        );
        fixture
            .call(
                "browser.screenshot",
                json!({"tab":tab,"output":"image.png"}),
            )
            .await;
        let bytes = tokio::fs::read(fixture.root.path().join("image.png"))
            .await
            .unwrap();
        let (code, result) = fixture
            .result_with_input(
                "browser.clipboard.write",
                json!({"tab":tab,"mime":"image/png"}),
                CancellationToken::new(),
                bytes,
            )
            .await;
        assert_eq!(code, 0, "{result}");
        let result = fixture
            .call(
                "browser.clipboard.read",
                json!({"tab":tab,"output-dir":"png"}),
            )
            .await;
        assert_eq!(result["items"][0]["mimeType"], "image/png");
        let bytes = tokio::fs::read(result["items"][0]["path"].as_str().unwrap())
            .await
            .unwrap();
        assert!(
            png::Decoder::new(std::io::Cursor::new(bytes))
                .read_info()
                .is_ok()
        );
        let (code, error) = fixture
            .result_with_input(
                "browser.clipboard.write",
                json!({"tab":tab,"mime":"image/png"}),
                CancellationToken::new(),
                b"not a png".to_vec(),
            )
            .await;
        assert_eq!(code, 2, "{error}");
        fixture
            .call("browser.click", json!({"tab":tab,"css":"#write"}))
            .await;
        assert_eq!(
            fixture
                .call("browser.clipboard.read", json!({"tab":tab,"format":"text"}))
                .await["text"],
            "page copied text"
        );
        fixture
    })
    .await;
}
