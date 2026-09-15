use demi_runner::commands::services::ResidentService;
use std::{collections::BTreeMap, process::Stdio, time::Duration};

use demi_command_service::{
    Client,
    protocol::{Completion, Invocation, Record},
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
            caller: None,
            resource: None,
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
        client.resource(request).await
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
async fn retained_browser_commands_share_state_and_retire() {
    use axum::{Router, response::Html, routing::get};
    use demi_command_service::{protocol::NativeResourceScope, serve};
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
        let scope = NativeResourceScope {
            id: uuid::Uuid::new_v4().to_string(),
            kind: "browser".into(),
        };
        let request = |operation: &str, args: Value| Invocation {
            operation: operation.into(),
            invocation_id: uuid::Uuid::new_v4().to_string(),
            caller: Some("agent-root".into()),
            cwd: root.path().to_str().unwrap().into(),
            args,
            env: BTreeMap::new(),
            edits: None,
            json: Some(true),
            resource: Some(scope.clone()),
        };
        let (completion, _, _) = exchange(&client, &request("acquire", json!({})), true).await;
        assert_eq!(completion.exit_code, 0);
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
        let (completion, stdout, _) = exchange(&client, &request("status", json!({})), true).await;
        assert_eq!(completion.exit_code, 0);
        assert_eq!(
            serde_json::from_slice::<Value>(&stdout).unwrap()["state"],
            "released"
        );
        let (completion, _, _) = exchange(
            &client,
            &request("browser.open", json!({ "url": "about:blank" })),
            false,
        )
        .await;
        assert_eq!(completion.exit_code, 1);
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
        assert_eq!(report.files[0].kind, "added");
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

#[tokio::test]
async fn verified_artifact_launches_and_retires_through_runtime() {
    use demi_command_service::protocol::{PackageArtifact, PackageDescriptor, TARGETS};
    use demi_runner::commands::cache::{
        ArtifactCache, ArtifactResolver, ArtifactSource, RuntimeError,
    };
    use futures_util::future::BoxFuture;
    use sha2::{Digest, Sha256};
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;
    struct Local;
    impl ArtifactResolver for Local {
        fn resolve<'a>(
            &'a self,
            _: &'a PackageArtifact,
            _: &'a CancellationToken,
        ) -> BoxFuture<'a, Result<ArtifactSource, RuntimeError>> {
            Box::pin(async {
                Ok(ArtifactSource::Local(
                    env!("CARGO_BIN_EXE_demi-commands").into(),
                ))
            })
        }
    }
    // Debug executables contain large symbol tables; hashing and copying them on
    // small CI hosts is separate from the resident service's own deadlines.
    tokio::time::timeout(Duration::from_secs(60), async {
        let root = tempfile::tempdir().unwrap();
        let bytes = tokio::fs::read(env!("CARGO_BIN_EXE_demi-commands")).await.unwrap();
        let artifact = PackageArtifact { sha256: format!("{:x}", Sha256::digest(&bytes)), size: bytes.len() as u64 };
        // A catalog fixture; this test executes only the current host target.
        let package = PackageDescriptor::parse(serde_json::json!({
            "id":"demicodes.demi", "version":"test", "protocolVersion":1,
            "operations":demi_command_service::Handler::operations(&demi_commands::DemiCommands::default()),
            "targets": TARGETS.iter().map(|target| (target.to_string(), serde_json::to_value(&artifact).unwrap())).collect::<BTreeMap<_, _>>()
        })).unwrap();
        let cache = ArtifactCache::new(root.path().join("cache")).await.unwrap();
        let executable = cache.acquire(artifact, Arc::new(Local), &CancellationToken::new()).await.unwrap();
        let service = ResidentService::start(&executable, &package, root.path(), &BTreeMap::new()).await.unwrap();
        let pid = service.pid();
        let (result, _, _) = call(service.client(), root.path().to_str().unwrap(), "file.create", serde_json::json!({"path":"through-cache", "content":"hello"})).await;
        assert_eq!(result.exit_code, 0);
        assert_eq!(std::fs::read(root.path().join("through-cache")).unwrap(), b"hello");
        assert_eq!(service.pid(), pid);
        service.shutdown().await.unwrap();
        cache.shutdown().await;
    }).await.unwrap();
}
