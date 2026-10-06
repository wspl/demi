//! The connection engine with the test as the runner.

use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::Rc,
    time::Duration,
};

use bytes::Bytes;
use demi_backend_remote_host::{
    Admission, ArtifactResolver, CommandCatalog, CommandKeeper, EnvironmentOptions, JobOrigin,
    JobStart, LinkEnd, LinkPolicy, RemoteHost, RemoteShellEnvironment,
    testing::{CommandPolicy, TEST_DEVICE, TestDevice, TestLink},
};
use demi_command_declarations::NativeOperation;
use demi_command_protocol::{
    ArtifactLocation, ArtifactUrl, EditCopies, EditKind as FileEditKind, PackageArtifact,
    PackageDescriptor, ServiceSequence, host_target,
};
use demi_host_interface::{
    Call, CommandMedium, CommandSet, CommandState, ExecRequest, GroupBuilder, HostError, HostErrorKind,
    HostProcess, JobCaller, LeafBuilder, ObservationWindow, OutputRecord, PageState, Process,
    ProcessEnd, RpcError, RpcInvocation, RpcPort, Seen, ShellEnvironment, ShellTarget, SpawnEnv,
    SpawnRequest, Streams, TypedRpc, WholeOutput,
    testing::{CountingNumbers, TestPages, test_command_context},
};
use demi_runner_protocol::wire::{
    ArtifactOwner, FsOk, FsResult, Inbound, JOB_VIEW_BYTES, StreamArtifactOwner,
    JobArtifactOwner, JobFileChange, KeptRecord, Outbound, OutputLengths, OutputStream,
    STDIN_CHUNK_BYTES, Signal, VolumeName, WireBytes, encode_record,
};
use demi_shared_gates::{ActivityGate, Purpose};
use demi_shared_types::{
    BlobRef, CommandId, EditKind, EditSegment, EditedFile, NodeId, StreamKind,
};
use futures_util::future::LocalBoxFuture;
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;

fn device() -> TestDevice {
    TestDevice::new(CommandPolicy::new(CommandSet::new()))
}

fn caller() -> JobCaller {
    JobCaller {
        node: NodeId::try_from("test-session").unwrap(),
    }
}

fn job_start(script: &str) -> JobStart {
    JobStart {
        script: script.into(),
        cwd: "/work".into(),
        env: BTreeMap::new(),
        context: test_command_context(),
        caller: Some(caller()),
        commands: None,
        stdin: None,
        stdout: None,
    }
}

fn spawn(command: &str, retained: bool) -> SpawnRequest {
    SpawnRequest {
        command: command.into(),
        args: Vec::new(),
        cwd: None,
        env: SpawnEnv::Inherit,
        retained,
    }
}

fn environment(host: RemoteHost) -> RemoteShellEnvironment {
    watched_environment(host, TestPages::new(false))
}

/// An environment whose commands' pages' views go to `pages`.
fn watched_environment(host: RemoteHost, pages: Rc<TestPages>) -> RemoteShellEnvironment {
    let context: demi_backend_remote_host::ContextSource =
        Rc::new(|| Box::pin(async { Ok(test_command_context()) }));
    RemoteShellEnvironment::new(EnvironmentOptions::new(
        host,
        context,
        pages,
        Rc::new(CountingNumbers::default()),
    ))
}

fn exec(script: &str) -> ExecRequest {
    ExecRequest {
        script: script.into(),
        shell: ShellTarget::Default,
        window: ObservationWindow::from_millis(1).unwrap(),
        caller: caller(),
        tool_use_id: "call".into(),
    }
}

/// Answers the backend's read of `job`'s kept output with `kept`, as a
/// runner streams it through the read's pipe.
async fn answer_read(device: &TestDevice, link: &mut TestLink, job: &str, kept: Vec<u8>) {
    let (id, output) = loop {
        let message = tokio::time::timeout(Duration::from_secs(10), link.next())
            .await
            .expect("a read within the hang guard");
        if let Inbound::JobRead { id, job_id, output } = message
            && job_id == job
        {
            break (id, output);
        }
    };
    link.send(Outbound::JobRead { id, error: None }).await;
    let source = device
        .pipes()
        .claim_source(&output.id, TEST_DEVICE)
        .unwrap();
    let body = futures_util::stream::iter([Ok::<_, std::io::Error>(bytes::Bytes::from(kept))]);
    source.pump(body).await.unwrap();
}

/// Everything the backend sent that the driver passed on by now.
async fn drain(link: &mut TestLink) -> Vec<Inbound> {
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    std::iter::from_fn(|| link.try_next()).collect()
}

fn job_exit(job_id: &str, exit_code: Option<i32>, signal: Option<&str>) -> Outbound {
    Outbound::JobExit {
        job_id: job_id.into(),
        exit_code,
        signal: signal.map(str::to_owned),
        spawn_error: None,
        cwd: None,
        output: None,
        files: Vec::new(),
        files_truncated: false,
    }
}

#[tokio::test(flavor = "local")]
async fn an_fs_reply_is_read_as_the_operation_the_caller_asked_for() {
    let device = device();
    let mut link = device.connect(None);
    let host = device.host("/work", Admission::Free);
    let stat = tokio::task::spawn_local(async move {
        demi_host_interface::Host::fs(&host)
            .stat("/work/file")
            .await
    });
    let Inbound::FsStat { id, .. } = link.next().await else {
        panic!("expected a stat request")
    };
    // A readlink result is well formed for the op the reply names, and is not
    // the stat this caller waits for.
    link.send(Outbound::FsOk(FsOk {
        id,
        result: FsResult::Readlink("/work/elsewhere".into()),
    }))
    .await;
    assert_eq!(
        stat.await.unwrap().unwrap_err().kind,
        HostErrorKind::Protocol
    );
}

#[tokio::test(flavor = "local")]
async fn admission_holds_calls_and_process_lifetimes_and_a_refusal_sends_nothing() {
    let device = device();
    let mut link = device.connect(None);
    let gate = ActivityGate::new();
    let blocked = Rc::new(Cell::new(false));
    let admission = {
        let (gate, blocked) = (gate.clone(), blocked.clone());
        Admission::Leased(Rc::new(move || {
            if blocked.get() {
                return Err(HostError::new(
                    HostErrorKind::Unavailable,
                    "machine transition",
                ));
            }
            gate.try_enter(Purpose::Demand)
                .ok_or_else(|| HostError::new(HostErrorKind::Unavailable, "machine transition"))
        }))
    };
    let host = device.host("/work", admission);
    let exists = {
        let host = host.clone();
        tokio::task::spawn_local(async move {
            demi_host_interface::Host::fs(&host)
                .exists("/work/file")
                .await
        })
    };
    let Inbound::FsExists { id, .. } = link.next().await else {
        panic!("expected an exists request")
    };
    assert_eq!(gate.state().demand, 1);
    link.send(Outbound::FsOk(FsOk {
        id,
        result: FsResult::Exists(true),
    }))
    .await;
    assert!(exists.await.unwrap().unwrap());
    assert_eq!(gate.state().demand, 0);
    let process = host.spawn(spawn("sleep", false)).await.unwrap();
    let job = host.start_job(job_start("sleep 10")).await.unwrap();
    assert_eq!(gate.state().demand, 2);
    drain(&mut link).await;
    blocked.set(true);
    let refused = demi_host_interface::Host::fs(&host)
        .read_file("/work/next")
        .await
        .unwrap_err();
    assert_eq!(refused.message, "machine transition");
    assert!(host.spawn(spawn("true", false)).await.is_err());
    assert!(host.start_job(job_start("true")).await.is_err());
    assert!(
        drain(&mut link).await.is_empty(),
        "a refused operation sends nothing"
    );
    assert_eq!(
        link.close().await,
        LinkEnd::Closed("runner disconnected".into())
    );
    assert!(matches!(process.exit.await, ProcessEnd::Lost(_)));
    assert!(matches!(job.end().await.status, ProcessEnd::Lost(_)));
    assert_eq!(gate.state().demand, 0);
}

#[tokio::test(flavor = "local")]
async fn dropping_a_running_process_kills_it_and_an_ended_one_is_left_alone() {
    let device = device();
    let mut link = device.connect(None);
    let host = device.host("/work", Admission::Free);
    let running = host.spawn(spawn("sleep", false)).await.unwrap();
    let ended = host.spawn(spawn("true", false)).await.unwrap();
    let spawned: Vec<String> = drain(&mut link)
        .await
        .into_iter()
        .filter_map(|message| match message {
            Inbound::Spawn { spawn_id, .. } => Some(spawn_id),
            _ => None,
        })
        .collect();
    let [running_id, ended_id] = &spawned[..] else {
        panic!("expected two spawns, got {spawned:?}")
    };
    link.send(Outbound::SpawnExit {
        spawn_id: ended_id.clone(),
        exit_code: Some(0),
        signal: None,
        spawn_error: None,
    })
    .await;
    let Process { exit, control, .. } = ended;
    assert_eq!(exit.await, ProcessEnd::Exited(0));
    drop(control);
    drop(running);
    assert_eq!(
        drain(&mut link).await,
        [Inbound::SpawnKill {
            spawn_id: running_id.clone(),
            signal: Some(Signal::Kill),
        }]
    );
}

#[tokio::test(flavor = "local")]
async fn a_retained_process_holds_no_admission_and_is_not_counted() {
    let device = device();
    let link = device.connect(None);
    let gate = ActivityGate::new();
    let admission = {
        let gate = gate.clone();
        Admission::Leased(Rc::new(move || {
            gate.try_enter(Purpose::Demand)
                .ok_or_else(|| HostError::new(HostErrorKind::Unavailable, "busy"))
        }))
    };
    let host = device.host("/work", admission);
    let retained = host.spawn(spawn("provider", true)).await.unwrap();
    assert_eq!(gate.state().demand, 0);
    assert_eq!(link.link().running_jobs(), 0);
    let work = host.spawn(spawn("sleep", false)).await.unwrap();
    assert_eq!(gate.state().demand, 1);
    assert_eq!(link.link().running_jobs(), 1);
    link.close().await;
    retained.exit.await;
    work.exit.await;
    assert_eq!(gate.state().demand, 0);
}

#[tokio::test(flavor = "local")]
async fn stdin_travels_in_bounded_frames_in_order_before_its_end() {
    let device = device();
    let mut link = device.connect(None);
    let host = device.host("/work", Admission::Free);
    let process = host.spawn(spawn("cat", false)).await.unwrap();
    let job = host.start_job(job_start("cat")).await.unwrap();
    drain(&mut link).await;
    for size in [0, STDIN_CHUNK_BYTES, STDIN_CHUNK_BYTES + 1] {
        let bytes: Vec<u8> = (0..size).map(|index| (index % 256) as u8).collect();
        process
            .control
            .write_stdin(Bytes::from(bytes.clone()))
            .await
            .unwrap();
        job.write_stdin(Bytes::from(bytes.clone())).await.unwrap();
        let frames = drain(&mut link).await;
        assert_eq!(frames.len(), 2 * size.div_ceil(STDIN_CHUNK_BYTES));
        let (mut spawned, mut jobbed) = (Vec::new(), Vec::new());
        for frame in frames {
            match frame {
                Inbound::SpawnStdin { bytes, .. } => {
                    assert!(bytes.0.len() <= STDIN_CHUNK_BYTES);
                    spawned.extend(bytes.0);
                }
                Inbound::JobStdin { bytes, .. } => {
                    assert!(bytes.0.len() <= STDIN_CHUNK_BYTES);
                    jobbed.extend(bytes.0);
                }
                other => panic!("unexpected {other:?}"),
            }
        }
        assert_eq!(spawned, bytes);
        assert_eq!(jobbed, bytes);
    }
    process.control.close_stdin().await.unwrap();
    job.close_stdin().await.unwrap();
    let frames = drain(&mut link).await;
    assert!(matches!(
        frames[..],
        [Inbound::SpawnStdinEnd { .. }, Inbound::JobStdinEnd { .. }]
    ));
}

#[tokio::test(flavor = "local")]
async fn a_lost_connection_fails_what_it_carried_and_the_next_one_serves_the_same_host() {
    let device = device();
    // A Host made before its runner ever connected learns the runner's
    // account from its hello, and keeps it while the runner is away.
    let host = device.host("/work", Admission::Free);
    assert_eq!(demi_host_interface::Host::identity(&host).hostname, "");
    let mut link = device.connect(None);
    let identity = demi_host_interface::Host::identity(&host);
    assert_eq!((identity.uid, identity.hostname.as_str()), (501, "test"));
    let pending = {
        let host = host.clone();
        tokio::task::spawn_local(async move {
            demi_host_interface::Host::fs(&host)
                .read_file("/work/x")
                .await
        })
    };
    let process = host.spawn(spawn("sleep", false)).await.unwrap();
    let job = host.start_job(job_start("sleep 30")).await.unwrap();
    drain(&mut link).await;
    assert_eq!(
        link.close().await,
        LinkEnd::Closed("runner disconnected".into())
    );
    let failed = pending.await.unwrap().unwrap_err();
    assert_eq!(
        (failed.kind, failed.message.as_str()),
        (HostErrorKind::Offline, "runner disconnected")
    );
    assert_eq!(
        process.exit.await,
        ProcessEnd::Lost("runner disconnected".into())
    );
    assert_eq!(
        job.end().await.status,
        ProcessEnd::Lost("runner disconnected".into())
    );
    assert_eq!(demi_host_interface::Host::identity(&host), identity);
    // Offline, a process and a job end at once, and a call fails.
    assert!(matches!(
        host.spawn(spawn("echo", false)).await.unwrap().exit.await,
        ProcessEnd::Lost(_)
    ));
    assert!(matches!(
        host.start_job(job_start("true"))
            .await
            .unwrap()
            .end()
            .await
            .status,
        ProcessEnd::Lost(_)
    ));
    assert_eq!(
        demi_host_interface::Host::fs(&host)
            .exists("/work")
            .await
            .unwrap_err()
            .kind,
        HostErrorKind::Offline
    );
    // The same Host serves the next connection.
    let mut link = device.connect(None);
    let exists = {
        let host = host.clone();
        tokio::task::spawn_local(async move {
            demi_host_interface::Host::fs(&host).exists("/work").await
        })
    };
    let Inbound::FsExists { id, .. } = link.next().await else {
        panic!("expected an exists request")
    };
    link.send(Outbound::FsOk(FsOk {
        id,
        result: FsResult::Exists(true),
    }))
    .await;
    assert!(exists.await.unwrap().unwrap());
}

struct NoArtifacts;

impl ArtifactResolver for NoArtifacts {
    fn resolve(
        &self,
        _: &PackageArtifact,
        _: &str,
        _: CancellationToken,
    ) -> LocalBoxFuture<'static, Result<ArtifactLocation, String>> {
        Box::pin(async { Err("no native artifacts".into()) })
    }
}

async fn ok(_: Call<Map<String, Value>>, _: RpcPort) -> Result<u8, RpcError> {
    Ok(0)
}

#[tokio::test(flavor = "local")]
async fn jobs_share_the_selected_manifest_only_within_one_connection() {
    let device = device();
    let mut link = device.connect(None);
    let host = device.host("/work", Admission::Free);
    let catalog = CommandCatalog::new(Vec::new(), Rc::new(NoArtifacts)).unwrap();
    let first = catalog.select(&CommandSet::new()).unwrap();
    let mut commands = CommandSet::new();
    commands
        .register(LeafBuilder::rpc("example", "Example command").bind(TypedRpc::new(ok)))
        .unwrap();
    let second = catalog.select(&commands).unwrap();
    let start = |selection: &demi_backend_remote_host::CommandSelection| {
        let host = host.clone();
        let mut start = job_start("true");
        start.commands = Some(selection.clone());
        async move { host.start_job(start).await }
    };
    let kinds = |frames: &[Inbound]| -> Vec<&'static str> {
        frames
            .iter()
            .map(|frame| match frame {
                Inbound::Manifest { .. } => "manifest",
                Inbound::JobStart { .. } => "job_start",
                _ => "other",
            })
            .collect()
    };
    for selection in [&first, &first, &second, &first] {
        start(selection).await.unwrap();
    }
    let frames = drain(&mut link).await;
    assert_eq!(
        kinds(&frames),
        [
            "manifest",
            "job_start",
            "job_start",
            "manifest",
            "job_start",
            "manifest",
            "job_start"
        ]
    );
    let hashes: Vec<_> = frames
        .iter()
        .filter_map(|frame| match frame {
            Inbound::Manifest { manifest } => manifest["hash"].as_str().map(str::to_owned),
            _ => None,
        })
        .collect();
    assert_eq!(hashes, [first.hash(), second.hash(), first.hash()]);
    // A new connection carries its first manifest again.
    link.close().await;
    let mut link = device.connect(None);
    start(&first).await.unwrap();
    assert_eq!(kinds(&drain(&mut link).await), ["manifest", "job_start"]);
    // A manifest that cannot go leaves the selection unknown: the next job
    // sends its own again.
    let mut huge = CommandSet::new();
    huge.register(LeafBuilder::rpc("huge", "x".repeat(5 * 1024 * 1024)).bind(TypedRpc::new(ok)))
        .unwrap();
    let huge = catalog.select(&huge).unwrap();
    assert_eq!(
        start(&huge).await.err().map(|error| error.kind),
        Some(HostErrorKind::TooLarge)
    );
    start(&first).await.unwrap();
    assert_eq!(kinds(&drain(&mut link).await), ["manifest", "job_start"]);
}

/// A resolver that answers from a script of its own.
struct Scripted {
    calls: Cell<usize>,
    waits: bool,
    observed: RefCell<Option<CancellationToken>>,
}

impl ArtifactResolver for Scripted {
    fn resolve(
        &self,
        _: &PackageArtifact,
        _: &str,
        cancel: CancellationToken,
    ) -> LocalBoxFuture<'static, Result<ArtifactLocation, String>> {
        self.calls.set(self.calls.get() + 1);
        *self.observed.borrow_mut() = Some(cancel.clone());
        let waits = self.waits;
        Box::pin(async move {
            if waits {
                cancel.cancelled().await;
                return Err("cancelled".into());
            }
            Ok(ArtifactLocation::Url(ArtifactUrl {
                url: "https://artifacts.example.test/exact".into(),
                expires_at: None,
            }))
        })
    }
}

fn native_catalog(resolver: Rc<Scripted>) -> (CommandCatalog, PackageDescriptor) {
    let descriptor = PackageDescriptor {
        id: "demicodes.fixture".into(),
        version: "1.0.0".into(),
        protocol_version: 1,
        operations: vec!["file.read".into()],
        targets: BTreeMap::from([(
            host_target().to_owned(),
            PackageArtifact {
                sha256: "a".repeat(64),
                size: 1,
            },
        )]),
    };
    (
        CommandCatalog::new(vec![descriptor.clone()], resolver).unwrap(),
        descriptor,
    )
}

fn native_commands() -> CommandSet {
    let mut commands = CommandSet::new();
    commands
        .register(LeafBuilder::native(
            "native",
            "Native",
            NativeOperation {
                package: "demicodes.fixture".into(),
                operation: "file.read".into(),
            },
        ))
        .unwrap();
    commands
}

fn artifact_answer(frame: &Inbound) -> Option<(Option<String>, Option<String>)> {
    match frame {
        Inbound::ArtifactLocation {
            location, error, ..
        } => Some((
            location.as_ref().map(|location| match location {
                ArtifactLocation::Url(url) => url.url.clone(),
                ArtifactLocation::Path(path) => path.path.clone(),
            }),
            error.clone(),
        )),
        _ => None,
    }
}

#[tokio::test(flavor = "local")]
async fn an_artifact_request_needs_the_live_job_and_an_artifact_of_its_manifest() {
    let device = device();
    let mut link = device.connect(None);
    let host = device.host("/work", Admission::Free);
    let resolver = Rc::new(Scripted {
        calls: Cell::new(0),
        waits: false,
        observed: RefCell::new(None),
    });
    let (catalog, descriptor) = native_catalog(resolver.clone());
    let selection = catalog.select(&native_commands()).unwrap();
    let mut start = job_start("native");
    start.commands = Some(selection.clone());
    let job = host.start_job(start).await.unwrap();
    drain(&mut link).await;
    let request = |sha256: String| Outbound::ArtifactResolve {
        id: "request".into(),
        owner: ArtifactOwner::Job(JobArtifactOwner {
            job_id: job.id().into(),
            manifest_hash: selection.hash().into(),
        }),
        sha256,
        target: host_target().into(),
    };
    link.send(request("f".repeat(64))).await;
    let answer = link.next().await;
    assert_eq!(
        artifact_answer(&answer),
        Some((
            None,
            Some("Artifact does not belong to the live work's packages".into())
        ))
    );
    assert_eq!(resolver.calls.get(), 0);
    let sha256 = descriptor.targets[host_target()].sha256.clone();
    link.send(request(sha256.clone())).await;
    assert_eq!(
        artifact_answer(&link.next().await),
        Some((Some("https://artifacts.example.test/exact".into()), None))
    );
    assert_eq!(resolver.calls.get(), 1);
    link.send(job_exit(job.id(), Some(0), None)).await;
    link.send(request(sha256)).await;
    assert_eq!(
        artifact_answer(&link.next().await),
        Some((None, Some("No matching live job or stream".into())))
    );
    assert_eq!(resolver.calls.get(), 1);
}

/// A policy that admits each direct stream once the test says so, with a
/// demand lease of `gate` and, for its one user stream `live`, the native
/// catalog's package; a stream of another name installs nothing.
struct AdmitsDirectStreams {
    gate: ActivityGate,
    package: PackageDescriptor,
    resolver: Rc<Scripted>,
    admit: tokio::sync::watch::Receiver<bool>,
}

impl LinkPolicy for AdmitsDirectStreams {
    fn admit_call(&self, _: &JobOrigin) -> Result<(), String> {
        Err("no calls".into())
    }

    fn dispatch(
        &self,
        _: Rc<JobOrigin>,
        _: RpcInvocation,
        _: RpcPort,
    ) -> LocalBoxFuture<'static, Result<u8, RpcError>> {
        panic!("no call reaches a handler")
    }

    fn read_blob(&self, _: BlobRef) -> LocalBoxFuture<'static, Result<Option<Bytes>, String>> {
        Box::pin(async { Ok(None) })
    }

    fn grow_volume(&self, _: VolumeName, _: u64) -> LocalBoxFuture<'static, Result<(), String>> {
        Box::pin(async { Err("no".into()) })
    }

    fn revoke_device(&self) -> LocalBoxFuture<'static, Result<(), String>> {
        Box::pin(async { Err("no".into()) })
    }

    fn reserve_numbers(
        &self,
        _: String,
        _: ServiceSequence,
        _: u32,
    ) -> LocalBoxFuture<'static, Result<u64, String>> {
        Box::pin(async { Err("no".into()) })
    }

    fn direct_stream(
        &self,
        _: String,
        name: String,
    ) -> LocalBoxFuture<'static, Result<demi_backend_remote_host::DirectAdmission, String>> {
        let gate = self.gate.clone();
        let package = self.package.clone();
        let resolver: Rc<dyn ArtifactResolver> = self.resolver.clone();
        let mut admit = self.admit.clone();
        Box::pin(async move {
            admit.wait_for(|admit| *admit).await.unwrap();
            Ok(demi_backend_remote_host::DirectAdmission {
                lease: gate.enter(Purpose::Demand).await,
                package: (name == "live").then_some(package),
                resolver,
            })
        })
    }
}

/// A direct stream the runner reports is known as a stream the backend
/// opened (`direct-channel.md` § Operations on the channel): its artifact
/// request, made before the policy admitted it, is answered once it is,
/// while it holds its conversation active; once it closed, the lease is
/// gone and a request naming it is refused. A stream of another name may
/// not install that package.
#[tokio::test(flavor = "local")]
async fn a_direct_streams_artifact_requests_are_answered_once_admitted_and_refused_once_closed() {
    let resolver = Rc::new(Scripted {
        calls: Cell::new(0),
        waits: false,
        observed: RefCell::new(None),
    });
    let (_, package) = native_catalog(resolver.clone());
    let (admit, admitting) = tokio::sync::watch::channel(false);
    let gate = ActivityGate::new();
    let device = TestDevice::new(Rc::new(AdmitsDirectStreams {
        gate: gate.clone(),
        package: package.clone(),
        resolver: resolver.clone(),
        admit: admitting,
    }));
    let mut link = device.connect(None);
    let stream = |id: &str, name: &str, open| Outbound::DirectStream {
        stream: id.into(),
        name: name.into(),
        conversation: "c1".into(),
        open,
    };
    let request = |id: &str| Outbound::ArtifactResolve {
        id: "request".into(),
        owner: ArtifactOwner::Stream(StreamArtifactOwner {
            stream_id: id.into(),
        }),
        sha256: package.targets[host_target()].sha256.clone(),
        target: host_target().into(),
    };
    link.send(stream("direct-1", "live", true)).await;
    link.send(request("direct-1")).await;
    assert!(drain(&mut link).await.is_empty(), "the request waits for the admission");
    admit.send_replace(true);
    assert_eq!(
        artifact_answer(&link.next().await),
        Some((Some("https://artifacts.example.test/exact".into()), None))
    );
    assert_eq!(gate.state().demand, 1, "the open stream is its conversation's activity");

    link.send(stream("direct-1", "live", false)).await;
    link.send(request("direct-1")).await;
    let refused = Some((None, Some("No matching live job or stream".into())));
    assert_eq!(artifact_answer(&link.next().await), refused);
    assert_eq!(gate.state().demand, 0);

    link.send(stream("direct-2", "other", true)).await;
    link.send(request("direct-2")).await;
    assert_eq!(
        artifact_answer(&link.next().await),
        Some((
            None,
            Some("Artifact does not belong to the live work's packages".into())
        ))
    );
    assert_eq!(resolver.calls.get(), 1);
}

#[tokio::test(flavor = "local")]
async fn a_job_that_ends_cancels_its_pending_artifact_requests_without_an_answer() {
    let device = device();
    let mut link = device.connect(None);
    let host = device.host("/work", Admission::Free);
    let resolver = Rc::new(Scripted {
        calls: Cell::new(0),
        waits: true,
        observed: RefCell::new(None),
    });
    let (catalog, descriptor) = native_catalog(resolver.clone());
    let selection = catalog.select(&native_commands()).unwrap();
    let mut start = job_start("native");
    start.commands = Some(selection.clone());
    let job = host.start_job(start).await.unwrap();
    drain(&mut link).await;
    let request = |id: &str| Outbound::ArtifactResolve {
        id: id.into(),
        owner: ArtifactOwner::Job(JobArtifactOwner {
            job_id: job.id().into(),
            manifest_hash: selection.hash().into(),
        }),
        sha256: descriptor.targets[host_target()].sha256.clone(),
        target: host_target().into(),
    };
    link.send(request("request")).await;
    drain(&mut link).await;
    // A request under an id still in flight is refused.
    link.send(request("request")).await;
    assert_eq!(
        artifact_answer(&link.next().await),
        Some((
            None,
            Some("Artifact resolution request limit or duplicate id".into())
        ))
    );
    let observed = resolver.observed.borrow().clone().unwrap();
    assert!(!observed.is_cancelled());
    link.send(job_exit(job.id(), Some(0), None)).await;
    assert!(drain(&mut link).await.is_empty(), "no stale answer");
    assert!(observed.is_cancelled());
}

#[tokio::test(flavor = "local")]
async fn a_status_shows_the_latest_first_registered_hint_and_none_once_the_job_ends() {
    let device = device();
    let mut link = device.connect(None);
    let host = device.host("/work", Admission::Free);
    let shell = environment(host.clone());
    let started = shell
        .exec(
            exec("attend first | attend second"),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    let Inbound::JobStart { job_id, .. } = link.next().await else {
        panic!("expected a job")
    };
    let hint = |invocation: &str, hint: Option<&str>, job: &str| Outbound::JobRunningHint {
        job_id: job.into(),
        invocation_id: invocation.into(),
        hint: hint.map(str::to_owned),
    };
    let shown =
        |shell: &RemoteShellEnvironment| match shell.status(&started.command_id).unwrap().state {
            CommandState::Running { hint } => hint,
            other => panic!("expected a running command, got {other:?}"),
        };
    link.send(hint("foreign", Some("another job"), "not-this-job"))
        .await;
    drain(&mut link).await;
    assert_eq!(shown(&shell), None);
    link.send(hint("first", Some("first hint"), &job_id)).await;
    link.send(hint("second", Some("second hint"), &job_id))
        .await;
    drain(&mut link).await;
    assert_eq!(shown(&shell).as_deref(), Some("second hint"));
    link.send(hint("second", None, &job_id)).await;
    drain(&mut link).await;
    assert_eq!(shown(&shell).as_deref(), Some("first hint"));
    link.send(hint("first", None, &job_id)).await;
    link.send(hint("third", Some("hint before abort"), &job_id))
        .await;
    drain(&mut link).await;
    let aborting = {
        let shell = shell.clone();
        let command = started.command_id.clone();
        tokio::task::spawn_local(async move { shell.abort(&command).await })
    };
    let Inbound::JobKill { signal, .. } = link.next().await else {
        panic!("expected the job to be stopped")
    };
    assert_eq!(signal, Some(demi_runner_protocol::wire::Signal::Terminate));
    link.send(job_exit(&job_id, None, Some("SIGTERM"))).await;
    aborting.await.unwrap().unwrap();
    assert!(matches!(
        shell.status(&started.command_id).unwrap().state,
        CommandState::Aborted
    ));
    link.send(hint("late", Some("arrived after the end"), &job_id))
        .await;
    drain(&mut link).await;
    assert!(matches!(
        shell.status(&started.command_id).unwrap().state,
        CommandState::Aborted
    ));

    // A lost connection ends a job and its hint with it.
    let job = host.start_job(job_start("attend")).await.unwrap();
    drain(&mut link).await;
    link.send(hint("i1", Some("attending"), job.id())).await;
    drain(&mut link).await;
    assert_eq!(job.running_hint().as_deref(), Some("attending"));
    link.close().await;
    assert!(matches!(job.end().await.status, ProcessEnd::Lost(_)));
    assert_eq!(job.running_hint(), None);
}

/// While a page watches, a command's job is followed, and the pages' view
/// holds its output in the order it came: each stream's first bytes, the
/// newest bytes beyond them with a note where the runner left some out, and
/// at the end the rest of each stream from its tail; the model's view holds
/// only the first bytes (`runtime.md` § Live output).
#[tokio::test(flavor = "local")]
async fn a_watched_command_is_followed_and_its_pages_view_holds_what_the_runner_sent() {
    let device = device();
    let mut link = device.connect(None);
    let pages = TestPages::new(false);
    let shell = watched_environment(device.host("/work", Admission::Free), pages.clone());
    let started = shell
        .exec(exec("build"), CancellationToken::new())
        .await
        .unwrap();
    // The pages learn of the command before its first output.
    let view = pages.next().await;
    assert_eq!(view.command_id, started.command_id);
    assert_eq!(
        (
            view.tool_use_id.as_str(),
            view.state,
            view.tail.as_str(),
            view.chars
        ),
        ("call", PageState::Running, "", 0)
    );
    let Inbound::JobStart { job_id, .. } = link.next().await else {
        panic!("expected a job")
    };
    // What the backend sends the runner next, within a hang guard.
    let next = async |link: &mut TestLink| {
        tokio::time::timeout(Duration::from_secs(10), link.next())
            .await
            .expect("a message within the hang guard")
    };

    // A job starts unfollowed; a page that comes makes it followed.
    pages.watch(true);
    assert!(matches!(
        next(&mut link).await,
        Inbound::JobFollow { follow: true, .. }
    ));
    let output = |offset: u64, bytes: &[u8]| Outbound::JobOutput {
        job_id: job_id.clone(),
        stream: OutputStream::Stdout,
        offset,
        bytes: WireBytes(bytes.to_vec()),
    };
    let view_end = JOB_VIEW_BYTES as u64;
    link.send(output(0, b"first\n")).await;
    assert_eq!(pages.next().await.tail, "first\n");
    link.send(output(6, &vec![b'x'; JOB_VIEW_BYTES - 6])).await;
    pages.next().await;
    // The runner left ten bytes out before its newest ones.
    link.send(output(view_end + 10, b"newest\n")).await;
    let view = pages.next().await;
    let note = "\n[... 10 bytes of stdout not shown ...]\n";
    assert!(
        view.tail.ends_with(&format!("x{note}newest\n")),
        "{:?}",
        &view.tail[view.tail.len() - 60..]
    );
    assert_eq!(view.chars, view_end + note.len() as u64 + 7);
    // The model's view holds only the stream's first bytes.
    let status = shell.status(&started.command_id).unwrap();
    assert!(status.stdout.tail.ends_with('x'));
    assert_eq!(
        (status.stdout.bytes, status.unreceived),
        (view_end + 17, 17)
    );

    // A page that leaves makes the job unfollowed.
    pages.watch(false);
    assert!(matches!(
        next(&mut link).await,
        Inbound::JobFollow { follow: false, .. }
    ));

    // The stream went beyond what the backend received: the end reads the
    // Host's kept output once, shows its end anew, and lets the job go.
    let mut stream = b"first\n".to_vec();
    stream.extend(vec![b'x'; JOB_VIEW_BYTES - 6]);
    stream.extend(b"yyyyyyyyyynewest\nlast\n");
    link.send(Outbound::JobExit {
        job_id: job_id.clone(),
        exit_code: Some(0),
        signal: None,
        spawn_error: None,
        cwd: None,
        output: Some(OutputLengths {
            stdout_bytes: stream.len() as u64,
            stderr_bytes: 0,
        }),
        files: Vec::new(),
        files_truncated: false,
    })
    .await;
    let record = KeptRecord::Output(OutputStream::Stdout, WireBytes(stream.clone()));
    answer_read(&device, &mut link, &job_id, encode_record(&record).unwrap()).await;
    let end = pages.next().await;
    assert_eq!(end.state, PageState::Exited { exit_code: 0 });
    assert!(end.tail.ends_with("newest\nlast\n"), "{:?}", end.tail);
    let status = shell.status(&started.command_id).unwrap();
    let whole = status.whole.expect("the whole output");
    assert_eq!(
        whole.output.records(),
        [OutputRecord::Output(StreamKind::Stdout, stream.into())]
    );
    assert!(
        drain(&mut link)
            .await
            .iter()
            .any(|message| matches!(message, Inbound::JobRelease { job_id: released } if *released == job_id)),
        "the job is released once its output is read"
    );
    assert_eq!(pages.drain(), []);

    // A command that starts while a page watches is followed at once.
    pages.watch(true);
    shell
        .exec(exec("again"), CancellationToken::new())
        .await
        .unwrap();
    assert!(matches!(next(&mut link).await, Inbound::JobStart { .. }));
    assert!(matches!(
        next(&mut link).await,
        Inbound::JobFollow { follow: true, .. }
    ));
}

/// A character the runner's messages split shows once, whole, in both
/// views, and a byte that is not text shows as U+FFFD (`runtime.md` § Live
/// output).
#[tokio::test(flavor = "local")]
async fn a_character_split_across_messages_shows_once_whole() {
    let device = device();
    let mut link = device.connect(None);
    let pages = TestPages::new(false);
    let shell = watched_environment(device.host("/work", Admission::Free), pages.clone());
    let started = shell
        .exec(exec("print"), CancellationToken::new())
        .await
        .unwrap();
    pages.next().await;
    let Inbound::JobStart { job_id, .. } = link.next().await else {
        panic!("expected a job")
    };
    let text = "aé€😀".as_bytes();
    for (offset, byte) in text.iter().enumerate() {
        link.send(Outbound::JobOutput {
            job_id: job_id.clone(),
            stream: OutputStream::Stdout,
            offset: offset as u64,
            bytes: WireBytes(vec![*byte]),
        })
        .await;
    }
    link.send(Outbound::JobOutput {
        job_id: job_id.clone(),
        stream: OutputStream::Stdout,
        offset: text.len() as u64,
        bytes: WireBytes(b"\xff!".to_vec()),
    })
    .await;
    let shown = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let view = pages.next().await;
            if view.tail.ends_with('!') {
                break view.tail;
            }
        }
    })
    .await
    .expect("the pages see the output");
    assert_eq!(shown, "aé€😀\u{fffd}!");
    let status = shell.status(&started.command_id).unwrap();
    assert_eq!(status.stdout.tail, "aé€😀\u{fffd}!");
}

/// The model's idle time counts from the latest growth of a command's
/// output, also beyond the runner's view of a stream, which the model's view
/// does not hold (`runtime.md` § Results and previews).
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_stream_that_grows_beyond_its_view_keeps_its_command_from_idling() {
    let device = device();
    let mut link = device.connect(None);
    let shell = environment(device.host("/work", Admission::Free));
    let started = shell
        .exec(exec("build"), CancellationToken::new())
        .await
        .unwrap();
    let Inbound::JobStart { job_id, .. } = link.next().await else {
        panic!("expected a job")
    };
    let output = |offset: u64, bytes: Vec<u8>| Outbound::JobOutput {
        job_id: job_id.clone(),
        stream: OutputStream::Stdout,
        offset,
        bytes: WireBytes(bytes),
    };
    link.send(output(0, vec![b'x'; JOB_VIEW_BYTES])).await;
    drain(&mut link).await;
    tokio::time::advance(Duration::from_secs(5)).await;
    let status = shell.status(&started.command_id).unwrap();
    assert_eq!(status.idle_ms, 5000);

    // The runner says the stream grew beyond its view: the command is not
    // idle, and the model's view still holds the stream's first bytes.
    link.send(output(JOB_VIEW_BYTES as u64 + 100, Vec::new()))
        .await;
    drain(&mut link).await;
    let status = shell.status(&started.command_id).unwrap();
    assert_eq!(status.idle_ms, 0);
    assert_eq!(status.stdout.tail.len(), 4096);
    assert_eq!(
        (status.stdout.bytes, status.unreceived),
        (JOB_VIEW_BYTES as u64 + 100, 100)
    );
}

/// While nobody follows a job, the runner sends each stream's newest bytes
/// beyond its view, again with every message; the model's view of the
/// running command holds the newest `JOB_VIEW_BYTES`, which continue,
/// overlap or replace the ones before (`runner.md` § Pipes and output).
#[tokio::test(flavor = "local")]
async fn a_running_command_holds_each_streams_newest_bytes_beyond_its_view() {
    let device = device();
    let mut link = device.connect(None);
    let shell = environment(device.host("/work", Admission::Free));
    let started = shell
        .exec(exec("build"), CancellationToken::new())
        .await
        .unwrap();
    let Inbound::JobStart { job_id, .. } = link.next().await else {
        panic!("expected a job")
    };
    let output = |offset: u64, bytes: &[u8]| Outbound::JobOutput {
        job_id: job_id.clone(),
        stream: OutputStream::Stdout,
        offset,
        bytes: WireBytes(bytes.to_vec()),
    };
    let view = JOB_VIEW_BYTES as u64;
    let newest = |shell: &RemoteShellEnvironment| {
        let status = shell.status(&started.command_id).unwrap();
        let [newest] = status.newest.as_slice() else {
            panic!("{:?}", status.newest)
        };
        (newest.offset, newest.left_out, newest.text.clone())
    };
    link.send(output(0, &vec![b'x'; JOB_VIEW_BYTES])).await;
    link.send(output(view + 100, b"one\ntw")).await;
    drain(&mut link).await;
    assert_eq!(newest(&shell), (view + 100, 100, "one\ntw".into()));

    // The next message repeats what the last one held and continues it.
    link.send(output(view + 104, b"two\nthree\n")).await;
    drain(&mut link).await;
    assert_eq!(
        newest(&shell),
        (view + 100, 100, "one\ntwo\nthree\n".into())
    );

    // One beyond them replaces them, and the newest `JOB_VIEW_BYTES` stay.
    let far = view * 10;
    let mut bytes = vec![b'y'; JOB_VIEW_BYTES];
    bytes.extend_from_slice(b"end\n");
    link.send(output(far, &bytes)).await;
    drain(&mut link).await;
    let (offset, left_out, text) = newest(&shell);
    assert_eq!((offset, left_out), (far + 4, far + 4 - view));
    assert_eq!(text.len(), JOB_VIEW_BYTES);
    assert!(text.ends_with("yend\n"));
}

#[tokio::test(flavor = "local")]
async fn a_job_the_runner_could_not_run_ends_127_with_the_runners_reason() {
    let device = device();
    let mut link = device.connect(None);
    let shell = environment(device.host("/work", Admission::Free));
    let started = shell
        .exec(exec("true"), CancellationToken::new())
        .await
        .unwrap();
    let Inbound::JobStart { job_id, .. } = link.next().await else {
        panic!("expected a job")
    };
    link.send(Outbound::JobExit {
        job_id,
        exit_code: None,
        signal: None,
        spawn_error: Some(demi_runner_protocol::wire::SpawnError {
            kind: demi_runner_protocol::wire::SpawnErrorKind::Other,
            detail: Some("the job was cancelled before it started".into()),
        }),
        cwd: None,
        output: None,
        files: Vec::new(),
        files_truncated: false,
    })
    .await;
    let mut status = shell.status(&started.command_id).unwrap();
    for _ in 0..100 {
        if !matches!(status.state, CommandState::Running { .. }) {
            break;
        }
        tokio::task::yield_now().await;
        status = shell.status(&started.command_id).unwrap();
    }
    assert!(
        matches!(status.state, CommandState::Exited { exit_code: 127, .. }),
        "{:?}",
        status.state
    );
    let whole = status.whole.expect("the whole output");
    let stderr = whole
        .output
        .text(Streams::Only(StreamKind::Stderr), None, Seen::default());
    assert_eq!(stderr.bytes(), b"the job was cancelled before it started\n");
}

#[tokio::test(flavor = "local")]
async fn a_conversation_release_waits_for_the_runner_and_admits_no_work() {
    let device = device();
    let refusing = Admission::Leased(Rc::new(|| panic!("a release must not admit work")));
    let host = device.host("/work", refusing);
    // Without a runner there is nothing to release.
    host.release_conversation("conversation").await.unwrap();
    let mut link = device.connect(None);
    let release = |host: &RemoteHost| {
        let host = host.clone();
        tokio::task::spawn_local(async move { host.release_conversation("conversation").await })
    };
    let released = release(&host);
    let Inbound::ConversationRelease {
        id,
        conversation_id,
    } = link.next().await
    else {
        panic!("expected a release")
    };
    assert_eq!(conversation_id, "conversation");
    assert!(!released.is_finished());
    link.send(Outbound::ConversationReleased { id, error: None })
        .await;
    released.await.unwrap().unwrap();
    for error in ["cleanup failed", ""] {
        let failed = release(&host);
        let Inbound::ConversationRelease { id, .. } = link.next().await else {
            panic!("expected a release")
        };
        link.send(Outbound::ConversationReleased {
            id,
            error: Some(error.into()),
        })
        .await;
        assert_eq!(failed.await.unwrap().unwrap_err().message, error);
    }
    let lost = release(&host);
    link.next().await;
    link.close().await;
    assert_eq!(
        lost.await.unwrap().unwrap_err().kind,
        HostErrorKind::Offline
    );
}

/// Publishes a job's edits once the test lets it.
struct Publisher {
    barrier: RefCell<Option<tokio::sync::oneshot::Receiver<()>>>,
    file: EditedFile,
}

impl CommandKeeper for Publisher {
    fn retain<'a>(
        &'a self,
        _: &'a CommandId,
        _: &'a [JobFileChange],
    ) -> LocalBoxFuture<'a, Vec<EditedFile>> {
        let barrier = self.barrier.borrow_mut().take();
        Box::pin(async move {
            if let Some(barrier) = barrier {
                let _ = barrier.await;
            }
            vec![self.file.clone()]
        })
    }

    fn stored_blob<'a>(&'a self, _: &'a BlobRef) -> LocalBoxFuture<'a, Option<Bytes>> {
        Box::pin(async { None })
    }

    fn keep_output<'a>(
        &'a self,
        _: &'a CommandId,
        _: &'a WholeOutput,
        _: &'a [CommandMedium],
    ) -> LocalBoxFuture<'a, ()> {
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "local")]
async fn a_command_ends_once_its_edits_are_published_and_keeps_them() {
    let device = device();
    let mut link = device.connect(None);
    let host = device.host("/work", Admission::Free);
    let file = EditedFile {
        path: "/work/file".into(),
        kind: EditKind::Modified,
        added: 1,
        removed: 1,
        edits: vec![EditSegment {
            copies: Some(demi_shared_types::EditCopies {
                original: BlobRef::of(b"before\n"),
                modified: BlobRef::of(b"after\n"),
            }),
        }],
    };
    let (publish, barrier) = tokio::sync::oneshot::channel();
    let context: demi_backend_remote_host::ContextSource =
        Rc::new(|| Box::pin(async { Ok(test_command_context()) }));
    let mut options = EnvironmentOptions::new(
        host,
        context,
        TestPages::new(false),
        Rc::new(CountingNumbers::default()),
    );
    options.keeper = Some(Rc::new(Publisher {
        barrier: RefCell::new(Some(barrier)),
        file: file.clone(),
    }));
    let shell = RemoteShellEnvironment::new(options);
    let started = shell
        .exec(exec("echo new > file"), CancellationToken::new())
        .await
        .unwrap();
    let Inbound::JobStart { job_id, .. } = link.next().await else {
        panic!("expected a job")
    };
    link.send(Outbound::JobExit {
        job_id,
        exit_code: Some(7),
        signal: None,
        spawn_error: None,
        cwd: None,
        output: None,
        files: vec![JobFileChange {
            path: "/work/file".into(),
            kind: FileEditKind::Modified,
            edits: vec![EditCopies {
                original: Some("/copies/before".into()),
                modified: Some("/copies/after".into()),
            }],
            added: 1,
            removed: 1,
        }],
        files_truncated: true,
    })
    .await;
    drain(&mut link).await;
    assert!(matches!(
        shell.status(&started.command_id).unwrap().state,
        CommandState::Running { .. }
    ));
    publish.send(()).unwrap();
    let mut status = shell.status(&started.command_id).unwrap();
    for _ in 0..100 {
        if !matches!(status.state, CommandState::Running { .. }) {
            break;
        }
        tokio::task::yield_now().await;
        status = shell.status(&started.command_id).unwrap();
    }
    assert!(matches!(
        status.state,
        CommandState::Exited { exit_code: 7, .. }
    ));
    let files = status.files.unwrap();
    assert_eq!((files.files, files.truncated), (vec![file.clone()], true));
    assert_eq!(
        shell
            .status(&started.command_id)
            .unwrap()
            .files
            .unwrap()
            .files,
        vec![file]
    );
}

/// A handler that follows the call's live input to its end, then its stop.
fn live_commands(ended: Rc<Cell<bool>>, context: Rc<RefCell<Option<Value>>>) -> CommandSet {
    let mut commands = CommandSet::new();
    let handler = TypedRpc::new(move |call: Call<Map<String, Value>>, port: RpcPort| {
        let ended = ended.clone();
        let context = context.clone();
        async move {
            *context.borrow_mut() = Some(serde_json::json!({
                "conversation": call.invocation.context.conversation,
                "caller": call.invocation.caller,
            }));
            while let Ok(Some(_)) = port.read_live_stdin().await {}
            port.cancelled().await;
            ended.set(true);
            Ok(0)
        }
    });
    commands
        .register(
            GroupBuilder::new("probe", "Probes.").index_entry("Runs probes.")
                .leaf(LeafBuilder::rpc("live", "Live input.").bind(handler)),
        )
        .unwrap();
    commands
}

fn rpc_call(job_id: &str, call_id: &str) -> Outbound {
    Outbound::RpcCall {
        job_id: job_id.into(),
        call_id: call_id.into(),
        root: "probe".into(),
        path: vec!["probe".into(), "live".into()],
        argv: vec!["live".into()],
        args: Map::new(),
        json: false,
        cwd: "/work".into(),
        env: BTreeMap::new(),
        stdin: true,
    }
}

/// What a call answered: its standard error and exit code.
async fn call_outcome(link: &mut TestLink) -> (String, u8) {
    let mut stderr = String::new();
    loop {
        match link.next().await {
            Inbound::RpcOutput { bytes, .. } => stderr.push_str(&String::from_utf8_lossy(&bytes.0)),
            Inbound::RpcExit { exit_code, .. } => return (stderr, exit_code),
            _ => {}
        }
    }
}

#[tokio::test(flavor = "local")]
async fn a_call_stops_on_its_first_cause_releases_its_live_input_and_exits_after_it() {
    for event in [
        "cancel",
        "job",
        "report",
        "stdout",
        "stdin",
        "chunk",
        "unasked",
        "disconnect",
        "shutdown",
    ] {
        let ended = Rc::new(Cell::new(false));
        let context = Rc::new(RefCell::new(None));
        let device = TestDevice::new(CommandPolicy::new(live_commands(
            ended.clone(),
            context.clone(),
        )));
        let mut link = device.connect(None);
        let host = device.host("/work", Admission::Free);
        let job = host.start_job(job_start("probe live")).await.unwrap();
        drain(&mut link).await;
        link.send(rpc_call(job.id(), "call")).await;
        let Inbound::RpcPipes { stdin, stdout, .. } = link.next().await else {
            panic!("the call's pipes come first")
        };
        let stdin = stdin.expect("the process has a pipe on its stdin");
        assert!(matches!(link.next().await, Inbound::RpcStdinPull { .. }));
        let expected = match event {
            "cancel" => {
                link.send(Outbound::RpcCancel {
                    call_id: "call".into(),
                })
                .await;
                "command cancelled".to_owned()
            }
            "job" => {
                link.send(job_exit(job.id(), Some(7), None)).await;
                format!("calling job {} exited before its RPC completed", job.id())
            }
            "report" => {
                link.send(Outbound::PipeDone {
                    pipe_id: stdout.id.clone(),
                    ok: false,
                    error: Some("upload failed".into()),
                })
                .await;
                "pipe failed: upload failed".into()
            }
            "stdout" | "stdin" => {
                let pipe = if event == "stdout" {
                    &stdout.id
                } else {
                    &stdin.id
                };
                device.pipes().fail(pipe, "HTTP connection lost");
                "pipe failed: HTTP connection lost".into()
            }
            "chunk" => {
                link.send(Outbound::RpcStdin {
                    call_id: "call".into(),
                    bytes: WireBytes(vec![0; STDIN_CHUNK_BYTES + 1]),
                })
                .await;
                "Unrequested or oversized RPC stdin chunk".into()
            }
            "unasked" => {
                // The first answers the read; the second arrives before the
                // handler asks again.
                for _ in 0..2 {
                    link.send(Outbound::RpcStdin {
                        call_id: "call".into(),
                        bytes: WireBytes(b"typed".to_vec()),
                    })
                    .await;
                }
                "Unrequested or oversized RPC stdin chunk".into()
            }
            "disconnect" => {
                link.close().await;
                for _ in 0..20 {
                    tokio::task::yield_now().await;
                }
                assert!(ended.get(), "{event}: the handler ends with the connection");
                continue;
            }
            _ => {
                link.link().disconnect("backend shutting down");
                assert_eq!(
                    link.ended().await,
                    LinkEnd::Disconnected("backend shutting down".into())
                );
                assert!(ended.get(), "{event}: the handler ends with the connection");
                continue;
            }
        };
        // A later end of the job does not replace the first cause.
        link.send(job_exit(job.id(), Some(7), None)).await;
        let (stderr, exit_code) = call_outcome(&mut link).await;
        assert!(ended.get(), "{event}: the handler was released");
        assert_eq!(stderr, format!("probe: {expected}\n"), "{event}");
        assert_eq!(
            exit_code,
            if event == "cancel" { 130 } else { 1 },
            "{event}"
        );
        assert_eq!(
            context.borrow().as_ref().unwrap()["conversation"],
            test_command_context().conversation.as_str()
        );
        // The handler knows the agent node the invoking job runs for.
        assert_eq!(
            context.borrow().as_ref().unwrap()["caller"],
            serde_json::to_value(caller()).unwrap()
        );
    }
}

#[tokio::test(flavor = "local")]
async fn a_call_exits_after_its_standard_output_drained() {
    let mut commands = CommandSet::new();
    let handler = TypedRpc::new(|_: Call<Map<String, Value>>, port: RpcPort| async move {
        port.stdout("hello").await?;
        Ok(3)
    });
    commands
        .register(
            GroupBuilder::new("probe", "Probes.").index_entry("Runs probes.")
                .leaf(LeafBuilder::rpc("live", "Speaks.").bind(handler)),
        )
        .unwrap();
    let device = TestDevice::new(CommandPolicy::new(commands));
    let mut link = device.connect(None);
    let job = device
        .host("/work", Admission::Free)
        .start_job(job_start("probe live"))
        .await
        .unwrap();
    drain(&mut link).await;
    let mut call = rpc_call(job.id(), "call");
    if let Outbound::RpcCall { stdin, .. } = &mut call {
        *stdin = false;
    }
    link.send(call).await;
    let Inbound::RpcPipes {
        stdin: None,
        stdout,
        ..
    } = link.next().await
    else {
        panic!("a process without a stdin pipe gets only stdout")
    };
    assert!(
        drain(&mut link).await.is_empty(),
        "the exit waits for the drain"
    );
    let mut sink = device.pipes().claim_sink(&stdout.id, TEST_DEVICE).unwrap();
    sink.source_arrived().await.unwrap();
    let mut body = sink.into_stream();
    let mut read = Vec::new();
    while let Some(chunk) = futures_util::StreamExt::next(&mut body).await {
        read.extend_from_slice(&chunk.unwrap());
    }
    assert_eq!(read, b"hello");
    assert_eq!(call_outcome(&mut link).await, (String::new(), 3));
}

// A call refused before its handler runs, as the permission check refuses
// one, fails at once: the process it was handed the stdout pipe for still
// finds the pipe, ended, and reads the refusal on stderr.
#[tokio::test(flavor = "local")]
async fn a_call_that_fails_at_once_keeps_its_standard_output_until_drained() {
    let mut commands = CommandSet::new();
    let handler = TypedRpc::new(|_: Call<Map<String, Value>>, _: RpcPort| async move {
        Err::<u8, _>(RpcError::Failed("refused".into()))
    });
    commands
        .register(
            GroupBuilder::new("probe", "Probes.").index_entry("Runs probes.")
                .leaf(LeafBuilder::rpc("live", "Refuses.").bind(handler)),
        )
        .unwrap();
    let device = TestDevice::new(CommandPolicy::new(commands));
    let mut link = device.connect(None);
    let job = device
        .host("/work", Admission::Free)
        .start_job(job_start("probe live"))
        .await
        .unwrap();
    drain(&mut link).await;
    let mut call = rpc_call(job.id(), "call");
    if let Outbound::RpcCall { stdin, .. } = &mut call {
        *stdin = false;
    }
    link.send(call).await;
    let Inbound::RpcPipes { stdout, .. } = link.next().await else {
        panic!("the call is handed its stdout pipe")
    };
    assert!(
        drain(&mut link).await.is_empty(),
        "the exit waits for the drain"
    );
    let mut sink = device.pipes().claim_sink(&stdout.id, TEST_DEVICE).unwrap();
    sink.source_arrived().await.unwrap();
    let mut body = sink.into_stream();
    assert!(futures_util::StreamExt::next(&mut body).await.is_none());
    assert_eq!(
        call_outcome(&mut link).await,
        ("probe: refused\n".to_owned(), 1)
    );
}

/// A policy that refuses every call.
struct Refusing;

impl LinkPolicy for Refusing {
    fn admit_call(&self, _: &JobOrigin) -> Result<(), String> {
        Err("rpc job belongs to another conversation".into())
    }

    fn dispatch(
        &self,
        _: Rc<JobOrigin>,
        _: RpcInvocation,
        _: RpcPort,
    ) -> LocalBoxFuture<'static, Result<u8, RpcError>> {
        panic!("a refused call reaches no handler")
    }

    fn read_blob(&self, _: BlobRef) -> LocalBoxFuture<'static, Result<Option<Bytes>, String>> {
        Box::pin(async { Ok(None) })
    }

    fn grow_volume(&self, _: VolumeName, _: u64) -> LocalBoxFuture<'static, Result<(), String>> {
        Box::pin(async { Err("no".into()) })
    }

    fn revoke_device(&self) -> LocalBoxFuture<'static, Result<(), String>> {
        Box::pin(async { Err("no".into()) })
    }

    fn reserve_numbers(
        &self,
        _: String,
        _: ServiceSequence,
        _: u32,
    ) -> LocalBoxFuture<'static, Result<u64, String>> {
        Box::pin(async { Err("no".into()) })
    }
    fn direct_stream(
        &self,
        _: String,
        _: String,
    ) -> LocalBoxFuture<'static, Result<demi_backend_remote_host::DirectAdmission, String>> {
        Box::pin(async { Err("no".into()) })
    }
}

#[tokio::test(flavor = "local")]
async fn a_call_runs_only_for_a_live_job_the_policy_admits_and_a_refusal_mints_no_pipe() {
    let device = TestDevice::new(Rc::new(Refusing));
    let mut link = device.connect(None);
    let job = device
        .host("/work", Admission::Free)
        .start_job(job_start("probe live"))
        .await
        .unwrap();
    drain(&mut link).await;
    link.send(rpc_call("invented", "unknown")).await;
    let outcome = call_outcome(&mut link).await;
    assert_eq!(
        outcome,
        (
            "probe: rpc requires a live job dispatched to this device\n".into(),
            1
        )
    );
    link.send(rpc_call(job.id(), "refused")).await;
    let frames = drain(&mut link).await;
    assert!(
        !frames
            .iter()
            .any(|frame| matches!(frame, Inbound::RpcPipes { .. }))
    );
    let stderr: String = frames
        .iter()
        .filter_map(|frame| match frame {
            Inbound::RpcOutput { bytes, .. } => {
                Some(String::from_utf8_lossy(&bytes.0).into_owned())
            }
            _ => None,
        })
        .collect();
    assert_eq!(stderr, "probe: rpc job belongs to another conversation\n");
    assert!(
        frames
            .iter()
            .any(|frame| matches!(frame, Inbound::RpcExit { exit_code: 1, .. }))
    );
    // A second call under a live call's id breaks the protocol.
    let device = TestDevice::new(CommandPolicy::new(live_commands(
        Rc::default(),
        Rc::default(),
    )));
    let mut link = device.connect(None);
    let job = device
        .host("/work", Admission::Free)
        .start_job(job_start("probe live"))
        .await
        .unwrap();
    drain(&mut link).await;
    link.send(rpc_call(job.id(), "twice")).await;
    link.send(rpc_call(job.id(), "twice")).await;
    assert_eq!(
        link.ended().await,
        LinkEnd::Disconnected("duplicate rpc call twice".into())
    );
}

#[tokio::test(flavor = "local", start_paused = true)]
async fn an_unanswered_ping_ends_the_connection_unless_liveness_is_paused() {
    let device = device();
    let mut link = device.connect(Some(Duration::from_secs(30)));
    tokio::time::sleep(Duration::from_millis(30_001)).await;
    assert!(matches!(link.next().await, Inbound::Ping {}));
    link.send(Outbound::Pong { jobs: 2 }).await;
    drain(&mut link).await;
    assert_eq!(link.link().running_jobs(), 2);
    tokio::time::sleep(Duration::from_secs(30)).await;
    assert!(matches!(link.next().await, Inbound::Ping {}));
    // A checkpoint copies the machine: silence is no death, and resuming
    // forgets the ping it missed.
    link.link().pause_liveness();
    tokio::time::sleep(Duration::from_secs(90)).await;
    assert!(drain(&mut link).await.is_empty());
    link.link().resume_liveness();
    tokio::time::sleep(Duration::from_secs(30)).await;
    assert!(matches!(link.next().await, Inbound::Ping {}));
    tokio::time::sleep(Duration::from_secs(30)).await;
    assert_eq!(
        link.ended().await,
        LinkEnd::Disconnected("liveness: ping unanswered".into())
    );
}

#[tokio::test(flavor = "local")]
async fn a_malformed_frame_ends_the_connection() {
    let device = device();
    let link = device.connect(None);
    link.send_frame(b"not a message".to_vec()).await;
    assert!(matches!(link.ended().await, LinkEnd::Refused(_)));
}

/// When the kept output cannot be read once the job ended, the command's
/// whole output is what the backend received: each stream's first bytes,
/// then, beyond what was left out, its newest bytes, which the runner sent
/// before the job's end; so the output's end, where a build says what
/// failed, still shows (`runner.md` § Pipes and output).
#[tokio::test(flavor = "local")]
async fn an_unread_output_ends_with_the_newest_bytes_the_runner_sent() {
    let device = device();
    let mut link = device.connect(None);
    let shell = environment(device.host("/work", Admission::Free));
    let started = shell
        .exec(exec("build"), CancellationToken::new())
        .await
        .unwrap();
    let Inbound::JobStart { job_id, .. } = link.next().await else {
        panic!("expected a job")
    };
    let output = |offset: u64, bytes: &[u8]| Outbound::JobOutput {
        job_id: job_id.clone(),
        stream: OutputStream::Stdout,
        offset,
        bytes: WireBytes(bytes.to_vec()),
    };
    let head = vec![b'x'; JOB_VIEW_BYTES];
    let length = JOB_VIEW_BYTES as u64 * 4;
    let newest = b"error: the build failed\n";
    link.send(output(0, &head)).await;
    link.send(output(length - newest.len() as u64, newest))
        .await;
    link.send(Outbound::JobExit {
        job_id: job_id.clone(),
        exit_code: Some(1),
        signal: None,
        spawn_error: None,
        cwd: None,
        output: Some(OutputLengths {
            stdout_bytes: length,
            stderr_bytes: 0,
        }),
        files: Vec::new(),
        files_truncated: false,
    })
    .await;
    let id = loop {
        let message = tokio::time::timeout(Duration::from_secs(10), link.next())
            .await
            .expect("a read within the hang guard");
        if let Inbound::JobRead { id, .. } = message {
            break id;
        }
    };
    link.send(Outbound::JobRead {
        id,
        error: Some("the job's output is gone".into()),
    })
    .await;
    let whole = loop {
        drain(&mut link).await;
        if let Some(whole) = shell.status(&started.command_id).unwrap().whole {
            break whole;
        }
    };
    assert_eq!(
        whole.output.records(),
        [
            OutputRecord::Output(StreamKind::Stdout, head.into()),
            OutputRecord::LeftOut(length - JOB_VIEW_BYTES as u64 - newest.len() as u64),
            OutputRecord::Output(StreamKind::Stdout, newest.to_vec().into()),
        ]
    );
    assert_eq!(whole.output.missing(), None);
}

/// Input typed while a command still acquires its Host, as while a Cloud
/// wakes, waits and reaches the command once it starts; the page shows the
/// command running all along.
#[tokio::test(flavor = "local")]
async fn input_written_while_a_command_acquires_its_host_reaches_it_once_it_starts() {
    let device = device();
    let mut link = device.connect(None);
    let acquired = Rc::new(tokio::sync::Notify::new());
    let context: demi_backend_remote_host::ContextSource = {
        let acquired = acquired.clone();
        Rc::new(move || {
            let acquired = acquired.clone();
            Box::pin(async move {
                acquired.notified().await;
                Ok(test_command_context())
            })
        })
    };
    let shell = RemoteShellEnvironment::new(EnvironmentOptions::new(
        device.host("/work", Admission::Free),
        context,
        TestPages::new(false),
        Rc::new(CountingNumbers::default()),
    ));
    let started = shell
        .exec(exec("read name; echo $name"), CancellationToken::new())
        .await
        .unwrap();
    let command = started.command_id;
    let written = shell.write(&command, Bytes::from_static(b"Ana\n"));
    let (written, ()) = tokio::join!(written, async {
        acquired.notify_one();
        let Inbound::JobStart { .. } = link.next().await else {
            panic!("expected the job's start")
        };
    });
    written.unwrap();
    let frames = drain(&mut link).await;
    assert!(
        frames
            .iter()
            .any(|frame| matches!(frame, Inbound::JobStdin { bytes, .. } if bytes.0 == b"Ana\n")),
        "{frames:?}"
    );
}

#[tokio::test(flavor = "local")]
async fn pages_that_watch_one_path_share_the_runners_watch_which_the_last_one_ends() {
    use demi_backend_remote_host::WatchUpdate;
    let device = device();
    let mut link = device.connect(None);
    let host = device.host("/work", Admission::Free);
    let mut first = host.watch("/work", true).unwrap();
    let mut second = host.watch("/work", true).unwrap();
    let id = match link.next().await {
        Inbound::FsWatch {
            id,
            path,
            recursive,
        } => {
            assert_eq!((path.as_str(), recursive), ("/work", true));
            id
        }
        other => panic!("expected the watch, got {other:?}"),
    };
    // One watch on the runner for both.
    assert!(link.try_next().is_none());

    link.send(Outbound::FsWatchReady { id: id.clone() }).await;
    assert_eq!(first.next().await, WatchUpdate::Ready);
    assert_eq!(second.next().await, WatchUpdate::Ready);
    let paths = vec!["/work/a.txt".to_owned()];
    link.send(Outbound::FsWatchChanged {
        id: id.clone(),
        paths: paths.clone(),
    })
    .await;
    assert_eq!(first.next().await, WatchUpdate::Changed(paths.clone()));
    assert_eq!(second.next().await, WatchUpdate::Changed(paths));

    // A page that comes later hears that the watch runs, without asking
    // the runner again.
    let mut third = host.watch("/work", true).unwrap();
    assert_eq!(third.next().await, WatchUpdate::Ready);
    drop(first);
    drop(third);
    assert!(link.try_next().is_none());
    drop(second);
    assert_eq!(link.next().await, Inbound::FsUnwatch { id });

    // The connection's end ends a watch.
    let mut last = host.watch("/work", false).unwrap();
    assert!(matches!(link.next().await, Inbound::FsWatch { recursive: false, .. }));
    link.close().await;
    assert_eq!(last.next().await, WatchUpdate::Ended);
}
