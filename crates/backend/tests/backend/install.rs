//! The public installation routes (`web-api.md` § Resource index): the
//! installer of the backend's current runner release, and each release's
//! executables. An installer run here installs this build's runner for its
//! backend and starts it, and the runner then waits to be paired; the test
//! drains every runner it installed.

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Output;

use demi_backend_remote_host::testing::runner_binary;
use demi_command_protocol::{TARGETS, VERSION, host_target};
use demi_provider_common::testing::MockVendor;
use demi_runner_protocol::release::{
    RELEASE_HEADER, RUNNER, TARGET_HEADER, TOKEN_HEADER, compressed_file,
};
use demi_runner_protocol::wire;
use demi_web_api_protocol::devices::DeviceState;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::conversations::{FIRST, Socket, anthropic, choose, create, tool_result, tool_use};
use crate::support::{Harness, Session, TestBackend, answer, eventually};

fn sha(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// A server release whose runner releases all carry `program`: its root,
/// with the releases' manifests in `runners/` and no command package, and
/// its files beside, where each target's runner is `program`'s compressed
/// copy, as a release's files hold it.
struct Releases {
    directory: tempfile::TempDir,
    /// `program` compressed, which each target's file links.
    compressed: PathBuf,
    /// The program's size and digest, which every release names.
    artifact: Value,
}

impl Releases {
    fn new(program: PathBuf) -> Self {
        let bytes = std::fs::read(&program).unwrap();
        let artifact = json!({ "sha256": sha(&bytes), "size": bytes.len() });
        let directory = tempfile::Builder::new()
            .prefix("demi-releases-")
            .tempdir()
            .unwrap();
        let compressed = directory.path().join("program.zst");
        let encoded =
            demi_shared_artifacts::encode_blocking(&bytes, demi_shared_artifacts::Effort::Fast)
                .unwrap();
        std::fs::write(&compressed, encoded).unwrap();
        Self {
            directory,
            compressed,
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
        self.publish_naming(name, &self.artifact.clone())
    }

    /// Publishes a release whose every target names `artifact`, which need
    /// not be the program's.
    fn publish_naming(&self, name: &str, artifact: &Value) -> String {
        let release = sha(name.as_bytes());
        let files = self.directory.path().join("files");
        std::fs::create_dir_all(&files).unwrap();
        std::fs::create_dir_all(self.path().join("commands")).unwrap();
        crate::support::write_server_release(&self.path(), &files);
        for target in TARGETS {
            let file = files.join(compressed_file(RUNNER, target));
            if !file.exists() {
                std::os::unix::fs::symlink(&self.compressed, &file).unwrap();
            }
        }
        std::fs::create_dir_all(self.runners().join(&release)).unwrap();
        // Test-only: every target names this machine's runner.
        let targets: serde_json::Map<String, Value> = TARGETS
            .iter()
            .map(|target| ((*target).to_owned(), artifact.clone()))
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

    /// The installation state the installer makes under the installation
    /// ID `id`.
    fn state_named(&self, id: &str) -> PathBuf {
        let state = self.home.path().join(".demi/instances").join(id);
        self.states.borrow_mut().push(state.clone());
        state
    }

    /// Fetches the backend's installer and runs it until it exits, pairing
    /// the runner with `session` when it shows a code; what it printed.
    async fn install(&self, backend: &TestBackend, session: &Session) -> String {
        self.install_with(backend, session, &[]).await
    }

    /// [`Installations::install`], with the installer's environment
    /// changed by `extra`.
    async fn install_with(
        &self,
        backend: &TestBackend,
        session: &Session,
        extra: &[(&str, &str)],
    ) -> String {
        let script = self.script(backend).await;
        let mut installer = self.shell();
        installer.arg(&script).envs(extra.iter().copied());
        paired(installer, backend, session, 0).await
    }

    /// Fetches the backend's installer and runs it from a shell whose file
    /// mode creation mask is `umask`, pairing the runner with `session` once
    /// `expired` codes went by; what it printed.
    async fn install_with_umask(
        &self,
        backend: &TestBackend,
        session: &Session,
        umask: &str,
        expired: usize,
    ) -> String {
        let script = self.script(backend).await;
        let mut installer = self.shell();
        installer
            .arg("-c")
            .arg(format!("umask {umask} && exec sh \"$0\""))
            .arg(&script);
        paired(installer, backend, session, expired).await
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

    /// A shell for an installer, with the home and without an installation
    /// ID; the runners it starts call their device [`DEVICE`].
    fn shell(&self) -> tokio::process::Command {
        let mut shell = tokio::process::Command::new("sh");
        shell
            .env("HOME", self.home.path())
            .env("DEMI_RUNNER_NAME", DEVICE)
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

/// The name of the device each installed runner pairs as.
const DEVICE: &str = "installed-laptop";

/// What an installer prints before the first pairing code and before each
/// one after it.
const CODES: [&str; 2] = [
    "Enter this pairing code in Add Device: ",
    "The code expired; enter this one instead: ",
];

/// Runs `installer`, which stays until its runner is paired: it shows each
/// pairing code as its runner receives one, and the code after `expired`
/// codes went by is claimed with `session`. What it printed, once it exited
/// successfully.
async fn paired(
    mut installer: tokio::process::Command,
    backend: &TestBackend,
    session: &Session,
    expired: usize,
) -> String {
    use tokio::io::AsyncBufReadExt as _;
    let mut child = installer
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut lines = tokio::io::BufReader::new(child.stdout.take().unwrap()).lines();
    let mut printed = String::new();
    let mut codes = 0;
    // A hang guard: an installer that never sees its runner paired would
    // wait for ever.
    let read = tokio::time::timeout(std::time::Duration::from_secs(60), async {
        while let Some(line) = lines.next_line().await.unwrap() {
            printed.push_str(&line);
            printed.push('\n');
            let Some(code) = CODES.iter().find_map(|prefix| line.strip_prefix(prefix)) else {
                continue;
            };
            codes += 1;
            if codes == expired + 1 {
                let claimed = backend
                    .post("/api/devices/claim", Some(session), json!({ "code": code }))
                    .await;
                assert_eq!(
                    claimed.status,
                    StatusCode::CREATED,
                    "{}",
                    String::from_utf8_lossy(&claimed.body)
                );
            }
        }
    })
    .await;
    if read.is_err() {
        // The runner it started stays, and the installations' drop drains it.
        child.start_kill().unwrap();
    }
    let output = child.wait_with_output().await.unwrap();
    assert!(
        read.is_ok() && output.status.success(),
        "{printed}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    printed
}

/// The installation's active runner: its endpoint and its release.
fn active(state: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(state.join("active.json")).unwrap()).unwrap()
}

/// Asserts that an installer said its runner is paired as [`DEVICE`],
/// with the launcher of the installation in `state` as the removal command.
fn says_paired(printed: &str, state: &Path) {
    let removal = format!(
        "To remove this runner, run: {} uninstall",
        state.join("run").display()
    );
    assert!(
        printed.contains(&format!("Paired as {DEVICE}\n{removal}\n")),
        "{printed}"
    );
}

// Several seconds: the installer runs for real, and each of its three installs
// downloads this build's runner (170 MB) from its backend, verifies it and
// starts it; the upgrade drains one.
#[tokio::test]
async fn an_installer_keeps_each_backend_apart_reuses_a_release_and_upgrades_only_its_own_runner() {
    let releases = Releases::new(runner_binary());
    let initial = releases.publish("initial");
    let (a, master_a) = Harness::new()
        .with_runner_releases(releases.path())
        .start_set_up()
        .await;
    let (b, master_b) = Harness::new()
        .with_runner_releases(releases.path())
        .start_set_up()
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

    // Each installer shows its runner's code and stays until the runner is
    // paired with it; then it names the device and the removal command.
    says_paired(&installations.install(&a, &master_a).await, &state_a);
    says_paired(&installations.install(&b, &master_b).await, &state_b);
    let first_a = active(&state_a);
    let first_b = active(&state_b);
    assert_ne!(first_a["endpoint"], first_b["endpoint"]);
    assert_eq!(first_a["release"], json!(initial));

    // Installed again, the running runner stays, paired already.
    let again = installations.install(&a, &master_a).await;
    assert!(again.contains("already running"), "{again}");
    says_paired(&again, &state_a);
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
    says_paired(&installations.install(&a, &master_a).await, &state_a);
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

/// The release the installation's active runner runs, once one is active.
fn active_release(state: &Path) -> Option<Value> {
    let bytes = std::fs::read(state.join("active.json")).ok()?;
    let active: Value = serde_json::from_slice(&bytes).ok()?;
    Some(active["release"].clone())
}

// Several seconds: the installer downloads this build's runner (170 MB) and
// starts it, and the runner downloads it again for each backend release it
// tries.
#[tokio::test]
async fn an_installed_runner_follows_its_backend_to_another_release() {
    let releases = Releases::new(runner_binary());
    let initial = releases.publish("initial");
    let harness = Harness::new().with_runner_releases(releases.path());
    let (backend, master) = harness.start_set_up().await;
    let address = backend.address();
    let installations = Installations::new();
    let state = installations.state(&format!("{}/", backend.url));
    installations.install(&backend, &master).await;
    assert_eq!(active(&state)["release"], json!(initial));

    // The backend comes back with a release whose runner it cannot supply:
    // the runner stays on its own and says why.
    backend.close().await;
    let size = std::fs::metadata(runner_binary()).unwrap().len();
    let broken = releases.publish_naming(
        "broken",
        &json!({ "sha256": sha(b"another program"), "size": size }),
    );
    let backend = harness.start_at(address).await;
    let log = state.join("runner.log");
    eventually("the runner reports the failed update", || async {
        std::fs::read_to_string(&log)
            .unwrap()
            .contains(&format!("the update to runner release {broken} failed"))
    })
    .await;
    assert_eq!(active(&state)["release"], json!(initial));

    // Once the backend's release has a runner, the runner's next attempt
    // replaces it with that one, which connects in its place.
    let upgraded = releases.publish("upgraded");
    eventually("the runner of the backend's release is active", || async {
        active_release(&state) == Some(json!(upgraded))
    })
    .await;
    assert_eq!(
        std::fs::read_to_string(state.join("release-id")).unwrap(),
        format!("{upgraded}\n")
    );
    let mut kept: Vec<String> = std::fs::read_dir(state.join("releases"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    kept.sort();
    let mut expected = vec![initial, upgraded];
    expected.sort();
    assert_eq!(kept, expected);
    drop(installations);
    backend.close().await;
}

/// A file's permission bits.
fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

// Several seconds (8 s here under load): the installer downloads this build's
// runner (170 MB) from its backend, verifies it and starts it, and shows a
// code that expires after two seconds before it shows the one claimed; a
// scripted model then runs one job on the paired runner.
#[tokio::test]
async fn an_installer_shows_each_code_until_paired_and_its_runner_works_with_the_shells_mask() {
    let releases = Releases::new(runner_binary());
    releases.publish("initial");
    let mut harness = Harness::new().with_runner_releases(releases.path());
    // Long enough for the installer to show a code and the test to claim it.
    harness.runners.claim_lifetime = std::time::Duration::from_secs(2);
    let (backend, master) = harness.start_set_up().await;
    let installations = Installations::new();
    let state = installations.state(&format!("{}/", backend.url));

    // The user's shell lets the group write, as some systems' shells do. The
    // installer shows the runner's code, and the next one once it expired,
    // which pairs the runner.
    let printed = installations
        .install_with_umask(&backend, &master, "002", 1)
        .await;
    assert!(printed.contains(CODES[1]), "{printed}");
    says_paired(&printed, &state);
    // The installation stays the user's alone: its log holds the pairing code.
    assert_eq!(mode(&state), 0o700);
    assert_eq!(mode(&state.join("runner.log")), 0o600);
    let devices = backend.devices(&master).await;
    let [device] = devices.as_slice() else {
        panic!("one paired device: {devices:?}");
    };
    assert_eq!(device.name, DEVICE);
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
        &json!({ "script": script, "description": "Show the mask", "timeoutMs": 60_000 }),
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

/// The command a paired device shows starts its stopped runner again in
/// the background: `run start` of the installation its runner reported,
/// here one the person named apart from the default (`runner.md`
/// § Installation, pairing and removal).
// Several seconds: the installer downloads this build's runner (170 MB)
// and starts it, and the runner starts a second time.
#[tokio::test]
async fn a_paired_devices_start_command_starts_its_stopped_runner_again() {
    let releases = Releases::new(runner_binary());
    releases.publish("initial");
    let harness = Harness::new().with_runner_releases(releases.path());
    let (backend, master) = harness.start_set_up().await;
    let installations = Installations::new();
    let state = installations.state_named("elsewhere");
    installations
        .install_with(&backend, &master, &[("DEMI_INSTALLATION_ID", "elsewhere")])
        .await;
    let devices = backend.devices(&master).await;
    let [device] = devices.as_slice() else {
        panic!("one paired device: {devices:?}");
    };
    let id = device.id.clone();
    backend.until_online(&master, id.as_str(), true).await;
    let drained = std::process::Command::new(state.join("run"))
        .arg("drain")
        .output()
        .unwrap();
    assert!(drained.status.success(), "{drained:?}");
    backend.until_online(&master, id.as_str(), false).await;

    // The command names the installation's launcher, and returns once the
    // runner it started in the background is connected; the installations'
    // drop drains it.
    let devices = backend.devices(&master).await;
    let command = devices[0].start_command.clone().expect("a paired device has a start command");
    assert_eq!(command, "~/.demi/instances/elsewhere/run start");
    let started = installations.shell().arg("-c").arg(&command).output().await.unwrap();
    assert!(started.status.success(), "{started:?}");
    assert!(backend.online(&master, id.as_str()).await);
    // Typed again, it leaves the running runner as it is.
    let endpoint = active(&state)["endpoint"].clone();
    let again = installations.shell().arg("-c").arg(&command).output().await.unwrap();
    assert!(again.status.success(), "{again:?}");
    assert!(String::from_utf8_lossy(&again.stdout).contains("running already"), "{again:?}");
    assert_eq!(active(&state)["endpoint"], endpoint);
    drop(installations);
    backend.close().await;
}

/// A backend sources the runner executables of its paired devices' systems
/// as it starts, so that a runner's update after the server's upgrade waits
/// on no origin (`native-runtime.md` § Runner releases).
// Several seconds: the installer downloads this build's runner (170 MB)
// and starts it.
#[tokio::test]
async fn a_backend_sources_the_runners_of_its_paired_devices_as_it_starts() {
    let releases = Releases::new(runner_binary());
    releases.publish("initial");
    let harness = Harness::new().with_runner_releases(releases.path());
    let (backend, master) = harness.start_set_up().await;
    let installations = Installations::new();
    let state = installations.state(&format!("{}/", backend.url));
    installations.install(&backend, &master).await;
    let devices = backend.devices(&master).await;
    let [device] = devices.as_slice() else {
        panic!("one paired device: {devices:?}");
    };
    // Nobody asks for a runner while the backend is away.
    let drained = std::process::Command::new(state.join("run"))
        .arg("drain")
        .output()
        .unwrap();
    assert!(drained.status.success(), "{drained:?}");
    backend.until_online(&master, device.id.as_str(), false).await;
    let address = backend.address();
    backend.close().await;

    // The server moves to a release with another runner, which its store
    // does not hold.
    let next = b"the next runner";
    let encoded =
        demi_shared_artifacts::encode_blocking(next, demi_shared_artifacts::Effort::Fast).unwrap();
    let files = releases.directory.path().join("files");
    for target in TARGETS {
        let file = files.join(compressed_file(RUNNER, target));
        std::fs::remove_file(&file).unwrap();
        std::fs::write(&file, &encoded).unwrap();
    }
    releases.publish_naming("next", &json!({ "sha256": sha(next), "size": next.len() }));
    let backend = harness.start_at(address).await;
    let stored = harness.data_dir().join("native/blobs").join(sha(next));
    eventually("the next runner is in the store", || {
        let held = stored.exists();
        async move { held }
    })
    .await;
    drop(installations);
    backend.close().await;
}

/// Asks for the runner socket as a runner of `release` asks for it, naming
/// its device with `token` when it has one, and answers the status; an
/// opened socket says hello with the token and answers whether it was
/// welcomed, the socket staying open while the answer is held.
async fn ask_socket(
    backend: &TestBackend,
    release: &str,
    token: Option<&str>,
) -> (
    StatusCode,
    Option<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>>,
) {
    use futures_util::{SinkExt as _, StreamExt as _};
    use tokio_tungstenite::tungstenite::{Error, Message, client::IntoClientRequest as _};
    let mut request = backend.ws_url("/api/runner").into_client_request().unwrap();
    let headers = request.headers_mut();
    headers.insert(RELEASE_HEADER, release.parse().unwrap());
    headers.insert(TARGET_HEADER, host_target().parse().unwrap());
    if let Some(token) = token {
        headers.insert(TOKEN_HEADER, token.parse().unwrap());
    }
    let mut socket = match tokio_tungstenite::connect_async(request).await {
        Ok((socket, _)) => socket,
        Err(Error::Http(response)) => {
            return (StatusCode::from_u16(response.status().as_u16()).unwrap(), None);
        }
        Err(error) => panic!("the runner socket failed: {error}"),
    };
    let hello = wire::Outbound::Hello {
        protocol: wire::VERSION,
        device_token: token.map(|token| token.to_owned().try_into().unwrap()),
        runner: wire::RunnerInfo {
            name: "raw".into(),
            platform: wire::RunnerPlatform::Linux,
            os: wire::OperatingSystem {
                name: "Ubuntu 26.04".into(),
                arch: "x86_64".into(),
            },
            version: release.into(),
            native_target: None,
            identity: wire::HostIdentity {
                uid: 1,
                gid: 1,
                hostname: "raw".into(),
                home_dir: "/home/raw".into(),
            },
            managed: None,
            installation: None,
        },
    };
    let frame = wire::encode(&hello).unwrap().into_bytes();
    socket.send(Message::Binary(frame.into())).await.unwrap();
    loop {
        match socket.next().await {
            Some(Ok(Message::Binary(frame))) => {
                let welcomed = matches!(
                    wire::decode::<wire::Inbound>(&frame).unwrap(),
                    wire::Inbound::HelloOk { .. }
                );
                assert!(welcomed);
                return (StatusCode::SWITCHING_PROTOCOLS, Some(socket));
            }
            Some(Ok(_)) => {}
            other => panic!("the socket closed before its hello was answered: {other:?}"),
        }
    }
}

/// A runner the backend sends to an update names its device with its token,
/// and the device shows as updating from that first 409 until its next hello
/// or the window ends, which a retry does not extend; a runner of an earlier
/// release names none, and its device shows as offline (`runner.md`
/// § Runner updates).
// Several seconds: the installer downloads this build's runner (170 MB)
// and starts it, and the window passes once.
#[tokio::test]
async fn a_runner_sent_to_an_update_shows_its_device_updating_until_its_next_hello_or_the_window_ends() {
    let releases = Releases::new(runner_binary());
    let initial = releases.publish("initial");
    let mut harness = Harness::new().with_runner_releases(releases.path());
    let window = std::time::Duration::from_millis(1500);
    harness.runners.updating = window;
    let (backend, master) = harness.start_set_up().await;
    let installations = Installations::new();
    let state = installations.state(&format!("{}/", backend.url));
    installations.install(&backend, &master).await;
    let devices = backend.devices(&master).await;
    let [device] = devices.as_slice() else {
        panic!("one paired device: {devices:?}");
    };
    let id = device.id.clone();
    backend.until_online(&master, id.as_str(), true).await;
    let token = std::fs::read_to_string(state.join("runner-token")).unwrap();
    let token = token.trim();
    let drained = std::process::Command::new(state.join("run"))
        .arg("drain")
        .output()
        .unwrap();
    assert!(drained.status.success(), "{drained:?}");
    backend.until_online(&master, id.as_str(), false).await;
    let upgraded = releases.publish("upgraded");
    let device_state = || async {
        backend
            .devices(&master)
            .await
            .into_iter()
            .find(|device| device.id == id)
            .expect("the device is listed")
            .state
    };

    // A runner of a release before 0.1.14 names no device.
    assert_eq!(ask_socket(&backend, &initial, None).await.0, StatusCode::CONFLICT);
    assert_eq!(device_state().await, DeviceState::Offline);

    // Its token names the device, which shows as updating until the window
    // ends.
    let asked = tokio::time::Instant::now();
    assert_eq!(ask_socket(&backend, &initial, Some(token)).await.0, StatusCode::CONFLICT);
    assert_eq!(device_state().await, DeviceState::Updating);
    eventually("the window ends", || async {
        device_state().await == DeviceState::Offline
    })
    .await;
    assert!(asked.elapsed() >= window, "{:?}", asked.elapsed());

    // A retry of the update does not show it again.
    assert_eq!(ask_socket(&backend, &initial, Some(token)).await.0, StatusCode::CONFLICT);
    assert_eq!(device_state().await, DeviceState::Offline);

    // The runner of the new release says hello, which ends the update.
    let (opened, socket) = ask_socket(&backend, &upgraded, Some(token)).await;
    assert_eq!(opened, StatusCode::SWITCHING_PROTOCOLS);
    assert_eq!(device_state().await, DeviceState::Online);
    drop(socket);
    eventually("the device is offline", || async {
        device_state().await == DeviceState::Offline
    })
    .await;
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
        format!("{release}/{}/demi-runner.zst.zst", host_target()),
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
    assert!(executable.headers().get("content-encoding").is_none());
    assert_eq!(
        executable.bytes().await.unwrap(),
        std::fs::read(stand_in.path()).unwrap()
    );
    // A runner's update downloads the compressed copy, which its client
    // decodes from the content coding and checks.
    let expected = demi_shared_artifacts::Digest {
        size: releases.artifact["size"].as_u64().unwrap(),
        sha256: releases.artifact["sha256"].as_str().unwrap().to_owned(),
    };
    let client = demi_shared_artifacts::client_allowing_http().unwrap();
    let mut decoded = Vec::new();
    demi_shared_artifacts::download(
        &client,
        &format!(
            "{}/runner-artifacts/{release}/{}/demi-runner.zst",
            served.url,
            host_target()
        ),
        &expected,
        &mut decoded,
        &tokio_util::sync::CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(decoded, std::fs::read(stand_in.path()).unwrap());
    backend.close().await;
    served.close().await;
}
