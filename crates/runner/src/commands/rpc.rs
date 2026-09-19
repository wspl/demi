//! Application callback streams carried over the authenticated runner connection.

use crate::commands::manifest::Parsed;
use crate::connection::wire::{self as wire, Inbound, JobStartStdin};
use crate::{
    commands::command_output::CommandOutput, commands::contexts::ExecutionContext,
    pipes::PipeClient,
};
use bytes::Bytes;
use demi_command_service::{Input, ServiceError};
use futures_util::StreamExt;
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

pub struct Request {
    pub context: Arc<ExecutionContext>,
    pub root: String,
    pub argv: Vec<String>,
    pub parsed: Parsed,
    pub cwd: String,
    pub env: BTreeMap<String, String>,
    pub live: bool,
    pub finite: bool,
}

enum Event {
    Pipes {
        stdin: Option<JobStartStdin>,
        stdout: JobStartStdin,
    },
    Stderr(Bytes),
    Pull,
    Exit(u8),
}
struct Entry {
    events: mpsc::Sender<Event>,
    cancel: CancellationToken,
}
struct CallTransport<'a> {
    id: &'a str,
    output: &'a mpsc::Sender<wire::Outbound>,
    stop: &'a CancellationToken,
}

struct Connection {
    output: mpsc::Sender<wire::Outbound>,
    stop: CancellationToken,
}
struct State {
    connection: Option<Connection>,
    calls: HashMap<String, Entry>,
}

pub struct Calls {
    state: Arc<Mutex<State>>,
    pipes: PipeClient,
}
struct Pending {
    id: String,
    state: Arc<Mutex<State>>,
}
impl Pending {
    fn complete(&self) {
        if let Some(entry) = self.state.lock().unwrap().calls.remove(&self.id) {
            entry.cancel.cancel();
        }
    }
}
impl Drop for Pending {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap();
        if let Some(entry) = state.calls.remove(&self.id) {
            entry.cancel.cancel();
            if let Some(connection) = &mut state.connection {
                match wire::rpc_cancel(self.id.clone()) {
                    Ok(message) => {
                        if connection.output.try_send(message).is_err() {
                            connection.stop.cancel();
                        }
                    }
                    // A dropped cancellation message could leave callback work live.
                    // Closing the transport makes backend teardown authoritative.
                    _ => connection.stop.cancel(),
                }
            }
        }
    }
}

/// A hint belongs to one invocation and is cleared even if its future is dropped.
pub struct RunningHint {
    clear: Option<wire::Outbound>,
    output: mpsc::Sender<wire::Outbound>,
    stop: CancellationToken,
}
impl Drop for RunningHint {
    fn drop(&mut self) {
        if let Some(message) = self.clear.take()
            && self.output.try_send(message).is_err()
        {
            // Backend disconnection clears job hints if a clear cannot be queued.
            self.stop.cancel();
        }
    }
}

impl Calls {
    pub async fn running_hint(
        &self,
        job_id: &str,
        hint: Option<&str>,
    ) -> Result<Option<RunningHint>, ServiceError> {
        let Some(hint) = hint else {
            return Ok(None);
        };
        let (output, stop) = {
            let state = self.state.lock().unwrap();
            let connection = state.connection.as_ref().ok_or(ServiceError::Cancelled)?;
            (connection.output.clone(), connection.stop.clone())
        };
        let id = uuid::Uuid::new_v4().simple().to_string();
        let set = wire::job_running_hint(job_id.into(), id.clone(), Some(hint.into()))
            .map_err(handler)?;
        let clear = wire::job_running_hint(job_id.into(), id, None).map_err(handler)?;
        let guard = RunningHint {
            clear: Some(clear),
            output,
            stop,
        };
        guard
            .output
            .send(set)
            .await
            .map_err(|_| ServiceError::Cancelled)?;
        Ok(Some(guard))
    }
    pub fn new(pipes: PipeClient) -> Arc<Self> {
        Arc::new(Self {
            state: Arc::new(Mutex::new(State {
                connection: None,
                calls: HashMap::new(),
            })),
            pipes,
        })
    }
    pub fn attach(&self, output: mpsc::Sender<wire::Outbound>, stop: CancellationToken) {
        self.detach();
        self.state.lock().unwrap().connection = Some(Connection { output, stop });
    }
    pub fn detach(&self) {
        let mut state = self.state.lock().unwrap();
        state.connection = None;
        for (_, entry) in state.calls.drain() {
            entry.cancel.cancel();
        }
    }
    pub fn reply(&self, message: &Inbound) -> bool {
        let (id, event) = match message {
            Inbound::RpcPipes {
                call_id,
                stdin,
                stdout,
            } => (
                call_id,
                Event::Pipes {
                    stdin: stdin.clone(),
                    stdout: stdout.clone(),
                },
            ),
            Inbound::RpcOutput { call_id, bytes } => {
                (call_id, Event::Stderr(bytes.0.clone().into()))
            }
            Inbound::RpcStdinPull { call_id } => (call_id, Event::Pull),
            Inbound::RpcExit { call_id, exit_code } => {
                if !exit_code.is_finite()
                    || exit_code.fract() != 0.0
                    || !(0.0..=255.0).contains(exit_code)
                {
                    if let Some(entry) = self.state.lock().unwrap().calls.get(call_id) {
                        entry.cancel.cancel();
                    }
                    return true;
                }
                (call_id, Event::Exit(*exit_code as u8))
            }
            _ => return false,
        };
        if let Some(entry) = self.state.lock().unwrap().calls.get(id)
            && entry.events.try_send(event).is_err()
        {
            entry.cancel.cancel();
        }
        true
    }

    pub async fn invoke(
        &self,
        request: Request,
        input: Input,
        output: CommandOutput,
        cancel: CancellationToken,
    ) -> Result<u8, ServiceError> {
        if request.context.agent_session_id.is_empty() || request.context.shell_id.is_empty() {
            return Err(ServiceError::Handler(
                "RPC requires an application-dispatched job and session".into(),
            ));
        }
        let id = uuid::Uuid::new_v4().simple().to_string();
        let stop = cancel.child_token();
        let _stop_guard = stop.clone().drop_guard();
        let (events, receiver) = mpsc::channel(8);
        let connection = {
            let mut state = self.state.lock().unwrap();
            let connection = state
                .connection
                .as_ref()
                .ok_or(ServiceError::Cancelled)?
                .output
                .clone();
            state.calls.insert(
                id.clone(),
                Entry {
                    events,
                    cancel: stop.clone(),
                },
            );
            connection
        };
        let pending = Pending {
            id: id.clone(),
            state: self.state.clone(),
        };
        let message = wire::rpc_call(
            request.context.job_id.clone(),
            id.clone(),
            request.context.agent_session_id.clone(),
            request.context.shell_id.clone(),
            request.root.clone(),
            request.parsed.path.clone(),
            request.argv.clone(),
            request.parsed.values.clone().into_iter().collect(),
            request.parsed.json,
            request.cwd.clone(),
            request.env.clone(),
            request.finite,
        )
        .map_err(handler)?;
        let result = async {
            connection
                .send(message)
                .await
                .map_err(|_| ServiceError::Cancelled)?;
            if !request.live {
                connection
                    .send(wire::rpc_stdin_end(id.clone()).map_err(handler)?)
                    .await
                    .map_err(|_| ServiceError::Cancelled)?;
            }
            let transport = CallTransport {
                id: &id,
                output: &connection,
                stop: &stop,
            };
            self.exchange(&request, receiver, input, output, &transport)
                .await
        };
        let result = tokio::select! {
            biased;
            _ = request.context.cancel.cancelled() => Err(ServiceError::Cancelled),
            _ = stop.cancelled() => Err(ServiceError::Cancelled),
            result = result => result,
        };
        if result.is_ok() {
            pending.complete();
        }
        result
    }

    async fn exchange(
        &self,
        request: &Request,
        mut events: mpsc::Receiver<Event>,
        input: Input,
        output: CommandOutput,
        transport: &CallTransport<'_>,
    ) -> Result<u8, ServiceError> {
        let &CallTransport {
            id,
            output: connection,
            stop,
        } = transport;
        let (stdin_sender, stdin_receiver) = oneshot::channel();
        let (stdout_sender, stdout_receiver) = oneshot::channel();
        let (pull, demanded) = mpsc::channel(1);
        let control = async {
            let mut pipe_senders = Some((stdin_sender, stdout_sender));
            while let Some(event) = events.recv().await {
                match event {
                    Event::Pipes { stdin, stdout } => {
                        let (stdin_sender, stdout_sender) = pipe_senders
                            .take()
                            .ok_or_else(|| ServiceError::Handler("duplicate RPC pipes".into()))?;
                        if stdin.is_some() != request.finite {
                            return Err(ServiceError::Handler(
                                "RPC input pipe disagrees with invocation".into(),
                            ));
                        }
                        stdin_sender
                            .send(stdin)
                            .map_err(|_| ServiceError::Cancelled)?;
                        stdout_sender
                            .send(Some(stdout))
                            .map_err(|_| ServiceError::Cancelled)?;
                    }
                    Event::Stderr(bytes) => output.stderr(bytes).await?,
                    Event::Pull if request.live => pull.try_send(()).map_err(|_| {
                        ServiceError::Handler("overlapping RPC stdin demands".into())
                    })?,
                    Event::Pull => connection
                        .send(wire::rpc_stdin_end(id.into()).map_err(handler)?)
                        .await
                        .map_err(|_| ServiceError::Cancelled)?,
                    Event::Exit(code) => {
                        if let Some((stdin, stdout)) = pipe_senders.take() {
                            if code == 0 {
                                return Err(ServiceError::Handler(
                                    "RPC success arrived before pipe descriptors".into(),
                                ));
                            }
                            stdin.send(None).map_err(|_| ServiceError::Cancelled)?;
                            stdout.send(None).map_err(|_| ServiceError::Cancelled)?;
                        }
                        return Ok(code);
                    }
                }
            }
            Err(ServiceError::Cancelled)
        };
        let download = async {
            let Some(reference): Option<JobStartStdin> =
                stdout_receiver.await.map_err(|_| ServiceError::Cancelled)?
            else {
                return Ok(());
            };
            let result = async {
                let mut stream = self.pipes.get(&reference.url, stop.clone()).await?;
                while let Some(bytes) = stream.next().await {
                    output.stdout(bytes?).await.map_err(io::Error::other)?;
                }
                Ok::<_, io::Error>(())
            }
            .await;
            report(connection, &reference, &result, stop).await?;
            result.map_err(handler)
        };
        let input = self.input(request.live, stdin_receiver, demanded, input, transport);
        let complete = async {
            let (code, ()) = tokio::try_join!(control, download)?;
            Ok(code)
        };
        tokio::pin!(complete);
        tokio::select! {
            result = &mut complete => result,
            result = input => {
                match result {
                    Ok(()) => complete.await,
                    Err(error) => {
                        // A callback may finish successfully without consuming its
                        // finite pipe; the broker then closes that unused upload.
                        // Give its ordered exit a bounded opportunity to arrive.
                        tokio::time::timeout(Duration::from_secs(15), complete).await.unwrap_or(Err(error))
                    }
                }
            }
        }
    }

    async fn input(
        &self,
        live: bool,
        reference: oneshot::Receiver<Option<JobStartStdin>>,
        mut demanded: mpsc::Receiver<()>,
        mut input: Input,
        transport: &CallTransport<'_>,
    ) -> Result<(), ServiceError> {
        let &CallTransport {
            id,
            output: connection,
            stop,
        } = transport;
        if live {
            let _reference = reference.await.map_err(|_| ServiceError::Cancelled)?;
            while demanded.recv().await.is_some() {
                let message = match input.next().await? {
                    Some(bytes) => wire::rpc_stdin(id.into(), wire::WireBytes(bytes.to_vec())),
                    None => {
                        connection
                            .send(wire::rpc_stdin_end(id.into()).map_err(handler)?)
                            .await
                            .map_err(|_| ServiceError::Cancelled)?;
                        return Ok(());
                    }
                }
                .map_err(handler)?;
                connection
                    .send(message)
                    .await
                    .map_err(|_| ServiceError::Cancelled)?;
            }
            return Ok(());
        }
        if let Some(reference) = reference.await.map_err(|_| ServiceError::Cancelled)? {
            let token = stop.clone();
            let body =
                futures_util::stream::unfold((input, token), |(mut input, stop)| async move {
                    tokio::select! {
                        _ = stop.cancelled() => None,
                        bytes = input.next() => match bytes {
                            Ok(Some(bytes)) => Some((Ok(bytes), (input, stop))),
                            Ok(None) => None,
                            Err(error) => Some((Err(io::Error::other(error)), (input, stop))),
                        }
                    }
                });
            let result = self.pipes.put(&reference.url, body, stop).await;
            report(connection, &reference, &result, stop).await?;
            result.map_err(handler)?;
        }
        Ok(())
    }
}

async fn report(
    output: &mpsc::Sender<wire::Outbound>,
    reference: &JobStartStdin,
    result: &io::Result<()>,
    stop: &CancellationToken,
) -> Result<(), ServiceError> {
    let message = wire::pipe_done(
        reference.id.clone(),
        result.is_ok(),
        result.as_ref().err().map(ToString::to_string),
    )
    .map_err(handler)?;
    tokio::select! {
        _ = stop.cancelled() => Err(ServiceError::Cancelled),
        result = output.send(message) => result.map_err(|_| ServiceError::Cancelled),
    }
}
fn handler(error: impl std::fmt::Display) -> ServiceError {
    ServiceError::Handler(error.to_string())
}
