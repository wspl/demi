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
    CommandInput, CommandOutput,
    protocol::{Invocation, MAX_RECORD_BYTES, Record},
};
use futures_util::{StreamExt, stream::BoxStream};
use tokio::sync::mpsc;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::{
    commands::{artifacts::Artifacts, contexts::Contexts, native::Services},
    connection::wire,
    host_log::{self, LineSplitter},
    pipes::PipeClient,
    tasks::report_pipe,
};

/// Output records wait here for the upload, so the invocation backs off
/// behind the output pipe rather than buffering without limit.
const OUTPUT_QUEUE: usize = 4;

pub struct ServiceStreams {
    output: mpsc::Sender<wire::Outbound>,
    pipes: PipeClient,
    services: Arc<Services>,
    artifacts: Arc<Artifacts>,
    target: String,
    draining: CancellationToken,
    /// How many open streams run on each artifact digest: an open stream
    /// keeps its service resident.
    live: Arc<Mutex<HashMap<String, usize>>>,
    /// Refreshed when a stream ends, so its service is reconsidered.
    contexts: Contexts,
    streams: TaskTracker,
    /// Ends every open stream: cancelled by `close` and by the host
    /// connection's shutdown.
    cancel: CancellationToken,
}

impl ServiceStreams {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        output: mpsc::Sender<wire::Outbound>,
        pipes: PipeClient,
        services: Arc<Services>,
        artifacts: Arc<Artifacts>,
        target: String,
        draining: CancellationToken,
        contexts: Contexts,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            output,
            pipes,
            services,
            artifacts,
            target,
            draining,
            live: Arc::new(Mutex::new(HashMap::new())),
            contexts,
            streams: TaskTracker::new(),
            cancel,
        }
    }

    /// The artifacts open streams run from; their services stay resident.
    pub fn retained_artifacts(&self) -> impl Iterator<Item = String> {
        self.live
            .lock()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
    }

    /// Starts one stream; connection cancellation (or `close`) ends every
    /// open stream.
    pub fn handle_open(&self, message: wire::Inbound) -> io::Result<()> {
        let wire::Inbound::ServiceOpen {
            stream_id,
            context,
            package,
            operation,
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
            .get(&self.target)
            .map(|artifact| artifact.sha256.clone());
        let draining = self.draining.clone();
        let live = self.live.clone();
        let contexts = self.contexts.clone();
        let log = StreamLog {
            source: format!("stream:{operation}"),
            conversation: context.conversation.clone(),
        };
        self.streams.spawn(async move {
            if draining.is_cancelled() {
                let message = "the runner is draining for an upgrade".to_owned();
                send_error(&reply, stream_id, "refused", message, &stream, &log).await;
                return;
            }
            if !package.operations.contains(&operation) {
                let message = format!("{} has no operation {operation}", package.id);
                send_error(&reply, stream_id, "unknown_operation", message, &stream, &log).await;
                return;
            }
            // The stream holds its service from the start: acquiring one no
            // job retains must not race its retirement.
            let _live = digest.map(|digest| Live::hold(live, digest, contexts));
            let opened = async {
                let client = services
                    .acquire(&package, resolver, &stream)
                    .await
                    .map_err(|error| error.to_string())?;
                let invocation = Invocation {
                    operation,
                    invocation_id: stream_id.clone(),
                    context,
                    args: serde_json::json!({}),
                    cwd,
                    env: Default::default(),
                    edits: None,
                    json: None,
                };
                client
                    .invoke(&invocation)
                    .await
                    .map_err(|error| error.to_string())
            };
            let (command_input, command_output) = tokio::select! {
                _ = stream.cancelled() => return,
                result = opened => match result {
                    Ok(exchange) => exchange,
                    Err(message) => {
                        send_error(&reply, stream_id, "service_failed", message, &stream, &log)
                            .await;
                        return;
                    }
                },
            };
            // No bytes move before the answer.
            match wire::service_opened(stream_id) {
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
            exchange.spawn({
                let reply = reply.clone();
                let cancel = stream.clone();
                let log = log.clone();
                async move {
                    let result = pump_output(
                        pipes,
                        &output.url,
                        command_output,
                        pulls,
                        &completed,
                        &cancel,
                        &log,
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

/// One open stream's claim on its service's artifact.
struct Live {
    live: Arc<Mutex<HashMap<String, usize>>>,
    digest: String,
    contexts: Contexts,
}

impl Live {
    fn hold(live: Arc<Mutex<HashMap<String, usize>>>, digest: String, contexts: Contexts) -> Self {
        *live.lock().unwrap().entry(digest.clone()).or_default() += 1;
        Self {
            live,
            digest,
            contexts,
        }
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        let mut live = self.live.lock().unwrap();
        if let Some(count) = live.get_mut(&self.digest) {
            *count -= 1;
            if *count == 0 {
                live.remove(&self.digest);
            }
        }
        drop(live);
        self.contexts.refresh();
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
) -> io::Result<()> {
    let (sender, receiver) = mpsc::channel::<io::Result<Bytes>>(OUTPUT_QUEUE);
    let records = async move {
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
                        if let Some(error) = completion.error {
                            log.event(&format!("failed: {}: {}", error.code, error.message));
                        }
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
    output: &mpsc::Sender<wire::Outbound>,
    stream_id: String,
    code: &str,
    message: String,
    cancel: &CancellationToken,
    log: &StreamLog,
) {
    log.event(&format!("refused ({code}): {message}"));
    match wire::service_error(stream_id, code.into(), message) {
        Ok(message) => {
            tokio::select! {
                _ = cancel.cancelled() => {},
                _ = output.send(message) => {},
            }
        }
        Err(error) => host_log::runner(format_args!("service_error encoding failed: {error}")),
    }
}
