use std::{collections::BTreeMap, process::Stdio, time::Duration};

use demi_command_service::{
    Client,
    protocol::{CommandCaller, CommandContext, CommandLocale, Completion, Invocation, Record},
};
use tokio::process::Command;

async fn call(
    client: &Client,
    cwd: &str,
    operation: &str,
    args: serde_json::Value,
) -> (Completion, Vec<u8>, Vec<u8>) {
    exchange(
        client,
        &Invocation {
            context: CommandContext {
                conversation: "file-test-conversation".into(),
                caller: CommandCaller::agent("file-test"),
                locale: CommandLocale {
                    time_zone: "UTC".into(),
                    languages: vec!["en-US".into()],
                },
            },
            json: None,
            edits: Some(demi_command_service::protocol::EditContext {
                directory: std::path::Path::new(cwd)
                    .join("changes")
                    .to_string_lossy()
                    .into_owned(),
                lock: std::path::Path::new(cwd)
                    .join("edits.lock")
                    .to_string_lossy()
                    .into_owned(),
            }),
            operation: operation.into(),
            invocation_id: operation.into(),
            args,
            cwd: cwd.into(),
            env: BTreeMap::new(),
        },
        false,
    )
    .await
}

async fn exchange(
    client: &Client,
    request: &Invocation,
    lifecycle: bool,
) -> (Completion, Vec<u8>, Vec<u8>) {
    let (_input, mut output) = if lifecycle {
        use demi_command_service::protocol::ConversationRequest;
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
#[ignore = "installs the pinned Chrome for Testing release and exercises a real browser"]
async fn conversation_browser_commands_share_state_and_retire() {
    use axum::{Router, response::Html, routing::get};
    use demi_command_service::serve;
    use serde_json::{Value, json};
    use std::sync::Arc;
    use tokio_util::{sync::CancellationToken, task::AbortOnDropHandle};

    tokio::time::timeout(Duration::from_secs(180), async {
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
                    get(|| async { Html(include_str!("browser/fixture.html")) }),
                ),
            )
            .with_graceful_shutdown(stopped.cancelled_owned())
            .await
            .unwrap();
        }));
        let (client_io, server_io) = tokio::io::duplex(128 * 1024);
        let server = AbortOnDropHandle::new(tokio::spawn(serve(
            server_io,
            Arc::new(demi_commands::DemiCommands::default()),
        )));
        let (client, connection) = Client::connect(client_io).await.unwrap();
        let driver = AbortOnDropHandle::new(tokio::spawn(connection));
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
                caller: CommandCaller::agent("agent-root"),
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
            &request("browser.info", json!({"tab": "t_AAAAAAAAAAAAAAAAAAAAAA"})),
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

#[tokio::test]
async fn resident_executable_runs_all_builtin_file_operations() {
    tokio::time::timeout(Duration::from_secs(15), async {
        let root = tempfile::tempdir().unwrap();
        let cwd = root.path().to_str().unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_demi-commands"))
            .arg("--command-service")
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit())
            .kill_on_drop(true).spawn().unwrap();
        let pid = child.id().unwrap();
        let io = tokio::io::join(child.stdout.take().unwrap(), child.stdin.take().unwrap());
        let (client, connection) = Client::connect(io).await.unwrap();
        let driver = tokio::spawn(connection);
        assert!(client.info().await.unwrap().operations.contains(&"file.read".into()));
        let (result, output, _) = call(&client, cwd, "file.create", serde_json::json!({"path":"nested/a.txt", "content":"alpha\nbeta\n"})).await;
        assert_eq!(result.exit_code, 0);
        assert_eq!(output, b"Created nested/a.txt\n");
        let (result, _, _) = call(&client, cwd, "file.create", serde_json::json!({"path":"nested/a.txt", "content":"overwrite"})).await;
        assert_eq!(result.exit_code, 1);
        let (result, _, _) = call(&client, cwd, "file.edit", serde_json::json!({"path":"nested/a.txt", "old":"beta", "new":"gamma"})).await;
        assert_eq!(result.exit_code, 0);
        let patch = "--- a/nested/a.txt\n+++ b/nested/a.txt\n@@ -1,2 +1,2 @@\n alpha\n-gamma\n+delta\n--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1 @@\n+created\n";
        let (result, output, error) = call(&client, cwd, "file.patch", serde_json::json!({"patch":patch})).await;
        assert_eq!(result.exit_code, 0, "{}", String::from_utf8_lossy(&error));
        assert_eq!(output, b"Patched 2 file(s)\n");
        let (_, output, _) = call(&client, cwd, "file.read", serde_json::json!({"path":"nested/a.txt"})).await;
        assert_eq!(output, b"alpha\ndelta\n");
        let binary = [0, 255, 10, 13, 128];
        std::fs::write(root.path().join("image.bin"), binary).unwrap();
        let (result, output, _) = call(&client, cwd, "file.read", serde_json::json!({"path":"image.bin"})).await;
        assert_eq!(result.exit_code, 0);
        assert_eq!(output, binary);
        let recorder = demi_command_service::edits::Recorder::new(demi_command_service::protocol::EditContext {
            directory: root.path().join("changes").to_string_lossy().into_owned(),
            lock: root.path().join("edits.lock").to_string_lossy().into_owned(),
        }).unwrap();
        let report = recorder.report().unwrap();
        let large = root.path().join("large.txt");
        std::fs::write(&large, "x".repeat(demi_command_service::protocol::EDIT_FILE_BYTES + 1)).unwrap();
        for (operation, args) in [
            ("file.create", serde_json::json!({"path":"large.txt", "content":"overwrite"})),
            ("file.edit", serde_json::json!({"path":"large.txt", "old":"absent", "new":"replacement"})),
            ("file.patch", serde_json::json!({"patch":"--- a/large.txt\n+++ b/large.txt\n@@ -1 +1 @@\n-absent\n+replacement\n"})),
        ] {
            let (result, _, _) = call(&client, cwd, operation, args).await;
            assert_eq!(result.exit_code, 1);
        }
        assert_eq!(recorder.report().unwrap().files.len(), 2);
        assert_eq!(report.files.len(), 2);
        assert_eq!(
            report.files[0].kind,
            demi_command_service::protocol::EditKind::Added
        );
        assert_eq!(report.files[0].edits.len(), 1);
        assert_eq!(std::fs::read(report.files[0].edits[0].modified.as_ref().unwrap()).unwrap(), b"alpha\ndelta\n");
        assert_eq!(std::fs::read(report.files[1].edits[0].modified.as_ref().unwrap()).unwrap(), b"created\n");
        assert_eq!(child.id(), Some(pid));
        client.shutdown().await.unwrap();
        let status = child.wait().await.unwrap();
        drop(client);
        driver.await.unwrap().unwrap();
        assert!(status.success());
    }).await.unwrap();
}
