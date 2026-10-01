use std::{collections::BTreeMap, time::Duration};

use demi_command_protocol::{
    CommandCaller, CommandContext, CommandLocale, Completion, Invocation, Record,
};
use demi_command_sdk::Client;

pub(crate) async fn exchange(
    client: &Client,
    request: &Invocation,
    lifecycle: bool,
) -> (Completion, Vec<u8>, Vec<u8>) {
    let (_input, mut output) = if lifecycle {
        use demi_command_protocol::ConversationRequest;
        let lifecycle = match request.operation.as_str() {
            "status" => ConversationRequest::Status {},
            "release" => ConversationRequest::Release {
                conversation: request.context.conversation.clone(),
            },
            other => panic!("unknown lifecycle operation {other}"),
        };
        client.conversation(&lifecycle).await
    } else {
        client.invoke(request).await
    }
    .unwrap();
    // These fixture commands never request stdin and may finish before invoke returns.
    // Keep the request handle alive without sending unsolicited input or EOF.
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut completion = None;
    while let Some(record) = output.next().await.unwrap() {
        match record {
            Record::Stdout(bytes) => stdout.extend_from_slice(&bytes),
            Record::Stderr(bytes) => stderr.extend_from_slice(&bytes),
            Record::Completion(value) => completion = Some(value),
            Record::InputPull => panic!("file operation must not read raw stdin"),
        }
    }
    (completion.unwrap(), stdout, stderr)
}

#[tokio::test]
#[ignore = "requires DEMI_TEST_CHROME; exercises a real browser"]
async fn conversation_browser_commands_share_state_and_retire() {
    use axum::{Router, response::Html, routing::get};
    use demi_command_sdk::serve;
    use serde_json::{Value, json};
    use std::sync::Arc;
    use tokio_util::{sync::CancellationToken, task::AbortOnDropHandle};

    tokio::time::timeout(Duration::from_secs(180), async {
        let (_chrome, directories) = crate::families::installed_chrome().await;
        let root = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let stop = CancellationToken::new();
        let stopped = stop.clone();
        let site = AbortOnDropHandle::new(tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/",
                    get(|| async { Html(include_str!("fixture.html")) }),
                ),
            )
            .with_graceful_shutdown(stopped.cancelled_owned())
            .await
            .unwrap();
        }));
        let (client_io, server_io) = tokio::io::duplex(128 * 1024);
        let server = AbortOnDropHandle::new(tokio::spawn(serve(
            server_io,
            Arc::new(demi_browser::DemiBrowser::new(directories)),
        )));
        let (client, connection) = Client::connect(client_io).await.unwrap();
        let driver = AbortOnDropHandle::new(tokio::spawn(connection));
        let _numbers = demi_command_sdk::testing::answer_numbers(&client).await.unwrap();
        let conversation = uuid::Uuid::new_v4().to_string();
        let request = |operation: &str, args: Value| Invocation {
            operation: operation.into(),
            invocation_id: uuid::Uuid::new_v4().to_string(),
            cwd: root.path().to_str().unwrap().into(),
            args,
            env: BTreeMap::new(),
            edits: None,
            json: Some(true),
            context: CommandContext {
                conversation: conversation.clone(),
                caller: CommandCaller::agent(1),
                locale: CommandLocale {
                    time_zone: "UTC".into(),
                    languages: vec!["en-US".into()],
                },
            },
        };
        let (completion, stdout, stderr) =
            exchange(&client, &request("browser.tabs", json!({})), false).await;
        assert_eq!(
            completion.exit_code,
            0,
            "{:?}",
            String::from_utf8_lossy(&stderr)
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&stdout).unwrap()["tabs"],
            json!([])
        );
        let (completion, stdout, stderr) = exchange(
            &client,
            &request("browser.open", json!({ "url": url, "timeout": 120000 })),
            false,
        )
        .await;
        assert_eq!(
            completion.exit_code,
            0,
            "{:?}",
            String::from_utf8_lossy(&stderr)
        );
        let tab = serde_json::from_slice::<Value>(&stdout).unwrap()["tab"]
            .as_str()
            .unwrap()
            .to_owned();
        let (completion, _, stderr) = exchange(
            &client,
            &request(
                "browser.goto",
                json!({ "tab": tab, "url": format!("{url}/?navigated") }),
            ),
            false,
        )
        .await;
        assert_eq!(
            completion.exit_code,
            0,
            "{:?}",
            String::from_utf8_lossy(&stderr)
        );
        let (completion, stdout, stderr) = exchange(
            &client,
            &request("browser.inspect", json!({ "tab": tab })),
            false,
        )
        .await;
        assert_eq!(
            completion.exit_code,
            0,
            "{:?}",
            String::from_utf8_lossy(&stderr)
        );
        assert!(
            !serde_json::from_slice::<Value>(&stdout).unwrap()["tree"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        for (operation, args) in [
            (
                "browser.fill",
                json!({ "tab": tab, "css": "#email", "text": "浏览器@example.test" }),
            ),
            ("browser.click", json!({ "tab": tab, "css": "#normal" })),
        ] {
            let (completion, _, stderr) = exchange(&client, &request(operation, args), false).await;
            assert_eq!(
                completion.exit_code,
                0,
                "{operation}: {:?}",
                String::from_utf8_lossy(&stderr)
            );
        }
        let (completion, stdout, stderr) = exchange(
            &client,
            &request(
                "browser.read",
                json!({ "tab": tab, "css": "#email", "property": "value" }),
            ),
            false,
        )
        .await;
        assert_eq!(
            completion.exit_code,
            0,
            "{:?}",
            String::from_utf8_lossy(&stderr)
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&stdout).unwrap()["value"],
            "浏览器@example.test"
        );
        let (completion, stdout, stderr) = exchange(
            &client,
            &request(
                "browser.screenshot",
                json!({ "tab": tab, "output": "browser.png" }),
            ),
            false,
        )
        .await;
        assert_eq!(
            completion.exit_code,
            0,
            "{:?}",
            String::from_utf8_lossy(&stderr)
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&stdout).unwrap()["mimeType"],
            "image/png"
        );
        assert!(
            std::fs::read(root.path().join("browser.png"))
                .unwrap()
                .starts_with(b"\x89PNG\r\n\x1a\n")
        );
        for (operation, args, dimensions) in [
            ("browser.screenshot", json!({"tab": tab, "output": "clip.png", "clip": "0,0,100,80"}), Some((100, 80))),
            ("browser.probe", json!({"tab": tab, "xy": "30,30", "include-non-interactable": true, "output": "probe.png"}), None),
        ] {
            let (completion, stdout, stderr) = exchange(&client, &request(operation, args), false).await;
            assert_eq!(completion.exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
            let result: Value = serde_json::from_slice(&stdout).unwrap();
            let bytes = std::fs::read(result["path"].as_str().unwrap()).unwrap();
            let decoded = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().unwrap();
            if let Some((width, height)) = dimensions {
                assert_eq!((decoded.info().width, decoded.info().height), (width, height));
            } else {
                assert!(!result["matches"].as_array().unwrap().is_empty());
            }
        }
        let (completion, stdout, stderr) = exchange(&client, &request("browser.tabs", json!({"offset": 1, "limit": 1})), false).await;
        assert_eq!(completion.exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
        assert_eq!(serde_json::from_slice::<Value>(&stdout).unwrap()["tabs"], json!([]));
        for (args, code) in [
            (
                json!({"tab": tab, "output": "browser.png"}),
                "output_exists",
            ),
            (
                json!({"tab": tab, "output": "missing-directory/browser.png"}),
                "io_error",
            ),
        ] {
            let (completion, _, stderr) =
                exchange(&client, &request("browser.screenshot", args), false).await;
            assert_eq!(completion.exit_code, 1);
            assert_eq!(
                serde_json::from_slice::<Value>(&stderr).unwrap()["error"]["code"],
                code
            );
        }
        let (completion, _, stderr) = exchange(
            &client,
            &request("browser.info", json!({"tab": "t999"})),
            false,
        )
        .await;
        assert_eq!(completion.exit_code, 1);
        assert_eq!(
            serde_json::from_slice::<Value>(&stderr).unwrap()["error"]["code"],
            "tab_not_found"
        );
        let (completion, _, stderr) = exchange(
            &client,
            &request(
                "browser.key",
                json!({"tab": tab, "css": "#email", "key": "not-a-key"}),
            ),
            false,
        )
        .await;
        assert_eq!(completion.exit_code, 2);
        let failure = serde_json::from_slice::<Value>(&stderr).unwrap();
        assert_eq!(failure["error"]["code"], "invalid_input");
        assert_eq!(failure["error"]["details"]["action"], "not_started");
        let mut invalid_key = request(
            "browser.key",
            json!({"tab": tab, "css": "#email", "key": "not-a-key"}),
        );
        invalid_key.json = Some(false);
        let (completion, _, stderr) = exchange(&client, &invalid_key, false).await;
        assert_eq!(completion.exit_code, 2);
        let text = String::from_utf8(stderr).unwrap();
        assert!(text.starts_with("Error: invalid_input\n"));
        assert!(text.contains("\nAction: not_started.\n"));
        assert!(text.contains(&format!("Tab: {tab}\n")));
        assert!(!text.contains("Details: {"));

        let mut inspect = request("browser.inspect", json!({"tab": tab, "limit": 1000}));
        inspect.json = Some(false);
        let (completion, stdout, _) = exchange(&client, &inspect, false).await;
        assert_eq!(completion.exit_code, 0);
        let text = String::from_utf8(stdout).unwrap();
        assert!(text.contains("[checked=false]"));
        assert!(text.contains("[value=\"浏览器@example.test\"]"));
        assert!(text.contains("[protected]"));
        assert!(!text.contains("fixture-secret"));
        let (completion, _, stderr) = exchange(
            &client,
            &request("browser.click", json!({"tab": tab, "css": "#popup"})),
            false,
        )
        .await;
        assert_eq!(
            completion.exit_code,
            0,
            "{}",
            String::from_utf8_lossy(&stderr)
        );
        let (completion, _, stderr) = exchange(
            &client,
            &request("browser.close", json!({ "tab": tab })),
            false,
        )
        .await;
        assert_eq!(
            completion.exit_code,
            0,
            "{:?}",
            String::from_utf8_lossy(&stderr)
        );
        let (completion, stdout, stderr) =
            exchange(&client, &request("browser.tabs", json!({})), false).await;
        assert_eq!(
            completion.exit_code,
            0,
            "{}",
            String::from_utf8_lossy(&stderr)
        );
        let listed: Value = serde_json::from_slice(&stdout).unwrap();
        let popup = &listed["tabs"][0];
        assert_eq!(listed["tabs"].as_array().unwrap().len(), 1);
        assert_eq!(popup["createdBy"]["opener"], tab);
        let (completion, _, stderr) = exchange(
            &client,
            &request("browser.close", json!({"tab": popup["id"]})),
            false,
        )
        .await;
        assert_eq!(
            completion.exit_code,
            0,
            "{}",
            String::from_utf8_lossy(&stderr)
        );
        let (completion, stdout, _) = exchange(&client, &request("status", json!({})), true).await;
        assert_eq!(completion.exit_code, 0);
        assert_eq!(
            serde_json::from_slice::<Value>(&stdout).unwrap()["conversations"],
            json!([])
        );
        let (completion, _, _) = exchange(
            &client,
            &request("browser.open", json!({ "url": "about:blank" })),
            false,
        )
        .await;
        assert_eq!(completion.exit_code, 0);
        let (completion, stdout, _) = exchange(&client, &request("release", json!({})), true).await;
        assert_eq!(completion.exit_code, 0);
        assert_eq!(serde_json::from_slice::<Value>(&stdout).unwrap(), json!({}));
        client.shutdown().await.unwrap();
        drop(client);
        server.await.unwrap().unwrap();
        driver.await.unwrap().unwrap();
        stop.cancel();
        site.await.unwrap();
    })
    .await
    .unwrap();
}

