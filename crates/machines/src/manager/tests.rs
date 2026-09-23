//! The manager's storage state machine without a sandbox: first-use
//! storage, reset, recovery of what a crash left, and reconciliation. Root
//! runs them in throwaway namespaces; the sandbox itself is acceptance's.

use std::{
    net::Ipv4Addr,
    num::{NonZeroU32, NonZeroU64},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    rc::Rc,
};

use demi_machines_protocol::{
    BaseVersion, DeviceId, GrowVolumeParams, HibernateParams, ImageStateParams, MachineCall, MachineImageState,
    ReconcileParams, ResetParams, RuntimeStateParams, Volume,
};
use tokio::sync::mpsc;

use super::{Core, Manager, OpError};
use crate::{
    config::{Config, Mode},
    linux::testing::isolate,
    recovery::{self, RecoveryError},
    server::MachineService,
    storage::store::ImagePair,
    tools::Tools,
};

struct Fixture {
    _directory: tempfile::TempDir,
    data: PathBuf,
    core: Rc<Core>,
    manager: Manager,
    base: BaseVersion,
    _deaths: mpsc::Receiver<DeviceId>,
}

/// A manager on a state directory with one imported base, whose skeleton
/// holds a profile. runsc is never run.
fn fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path().join("data");
    let config = Config {
        mode: Mode::Serve,
        socket: None,
        data: data.clone(),
        runsc: PathBuf::from("/nonexistent/runsc"),
        image: directory.path().join("image"),
        backend_url: "http://203.0.113.10:3271".parse().unwrap(),
        cpus: NonZeroU32::new(2).unwrap(),
        memory_mib: NonZeroU32::new(2048).unwrap(),
        system_mib: NonZeroU32::new(32).unwrap(),
        home_mib: NonZeroU32::new(32).unwrap(),
        subnet: "172.30.0.0/16".parse().unwrap(),
        slots: 8,
        dns: vec![Ipv4Addr::new(1, 1, 1, 1)],
    };
    let base = BaseVersion::parse("b".repeat(64)).unwrap();
    let rootfs = config.images().join("bases").join(&base).join("rootfs");
    std::fs::create_dir_all(rootfs.join("etc/skel")).unwrap();
    std::fs::write(rootfs.join("etc/skel/.profile"), "export EDITOR=vi\n").unwrap();
    std::fs::write(rootfs.parent().unwrap().join("manifest.json"), "{}").unwrap();
    std::fs::create_dir_all(config.working()).unwrap();
    let core = Rc::new(Core::new(config, Tools::on_path()));
    let (deaths, received) = mpsc::channel(4);
    Fixture {
        _directory: directory,
        data,
        manager: Manager::new(core.clone(), base.clone(), deaths),
        core,
        base,
        _deaths: received,
    }
}

impl Fixture {
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
        self.core.store.images(&DeviceId::parse(device).unwrap(), &state.generation)
    }

    fn generations(&self, device: &str) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(self.data.join("images").join(device).join("generations"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

fn reset(device: &str, operation: &str, base: &BaseVersion) -> ResetParams {
    ResetParams {
        device_id: device.into(),
        operation_id: operation.into(),
        base_version: base.to_string(),
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
    fixture.call(reset("dev-1", "op-1", &fixture.base)).await.unwrap();
    let first = fixture.state("dev-1").await.expect("a generation");
    assert_eq!(first.reset_id.as_deref(), Some("op-1"));
    assert_eq!(first.base_version, fixture.base);
    assert_eq!((first.system_bytes.get(), first.home_bytes.get()), (32 << 20, 32 << 20));
    // The first use made the initial pair, which the reset replaced but for home.
    assert_eq!(fixture.generations("dev-1").len(), 2);
    let images = fixture.generation("dev-1", &first);

    // The same operation again changes nothing.
    fixture.call(reset("dev-1", "op-1", &fixture.base)).await.unwrap();
    assert_eq!(fixture.state("dev-1").await.as_ref(), Some(&first));

    fixture.call(reset("dev-1", "op-2", &fixture.base)).await.unwrap();
    let second = fixture.state("dev-1").await.expect("a generation");
    assert_eq!(second.reset_id.as_deref(), Some("op-2"));
    assert_ne!(second.generation, first.generation);
    let reset_images = fixture.generation("dev-1", &second);
    assert_eq!(inode(&reset_images.home), inode(&images.home), "home is carried over, not copied");
    assert_ne!(inode(&reset_images.system), inode(&images.system));
    // The current and the previous generation remain.
    let mut kept = vec![first.generation.to_string(), second.generation.to_string()];
    kept.sort();
    assert_eq!(fixture.generations("dev-1"), kept);

    let missing = BaseVersion::parse("c".repeat(64)).unwrap();
    let error = fixture.call(reset("dev-1", "op-3", &missing)).await.unwrap_err();
    assert_eq!(error.to_string(), format!("Cloud base {missing} is not imported"));
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
    fixture.call(reset("dev-1", "op-1", &fixture.base)).await.unwrap();
    let committed = fixture.state("dev-1").await.unwrap();
    // A crash after wake staged the working pair: its images, its record and
    // a stage beside it.
    let working = fixture.data.join("working");
    let pair = working.join("dev-1");
    std::fs::create_dir(&pair).unwrap();
    for volume in Volume::ALL {
        let file = format!("{volume}.ext4");
        std::fs::copy(fixture.generation("dev-1", &committed).get(volume), pair.join(&file)).unwrap();
    }
    std::fs::write(pair.join("manifest.json"), serde_json::to_vec(&committed).unwrap()).unwrap();
    std::fs::create_dir(working.join(".wake-0f6c3d4e")).unwrap();
    recovery::fence_and_save(&fixture.core).await.unwrap();
    let saved = fixture.state("dev-1").await.unwrap();
    assert_ne!(saved.generation, committed.generation);
    assert_eq!(saved.reset_id, committed.reset_id);
    assert!(!pair.exists());
    assert!(!working.join(".wake-0f6c3d4e").exists());

    // A working entry that names no device is an error, never skipped.
    std::fs::create_dir(working.join("not a device")).unwrap();
    let error = recovery::fence_and_save(&fixture.core).await.unwrap_err();
    assert!(matches!(error, RecoveryError::Name(_)), "{error}");
    std::fs::remove_dir(working.join("not a device")).unwrap();

    // A runtime record outside the configured pool stops recovery.
    std::fs::create_dir(&pair).unwrap();
    std::fs::write(pair.join("sandbox.json"), r#"{"id":"demi-0f6c3d4e","slot":8}"#).unwrap();
    let error = recovery::fence_and_save(&fixture.core).await.unwrap_err();
    assert_eq!(error.to_string(), "Existing Cloud slot exceeds configured pool");
}

#[tokio::test(flavor = "local")]
#[ignore = "needs root: run the Linux suite with --ignored as root"]
async fn operations_on_a_stopped_device_answer_in_order() {
    isolate();
    let fixture = fixture();
    fixture
        .call(HibernateParams {
            device_id: "dev-1".into(),
        })
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
        .call(GrowVolumeParams {
            device_id: "dev-1".into(),
            volume: Volume::Home,
            bytes: NonZeroU64::new(1 << 30).unwrap(),
        })
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "Cloud is not running");
    let error = fixture
        .call(HibernateParams {
            device_id: "dev/1".into(),
        })
        .await
        .unwrap_err();
    assert!(matches!(error, OpError::InvalidName(_)), "{error}");
    // Reconciliation drains the devices, recovers and installs the policy.
    fixture.call(ReconcileParams {}).await.unwrap();
    // After shutdown, requests are refused.
    fixture.manager.close().await.unwrap();
    let error = fixture
        .call(HibernateParams {
            device_id: "dev-1".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "the Cloud manager is stopping");
}
