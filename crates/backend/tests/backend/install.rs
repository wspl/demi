//! The public installation routes (`web-api.md` § Resource index): the
//! installer of the backend's current runner release, and each release's
//! executables. An installer run here installs this build's runner for its
//! backend and starts it, and the runner then waits to be paired; the test
//! drains every runner it installed.

use std::path::{Path, PathBuf};
use std::process::Output;

use demi_command_service::protocol::{TARGETS, VERSION, host_target};
use demi_host_remote::testing::runner_binary;
use demi_runner_protocol::wire;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::support::{Harness, TestBackend, answer};

fn sha(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// A runner release directory whose releases all carry this build's runner.
struct Releases {
    directory: tempfile::TempDir,
    runner: Vec<u8>,
    /// The runner's size and digest, which every release names.
    artifact: Value,
}

impl Releases {
    fn new() -> Self {
        let runner = std::fs::read(runner_binary()).unwrap();
        let artifact = json!({ "sha256": sha(&runner), "size": runner.len() });
        Self {
            directory: tempfile::Builder::new().prefix("demi-releases-").tempdir().unwrap(),
            runner,
            artifact,
        }
    }

    fn path(&self) -> PathBuf {
        self.directory.path().to_owned()
    }

    /// Publishes a release named by `name`'s digest, which the top-level
    /// manifest names from now on.
    fn publish(&self, name: &str) -> String {
        let release = sha(name.as_bytes());
        let executable = self.directory.path().join(&release).join(host_target()).join("demi-runner");
        std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
        std::fs::write(&executable, &self.runner).unwrap();
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
        std::fs::write(self.directory.path().join(&release).join("manifest.json"), &manifest).unwrap();
        std::fs::write(self.directory.path().join("manifest.json"), &manifest).unwrap();
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
            home: tempfile::Builder::new().prefix("demi-install-home-").tempdir().unwrap(),
            states: std::cell::RefCell::default(),
        }
    }

    /// The installation state of the backend at `url`.
    fn state(&self, url: &str) -> PathBuf {
        let state = self.home.path().join(".demi/instances").join(sha(url.as_bytes()));
        self.states.borrow_mut().push(state.clone());
        state
    }

    /// Fetches the backend's installer and runs it.
    async fn install(&self, backend: &TestBackend, extra: &[(&str, &str)]) -> Output {
        let script = reqwest::get(format!("{}/install.sh", backend.url)).await.unwrap();
        assert_eq!(script.status(), StatusCode::OK);
        let path = self.home.path().join(format!("install-{}.sh", backend.address().port()));
        std::fs::write(&path, script.bytes().await.unwrap()).unwrap();
        self.run(&path, extra).await
    }

    async fn run(&self, script: &Path, extra: &[(&str, &str)]) -> Output {
        tokio::process::Command::new("sh")
            .arg(script)
            .env("HOME", self.home.path())
            .env_remove("DEMI_INSTALLATION_ID")
            .envs(extra.iter().copied())
            .output()
            .await
            .unwrap()
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

#[tokio::test]
async fn an_installer_keeps_each_backend_apart_reuses_a_release_and_upgrades_only_its_own_runner() {
    let releases = Releases::new();
    let initial = releases.publish("initial");
    let a = Harness::new().with_runner_releases(releases.path()).start().await;
    let b = Harness::new().with_runner_releases(releases.path()).start().await;
    let installations = Installations::new();
    let state_a = installations.state(&format!("{}/", a.url));
    let state_b = installations.state(&format!("{}/", b.url));

    // The Windows installer names the release's Windows runners.
    let windows = reqwest::get(format!("{}/install.ps1", a.url)).await.unwrap();
    assert_eq!(windows.status(), StatusCode::OK);
    assert_eq!(windows.headers()["content-type"], "text/plain; charset=utf-8");
    assert!(windows.text().await.unwrap().contains("aarch64-pc-windows-msvc"));

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
    let script_b = installations.home.path().join(format!("install-{}.sh", b.address().port()));
    let registration_a = sha(format!("{}/", a.url).as_bytes());
    let collision = installations
        .run(&script_b, &[("DEMI_INSTALLATION_ID", registration_a.as_str())])
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
            format!("{}/runner-artifacts/{initial}/{}/demi-runner", a.url, host_target()),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(old.status(), StatusCode::OK);
    assert_eq!(old.headers()["cache-control"], "public, max-age=31536000, immutable");
    assert_eq!(active(&state_a)["release"], json!(upgraded));
    assert_ne!(active(&state_a)["endpoint"], first_a["endpoint"]);
    assert_eq!(active(&state_b)["endpoint"], first_b["endpoint"]);
    drop(installations);
    a.close().await;
    b.close().await;
}

#[tokio::test]
async fn without_runner_releases_the_installers_say_so_and_no_artifact_is_served() {
    let backend = Harness::new().start().await;
    let script = answer(reqwest::get(format!("{}/install.sh", backend.url)).await.unwrap()).await;
    assert_eq!(script.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(script.body, b"Runner releases are not configured on this backend.\n");
    let artifact = reqwest::get(format!(
        "{}/runner-artifacts/{}/{}/demi-runner",
        backend.url,
        sha(b"initial"),
        host_target()
    ))
    .await
    .unwrap();
    assert_eq!(artifact.status(), StatusCode::NOT_FOUND);

    // With them, only a release's own executable is served, by its name.
    let releases = Releases::new();
    let release = releases.publish("initial");
    let served = Harness::new().with_runner_releases(releases.path()).start().await;
    let script = reqwest::get(format!("{}/install.sh", served.url)).await.unwrap();
    assert_eq!(script.headers()["content-type"], "text/x-shellscript; charset=utf-8");
    assert_eq!(script.headers()["cache-control"], "no-store");
    let text = script.text().await.unwrap();
    assert!(text.contains(&format!("backend={}/", served.url)), "{text}");
    for wrong in [
        format!("{release}/{}/demi-runner.exe", host_target()),
        format!("{release}/aarch64-apple-ios/demi-runner"),
        format!("{}/{}/demi-runner", sha(b"unpublished"), host_target()),
        format!("{}/{}/demi-runner", &release[..16], host_target()),
    ] {
        let refused = reqwest::get(format!("{}/runner-artifacts/{wrong}", served.url)).await.unwrap();
        assert_eq!(refused.status(), StatusCode::NOT_FOUND, "{wrong}");
    }
    let executable = reqwest::get(format!("{}/runner-artifacts/{release}/{}/demi-runner", served.url, host_target()))
        .await
        .unwrap();
    assert_eq!(executable.status(), StatusCode::OK);
    assert_eq!(executable.bytes().await.unwrap().len(), releases.runner.len());
    backend.close().await;
    served.close().await;
}
