//! The public installation routes (`web-api.md` § Resource index): the
//! installer of the backend's current runner release, and each release's
//! executables. An installer run here installs this build's runner for its
//! backend and starts it, and the runner then waits to be paired; the test
//! drains every runner it installed.

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Output;

use demi_backend_remote_host::testing::{PAIRING_CODE, runner_binary};
use demi_command_protocol::{TARGETS, VERSION, host_target};
use demi_provider_common::testing::MockVendor;
use demi_runner_protocol::release::release_file;
use demi_runner_protocol::wire;
use demi_web_api_protocol::devices::ClaimedDevice;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::conversations::{FIRST, Socket, anthropic, choose, create, tool_result, tool_use};
use crate::support::{Harness, TestBackend, answer, eventually};

fn sha(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// A server release whose runner releases all carry `program`: its root,
/// with the releases' manifests in `runners/` and no command package, and
/// its files beside, where each target's runner is `program`.
struct Releases {
    directory: tempfile::TempDir,
    program: PathBuf,
    /// The program's size and digest, which every release names.
    artifact: Value,
}

impl Releases {
    fn new(program: PathBuf) -> Self {
        let bytes = std::fs::read(&program).unwrap();
        let artifact = json!({ "sha256": sha(&bytes), "size": bytes.len() });
        Self {
            directory: tempfile::Builder::new()
                .prefix("demi-releases-")
                .tempdir()
                .unwrap(),
            program,
            artifact,
        }
    }

    /// The server release's root.
    fn path(&self) -> PathBuf {
        self.directory.path().join("root")
    }

    fn runners(&self) -> PathBuf {
        self.path().join("runners")
    }

    /// Publishes a release named by `name`'s digest, which the top-level
    /// manifest names from now on. Each target's file links the program
    /// rather than copying it.
    fn publish(&self, name: &str) -> String {
        let release = sha(name.as_bytes());
        let files = self.directory.path().join("files");
        std::fs::create_dir_all(&files).unwrap();
        std::fs::create_dir_all(self.path().join("commands")).unwrap();
        crate::support::write_server_release(&self.path(), &files);
        for target in TARGETS {
            let file = files.join(release_file("demi-runner", target));
            if !file.exists() {
                std::os::unix::fs::symlink(&self.program, &file).unwrap();
            }
        }
        std::fs::create_dir_all(self.runners().join(&release)).unwrap();
        // Test-only: every target names this machine's runner.
        let targets: serde_json::Map<String, Value> = TARGETS
            .iter()
            .map(|target| ((*target).to_owned(), self.artifact.clone()))
            .collect();
        let manifest = json!({
            "release": release,
            "wire": wire::VERSION,
            "commandProtocol": VERSION,
            "targets": targets,
        })
        .to_string();
        std::fs::write(
            self.runners().join(&release).join("manifest.json"),
            &manifest,
        )
        .unwrap();
        std::fs::write(self.runners().join("manifest.json"), &manifest).unwrap();
        release
    }
}

/// The installations the test made under `home`, each drained when the test
/// ends, however it ends.
struct Installations {
    home: tempfile::TempDir,
    states: std::cell::RefCell<Vec<PathBuf>>,
}

impl Installations {
    fn new() -> Self {
        Self {
            home: tempfile::Builder::new()
                .prefix("demi-install-home-")
                .tempdir()
                .unwrap(),
            states: std::cell::RefCell::default(),
        }
    }

    /// The installation state of the backend at `url`.
    fn state(&self, url: &str) -> PathBuf {
        let state = self
            .home
            .path()
            .join(".demi/instances")
            .join(sha(url.as_bytes()));
        self.states.borrow_mut().push(state.clone());
        state
    }

    /// Fetches the backend's installer and runs it.
    async fn install(&self, backend: &TestBackend, extra: &[(&str, &str)]) -> Output {
        let script = self.script(backend).await;
        self.run(&script, extra).await
    }

    /// Fetches the backend's installer and runs it from a shell whose file
    /// mode creation mask is `umask`.
    async fn install_with_umask(&self, backend: &TestBackend, umask: &str) -> Output {
        let script = self.script(backend).await;
        self.shell()
            .arg("-c")
            .arg(format!("umask {umask} && exec sh \"$0\""))
            .arg(&script)
            .output()
            .await
            .unwrap()
    }

    /// The backend's installer, saved in the home.
    async fn script(&self, backend: &TestBackend) -> PathBuf {
        let script = reqwest::get(format!("{}/install.sh", backend.url))
            .await
            .unwrap();
        assert_eq!(script.status(), StatusCode::OK);
        let path = self
            .home
            .path()
            .join(format!("install-{}.sh", backend.address().port()));
        std::fs::write(&path, script.bytes().await.unwrap()).unwrap();
        path
    }

    async fn run(&self, script: &Path, extra: &[(&str, &str)]) -> Output {
        self.shell()
            .arg(script)
            .envs(extra.iter().copied())
            .output()
            .await
            .unwrap()
    }

    /// A shell for an installer, with the home and without an installation ID.
    fn shell(&self) -> tokio::process::Command {
        let mut shell = tokio::process::Command::new("sh");
        shell
            .env("HOME", self.home.path())
            .env_remove("DEMI_INSTALLATION_ID");
        shell
    }
}

impl Drop for Installations {
    fn drop(&mut self) {
        for state in self.states.borrow().iter() {
            let launcher = state.join("run");
            if launcher.exists() {
                // A runner that never started has nothing to drain.
                let _ = std::process::Command::new(launcher).arg("drain").output();
            }
        }
    }
}

/// The installation's active runner: its endpoint and its release.
fn active(state: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(state.join("active.json")).unwrap()).unwrap()
}

fn succeeded(output: &Output) -> String {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

// Several seconds: the installer runs for real, and each of its three installs
// downloads this build's runner (170 MB) from its backend, verifies it and
// starts it; the upgrade drains one.
#[tokio::test]
async fn an_installer_keeps_each_backend_apart_reuses_a_release_and_upgrades_only_its_own_runner() {
    let releases = Releases::new(runner_binary());
    let initial = releases.publish("initial");
    let a = Harness::new()
        .with_runner_releases(releases.path())
        .start()
        .await;
    let b = Harness::new()
        .with_runner_releases(releases.path())
        .start()
        .await;
    let installations = Installations::new();
    let state_a = installations.state(&format!("{}/", a.url));
    let state_b = installations.state(&format!("{}/", b.url));

    // The Windows installer names the release's Windows runners.
    let windows = reqwest::get(format!("{}/install.ps1", a.url))
        .await
        .unwrap();
    assert_eq!(windows.status(), StatusCode::OK);
    assert_eq!(
        windows.headers()["content-type"],
        "text/plain; charset=utf-8"
    );
    assert!(
        windows
            .text()
            .await
            .unwrap()
            .contains("aarch64-pc-windows-msvc")
    );

    succeeded(&installations.install(&a, &[]).await);
    succeeded(&installations.install(&b, &[]).await);
    let first_a = active(&state_a);
    let first_b = active(&state_b);
    assert_ne!(first_a["endpoint"], first_b["endpoint"]);
    assert_eq!(first_a["release"], json!(initial));

    // Installed again, the running runner stays.
    let again = succeeded(&installations.install(&a, &[]).await);
    assert!(again.contains("already running"), "{again}");
    assert_eq!(active(&state_a)["endpoint"], first_a["endpoint"]);

    // One backend's installer cannot take another backend's installation.
    let script_b = installations
        .home
        .path()
        .join(format!("install-{}.sh", b.address().port()));
    let registration_a = sha(format!("{}/", a.url).as_bytes());
    let collision = installations
        .run(
            &script_b,
            &[("DEMI_INSTALLATION_ID", registration_a.as_str())],
        )
        .await;
    assert_eq!(collision.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&collision.stderr).contains("another backend"));

    // A new release drains and replaces this backend's runner alone, and the
    // old release's runner stays downloadable.
    let upgraded = releases.publish("upgraded");
    succeeded(&installations.install(&a, &[]).await);
    let old = reqwest::Client::new()
        .request(
            Method::HEAD,
            format!(
                "{}/runner-artifacts/{initial}/{}/demi-runner",
                a.url,
                host_target()
            ),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(old.status(), StatusCode::OK);
    assert_eq!(
        old.headers()["cache-control"],
        "public, max-age=31536000, immutable"
    );
    assert_eq!(active(&state_a)["release"], json!(upgraded));
    assert_ne!(active(&state_a)["endpoint"], first_a["endpoint"]);
    assert_eq!(active(&state_b)["endpoint"], first_b["endpoint"]);
    drop(installations);
    a.close().await;
    b.close().await;
}

/// A file's permission bits.
fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

// Several seconds (8 s here under load): the installer downloads this build's
// runner (170 MB) from its backend, verifies it and starts it; a scripted
// model then runs one job on the paired runner.
#[tokio::test]
async fn an_installed_runner_works_with_the_mask_of_the_shell_that_ran_the_installer() {
    let releases = Releases::new(runner_binary());
    releases.publish("initial");
    let harness = Harness::new().with_runner_releases(releases.path());
    let (backend, master) = harness.start_set_up().await;
    let installations = Installations::new();
    let state = installations.state(&format!("{}/", backend.url));

    // The user's shell lets the group write, as some systems' shells do.
    succeeded(&installations.install_with_umask(&backend, "002").await);
    // The installation stays the user's alone: its log holds the pairing code.
    assert_eq!(mode(&state), 0o700);
    assert_eq!(mode(&state.join("runner.log")), 0o600);

    let log = state.join("runner.log");
    let mut code = None;
    eventually("the installed runner prints its pairing code", || {
        code = std::fs::read_to_string(&log)
            .unwrap()
            .lines()
            .find_map(|line| line.strip_prefix(PAIRING_CODE))
            .map(|code| code.trim().to_owned());
        let printed = code.is_some();
        async move { printed }
    })
    .await;
    let claimed = backend
        .post("/api/devices/claim", Some(&master), json!({ "code": code }))
        .await;
    assert_eq!(
        claimed.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&claimed.body)
    );
    let device = claimed.json::<ClaimedDevice>().device;
    backend
        .until_online(&master, device.id.as_str(), true)
        .await;

    // The agent's job reports the user's mask, and a file it makes has the
    // mode the user's own programs would give it.
    let vendor = MockVendor::start().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    let home = installations.home.path();
    let target =
        json!({ "target": { "kind": "device", "deviceId": device.id.as_str(), "path": home } });
    let moved = backend
        .patch(&format!("/api/conversations/{FIRST}"), &master, target)
        .await;
    assert_eq!(
        moved.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&moved.body)
    );
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    let script = "umask; echo made > made.txt";
    vendor.respond(tool_use(
        "mask",
        "shell_exec",
        &json!({ "script": script, "timeoutMs": 60_000 }),
    ));
    vendor.respond(crate::conversations::answer(&["done"], 1, 1));
    socket.chat("m1", "show the mask").await;
    let requests = vendor.requests();
    let result = tool_result(&requests[1].json(), "mask");
    assert!(result.contains("0002"), "{result}");
    assert_eq!(mode(&home.join("made.txt")), 0o664);
    drop(installations);
    backend.close().await;
}

#[tokio::test]
async fn without_runner_releases_the_installers_say_so_and_no_artifact_is_served() {
    let backend = Harness::new().start().await;
    let script = answer(
        reqwest::get(format!("{}/install.sh", backend.url))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(script.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        script.body,
        b"Runner releases are not configured on this backend.\n"
    );
    let artifact = reqwest::get(format!(
        "{}/runner-artifacts/{}/{}/demi-runner",
        backend.url,
        sha(b"initial"),
        host_target()
    ))
    .await
    .unwrap();
    assert_eq!(artifact.status(), StatusCode::NOT_FOUND);

    // With them, only a release's own executable is served, by its name. No
    // installer runs here, so a stand-in carries each release.
    let stand_in = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(stand_in.path(), "a stand-in for the runner").unwrap();
    let releases = Releases::new(stand_in.path().to_owned());
    let release = releases.publish("initial");
    let served = Harness::new()
        .with_runner_releases(releases.path())
        .start()
        .await;
    let script = reqwest::get(format!("{}/install.sh", served.url))
        .await
        .unwrap();
    assert_eq!(
        script.headers()["content-type"],
        "text/x-shellscript; charset=utf-8"
    );
    assert_eq!(script.headers()["cache-control"], "no-store");
    let text = script.text().await.unwrap();
    assert!(text.contains(&format!("backend={}/", served.url)), "{text}");
    for wrong in [
        format!("{release}/{}/demi-runner.exe", host_target()),
        format!("{release}/aarch64-apple-ios/demi-runner"),
        format!("{}/{}/demi-runner", sha(b"unpublished"), host_target()),
        format!("{}/{}/demi-runner", &release[..16], host_target()),
    ] {
        let refused = reqwest::get(format!("{}/runner-artifacts/{wrong}", served.url))
            .await
            .unwrap();
        assert_eq!(refused.status(), StatusCode::NOT_FOUND, "{wrong}");
    }
    let executable = reqwest::get(format!(
        "{}/runner-artifacts/{release}/{}/demi-runner",
        served.url,
        host_target()
    ))
    .await
    .unwrap();
    assert_eq!(executable.status(), StatusCode::OK);
    assert_eq!(
        json!(executable.bytes().await.unwrap().len()),
        releases.artifact["size"]
    );
    backend.close().await;
    served.close().await;
}
