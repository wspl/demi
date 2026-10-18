//! Starting with the computer (`runner.md` § Starting with the computer):
//! the runner of an installer's installation keeps its registration with
//! the system's service manager and removes it last as it goes; its exit
//! status says whether a new start may cure what ended it; `run start`
//! has the manager start the runner; where no registration can be made the
//! hello says why; and the runner sets what a terminal would have. The
//! manager's commands go to the harness's stand-in, which records them.

#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

use demi_backend_remote_host::testing::{
    DetachedRunners, RunnerProcessOptions, SERVICE_MANAGER_CALLS, executable, login_shell_stand_in,
    runner_binary, service_manager_stand_in,
};
use demi_host_interface::SpawnEnv;
use demi_runner_protocol::{
    SERVICE_MANAGER_STAND_IN, SUPERVISED,
    console::{HANDED_OVER, NO_AUTOSTART},
    wire::{self, AutostartUnavailable, Inbound, OutputStream, Outbound},
};
use futures_util::StreamExt as _;

use crate::Host;

/// A runner of an installer's installation, whose executable lies in its
/// `releases/`.
fn installed() -> RunnerProcessOptions {
    RunnerProcessOptions {
        release: Some("installed".into()),
        ..RunnerProcessOptions::default()
    }
}

/// The installation's directory name, which names its registration.
fn id(state: &Path) -> String {
    state.file_name().unwrap().to_string_lossy().into_owned()
}

/// The file of the registration of the installation in `state` under
/// `home`.
fn registration(home: &Path, state: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/LaunchAgents")
            .join(format!("sh.demi.runner.{}.plist", id(state)))
    } else {
        home.join(".config/systemd/user")
            .join(format!("demi-runner-{}.service", id(state)))
    }
}

/// The user's `gui` domain, which launchd names by the user's id.
fn domain() -> String {
    let uid = std::process::Command::new("id").arg("-u").output().unwrap();
    format!("gui/{}", String::from_utf8_lossy(&uid.stdout).trim())
}

/// The service-manager commands the runners under `home` gave.
fn calls(home: &Path) -> String {
    std::fs::read_to_string(home.join(SERVICE_MANAGER_CALLS)).unwrap_or_default()
}

/// Waits until the runner process has exited; its exit code.
async fn exit_code_now(host: &mut Host) -> Option<i32> {
    while host.process.running() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    host.process.exit_code()
}

/// Runs `demi-runner <action>` for the installation in `state`; its exit
/// code.
async fn manage(action: &str, state: &Path) -> Option<i32> {
    tokio::process::Command::new(runner_binary())
        .arg(action)
        .env("DEMI_HOME", state)
        .output()
        .await
        .unwrap()
        .status
        .code()
}

/// Waits until the runner has exited while the backend end answers the
/// close of its connection; its exit code.
async fn exit_code(host: &mut Host) -> Option<i32> {
    let socket = &mut host.socket;
    let process = &mut host.process;
    let answered = async { while socket.next().await.is_some() {} };
    let exited = async {
        while process.running() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    tokio::join!(answered, exited);
    host.process.exit_code()
}

/// Each start makes sure of the registration: the first writes it and has
/// the manager load it, one that finds it in place changes nothing, and
/// one that finds it naming another installation writes and loads it
/// again. It runs the installation's launcher with `run --log`, at login,
/// and again after a failure, every 10 s at most. Before the rule, a
/// runner wrote none.
// About a second: the runner starts three times.
#[tokio::test]
async fn a_runner_keeps_its_installations_registration_with_the_service_manager() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start_with(installed()).await.online().await;
        let (home, state) = (host.home().to_owned(), host.state());
        let file = registration(&home, &state);
        let launcher = state.join("run");
        let written = std::fs::read_to_string(&file).unwrap_or_else(|error| {
            panic!("{}: {error}\n{}", file.display(), host.process.output())
        });
        if cfg!(target_os = "macos") {
            for expected in [
                format!("<string>sh.demi.runner.{}</string>", id(&state)),
                format!(
                    "<array>\n\t\t<string>{}</string>\n\t\t<string>run</string>\n\t\t<string>--log</string>\n\t</array>",
                    launcher.display()
                ),
                "<key>EnvironmentVariables</key>\n\t<dict>\n\t\t<key>DEMI_SUPERVISED</key>\n\t\t<string>1</string>\n\t</dict>".into(),
                "<key>RunAtLoad</key>\n\t<true/>".into(),
                "<key>KeepAlive</key>\n\t<dict>\n\t\t<key>SuccessfulExit</key>\n\t\t<false/>\n\t</dict>".into(),
                "<key>ThrottleInterval</key>\n\t<integer>10</integer>".into(),
                "<key>ProcessType</key>\n\t<string>Interactive</string>".into(),
            ] {
                assert!(written.contains(&expected), "{expected}\nin\n{written}");
            }
            let bootstrap = format!("launchctl bootstrap {} {}", domain(), file.display());
            assert_eq!(calls(&home).matches(&bootstrap).count(), 1, "{}", calls(&home));
        } else {
            for expected in [
                format!("ExecStart=\"{}\" run --log\nEnvironment=DEMI_SUPERVISED=1\n", launcher.display()),
                "Restart=on-failure\nRestartSec=10\n".into(),
                "WantedBy=default.target\n".into(),
            ] {
                assert!(written.contains(&expected), "{expected}\nin\n{written}");
            }
            let said = calls(&home);
            assert_eq!(said.matches("systemctl --user daemon-reload").count(), 1, "{said}");
            assert!(
                said.contains(&format!("systemctl --user enable demi-runner-{}.service", id(&state))),
                "{said}"
            );
        }

        // In place, it stays as it is.
        let before = calls(&home);
        host.restart().await;
        host.welcome(&[crate::CONVERSATION]).await;
        let again = calls(&home)[before.len()..].to_owned();
        assert!(!again.contains("bootstrap") && !again.contains("daemon-reload"), "{again}");

        // Naming another installation, it is written again; launchd reads
        // it at the next login, since loading it now would stop the
        // manager's runner, this one.
        std::fs::write(&file, written.replace(&*launcher.to_string_lossy(), "/elsewhere/run")).unwrap();
        let before = calls(&home);
        host.restart().await;
        host.welcome(&[crate::CONVERSATION]).await;
        assert_eq!(std::fs::read_to_string(&file).unwrap(), written);
        let again = calls(&home)[before.len()..].to_owned();
        if cfg!(target_os = "macos") {
            assert!(!again.contains("launchctl bootout"), "{again}");
        } else {
            assert!(again.contains("systemctl --user daemon-reload"), "{again}");
        }
        host.close().await;
    })
    .await
    .unwrap();
}

/// A runner whose device its backend no longer knows exits 0, since a new
/// start would come to the same, so a service manager does not start it
/// again in a loop; a second copy is the busy-command test's in
/// `backend-remote-host`. Before the rule, it exited 1.
#[tokio::test]
async fn a_refused_runner_exits_with_0() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await;
        host.send(Inbound::HelloError {
            code: wire::HelloErrorCode::UnknownDevice,
            reason: "unknown device".into(),
        })
        .await;
        assert_eq!(exit_code(&mut host).await, Some(0), "{}", host.process.output());
    })
    .await
    .unwrap();
}

/// `run start` of a registered installation has the service manager start
/// the runner, through the installation's launcher, so the runner is the
/// manager's. Before the rule, it started the runner itself.
// Under a second: the runner's backend refuses it at once.
#[tokio::test]
async fn run_start_has_the_service_manager_start_a_registered_runner() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let home = tempfile::tempdir().unwrap();
        let home = home.path();
        let state = home.join("installation");
        std::fs::create_dir(&state).unwrap();
        let runners = DetachedRunners::new();
        runners.watch(&state);
        let launcher = state.join("run");
        std::fs::write(
            &launcher,
            format!(
                "#!/bin/sh\n: > \"$DEMI_HOME/started-by-the-manager\"\nexec '{}' \"$@\" --backend http://127.0.0.1:9/\n",
                runner_binary().display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(
            &launcher,
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();
        let file = registration(home, &state);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "registered").unwrap();
        // The agent is loaded, as after a login.
        std::fs::write(home.join("loaded"), "").unwrap();

        let started = tokio::process::Command::new(runner_binary())
            .args(["start", "--backend", "http://127.0.0.1:9/"])
            .env("HOME", home)
            .env("SHELL", login_shell_stand_in(home))
            .env("DEMI_HOME", &state)
            .env(SERVICE_MANAGER_STAND_IN, service_manager_stand_in(home))
            .output()
            .await
            .unwrap();
        let said = String::from_utf8_lossy(&started.stdout);
        assert!(said.contains("The runner cannot connect"), "{said}{started:?}");
        assert!(state.join("started-by-the-manager").exists(), "{said}{}", calls(home));
        let asked = if cfg!(target_os = "macos") {
            format!("launchctl kickstart -k {}/sh.demi.runner.installation", domain())
        } else {
            "systemctl --user restart demi-runner-installation.service".into()
        };
        assert!(calls(home).contains(&asked), "{}", calls(home));
    })
    .await
    .unwrap();
}

/// `uninstall` removes the registration with the installation, and so
/// does the runner of a revoked device, so the service manager never
/// starts a runner that is gone. Before the rule, it stayed.
#[tokio::test]
async fn removing_a_runner_removes_its_registration() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start_with(installed()).await.online().await;
        let (home, state) = (host.home().to_owned(), host.state());
        let file = registration(&home, &state);
        assert!(file.exists(), "{}", host.process.output());
        let backend = async {
            assert!(matches!(host.frame().await, Outbound::Revoke {}));
            host.send(Inbound::Revoked { projects: vec![] }).await;
            while host.socket.next().await.is_some() {}
        };
        let uninstall = tokio::process::Command::new(runner_binary())
            .arg("uninstall")
            .env("HOME", &home)
            .env("DEMI_HOME", &state)
            .env(SERVICE_MANAGER_STAND_IN, service_manager_stand_in(&home))
            .output();
        let (uninstalled, ()) = tokio::join!(uninstall, backend);
        let uninstalled = uninstalled.unwrap();
        assert!(uninstalled.status.success(), "{uninstalled:?}");
        assert!(!state.exists());
        assert!(!file.exists(), "{uninstalled:?}");
        let unloaded = if cfg!(target_os = "macos") { "launchctl bootout" } else { "systemctl --user disable" };
        assert!(calls(&home).contains(unloaded), "{}", calls(&home));

        let mut host = Host::start_with(installed()).await.online().await;
        let (home, state) = (host.home().to_owned(), host.state());
        let file = registration(&home, &state);
        assert!(file.exists(), "{}", host.process.output());
        host.send(Inbound::Revoked { projects: vec![] }).await;
        assert_eq!(exit_code(&mut host).await, Some(0));
        assert!(!state.exists());
        assert!(!file.exists(), "{}", host.process.output());
    })
    .await
    .unwrap();
}

/// A runner that cannot register says why in its hello and on its
/// console, where an installer reads it; here its registration's
/// directory is a file.
// About a second: the runner starts twice.
#[tokio::test]
async fn a_runner_that_cannot_register_says_why_in_its_hello() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start_with(installed()).await.online().await;
        let reason = |hello: &Outbound| match hello {
            Outbound::Hello { runner, .. } => runner.autostart_unavailable.clone(),
            other => panic!("not a hello: {other:?}"),
        };
        assert_eq!(reason(&host.hello), None, "{}", host.process.output());
        let file = registration(host.home(), &host.state());
        let directory = file.parent().unwrap();
        std::fs::remove_dir_all(directory).unwrap();
        std::fs::write(directory, "not a directory").unwrap();
        host.restart().await;
        let said = reason(&host.hello).unwrap_or_else(|| panic!("no reason: {}", host.process.output()));
        let AutostartUnavailable::NotWritten { path, .. } = &said else {
            panic!("{said:?}");
        };
        assert_eq!(Path::new(path), file);
        // The console says it in English, the path from `~`, where an
        // installer reads it. Stopping the runner joins its console's
        // readers, which may still be appending what it printed before the
        // hello.
        host.stop().await;
        let output = host.process.output();
        let sentence = said.sentence(&host.home().to_string_lossy());
        assert!(sentence.contains("~/"), "{sentence}");
        assert!(output.contains(&format!("{NO_AUTOSTART}{sentence}")), "{output}");
    })
    .await
    .unwrap();
}

/// The runner the service manager did not start, such as the installer's,
/// hands the installation over once its registration works: it asks the
/// manager to start the installation's runner, says so in its log, where
/// the installer and `run start` read it, and exits 0; the manager's runner,
/// which `DEMI_SUPERVISED` names, connects and keeps it. Before the rule,
/// the installer's runner ran unsupervised until the next login.
// About a second: two runners start.
#[tokio::test]
async fn a_runner_the_manager_did_not_start_hands_over_to_the_managers() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let runners = DetachedRunners::new();
        let mut host = Host::start_with(RunnerProcessOptions {
            env: SpawnEnv::Overlay(BTreeMap::from([(SUPERVISED.to_owned(), None)])),
            ..installed()
        })
        .await;
        let (home, state) = (host.home().to_owned(), host.state());
        runners.watch(&state);
        // The hello is the manager's runner's: the first one ended.
        assert_eq!(exit_code_now(&mut host).await, Some(0), "{}", host.process.output());
        let output = host.process.output();
        assert!(output.contains(HANDED_OVER), "{output}");
        let asked = if cfg!(target_os = "macos") {
            format!("launchctl kickstart -k {}/sh.demi.runner.{}", domain(), id(&state))
        } else {
            format!("systemctl --user restart demi-runner-{}.service", id(&state))
        };
        assert!(calls(&home).contains(&asked), "{}", calls(&home));
        host.welcome(&[crate::CONVERSATION]).await;
        // The manager's runner holds the installation.
        assert_eq!(manage("status", &state).await, Some(0));
    })
    .await
    .unwrap();
}

/// Where the manager does not start the installation's runner, the runner
/// goes on unsupervised and says why.
#[tokio::test]
async fn a_runner_the_manager_refuses_goes_on_unsupervised() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start_with(RunnerProcessOptions {
            env: SpawnEnv::Overlay(BTreeMap::from([
                (SUPERVISED.to_owned(), None),
                ("DEMI_TEST_MANAGER_REFUSES".to_owned(), Some("1".to_owned())),
            ])),
            ..installed()
        })
        .await
        .online()
        .await;
        assert!(host.process.running());
        let output = host.process.output();
        assert!(output.contains("goes on unsupervised") && !output.contains(HANDED_OVER), "{output}");
        host.close().await;
    })
    .await
    .unwrap();
}

/// A runner whose installation is registered to another backend exits 0,
/// logged, since no start changes that. Before the rule, it exited 1, and
/// a service manager would start it again every ten seconds.
#[tokio::test]
async fn a_runner_whose_state_no_start_changes_exits_with_0() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let mut host = Host::start(BTreeMap::new()).await.online().await;
        let state = host.state();
        host.stop().await;
        std::fs::write(
            state.join("runner.json"),
            r#"{"backendUrl":"http://elsewhere.example.test/","deviceId":null}"#,
        )
        .unwrap();
        let again = host.process.command().output().await.unwrap();
        let said = String::from_utf8_lossy(&again.stderr);
        assert_eq!(again.status.code(), Some(0), "{said}");
        assert!(said.contains("registered to another backend"), "{said}");
    })
    .await
    .unwrap();
}

/// A login shell for the runner that sets what the user's shell files would,
/// `FROM_PROFILE` and a directory first on `PATH`, after `before`.
fn profile_shell(directory: &Path, before: &str) -> PathBuf {
    executable(
        &directory.join(format!("profile-shell-{}", before.len())),
        &format!(
            "#!/bin/sh\n{before}\nexport FROM_PROFILE=yes PATH=/from/profile:$PATH\nwhile [ \"$#\" -gt 1 ] && [ \"$1\" != -c ]; do shift; done\nexec /bin/sh -c \"$2\"\n"
        ),
    )
}

/// The environment a raw process the runner starts sees.
async fn device_environment(host: &mut Host) -> BTreeMap<String, String> {
    host.send(Inbound::Spawn {
        spawn_id: "process".into(),
        command: "/usr/bin/env".into(),
        args: None,
        cwd: None,
        env: None,
        inherit_env: None,
        kill_process_group: Some(true),
        descriptors: None,
    })
    .await;
    let mut stdout = Vec::new();
    loop {
        match host.frame().await {
            Outbound::SpawnExit { exit_code, .. } => {
                assert_eq!(exit_code, Some(0));
                break;
            }
            Outbound::SpawnOutput {
                stream: OutputStream::Stdout,
                bytes,
                ..
            } => stdout.extend(bytes.0),
            _ => {}
        }
    }
    String::from_utf8(stdout)
        .unwrap()
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect()
}

/// The device environment is the user's login environment, whoever
/// started the runner: what the login shell's files set reaches a process
/// the runner starts, `SHELL` names the shell in the hello, and on macOS
/// `LANG` is set to UTF-8 when the shell leaves it unset, from a region the
/// system has no locale for (`en_CN`) the first of its language. A login
/// shell that fails or takes too long leaves the runner's own environment,
/// and the log says why. Before the rule, a runner a service manager
/// started gave its jobs launchd's bare `PATH`.
// About two seconds: the runner starts three times, one shell waiting out
// a shortened limit.
#[tokio::test]
async fn the_device_environment_is_the_login_shells() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let shells = tempfile::tempdir().unwrap();
        let resolved = profile_shell(shells.path(), ":");
        let small = |shell: &Path, extra: &[(&str, &str)]| {
            let mut env = BTreeMap::from([
                ("SHELL".to_owned(), shell.to_string_lossy().into_owned()),
                ("PATH".to_owned(), "/usr/bin:/bin:/usr/sbin:/sbin".to_owned()),
                ("DEMI_APPLE_LOCALE".to_owned(), "en_CN".to_owned()),
            ]);
            env.extend(extra.iter().map(|(name, value)| ((*name).to_owned(), (*value).to_owned())));
            RunnerProcessOptions {
                env: SpawnEnv::Exactly(env),
                ..RunnerProcessOptions::default()
            }
        };
        let mut host = Host::start_with(small(&resolved, &[])).await.online().await;
        let env = device_environment(&mut host).await;
        assert_eq!(env.get("FROM_PROFILE").map(String::as_str), Some("yes"), "{env:?}");
        assert!(env["PATH"].starts_with("/from/profile:"), "{env:?}");
        match &host.hello {
            Outbound::Hello { runner, .. } => {
                assert_eq!(runner.shell.as_deref(), Some(&*resolved.to_string_lossy()))
            }
            other => panic!("not a hello: {other:?}"),
        }
        if cfg!(target_os = "macos") {
            let lang = env.get("LANG").cloned().unwrap_or_default();
            assert!(lang.starts_with("en_") && lang.ends_with(".UTF-8") && lang != "en_CN.UTF-8", "{lang}");
        }
        host.close().await;

        for (shell, said, extra) in [
            (profile_shell(shells.path(), "sleep 30"), "took longer", ("DEMI_LOGIN_SHELL_TIMEOUT_MS", "300")),
            (profile_shell(shells.path(), "exit 3"), "ended with", ("UNUSED", "")),
        ] {
            let mut host = Host::start_with(small(&shell, &[extra])).await.online().await;
            let env = device_environment(&mut host).await;
            assert_eq!(env.get("FROM_PROFILE"), None, "{env:?}");
            let output = host.process.output();
            assert!(
                output.contains("the device environment is the runner's own") && output.contains(said),
                "{output}"
            );
            host.close().await;
        }
    })
    .await
    .unwrap();
}
