//! Test support: an in-process fake runner ([`TestLink`]) over the real
//! connection driver, a real runner process for a backend at any address
//! ([`RunnerProcess`]) and one connected to a backend end of its own for one
//! device ([`RunnerFixture`]), the native fixture package their tests run,
//! and a connection policy that runs every call in one command set.

mod process;
mod runner;

pub use process::{RunnerProcess, RunnerProcessOptions, native_fixture_binary, runner_binary};
pub use runner::{FixtureOptions, RunnerFixture};

use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    rc::Rc,
    time::Duration,
};

use demi_command_service::protocol::{
    ArtifactLocation, ArtifactPath, PackageArtifact, PackageDescriptor, host_target,
};
use demi_runner_protocol::wire::{self, Inbound, Outbound, VolumeName};
use demi_shell::{
    CommandSet, HostIdentity, HostKey, PortError, RpcError, RpcInvocation, RpcPort, StorageOp,
    StorageReply, testing::MemoryStorage,
};
use futures_util::future::LocalBoxFuture;
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::{CancellationToken, PollSender};

use crate::{
    Admission, ArtifactResolver, DeviceLink, JobOrigin, Link, LinkEnd, LinkOptions, LinkPolicy,
    Pipes, RemoteHost,
};

/// The device a test connection serves.
pub const TEST_DEVICE: &str = "test-device";

/// A policy for tests: every call runs, in one command set, and each agent
/// node's command storage is in memory.
pub struct CommandPolicy {
    commands: Rc<CommandSet>,
    storage: RefCell<HashMap<String, Rc<MemoryStorage>>>,
}

impl CommandPolicy {
    pub fn new(commands: CommandSet) -> Rc<Self> {
        Rc::new(Self {
            commands: Rc::new(commands),
            storage: RefCell::default(),
        })
    }

    /// The command storage of agent node `node`.
    pub fn storage(&self, node: &str) -> Rc<MemoryStorage> {
        self.storage
            .borrow_mut()
            .entry(node.to_owned())
            .or_insert_with(MemoryStorage::new)
            .clone()
    }
}

impl LinkPolicy for CommandPolicy {
    fn admit_call(&self, _: &JobOrigin) -> Result<(), String> {
        Ok(())
    }

    fn dispatch(
        &self,
        _: Rc<JobOrigin>,
        invocation: RpcInvocation,
        port: RpcPort,
    ) -> LocalBoxFuture<'static, Result<u8, RpcError>> {
        let commands = self.commands.clone();
        Box::pin(async move { commands.dispatch(invocation, port).await })
    }

    fn storage(
        &self,
        job: Rc<JobOrigin>,
        op: StorageOp,
        call: CancellationToken,
    ) -> LocalBoxFuture<'static, Result<StorageReply, PortError>> {
        let storage = job
            .caller
            .as_ref()
            .map(|caller| self.storage(caller.node.as_str()));
        Box::pin(async move {
            let storage = storage
                .ok_or_else(|| PortError::Storage("the job has no command storage".into()))?;
            // A stopped call commits nothing.
            if call.is_cancelled() {
                return Err(PortError::Ended("the call was stopped".into()));
            }
            Ok(storage.apply(op))
        })
    }

    fn grow_volume(&self, _: VolumeName, _: u64) -> LocalBoxFuture<'static, Result<(), String>> {
        Box::pin(async { Err("volume growth is not available".into()) })
    }
}

/// A device whose runner is the test: connections to it are [`TestLink`]s.
pub struct TestDevice {
    link: Rc<watch::Sender<DeviceLink>>,
    pipes: Pipes,
    policy: Rc<dyn LinkPolicy>,
    identity: HostIdentity,
}

impl TestDevice {
    pub fn new(policy: Rc<dyn LinkPolicy>) -> Self {
        Self {
            link: Rc::new(watch::Sender::new(DeviceLink::Offline { last: None })),
            pipes: Pipes::new(crate::ARRIVAL),
            policy,
            identity: HostIdentity {
                uid: 501,
                gid: 20,
                hostname: "test".into(),
                home_dir: "/work".into(),
            },
        }
    }

    pub fn pipes(&self) -> &Pipes {
        &self.pipes
    }

    /// A Host on the device, starting work in `cwd`.
    pub fn host(&self, cwd: &str, admission: Admission) -> RemoteHost {
        RemoteHost::new(
            HostKey::new(format!("{TEST_DEVICE}:{cwd}")),
            cwd.into(),
            self.link.subscribe(),
            admission,
        )
    }

    /// Connects the device's runner; `ping` turns liveness on.
    pub fn connect(&self, ping: Option<Duration>) -> TestLink {
        let (link, driver) = Link::new(LinkOptions {
            device: TEST_DEVICE.into(),
            identity: self.identity.clone(),
            pipes: self.pipes.clone(),
            policy: self.policy.clone(),
            ping,
        });
        let (runner, incoming) = mpsc::channel::<Result<Vec<u8>, String>>(8);
        let (outgoing, sent) = mpsc::channel::<Vec<u8>>(crate::OUTBOUND_FRAMES);
        let incoming = futures_util::stream::unfold(incoming, |mut incoming| async move {
            incoming.recv().await.map(|frame| (frame, incoming))
        });
        self.link.send_replace(DeviceLink::Online(link.clone()));
        let device = self.link.clone();
        let identity = self.identity.clone();
        let served = {
            let link = link.clone();
            tokio::task::spawn_local(async move {
                let end = driver.serve(incoming, PollSender::new(outgoing)).await;
                went_offline(&device, &link, identity);
                end
            })
        };
        TestLink {
            link,
            sent,
            runner: Some(runner),
            served,
        }
    }
}

/// Marks the device offline once `link` ended, unless a newer connection
/// already serves it.
fn went_offline(device: &watch::Sender<DeviceLink>, link: &Link, identity: HostIdentity) {
    device.send_if_modified(|current| {
        let ended = matches!(current, DeviceLink::Online(current) if current == link);
        if ended {
            *current = DeviceLink::Offline {
                last: Some(identity),
            };
        }
        ended
    });
}

/// One connection with the test as its runner: it reads what the backend
/// sends and sends what a runner would.
pub struct TestLink {
    link: Link,
    sent: mpsc::Receiver<Vec<u8>>,
    runner: Option<mpsc::Sender<Result<Vec<u8>, String>>>,
    served: tokio::task::JoinHandle<LinkEnd>,
}

impl TestLink {
    pub fn link(&self) -> &Link {
        &self.link
    }

    /// The next message the backend sent.
    pub async fn next(&mut self) -> Inbound {
        let frame = self.sent.recv().await.expect("the connection is served");
        wire::decode(&frame).expect("the backend sends valid messages")
    }

    /// The next message the backend sent, when it sent one already.
    pub fn try_next(&mut self) -> Option<Inbound> {
        let frame = self.sent.try_recv().ok()?;
        Some(wire::decode(&frame).expect("the backend sends valid messages"))
    }

    /// Sends `message` as the runner, through the wire's encoding.
    pub async fn send(&self, message: Outbound) {
        let frame = wire::encode(&message)
            .expect("a valid runner message")
            .into_bytes();
        self.send_frame(frame).await;
    }

    /// Sends raw bytes as the runner.
    pub async fn send_frame(&self, frame: Vec<u8>) {
        let runner = self.runner.as_ref().expect("the runner is connected");
        // The driver ended when nothing receives; its end says why.
        let _ = runner.send(Ok(frame)).await;
    }

    /// The runner goes away; resolves with why the connection ended.
    pub async fn close(mut self) -> LinkEnd {
        self.runner = None;
        self.ended().await
    }

    /// Why the connection ended, once it has.
    pub async fn ended(self) -> LinkEnd {
        self.served
            .await
            .expect("the connection driver does not panic")
    }
}

/// The native fixture package, whose operations the runner's own tests use
/// (`where`, `echo`, `first`, `spin`, `result`, `retain`, `crash`): its
/// descriptor for this host and where its executable is.
pub struct NativeFixture {
    pub descriptor: PackageDescriptor,
    path: PathBuf,
}

impl NativeFixture {
    pub fn load() -> Self {
        let path = native_fixture_binary();
        let bytes = std::fs::read(&path).unwrap_or_else(|error| {
            panic!(
                "read {}: {error}; build it with cargo build --workspace --features demi-runner/test-fixtures",
                path.display()
            )
        });
        let descriptor = PackageDescriptor {
            id: "demicodes.runner-test".into(),
            version: "test".into(),
            protocol_version: 1,
            operations: [
                "where", "echo", "first", "spin", "result", "retain", "crash",
            ]
            .map(String::from)
            .to_vec(),
            targets: BTreeMap::from([(
                host_target().to_owned(),
                PackageArtifact {
                    sha256: format!("{:x}", Sha256::digest(&bytes)),
                    size: bytes.len() as u64,
                },
            )]),
        };
        Self { descriptor, path }
    }

    /// Resolves the package's executable to its path on this machine.
    pub fn resolver(&self) -> Rc<dyn ArtifactResolver> {
        Rc::new(LocalArtifact {
            path: self.path.to_string_lossy().into_owned(),
            artifact: self.descriptor.targets[host_target()].clone(),
        })
    }
}

struct LocalArtifact {
    path: String,
    artifact: PackageArtifact,
}

impl ArtifactResolver for LocalArtifact {
    fn resolve(
        &self,
        artifact: &PackageArtifact,
        _: &str,
        _: CancellationToken,
    ) -> LocalBoxFuture<'static, Result<ArtifactLocation, String>> {
        let found = (*artifact == self.artifact).then(|| {
            ArtifactLocation::Path(ArtifactPath {
                path: self.path.clone(),
            })
        });
        Box::pin(async move { found.ok_or_else(|| "the artifact is not the fixture's".into()) })
    }
}
