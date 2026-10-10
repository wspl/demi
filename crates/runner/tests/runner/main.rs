//! The runner's tests: each drives the built `demi-runner` as a backend
//! does, over its runner socket, in one process. Each test binary costs a
//! link and, when it is new, a first-launch check, so the tests live here.
//! Only the open-file test keeps a binary of its own (`tests/open_files.rs`):
//! it lowers the process's open-file limit and holds every descriptor left.

mod connection;
mod host;
mod host_log;
mod jobs;
mod registration;

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

use demi_backend_remote_host::testing::{RunnerProcess, RunnerProcessOptions};
use demi_command_protocol::{CommandCaller, CommandContext, CommandLocale};
use demi_host_interface::SpawnEnv;
use demi_runner_protocol::wire::{self, Inbound, Outbound};
use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{WebSocketStream, tungstenite::Message};

// Each test's whole-body timeout is a hang guard of 60 s, not a latency
// check: the tests here run together, each with a runner of its own, and a
// tighter bound fails them on a loaded machine.

/// How long a runner may take to connect.
const CONNECT: Duration = Duration::from_secs(15);

/// A built runner and the backend end of its connection: a listener that
/// stands in for the backend's runner socket.
pub struct Host {
    pub process: RunnerProcess,
    listener: TcpListener,
    socket: WebSocketStream<TcpStream>,
}

impl Host {
    /// Starts a runner whose environment is the test's with `env` added, as
    /// the device the backend already paired, and takes its `hello`.
    pub async fn start(env: BTreeMap<String, String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a local port");
        let backend = format!(
            "http://{}",
            listener.local_addr().expect("the listener's address")
        );
        let process = RunnerProcess::start(
            &backend,
            RunnerProcessOptions {
                env: SpawnEnv::Overlay(
                    env.into_iter()
                        .map(|(name, value)| (name, Some(value)))
                        .collect(),
                ),
                token: Some("test-token".into()),
                ..RunnerProcessOptions::default()
            },
        );
        let (socket, hello) = accept(&listener, &process).await;
        assert!(matches!(hello, Outbound::Hello { .. }), "{hello:?}");
        Self {
            process,
            listener,
            socket,
        }
    }

    /// Accepts the registration, as the backend's `hello_ok` does.
    pub async fn online(mut self) -> Self {
        self.send(Inbound::HelloOk {
            device_id: "device".into(),
            device_name: "fixture".into(),
        })
        .await;
        self
    }

    /// The runner's home, its `$HOME`, where its jobs start.
    pub fn home(&self) -> &Path {
        Path::new(self.process.home())
    }

    /// The runner's installation state: its log, its job root and its lock.
    pub fn state(&self) -> PathBuf {
        self.process.state_dir().to_owned()
    }

    pub async fn send(&mut self, message: Inbound) {
        let frame = wire::encode(&message).expect("a valid backend message");
        self.socket
            .send(Message::Binary(frame.into_bytes().into()))
            .await
            .expect("the runner reads its socket");
    }

    /// The next message the runner sends.
    pub async fn frame(&mut self) -> Outbound {
        next(&mut self.socket, &self.process).await
    }

    /// Stops the runner, as a signal does, and answers its close frame; then
    /// starts it again with the same home and state, and takes the `hello`
    /// of its next connection.
    pub async fn restart(&mut self) {
        self.stop().await;
        self.process.start_again();
        let (socket, hello) = accept(&self.listener, &self.process).await;
        assert!(matches!(hello, Outbound::Hello { .. }), "{hello:?}");
        self.socket = socket;
    }

    /// Takes the runner's next connection, as after it lost the last one.
    pub async fn reconnected(&mut self) {
        let (socket, hello) = accept(&self.listener, &self.process).await;
        assert!(matches!(hello, Outbound::Hello { .. }), "{hello:?}");
        self.socket = socket;
    }

    /// Stops the runner, as a signal does, while the backend end reads on and
    /// answers its close frame.
    pub async fn stop(&mut self) {
        let socket = &mut self.socket;
        let answered = async { while socket.next().await.is_some() {} };
        tokio::join!(self.process.stop(), answered);
    }

    pub async fn close(mut self) {
        self.stop().await;
    }
}

/// The runner's next connection and its first message.
async fn accept(
    listener: &TcpListener,
    process: &RunnerProcess,
) -> (WebSocketStream<TcpStream>, Outbound) {
    let accepted = tokio::time::timeout(CONNECT, listener.accept()).await;
    let Ok(Ok((stream, _))) = accepted else {
        panic!("the runner did not connect:\n{}", process.output());
    };
    let mut socket = tokio_tungstenite::accept_async(stream)
        .await
        .expect("a WebSocket handshake");
    let hello = next(&mut socket, process).await;
    (socket, hello)
}

async fn next(socket: &mut WebSocketStream<TcpStream>, process: &RunnerProcess) -> Outbound {
    loop {
        match socket.next().await {
            Some(Ok(Message::Binary(bytes))) => {
                match wire::decode(&bytes).expect("the runner sends valid messages") {
                    // What the cache holds, which the runner reports once
                    // online and after each install; the backend's tests
                    // follow it (`native-runtime.md` § Installed artifacts).
                    Outbound::Installed { .. } => {}
                    message => return message,
                }
            }
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
            other => panic!(
                "the runner's connection ended: {other:?}\n{}",
                process.output()
            ),
        }
    }
}

/// A command context for jobs that run no declared command.
fn context() -> CommandContext {
    CommandContext {
        color_scheme: demi_command_protocol::ColorScheme::Light,
        conversation: "conversation".into(),
        caller: CommandCaller::agent(1),
        locale: CommandLocale {
            time_zone: "UTC".into(),
            languages: vec!["en-US".into()],
        },
    }
}

/// Starts `script` as job `job` in the runner's home.
async fn start_job(host: &mut Host, job: &str, script: &str) {
    let cwd = host.home().to_string_lossy().into_owned();
    host.send(Inbound::JobStart {
        manifest_hash: None,
        viewable: None,
        context: context(),
        job_id: job.into(),
        script: script.into(),
        cwd,
        env: BTreeMap::new(),
        stdin: None,
        stdout: None,
    })
    .await;
}
