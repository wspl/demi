//! User streams the backend opens on a resident native service (`runner.md`
//! § Service streams): invoke the declared operation with the stream's command
//! context, then carry bytes between the invocation and the two pipes the
//! request named.

use std::{
    collections::HashMap,
    io,
    sync::{Arc, Mutex},
};

use bytes::Bytes;
use demi_command_service::{
    Exchange, ExchangeError, InputSource, OutputSink,
    protocol::{Invocation, MAX_RECORD_BYTES},
};
use futures_util::{StreamExt, stream::BoxStream};
use tokio::sync::mpsc;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::{
    commands::artifacts::StreamArtifacts,
    connection::{
        ConnectionHandle,
        wire::{self, ServiceErrorCode},
    },
    host_log::LineSplitter,
    pipes::PipeClient,
    services::{ServiceHandle, ServiceLease},
    tail::TailBuffer,
    tasks::report_pipe,
};

/// Output records wait here for the upload, so the invocation backs off
/// behind the output pipe rather than buffering without limit.
const OUTPUT_QUEUE: usize = 4;

pub struct ServiceStreams {
    connection: ConnectionHandle,
    pipes: PipeClient,
    services: ServiceHandle,
    bindings: Bindings,
    draining: CancellationToken,
    streams: TaskTracker,
    /// Ends every open stream: cancelled by `close` and by the host
    /// connection's shutdown.
    cancel: CancellationToken,
}

impl ServiceStreams {
    pub fn new(
        connection: ConnectionHandle,
        pipes: PipeClient,
        services: ServiceHandle,
        draining: CancellationToken,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            connection,
            pipes,
            services,
            bindings: Bindings::default(),
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
        let reply = self.connection.control.clone();
        let reporting = self.cancel.clone();
        let stream = self.cancel.child_token();
        let services = self.services.clone();
        let resolver = Arc::new(StreamArtifacts::new(
            self.connection.clone(),
            stream_id.clone(),
        ));
        let digest = package
            .targets
            .get(crate::services::target())
            .map(|artifact| artifact.sha256.clone());
        // The connection keeps the release its streams bound last
        // (`Bindings`). The binding moves here, in the order the streams
        // arrive, and the stream that moved it takes the new lease first,
        // also when it is refused below, so no binding stays without one.
        let moved = digest
            .as_ref()
            .is_some_and(|digest| self.bindings.bind(&package.id, digest));
        let bindings = self.bindings.clone();
        let draining = self.draining.clone();
        let log = StreamLog {
            source: format!("stream:{operation}"),
            conversation: context.conversation.clone(),
        };
        self.streams.spawn(async move {
            if moved && let Some(digest) = &digest {
                let lease = services.lease(digest.clone()).await;
                bindings.hold(&package.id, digest, lease);
            }
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
            let _lease = match digest {
                Some(digest) => Some(services.lease(digest).await),
                None => None,
            };
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
                    tracing::warn!("service_opened encoding failed: {error}");
                    return;
                }
            }
            log.event("opened");
            // Input pipe → the invocation, one chunk per pull; its standard
            // output → the output pipe, which only its completion ends cleanly.
            let (uploads, uploaded) = mpsc::channel::<io::Result<Bytes>>(OUTPUT_QUEUE);
            let mut source = PipeSource {
                pipes: pipes.clone(),
                url: input.url.clone(),
                cancel: stream.clone(),
                body: None,
                pending: Bytes::new(),
            };
            let mut sink = StreamSink {
                uploads,
                log: log.clone(),
                lines: LineSplitter::default(),
                // A byte is at most one UTF-16 unit of the text `service_done` carries.
                tail: TailBuffer::new(wire::SERVICE_STDERR_CHARS),
            };
            let cancelled = stream.clone();
            let exchange = async move {
                let exchange = Exchange::new(command_input, command_output);
                let result = tokio::select! {
                    // Dropping the exchange resets its invocation.
                    _ = cancelled.cancelled() => Err(None),
                    result = exchange.run(&mut source, &mut sink) => result.map_err(Some),
                };
                if result.is_err() {
                    // The upload must not end as if the invocation completed.
                    let ended = io::Error::other("service stream ended before its completion");
                    let _closed = sink.uploads.send(Err(ended)).await;
                }
                (result, sink.finish())
            };
            let body = futures_util::stream::unfold(uploaded, |mut uploaded| async move {
                uploaded.recv().await.map(|item| (item, uploaded))
            });
            let ((result, stderr), upload) =
                tokio::join!(exchange, pipes.put(&output.url, body, &stream));
            let (done, input_result) = match result {
                Ok(completion) => {
                    if let Some(error) = &completion.error {
                        log.event(&format!("failed: {}: {}", error.code, error.message));
                    }
                    let outcome = Outcome {
                        exit_code: completion.exit_code,
                        stderr,
                    };
                    (Some(outcome), Ok(()))
                }
                Err(Some(ExchangeError::Input(error))) => (None, Err(error)),
                Err(Some(ExchangeError::Service(error))) => {
                    log.event(&format!("failed: {error}"));
                    (None, Err(io::Error::other("the service stream's invocation failed")))
                }
                Err(Some(ExchangeError::Output(_)) | None) => (
                    None,
                    Err(io::Error::new(io::ErrorKind::Interrupted, "service stream cancelled")),
                ),
            };
            if upload.is_err() || input_result.is_err() {
                stream.cancel();
            }
            report_pipe(&reply, input.id, input_result, &reporting).await;
            report_pipe(&reply, output.id, upload, &reporting).await;
            // A one-shot call has no page to tell: its caller learns the
            // exit code and the operation's own words from this message.
            if let Some(Outcome { exit_code, stderr }) = done {
                match wire::encode(&wire::Outbound::ServiceDone { stream_id, exit_code, stderr }) {
                    Ok(message) => {
                        // A disconnected backend no longer waits for the call.
                        let _ = reply.send(message).await;
                    }
                    Err(error) => {
                        tracing::warn!("service_done encoding failed: {error}");
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
        self.bindings.clear();
    }
}

/// The package releases the connection's streams bound last, one per
/// package, each with the lease that keeps its service resident until the
/// connection ends (`native-runtime.md` § Keep a service resident), so
/// consecutive user calls reuse the service as consecutive jobs do.
#[derive(Clone, Default)]
struct Bindings(Arc<Mutex<HashMap<String, Bound>>>);

/// The release a package is bound to, by its artifact for this host.
struct Bound {
    digest: String,
    /// Empty until the stream that bound the release has taken its lease.
    lease: Option<ServiceLease>,
}

impl Bindings {
    /// Binds `package` to the release whose artifact is `digest`, and says
    /// whether that moved the binding. A move ends the lease on the release
    /// bound before; the caller then takes the new lease and hands it to
    /// `hold`.
    fn bind(&self, package: &str, digest: &str) -> bool {
        let mut bound = self.0.lock().expect("the bindings are intact");
        if bound.get(package).is_some_and(|bound| bound.digest == digest) {
            return false;
        }
        bound.insert(
            package.to_owned(),
            Bound {
                digest: digest.to_owned(),
                lease: None,
            },
        );
        true
    }

    /// Keeps `lease` for `package` when the package is still bound to the
    /// release of `digest` and that binding holds no lease yet. Otherwise
    /// the lease ends here: another release was bound meanwhile, the same
    /// release was bound again and holds one already, or the connection has
    /// ended.
    fn hold(&self, package: &str, digest: &str, lease: ServiceLease) {
        let mut bound = self.0.lock().expect("the bindings are intact");
        if let Some(bound) = bound.get_mut(package)
            && bound.digest == digest
            && bound.lease.is_none()
        {
            bound.lease = Some(lease);
        }
    }

    /// Ends every binding and its lease, with the connection.
    fn clear(&self) {
        self.0.lock().expect("the bindings are intact").clear();
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
        tracing::info!(conversation = self.conversation.as_str(), "{} {text}", self.source);
    }

    fn stderr(&self, line: &str) {
        tracing::info!(
            source = self.source.as_str(),
            conversation = self.conversation.as_str(),
            "{line}"
        );
    }
}

/// The page's bytes as the invocation asks for them, each chunk within the
/// protocol's record limit; the pipe opens at the first pull.
struct PipeSource {
    pipes: PipeClient,
    url: String,
    cancel: CancellationToken,
    body: Option<BoxStream<'static, io::Result<Bytes>>>,
    pending: Bytes,
}

impl InputSource for PipeSource {
    type Error = io::Error;

    async fn next(&mut self) -> io::Result<Option<Bytes>> {
        let body = match &mut self.body {
            Some(body) => body,
            None => self
                .body
                .insert(self.pipes.get(&self.url, self.cancel.clone()).await?),
        };
        while self.pending.is_empty() {
            match body.next().await {
                Some(chunk) => self.pending = chunk?,
                None => return Ok(None),
            }
        }
        Ok(Some(
            self.pending
                .split_to(self.pending.len().min(MAX_RECORD_BYTES)),
        ))
    }
}

/// How an invocation completed: its exit code and the tail of its standard error.
struct Outcome {
    exit_code: u8,
    stderr: String,
}

/// Standard output goes up the output pipe; standard error goes to the log
/// line by line, and its end is kept for `service_done`.
struct StreamSink {
    uploads: mpsc::Sender<io::Result<Bytes>>,
    log: StreamLog,
    lines: LineSplitter,
    tail: TailBuffer,
}

impl StreamSink {
    /// Logs what a last chunk left without a newline, however the invocation
    /// ended, and returns the end of standard error. Dropping the uploads
    /// ends the output pipe.
    fn finish(self) -> String {
        if let Some(line) = self.lines.finish() {
            self.log.stderr(&line);
        }
        self.tail.text()
    }
}

impl OutputSink for StreamSink {
    type Error = io::Error;

    async fn stdout(&mut self, bytes: Bytes) -> io::Result<()> {
        self.uploads
            .send(Ok(bytes))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "the output pipe closed"))
    }

    async fn stderr(&mut self, bytes: Bytes) -> io::Result<()> {
        self.tail.push(&bytes);
        for line in self.lines.push(&bytes) {
            self.log.stderr(&line);
        }
        Ok(())
    }
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
        Err(error) => tracing::warn!("service_error encoding failed: {error}"),
    }
}
