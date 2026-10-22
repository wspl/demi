#![cfg(unix)]

use std::{os::unix::fs::PermissionsExt, path::PathBuf, time::Duration};

use demi_command_package_browser_chrome::driver::{
    installation::Installation, numbers::TabNumbers, operation::BrowserError,
};
use demi_command_package_browser_chrome::tabs::environment::{
    LaunchOptions, with_browser,
};
use crate::processes::processes;
use demi_command_sdk::testing::counting_numbers;
use serde_json::json;
use tokio_util::sync::CancellationToken;

/// About 1.5 s in the Linux container: the helper the launcher left is
/// reparented to the container's init, which reaps it about once a second,
/// and retirement ends only once the helper is gone.
#[tokio::test]
async fn canceled_launch_reaps_helpers_before_removing_profile() {
    let directory = tempfile::tempdir().unwrap();
    let launcher = directory.path().join("pending-chrome");
    std::fs::write(
        &launcher,
        "#!/bin/sh\nsleep 120 &\nprintf '%s\\n' \"$$\" \"$!\" \"$@\" > \"$0.record\"\nwait\n",
    )
    .unwrap();
    std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o700)).unwrap();
    let record = directory.path().join("pending-chrome.record");
    let stop = CancellationToken::new();
    let cancel = stop.clone();
    let (result, recorded) = tokio::join!(
        with_browser(
            LaunchOptions::pinned(
                Installation {
                    executable: launcher,
                    runtime: None,
                },
                demi_command_protocol::CommandLocale {
                    time_zone: "UTC".into(),
                    languages: vec!["en-US".into()]
                },
                demi_command_protocol::ColorScheme::Light,
            )
            .unwrap(),
            TabNumbers::new(counting_numbers(), "conversation".into()),
            stop,
            |_| async {
                Err::<(), _>(BrowserError::Configuration(
                    "pending launch delivered an environment".into(),
                ))
            }
        ),
        async {
            let recording = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if let Ok(contents) = tokio::fs::read_to_string(&record).await
                        && contents.contains("--user-data-dir=")
                    {
                        break contents;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await;
            cancel.cancel();
            recording.unwrap()
        }
    );
    assert!(matches!(result, Err(BrowserError::Cancelled)), "{result:?}");
    let mut lines = recorded.lines();
    let leader: i32 = lines.next().unwrap().parse().unwrap();
    let helper: i32 = lines.next().unwrap().parse().unwrap();
    let profile = PathBuf::from(
        lines
            .find_map(|line| line.strip_prefix("--user-data-dir="))
            .unwrap(),
    );
    assert!(
        !processes()
            .iter()
            .any(|process| process.pid == helper || process.group == leader)
    );
    assert!(!profile.exists());
}

/// A service that is killed leaves its profile and a download saved without
/// `--output` behind. The next service of the same user removes them as it
/// starts.
///
/// The next service sweeps beside serving and gives the sweep up when serving
/// ends. The sweep asks the runner where Chrome is installed, so the test
/// waits, while the service serves, for the directories to go.
///
/// Cost: about 6 s, two service programs and one Chrome.
#[tokio::test]
#[ignore = "requires DEMI_TEST_CHROME; kills the service program and sweeps what it left"]
async fn the_next_service_sweeps_a_killed_services_browser_and_downloads() {
    let _turn = crate::families::browser_turn().await;
    let scratch = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let temporary = tempfile::tempdir().unwrap();
    let page = url::Url::from_file_path(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/browser/download.html"),
    )
    .unwrap();
    tokio::time::timeout(Duration::from_secs(60), async {
        let killed = service_program(home.path(), temporary.path().to_str().unwrap()).await;
        let tab = agent_call(
            &killed,
            "browser.open",
            json!({"url": page.as_str()}),
            scratch.path(),
        )
        .await["tab"]
            .clone();
        let download = agent_call(
            &killed,
            "browser.download",
            json!({"tab": tab, "css": "#instant"}),
            scratch.path(),
        )
        .await;
        let saved = PathBuf::from(download["path"].as_str().unwrap());
        assert!(saved.is_file(), "{}", saved.display());
        let pid = i32::try_from(killed.id().unwrap()).unwrap();
        // SAFETY: `kill` has no memory preconditions; the process is this
        // test's child.
        assert_eq!(unsafe { libc::kill(pid, libc::SIGKILL) }, 0);
        drop(killed);
        let next = service_program(home.path(), temporary.path().to_str().unwrap()).await;
        // The outer deadline guards against a sweep that never ends.
        // Only the top level is read: the sweep removes what lies below meanwhile.
        while std::fs::read_dir(temporary.path()).unwrap().next().is_some() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(next.shutdown().await.unwrap().success());
    })
    .await
    .unwrap_or_else(|_| panic!("the profile and its download stay: {:?}", walk(temporary.path())));
}

/// The service program as the runner starts it, with `home` as its home and
/// `temporary` as its `TMPDIR`; its numbers stream is answered as a runner
/// answers it, its messages are taken as a runner takes them, and its request
/// for Chrome is answered with the pinned Chrome that `DEMI_TEST_CHROME`
/// names.
async fn service_program(
    home: &std::path::Path,
    temporary: &str,
) -> demi_command_sdk::testing::ServiceProcess {
    let service = demi_command_sdk::testing::ServiceProcess::start(
        env!("CARGO_BIN_EXE_demi-browser"),
        &["--command-service"],
        &[("HOME", home.to_str().unwrap()), ("TMPDIR", temporary)],
    )
    .await
    .unwrap();
    // The answering task ends with the stream, as the service shuts down.
    let _answering = demi_command_sdk::testing::answer_numbers(service.client())
        .await
        .unwrap();
    let _artifacts = demi_command_sdk::testing::answer_artifacts(
        service.client(),
        crate::families::answer_chrome,
    )
    .await
    .unwrap();
    // Without it, the tabs' first change would wait for the stream, and the
    // service's shutdown for that change to be told.
    let _messages = demi_command_sdk::testing::answer_messages(service.client(), |_message| {})
        .await
        .unwrap();
    service
}

/// The service program's conversation and caller.
fn service_context() -> demi_command_protocol::CommandContext {
    use demi_command_protocol::{CommandCaller, CommandContext, CommandLocale};
    CommandContext {
        color_scheme: demi_command_protocol::ColorScheme::Light,
        conversation: "service-program".into(),
        caller: CommandCaller::agent(1),
        locale: CommandLocale {
            time_zone: "UTC".into(),
            languages: vec!["en-US".into()],
        },
    }
}

/// The agent's `operation` on the service program, from `cwd`; answers its
/// JSON result, once it succeeded.
async fn agent_call(
    service: &demi_command_sdk::testing::ServiceProcess,
    operation: &str,
    args: serde_json::Value,
    cwd: &std::path::Path,
) -> serde_json::Value {
    let invocation = demi_command_protocol::Invocation {
        operation: operation.into(),
        invocation_id: uuid::Uuid::new_v4().to_string(),
        command: "demi test".into(),
        context: service_context(),
        json: Some(true),
        edits: None,
        args,
        cwd: cwd.to_str().unwrap().into(),
        env: std::collections::BTreeMap::new(),
        live_input: None,
        viewable: None,
    };
    let (completion, stdout, stderr) =
        exchange(service.client(), &invocation, false).await;
    assert_eq!(
        completion.exit_code,
        0,
        "{operation}: {}",
        String::from_utf8_lossy(&stderr)
    );
    serde_json::from_slice(&stdout).unwrap()
}

/// Every file and directory under `root`, relative to it.
fn walk(root: &std::path::Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_owned()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            if path.symlink_metadata().unwrap().is_dir() {
                pending.push(path.clone());
            }
            found.push(path.strip_prefix(root).unwrap().to_owned());
        }
    }
    found
}

/// Runs one invocation, or a conversation lifecycle call, and collects its
/// completion, standard output and standard error.
async fn exchange(
    client: &demi_command_sdk::Client,
    request: &demi_command_protocol::Invocation,
    lifecycle: bool,
) -> (demi_command_protocol::Completion, Vec<u8>, Vec<u8>) {
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
            demi_command_protocol::Record::Stdout(bytes) => stdout.extend_from_slice(&bytes),
            demi_command_protocol::Record::Stderr(bytes) => stderr.extend_from_slice(&bytes),
            demi_command_protocol::Record::Completion(value) => completion = Some(value),
            demi_command_protocol::Record::InputPull => panic!("file operation must not read raw stdin"),
            demi_command_protocol::Record::Medium { .. } | demi_command_protocol::Record::MediumBytes(_) => {
                panic!("an invocation that is no job command's returns no media")
            }
        }
    }
    (completion.unwrap(), stdout, stderr)
}
