//! The runner's link tests: each drives the built `demi-runner` as a backend
//! does, over its runner socket, in one process: its streams, flow control
//! and reconnects.

mod connection;
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
use demi_shared_link::{Handshake, Incoming, Liveness, Outgoing, Received, Refusal, Side, Wire};
use demi_runner_protocol::wire::{self, Inbound, Outbound};
use futures_util::{SinkExt, StreamExt, future::ready};
use tokio::{net::TcpListener, sync::mpsc};
use tokio_tungstenite::tungstenite::{
    Error, Message,
    protocol::{CloseFrame, frame::coding::CloseCode},
};
use tokio_util::{sync::PollSender, task::AbortOnDropHandle};

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
    socket: Socket,
    /// The hello of the runner's current connection.
    pub hello: Outbound,
}

impl Host {
    /// Starts a runner whose environment is the test's with `env` added, as
    /// the device the backend already paired, and takes its `hello`.
    pub async fn start(env: BTreeMap<String, String>) -> Self {
        Self::start_with(RunnerProcessOptions {
            env: SpawnEnv::Overlay(
                env.into_iter()
                    .map(|(name, value)| (name, Some(value)))
                    .collect(),
            ),
            ..RunnerProcessOptions::default()
        })
        .await
    }

    /// [`Host::start`] with the runner's `options`, as the device the
    /// backend already paired.
    pub async fn start_with(options: RunnerProcessOptions) -> Self {
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
                token: Some(TOKEN.into()),
                ..options
            },
        );
        let (socket, hello) = accept(&listener, &process).await;
        assert!(matches!(hello, Outbound::Hello { .. }), "{hello:?}");
        Self {
            process,
            listener,
            socket,
            hello,
        }
    }

    /// Accepts the registration, as the backend's `hello_ok` does, with the
    /// conversation the tests' jobs belong to.
    pub async fn online(mut self) -> Self {
        self.welcome(&[CONVERSATION]).await;
        self
    }

    /// Accepts the current connection's registration with the
    /// conversations of the user that still exist.
    pub async fn welcome(&mut self, conversations: &[&str]) {
        self.welcome_archived(conversations, &[]).await;
    }

    /// [`Host::welcome`] naming which of the conversations are archived.
    pub async fn welcome_archived(&mut self, conversations: &[&str], archived: &[&str]) {
        self.send(Inbound::HelloOk {
            device_id: "device".into(),
            device_name: "fixture".into(),
            conversations: conversations.iter().map(|&name| name.to_owned()).collect(),
            archived: archived.iter().map(|&name| name.to_owned()).collect(),
        })
        .await;
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
        let starts = matches!(message, Inbound::HelloOk { .. } | Inbound::Claimed { .. });
        self.socket.send_frame(frame, starts).await;
    }

    /// The next message the runner sends, but the periodic job counts.
    pub async fn frame(&mut self) -> Outbound {
        loop {
            match next(&mut self.socket, &self.process).await {
                Outbound::JobCount { .. } => {}
                message => return message,
            }
        }
    }

    /// Sends `manifest` as the backend does, and waits for the runner's
    /// answer; the reason it gives when it could not install it.
    pub async fn install(&mut self, manifest: &demi_runner_protocol::manifest::Manifest) -> Option<String> {
        self.send(Inbound::Manifest {
            hash: manifest.hash.clone(),
            manifest: serde_json::to_value(manifest).unwrap(),
        })
        .await;
        loop {
            match self.frame().await {
                Outbound::ManifestInstalled { hash, error } if hash == manifest.hash => return error,
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    /// The next message the runner sends, job counts included.
    pub async fn frame_with_counts(&mut self) -> Outbound {
        next(&mut self.socket, &self.process).await
    }

    /// Stops the runner, as a signal does, and answers its close frame; then
    /// starts it again with the same home and state, and takes the `hello`
    /// of its next connection.
    pub async fn restart(&mut self) {
        self.stop().await;
        self.process.start_again();
        self.reconnected().await;
    }

    /// Takes the runner's next connection, as after it lost the last one,
    /// and its hello.
    pub async fn reconnected(&mut self) {
        let (socket, hello) = accept(&self.listener, &self.process).await;
        assert!(matches!(hello, Outbound::Hello { .. }), "{hello:?}");
        self.socket = socket;
        self.hello = hello;
    }

    /// Accepts the runner's next connection and drops it before its
    /// welcome, as a backend still out of reach would: the runner tries
    /// again on its own.
    pub async fn drop_next(&mut self) {
        let (mut socket, hello) = accept(&self.listener, &self.process).await;
        assert!(matches!(hello, Outbound::Hello { .. }), "{hello:?}");
        // A runner that already went sees the connection end all the same.
        socket.close().await;
    }

    /// Drops the connection without a close frame, as a network that went
    /// away does; the runner connects again on its own.
    pub async fn cut(&mut self) {
        // The runner sees the connection end either way.
        let _ = self.socket.cutter.shutdown(std::net::Shutdown::Both);
    }

    /// The jobs the current connection's hello lists, and the runner's
    /// instance.
    pub fn kept(&self) -> (u64, Vec<wire::KeptJob>) {
        match &self.hello {
            Outbound::Hello { instance, jobs, .. } => (*instance, jobs.clone()),
            other => panic!("not a hello: {other:?}"),
        }
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

/// The backend's end of the runner's connection: the link over the
/// WebSocket, and a way to put a raw WebSocket message beside it or to cut
/// the connection without a close frame.
struct Socket {
    outgoing: Outgoing,
    /// The socket's own messages until the backend's welcome; the link's
    /// session after.
    phase: Option<Phase>,
    /// Sends a message below the link.
    raw: mpsc::Sender<Message>,
    /// Shuts the connection's TCP stream down.
    cutter: std::net::TcpStream,
    _writer: AbortOnDropHandle<()>,
}

enum Phase {
    Handshake(Handshake),
    Link(Incoming),
}

impl Socket {
    /// Opens the backend's end of the link over `stream`'s WebSocket.
    async fn open(stream: tokio::net::TcpStream) -> Self {
        let std_stream = stream.into_std().expect("a standard stream");
        let cutter = std_stream.try_clone().expect("a second handle");
        let stream = tokio::net::TcpStream::from_std(std_stream).expect("a tokio stream");
        let socket = tokio_tungstenite::accept_async(stream)
            .await
            .expect("a WebSocket handshake");
        let (mut writer, reader) = socket.split();
        let (raw, mut queued) = mpsc::channel::<Message>(16);
        let writer = tokio::spawn(async move {
            while let Some(message) = queued.recv().await {
                if writer.send(message).await.is_err() {
                    return;
                }
            }
        });
        let incoming = reader.map(received);
        let outgoing = PollSender::new(raw.clone())
            .with(|wire| ready(Ok::<_, tokio_util::sync::PollSendError<Message>>(sent(wire))));
        let handshake = Handshake::open(incoming, outgoing, Side::Backend, Some(Liveness::default()));
        Self {
            outgoing: handshake.outgoing(),
            phase: Some(Phase::Handshake(handshake)),
            raw,
            cutter,
            _writer: AbortOnDropHandle::new(writer),
        }
    }

    /// The next message of the runner: of the socket before the welcome, and
    /// of the link after it.
    async fn next(&mut self) -> Option<Result<Vec<u8>, demi_shared_link::SocketEnd>> {
        match self.phase.as_mut()? {
            Phase::Handshake(handshake) => handshake.read().await,
            Phase::Link(incoming) => incoming.next().await,
        }
    }

    /// Sends the backend's `frame`: as a message of the socket until the
    /// welcome, which starts the link's session, and on the link after it.
    async fn send_frame(&mut self, frame: wire::Frame, starts: bool) {
        match self.phase.take().expect("the socket is open") {
            Phase::Handshake(mut handshake) => {
                handshake
                    .write(frame.into_bytes())
                    .await
                    .expect("the runner reads its socket");
                self.phase = Some(if starts {
                    Phase::Link(handshake.start().incoming)
                } else {
                    Phase::Handshake(handshake)
                });
            }
            Phase::Link(incoming) => {
                self.outgoing
                    .send(frame)
                    .await
                    .expect("the runner reads its socket");
                self.phase = Some(Phase::Link(incoming));
            }
        }
    }

    /// Ends the link from the backend's end, as the backend closing the
    /// socket does.
    async fn close(&mut self) {
        match self.phase.take() {
            Some(Phase::Handshake(handshake)) => handshake.close(None).await,
            Some(Phase::Link(incoming)) => {
                self.outgoing.close(None).await;
                self.phase = Some(Phase::Link(incoming));
            }
            None => {}
        }
    }

    /// Puts `message` on the WebSocket as it is, beside the link.
    async fn send_raw(&self, message: Message) {
        self.raw.send(message).await.expect("the socket's writer");
    }
}

/// What the link reads of a message of the runner's socket.
fn received(message: Result<Message, Error>) -> Result<Received, String> {
    match message {
        Ok(Message::Binary(bytes)) => Ok(Received::Binary(bytes.to_vec())),
        Ok(Message::Close(Some(frame))) if frame.code == CloseCode::Invalid => {
            Ok(Received::Close(Some(Refusal(frame.reason.to_string()))))
        }
        Ok(Message::Close(_)) => Ok(Received::Close(None)),
        Ok(Message::Ping(_) | Message::Pong(_)) => Ok(Received::Nothing),
        Ok(_) => Err("the runner sent a text frame".to_owned()),
        Err(error) => Err(error.to_string()),
    }
}

/// The message the socket sends for what the link gives it.
fn sent(message: Wire) -> Message {
    match message {
        Wire::Binary(bytes) => Message::Binary(bytes.into()),
        Wire::Close(Some(reason)) => Message::Close(Some(CloseFrame {
            code: CloseCode::Invalid,
            reason: wire::close_reason(&reason).to_owned().into(),
        })),
        Wire::Close(None) => Message::Close(None),
    }
}

/// The runner's next connection and its first message.
async fn accept(listener: &TcpListener, process: &RunnerProcess) -> (Socket, Outbound) {
    let accepted = tokio::time::timeout(CONNECT, listener.accept()).await;
    let Ok(Ok((stream, _))) = accepted else {
        panic!("the runner did not connect:\n{}", process.output());
    };
    let mut socket = Socket::open(stream).await;
    let hello = next(&mut socket, process).await;
    (socket, hello)
}

async fn next(socket: &mut Socket, process: &RunnerProcess) -> Outbound {
    loop {
        match socket.next().await {
            Some(Ok(bytes)) => {
                match wire::decode(&bytes).expect("the runner sends valid messages") {
                    // What the cache holds, which the runner reports once
                    // online and after each install; the backend's tests
                    // follow it (`crates/runner-command-packages/src/cache.rs`).
                    Outbound::Installed { .. } => {}
                    message => return message,
                }
            }
            other => panic!(
                "the runner's connection ended: {other:?}\n{}",
                process.output()
            ),
        }
    }
}

/// The device token of the tests' runners.
const TOKEN: &str = "test-token";

/// The conversation the tests' jobs belong to.
const CONVERSATION: &str = "5b2c9e41-7a3d-4f08-9c6e-1d8b3a7f2e90";

/// The command context of a job of `CONVERSATION` that `caller` starts.
fn context_of(caller: CommandCaller) -> CommandContext {
    CommandContext {
        color_scheme: demi_command_protocol::ColorScheme::Light,
        conversation: CONVERSATION.into(),
        caller,
        locale: CommandLocale {
            time_zone: "UTC".into(),
            languages: vec!["en-US".into()],
        },
    }
}

/// Starts `script` as job `job` in the runner's home.
async fn start_job(host: &mut Host, job: &str, script: &str) {
    let cwd = host.home().to_owned();
    start_job_in(host, job, &cwd, script).await;
}

/// Starts `script` as job `job` in `cwd`.
async fn start_job_in(host: &mut Host, job: &str, cwd: &Path, script: &str) {
    start_job_by(host, job, cwd, CommandCaller::agent(1), script).await;
}

/// Starts `script` as job `job` of `caller` in `cwd`.
async fn start_job_by(host: &mut Host, job: &str, cwd: &Path, caller: CommandCaller, script: &str) {
    let cwd = cwd.to_string_lossy().into_owned();
    host.send(Inbound::JobStart {
        manifest_hash: None,
        viewable: None,
        context: context_of(caller),
        job_id: job.into(),
        script: script.into(),
        cwd,
        env: BTreeMap::new(),
        stdin: None,
        stdout: None,
    })
    .await;
}
