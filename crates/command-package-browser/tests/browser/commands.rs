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
            Record::Medium { .. } | Record::MediumBytes(_) => {
                panic!("an invocation that is no job command's returns no media")
            }
        }
    }
    (completion.unwrap(), stdout, stderr)
}

#[tokio::test]
#[ignore = "requires DEMI_TEST_CHROME; exercises a real browser"]
async fn conversation_browser_commands_share_state_and_retire() {
    let _turn = crate::families::browser_turn().await;
    use axum::{Router, response::Html, routing::get};
    use demi_command_sdk::serve;
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
            Arc::new(demi_browser::DemiBrowser::new()),
        )));
        let (client, connection) = Client::connect(client_io).await.unwrap();
        let driver = AbortOnDropHandle::new(tokio::spawn(connection));
        let _numbers = demi_command_sdk::testing::answer_numbers(&client).await.unwrap();
        let _artifacts =
            demi_command_sdk::testing::answer_artifacts(&client, crate::families::answer_chrome)
                .await
                .unwrap();
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
            stdout: None,
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

/// The Linux target the install tests take, whatever this machine is.
const LINUX: &str = "x86_64-unknown-linux-musl";

/// A request for `operation` with `args` in a conversation of its own, from
/// `root`, answering in JSON when `json` says so.
fn invocation(
    operation: &str,
    args: serde_json::Value,
    root: &std::path::Path,
    json: Option<bool>,
) -> Invocation {
    Invocation {
        operation: operation.into(),
        invocation_id: uuid::Uuid::new_v4().to_string(),
        cwd: root.to_str().unwrap().into(),
        args,
        env: BTreeMap::new(),
        edits: None,
        json,
        context: CommandContext {
            conversation: uuid::Uuid::new_v4().to_string(),
            caller: CommandCaller::agent(1),
            locale: CommandLocale {
                time_zone: "UTC".into(),
                languages: vec!["en-US".into()],
            },
        },
        stdout: None,
    }
}

/// The service with `chrome` in this process, and a client of it that
/// answers its tab numbers.
async fn serve(
    chrome: demi_command_package_browser_chrome::driver::installation::Chrome,
) -> (
    Client,
    Vec<tokio_util::task::AbortOnDropHandle<()>>,
) {
    use std::sync::Arc;
    use tokio_util::task::AbortOnDropHandle;

    let (client_io, server_io) = tokio::io::duplex(128 * 1024);
    let server = AbortOnDropHandle::new(tokio::spawn(async move {
        let _served = demi_command_sdk::serve(
            server_io,
            Arc::new(demi_browser::DemiBrowser::with_chrome(chrome)),
        )
        .await;
    }));
    let (client, connection) = Client::connect(client_io).await.unwrap();
    let driver = AbortOnDropHandle::new(tokio::spawn(async move {
        let _driven = connection.await;
    }));
    // The numbers' answers end with the client's connection.
    let _numbers = demi_command_sdk::testing::answer_numbers(&client).await.unwrap();
    (client, vec![server, driver])
}

/// `demi browser install` reports its downloads in its own output
/// (`browser.md` § Installation): on Linux, Chrome's, then the Chrome
/// runtime's two archives as one download, with a line for each report of
/// the runner's that passes a tenth of their total and one as the last is
/// unpacked, then where the browser is. In JSON, the output is the one
/// document.
// Cost: about 0.1 s; the service runs in this process and no Chrome starts.
#[tokio::test]
async fn install_reports_its_downloads_in_its_output() {
    use demi_command_package_browser_chrome::driver::glibc::GlibcVersion;
    use demi_command_package_browser_chrome::driver::installation::Chrome;
    use demi_command_package_browser_protocol::release::{
        ARTIFACT, BrowserRelease, RUNTIME_FONTS,
    };
    use demi_command_protocol::{ArtifactAsk, ArtifactProgress, ArtifactReply};
    use demi_command_sdk::ArtifactsAsk;
    use tokio_util::task::AbortOnDropHandle;

    let release = BrowserRelease::pinned().unwrap();
    let chrome_size = release.platform(LINUX).unwrap().size;
    let archives = release.runtime_archives(LINUX).unwrap();
    let (libraries, fonts) = (archives.libraries.size, archives.fonts.size);
    let root = tempfile::tempdir().unwrap();
    let chrome = root.path().join("chrome");
    let glibc = GlibcVersion {
        major: 2,
        minor: 39,
    };
    let (client, _tasks) = serve(Chrome::on(LINUX, Some(glibc))).await;
    // A runner that downloads each archive in two halves, unpacks it, and
    // answers where its entry is.
    let stream = client.artifacts().await.unwrap();
    let path = chrome.to_string_lossy().into_owned();
    let _artifacts = AbortOnDropHandle::new(tokio::spawn(async move {
        stream
            .answer::<ArtifactsAsk, _>(|request, reporter| {
                let path = path.clone();
                async move {
                    let Ok(ArtifactAsk::Install(install)) = request.ask() else {
                        panic!("the service asks for installs");
                    };
                    let size = install.size;
                    reporter.report(ArtifactProgress::Download {
                        done: size / 2,
                        total: size,
                    });
                    reporter.report(ArtifactProgress::Download {
                        done: size,
                        total: size,
                    });
                    reporter.report(ArtifactProgress::Unpack);
                    let path = match install.name.as_str() {
                        ARTIFACT => path,
                        RUNTIME_FONTS => "/runtime/fontconfig/fonts.conf".to_owned(),
                        _ => "/runtime/lib/libnss3.so".to_owned(),
                    };
                    Ok(ArtifactReply::Path(path))
                }
            })
            .await
    }));

    let (completion, stdout, stderr) = exchange(
        &client,
        &invocation("browser.install", serde_json::json!({}), root.path(), None),
        false,
    )
    .await;
    assert_eq!(completion.exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let title = release.title();
    let megabytes = |bytes: u64| (bytes as f64 / (1024.0 * 1024.0)).round();
    let runtime = format!("the Chrome runtime {}", release.runtime.release);
    let total = libraries + fonts;
    // With release 1's sizes each report of the runtime's downloads passes
    // another tenth of their total; the libraries' unpacking is not the
    // last.
    let expected = format!(
        "Downloading {title}: {} of {chrome_total} MB\n\
         Downloading {title}: {chrome_total} of {chrome_total} MB\n\
         Unpacking {title}\n\
         Downloading {runtime}: {} of {runtime_total} MB\n\
         Downloading {runtime}: {} of {runtime_total} MB\n\
         Downloading {runtime}: {} of {runtime_total} MB\n\
         Downloading {runtime}: {runtime_total} of {runtime_total} MB\n\
         Unpacking {runtime}\n\
         Installed {title} at {}\n",
        megabytes(chrome_size / 2),
        megabytes(libraries / 2),
        megabytes(libraries),
        megabytes(libraries + fonts / 2),
        chrome.display(),
        chrome_total = megabytes(chrome_size),
        runtime_total = megabytes(total),
    );
    assert_eq!(String::from_utf8(stdout).unwrap(), expected);

    let (completion, stdout, stderr) = exchange(
        &client,
        &invocation("browser.install", serde_json::json!({}), root.path(), Some(true)),
        false,
    )
    .await;
    assert_eq!(completion.exit_code, 0, "{}", String::from_utf8_lossy(&stderr));
    let document: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(document["path"], chrome.to_string_lossy().as_ref());
}

/// A command that would start Chrome on a Host that lacks it says that
/// installing it is the next step, with the size of Chrome and the Chrome
/// runtime on Linux; a Linux Host that has Chrome but not the runtime lacks
/// it all the same (`browser.md` § Browser distribution).
// Cost: about 0.1 s; the service runs in this process and no Chrome starts.
#[tokio::test]
async fn a_host_without_chrome_or_its_runtime_names_the_next_step() {
    use demi_command_package_browser_chrome::driver::glibc::GlibcVersion;
    use demi_command_package_browser_chrome::driver::installation::Chrome;
    use demi_command_package_browser_protocol::release::{ARTIFACT, BrowserRelease};
    use demi_command_protocol::{ArtifactAsk, ArtifactReply, InstalledArtifact};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let release = BrowserRelease::pinned().unwrap();
    let platform = release.platform(LINUX).unwrap().clone();
    let archives = release.runtime_archives(LINUX).unwrap();
    let size = platform.size + archives.libraries.size + archives.fonts.size;
    let root = tempfile::tempdir().unwrap();
    // No Chrome lies there: a command that started it would fail otherwise.
    let chrome = root.path().join("chrome").to_string_lossy().into_owned();
    let glibc = GlibcVersion {
        major: 2,
        minor: 39,
    };
    let (client, _tasks) = serve(Chrome::on(LINUX, Some(glibc))).await;
    // A runner that holds Chrome once the agent installed it, and nothing
    // of the runtime.
    let installed = Arc::new(AtomicBool::new(false));
    let holds = installed.clone();
    let version = release.version.clone();
    let sha256 = platform.sha256.clone();
    let _artifacts = demi_command_sdk::testing::answer_artifacts(&client, move |ask| match ask {
        ArtifactAsk::Installed(question)
            if question.name == ARTIFACT && holds.load(Ordering::SeqCst) =>
        {
            Ok(ArtifactReply::Installed(vec![InstalledArtifact {
                version: version.clone(),
                sha256: sha256.clone(),
                path: chrome.clone(),
            }]))
        }
        ArtifactAsk::Installed(_) => Ok(ArtifactReply::Installed(Vec::new())),
        ArtifactAsk::Install(_) => Err("this runner installs nothing".into()),
    })
    .await
    .unwrap();
    let open = invocation(
        "browser.open",
        serde_json::json!({"url": "about:blank"}),
        root.path(),
        None,
    );
    let megabytes = (size as f64 / (1024.0 * 1024.0)).round();
    let not_installed = format!(
        "Error: browser_unavailable\n\
         {} is not installed on this Host yet. Install it with `demi browser install` ({megabytes} MB), then run this command again.\n\
         Action: not_started.\n",
        release.title()
    );

    let (completion, _, stderr) = exchange(&client, &open, false).await;
    assert_eq!(completion.exit_code, 1);
    assert_eq!(String::from_utf8(stderr).unwrap(), not_installed);

    installed.store(true, Ordering::SeqCst);
    let (completion, _, stderr) = exchange(&client, &open, false).await;
    assert_eq!(completion.exit_code, 1);
    assert_eq!(String::from_utf8(stderr).unwrap(), not_installed);
}

/// A Linux Host whose glibc is older than the Chrome runtime's installs
/// nothing and says what it has and what it needs (`browser.md` § Browser
/// distribution).
// Cost: about 0.1 s; the service runs in this process and no Chrome starts.
#[tokio::test]
async fn a_host_with_too_old_a_glibc_installs_nothing() {
    use demi_command_package_browser_chrome::driver::glibc::GlibcVersion;
    use demi_command_package_browser_chrome::driver::installation::Chrome;
    use demi_command_protocol::ArtifactAsk;

    let root = tempfile::tempdir().unwrap();
    let glibc = GlibcVersion {
        major: 2,
        minor: 26,
    };
    let (client, _tasks) = serve(Chrome::on(LINUX, Some(glibc))).await;
    let _artifacts = demi_command_sdk::testing::answer_artifacts(&client, |ask| match ask {
        ArtifactAsk::Install(install) => panic!("{} is installed", install.name),
        ArtifactAsk::Installed(_) => Ok(demi_command_protocol::ArtifactReply::Installed(Vec::new())),
    })
    .await
    .unwrap();

    let (completion, stdout, stderr) = exchange(
        &client,
        &invocation("browser.install", serde_json::json!({}), root.path(), None),
        false,
    )
    .await;
    assert_eq!(completion.exit_code, 1);
    assert!(stdout.is_empty());
    assert_eq!(
        String::from_utf8(stderr).unwrap(),
        "Error: browser_unavailable\n\
         Chrome on Linux needs glibc 2.28 or newer; this Host has 2.26\n\
         Action: not_started.\n"
    );
}
