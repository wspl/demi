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

/// Snapshot OS process relationships independently of the browser's cleanup implementation.
fn processes() -> Vec<Process> {
    let output = std::process::Command::new("ps")
        .args(["-axo", "pid=,ppid=,pgid="])
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| {
            let values: Vec<i32> = line
                .split_whitespace()
                .map(|value| value.parse().unwrap())
                .collect();
            assert_eq!(values.len(), 3);
            Process {
                pid: values[0],
                parent: values[1],
                group: values[2],
            }
        })
        .collect()
}

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
                *observation = Some((leader, descendants, profile));
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
        let (leader, descendants, profile) = observed.unwrap();
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
    }
}


/// Wait until a concurrent browser command holds this fixture tab's admission.
async fn wait_until_busy(fixture: &crate::families::BrowserFixture, tab: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let (_, result) = fixture
                .result(
                    "browser.info",
                    serde_json::json!({"tab": tab}),
                    CancellationToken::new(),
                )
                .await;
            if result["error"]["code"] == "tab_busy" {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

/// Locate the test service's browser profiles without exposing diagnostic product APIs.
fn chrome_profiles() -> std::collections::BTreeMap<i32, PathBuf> {
    let parent = i32::try_from(std::process::id()).unwrap();
    let children: HashSet<_> = processes()
        .into_iter()
        .filter(|process| process.parent == parent)
        .map(|process| process.pid)
        .collect();
    let mut profiles = std::collections::BTreeMap::new();
    // Chrome can replace its Linux argv with a single process-title string.
    // Its singleton lock records the profile owner without parsing that title.
    for entry in std::fs::read_dir(std::env::temp_dir()).unwrap() {
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
#[ignore = "installs the pinned Chrome release and exercises conversation retirement"]
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
            wait_until_busy(&first, &first_tab).await;
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
#[ignore = "installs the pinned Chrome release and verifies trusted invocation identity"]
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
            wait_until_busy(&fixture, &tab).await;
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
        let executable = PathBuf::from(std::env::var_os("DEMI_TEST_CHROME").unwrap());
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
                    && process.exe() == Some(executable.as_path())
            })
            .expect("the fixture owns its Chrome child");
        let leader = i32::try_from(main.pid().as_u32()).unwrap();
        let profile = chrome_profiles()
            .remove(&leader)
            .expect("the fixture owns Chrome's profile");
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
