//! The connection engine with the test as the runner.

use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::Rc,
    time::Duration,
};

use bytes::Bytes;
use demi_command_service::protocol::{
    ArtifactLocation, ArtifactUrl, EditCopies, EditKind as FileEditKind, PackageArtifact,
    PackageDescriptor, host_target,
};
use demi_command_tree::NativeOperation;
use demi_core::{CommandId, EditKind, EditedFile, KeptEdit, NodeId};
use demi_gates::{ActivityGate, Purpose};
use demi_host_remote::{
    Admission, ArtifactResolver, CommandCatalog, EnvironmentOptions, JobOrigin, JobStart, LinkEnd,
    LinkPolicy, RemoteHost, RemoteShellEnvironment, RetainEdits,
    testing::{CommandPolicy, TEST_DEVICE, TestDevice, TestLink},
};
use demi_runner_protocol::wire::{
    ArtifactOwner, FsOk, FsResult, Inbound, JobArtifactOwner, JobFileChange, Outbound,
    STDIN_CHUNK_BYTES, Signal, VolumeName, WireBytes,
};
use demi_shell::{
    Call, CommandSet, CommandState, ExecRequest, GroupBuilder, HostError, HostErrorKind,
    HostProcess, JobCaller, LeafBuilder, ObservationWindow, PortError, Process, ProcessEnd, RpcError,
    Reader, RpcInvocation, RpcPort, ShellEnvironment, ShellTarget, SpawnEnv, SpawnRequest, StorageOp,
    StorageReply, TypedRpc, testing::test_command_context,
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
        generation: 0,
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
    let context: demi_host_remote::ContextSource =
        Rc::new(|| Box::pin(async { Ok(test_command_context()) }));
    RemoteShellEnvironment::new(EnvironmentOptions::new(host, context))
}

fn exec(script: &str) -> ExecRequest {
    ExecRequest {
        script: script.into(),
        shell: ShellTarget::Default,
        window: ObservationWindow::from_millis(1).unwrap(),
        caller: caller(),
    }
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
    let stat =
        tokio::task::spawn_local(
            async move { demi_shell::Host::fs(&host).stat("/work/file").await },
        );
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
        tokio::task::spawn_local(
            async move { demi_shell::Host::fs(&host).exists("/work/file").await },
        )
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
    let refused = demi_shell::Host::fs(&host)
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
    assert_eq!(demi_shell::Host::identity(&host).hostname, "");
    let mut link = device.connect(None);
    let identity = demi_shell::Host::identity(&host);
    assert_eq!((identity.uid, identity.hostname.as_str()), (501, "test"));
    let pending = {
        let host = host.clone();
        tokio::task::spawn_local(
            async move { demi_shell::Host::fs(&host).read_file("/work/x").await },
        )
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
    assert_eq!(demi_shell::Host::identity(&host), identity);
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
        demi_shell::Host::fs(&host)
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
        tokio::task::spawn_local(async move { demi_shell::Host::fs(&host).exists("/work").await })
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
    let start = |selection: &demi_host_remote::CommandSelection| {
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
        |shell: &RemoteShellEnvironment| match shell.status(&started.command_id, Reader::Model).unwrap().state {
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
        tokio::task::spawn_local(async move { shell.abort(&command, Reader::Model).await })
    };
    let Inbound::JobKill { signal, .. } = link.next().await else {
        panic!("expected the job to be stopped")
    };
    assert_eq!(signal, Some(demi_runner_protocol::wire::Signal::Terminate));
    link.send(job_exit(&job_id, None, Some("SIGTERM"))).await;
    let aborted = aborting.await.unwrap().unwrap();
    assert!(matches!(aborted.state, CommandState::Aborted));
    link.send(hint("late", Some("arrived after the end"), &job_id))
        .await;
    drain(&mut link).await;
    assert!(matches!(
        shell.status(&started.command_id, Reader::Model).unwrap().state,
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
    let mut status = shell.status(&started.command_id, Reader::Model).unwrap();
    for _ in 0..100 {
        if !matches!(status.state, CommandState::Running { .. }) {
            break;
        }
        tokio::task::yield_now().await;
        status = shell.status(&started.command_id, Reader::Model).unwrap();
    }
    assert!(
        matches!(status.state, CommandState::Exited { exit_code: 127, .. }),
        "{:?}",
        status.state
    );
    assert_eq!(
        status.stderr.tail,
        "the job was cancelled before it started\n"
    );
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

impl RetainEdits for Publisher {
    fn retain<'a>(
        &'a self,
        _: &'a CommandId,
        _: &'a [JobFileChange],
    ) -> LocalBoxFuture<'a, Result<Vec<EditedFile>, String>> {
        let barrier = self.barrier.borrow_mut().take();
        Box::pin(async move {
            if let Some(barrier) = barrier {
                let _ = barrier.await;
            }
            Ok(vec![self.file.clone()])
        })
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
        edits: vec![KeptEdit { kept: true }],
    };
    let (publish, barrier) = tokio::sync::oneshot::channel();
    let context: demi_host_remote::ContextSource =
        Rc::new(|| Box::pin(async { Ok(test_command_context()) }));
    let mut options = EnvironmentOptions::new(host, context);
    options.retain = Some(Rc::new(Publisher {
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
        shell.status(&started.command_id, Reader::Model).unwrap().state,
        CommandState::Running { .. }
    ));
    publish.send(()).unwrap();
    let mut status = shell.status(&started.command_id, Reader::Model).unwrap();
    for _ in 0..100 {
        if !matches!(status.state, CommandState::Running { .. }) {
            break;
        }
        tokio::task::yield_now().await;
        status = shell.status(&started.command_id, Reader::Model).unwrap();
    }
    assert!(matches!(
        status.state,
        CommandState::Exited { exit_code: 7, .. }
    ));
    let files = status.files.unwrap();
    assert_eq!((files.files, files.truncated), (vec![file.clone()], true));
    assert_eq!(
        shell
            .status(&started.command_id, Reader::Model)
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
            GroupBuilder::new("probe", "Probes.")
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
        // The handler knows whose command storage the invoking job reaches.
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
            GroupBuilder::new("probe", "Probes.")
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

    fn storage(
        &self,
        _: Rc<JobOrigin>,
        _: StorageOp,
        _: CancellationToken,
    ) -> LocalBoxFuture<'static, Result<StorageReply, PortError>> {
        panic!("a refused call reaches no storage")
    }

    fn grow_volume(&self, _: VolumeName, _: u64) -> LocalBoxFuture<'static, Result<(), String>> {
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
