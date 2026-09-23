//! User streams the backend opens on a resident native service (`runner.md`
//! § Service streams): invoke the declared operation with the stream's command
//! context, then carry bytes between the invocation and the two pipes the
//! request named.

use std::{
    io,
    sync::{Arc, Mutex},
};

use bytes::Bytes;
use demi_command_service::{
    CommandInput, CommandOutput,
    protocol::{Invocation, MAX_RECORD_BYTES, Record},
};
use futures_util::{StreamExt, stream::BoxStream};
use tokio::sync::mpsc;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::{
    commands::artifacts::Artifacts,
    connection::wire::{self, ServiceErrorCode},
    host_log::{self, LineSplitter},
    pipes::PipeClient,
    services::ServiceHandle,
    tail::TailBuffer,
    tasks::report_pipe,
};

/// Output records wait here for the upload, so the invocation backs off
/// behind the output pipe rather than buffering without limit.
const OUTPUT_QUEUE: usize = 4;

pub struct ServiceStreams {
    output: mpsc::Sender<wire::Frame>,
    pipes: PipeClient,
    services: ServiceHandle,
    artifacts: Arc<Artifacts>,
    draining: CancellationToken,
    streams: TaskTracker,
    /// Ends every open stream: cancelled by `close` and by the host
    /// connection's shutdown.
    cancel: CancellationToken,
}

impl ServiceStreams {
    pub fn new(
        output: mpsc::Sender<wire::Frame>,
        pipes: PipeClient,
        services: ServiceHandle,
        artifacts: Arc<Artifacts>,
        draining: CancellationToken,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            output,
            pipes,
            services,
            artifacts,
            draining,
            streams: TaskTracker::new(),
            cancel,
        }
    }

    /// Starts one stream; connection cancellation (or `close`) ends every
    /// open stream.
    pub fn handle_open(&self, message: wire::Inbound) -> io::Result<()> {
        let wire::Inbound::ServiceOpen {
            stream_id,
            context,
            package,
            operation,
            args,
            json,
            cwd,
            input,
            output,
        } = message
        else {
            return Err(io::Error::other("not a service_open request"));
        };
        if self.cancel.is_cancelled() {
            return Err(io::Error::other("host connection closed"));
        }
        let pipes = self.pipes.clone();
        let reply = self.output.clone();
        let reporting = self.cancel.clone();
        let stream = self.cancel.child_token();
        let services = self.services.clone();
        let resolver = self.artifacts.for_stream(stream_id.clone());
        let digest = package
            .targets
            .get(crate::services::target())
            .map(|artifact| artifact.sha256.clone());
        let draining = self.draining.clone();
        let log = StreamLog {
            source: format!("stream:{operation}"),
            conversation: context.conversation.clone(),
        };
        self.streams.spawn(async move {
            if draining.is_cancelled() {
                let message = "the runner is draining for an upgrade".to_owned();
                send_error(&reply, stream_id, ServiceErrorCode::Refused, message, &stream, &log).await;
                return;
            }
            if !package.operations.contains(&operation) {
                let message = format!("{} has no operation {operation}", package.id);
                send_error(&reply, stream_id, ServiceErrorCode::UnknownOperation, message, &stream, &log).await;
                return;
            }
            // The stream holds its service from the start
            // (`native-runtime.md` § Keep a service resident).
            let _lease = digest.map(|digest| services.lease(digest));
            let opened = async {
                let mut resident = services
                    .acquire(&package, resolver, &stream)
                    .await
                    .map_err(|error| error.to_string())?;
                let invocation = Invocation {
                    operation,
                    invocation_id: stream_id.clone(),
                    context,
                    args: serde_json::Value::Object(args.unwrap_or_default().into_iter().collect()),
                    cwd,
                    env: Default::default(),
                    edits: None,
                    json,
                };
                match resident.client().invoke(&invocation).await {
                    Ok(exchange) => Ok(exchange),
                    Err(error) => Err(resident.failure(error).await),
                }
            };
            let (command_input, command_output) = tokio::select! {
                _ = stream.cancelled() => return,
                result = opened => match result {
                    Ok(exchange) => exchange,
                    Err(message) => {
                        send_error(&reply, stream_id, ServiceErrorCode::ServiceFailed, message, &stream, &log)
                            .await;
                        return;
                    }
                },
            };
            // No bytes move before the answer.
            match wire::encode(&wire::Outbound::ServiceOpened { stream_id: stream_id.clone() }) {
                Ok(message) => {
                    tokio::select! {
                        _ = stream.cancelled() => return,
                        result = reply.send(message) => {
                            if result.is_err() {
                                return;
                            }
                        }
                    }
                }
                Err(error) => {
                    host_log::runner(format_args!("service_opened encoding failed: {error}"));
                    return;
                }
            }
            log.event("opened");
            let (pulls, demanded) = mpsc::channel(1);
            let completed = CancellationToken::new();
            let exchange = TaskTracker::new();
            // Input pipe → the invocation's input, one chunk per pull; input
            // EOF ends the invocation's input.
            exchange.spawn({
                let pipes = pipes.clone();
                let reply = reply.clone();
                let cancel = stream.clone();
                let completed = completed.clone();
                let reporting = reporting.clone();
                async move {
                    let result = tokio::select! {
                        biased;
                        // The invocation is over; it asks for no more input.
                        _ = completed.cancelled() => Ok(()),
                        result = pump_input(pipes, &input.url, command_input, demanded, &cancel) => result,
                    };
                    if result.is_err() {
                        cancel.cancel();
                    }
                    report_pipe(&reply, input.id, result, &reporting).await;
                }
            });
            // The invocation's standard output → output pipe; its completion
            // ends the upload.
            let outcome = Arc::new(Mutex::new(None));
            exchange.spawn({
                let reply = reply.clone();
                let cancel = stream.clone();
                let log = log.clone();
                let outcome = outcome.clone();
                async move {
                    let result = pump_output(
                        pipes,
                        &output.url,
                        command_output,
                        pulls,
                        &completed,
                        &cancel,
                        &log,
                        &outcome,
                    )
                    .await;
                    if result.is_err() {
                        cancel.cancel();
                    }
                    report_pipe(&reply, output.id, result, &reporting).await;
                }
            });
            exchange.close();
            exchange.wait().await;
            // A one-shot call has no page to tell: its caller learns the
            // exit code and the operation's own words from this message.
            let done = outcome.lock().unwrap().take();
            if let Some(Outcome { exit_code, stderr }) = done {
                match wire::encode(&wire::Outbound::ServiceDone { stream_id, exit_code, stderr }) {
                    Ok(message) => {
                        // A disconnected backend no longer waits for the call.
                        let _ = reply.send(message).await;
                    }
                    Err(error) => {
                        host_log::runner(format_args!("service_done encoding failed: {error}"));
                    }
                }
            }
            log.event("ended");
        });
        Ok(())
    }

    pub async fn close(&self) {
        self.cancel.cancel();
        self.streams.close();
        self.streams.wait().await;
    }
}

/// Where one stream's lines go in the Host's log (`runner.md` § Host log):
/// its invocation's standard error under the stream's own source, the
/// runner's words about it under `runner`, both with its conversation.
#[derive(Clone)]
struct StreamLog {
    source: String,
    conversation: String,
}

impl StreamLog {
    fn event(&self, text: &str) {
        host_log::write(
            host_log::RUNNER,
            Some(&self.conversation),
            &format!("{} {text}", self.source),
        );
    }

    fn stderr(&self, line: &str) {
        host_log::write(&self.source, Some(&self.conversation), line);
    }
}

/// Delivers the page's bytes to the invocation as it asks for them, each
/// chunk within the protocol's record limit.
async fn pump_input(
    pipes: PipeClient,
    url: &str,
    mut input: CommandInput,
    mut demanded: mpsc::Receiver<()>,
    cancel: &CancellationToken,
) -> io::Result<()> {
    let result = async {
        let mut body: BoxStream<'static, io::Result<Bytes>> =
            pipes.get(url, cancel.clone()).await?;
        let mut pending = Bytes::new();
        while demanded.recv().await.is_some() {
            if pending.is_empty() {
                match body.next().await {
                    Some(chunk) => pending = chunk?,
                    None => {
                        input.end().map_err(io::Error::other)?;
                        return Ok(());
                    }
                }
            }
            let chunk = pending.split_to(pending.len().min(MAX_RECORD_BYTES));
            input.write(chunk).await.map_err(io::Error::other)?;
        }
        // The invocation's output ended without completing it.
        Err(io::Error::other("service stream invocation ended"))
    };
    tokio::select! {
        _ = cancel.cancelled() => {
            input.cancel();
            Err(io::Error::new(io::ErrorKind::Interrupted, "service stream cancelled"))
        }
        result = result => {
            if result.is_err() {
                input.cancel();
            }
            result
        }
    }
}

/// How an invocation completed: its exit code and the tail of its standard error.
struct Outcome {
    exit_code: u8,
    stderr: String,
}

/// Uploads the invocation's standard output until its completion ends the
/// pipe; input pulls go to the input pump, standard error to the log.
async fn pump_output(
    pipes: PipeClient,
    url: &str,
    mut output: CommandOutput,
    pulls: mpsc::Sender<()>,
    completed: &CancellationToken,
    cancel: &CancellationToken,
    log: &StreamLog,
    outcome: &Mutex<Option<Outcome>>,
) -> io::Result<()> {
    let (sender, receiver) = mpsc::channel::<io::Result<Bytes>>(OUTPUT_QUEUE);
    let records = async move {
        // A byte is at most one UTF-16 unit of the text `service_done` carries.
        let mut tail = TailBuffer::new(wire::SERVICE_STDERR_CHARS);
        let mut lines = LineSplitter::default();
        let stderr = &mut lines;
        let pump = async move {
            loop {
                let record = tokio::select! {
                    _ = cancel.cancelled() => return,
                    record = output.next() => record,
                };
                let item = match record {
                    Ok(Some(Record::Stdout(bytes))) => Ok(bytes),
                    Ok(Some(Record::Stderr(bytes))) => {
                        tail.push(&bytes);
                        for line in stderr.push(&bytes) {
                            log.stderr(&line);
                        }
                        continue;
                    }
                    Ok(Some(Record::InputPull)) => {
                        if pulls.try_send(()).is_err() {
                            let _ = sender
                                .send(Err(io::Error::other(
                                    "overlapping service stream input demands",
                                )))
                                .await;
                            return;
                        }
                        continue;
                    }
                    Ok(Some(Record::Completion(completion))) => {
                        if let Some(error) = &completion.error {
                            log.event(&format!("failed: {}: {}", error.code, error.message));
                        }
                        let stderr = tail.text();
                        *outcome.lock().unwrap() = Some(Outcome {
                            exit_code: completion.exit_code,
                            stderr,
                        });
                        completed.cancel();
                        continue;
                    }
                    // The response ended: complete only after a completion record.
                    Ok(None) if completed.is_cancelled() => return,
                    Ok(None) => Err(io::Error::other(
                        "service stream invocation has no completion",
                    )),
                    Err(error) => Err(io::Error::other(error)),
                };
                let failed = item.is_err();
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    result = sender.send(item) => {
                        if result.is_err() || failed {
                            return;
                        }
                    }
                }
            }
        };
        pump.await;
        // The words a last chunk left without a newline, however the
        // invocation ended.
        if let Some(line) = lines.finish() {
            log.stderr(&line);
        }
    };
    // The upload's end tells the backend the stream is over: only a
    // completed invocation ends it cleanly.
    let finished = completed.clone();
    let body = futures_util::stream::unfold(Some(receiver), move |receiver| {
        let finished = finished.clone();
        async move {
            let mut receiver = receiver?;
            match receiver.recv().await {
                Some(item) => Some((item, Some(receiver))),
                None if finished.is_cancelled() => None,
                None => Some((
                    Err(io::Error::other(
                        "service stream ended before its completion",
                    )),
                    None,
                )),
            }
        }
    });
    let (result, ()) = tokio::join!(pipes.put(url, body, cancel), records);
    result
}

async fn send_error(
    output: &mpsc::Sender<wire::Frame>,
    stream_id: String,
    code: ServiceErrorCode,
    message: String,
    cancel: &CancellationToken,
    log: &StreamLog,
) {
    log.event(&format!("refused ({code}): {message}"));
    match wire::encode(&wire::Outbound::ServiceError {
        stream_id,
        code,
        message,
    }) {
        Ok(message) => {
            tokio::select! {
                _ = cancel.cancelled() => {},
                _ = output.send(message) => {},
            }
        }
        Err(error) => host_log::runner(format_args!("service_error encoding failed: {error}")),
    }
}
