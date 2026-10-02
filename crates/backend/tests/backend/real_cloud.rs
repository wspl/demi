//! The Cloud suite (`scenarios.md` § Cloud suite; `managed-hosts.md`
//! § Verification): the backend against a real machine manager, its
//! gVisor/systrap sandboxes and the shipped Cloud image, with a scripted
//! model. An ordinary run ignores every test here; the suite's command runs
//! them as root on the manager's host, with the variables that say where the
//! manager, its state and the image's command releases are.
//!
//! The tests reach the Cloud as a conversation does: the model's shell jobs,
//! the conversation's file and browser routes, and the Cloud's status and
//! reset. They ask the manager itself only for a checkpoint and a device's
//! committed generation, and read its state directory only to look inside a
//! saved generation and to find a boot's processes on the host. What they
//! measure they print, apart from their assertions.
//!
//! Each test takes tens of seconds: a Cloud boots in seconds, and a stop, a
//! wake and a reset each save or boot it once more.

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use demi_backend_cloud::client::MachinesClient;
use demi_command_package_browser_protocol::browser::TabId;
use demi_machine_manager_protocol::{CheckpointParams, ImageStateParams, MachineImageState};
use demi_plugin_browser::page::{BrowserTabs, OpenedTab};
use demi_provider_common::testing::MockVendor;
use demi_shared_gates::Purpose;
use demi_web_api_protocol::auth::Role;
use demi_web_api_protocol::cloud::{CloudState, CloudStatus, ResetPhase};
use demi_web_api_protocol::error::ErrorCode;
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde_json::json;

use crate::cloud::{idle_after, reset, status};
use crate::conversations::{anthropic_at, create};
use crate::support::{Harness, Session, TestBackend, answer};
use crate::work::{Driven, say, shell};

/// The conversation ids the tests create.
const FIRST: &str = "8e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a01";
const SECOND: &str = "8e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a02";

/// How long a test waits for a Cloud to boot, stop, wake or reset before it
/// fails as hung: a hang guard on a loaded machine, not a window any of them
/// must fit.
const PATIENCE: Duration = Duration::from_secs(240);

/// How long the model watches a shell job before its turn goes on; a job
/// that outlasts it keeps running.
const WATCH_MS: u64 = 240_000;

/// What the Cloud suite's environment supplies (`scenarios.md` § Cloud suite).
struct Environment {
    /// The manager's socket.
    socket: PathBuf,
    /// The backend URL the manager allows.
    url: url::Url,
    /// The manager's state directory, which the suite only reads.
    data: PathBuf,
    /// The native configuration of the command releases the image embeds.
    native: PathBuf,
}

impl Environment {
    fn read() -> Self {
        let variable = |name: &str| {
            std::env::var(name).unwrap_or_else(|_| {
                panic!("the Cloud suite needs {name} (scenarios.md § Cloud suite)")
            })
        };
        Self {
            socket: variable("DEMI_TEST_MACHINES_SOCKET").into(),
            url: variable("DEMI_TEST_CLOUD_URL")
                .parse()
                .expect("DEMI_TEST_CLOUD_URL is a URL"),
            data: variable("DEMI_TEST_MACHINES_DATA").into(),
            native: variable("DEMI_TEST_CLOUD_NATIVE").into(),
        }
    }

    /// The address the backend listens on: the URL's own, since the
    /// manager lets the Clouds reach only that one.
    fn address(&self) -> SocketAddr {
        let host: IpAddr = self
            .url
            .host_str()
            .and_then(|host| host.parse().ok())
            .expect("DEMI_TEST_CLOUD_URL names an IP address");
        SocketAddr::new(
            host,
            self.url
                .port_or_known_default()
                .expect("an http URL has a port"),
        )
    }

    /// A harness whose backends use the real manager, load the image's
    /// command releases, and give the Clouds' runners the URL the manager
    /// allows. A Cloud stops a moment after the test lets go of its
    /// conversation's file gate, whose lease is the conversation's work
    /// (`scenarios.md` § System under test).
    fn harness(&self) -> Harness {
        let mut harness = Harness::new();
        harness.machines = Some(self.socket.clone());
        harness.native = Some(self.native.clone());
        harness.public_url = Some(self.url.clone());
        harness.lifecycle = idle_after(Duration::from_secs(2));
        harness.cloud.sweep = Duration::from_millis(200);
        harness
    }

    /// The manager's client, for what only the manager answers: a
    /// checkpoint, and a device's committed generation.
    fn manager(&self) -> MachinesClient {
        let (client, _deaths) = MachinesClient::new(self.socket.clone());
        client
    }

    /// Where the manager keeps `generation` of `device`
    /// (`managed-hosts.md` § Images).
    fn generation(&self, device: &str, state: &MachineImageState) -> PathBuf {
        self.data
            .join("images")
            .join(device)
            .join("generations")
            .join(state.generation.to_string())
    }
}

/// The tests share one manager, and a backend that starts reconciles it,
/// which stops every Cloud: one test runs at a time, whatever the number of
/// test threads. A test holds the suite across its awaits, so the lock is
/// an asynchronous one; a test that fails while it holds the suite releases
/// it as it unwinds and leaves nothing the next one depends on.
async fn one_at_a_time() -> tokio::sync::MutexGuard<'static, ()> {
    static SUITE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    SUITE.lock().await
}

/// Prints a measurement, apart from the assertions.
fn measured(test: &str, what: &str, took: Duration) {
    eprintln!(
        "cloud-suite measurement: {test}: {what}: {} ms",
        took.as_millis()
    );
}

/// Prints the peak resident memory of the host processes of `device`'s
/// running boot, its Sentry and Gofer and the Sentry's helpers, which carry
/// the boot's id, the one the manager's runtime record names.
fn measure_memory(test: &str, environment: &Environment, device: &str) {
    #[derive(Deserialize)]
    struct Boot {
        id: String,
    }
    let record = environment
        .data
        .join("working")
        .join(device)
        .join("sandbox.json");
    let boot: Boot = serde_json::from_slice(&std::fs::read(&record).unwrap()).unwrap();
    // Each program: how many processes run it and the largest peak.
    let mut peaks: BTreeMap<String, (usize, u64)> = BTreeMap::new();
    for entry in std::fs::read_dir("/proc").unwrap() {
        let name = entry.unwrap().file_name();
        let Ok(pid) = name.to_string_lossy().parse::<u32>() else {
            continue;
        };
        // A process may end while it is read; it is then not measured.
        let (Ok(command), Ok(status)) = (
            std::fs::read(format!("/proc/{pid}/cmdline")),
            std::fs::read_to_string(format!("/proc/{pid}/status")),
        ) else {
            continue;
        };
        let arguments: Vec<String> = command
            .split(|byte| *byte == 0)
            .map(|argument| String::from_utf8_lossy(argument).into_owned())
            .collect();
        if !arguments.iter().any(|argument| argument.contains(&boot.id)) {
            continue;
        }
        let peak = status
            .lines()
            .find_map(|line| line.strip_prefix("VmHWM:"))
            .and_then(|value| value.trim().trim_end_matches(" kB").parse::<u64>().ok())
            .unwrap_or(0);
        let entry = peaks.entry(arguments[0].clone()).or_default();
        entry.0 += 1;
        entry.1 = entry.1.max(peak);
    }
    for (program, (count, peak)) in peaks {
        eprintln!(
            "cloud-suite measurement: {test}: {program}: {count} processes, largest peak {} MiB",
            peak / 1024
        );
    }
}

/// The Cloud's status once `check` holds, asking every 100 ms for at most
/// [`PATIENCE`].
async fn until_cloud(
    backend: &TestBackend,
    session: &Session,
    what: &str,
    check: impl Fn(&CloudStatus) -> bool,
) -> CloudStatus {
    let deadline = Instant::now() + PATIENCE;
    loop {
        let status = status(backend, session).await;
        if check(&status) {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "never came true: {what}: {status:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The id of `session`'s Cloud.
async fn the_device(backend: &TestBackend, session: &Session) -> String {
    let status = status(backend, session).await;
    status
        .device
        .expect("the Cloud is allocated")
        .id
        .as_str()
        .to_owned()
}

/// Lists the conversation's directory, which needs its Host: it boots or
/// wakes the Cloud, and answers once its runner is ready.
async fn list(
    backend: &TestBackend,
    session: &Session,
    conversation: &str,
) -> crate::support::Answer {
    backend
        .get(
            &format!("/api/conversations/{conversation}/fs"),
            Some(session),
        )
        .await
}

/// Opens a tab at `url` from the conversation's panel, whose browser
/// plugin's calls are under `calls`, and answers its id. The panel's open
/// answers before the page loads (`live-view.md` § The tab methods).
async fn open_tab(backend: &TestBackend, session: &Session, calls: &str, url: &str) -> TabId {
    let opened = backend
        .post(
            &format!("{calls}/open"),
            Some(session),
            json!({ "url": url }),
        )
        .await;
    assert!(
        opened.status.is_success(),
        "{}",
        String::from_utf8_lossy(&opened.body)
    );
    opened.json::<OpenedTab>().tab.id
}

/// The conversation browser's tabs, as the panel lists them through the
/// plugin's calls under `calls`.
async fn list_tabs(
    backend: &TestBackend,
    session: &Session,
    calls: &str,
) -> crate::support::Answer {
    backend
        .post(&format!("{calls}/tabs"), Some(session), json!({}))
        .await
}

/// Waits until the tab `id` shows the page titled `title`, which Chrome
/// reports once it has read the page, asking every 100 ms for at most
/// [`PATIENCE`].
async fn until_titled(
    backend: &TestBackend,
    session: &Session,
    calls: &str,
    id: &TabId,
    title: &str,
) {
    let deadline = Instant::now() + PATIENCE;
    loop {
        let listed = list_tabs(backend, session, calls).await;
        assert_eq!(
            listed.status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&listed.body)
        );
        let listed: BrowserTabs = listed.json();
        if listed
            .tabs
            .iter()
            .any(|tab| tab.id == *id && tab.title == title)
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "tab {id} never showed {title:?}: {:?}",
            listed.tabs
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// A conversation on the Cloud: its model answers from `vendor` under
/// `prefix`, and its socket waits as long as a boot within a turn may take.
async fn on_cloud<'a>(
    backend: &TestBackend,
    master: &Session,
    session: &Session,
    vendor: &'a MockVendor,
    conversation: &str,
    prefix: &str,
) -> Driven<'a> {
    let model = anthropic_at(backend, master, vendor, prefix).await;
    create(backend, session, conversation).await;
    let mut driven = Driven::open(backend, session, vendor, conversation, &model, prefix).await;
    driven.socket.patience = PATIENCE;
    driven
}

/// Runs `script` as the model's shell job `id`, watched for at most `watch`,
/// and answers what the model read.
async fn run_watched(driven: &mut Driven<'_>, id: &str, script: &str, watch: u64) -> String {
    let turn = driven
        .turn(vec![shell(id, script, watch), say("done")])
        .await;
    turn.received.into_iter().next().expect("the job's result")
}

/// Runs `script` as the model's shell job `id` to its end.
async fn run(driven: &mut Driven<'_>, id: &str, script: &str) -> String {
    run_watched(driven, id, script, WATCH_MS).await
}

/// The capacity of the ext4 filesystem in `image`, as its superblock
/// records it: its block count times its block size, read by `dumpe2fs`.
fn superblock_capacity(image: &Path) -> u64 {
    let output = Command::new("dumpe2fs")
        .arg("-h")
        .arg(image)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    let field = |name: &str| -> u64 {
        text.lines()
            .find_map(|line| line.strip_prefix(name))
            .unwrap_or_else(|| panic!("dumpe2fs names no {name}"))
            .trim()
            .parse()
            .unwrap()
    };
    field("Block count:") * field("Block size:")
}

/// The file `path` inside the ext4 image `image`, read by debugfs without
/// mounting the image.
fn read_in_image(image: &Path, path: &str) -> String {
    let output = Command::new("debugfs")
        .arg("-R")
        .arg(format!("cat {path}"))
        .arg(image)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The names in the directory `path` inside the ext4 image `image`, listed
/// by debugfs without mounting the image; none when there is no such
/// directory.
fn names_in_image(image: &Path, path: &str) -> Vec<String> {
    let output = Command::new("debugfs")
        .arg("-R")
        .arg(format!("ls -p {path}"))
        .arg(image)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Each entry is `/inode/mode/uid/gid/name/size/`.
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.split('/').nth(5))
        .filter(|name| !name.is_empty() && *name != "." && *name != "..")
        .map(str::to_owned)
        .collect()
}

/// Where the runner keeps its job directories on a Cloud, in the system
/// image's upper layer (`managed-hosts.md` § Images).
const JOBS_IN_SYSTEM_IMAGE: &str = "/upper/var/lib/demi/jobs";

/// A package this suite builds and installs in the Cloud with dpkg, which
/// needs no network: `cloud-suite-probe`, which installs
/// `/usr/share/cloud-suite/marker`.
const PACKAGE: &str = r#"mkdir -p package/DEBIAN package/usr/share/cloud-suite
printf 'Package: cloud-suite-probe\nVersion: 1.0\nArchitecture: all\nMaintainer: Cloud suite <suite@example.test>\nDescription: what the Cloud suite installs\n' > package/DEBIAN/control
echo installed > package/usr/share/cloud-suite/marker
dpkg-deb --root-owner-group --build package probe.deb > /dev/null
sudo -n dpkg -i probe.deb > /dev/null"#;

// Tens of seconds: the Cloud boots, stops when idle, wakes and is saved.
#[tokio::test]
#[ignore = "the Cloud suite: needs a machine manager and the suite's variables (scenarios.md § Cloud suite)"]
async fn a_cloud_runs_as_uid_1000_for_every_conversation_and_keeps_a_system_package_and_home_across_a_stop()
 {
    const TEST: &str = "identity and persistence";
    let _one = one_at_a_time().await;
    let environment = Environment::read();
    let vendor = MockVendor::start().await;
    let harness = environment.harness();
    let backend = harness.start_at(environment.address()).await;
    let master = backend.setup().await;
    let mut first = on_cloud(&backend, &master, &master, &vendor, FIRST, "/a").await;
    let mut second = on_cloud(&backend, &master, &master, &vendor, SECOND, "/b").await;
    let working = backend
        .file_gate(&master, FIRST)
        .await
        .enter(Purpose::Demand)
        .await;

    // Opening a conversation leaves its Cloud as it is; the first Host
    // operation boots it.
    let started = Instant::now();
    let listed = list(&backend, &master, FIRST).await;
    assert_eq!(
        listed.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&listed.body)
    );
    measured(
        TEST,
        "first boot until the runner is ready",
        started.elapsed(),
    );
    let device = the_device(&backend, &master).await;
    measure_memory(TEST, &environment, &device);

    let started = Instant::now();
    let facts = run(
        &mut first,
        "facts",
        &format!(
            "set -e\nid -u\nid -g\nsudo -n id -u\necho home-data > ~/note\n{PACKAGE}\n\
             dpkg-query -W -f='${{Status}}\\n' cloud-suite-probe\ngrep MemTotal /proc/meminfo"
        ),
    )
    .await;
    measured(TEST, "first command", started.elapsed());
    assert!(facts.contains("1000\n1000\n0\n"), "{facts}");
    assert!(facts.contains("install ok installed"), "{facts}");
    let memory = facts
        .lines()
        .find(|line| line.starts_with("MemTotal:"))
        .unwrap_or("MemTotal: unknown");
    eprintln!("cloud-suite measurement: {TEST}: the Cloud's {memory}");

    // A Host file operation of the first conversation writes as the user,
    // and the second conversation, on the same Cloud, reads it.
    let upload =
        format!("/api/conversations/{FIRST}/fs/raw?path=/home/demi/sessions/{FIRST}/uploaded");
    let uploaded = answer(
        backend
            .response(Method::PUT, &upload, &master, &[], Some("from-api".into()))
            .await,
    )
    .await;
    assert_eq!(
        uploaded.status,
        StatusCode::NO_CONTENT,
        "{}",
        String::from_utf8_lossy(&uploaded.body)
    );
    let seen = run(
        &mut second,
        "seen",
        &format!("stat -c %u:%g /home/demi/sessions/{FIRST}/uploaded; cat /home/demi/sessions/{FIRST}/uploaded; echo; cat ~/note"),
    )
    .await;
    assert!(seen.contains("1000:1000\nfrom-api\nhome-data"), "{seen}");
    assert_eq!(the_device(&backend, &master).await, device);

    let started = Instant::now();
    drop(working);
    until_cloud(&backend, &master, "the idle Cloud stops", |status| {
        status.state == CloudState::Off
    })
    .await;
    measured(TEST, "idle stop, window included", started.elapsed());
    // Both conversations were released before the stop saved the Cloud: the
    // generation it saved holds none of their job output.
    let stopped = environment
        .manager()
        .call(ImageStateParams {
            device_id: device.clone(),
        })
        .await
        .unwrap()
        .expect("the stopped Cloud has a generation");
    let system = environment
        .generation(&device, &stopped)
        .join("system.ext4");
    let jobs = names_in_image(&system, JOBS_IN_SYSTEM_IMAGE);
    assert!(jobs.contains(&"edits.lock".to_owned()), "{jobs:?}");
    assert!(
        !jobs.contains(&FIRST.to_owned()) && !jobs.contains(&SECOND.to_owned()),
        "{jobs:?}"
    );

    let working = backend
        .file_gate(&master, FIRST)
        .await
        .enter(Purpose::Demand)
        .await;
    let started = Instant::now();
    let listed = list(&backend, &master, FIRST).await;
    assert_eq!(
        listed.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&listed.body)
    );
    measured(TEST, "wake until the runner is ready", started.elapsed());
    let woken = run(
        &mut first,
        "woken",
        "cat ~/note; dpkg-query -W -f='${Status}\\n' cloud-suite-probe; cat /usr/share/cloud-suite/marker",
    )
    .await;
    assert!(
        woken.contains("home-data\ninstall ok installed\ninstalled"),
        "{woken}"
    );
    assert_eq!(the_device(&backend, &master).await, device);
    measure_memory(TEST, &environment, &device);
    drop(working);
    backend.close().await;

    // The saved generation records the home's capacity as its superblock
    // gives it.
    let saved = environment
        .manager()
        .call(ImageStateParams {
            device_id: device.clone(),
        })
        .await
        .unwrap()
        .expect("the saved Cloud has a generation");
    let home = environment.generation(&device, &saved).join("home.ext4");
    assert_eq!(saved.home_bytes.get(), superblock_capacity(&home));
}

// Tens of seconds: the Cloud boots, stops when idle, wakes, grows its home
// and is saved; the runner asks for growth as it connects. It needs a
// manager that holds CAP_SYS_RESOURCE (`scenarios.md` § Cloud suite).
#[tokio::test]
#[ignore = "the Cloud suite: needs a machine manager and the suite's variables (scenarios.md § Cloud suite)"]
async fn a_cloud_grows_its_home_online_and_its_saved_generation_records_the_grown_capacity() {
    const TEST: &str = "growth";
    let _one = one_at_a_time().await;
    let environment = Environment::read();
    let vendor = MockVendor::start().await;
    let harness = environment.harness();
    let backend = harness.start_at(environment.address()).await;
    let master = backend.setup().await;
    let mut first = on_cloud(&backend, &master, &master, &vendor, FIRST, "/a").await;
    let working = backend
        .file_gate(&master, FIRST)
        .await
        .enter(Purpose::Demand)
        .await;

    // Fills the home past what its runner keeps free, without writing the
    // blocks, so the host stores none of it. The runner looks at its
    // volumes when it connects, and then once a minute.
    let filled = run(
        &mut first,
        "fill",
        "fallocate -l 800M ~/fill && df -B1 --output=size /home | tail -1",
    )
    .await;
    let initial: u64 = filled
        .lines()
        .find_map(|line| line.trim().parse().ok())
        .unwrap_or_else(|| panic!("no size: {filled}"));
    let device = the_device(&backend, &master).await;

    // Woken by a Host operation, not a job: the runner looks at its volumes
    // as it connects only while no job runs.
    drop(working);
    until_cloud(&backend, &master, "the idle Cloud stops", |status| {
        status.state == CloudState::Off
    })
    .await;
    let working = backend
        .file_gate(&master, FIRST)
        .await
        .enter(Purpose::Demand)
        .await;
    let listed = list(&backend, &master, FIRST).await;
    assert_eq!(
        listed.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&listed.body)
    );

    // The home grows while the Cloud runs.
    let started = Instant::now();
    let grown = run(
        &mut first,
        "grown",
        &format!(
            "for second in $(seq 180); do size=$(df -B1 --output=size /home | tail -1); \
             [ $size -gt {initial} ] && break; sleep 1; done; echo home-size $size"
        ),
    )
    .await;
    measured(TEST, "growth after the wake", started.elapsed());
    let size: u64 = grown
        .lines()
        .find_map(|line| {
            line.strip_prefix("home-size ")
                .and_then(|size| size.trim().parse().ok())
        })
        .unwrap_or_else(|| panic!("no size: {grown}"));
    assert!(size > initial, "the home stayed at {size} bytes: {grown}");
    drop(working);
    backend.close().await;

    // The saved generation records the grown filesystem's capacity as its
    // superblock gives it, not the size the runner asked for.
    let saved = environment
        .manager()
        .call(ImageStateParams {
            device_id: device.clone(),
        })
        .await
        .unwrap()
        .expect("the saved Cloud has a generation");
    let home = environment.generation(&device, &saved).join("home.ext4");
    assert_eq!(saved.home_bytes.get(), superblock_capacity(&home));
    assert!(saved.home_bytes.get() > initial, "{saved:?}");
}

// Tens of seconds: the Cloud boots, resets twice, stops once, and fails one
// boot, which waits for its runner for as long as the harness allows.
#[tokio::test]
#[ignore = "the Cloud suite: needs a machine manager and the suite's variables (scenarios.md § Cloud suite)"]
async fn a_reset_brings_back_a_cloud_whose_bash_or_runner_is_broken_and_keeps_its_home() {
    const TEST: &str = "reset";
    let _one = one_at_a_time().await;
    let environment = Environment::read();
    let vendor = MockVendor::start().await;
    let mut harness = environment.harness();
    // A runner that cannot start never connects: the boot fails after this.
    harness.cloud.runner_connection = Duration::from_secs(30);
    let backend = harness.start_at(environment.address()).await;
    let master = backend.setup().await;
    let mut first = on_cloud(&backend, &master, &master, &vendor, FIRST, "/a").await;
    let working = backend
        .file_gate(&master, FIRST)
        .await
        .enter(Purpose::Demand)
        .await;

    let broken = run(
        &mut first,
        "break-bash",
        "echo latest-home > ~/note; sudo -n chmod 000 /usr/bin/bash; echo broken",
    )
    .await;
    assert!(broken.contains("broken"), "{broken}");
    let device = the_device(&backend, &master).await;

    // A reset waits for the conversation's file operations to end, so the
    // test lets go of its own before each one.
    drop(working);
    let started = Instant::now();
    reset(&backend, &master, &uuid::Uuid::new_v4().to_string()).await;
    until_cloud(
        &backend,
        &master,
        "the reset of a Cloud without bash is ready",
        |status| {
            status
                .operation
                .as_ref()
                .is_some_and(|operation| operation.phase == ResetPhase::Ready)
        },
    )
    .await;
    measured(TEST, "reset until ready", started.elapsed());
    let working = backend
        .file_gate(&master, FIRST)
        .await
        .enter(Purpose::Demand)
        .await;
    let repaired = run(
        &mut first,
        "repaired",
        "cat ~/note; stat -c %a /usr/bin/bash",
    )
    .await;
    assert!(repaired.contains("latest-home\n755"), "{repaired}");

    // Without its runner's executable, the next boot cannot start one.
    let removed = run(
        &mut first,
        "remove-runner",
        "sudo -n rm /usr/bin/demi-runner; echo removed",
    )
    .await;
    assert!(removed.contains("removed"), "{removed}");
    drop(working);
    until_cloud(&backend, &master, "the idle Cloud stops", |status| {
        status.state == CloudState::Off
    })
    .await;
    let refused = list(&backend, &master, FIRST).await;
    assert_eq!(
        refused.refusal(),
        (StatusCode::SERVICE_UNAVAILABLE, ErrorCode::CloudUnavailable)
    );

    reset(&backend, &master, &uuid::Uuid::new_v4().to_string()).await;
    until_cloud(
        &backend,
        &master,
        "the reset of a Cloud without a runner is ready",
        |status| {
            status
                .operation
                .as_ref()
                .is_some_and(|operation| operation.phase == ResetPhase::Ready)
        },
    )
    .await;
    let working = backend
        .file_gate(&master, FIRST)
        .await
        .enter(Purpose::Demand)
        .await;
    let back = run(
        &mut first,
        "back",
        "cat ~/note; test -x /usr/bin/demi-runner && echo runner-back",
    )
    .await;
    assert!(back.contains("latest-home\nrunner-back"), "{back}");
    assert_eq!(the_device(&backend, &master).await, device);
    drop(working);
    backend.close().await;
}

/// Maps a file of its own, writes a line through the mapping without
/// flushing it, says so, and keeps the mapping while it waits.
const MAPPER: &str = r#"import mmap, os, sys, time
mark = sys.argv[2].encode()
descriptor = os.open(sys.argv[1], os.O_RDWR | os.O_CREAT | os.O_TRUNC, 0o644)
os.ftruncate(descriptor, 4096)
mapping = mmap.mmap(descriptor, 4096, mmap.MAP_SHARED, mmap.PROT_READ | mmap.PROT_WRITE)
mapping[:len(mark)] = mark
print("mapped", flush=True)
time.sleep(600)
"#;

// Tens of seconds: the Cloud boots, Chrome starts in it, the checkpoint copies
// both images, and one conversation's idle window passes.
#[tokio::test]
#[ignore = "the Cloud suite: needs a machine manager and the suite's variables (scenarios.md § Cloud suite)"]
async fn a_checkpoint_with_chrome_open_saves_both_images_with_what_a_mapping_wrote_and_keeps_every_process_until_the_idle_conversations_release()
 {
    const TEST: &str = "checkpoint";
    let _one = one_at_a_time().await;
    let environment = Environment::read();
    let vendor = MockVendor::start().await;
    let harness = environment.harness();
    let backend = harness.start_at(environment.address()).await;
    let master = backend.setup().await;
    let mut first = on_cloud(&backend, &master, &master, &vendor, FIRST, "/a").await;
    let working = backend
        .file_gate(&master, FIRST)
        .await
        .enter(Purpose::Demand)
        .await;
    let session = format!("/home/demi/sessions/{FIRST}");

    let written = run(
        &mut first,
        "page",
        &format!(
            "cat > page.html <<'HTML'\n<!doctype html><title>cloud-suite page</title><p>Chrome on the Cloud</p>\nHTML\n\
             cat > mapper.py <<'PY'\n{MAPPER}PY\necho written"
        ),
    )
    .await;
    assert!(written.contains("written"), "{written}");
    // The model stops watching the mapper once it said it mapped; the
    // mapper keeps its mapping while the checkpoint runs.
    let mapped = run_watched(
        &mut first,
        "mapper",
        "python3 mapper.py mapped 'written through a mapping'",
        5_000,
    )
    .await;
    assert!(mapped.contains("mapped"), "{mapped}");

    let tabs = format!("/api/conversations/{FIRST}/plugins/browser/calls");
    let page = format!("file://{session}/page.html");
    let started = Instant::now();
    let opened = open_tab(&backend, &master, &tabs, &page).await;
    until_titled(&backend, &master, &tabs, &opened, "cloud-suite page").await;
    measured(
        TEST,
        "first Chrome tab until its page shows",
        started.elapsed(),
    );
    let started = Instant::now();
    let again = open_tab(&backend, &master, &tabs, &page).await;
    until_titled(&backend, &master, &tabs, &again, "cloud-suite page").await;
    measured(
        TEST,
        "later Chrome tab until its page shows",
        started.elapsed(),
    );
    // Chrome keeps its own sandbox: its renderers run under seccomp.
    let renderers = run(
        &mut first,
        "renderers",
        "for pid in $(pgrep -f -- '--type=renderer'); do grep '^Seccomp:' /proc/$pid/status; done",
    )
    .await;
    assert!(renderers.contains("Seccomp:\t2"), "{renderers}");

    let device = the_device(&backend, &master).await;
    measure_memory(TEST, &environment, &device);
    let manager = environment.manager();
    let committed =
        |state: Option<MachineImageState>| state.expect("the Cloud has a committed generation");
    let image = || ImageStateParams {
        device_id: device.clone(),
    };
    let before = committed(manager.call(image()).await.unwrap());
    let started = Instant::now();
    manager
        .call(CheckpointParams {
            device_id: device.clone(),
        })
        .await
        .unwrap();
    measured(TEST, "checkpoint", started.elapsed());
    let after = committed(manager.call(image()).await.unwrap());
    assert_ne!(after.generation, before.generation);

    // Both images are in the new generation, and the home image holds what
    // the mapper wrote and never flushed.
    let generation = environment.generation(&device, &after);
    let (system, home) = (generation.join("system.ext4"), generation.join("home.ext4"));
    assert!(
        system.is_file() && home.is_file(),
        "{}",
        generation.display()
    );
    let saved = read_in_image(&home, &format!("/demi/sessions/{FIRST}/mapped"));
    assert!(saved.starts_with("written through a mapping"), "{saved:?}");
    // On ext4 a copy allocates no more blocks than cp's sparse copy.
    let copies = tempfile::tempdir().unwrap();
    for image in [&system, &home] {
        let copy = copies.path().join("copy.ext4");
        let copied = Command::new("cp")
            .args(["--reflink=auto", "--sparse=always"])
            .arg(image)
            .arg(&copy)
            .status()
            .unwrap();
        assert!(copied.success());
        // Delayed allocation counts a new file's extent tree only once the
        // file is written back, as the saved image was.
        std::fs::File::open(&copy).unwrap().sync_all().unwrap();
        let blocks = |path: &Path| std::fs::metadata(path).unwrap().blocks();
        assert!(
            blocks(image) <= blocks(&copy),
            "{}: {} blocks, cp's {}",
            image.display(),
            blocks(image),
            blocks(&copy)
        );
        std::fs::remove_file(&copy).unwrap();
    }

    // The processes live on: Chrome's tabs, and the mapper.
    let listed = list_tabs(&backend, &master, &tabs).await;
    assert_eq!(
        listed.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&listed.body)
    );
    assert_eq!(listed.json::<BrowserTabs>().tabs.len(), 2);
    let alive = run(
        &mut first,
        "alive",
        "pgrep -f mapper.py > /dev/null && echo mapper-alive",
    )
    .await;
    assert!(alive.contains("mapper-alive"), "{alive}");

    // A second conversation keeps the Cloud running while the first goes
    // idle, its mapper ended: the first hears its release there, which
    // closes its Chrome (`resource-lifecycle.md` § Conversation release).
    let mut second = on_cloud(&backend, &master, &master, &vendor, SECOND, "/b").await;
    let busy = backend
        .file_gate(&master, SECOND)
        .await
        .enter(Purpose::Demand)
        .await;
    let ended = run(&mut first, "end", "pkill -f mapper.py; echo ended").await;
    assert!(ended.contains("ended"), "{ended}");
    let started = Instant::now();
    drop(working);
    let deadline = Instant::now() + PATIENCE;
    loop {
        let listed = list_tabs(&backend, &master, &tabs).await;
        assert_eq!(
            listed.status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&listed.body)
        );
        if listed.json::<BrowserTabs>().tabs.is_empty() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the idle conversation's Chrome never closed"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    measured(
        TEST,
        "release of an idle conversation, window included",
        started.elapsed(),
    );
    let held = run(&mut second, "held", "echo held").await;
    assert!(held.contains("held"), "{held}");
    assert_eq!(status(&backend, &master).await.state, CloudState::Running);
    drop(busy);
    backend.close().await;
}

/// Tries each `name host port` target of its standard input and prints
/// whether a connection was made.
const PROBE: &str = r#"import socket, sys
for line in sys.stdin:
    name, host, port = line.split()
    try:
        socket.create_connection((host, int(port)), timeout=3).close()
        print(name, "reached")
    except OSError:
        print(name, "refused")
"#;

// Tens of seconds: two Clouds boot, and each refused connection waits for its
// three-second timeout.
#[tokio::test]
#[ignore = "the Cloud suite: needs a machine manager and the suite's variables (scenarios.md § Cloud suite)"]
async fn two_users_clouds_run_at_once_and_reach_the_backend_but_nothing_else_private() {
    const TEST: &str = "network";
    let _one = one_at_a_time().await;
    let environment = Environment::read();
    let vendor = MockVendor::start().await;
    let harness = environment.harness();
    let backend = harness.start_at(environment.address()).await;
    let master = backend.setup().await;
    harness.add_user("ana@example.test", "ana-pass-1", Role::User);
    let ana = backend.login("ana@example.test", "ana-pass-1").await;
    let mut first = on_cloud(&backend, &master, &master, &vendor, FIRST, "/a").await;
    let mut second = on_cloud(&backend, &master, &ana, &vendor, SECOND, "/b").await;
    let working = backend
        .file_gate(&master, FIRST)
        .await
        .enter(Purpose::Demand)
        .await;
    let also_working = backend
        .file_gate(&ana, SECOND)
        .await
        .enter(Purpose::Demand)
        .await;

    // A service in the second user's Cloud, which the model stops watching
    // once it printed its address.
    let serving = run_watched(
        &mut second,
        "serve",
        "ip -4 -o addr show scope global | awk '{print $4}'; exec python3 -m http.server 8123",
        5_000,
    )
    .await;
    let other: Ipv4Addr = serving
        .lines()
        .find_map(|line| {
            line.trim()
                .split_once('/')
                .and_then(|(address, _)| address.parse().ok())
        })
        .unwrap_or_else(|| panic!("no address: {serving}"));
    // The first user's Cloud boots too: both run at once on the one
    // manager.
    let listed = list(&backend, &master, FIRST).await;
    assert_eq!(
        listed.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&listed.body)
    );
    for session in [&master, &ana] {
        assert_eq!(status(&backend, session).await.state, CloudState::Running);
    }
    assert_ne!(
        the_device(&backend, &master).await,
        the_device(&backend, &ana).await
    );

    // A service of the backend's host other than the backend.
    let host = environment.address().ip();
    let private = tokio::net::TcpListener::bind((host, 0)).await.unwrap();
    let private_port = private.local_addr().unwrap().port();
    let targets = format!(
        "backend {host} {backend_port}\nhost-service {host} {private_port}\nprivate 10.0.0.1 80\n\
         metadata 169.254.169.254 80\nother-cloud {other} 8123\nipv6 fd00::1 80\n",
        backend_port = environment.address().port()
    );
    let probed = run(
        &mut first,
        "probe",
        &format!(
            "cat > probe.py <<'PY'\n{PROBE}PY\npython3 probe.py <<'TARGETS'\n{targets}TARGETS\n\
             echo ipv6-addresses $(ip -6 addr show scope global | wc -l)"
        ),
    )
    .await;
    for expected in [
        "backend reached",
        "host-service refused",
        "private refused",
        "metadata refused",
        "other-cloud refused",
        // IPv6 is off: no address, and no connection.
        "ipv6 refused",
        "ipv6-addresses 0",
    ] {
        assert!(probed.contains(expected), "{expected}: {probed}");
    }
    measure_memory(TEST, &environment, &the_device(&backend, &master).await);
    drop((working, also_working));
    backend.close().await;
}

// Tens of seconds: the Cloud boots, dies, and boots again.
#[tokio::test]
#[ignore = "the Cloud suite: needs a machine manager and the suite's variables (scenarios.md § Cloud suite)"]
async fn a_cloud_whose_sandbox_is_killed_reports_a_death_and_boots_again_with_its_files() {
    const TEST: &str = "runtime loss";
    let _one = one_at_a_time().await;
    let environment = Environment::read();
    let vendor = MockVendor::start().await;
    let harness = environment.harness();
    let backend = harness.start_at(environment.address()).await;
    let master = backend.setup().await;
    let mut first = on_cloud(&backend, &master, &master, &vendor, FIRST, "/a").await;
    let working = backend
        .file_gate(&master, FIRST)
        .await
        .enter(Purpose::Demand)
        .await;
    let wrote = run(
        &mut first,
        "note",
        "echo before-death > ~/note; sync; echo wrote",
    )
    .await;
    assert!(wrote.contains("wrote"), "{wrote}");
    let device = the_device(&backend, &master).await;

    // The boot's Sentry, killed on the host as a crash would.
    #[derive(Deserialize)]
    struct Boot {
        id: String,
    }
    let record = environment
        .data
        .join("working")
        .join(&device)
        .join("sandbox.json");
    let boot: Boot = serde_json::from_slice(&std::fs::read(&record).unwrap()).unwrap();
    let killed = Command::new("pkill")
        .args(["-KILL", "-f", &format!("^runsc-sandbox .*{}", boot.id)])
        .status()
        .unwrap();
    assert!(killed.success(), "no Sentry of {}", boot.id);
    let started = Instant::now();
    until_cloud(&backend, &master, "the dead Cloud is off", |status| {
        status.state == CloudState::Off
    })
    .await;
    measured(TEST, "death until off", started.elapsed());

    let started = Instant::now();
    let back = run(
        &mut first,
        "back",
        "cat ~/note; ls -d /var/lib/demi/jobs/job-* | wc -l",
    )
    .await;
    measured(TEST, "boot after the death and command", started.elapsed());
    // The home outlived the death. The new runner removed the job
    // directories the dead one left (`runner.md` § Pipes and output), so only this
    // job's own is there.
    assert!(back.contains("before-death\n1"), "{back}");
    drop(working);
    backend.close().await;
}
