//! Test support (`crates-and-packages.md` § command-service): the programs a
//! test finds beside itself, a command service's binary started and driven
//! with a client, a numbers source that counts, and the count of the
//! process's pauses before trying an operation again.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
    sync::atomic::{AtomicU64, Ordering},
};

use tokio::{
    process::{Child, ChildStdin, ChildStdout, Command},
    task::JoinHandle,
};

use crate::{Client, Numbers, ServiceError, protocol::ServiceSequence};

/// The operations of the runner's native fixture service
/// (`crates/runner/tests/fixtures/service.rs`), which the runner's and
/// host-remote's tests name in its descriptors.
pub const FIXTURE_OPERATIONS: [&str; 12] = [
    "where", "echo", "first", "spin", "result", "retain", "stall_release", "held", "crash", "stalled",
    "proceed", "number",
];

/// Every pause a [`crate::descriptors::Backoff`] of this process has taken.
static PAUSES: AtomicU64 = AtomicU64::new(0);

pub(crate) fn count_pause() {
    PAUSES.fetch_add(1, Ordering::Relaxed);
}

/// How many times this process has paused before trying an operation again
/// ([`crate::descriptors::Backoff`]). Nothing else shows that an operation
/// waits out a lack of open files instead of failing, so a test that takes
/// every descriptor away watches this count.
pub fn pauses() -> u64 {
    PAUSES.load(Ordering::Relaxed)
}

/// One counter per conversation and sequence, each from 1: what the
/// backend's sequences give a service whose conversations are new
/// (`native-runtime.md` § Conversation numbers).
#[derive(Default)]
struct Counters(HashMap<(String, ServiceSequence), u64>);

impl Counters {
    /// The first of the next `count` numbers.
    fn take(&mut self, conversation: String, sequence: ServiceSequence, count: u32) -> u64 {
        let next = self.0.entry((conversation, sequence)).or_insert(1);
        let first = *next;
        *next += u64::from(count);
        first
    }
}

/// A numbers source that answers every draw from counters of its own, for
/// a test that calls a handler without a numbers stream. Its task ends with
/// the last clone.
pub fn counting_numbers() -> Numbers {
    let (numbers, mut draws) = Numbers::channel();
    tokio::spawn(async move {
        let mut counters = Counters::default();
        while let Some(draw) = draws.recv().await {
            let first = counters.take(draw.conversation, draw.sequence, draw.count);
            // A caller that stopped waiting needs no answer.
            let _left = draw.answer.send(Ok(first));
        }
    });
    numbers
}

/// Opens the numbers stream of the service `client` drives and answers it
/// from counters, as a runner answers it from the backend's sequences, until
/// the service ends it.
pub async fn answer_numbers(client: &Client) -> Result<JoinHandle<Result<(), ServiceError>>, ServiceError> {
    let stream = client.numbers().await?;
    Ok(tokio::spawn(async move {
        let counters = std::sync::Mutex::new(Counters::default());
        stream
            .answer(|request| {
                let first = counters
                    .lock()
                    .expect("the counters are never poisoned")
                    .take(request.conversation, request.sequence, request.count);
                std::future::ready(Ok(first))
            })
            .await
    }))
}

/// A program Cargo built into the target directory this test runs from,
/// such as a workspace crate's executable.
pub fn built_program(name: &str) -> PathBuf {
    let executable = std::env::current_exe().expect("the test knows its executable");
    let directory = executable
        .parent()
        .and_then(Path::parent)
        .expect("a test runs from the target directory's deps");
    directory.join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

type Transport = tokio::io::Join<ChildStdout, ChildStdin>;

/// A command service's process, serving this test's client over its
/// standard input and output; its standard error is the test's. Dropping it
/// kills the process.
pub struct ServiceProcess {
    client: Client,
    child: Child,
    connection: JoinHandle<Result<(), h2::Error>>,
}

impl ServiceProcess {
    /// Starts `program` with `args`, and `env` beside the test's environment,
    /// and connects to it.
    pub async fn start(
        program: impl AsRef<Path>,
        args: &[&str],
        env: &[(&str, &str)],
    ) -> Result<Self, ServiceError> {
        let mut child = Command::new(program.as_ref())
            .args(args)
            .envs(env.iter().copied())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()?;
        let stdout = child.stdout.take().expect("the service's stdout is piped");
        let stdin = child.stdin.take().expect("the service's stdin is piped");
        let transport: Transport = tokio::io::join(stdout, stdin);
        let (client, connection) = Client::connect(transport).await?;
        Ok(Self {
            client,
            child,
            connection: tokio::spawn(connection),
        })
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    /// The process's ID, while it runs.
    pub fn id(&self) -> Option<u32> {
        self.child.id()
    }

    /// Asks the service to shut down, and waits for its connection to end and
    /// its process to exit. A service that answered may exit before this
    /// side's closing frames reach it, so a connection that ends because the
    /// service closed its side is not a failure (`native-runtime.md`
    /// § Invoke and retire a service).
    pub async fn shutdown(mut self) -> Result<ExitStatus, ServiceError> {
        self.client.shutdown().await?;
        let status = self.child.wait().await?;
        drop(self.client);
        if let Err(error) = self.connection.await?
            && !crate::stream::peer_closed(&error)
        {
            return Err(error.into());
        }
        Ok(status)
    }
}
