//! The connection engine with the test as the runner.

use std::{
    collections::BTreeMap,
    rc::Rc,
    time::Duration,
};

use bytes::Bytes;
use demi_backend_remote_host::{
    Admission, ArtifactResolver, CommandCatalog, EnvironmentOptions,
    JobStart, LinkEnd, RemoteHost, RemoteShellEnvironment,
    testing::{CommandPolicy, TEST_DEVICE, TestDevice, TestLink},
};
use demi_command_protocol::{
    ArtifactLocation, PackageArtifact,
};
use demi_host_interface::{
    Call, CommandSet, CommandState, HostErrorKind,
    HostProcess, JobCaller, LeafBuilder, OutputRecord, PageState,
    ProcessEnd, RpcError, RpcPort, ShellEnvironment, SpawnEnv,
    SpawnRequest, TypedRpc, WholeOutput,
    testing::{CountingNumbers, TestPages, test_command_context},
};
use demi_runner_protocol::wire::{
    FsOk, FsResult, Inbound, KeptRecord, Outbound, OutputLengths, OutputStream,
    STDIN_CHUNK_BYTES, Signal, WireBytes, encode_record,
};
use demi_shared_types::{
    CommandId, NodeId,
};
use futures_util::future::LocalBoxFuture;
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;

use crate::runner::Watched;

fn device() -> TestDevice {
    TestDevice::new(CommandPolicy::new(CommandSet::new()))
}

fn caller() -> JobCaller {
    JobCaller {
        node: NodeId::try_from("test-session").unwrap(),
        tool_use_id: "call".into(),
        turn: None,
    }
}

fn job_start(script: &str) -> JobStart {
    JobStart {
        id: RemoteHost::job_id(),
        script: script.into(),
        cwd: "/work".into(),
        env: BTreeMap::new(),
        context: test_command_context(),
        viewable: None,
        caller: Some(caller()),
        commands: None,
        stdin: None,
        stdout: None,
        stop: CancellationToken::new(),
    }
}

fn spawn(command: &str, retained: bool) -> SpawnRequest {
    SpawnRequest {
        command: command.into(),
        args: Vec::new(),
        cwd: None,
        env: SpawnEnv::Inherit,
        descriptors: Vec::new(),
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

/// An exec of `script`, watched for a millisecond.
fn exec(script: &str) -> crate::runner::Exec {
    crate::runner::exec(script, 1)
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
        output: None,
        files: Vec::new(),
        path_changes: Vec::new(),
        files_truncated: false,
    }
}

// A paused clock stands in for the minute the reply may take.
/// A stop issued while a job's start waits to go again over the new
/// connection reaches the job after its start, so the runner stops it at
/// once instead of starting it unstopped. A few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_stop_while_a_jobs_start_waits_to_go_again_follows_the_start() {
    let device = device();
    let mut link = device.connect();
    let host = device.host("/work", Admission::Free);
    let job = host.start_job(job_start("sleep 60")).await.unwrap();
    drain(&mut link).await;
    link.close().await;
    let mut again = device.connect();
    // The stop runs its first step before the start that goes again is
    // queued: the connection already serves the device.
    let mut stopping = {
        let job = job.clone();
        Box::pin(async move { job.stop().await })
    };
    std::future::poll_fn(|cx| {
        assert!(stopping.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    let stopped = tokio::task::spawn_local(stopping);
    let sent: Vec<Inbound> = drain(&mut again)
        .await
        .into_iter()
        .filter(|message| matches!(message, Inbound::JobStart { .. } | Inbound::JobKill { .. }))
        .collect();
    assert!(
        matches!(&sent[..], [
            Inbound::JobStart { job_id: started, .. },
            Inbound::JobKill { job_id: stopped, signal: Some(Signal::Terminate) },
        ] if started == job.id() && stopped == job.id()),
        "{sent:?}"
    );
    again.send(job_exit(job.id(), None, Some("TERM"))).await;
    assert!(matches!(stopped.await.unwrap().status, ProcessEnd::Signalled(_)));
}

/// A job's start goes again over a new connection only when the runner it
/// went to, the same instance since, does not list it, and only once; a
/// runner that lists it has it, and one of another instance may have run
/// it, so it is lost (`crates/backend-remote-host/src/device_jobs.rs`). A few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_jobs_start_goes_again_once_and_only_to_the_runner_that_did_not_receive_it() {
    let starts = |sent: &[Inbound], id: &str| {
        sent.iter()
            .filter(|message| matches!(message, Inbound::JobStart { job_id, .. } if job_id == id))
            .count()
    };
    // The same runner lists nothing: the start goes again, once.
    let device = device();
    let mut link = device.connect();
    let host = device.host("/work", Admission::Free);
    let job = host.start_job(job_start("true")).await.unwrap();
    assert_eq!(starts(&drain(&mut link).await, job.id()), 1);
    link.close().await;
    let mut again = device.connect();
    assert_eq!(starts(&drain(&mut again).await, job.id()), 1);
    assert!(job.ended().is_none());
    again.close().await;
    let mut third = device.connect();
    assert_eq!(starts(&drain(&mut third).await, job.id()), 0, "a start goes again once");
    assert_eq!(
        job.end().await.status,
        ProcessEnd::Lost(demi_backend_remote_host::NEVER_RECEIVED.into())
    );

    // The same runner lists it: it has it, and nothing goes again.
    let device = self::device();
    let mut link = device.connect();
    let host = device.host("/work", Admission::Free);
    let job = host.start_job(job_start("sleep 1")).await.unwrap();
    drain(&mut link).await;
    link.close().await;
    let mut again = device.connect_with(1, vec![kept_job(job.id(), 0, false)]);
    assert_eq!(starts(&drain(&mut again).await, job.id()), 0);
    again.send(job_exit(job.id(), Some(0), None)).await;
    assert_eq!(job.end().await.status, ProcessEnd::Exited(0));

    // A start that carries pipes is lost with them, by the same runner too.
    let device = self::device();
    let mut link = device.connect();
    let host = device.host("/work", Admission::Free);
    let piped = JobStart {
        stdout: Some(demi_runner_protocol::wire::PipeRef {
            id: "out".into(),
            url: "http://backend/pipes/out".into(),
        }),
        ..job_start("true")
    };
    let job = host.start_job(piped).await.unwrap();
    drain(&mut link).await;
    link.close().await;
    let mut again = device.connect();
    assert_eq!(starts(&drain(&mut again).await, job.id()), 0);
    assert_eq!(
        job.end().await.status,
        ProcessEnd::Lost(demi_backend_remote_host::NEVER_RECEIVED.into())
    );

    // Another runner lists nothing: the job is lost, and nothing goes again.
    let device = self::device();
    let mut link = device.connect();
    let host = device.host("/work", Admission::Free);
    let job = host.start_job(job_start("true")).await.unwrap();
    drain(&mut link).await;
    link.close().await;
    let mut other = device.connect_with(2, Vec::new());
    assert_eq!(starts(&drain(&mut other).await, job.id()), 0);
    assert_eq!(
        job.end().await.status,
        ProcessEnd::Lost(demi_backend_remote_host::RUNNER_RESTARTED.into())
    );
}

#[tokio::test(flavor = "local")]
async fn stdin_travels_in_bounded_frames_in_order_before_its_end() {
    let device = device();
    let mut link = device.connect();
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
    let mut link = device.connect();
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
    // The job outlives the connection (`crates/runner/src/connection/kept.rs`).
    assert!(job.ended().is_none());
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
    // The same Host serves the next connection, over which the job its
    // runner kept goes on and ends.
    let kept = demi_runner_protocol::wire::KeptJob {
        job_id: job.id().to_owned(),
        ended: None,
        output: OutputLengths {
            stdout_bytes: 0,
            stderr_bytes: 0,
        },
        media: 0,
        outliving: Vec::new(),
    };
    let mut link = device.connect_with(1, vec![kept]);
    link.send(job_exit(job.id(), Some(0), None)).await;
    assert_eq!(job.end().await.status, ProcessEnd::Exited(0));
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
        read_at: read_at(),
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

/// Reads what the backend sends until the start of `job`, answering each
/// manifest as the runner does once it is installed: with `error` when there
/// is one. Returns the frames up to and including the start, or up to the
/// manifest that was refused.
async fn answer_until_start(
    link: &mut TestLink,
    job: Option<&str>,
    error: Option<&str>,
) -> Vec<Inbound> {
    let mut frames = Vec::new();
    loop {
        let frame = link.next().await;
        let hash = match &frame {
            Inbound::Manifest { hash, .. } => Some(hash.clone()),
            _ => None,
        };
        let start = matches!(&frame, Inbound::JobStart { job_id, .. } if Some(job_id.as_str()) == job || job.is_none());
        frames.push(frame);
        if let Some(hash) = hash {
            link.send(Outbound::ManifestInstalled {
                hash,
                error: error.map(str::to_owned),
            })
            .await;
            if error.is_some() {
                return frames;
            }
        }
        if start {
            return frames;
        }
    }
}

fn kinds(frames: &[Inbound]) -> Vec<&'static str> {
    frames
        .iter()
        .map(|frame| match frame {
            Inbound::Manifest { .. } => "manifest",
            Inbound::JobStart { .. } => "job_start",
            _ => "other",
        })
        .collect()
}

/// The manifests a connection carried are its own: a job whose manifest the
/// runner reported installed starts after it, jobs that name one the
/// connection carried send nothing more, and a new connection carries its
/// first again.
/// Cost: in process, no runner, a few milliseconds.
#[tokio::test(flavor = "local")]
async fn jobs_start_after_the_manifest_they_name_is_installed_once_per_connection() {
    let device = device();
    let mut link = device.connect();
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
        let id = start.id.clone();
        (id, tokio::task::spawn_local(async move { host.start_job(start).await }))
    };
    let mut frames = Vec::new();
    for selection in [&first, &first, &second, &first] {
        let (id, started) = start(selection);
        frames.extend(answer_until_start(&mut link, Some(&id), None).await);
        started.await.unwrap().unwrap();
    }
    assert_eq!(
        kinds(&frames),
        ["manifest", "job_start", "job_start", "manifest", "job_start", "job_start"]
    );
    let hashes: Vec<_> = frames
        .iter()
        .filter_map(|frame| match frame {
            Inbound::Manifest { hash, manifest } => {
                assert_eq!(manifest["hash"].as_str(), Some(hash.as_str()));
                Some(hash.clone())
            }
            _ => None,
        })
        .collect();
    assert_eq!(hashes, [first.hash(), second.hash()]);
    // A new connection carries its first manifest again. Its runner is
    // another one, so the jobs above are lost rather than sent again.
    link.close().await;
    let mut link = device.connect_with(2, Vec::new());
    let (id, started) = start(&first);
    assert_eq!(
        kinds(&answer_until_start(&mut link, Some(&id), None).await),
        ["manifest", "job_start"]
    );
    started.await.unwrap().unwrap();
    // A manifest that cannot go leaves the selection unknown.
    let mut huge = CommandSet::new();
    huge.register(LeafBuilder::rpc("huge", "x".repeat(5 * 1024 * 1024)).bind(TypedRpc::new(ok)))
        .unwrap();
    let huge = catalog.select(&huge).unwrap();
    assert_eq!(
        start(&huge).1.await.unwrap().err().map(|error| error.kind),
        Some(HostErrorKind::TooLarge)
    );
}

/// A start waits for the answer to its manifest: no start goes before the
/// runner reported the manifest installed, and the jobs that name the same
/// manifest meanwhile wait for the one answer and send no manifest of their
/// own.
/// Cost: in process, no runner, a few milliseconds.
#[tokio::test(flavor = "local")]
async fn no_start_goes_before_its_manifest_is_installed() {
    let device = device();
    let mut link = device.connect();
    let host = device.host("/work", Admission::Free);
    let catalog = CommandCatalog::new(Vec::new(), Rc::new(NoArtifacts)).unwrap();
    let selection = catalog.select(&CommandSet::new()).unwrap();
    let starts: Vec<_> = (0..3)
        .map(|_| {
            let host = host.clone();
            let mut start = job_start("true");
            start.commands = Some(selection.clone());
            tokio::task::spawn_local(async move { host.start_job(start).await.map(drop) })
        })
        .collect();
    let frames = drain(&mut link).await;
    assert_eq!(kinds(&frames), ["manifest"]);
    link.send(Outbound::ManifestInstalled {
        hash: selection.hash().to_owned(),
        error: None,
    })
    .await;
    for start in starts {
        start.await.unwrap().unwrap();
    }
    assert_eq!(kinds(&drain(&mut link).await), ["job_start", "job_start", "job_start"]);
}

/// A manifest the runner could not install fails the jobs waiting for it,
/// with the runner's reason, and the next job sends it again.
/// Cost: in process, no runner, under a millisecond.
#[tokio::test(flavor = "local")]
async fn a_manifest_the_runner_could_not_install_fails_the_start_and_goes_again() {
    let device = device();
    let mut link = device.connect();
    let host = device.host("/work", Admission::Free);
    let catalog = CommandCatalog::new(Vec::new(), Rc::new(NoArtifacts)).unwrap();
    let selection = catalog.select(&CommandSet::new()).unwrap();
    let start = || {
        let host = host.clone();
        let mut start = job_start("true");
        start.commands = Some(selection.clone());
        let id = start.id.clone();
        (id, tokio::task::spawn_local(async move { host.start_job(start).await }))
    };
    let (_, started) = start();
    let frames = answer_until_start(&mut link, None, Some("unsupported manifest")).await;
    assert_eq!(kinds(&frames), ["manifest"]);
    let error = started.await.unwrap().err().expect("the start fails");
    assert!(error.message.contains("unsupported manifest"), "{}", error.message);
    assert!(drain(&mut link).await.is_empty());
    let (id, started) = start();
    assert_eq!(
        kinds(&answer_until_start(&mut link, Some(&id), None).await),
        ["manifest", "job_start"]
    );
    started.await.unwrap().unwrap();
}

/// A runner that never answers for a manifest does not hold its jobs for
/// ever: their starts fail once the wait is over, and the next job sends the
/// manifest anew.
/// Cost: in process, no runner, virtual time.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_runner_that_does_not_answer_for_a_manifest_fails_the_start_in_time() {
    let device = device();
    let mut link = device.connect();
    let host = device.host("/work", Admission::Free);
    let catalog = CommandCatalog::new(Vec::new(), Rc::new(NoArtifacts)).unwrap();
    let selection = catalog.select(&CommandSet::new()).unwrap();
    let mut start = job_start("true");
    start.commands = Some(selection.clone());
    let error = host.start_job(start).await.err().expect("the start fails");
    assert!(error.message.contains("did not answer"), "{}", error.message);
    assert_eq!(kinds(&drain(&mut link).await), ["manifest"]);
    let mut start = job_start("true");
    start.commands = Some(selection);
    let next = start.id.clone();
    let starting = tokio::task::spawn_local({
        let host = host.clone();
        async move { host.start_job(start).await }
    });
    assert_eq!(
        kinds(&answer_until_start(&mut link, Some(&next), None).await),
        ["manifest", "job_start"]
    );
    starting.await.unwrap().unwrap();
}

/// A job whose manifest waits for room in the connection's queue and is
/// given up before it got there leaves the manifest unsent: the next job
/// with the same commands sends it before its start.
/// Cost: in process, no runner, a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_manifest_given_up_before_it_was_queued_goes_with_the_next_job() {
    let device = device();
    let mut link = device.connect();
    let host = device.host("/work", Admission::Free);
    let catalog = CommandCatalog::new(Vec::new(), Rc::new(NoArtifacts)).unwrap();
    let selection = catalog.select(&CommandSet::new()).unwrap();
    let start = |commands: Option<&demi_backend_remote_host::CommandSelection>| {
        let host = host.clone();
        let mut start = job_start("true");
        start.commands = commands.cloned();
        let id = start.id.clone();
        let started = tokio::task::spawn_local(async move { host.start_job(start).await.map(drop) });
        (id, started)
    };
    // Starts without commands fill the queue and the test's end of it, so
    // the next send waits for room.
    let fillers: Vec<_> = (0..4 * demi_backend_remote_host::testing::OUTBOUND_FRAMES)
        .map(|_| start(None))
        .collect();
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    let (_, given_up) = start(Some(&selection));
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    given_up.abort();
    assert!(given_up.await.unwrap_err().is_cancelled());
    let (next, started) = start(Some(&selection));
    let mut manifests = 0;
    loop {
        match link.next().await {
            Inbound::Manifest { hash, .. } => {
                manifests += 1;
                link.send(Outbound::ManifestInstalled { hash, error: None }).await;
            }
            Inbound::JobStart { job_id, .. } if job_id == next => break,
            _ => {}
        }
    }
    assert_eq!(manifests, 1);
    started.await.unwrap().unwrap();
    for (_, filler) in fillers {
        filler.await.unwrap().unwrap();
    }
}

// A message the backend cannot decode ends the connection, and the
// refusal names its type and the decoding error, which the work lost with
// the connection says (`crates/runner-protocol/src/wire/mod.rs`). A few
// milliseconds.
#[tokio::test(flavor = "local")]
async fn a_message_the_backend_cannot_decode_ends_the_connection_naming_it() {
    let device = device();
    let mut link = device.connect();
    let host = device.host("/work", Admission::Free);
    let process = host.spawn(spawn("sleep", false)).await.unwrap();
    drain(&mut link).await;
    // A newer runner's job count, with a field this backend does not know: the
    // MessagePack map of two fields gains a third, `load: 1`.
    let mut newer = demi_runner_protocol::wire::encode(&Outbound::JobCount { jobs: 0 })
        .unwrap()
        .into_bytes();
    assert_eq!(newer[0], 0x82, "a map of two fields");
    newer[0] = 0x83;
    newer.extend(b"\xa4load\x01");
    link.send_frame(newer).await;
    let LinkEnd::Refused(refusal) = link.ended().await else {
        panic!("the connection did not end with a refusal");
    };
    assert!(
        refusal.starts_with("cannot decode job_count: unknown field `load`"),
        "{refusal}"
    );
    assert_eq!(
        process.exit.await,
        ProcessEnd::Lost(format!("runner disconnected: the backend {refusal}"))
    );
    // Bytes that are no message at all are named as a message.
    let link = device.connect();
    link.send_frame(b"not a message".to_vec()).await;
    let LinkEnd::Refused(refusal) = link.ended().await else {
        panic!("the connection did not end with a refusal");
    };
    assert!(refusal.starts_with("cannot decode a message: "), "{refusal}");
}

/// A Host's number, as every fs reply carries one.
fn read_at() -> demi_runner_protocol::wire::ReadAt {
    demi_runner_protocol::wire::ReadAt {
        epoch: "e".into(),
        number: 1,
    }
}

/// A message of `job`'s stdout at `offset`, as its runner sends it.
fn stdout(job: &str, offset: u64, bytes: &[u8]) -> Outbound {
    Outbound::JobOutput {
        job_id: job.into(),
        stream: OutputStream::Stdout,
        offset,
        bytes: WireBytes(bytes.to_vec()),
    }
}

/// `job`'s exit with code 0 after `stdout_bytes` of stdout.
fn exited_after(job: &str, stdout_bytes: u64) -> Outbound {
    Outbound::JobExit {
        job_id: job.into(),
        exit_code: Some(0),
        signal: None,
        spawn_error: None,
        output: Some(OutputLengths {
            stdout_bytes,
            stderr_bytes: 0,
        }),
        files: Vec::new(),
        path_changes: Vec::new(),
        files_truncated: false,
    }
}

/// `job` as a hello lists it: with `stdout_bytes` of stdout, and ended
/// with code 0 when `ended`.
fn kept_job(job: &str, stdout_bytes: u64, ended: bool) -> demi_runner_protocol::wire::KeptJob {
    demi_runner_protocol::wire::KeptJob {
        job_id: job.into(),
        ended: ended.then(|| demi_runner_protocol::wire::KeptEnd {
            exit_code: Some(0),
            signal: None,
        }),
        output: OutputLengths {
            stdout_bytes,
            stderr_bytes: 0,
        },
        media: 0,
        outliving: Vec::new(),
    }
}

/// `bytes` as the kept output holds them, one read of stdout.
fn kept_stdout(bytes: &[u8]) -> Vec<u8> {
    encode_record(&KeptRecord::Output(OutputStream::Stdout, WireBytes(bytes.to_vec()))).unwrap()
}

/// Waits for the backend's read of `job`'s kept output, under the hang
/// guard, and answers its request id and pipe.
async fn requested_read(
    link: &mut TestLink,
    job: &str,
) -> (String, demi_runner_protocol::wire::PipeRef) {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(10), link.next())
            .await
            .expect("a read within the hang guard");
        if let Inbound::JobRead { id, job_id, output } = message
            && job_id == job
        {
            return (id, output);
        }
    }
}

/// The command's end and its whole output, once it ended, under the hang
/// guard.
async fn whole_once_ended(
    shell: &RemoteShellEnvironment,
    command: &CommandId,
) -> demi_host_interface::WholeView {
    tokio::time::timeout(Duration::from_secs(10), shell.ended(command))
        .await
        .expect("an end within the hang guard")
        .unwrap();
    shell.status(command).unwrap().whole.expect("the whole output")
}

/// The output's text, each part left out shown where it was.
fn printed(output: &WholeOutput) -> String {
    output
        .records()
        .iter()
        .map(|record| match record {
            OutputRecord::Output(_, bytes) => String::from_utf8_lossy(bytes).into_owned(),
            OutputRecord::LeftOut(bytes) => format!("[{bytes} left out]"),
        })
        .collect()
}

/// A command whose runner printed while no connection served: the first
/// connection gets "a", the runner prints "bc" meanwhile, and the next
/// connection's read of the kept output is under way when "d" and, with
/// `ended`, the job's end arrive, and that connection is lost too. The
/// device's runner is instance 1.
async fn away_twice(
    device: &TestDevice,
    shell: &RemoteShellEnvironment,
    ended: bool,
) -> (CommandId, String) {
    let mut link = device.connect();
    let started = shell
        .exec(exec("print"), CancellationToken::new())
        .await
        .unwrap();
    let Inbound::JobStart { job_id, .. } = link.next().await else {
        panic!("expected a job")
    };
    link.send(stdout(&job_id, 0, b"a")).await;
    drain(&mut link).await;
    link.close().await;

    let mut link = device.connect_with(1, vec![kept_job(&job_id, 3, false)]);
    requested_read(&mut link, &job_id).await;
    link.send(stdout(&job_id, 3, b"d")).await;
    if ended {
        link.send(exited_after(&job_id, 4)).await;
    }
    drain(&mut link).await;
    link.close().await;
    // The command is as it was when the first connection was lost.
    let status = shell.status(&started.command_id).unwrap();
    assert!(status.whole.is_none(), "the command waits for the next connection");
    (started.command_id, job_id)
}

/// A job that goes on over a new connection gets the output its runner
/// printed while no connection served: the backend reads the kept output
/// again and delivers what its consumer did not receive, before what the
/// job prints from then on and before its end, which may both come while
/// the read is under way (`crates/backend-remote-host/src/device_jobs.rs`). A few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_job_kept_through_a_lost_connection_gets_the_output_it_printed_meanwhile() {
    let device = device();
    let mut link = device.connect();
    let host = device.host("/work", Admission::Free);
    let job = host.start_job(job_start("print")).await.unwrap();
    drain(&mut link).await;
    link.send(stdout(job.id(), 0, b"a")).await;
    link.close().await;

    let mut link = device.connect_with(1, vec![kept_job(job.id(), 3, false)]);
    // Output and the end that arrive during the read wait for it.
    link.send(stdout(job.id(), 3, b"d")).await;
    link.send(exited_after(job.id(), 4)).await;
    answer_read(&device, &mut link, job.id(), kept_stdout(b"abc")).await;

    let mut received = Vec::new();
    while let Some(chunk) = job.next_output().await {
        received.push((chunk.offset, chunk.bytes.to_vec()));
    }
    assert_eq!(
        received,
        [(0, b"a".to_vec()), (1, b"bc".to_vec()), (3, b"d".to_vec())]
    );
    assert_eq!(job.end().await.status, ProcessEnd::Exited(0));
}

/// A connection lost again while the read after a reconnect is under way,
/// with output and the job's end waiting behind it, leaves the command
/// running; the next connection's read completes it, and the end stores
/// the whole output (`crates/backend-remote-host/src/device_jobs.rs`). A few milliseconds.
#[tokio::test(flavor = "local")]
async fn an_end_that_waited_through_another_loss_completes_with_the_next_connection() {
    let device = device();
    let shell = environment(device.host("/work", Admission::Free));
    let (command, job) = away_twice(&device, &shell, true).await;

    // The runner lists the job as ended and sends its exit again.
    let mut link = device.connect_with(1, vec![kept_job(&job, 4, true)]);
    link.send(exited_after(&job, 4)).await;
    answer_read(&device, &mut link, &job, kept_stdout(b"abcd")).await;
    let whole = whole_once_ended(&shell, &command).await;
    assert_eq!(printed(&whole.output), "abcd");
    assert_eq!(whole.output.missing(), None);
}

/// The same loss before the job ended: the job goes on over the next
/// connection, whose read gives what the runner printed meanwhile, and
/// the command ends with all of it. A few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_job_that_runs_through_another_loss_gets_all_its_output_with_the_next_connection() {
    let device = device();
    let shell = environment(device.host("/work", Admission::Free));
    let (command, job) = away_twice(&device, &shell, false).await;

    let mut link = device.connect_with(1, vec![kept_job(&job, 5, false)]);
    answer_read(&device, &mut link, &job, kept_stdout(b"abcde")).await;
    link.send(stdout(&job, 5, b"f")).await;
    link.send(exited_after(&job, 6)).await;
    let whole = whole_once_ended(&shell, &command).await;
    assert_eq!(printed(&whole.output), "abcdef");
    assert_eq!(whole.output.missing(), None);
}

/// The same loss, after which the runner comes back without the job, as
/// one that started anew after the grace: the command is lost, with the
/// output that arrived up to the first gap and a note that counts the
/// rest; "d", past the gap, is not joined to "a". A few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_job_whose_runner_comes_back_without_it_keeps_what_arrived_and_says_the_rest_is_missing() {
    let device = device();
    let shell = environment(device.host("/work", Admission::Free));
    let (command, _) = away_twice(&device, &shell, true).await;

    let _link = device.connect_with(2, Vec::new());
    let whole = whole_once_ended(&shell, &command).await;
    assert_eq!(printed(&whole.output), "a");
    let missing = whole.output.missing().expect("the rest is missing");
    assert_eq!(missing.bytes, 3);
    let CommandState::Lost { reason, .. } = shell.status(&command).unwrap().state else {
        panic!("the command is lost")
    };
    assert_eq!(reason, demi_backend_remote_host::RUNNER_RESTARTED);
}

/// A stop while the runner's connection is away reaches the job once the
/// runner is back (`crates/runner-shell/src/job.rs`): meanwhile
/// the command shows stopping and still runs; the new connection carries
/// `TERM`, and `KILL` once the grace passed without the job's end; and the
/// command turns stopped only when the runner reports that end. Before, the
/// stop reached nothing, and the command was marked stopped ten seconds
/// later while its job ran on. The grace passes on a paused clock.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_stop_while_the_runner_is_away_reaches_the_job_when_it_is_back() {
    use demi_runner_protocol::wire::Signal;
    let device = device();
    let mut link = device.connect();
    let shell = environment(device.host("/work", Admission::Free));
    let started = shell
        .exec(exec("sleep 30"), CancellationToken::new())
        .await
        .unwrap();
    let job = loop {
        if let Inbound::JobStart { job_id, .. } = link.next().await {
            break job_id;
        }
    };
    let command = &started.command_id;
    let state = |shell: &RemoteShellEnvironment| {
        let view = shell
            .page_views()
            .into_iter()
            .find(|view| view.command_id == *command)
            .expect("the command's view");
        (view.state, shell.status(command).unwrap().state)
    };
    link.close().await;

    shell.abort(command).await.unwrap();
    assert!(matches!(
        state(&shell),
        (PageState::Stopping, CommandState::Running { .. })
    ));

    let kept = demi_runner_protocol::wire::KeptJob {
        job_id: job.clone(),
        ended: None,
        output: OutputLengths {
            stdout_bytes: 0,
            stderr_bytes: 0,
        },
        media: 0,
        outliving: Vec::new(),
    };
    let mut link = device.connect_with(1, vec![kept]);
    let mut signals = Vec::new();
    while signals.len() < 2 {
        if let Inbound::JobKill { job_id, signal } = link.next().await {
            assert_eq!(job_id, job);
            signals.push(signal);
        }
    }
    assert_eq!(signals, [Some(Signal::Terminate), Some(Signal::Kill)]);
    assert!(matches!(
        state(&shell),
        (PageState::Stopping, CommandState::Running { .. })
    ));

    link.send(job_exit(&job, None, Some("SIGKILL"))).await;
    shell.ended(command).await.unwrap();
    assert!(matches!(state(&shell), (PageState::Aborted, CommandState::Aborted)));
}

#[tokio::test(flavor = "local")]
async fn a_services_messages_and_its_loss_reach_the_policy_in_the_order_the_runner_sent_them() {
    let policy = CommandPolicy::new(CommandSet::new());
    let device = TestDevice::new(policy.clone());
    let link = device.connect();
    let mut events = policy.package_events();
    for message in [serde_json::json!({ "tabs": "changed" }), serde_json::json!(2)] {
        link.send(Outbound::PackageMessage {
            package: "demi.browser".into(),
            conversation_id: "c1".into(),
            message,
        })
        .await;
    }
    link.send(Outbound::PackageLost {
        package: "demi.browser".into(),
    })
    .await;
    let handed = events
        .wait_for(|events| events.len() == 3)
        .await
        .unwrap()
        .clone();
    let message = |message: Value| demi_backend_remote_host::PackageEvent::Message {
        package: "demi.browser".into(),
        conversation: "c1".into(),
        message,
    };
    assert_eq!(
        handed,
        vec![
            message(serde_json::json!({ "tabs": "changed" })),
            message(serde_json::json!(2)),
            demi_backend_remote_host::PackageEvent::Lost {
                package: "demi.browser".into()
            },
        ]
    );
}
