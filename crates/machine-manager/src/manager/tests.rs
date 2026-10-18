//! The manager's storage state machine without a sandbox: first-use
//! storage, reset, recovery of what a crash left, and reconciliation. Root
//! runs them in throwaway namespaces; the sandbox itself is acceptance's.

use std::{
    net::Ipv4Addr,
    num::{NonZeroU32, NonZeroU64},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    rc::Rc,
};

use demi_machine_manager_protocol::{
    BaseVersion, Capacity, DeviceId, HibernateParams, ImageStateParams, MachineCall,
    MachineImageState, ReconcileParams, ResetParams, RuntimeStateParams, SandboxMachine, Volume,
    WakeParams,
};
use tokio::sync::mpsc;

use super::{Core, Manager, OpError};
use crate::{
    blocking,
    config::{Config, Limits, Mode},
    linux::testing::isolate,
    recovery::{self, RecoveryError},
    server::MachineService,
    storage::{ext4, store::ImagePair},
    tools::{Tool, Tools},
};

struct Fixture {
    /// While it exists, the manager's resize2fs refuses.
    refusal: PathBuf,
    _directory: tempfile::TempDir,
    data: PathBuf,
    core: Rc<Core>,
    manager: Manager,
    base: BaseVersion,
    _deaths: mpsc::Receiver<DeviceId>,
}

/// A manager on a state directory with one imported base, whose skeleton
/// holds a profile. Its runsc lists no sandbox and starts none; its
/// resize2fs refuses while [`Fixture::refuse_resize`] says so.
fn fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path().join("data");
    let program = |name: &str, script: String| {
        let path = directory.path().join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    };
    let runsc = program("runsc", "#!/bin/sh\necho '[]'\n".into());
    let refusal = directory.path().join("refuse-resize");
    let resize2fs = program(
        "resize2fs",
        format!(
            "#!/bin/sh\n[ -e {} ] && {{ echo 'resize2fs: refused' >&2; exit 1; }}\nexec {} \"$@\"\n",
            refusal.display(),
            which::which("resize2fs").unwrap().display(),
        ),
    );
    let config = Config {
        mode: Mode::Serve,
        socket: PathBuf::from("/nonexistent/machines.sock"),
        data: data.clone(),
        runsc: runsc.clone(),
        image: directory.path().join("image"),
        backend_url: "http://203.0.113.10:3271".parse().unwrap(),
        runner_socket: PathBuf::from("/nonexistent/runner/runners.sock"),
        limits: Some(Limits {
            cpus: NonZeroU32::new(2).unwrap(),
            memory_mib: NonZeroU32::new(2048).unwrap(),
            claude_code_memory_mib: NonZeroU32::new(4096).unwrap(),
        }),
        subnet: "172.30.0.0/16".parse().unwrap(),
        slots: 8,
        dns: vec![Ipv4Addr::new(1, 1, 1, 1)],
    };
    let base = BaseVersion::parse("b".repeat(64)).unwrap();
    let rootfs = config.images().join("bases").join(&base).join("rootfs");
    std::fs::create_dir_all(rootfs.join("etc/skel")).unwrap();
    std::fs::write(rootfs.join("etc/skel/.profile"), "export EDITOR=vi\n").unwrap();
    std::fs::write(
        rootfs.parent().unwrap().join("manifest.json"),
        image_manifest().to_string(),
    )
    .unwrap();
    std::fs::create_dir_all(config.working()).unwrap();
    let tools = Tools::on_path()
        .with(Tool::Runsc, runsc)
        .with(Tool::Resize2fs, resize2fs);
    let core = Rc::new(Core::new(config, tools));
    let (deaths, received) = mpsc::channel(4);
    Fixture {
        refusal,
        _directory: directory,
        data,
        manager: Manager::new(core.clone(), base.clone(), deaths),
        core,
        base,
        _deaths: received,
    }
}

impl Fixture {
    /// Makes the manager's resize2fs refuse, or work again.
    fn refuse_resize(&self, refuse: bool) {
        if refuse {
            std::fs::write(&self.refusal, "").unwrap();
        } else {
            std::fs::remove_file(&self.refusal).unwrap();
        }
    }

    async fn call(&self, call: impl Into<MachineCall>) -> Result<serde_json::Value, OpError> {
        self.manager.handle(call.into()).await
    }

    async fn state(&self, device: &str) -> Option<MachineImageState> {
        let result = self
            .call(ImageStateParams {
                device_id: device.into(),
            })
            .await
            .unwrap();
        serde_json::from_value(result).unwrap()
    }

    fn generation(&self, device: &str, state: &MachineImageState) -> ImagePair<PathBuf> {
        self.core
            .store
            .images(&DeviceId::parse(device).unwrap(), &state.generation)
    }

    fn generations(&self, device: &str) -> Vec<String> {
        let mut names: Vec<_> =
            std::fs::read_dir(self.data.join("images").join(device).join("generations"))
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
        names.sort();
        names
    }
}

/// The capacity the tests' resets name: 32 MiB for the system, 48 MiB for
/// the home.
const CAPACITY: Capacity = Capacity {
    system_bytes: NonZeroU64::new(32 << 20).unwrap(),
    home_bytes: NonZeroU64::new(48 << 20).unwrap(),
};

fn reset(device: &str, operation: &str, base: &BaseVersion) -> ResetParams {
    ResetParams {
        device_id: device.into(),
        operation_id: operation.into(),
        base_version: base.to_string(),
        capacity: CAPACITY,
        machine: SandboxMachine::Cloud,
    }
}

/// A user's Cloud's hibernation.
fn hibernate(device: &str) -> HibernateParams {
    HibernateParams {
        device_id: device.into(),
        machine: SandboxMachine::Cloud,
    }
}

fn inode(path: &Path) -> u64 {
    std::fs::metadata(path).unwrap().ino()
}

#[tokio::test(flavor = "local")]
#[ignore = "needs root: run the Linux suite with --ignored as root"]
async fn a_reset_publishes_a_fresh_system_with_the_saved_home_once_per_operation() {
    isolate();
    let fixture = fixture();
    assert_eq!(fixture.state("dev-1").await, None);
    fixture
        .call(reset("dev-1", "op-1", &fixture.base))
        .await
        .unwrap();
    let first = fixture.state("dev-1").await.expect("a generation");
    assert_eq!(first.reset_id.as_deref(), Some("op-1"));
    assert_eq!(first.base_version, fixture.base);
    // A first use makes both filesystems at the capacity named.
    assert_eq!(
        (first.system_bytes.get(), first.home_bytes.get()),
        (32 << 20, 48 << 20)
    );
    // The first use made the initial pair, which the reset replaced but for home.
    assert_eq!(fixture.generations("dev-1").len(), 2);
    let images = fixture.generation("dev-1", &first);

    // The same operation again changes nothing.
    fixture
        .call(reset("dev-1", "op-1", &fixture.base))
        .await
        .unwrap();
    assert_eq!(fixture.state("dev-1").await.as_ref(), Some(&first));

    fixture
        .call(reset("dev-1", "op-2", &fixture.base))
        .await
        .unwrap();
    let second = fixture.state("dev-1").await.expect("a generation");
    assert_eq!(second.reset_id.as_deref(), Some("op-2"));
    assert_ne!(second.generation, first.generation);
    let reset_images = fixture.generation("dev-1", &second);
    assert_eq!(
        inode(&reset_images.home),
        inode(&images.home),
        "home is carried over, not copied"
    );
    assert_ne!(inode(&reset_images.system), inode(&images.system));
    // The current and the previous generation remain.
    let mut kept = vec![first.generation.to_string(), second.generation.to_string()];
    kept.sort();
    assert_eq!(fixture.generations("dev-1"), kept);

    let missing = BaseVersion::parse("c".repeat(64)).unwrap();
    let error = fixture
        .call(reset("dev-1", "op-3", &missing))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("Cloud base {missing} is not imported")
    );
    let invalid = fixture
        .call(ResetParams {
            base_version: "../b".into(),
            ..reset("dev-1", "op-3", &fixture.base)
        })
        .await
        .unwrap_err();
    assert!(matches!(invalid, OpError::InvalidName(_)), "{invalid}");
}

#[tokio::test(flavor = "local")]
#[ignore = "needs root: run the Linux suite with --ignored as root"]
async fn recovery_publishes_the_working_pair_a_crash_left_and_removes_stages() {
    isolate();
    let fixture = fixture();
    fixture
        .call(reset("dev-1", "op-1", &fixture.base))
        .await
        .unwrap();
    let committed = fixture.state("dev-1").await.unwrap();
    // A crash after wake staged the working pair: its images, its record and
    // a stage beside it.
    let working = fixture.data.join("working");
    let pair = working.join("dev-1");
    std::fs::create_dir(&pair).unwrap();
    for volume in Volume::ALL {
        let file = format!("{volume}.ext4");
        std::fs::copy(
            fixture.generation("dev-1", &committed).get(volume),
            pair.join(&file),
        )
        .unwrap();
    }
    std::fs::write(
        pair.join("manifest.json"),
        serde_json::to_vec(&committed).unwrap(),
    )
    .unwrap();
    std::fs::create_dir(working.join(".wake-0f6c3d4e")).unwrap();
    recovery::fence_and_save(&fixture.core, &[]).await.unwrap();
    let saved = fixture.state("dev-1").await.unwrap();
    assert_ne!(saved.generation, committed.generation);
    assert_eq!(saved.reset_id, committed.reset_id);
    assert!(!pair.exists());
    assert!(!working.join(".wake-0f6c3d4e").exists());

    // A working entry that names no device is an error, never skipped.
    std::fs::create_dir(working.join("not a device")).unwrap();
    let error = recovery::fence_and_save(&fixture.core, &[]).await.unwrap_err();
    assert!(matches!(error, RecoveryError::Name(_)), "{error}");
    std::fs::remove_dir(working.join("not a device")).unwrap();

    // A runtime record outside the configured pool stops recovery.
    std::fs::create_dir(&pair).unwrap();
    std::fs::write(
        pair.join("sandbox.json"),
        r#"{"id":"demi-0f6c3d4e","slot":8}"#,
    )
    .unwrap();
    let error = recovery::fence_and_save(&fixture.core, &[]).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "Existing Cloud slot exceeds configured pool"
    );
}

#[tokio::test(flavor = "local")]
#[ignore = "needs root: run the Linux suite with --ignored as root"]
async fn operations_on_a_stopped_device_answer_in_order() {
    isolate();
    let fixture = fixture();
    fixture
        .call(hibernate("dev-1"))
        .await
        .unwrap();
    let state = fixture
        .call(RuntimeStateParams {
            device_id: "dev-1".into(),
        })
        .await
        .unwrap();
    assert_eq!(state, serde_json::json!("stopped"));
    let error = fixture
        .call(hibernate("dev/1"))
        .await
        .unwrap_err();
    assert!(matches!(error, OpError::InvalidName(_)), "{error}");
    // Reconciliation drains the devices, recovers and installs the policy.
    fixture.call(ReconcileParams {}).await.unwrap();
    // After shutdown, requests are refused.
    fixture.manager.close().await.unwrap();
    let error = fixture
        .call(hibernate("dev-1"))
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "the Cloud manager is stopping");
}

/// A wake whose sandbox does not start: its boot names a backend the
/// manager does not allow, so the wake stops after it staged the working
/// pair, before it mounts anything. The runsc that the failed start's close
/// asks lists no sandbox.
fn refused_wake(device: &str, capacity: Capacity) -> WakeParams {
    refused_wake_of(device, capacity, SandboxMachine::Cloud)
}

/// [`refused_wake`] of `machine`.
fn refused_wake_of(device: &str, capacity: Capacity, machine: SandboxMachine) -> WakeParams {
    WakeParams {
        device_id: device.into(),
        capacity,
        boot: serde_json::from_value(serde_json::json!({
            "backendUrl": "http://198.51.100.1:3271",
            "deviceToken": "token",
        }))
        .unwrap(),
        machine,
    }
}

/// The Claude Code machine keeps its current generation alone; a user's
/// Cloud keeps the one before as well, its fallback.
#[tokio::test(flavor = "local")]
#[ignore = "needs root: run the Linux suite with --ignored as root"]
async fn the_claude_code_machine_keeps_one_generation_where_a_cloud_keeps_two() {
    isolate();
    let fixture = fixture();
    for (device, machine) in [
        ("cloud", SandboxMachine::Cloud),
        ("claude-code", SandboxMachine::ClaudeCode),
    ] {
        // Two saves, each of a working pair a wake staged.
        for _ in 0..2 {
            fixture
                .call(refused_wake_of(device, CAPACITY, machine))
                .await
                .unwrap_err();
            fixture
                .call(HibernateParams {
                    device_id: device.into(),
                    machine,
                })
                .await
                .unwrap();
        }
    }
    assert_eq!(fixture.generations("cloud").len(), 2);
    let current = fixture.state("claude-code").await.unwrap();
    assert_eq!(
        fixture.generations("claude-code"),
        [current.generation.to_string()]
    );
}

/// The blocks a file of `path` takes on its filesystem, in bytes.
fn allocated(path: &Path) -> u64 {
    std::fs::metadata(path).unwrap().blocks() * 512
}

/// A Cloud wrote a large file to its home and deleted it, then its manager
/// stopped. The save that follows its recovery holds only what the home
/// still holds: the deleted file's blocks were given back to the host.
/// Cost: a 256 MiB home that 128 MiB fill and free; about a second.
#[tokio::test(flavor = "local")]
#[ignore = "needs root: run the Linux suite with --ignored as root"]
async fn a_save_gives_back_the_space_of_what_the_cloud_deleted() {
    use crate::{
        linux::{loopdev, mount},
        sandbox::files::{RuntimeDirectory, SandboxId, SandboxRecord},
    };
    const MIB: u64 = 1 << 20;
    isolate();
    let fixture = fixture();
    let capacity = Capacity {
        system_bytes: NonZeroU64::new(32 * MIB).unwrap(),
        home_bytes: NonZeroU64::new(256 * MIB).unwrap(),
    };
    // A working pair staged; then the boot as a manager's stop left it: its
    // record, and its home mounted where the sandbox had it.
    fixture.call(refused_wake("dev-1", capacity)).await.unwrap_err();
    let pair = fixture.data.join("working/dev-1");
    let record = SandboxRecord {
        id: SandboxId::new(),
        slot: 0,
    };
    std::fs::write(pair.join("sandbox.json"), serde_json::to_vec(&record).unwrap()).unwrap();
    let directory = RuntimeDirectory::new(fixture.core.config.runtime(), record.id.as_str());
    let home = directory.volume(Volume::Home);
    std::fs::create_dir_all(&home).unwrap();
    let image = pair.join("home.ext4");
    let mounted = (image.clone(), home.clone());
    blocking::run(move |off| -> std::io::Result<()> {
        let device = loopdev::attach(off, &mounted.0)?;
        mount::ext4(off, &device.path(), &mounted.1)
    })
    .await
    .unwrap();

    // The Cloud writes 128 MiB that are not zeros, and deletes them.
    let file = home.join("large");
    let chunk: Vec<u8> = (0..MIB).map(|byte| (byte % 251 + 1) as u8).collect();
    {
        use std::io::Write as _;
        let mut writing = std::fs::File::create(&file).unwrap();
        for _ in 0..128 {
            writing.write_all(&chunk).unwrap();
        }
        writing.sync_all().unwrap();
    }
    std::fs::remove_file(&file).unwrap();
    rustix::fs::syncfs(std::fs::File::open(&home).unwrap()).unwrap();
    assert!(allocated(&image) >= 128 * MIB, "{} bytes", allocated(&image));

    // Recovery closes the boot and saves its pair.
    fixture.call(ReconcileParams {}).await.unwrap();
    let saved = fixture.state("dev-1").await.unwrap();
    let images = fixture.generation("dev-1", &saved);
    assert!(
        allocated(&images.home) < 64 * MIB,
        "the saved home takes {} bytes",
        allocated(&images.home)
    );
}

/// The text of `path` in the ext4 image `image`, as `debugfs` reads it.
fn read_in_image(image: &Path, path: &str) -> String {
    let output = std::process::Command::new("debugfs")
        .arg("-R")
        .arg(format!("cat {path}"))
        .arg(image)
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Cost: makes a 1 GiB pair and enlarges it to 16 GiB, sparse; about a
/// second.
#[tokio::test(flavor = "local")]
#[ignore = "needs root: run the Linux suite with --ignored as root"]
async fn a_wake_enlarges_a_smaller_saved_pair_to_the_capacity_named_and_keeps_its_files() {
    const GIB: u64 = 1 << 30;
    isolate();
    let fixture = fixture();

    // A Cloud first made at 1 GiB each, as before the capacity was 16 GiB.
    let small = Capacity {
        system_bytes: NonZeroU64::new(GIB).unwrap(),
        home_bytes: NonZeroU64::new(GIB).unwrap(),
    };
    let large = Capacity {
        system_bytes: NonZeroU64::new(16 * GIB).unwrap(),
        home_bytes: NonZeroU64::new(16 * GIB).unwrap(),
    };
    fixture.call(refused_wake("dev-1", small)).await.unwrap_err();
    fixture.call(hibernate("dev-1")).await.unwrap();
    let saved = fixture.state("dev-1").await.unwrap();
    assert_eq!((saved.system_bytes.get(), saved.home_bytes.get()), (GIB, GIB));

    // The next wake names 16 GiB: both images are enlarged before they would
    // be mounted, and the save records what each superblock says.
    let refused = fixture.call(refused_wake("dev-1", large)).await.unwrap_err();
    assert_eq!(refused.to_string(), "Cloud backend differs from configured allowlist");
    fixture.call(hibernate("dev-1")).await.unwrap();
    let woken = fixture.state("dev-1").await.unwrap();
    assert_eq!((woken.system_bytes.get(), woken.home_bytes.get()), (16 * GIB, 16 * GIB));
    let images = fixture.generation("dev-1", &woken);
    for volume in Volume::ALL {
        let image = images.get(volume).clone();
        let read = blocking::run(move |off| ext4::capacity(off, &image)).await.unwrap();
        assert_eq!(read.get(), 16 * GIB, "{volume}");
    }
    // The home's files are as they were.
    assert_eq!(read_in_image(&images.home, "/demi/.profile"), "export EDITOR=vi\n");

    // A wake that names less keeps the larger images.
    fixture.call(refused_wake("dev-1", small)).await.unwrap_err();
    fixture.call(hibernate("dev-1")).await.unwrap();
    let kept = fixture.state("dev-1").await.unwrap();
    assert_eq!((kept.system_bytes.get(), kept.home_bytes.get()), (16 * GIB, 16 * GIB));
}

/// A valid image manifest whose runner speaks this build's protocols, as a
/// base the manager imported names it.
fn image_manifest() -> serde_json::Value {
    let runner = serde_json::json!({ "sha256": "a".repeat(64), "size": 38710848 });
    let (architecture, target) = if cfg!(target_arch = "aarch64") {
        ("arm64", "aarch64-unknown-linux-musl")
    } else {
        ("amd64", "x86_64-unknown-linux-musl")
    };
    serde_json::json!({
        "formatVersion": 1,
        "os": "linux",
        "architecture": architecture,
        "rootfs": { "sha256": "d".repeat(64), "size": 733308752, "file": "rootfs.tar.zst" },
        "ubuntu": "26.04",
        "packages": [],
        "executables": { "/opt/demi/bin/demi-runner": runner },
        "releases": [],
        "runner": {
            "release": "f".repeat(64),
            "wire": demi_runner_protocol::wire::VERSION,
            "commandProtocol": 1,
            "targets": { target: runner },
        },
    })
}

/// Cost: makes a 1 GiB pair, reads it twice and enlarges it to 16 GiB,
/// once refused and once done; about half a minute.
#[tokio::test(flavor = "local")]
#[ignore = "needs root: run the Linux suite with --ignored as root"]
async fn an_enlargement_that_fails_midway_keeps_the_saved_pair_and_the_next_wake_enlarges_it() {
    const GIB: u64 = 1 << 30;
    isolate();
    let fixture = fixture();
    let small = Capacity {
        system_bytes: NonZeroU64::new(GIB).unwrap(),
        home_bytes: NonZeroU64::new(GIB).unwrap(),
    };
    let large = Capacity {
        system_bytes: NonZeroU64::new(16 * GIB).unwrap(),
        home_bytes: NonZeroU64::new(16 * GIB).unwrap(),
    };
    fixture.call(refused_wake("dev-1", small)).await.unwrap_err();
    fixture.call(hibernate("dev-1")).await.unwrap();
    let saved = fixture.state("dev-1").await.unwrap();
    let images = fixture.generation("dev-1", &saved);
    let digest = |path: &Path| {
        let output = std::process::Command::new("sha256sum").arg(path).output().unwrap();
        String::from_utf8(output.stdout).unwrap()
    };
    let before: Vec<_> = Volume::ALL.iter().map(|volume| digest(images.get(*volume))).collect();

    // resize2fs refuses after the copy's file was extended: the wake fails
    // with its error, and the saved pair is as it was.
    fixture.refuse_resize(true);
    let failed = fixture.call(refused_wake("dev-1", large)).await.unwrap_err();
    assert!(crate::server::chain(&failed).contains("resize2fs: refused"), "{}", crate::server::chain(&failed));
    fixture.call(hibernate("dev-1")).await.unwrap();
    assert_eq!(fixture.state("dev-1").await.unwrap(), saved);
    let after: Vec<_> = Volume::ALL.iter().map(|volume| digest(images.get(*volume))).collect();
    assert_eq!(after, before);

    // The next wake enlarges both images.
    fixture.refuse_resize(false);
    let refused = fixture.call(refused_wake("dev-1", large)).await.unwrap_err();
    assert_eq!(refused.to_string(), "Cloud backend differs from configured allowlist");
    fixture.call(hibernate("dev-1")).await.unwrap();
    let woken = fixture.state("dev-1").await.unwrap();
    assert_eq!((woken.system_bytes.get(), woken.home_bytes.get()), (16 * GIB, 16 * GIB));
    let images = fixture.generation("dev-1", &woken);
    assert_eq!(read_in_image(&images.home, "/demi/.profile"), "export EDITOR=vi\n");
}

