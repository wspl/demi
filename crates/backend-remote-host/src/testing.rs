//! Test support:
//!
//! - An in-process fake runner ([`TestLink`]) over the real connection driver.
//! - A real runner process for a backend at any address ([`RunnerProcess`]),
//!   and one connected to a backend end of its own for one device
//!   ([`RunnerFixture`]).
//! - The native fixture package their tests run.
//! - A connection policy that runs every call in one command set.
//! - A pseudo-terminal to run a program at as a person does ([`Terminal`]).
//! - The guard that stops the runners a test starts in sessions of their own
//!   ([`DetachedRunners`]).

#[cfg(unix)]
mod detached;
mod network;
mod process;
mod runner;
#[cfg(unix)]
mod terminal;

#[cfg(unix)]
pub use detached::DetachedRunners;
pub use process::{
    PAIRING_CODE, RunnerProcess, RunnerProcessOptions, native_fixture_binary, runner_binary,
    service_manager_stand_in,
};
#[cfg(unix)]
pub use process::{executable, login_shell_stand_in};
pub use runner::{FixtureOptions, RunnerFixture};
#[cfg(unix)]
pub use terminal::Terminal;

use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    rc::Rc,
};

use bytes::Bytes;
use demi_command_protocol::{
    ArtifactLocation, ArtifactPath, PackageArtifact, PackageDescriptor, ServiceSequence,
    host_target,
};
use demi_host_interface::{CommandSet, HostIdentity, HostKey, RpcError, RpcInvocation, RpcPort};
use demi_runner_protocol::wire::{self, Inbound, Outbound};
use demi_shared_types::BlobRef;
use futures_util::future::LocalBoxFuture;
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

use crate::{
    Admission, ArtifactResolver, Health, Outgoing, DeviceJobs, DeviceLink, Hello, JobOrigin, Link, LinkEnd,
    LinkOptions, LinkPolicy, Pipes, RemoteHost,
};

/// How many requests the runner answered among the messages a fixture's
/// tap received so far, which it takes: each Host request is answered once,
/// so this counts the round trips an operation cost.
pub fn answered_requests(tap: &mut mpsc::Receiver<Outbound>) -> usize {
    let mut answered = 0;
    while let Ok(message) = tap.try_recv() {
        if matches!(
            message,
            Outbound::FsOk(_)
                | Outbound::FsError { .. }
                | Outbound::GitOk(_)
                | Outbound::GitError { .. }
                | Outbound::JobRead { .. }
                | Outbound::JobMediaRead { .. }
        ) {
            answered += 1;
        }
    }
    answered
}

/// The frames a test connection queues for its runner before a sender waits.
pub const OUTBOUND_FRAMES: usize = 64;

/// The device a test connection serves.
pub const TEST_DEVICE: &str = "test-device";

/// A policy for tests: every call runs, in one command set.
pub struct CommandPolicy {
    commands: Rc<CommandSet>,
    /// The next number of each conversation's sequence.
    sequences: RefCell<HashMap<(String, ServiceSequence), u64>>,
    /// The package events the connection handed on, in order.
    package_events: watch::Sender<Vec<crate::PackageEvent>>,
}

impl CommandPolicy {
    pub fn new(commands: CommandSet) -> Rc<Self> {
        Rc::new(Self {
            commands: Rc::new(commands),
            sequences: RefCell::default(),
            package_events: watch::Sender::new(Vec::new()),
        })
    }

    /// The package events the connection hands on, in order, as they come.
    pub fn package_events(&self) -> watch::Receiver<Vec<crate::PackageEvent>> {
        self.package_events.subscribe()
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

    // No test handler returns a medium.
    fn read_blob(&self, _: BlobRef) -> LocalBoxFuture<'static, Result<Option<Bytes>, String>> {
        Box::pin(async { Ok(None) })
    }


    fn revoke_device(&self) -> LocalBoxFuture<'static, Result<(), String>> {
        Box::pin(async { Err("revocation is not available".into()) })
    }

    fn take_up_job(&self, _: String) -> LocalBoxFuture<'static, Result<(), String>> {
        Box::pin(async { Err("no agent takes jobs up here".into()) })
    }

    /// Counts each conversation's sequence from 1, as the backend's
    /// sequences do for a new conversation.
    fn reserve_numbers(
        &self,
        conversation: String,
        sequence: ServiceSequence,
        count: u32,
    ) -> LocalBoxFuture<'static, Result<u64, String>> {
        let mut sequences = self.sequences.borrow_mut();
        let next = sequences.entry((conversation, sequence)).or_insert(1);
        let first = *next;
        *next += u64::from(count);
        Box::pin(async move { Ok(first) })
    }
    fn direct_stream(
        &self,
        _: String,
        _: String,
    ) -> LocalBoxFuture<'static, Result<crate::DirectAdmission, String>> {
        Box::pin(async { Err("direct streams are not available".into()) })
    }

    fn watch_stream(&self, _: String) -> LocalBoxFuture<'static, Result<demi_shared_gates::GateLease, String>> {
        Box::pin(async { Err("direct streams are not available".into()) })
    }

    /// Keeps every event, for the test to read.
    fn package_event(&self, event: crate::PackageEvent) -> LocalBoxFuture<'static, ()> {
        self.package_events.send_modify(|events| events.push(event));
        Box::pin(async {})
    }

    // These runners forward no port.
    fn forward_event(&self, _: crate::ForwardEvent) {}
}

/// A device whose runner is the test: connections to it are [`TestLink`]s.
pub struct TestDevice {
    link: Rc<watch::Sender<DeviceLink>>,
    jobs: DeviceJobs,
    pipes: Pipes,
    policy: Rc<dyn LinkPolicy>,
    identity: HostIdentity,
}

impl TestDevice {
    pub fn new(policy: Rc<dyn LinkPolicy>) -> Self {
        Self {
            link: Rc::new(watch::Sender::new(DeviceLink::Offline { last: None })),
            jobs: DeviceJobs::default(),
            pipes: Pipes::new(crate::ARRIVAL),
            policy,
            identity: HostIdentity {
                uid: 501,
                gid: 20,
                hostname: "test".into(),
                home_dir: "/work".into(),
                scratch_root: "/demi/scratch".into(),
                paths: demi_host_interface::HostPaths::Posix,
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
            self.jobs.clone(),
            admission,
        )
    }

    /// Connects the device's runner, which keeps no jobs.
    pub fn connect(&self) -> TestLink {
        self.connect_with(1, Vec::new())
    }

    /// Connects the device's runner of instance `instance`, which keeps
    /// `jobs`.
    pub fn connect_with(&self, instance: u64, jobs: Vec<wire::KeptJob>) -> TestLink {
        self.connect_hello(
            Hello {
                instance,
                release: "0".into(),
                jobs,
                last_release: None,
                last_instance: None,
                managed: false,
                recorded: Default::default(),
            },
        )
    }

    /// Connects the device's runner, whose hello and what the backend
    /// knows of the device are `hello`. Everything the backend sends is one
    /// queue the test reads, whatever the lane of a frame.
    pub fn connect_hello(&self, hello: Hello) -> TestLink {
        let (outgoing, sent) = Outgoing::plain(OUTBOUND_FRAMES);
        let (link, driver) = Link::new(LinkOptions {
            device: TEST_DEVICE.into(),
            identity: self.identity.clone(),
            kernel: None,
            shell: None,
            pipes: self.pipes.clone(),
            policy: self.policy.clone(),
            outgoing,
            health: Health::inert(),
            jobs: self.jobs.clone(),
            hello,
        });
        let (runner, incoming) = mpsc::channel::<Result<Vec<u8>, crate::SocketEnd>>(8);
        let incoming = futures_util::stream::unfold(incoming, |mut incoming| async move {
            incoming.recv().await.map(|frame| (frame, incoming))
        });
        self.link.send_replace(DeviceLink::Online(link.clone()));
        let device = self.link.clone();
        let identity = self.identity.clone();
        let served = {
            let link = link.clone();
            tokio::task::spawn_local(async move {
                let end = driver.serve(incoming).await;
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
    sent: mpsc::Receiver<wire::Frame>,
    runner: Option<mpsc::Sender<Result<Vec<u8>, crate::SocketEnd>>>,
    served: tokio::task::JoinHandle<LinkEnd>,
}

impl TestLink {
    pub fn link(&self) -> &Link {
        &self.link
    }

    /// The next message the backend sent.
    pub async fn next(&mut self) -> Inbound {
        let frame = self.sent.recv().await.expect("the connection is served");
        wire::decode(&frame.into_bytes()).expect("the backend sends valid messages")
    }

    /// The next message the backend sent, when it sent one already.
    pub fn try_next(&mut self) -> Option<Inbound> {
        let frame = self.sent.try_recv().ok()?;
        Some(wire::decode(&frame.into_bytes()).expect("the backend sends valid messages"))
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

/// A command package built in the workspace: its descriptor for this host
/// and where its executable is.
pub struct NativeFixture {
    pub descriptor: PackageDescriptor,
    path: PathBuf,
}

impl NativeFixture {
    /// The runner's native fixture package
    /// (`demi_command_protocol::testing::FIXTURE_OPERATIONS`).
    pub fn load() -> Self {
        Self::package(
            "demicodes.runner-test",
            native_fixture_binary(),
            demi_command_protocol::testing::FIXTURE_OPERATIONS,
        )
    }

    /// The package `id` that serves `operations`, whose executable for this
    /// host is `path`.
    pub fn package<'a>(
        id: &str,
        path: PathBuf,
        operations: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        let bytes = std::fs::read(&path).unwrap_or_else(|error| {
            panic!(
                "read {}: {error}; build it with cargo build --workspace --features demi-runner/test-fixtures",
                path.display()
            )
        });
        let descriptor = PackageDescriptor {
            id: id.into(),
            version: "test".into(),
            protocol_version: 1,
            operations: operations.into_iter().map(String::from).collect(),
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
