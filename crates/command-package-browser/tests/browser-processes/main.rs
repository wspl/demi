//! The `demi-browser` tests that read the whole process table of this test
//! process, in its own binary: each counts every Chrome the test process
//! started and every descendant left behind, so a Chrome another test of
//! the same process runs at the same time reads as theirs. In the `browser`
//! binary, whose tests run in parallel, they failed whenever their neighbours
//! had a browser open. Here they also take turns (`ONE_AT_A_TIME`).
//! They start the pinned Chrome for Testing and are ignored unless asked for:
//! `DEMI_TEST_CHROME=<chrome> cargo test --workspace --features
//! demi-runner/test-fixtures --test browser-processes -- --include-ignored`,
//! on Linux with `DEMI_TEST_CHROME_RUNTIME=<directory>` too, the pinned
//! Chrome runtime unpacked.
#![cfg(unix)]
// The service is `Send` and `Sync` deeper than the trait solver's default 128
// steps, as in the library (`src/lib.rs`).
#![recursion_limit = "256"]

#[path = "../browser/families/mod.rs"]
mod families;
#[path = "../browser/fixture.rs"]
mod fixture;
#[path = "../browser/processes.rs"]
mod processes;
#[path = "../browser/server.rs"]
mod server;

use std::{collections::HashSet, os::unix::fs::PermissionsExt, path::PathBuf, time::Duration};

use demi_command_package_browser_chrome::driver::{numbers::TabNumbers, operation::BrowserError};
use demi_command_package_browser_chrome::tabs::environment::{
    DirectoryBases, LaunchOptions, with_browser,
};
use demi_command_sdk::testing::counting_numbers;
use processes::{chrome_profiles_of, processes};
use serde_json::json;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use tokio_util::sync::CancellationToken;

/// Held by each test for its whole run: the tests count each other's
/// processes otherwise.
static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The profiles of the Chrome processes this test started, by main process.
fn chrome_profiles() -> std::collections::BTreeMap<i32, PathBuf> {
    chrome_profiles_of(std::process::id(), &DirectoryBases::host().profiles)
}

/// Verify actual Chrome helpers and the profile across every retirement entrance.
#[tokio::test]
#[ignore = "requires DEMI_TEST_CHROME pointing to an installed Chrome for Testing release"]
async fn chrome_process_tree_and_profile_retire_together() {
    let _turn = ONE_AT_A_TIME.lock().await;
    let installation = demi_command_package_browser_chrome::driver::testing::installation();
    for mode in ["success", "failure", "cancel", "killed"] {
        let directory = tempfile::tempdir().unwrap();
        let fixture = directory.path().join("retirement.html");
        std::fs::write(&fixture, "<!doctype html><h1>Retirement</h1>").unwrap();
        let fixture = url::Url::from_file_path(fixture).unwrap();
        let chrome = installation.executable.clone();
        let stop = CancellationToken::new();
        let cancel = stop.clone();
        let mut observed = None;
        let observation = &mut observed;
        let result = with_browser(
            LaunchOptions::pinned(
                installation.clone(),
                demi_command_protocol::CommandLocale {
                    time_zone: "UTC".into(),
                    languages: vec!["en-US".into()],
                },
            )
            .unwrap(),
            TabNumbers::new(counting_numbers(), "conversation".into()),
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
                // Chrome links its process-singleton socket from the profile
                // into its temporary directory, the environment's runtime
                // directory.
                let socket = std::fs::read_link(profile.join("SingletonSocket"))
                    .expect("Chrome links its socket from the profile");
                let runtime = socket
                    .parent()
                    .and_then(std::path::Path::parent)
                    .expect("the socket's directory is in the runtime directory")
                    .to_owned();
                assert_eq!(
                    runtime.parent(),
                    Some(DirectoryBases::host().runtime.as_path())
                );
                // Every user shares the bases: only this one reads either.
                for directory in [&profile, &runtime] {
                    assert_eq!(
                        std::fs::metadata(directory).unwrap().permissions().mode() & 0o777,
                        0o700,
                        "{}",
                        directory.display()
                    );
                }
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
                *observation = Some((leader, descendants, profile, runtime));
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
        let (leader, descendants, profile, runtime) = observed.unwrap();
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
        assert!(
            !runtime.exists(),
            "Chrome's temporary directory survived {mode}: {}",
            runtime.display()
        );
    }
}

#[tokio::test]
#[ignore = "requires DEMI_TEST_CHROME; exercises conversation retirement"]
async fn conversation_release_cancels_only_its_commands_and_retires_its_profile() {
    let _turn = ONE_AT_A_TIME.lock().await;
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
        // A released conversation's next tab takes a number it has not given
        // out before.
        assert_ne!(first.open("fixture.html").await, first_tab);
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
#[ignore = "requires DEMI_TEST_CHROME; verifies joined retirement after failed assertions"]
async fn fixture_assertions_retire_chrome_and_profiles_before_resuming_panic() {
    let _turn = ONE_AT_A_TIME.lock().await;
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
#[ignore = "requires installed pinned Chrome for Testing release"]
async fn a_new_open_recovers_after_chrome_crashes_without_replaying_old_tabs() {
    let _turn = ONE_AT_A_TIME.lock().await;
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
