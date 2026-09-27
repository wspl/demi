#![cfg(unix)]

use std::{collections::HashSet, os::unix::fs::PermissionsExt, path::PathBuf, time::Duration};

use demi_commands::browser::{BrowserError, LaunchOptions, with_browser};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use tokio_util::sync::CancellationToken;

struct Process {
    pid: i32,
    parent: i32,
    group: i32,
}

/// Snapshot OS process relationships independently of the browser's cleanup
/// implementation: every process, without its threads, from the process
/// table as sysinfo reads it.
fn processes() -> Vec<Process> {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().without_tasks(),
    );
    system
        .processes()
        .values()
        .filter_map(|process| {
            let pid = i32::try_from(process.pid().as_u32()).unwrap();
            // sysinfo does not report a process's group; the system call
            // does, and fails only for a process that has ended since.
            let group = unsafe { libc::getpgid(pid) };
            (group != -1).then(|| Process {
                pid,
                parent: process
                    .parent()
                    .map_or(0, |parent| i32::try_from(parent.as_u32()).unwrap()),
                group,
            })
        })
        .collect()
}

/// Chrome refuses root on Linux with its sandbox, which Demi keeps: the
/// browser's error says to run the runner as an ordinary user, rather than
/// passing on Chrome's advice to drop the sandbox (`browser.md` § Native
/// driver). The launcher refuses as Chrome does, and says so and exits while
/// this test holds the runtime's only thread, as a loaded Host may: the
/// launch then finds the message and the exit at once. When it picked one of
/// the two at random, it lost the message about every other time, so the
/// launch runs eight times.
#[tokio::test]
async fn a_launch_as_root_says_to_run_the_runner_as_an_ordinary_user() {
    let directory = tempfile::tempdir().unwrap();
    let launcher = directory.path().join("root-chrome");
    let started = directory.path().join("root-chrome.pid");
    let go = directory.path().join("root-chrome.go");
    std::fs::write(
        &launcher,
        "#!/bin/sh
echo $$ > \"$0.pid\"
until [ -e \"$0.go\" ]; do :; done
echo '[1:1:0927/010848.716678:ERROR:content/browser/zygote_host/zygote_host_impl_linux.cc:102] Running as root without --no-sandbox is not supported. See https://crbug.com/638180.' >&2
exit 1
",
    )
    .unwrap();
    std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o700)).unwrap();
    let locale = demi_command_service::protocol::CommandLocale {
        time_zone: "UTC".into(),
        languages: vec!["en-US".into()],
    };
    for _ in 0..8 {
        let (result, ()) = tokio::join!(
            with_browser(
                LaunchOptions::pinned(launcher.clone(), locale.clone()).unwrap(),
                CancellationToken::new(),
                |_| async { Ok(()) },
            ),
            async {
                let pid: libc::id_t = tokio::time::timeout(Duration::from_secs(5), async {
                    loop {
                        if let Ok(recorded) = tokio::fs::read_to_string(&started).await
                            && let Ok(pid) = recorded.trim().parse()
                        {
                            break pid;
                        }
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                })
                .await
                .expect("the launcher starts");
                std::fs::write(&go, "").unwrap();
                // Until the launcher has exited, without reaping it.
                let mut exited: libc::siginfo_t = unsafe { std::mem::zeroed() };
                let waited = unsafe {
                    libc::waitid(libc::P_PID, pid, &mut exited, libc::WEXITED | libc::WNOWAIT)
                };
                assert_eq!(waited, 0, "{}", std::io::Error::last_os_error());
                std::fs::remove_file(&started).unwrap();
                std::fs::remove_file(&go).unwrap();
            }
        );
        let error = result.expect_err("the launch fails");
        assert!(matches!(error, BrowserError::Root), "{error:?}");
        assert!(error.to_string().contains("run the runner as an ordinary user"), "{error}");
    }
}

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
                launcher,
                demi_command_service::protocol::CommandLocale {
                    time_zone: "UTC".into(),
                    languages: vec!["en-US".into()]
                }
            )
            .unwrap(),
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

/// Verify actual Chrome helpers and the profile across every retirement entrance.
#[tokio::test]
#[ignore = "requires DEMI_TEST_CHROME pointing to an installed Chrome for Testing release"]
async fn chrome_process_tree_and_profile_retire_together() {
    let executable =
        PathBuf::from(std::env::var_os("DEMI_TEST_CHROME").expect("installed Chrome executable"));
    for mode in ["success", "failure", "cancel", "killed"] {
        let directory = tempfile::tempdir().unwrap();
        let fixture = directory.path().join("retirement.html");
        std::fs::write(&fixture, "<!doctype html><h1>Retirement</h1>").unwrap();
        let fixture = url::Url::from_file_path(fixture).unwrap();
        let chrome = executable.clone();
        let stop = CancellationToken::new();
        let cancel = stop.clone();
        let mut observed = None;
        let observation = &mut observed;
        let result = with_browser(
            LaunchOptions::pinned(
                executable.clone(),
                demi_command_service::protocol::CommandLocale {
                    time_zone: "UTC".into(),
                    languages: vec!["en-US".into()],
                },
            )
            .unwrap(),
            stop,
            |browser| async move {
                browser
                    .open(
                        fixture.as_str(),
                        &CancellationToken::new(),
                        Duration::from_secs(5),
                    )
                    .await?;
                let mut system = System::new();
                system.refresh_processes_specifics(
                    ProcessesToUpdate::All,
                    true,
                    ProcessRefreshKind::nothing().with_exe(UpdateKind::Always),
                );
                let main = system
                    .processes()
                    .values()
                    .find(|process| {
                        process.parent() == Some(sysinfo::Pid::from_u32(std::process::id()))
                            && process.exe() == Some(chrome.as_path())
                    })
                    .expect("the test owns Chrome's direct child");
                let leader = i32::try_from(main.pid().as_u32()).unwrap();
                let profile = chrome_profiles()
                    .remove(&leader)
                    .expect("the test owns Chrome's profile");
                assert!(profile.is_dir());
                // Every user shares `/tmp`: only this one reads the profile.
                assert_eq!(
                    std::fs::metadata(&profile).unwrap().permissions().mode() & 0o777,
                    0o700,
                    "{}",
                    profile.display()
                );
                // Chrome links its process-singleton socket from the profile.
                let socket = std::fs::read_link(profile.join("SingletonSocket"))
                    .expect("Chrome links its socket from the profile");
                let snapshot = processes();
                assert_eq!(
                    snapshot
                        .iter()
                        .find(|process| process.pid == leader)
                        .unwrap()
                        .group,
                    leader
                );
                let installation = chrome
                    .ancestors()
                    .find(|path| path.extension().is_some_and(|extension| extension == "app"))
                    .or_else(|| chrome.parent())
                    .unwrap();
                let candidates: Vec<_> = system
                    .processes()
                    .iter()
                    .filter(|(_, process)| {
                        process
                            .exe()
                            .is_some_and(|exe| exe.starts_with(installation))
                    })
                    .map(|(pid, _)| *pid)
                    .collect();
                system.refresh_processes_specifics(
                    ProcessesToUpdate::Some(&candidates),
                    false,
                    ProcessRefreshKind::nothing().with_environ(UpdateKind::Always),
                );
                let mut marker = std::ffi::OsString::from("DEMI_BROWSER_PROFILE=");
                marker.push(&profile);
                let mut descendants: HashSet<i32> = system
                    .processes()
                    .values()
                    .filter(|process| process.environ().contains(&marker))
                    .map(|process| i32::try_from(process.pid().as_u32()).unwrap())
                    .collect();
                // Linux Chrome rewrites its original argv/envp memory as its
                // process title, so /proc may no longer expose its marker. The
                // direct child and SingletonLock already identify the leader;
                // inherited markers additionally identify detached helpers.
                descendants.insert(leader);
                loop {
                    let previous = descendants.len();
                    for process in &snapshot {
                        if descendants.contains(&process.parent) || process.group == leader {
                            descendants.insert(process.pid);
                        }
                    }
                    if previous == descendants.len() {
                        break;
                    }
                }
                assert!(
                    descendants.len() > 1,
                    "Chrome must have spawned helpers on {mode}"
                );
                *observation = Some((leader, descendants, profile, socket));
                match mode {
                    "failure" => Err(BrowserError::Configuration("injected work failure".into())),
                    "cancel" => {
                        cancel.cancel();
                        std::future::pending().await
                    }
                    "killed" => {
                        assert_eq!(unsafe { libc::kill(leader, libc::SIGKILL) }, 0);
                        std::future::pending().await
                    }
                    _ => Ok(()),
                }
            },
        )
        .await;
        match mode {
            "success" => assert!(result.is_ok(), "{result:?}"),
            "failure" => assert!(
                matches!(result, Err(BrowserError::Configuration(_))),
                "{result:?}"
            ),
            "cancel" => assert!(matches!(result, Err(BrowserError::Cancelled)), "{result:?}"),
            "killed" => assert!(
                matches!(
                    result,
                    Err(BrowserError::Closed | BrowserError::Connection(_))
                ),
                "{result:?}"
            ),
            _ => unreachable!(),
        }
        let (leader, descendants, profile, socket) = observed.unwrap();
        let remaining = processes();
        assert!(
            !remaining
                .iter()
                .any(|process| descendants.contains(&process.pid) || process.group == leader),
            "Chrome descendants survived {mode}"
        );
        assert!(
            !profile.exists(),
            "profile survived {mode}: {}",
            profile.display()
        );
        let socket_directory = socket.parent().unwrap();
        assert!(
            !socket_directory.exists(),
            "Chrome's socket directory survived {mode}: {}",
            socket_directory.display()
        );
    }
}

/// The service's own temporary directory can be long: a Mac's is about 49
/// characters, and a user can set any. Chrome's profile, with the
/// process-singleton socket Chrome makes inside it, stays in the short
/// profile base anyway, so the socket's path keeps within its limit and
/// Chrome starts (`browser.md` § Native driver). Under this directory of at
/// least 90 characters the socket's path would take at least 155 bytes, past
/// Linux's 107. Chrome writes nothing of its own in the service's home but
/// the user's certificate database: its crash reports go with the profile.
/// About 5 s: the service's `TMPDIR` and home differ from this test
/// process's only when the service runs as a program of its own, which finds
/// Chrome only where it installs it (1.5 s); starting Chrome and opening a
/// tab take 2 s, and retiring it 1.5 s.
#[tokio::test]
#[ignore = "requires DEMI_TEST_CHROME; starts the service program with its own home and a long TMPDIR"]
async fn chrome_starts_and_keeps_out_of_the_services_home_whatever_its_temporary_directory() {
    use demi_command_service::{
        protocol::{CommandCaller, CommandContext, CommandLocale, Invocation},
        testing::ServiceProcess,
    };
    use serde_json::json;
    let scratch = tempfile::tempdir().unwrap();
    let padding = 90usize
        .saturating_sub(scratch.path().as_os_str().len() + 1)
        .max(1);
    let temporary = scratch.path().join("t".repeat(padding));
    std::fs::create_dir(&temporary).unwrap();
    // The service installs Chrome under its home and finds it there.
    let home = tempfile::tempdir().unwrap();
    crate::families::install_chrome(&home.path().join(".demi/browsers")).await;
    tokio::time::timeout(Duration::from_secs(60), async {
        let service = ServiceProcess::start(
            env!("CARGO_BIN_EXE_demi-commands"),
            &["--command-service"],
            &[
                ("HOME", home.path().to_str().unwrap()),
                ("TMPDIR", temporary.to_str().unwrap()),
            ],
        )
        .await
        .unwrap();
        let invocation = |operation: &str, args| Invocation {
            operation: operation.into(),
            invocation_id: uuid::Uuid::new_v4().to_string(),
            context: CommandContext {
                conversation: "long-temporary-directory".into(),
                caller: CommandCaller::agent("agent-root"),
                locale: CommandLocale {
                    time_zone: "UTC".into(),
                    languages: vec!["en-US".into()],
                },
            },
            json: Some(true),
            edits: None,
            args,
            cwd: scratch.path().to_str().unwrap().into(),
            env: std::collections::BTreeMap::new(),
        };
        let (completion, _, stderr) = crate::commands::exchange(
            service.client(),
            &invocation("browser.open", json!({"url": "about:blank"})),
            false,
        )
        .await;
        assert_eq!(
            completion.exit_code,
            0,
            "{}",
            String::from_utf8_lossy(&stderr)
        );
        assert_eq!(
            chrome_profiles_of(service.id().unwrap()).len(),
            1,
            "the service's Chrome keeps its profile in {}",
            demi_commands::browser::profile_base().display()
        );
        let (completion, _, _) =
            crate::commands::exchange(service.client(), &invocation("release", json!({})), true)
                .await;
        assert_eq!(completion.exit_code, 0);
        assert!(service.shutdown().await.unwrap().success());
    })
    .await
    .unwrap();
    // Besides the installation, only what Chrome keeps for the user, and the
    // directories above it, are in the home (`browser.md` § Native driver):
    // on Linux the certificate database, on macOS the crash reports.
    let kept = if cfg!(target_os = "macos") {
        [".demi", "Library/Application Support/Google/Chrome for Testing"]
    } else {
        [".demi", ".local/share/pki"]
    };
    let written: Vec<_> = walk(home.path())
        .into_iter()
        .filter(|path| {
            !kept.iter().any(|kept| path.starts_with(kept) || std::path::Path::new(kept).starts_with(path))
        })
        .collect();
    assert!(written.is_empty(), "Chrome wrote in the service's home: {written:?}");
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

/// The profiles of the Chrome processes this test started, by main process.
fn chrome_profiles() -> std::collections::BTreeMap<i32, PathBuf> {
    chrome_profiles_of(std::process::id())
}

/// The profiles of the Chrome processes `parent` started, by main process,
/// read from the profiles' singleton locks rather than asked of the service.
fn chrome_profiles_of(parent: u32) -> std::collections::BTreeMap<i32, PathBuf> {
    let parent = i32::try_from(parent).unwrap();
    let children: HashSet<_> = processes()
        .into_iter()
        .filter(|process| process.parent == parent)
        .map(|process| process.pid)
        .collect();
    let mut profiles = std::collections::BTreeMap::new();
    // Chrome can replace its Linux argv with a single process-title string.
    // Its singleton lock records the profile owner without parsing that title.
    for entry in std::fs::read_dir(demi_commands::browser::profile_base()).unwrap() {
        let path = entry.unwrap().path();
        if !path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("demi-browser-")
        {
            continue;
        }
        let lock = match std::fs::read_link(path.join("SingletonLock")) {
            Ok(lock) => lock,
            // A profile that has not started, or has retired, owns no browser.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            // Another user's: on Unix every user's profiles share `/tmp`.
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => continue,
            Err(error) => panic!("read Chrome profile owner: {error}"),
        };
        let owner: i32 = lock
            .to_str()
            .unwrap()
            .rsplit_once('-')
            .unwrap()
            .1
            .parse()
            .unwrap();
        if children.contains(&owner) {
            profiles.insert(owner, path);
        }
    }
    profiles
}

#[tokio::test]
#[ignore = "requires DEMI_TEST_CHROME; exercises conversation retirement"]
async fn conversation_release_cancels_only_its_commands_and_retires_its_profile() {
    use serde_json::json;
    crate::families::with_browser_fixture(|first| async move {
        let mut second = first.clone();
        second.conversation = "second-conversation".into();
        let first_tab = first.open("fixture.html").await;
        let first_profiles = chrome_profiles();
        assert_eq!(first_profiles.len(), 1);
        let (&leader, profile) = first_profiles.first_key_value().unwrap();
        assert!(profile.is_dir());
        let second_tab = second.open("fixture.html").await;
        let profiles = chrome_profiles();
        assert_eq!(profiles.len(), 2);
        let second_profiles: std::collections::BTreeMap<_, _> = profiles
            .into_iter()
            .filter(|(pid, _)| *pid != leader)
            .collect();
        let descendants: HashSet<_> = processes()
            .iter()
            .filter(|process| process.group == leader)
            .map(|process| process.pid)
            .collect();
        assert!(descendants.len() > 1);
        let mut held = vec![first.conversation.clone(), second.conversation.clone()];
        held.sort();
        assert_eq!(
            first.lifecycle("status").await,
            json!({"conversations": held})
        );

        let waiting = first.result(
            "browser.wait",
            json!({"tab": first_tab, "url": "**/never", "timeout": 30000}),
            CancellationToken::new(),
        );
        tokio::pin!(waiting);
        let release = async {
            first.wait_until_busy(&first_tab).await;
            assert_eq!(first.lifecycle("release").await, json!({}));
        };
        let ((code, error), ()) = tokio::join!(&mut waiting, release);
        assert_eq!(code, 130, "{error}");
        assert_eq!(error["error"]["code"], "cancelled");
        assert!(!profile.exists());
        assert!(
            !processes()
                .iter()
                .any(|process| descendants.contains(&process.pid) || process.group == leader)
        );
        assert_eq!(chrome_profiles(), second_profiles);
        assert_eq!(
            first.lifecycle("status").await,
            json!({"conversations": [second.conversation]})
        );
        assert_eq!(
            second.call("browser.tabs", json!({})).await["tabs"][0]["id"],
            second_tab
        );
        assert_eq!(
            first.call("browser.tabs", json!({})).await["tabs"],
            json!([])
        );
        assert_eq!(first.lifecycle("release").await, json!({}));
        assert_eq!(second.lifecycle("release").await, json!({}));
        assert_eq!(
            first.lifecycle("status").await,
            json!({"conversations": []})
        );
        assert!(chrome_profiles().is_empty());
        assert!(second_profiles.values().all(|path| !path.exists()));
        first.clone()
    })
    .await;
}

#[tokio::test]
#[ignore = "requires DEMI_TEST_CHROME; verifies trusted invocation identity"]
async fn browser_uses_trusted_conversation_and_caller_despite_script_environment() {
    use serde_json::json;
    crate::families::with_browser_fixture(|mut first| async move {
        let mut second = first.clone();
        second.conversation = "other-conversation".into();
        second.caller = "other-agent".into();
        let second_tab = second.open("fixture.html").await;
        first
            .env
            .insert("DEMI_CONVERSATION_ID".into(), second.conversation.clone());
        first
            .env
            .insert("DEMI_AGENT_NODE_ID".into(), second.caller.clone());
        let first_tab = first.open("fixture.html").await;
        let tabs = first.call("browser.tabs", json!({})).await;
        assert_eq!(tabs["tabs"].as_array().unwrap().len(), 1);
        assert_eq!(tabs["tabs"][0]["id"], first_tab);
        assert_eq!(tabs["tabs"][0]["createdBy"]["nodeId"], first.caller);
        let (_, error) = first
            .result(
                "browser.info",
                json!({"tab": second_tab}),
                CancellationToken::new(),
            )
            .await;
        assert_eq!(error["error"]["code"], "tab_not_found");
        assert_eq!(
            second.call("browser.tabs", json!({})).await["tabs"][0]["id"],
            second_tab
        );
        first.call("browser.close", json!({"tab": first_tab})).await;
        assert_eq!(
            first.lifecycle("status").await,
            json!({"conversations": [second.conversation]})
        );
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
#[ignore = "requires DEMI_TEST_CHROME; verifies joined retirement after failed assertions"]
async fn fixture_assertions_retire_chrome_and_profiles_before_resuming_panic() {
    use futures_util::FutureExt;
    use std::{collections::BTreeMap, panic::AssertUnwindSafe, sync::Mutex};
    for harness in ["direct", "service"] {
        let recorded = Mutex::new(None::<(BTreeMap<i32, PathBuf>, HashSet<i32>)>);
        let fail = || {
            let profiles = chrome_profiles();
            assert_eq!(profiles.len(), 1);
            let snapshot = processes();
            let mut owned: HashSet<_> = profiles.keys().copied().collect();
            loop {
                let count = owned.len();
                for process in &snapshot {
                    if owned.contains(&process.parent) || profiles.contains_key(&process.group) {
                        owned.insert(process.pid);
                    }
                }
                if count == owned.len() {
                    break;
                }
            }
            assert!(
                owned.len() > profiles.len(),
                "Chrome helpers were not captured"
            );
            *recorded.lock().unwrap() = Some((profiles, owned));
            assert_eq!("actual", "expected", "deliberate fixture assertion");
        };
        let result = if harness == "direct" {
            AssertUnwindSafe(crate::fixture::with_fixture(|browser, base| async move {
                browser
                    .open(&base, &CancellationToken::new(), Duration::from_secs(10))
                    .await?;
                fail();
                Ok(())
            }))
            .catch_unwind()
            .await
        } else {
            AssertUnwindSafe(crate::families::with_browser_fixture(
                |fixture| async move {
                    fixture.open("repairs.html").await;
                    fail();
                    fixture
                },
            ))
            .catch_unwind()
            .await
        };
        let panic = result.expect_err("the harness must resume the assertion panic");
        assert!(
            panic
                .downcast_ref::<String>()
                .unwrap()
                .contains("deliberate fixture assertion")
        );
        let (profiles, owned) = recorded.into_inner().unwrap().unwrap();
        assert!(
            !processes()
                .iter()
                .any(|process| owned.contains(&process.pid)),
            "{harness} left a Chrome process or helper"
        );
        for profile in profiles.values() {
            assert!(
                !profile.exists(),
                "{harness} retained {}",
                profile.display()
            );
        }
    }
}

#[tokio::test]
#[ignore = "requires DEMI_TEST_CHROME; verifies the last-tab closing outcome"]
async fn last_tab_close_fails_its_running_command_as_browser_lost() {
    use serde_json::json;
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

#[tokio::test]
#[ignore = "requires installed pinned Chrome for Testing release"]
async fn a_new_open_recovers_after_chrome_crashes_without_replaying_old_tabs() {
    use serde_json::json;
    crate::families::with_browser_fixture(|fixture| async move {
        let old = fixture.open("cdp.html").await;
        fixture
            .call(
                "browser.cdp.send",
                json!({"tab":old,"method":"Debugger.enable","params":"{}"}),
            )
            .await;
        fixture
            .call(
                "browser.cdp.send",
                json!({"tab":old,"method":"Fetch.enable","params":"{}"}),
            )
            .await;
        // The fixture's Chrome runs from the service's own installation, not
        // from `DEMI_TEST_CHROME`; the one profile it holds names its main
        // process.
        let mut profiles = chrome_profiles();
        assert_eq!(profiles.len(), 1, "the fixture owns one Chrome");
        let (leader, profile) = profiles.pop_first().unwrap();
        assert_eq!(unsafe { libc::kill(leader, libc::SIGKILL) }, 0);
        tokio::time::timeout(Duration::from_secs(10), async {
            while profile.exists() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("crash retirement removes the profile");
        let new = fixture.open("cdp.html").await;
        assert_ne!(new, old);
        let tabs = fixture.call("browser.tabs", json!({})).await;
        assert_eq!(tabs["tabs"].as_array().unwrap().len(), 1);
        assert_eq!(tabs["tabs"][0]["id"], new);
        let (_, error) = fixture
            .result("browser.info", json!({"tab":old}), CancellationToken::new())
            .await;
        assert_eq!(error["error"]["code"], "tab_not_found", "{error}");
        fixture
    })
    .await;
}
