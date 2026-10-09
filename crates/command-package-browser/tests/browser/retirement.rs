#![cfg(unix)]

use std::{os::unix::fs::PermissionsExt, path::PathBuf, time::Duration};

use demi_command_package_browser_chrome::driver::{
    installation::Installation, numbers::TabNumbers, operation::BrowserError,
};
use demi_command_package_browser_chrome::tabs::environment::{
    LaunchOptions, with_browser,
};
use crate::processes::{chrome_profiles_of, processes};
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

/// The service's own temporary directory can be long: a Mac's is about 49
/// characters, and a user can set any. The profile, which can grow large,
/// goes there, while Chrome's temporary directory, where it makes its
/// process-singleton socket, stays in the short, fixed runtime base, so the
/// socket's path keeps within its limit and Chrome starts (`browser.md`
/// § Native driver). Under this directory of at least 90 characters the
/// socket's path would take at least 155 bytes, past Linux's 107. Retirement
/// leaves nothing in that directory, and Chrome writes nothing of its own in
/// the service's home but the user's certificate database: its crash
/// reports go with the profile. About 5 s: the service's `TMPDIR` and home
/// differ from this test process's only when the service runs as a program
/// of its own; starting Chrome and opening a tab take 2 s, and retiring it
/// 1.5 s.
#[tokio::test]
#[ignore = "requires DEMI_TEST_CHROME; starts the service program with its own home and a long TMPDIR"]
async fn chrome_keeps_its_directories_apart_whatever_the_services_home_and_temporary_directory() {
    let _turn = crate::families::browser_turn().await;
    let scratch = tempfile::tempdir().unwrap();
    let padding = 90usize
        .saturating_sub(scratch.path().as_os_str().len() + 1)
        .max(1);
    let temporary = scratch.path().join("t".repeat(padding));
    std::fs::create_dir(&temporary).unwrap();
    let home = tempfile::tempdir().unwrap();
    tokio::time::timeout(Duration::from_secs(60), async {
        let service = service_program(home.path(), temporary.to_str().unwrap()).await;
        agent_call(
            &service,
            "browser.open",
            json!({"url": "about:blank"}),
            scratch.path(),
        )
        .await;
        assert_eq!(
            chrome_profiles_of(service.id().unwrap(), &temporary).len(),
            1,
            "the service's Chrome keeps its profile in {}",
            temporary.display()
        );
        release_and_shut_down(service, scratch.path()).await;
    })
    .await
    .unwrap();
    assert_eq!(walk(&temporary), Vec::<PathBuf>::new());
    // Only what Chrome keeps for the user, and the directories above it, are
    // in the home (`browser.md` § Native driver): on Linux the certificate
    // database, on macOS the crash reports.
    let kept = if cfg!(target_os = "macos") {
        ["Library/Application Support/Google/Chrome for Testing"]
    } else {
        [".local/share/pki"]
    };
    let written: Vec<_> = walk(home.path())
        .into_iter()
        .filter(|path| {
            !kept
                .iter()
                .any(|kept| path.starts_with(kept) || std::path::Path::new(kept).starts_with(path))
        })
        .collect();
    assert!(
        written.is_empty(),
        "Chrome wrote in the service's home: {written:?}"
    );
}

/// Without `TMPDIR`, as on the Cloud, a profile and a download saved without
/// `--output` go to `/var/tmp`, on disk, rather than to a `/tmp` that may be
/// held in memory (`browser.md` § Native driver, § Upload, download, and
/// clipboard). The download is in its environment's directory and goes when
/// the environment retires. An empty `TMPDIR` stands in for none, since the
/// service program inherits this test's environment. About 5 s, as above.
#[tokio::test]
#[ignore = "requires DEMI_TEST_CHROME; starts the service program without TMPDIR, so it uses /var/tmp"]
async fn without_tmpdir_profiles_and_downloads_go_to_var_tmp() {
    let _turn = crate::families::browser_turn().await;
    let scratch = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let base = std::path::Path::new("/var/tmp");
    let page = url::Url::from_file_path(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/browser/download.html"),
    )
    .unwrap();
    let profile = tokio::time::timeout(Duration::from_secs(60), async {
        let service = service_program(home.path(), "").await;
        let tab = agent_call(
            &service,
            "browser.open",
            json!({"url": page.as_str()}),
            scratch.path(),
        )
        .await["tab"]
            .clone();
        let profiles = chrome_profiles_of(service.id().unwrap(), base);
        assert_eq!(
            profiles.len(),
            1,
            "the service's Chrome keeps its profile in {}",
            base.display()
        );
        let download = agent_call(
            &service,
            "browser.download",
            json!({"tab": tab, "css": "#instant"}),
            scratch.path(),
        )
        .await;
        let saved = PathBuf::from(download["path"].as_str().unwrap());
        let profile = profiles.into_values().next().unwrap();
        assert!(saved.starts_with(&profile), "{}", saved.display());
        assert_eq!(std::fs::read(&saved).unwrap(), b"download fixture\n");
        release_and_shut_down(service, scratch.path()).await;
        (profile, saved)
    })
    .await
    .unwrap();
    assert!(!profile.0.exists(), "{}", profile.0.display());
    assert!(!profile.1.exists(), "{}", profile.1.display());
}

/// A service that is killed leaves its profile and a download saved without
/// `--output` behind; the next service of the same user removes them as it
/// starts (`browser.md` § Native driver). The next service sweeps beside
/// serving and gives the sweep up when serving ends, and the sweep asks the
/// runner where Chrome is installed, so the test waits, while the service
/// serves, for the directories to go. About 6 s: two service programs, one
/// Chrome.
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
/// answers it, and its request for Chrome with the pinned Chrome that
/// `DEMI_TEST_CHROME` names.
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
        stdout: None,
    };
    let (completion, stdout, stderr) =
        crate::commands::exchange(service.client(), &invocation, false).await;
    assert_eq!(
        completion.exit_code,
        0,
        "{operation}: {}",
        String::from_utf8_lossy(&stderr)
    );
    serde_json::from_slice(&stdout).unwrap()
}

/// Releases the service program's conversation, which retires its browser,
/// and shuts the service down.
async fn release_and_shut_down(
    service: demi_command_sdk::testing::ServiceProcess,
    cwd: &std::path::Path,
) {
    let release = demi_command_protocol::Invocation {
        operation: "release".into(),
        invocation_id: uuid::Uuid::new_v4().to_string(),
        command: "demi test".into(),
        context: service_context(),
        json: Some(true),
        edits: None,
        args: json!({}),
        cwd: cwd.to_str().unwrap().into(),
        env: std::collections::BTreeMap::new(),
        stdout: None,
    };
    let (completion, _, _) = crate::commands::exchange(service.client(), &release, true).await;
    assert_eq!(completion.exit_code, 0);
    assert!(service.shutdown().await.unwrap().success());
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

#[tokio::test]
#[ignore = "requires DEMI_TEST_CHROME; verifies trusted invocation identity"]
async fn browser_uses_trusted_conversation_and_caller_despite_script_environment() {
    crate::families::with_browser_fixture(|mut first| async move {
        let mut second = first.clone();
        second.conversation = "other-conversation".into();
        second.caller = 2;
        // Each conversation numbers its own tabs: the other's second tab has
        // a number this conversation has not given out.
        second.open("fixture.html").await;
        let second_tab = second.open("fixture.html").await;
        first
            .env
            .insert("DEMI_CONVERSATION_ID".into(), second.conversation.clone());
        first
            .env
            .insert("DEMI_AGENT_NODE_ID".into(), second.caller.to_string());
        let first_tab = first.open("fixture.html").await;
        let tabs = first.call("browser.tabs", json!({})).await;
        assert_eq!(tabs["tabs"].as_array().unwrap().len(), 1);
        assert_eq!(tabs["tabs"][0]["id"], first_tab);
        assert_eq!(
            tabs["tabs"][0]["createdBy"],
            json!({"kind": "agent", "number": first.caller})
        );
        let (_, error) = first
            .result(
                "browser.info",
                json!({"tab": second_tab}),
                CancellationToken::new(),
            )
            .await;
        assert_eq!(error["error"]["code"], "tab_not_found");
        assert_eq!(
            second.call("browser.tabs", json!({})).await["tabs"][1]["id"],
            second_tab
        );
        first.call("browser.close", json!({"tab": first_tab})).await;
        assert_eq!(
            first.lifecycle("status").await,
            json!({"conversations": [second.conversation]})
        );
        // The next browser's tab takes the conversation's next number, never
        // the one the closed tab had.
        let fresh = first.open("fixture.html").await;
        assert_ne!(fresh, first_tab);
        let (_, error) = first
            .result(
                "browser.info",
                json!({"tab": first_tab}),
                CancellationToken::new(),
            )
            .await;
        assert_eq!(error["error"]["code"], "tab_not_found");
        first
    })
    .await;
}

#[tokio::test]
#[ignore = "requires DEMI_TEST_CHROME; verifies the last-tab closing outcome"]
async fn last_tab_close_fails_its_running_command_as_browser_lost() {
    crate::families::with_browser_fixture(|fixture| async move {
        let tab = fixture.open("fixture.html").await;
        let waiting = fixture.result(
            "browser.wait",
            json!({"tab":tab,"url":"**/never","timeout":30000}),
            CancellationToken::new(),
        );
        let close = async {
            fixture.wait_until_busy(&tab).await;
            fixture.call("browser.close", json!({"tab":tab})).await;
        };
        let ((code, failure), ()) = tokio::join!(waiting, close);
        assert_eq!(code, 1, "{failure}");
        assert_eq!(failure["error"]["code"], "browser_lost");
        assert_eq!(failure["error"]["details"]["action"], "not_started");
        assert_eq!(
            fixture.lifecycle("status").await,
            json!({"conversations":[]})
        );
        fixture
    })
    .await;
}
