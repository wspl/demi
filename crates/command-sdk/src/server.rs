use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

use bytes::Bytes;
use demi_command_protocol::{
    ARTIFACTS_PATH, CONVERSATION_PATH, CommandError, Completion, ConversationRequest, INFO_PATH,
    INVOKE_PATH, Invocation, MAX_MEDIUM_BYTES, MAX_METADATA_BYTES, MAX_RECORD_BYTES, Metadata,
    NUMBERS_PATH,
    ProtocolError, Record, SHUTDOWN_PATH, ServiceInfo, StreamOpen, VERSION,
};
use futures_util::future::poll_fn;
use h2::{Reason, server::SendResponse};
use http::{Method, Request, Response, StatusCode};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::mpsc,
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;
use tokio_util::task::AbortOnDropHandle;

use crate::{
    Input, ServiceError,
    artifacts::{Artifacts, ArtifactsAsk},
    asking::{self, Asked, Pending},
    numbers::{Numbers, NumbersAsk},
    stream::{CONNECTION_WINDOW, HttpInput, send_bytes},
};

const OUTPUT_QUEUE_RECORDS: usize = 4;
const METADATA_TIMEOUT: Duration = Duration::from_secs(10);
const CANCEL_TIMEOUT: Duration = Duration::from_secs(5);

pub struct InvocationContext<M = Invocation> {
    pub request: M,
    pub input: Input,
    pub output: Output,
    pub cancellation: CancellationToken,
}

/// Trusted conversation lifecycle metadata has no command stdin or script environment.
pub struct ConversationContext {
    pub request: ConversationRequest,
    pub output: Output,
    pub cancellation: CancellationToken,
}

/// Handlers own per-invocation state and must cooperate with cancellation.
/// Blocking and CPU work must run away from the connection's async worker.
pub trait Handler: Send + Sync + 'static {
    /// What opens each invocation: [`Invocation`] for a native service.
    type Metadata: Metadata;

    fn operations(&self) -> Vec<String>;
    fn invoke(
        &self,
        context: InvocationContext<Self::Metadata>,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>>;

    /// Trusted parent-only lifecycle calls, never declared CLI operations.
    fn conversation(
        &self,
        context: ConversationContext,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>> {
        Box::pin(async move {
            let body = if context.request == (ConversationRequest::Status {}) {
                Bytes::from_static(b"{\"conversations\":[]}")
            } else {
                Bytes::from_static(b"{}")
            };
            context.output.stdout(body).await?;
            Ok(Completion {
                exit_code: 0,
                error: None,
            })
        })
    }

    /// Invocations have stopped before the service releases conversation state.
    fn close(&self) -> Pin<Box<dyn Future<Output = Result<(), ServiceError>> + Send>> {
        Box::pin(async { Ok(()) })
    }

    /// Takes the connection's numbers source before the first call
    /// (`native-runtime.md` § Conversation numbers); the runner's numbers
    /// stream answers its draws.
    fn numbers(&self, numbers: Numbers) {
        // A service that names nothing with conversation numbers draws none.
        drop(numbers);
    }

    /// Takes the connection's artifacts source before the first call
    /// (`native-runtime.md` § The artifacts stream); the runner's artifacts
    /// stream answers its requests.
    fn artifacts(&self, artifacts: Artifacts) {
        // A service that needs nothing beside its executable asks for none.
        drop(artifacts);
    }
}

/// What serves one of the streams the runner answers, once it opens.
type Relay = Box<
    dyn FnOnce(
            Input,
            Output,
            CancellationToken,
        ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>>
        + Send,
>;

/// The relay of a stream of kind `A`, whose requests arrive at `asks`.
fn relay_of<A: Asked>(asks: mpsc::Receiver<Pending<A>>) -> Relay {
    Box::new(move |input, output, finish| Box::pin(asking::relay(asks, input, output, finish)))
}

/// An invocation's writer: its records, in the order its handler wrote
/// them. A medium's records are written in one turn, so no other record of
/// the invocation comes between them (`native-runtime.md` § Response
/// records and completion).
#[derive(Clone)]
pub struct Output {
    sender: mpsc::Sender<Record>,
    cancellation: CancellationToken,
    /// Whether the invocation may return media: only one a job's command
    /// makes does.
    media: bool,
    turn: Arc<tokio::sync::Mutex<()>>,
}

/// Why a writer refused a medium; the handler learns it from the write.
#[derive(Debug, thiserror::Error)]
pub enum MediumRefused {
    #[error("this invocation cannot return media: only a job's command can")]
    NotAJobCommand,
    #[error("a medium is at most {MAX_MEDIUM_BYTES} bytes; this one is {0}")]
    TooLarge(usize),
}

impl Output {
    /// A writer whose records arrive at the returned receiver; it refuses
    /// media.
    pub fn channel(cancellation: CancellationToken) -> (Self, mpsc::Receiver<Record>) {
        let (sender, receiver) = mpsc::channel(OUTPUT_QUEUE_RECORDS);
        (
            Self {
                sender,
                cancellation,
                media: false,
                turn: Arc::default(),
            },
            receiver,
        )
    }

    /// This writer, accepting media, for an invocation a job's command
    /// makes.
    pub fn returning_media(mut self) -> Self {
        self.media = true;
        self
    }

    pub(crate) async fn pull(&self) -> Result<(), ServiceError> {
        let _turn = self.turn.lock().await;
        self.send(Record::InputPull).await
    }

    pub async fn stdout(&self, bytes: Bytes) -> Result<(), ServiceError> {
        self.write(bytes, Record::Stdout).await
    }

    pub async fn stderr(&self, bytes: Bytes) -> Result<(), ServiceError> {
        self.write(bytes, Record::Stderr).await
    }

    /// Returns `bytes` as a medium (`commands.md` § Return media): the
    /// runner sends it where the calling process's stdout goes. A writer of
    /// an invocation that is no job command's refuses it, as it refuses a
    /// medium over [`MAX_MEDIUM_BYTES`].
    pub async fn medium(&self, mut bytes: Bytes) -> Result<(), ServiceError> {
        if !self.media {
            return Err(ServiceError::failed(MediumRefused::NotAJobCommand));
        }
        if bytes.len() as u64 > MAX_MEDIUM_BYTES {
            return Err(ServiceError::failed(MediumRefused::TooLarge(bytes.len())));
        }
        let _turn = self.turn.lock().await;
        self.send(Record::Medium {
            size: bytes.len() as u64,
        })
        .await?;
        while !bytes.is_empty() {
            let count = bytes.len().min(MAX_RECORD_BYTES);
            self.send(Record::MediumBytes(bytes.split_to(count))).await?;
        }
        Ok(())
    }

    async fn write(
        &self,
        mut bytes: Bytes,
        record: fn(Bytes) -> Record,
    ) -> Result<(), ServiceError> {
        while !bytes.is_empty() {
            let count = bytes.len().min(MAX_RECORD_BYTES);
            let item = record(bytes.split_to(count));
            let _turn = self.turn.lock().await;
            self.send(item).await?;
        }
        Ok(())
    }

    async fn send(&self, record: Record) -> Result<(), ServiceError> {
        tokio::select! {
            biased;
            _ = self.cancellation.cancelled() => Err(ServiceError::Cancelled),
            result = self.sender.send(record) => result.map_err(|_| ServiceError::Cancelled),
        }
    }
}

/// Serves one parent-owned connection and joins cooperative invocation tasks.
/// A cancellation deadline fault requires the process owner to retire the service.
pub async fn serve<T, H>(io: T, handler: Arc<H>) -> Result<(), ServiceError>
where
    T: AsyncRead + AsyncWrite + Unpin,
    H: Handler + ?Sized,
{
    serve_cancellable(io, handler, CancellationToken::new()).await
}

/// Stops accepting requests on owner cancellation, then cancels and joins handlers.
pub async fn serve_cancellable<T, H>(
    io: T,
    handler: Arc<H>,
    owner: CancellationToken,
) -> Result<(), ServiceError>
where
    T: AsyncRead + AsyncWrite + Unpin,
    H: Handler + ?Sized,
{
    let catalog = ServiceInfo {
        protocol_version: VERSION,
        operations: handler.operations(),
    };
    catalog.validate()?;
    let info = Bytes::from(serde_json::to_vec(&catalog)?);
    if info.len() > MAX_METADATA_BYTES {
        return Err(ProtocolError::TooLarge.into());
    }
    let operations = catalog.operations;
    // Invocations are not counted (`native-runtime.md` § Validation and flow
    // control). The runner may abandon any number of requests it just sent,
    // which h2's guard against a flood of resets would otherwise answer by
    // closing the connection.
    let mut builder = h2::server::Builder::new();
    builder
        .max_header_list_size(16 * 1024)
        .initial_window_size(MAX_RECORD_BYTES as u32)
        .initial_connection_window_size(CONNECTION_WINDOW)
        .max_pending_accept_reset_streams(usize::MAX);
    let mut connection = tokio::select! {
        _ = owner.cancelled() => return Err(ServiceError::Cancelled),
        result = tokio::time::timeout(METADATA_TIMEOUT, builder.handshake(io)) => result.map_err(|_| ServiceError::HandshakeTimeout)??,
    };
    let cancellation = owner.child_token();
    let (numbers, draws) = Numbers::channel();
    handler.numbers(numbers);
    let (artifacts, requests) = Artifacts::channel();
    handler.artifacts(artifacts);
    // The runner opens each stream once. Shutdown finishes them, since they
    // would otherwise hold the draining connection open.
    let mut numbers_relay = Some(relay_of::<NumbersAsk>(draws));
    let mut artifacts_relay = Some(relay_of::<ArtifactsAsk>(requests));
    let finish_streams = CancellationToken::new();
    let mut tasks = JoinSet::new();
    let mut draining = false;
    let _cancel_on_drop = cancellation.drop_guard_ref();
    let mut outcome = async {
        loop {
            tokio::select! {
                _ = owner.cancelled() => break Ok(()),
                incoming = connection.accept() => {
                    let (request, mut response) = match incoming {
                        Some(Ok(pair)) => pair,
                        Some(Err(error)) => {
                            // A peer may close after receiving shutdown/GOAWAY while
                            // h2 still has its final control frames queued. Calls are
                            // cancelled and joined below; no completion is fabricated.
                            if draining && crate::stream::peer_closed(&error) {
                                break Ok(());
                            }
                            break Err(error.into());
                        }
                        None => break Ok(()),
                    };
                    match (request.method(), request.uri().path()) {
                        (&Method::GET, INFO_PATH) if !draining => {
                            let body = info.clone();
                            tasks.spawn(async move {
                                let mut stream = response.send_response(Response::new(()), false)?;
                                send_bytes(&mut stream, body).await?;
                                stream.send_data(Bytes::new(), true)?;
                                Ok::<_, ServiceError>(())
                            });
                        }
                        (&Method::POST, SHUTDOWN_PATH) => {
                            draining = true;
                            finish_streams.cancel();
                            response.send_response(Response::new(()), true)?;
                            connection.graceful_shutdown();
                        }
                        (&Method::POST, INVOKE_PATH | CONVERSATION_PATH) if !draining => {
                            let kind = if request.uri().path() == CONVERSATION_PATH {
                                Kind::Conversation
                            } else {
                                Kind::Invocation
                            };
                            tasks.spawn(invoke(
                                request, response, handler.clone(), operations.clone(),
                                cancellation.child_token(), kind,
                            ));
                        }
                        (&Method::POST, path @ (NUMBERS_PATH | ARTIFACTS_PATH)) if !draining => {
                            // Only the first opens the stream; a later one is
                            // refused once its metadata is read.
                            let relay = if path == NUMBERS_PATH {
                                numbers_relay.take()
                            } else {
                                artifacts_relay.take()
                            };
                            tasks.spawn(invoke(
                                request, response, handler.clone(), operations.clone(),
                                cancellation.child_token(),
                                Kind::Stream { relay, finish: finish_streams.clone() },
                            ));
                        }
                        _ => {
                            let status = if draining {
                                StatusCode::SERVICE_UNAVAILABLE
                            } else {
                                StatusCode::NOT_FOUND
                            };
                            // The caller may already have reset this stream; that
                            // ends its own request, never the connection.
                            let _already_reset = reject(&mut response, status);
                        }
                    }
                }
                finished = tasks.join_next(), if !tasks.is_empty() => {
                    if let Some(Ok(Err(error @ (ServiceError::CancellationDeadline | ServiceError::ConversationCleanup(_))))) = finished {
                        break Err(error);
                    }
                    // Invocation failures are represented by stream reset; other calls remain usable.
                }
            }
        }
    }.await;
    cancellation.cancel();
    drop(connection);
    while let Some(result) = tasks.join_next().await {
        if let Ok(Err(
            error @ (ServiceError::CancellationDeadline | ServiceError::ConversationCleanup(_)),
        )) = result
        {
            outcome = Err(error);
        }
        // Ordinary invocation errors already fail their individual HTTP/2 streams.
    }
    match tokio::time::timeout(CANCEL_TIMEOUT, handler.close()).await {
        Ok(Ok(())) => outcome,
        Ok(Err(error)) => Err(error),
        Err(_) => Err(ServiceError::CancellationDeadline),
    }
}

fn reject(response: &mut SendResponse<Bytes>, status: StatusCode) -> Result<(), ServiceError> {
    let mut headers = Response::new(());
    *headers.status_mut() = status;
    response.send_response(headers, true)?;
    Ok(())
}

/// What a request to run something opens.
enum Kind {
    Invocation,
    Conversation,
    /// A stream the runner answers, served by `relay` until `finish`; none
    /// when the stream is open already.
    Stream {
        relay: Option<Relay>,
        finish: CancellationToken,
    },
}

async fn invoke<H: Handler + ?Sized>(
    request: Request<h2::RecvStream>,
    mut response: SendResponse<Bytes>,
    handler: Arc<H>,
    operations: Vec<String>,
    cancellation: CancellationToken,
    kind: Kind,
) -> Result<(), ServiceError> {
    let _cancel_on_drop = cancellation.drop_guard_ref();
    let mut body = HttpInput::new(request.into_body());
    enum Call<M> {
        Invocation(Box<M>),
        Conversation(ConversationRequest),
        Stream(Relay, CancellationToken),
    }
    let metadata = async {
        match kind {
            Kind::Conversation => {
                let request: ConversationRequest = body.metadata().await?;
                request.validate()?;
                Ok::<_, ServiceError>(Call::Conversation(request))
            }
            Kind::Invocation => {
                let request: H::Metadata = body.metadata().await?;
                request.validate()?;
                Ok(Call::Invocation(Box::new(request)))
            }
            Kind::Stream { relay, finish } => {
                let open: StreamOpen = body.metadata().await?;
                open.validate()?;
                let relay = relay.ok_or(ServiceError::Rejected(StatusCode::CONFLICT.as_u16()))?;
                Ok(Call::Stream(relay, finish))
            }
        }
    };
    let metadata = tokio::select! {
        _ = cancellation.cancelled() => return Err(ServiceError::Cancelled),
        result = tokio::time::timeout(METADATA_TIMEOUT, metadata) => result,
    };
    let call = match metadata {
        Ok(Ok(value)) => value,
        Ok(Err(ServiceError::Rejected(status))) => {
            let status = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST);
            return reject(&mut response, status);
        }
        _ => return reject(&mut response, StatusCode::BAD_REQUEST),
    };
    if let Call::Invocation(request) = &call
        && !operations
            .iter()
            .any(|operation| operation == request.operation())
    {
        return reject(&mut response, StatusCode::NOT_FOUND);
    }
    let release = matches!(
        &call,
        Call::Conversation(ConversationRequest::Release { .. })
    );
    let mut stream = response.send_response(Response::new(()), false)?;
    let (output, mut receiver) = Output::channel(cancellation.clone());
    let work = match call {
        Call::Conversation(request) => handler.conversation(ConversationContext {
            request,
            output,
            cancellation: cancellation.clone(),
        }),
        Call::Invocation(request) => {
            let output = if request.returns_media() {
                output.returning_media()
            } else {
                output
            };
            body.set_output(output.clone());
            handler.invoke(InvocationContext {
                request: *request,
                input: Input::http(body),
                output,
                cancellation: cancellation.clone(),
            })
        }
        Call::Stream(relay, finish) => {
            body.set_output(output.clone());
            relay(Input::http(body), output, finish)
        }
    };
    let mut task = AbortOnDropHandle::new(tokio::spawn(work));
    let outcome = loop {
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => break Err(ServiceError::Cancelled),
            _ = poll_fn(|cx| stream.poll_reset(cx)) => break Err(ServiceError::Cancelled),
            record = receiver.recv() => {
                match record {
                    Some(record) => {
                        let encoded = record.encode()?;
                        let sent = tokio::select! {
                            _ = cancellation.cancelled() => Err(ServiceError::Cancelled),
                            result = send_bytes(&mut stream, encoded) => result,
                        };
                        if let Err(error) = sent {
                            break Err(error);
                        }
                    }
                    None => break Ok(()),
                }
            }
        }
    };
    let completion = if outcome.is_ok() {
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => None,
            _ = poll_fn(|cx| stream.poll_reset(cx)) => None,
            result = &mut task => Some(result),
        }
    } else {
        None
    };
    let Some(completion) = completion else {
        cancellation.cancel();
        receiver.close();
        stream.send_reset(Reason::CANCEL);
        match tokio::time::timeout(CANCEL_TIMEOUT, &mut task).await {
            Err(_) => {
                task.abort();
                // Native work may be non-cooperative; the owner must retire
                // this process rather than treating task abortion as cleanup.
                return Err(ServiceError::CancellationDeadline);
            }
            Ok(Err(error)) if release => {
                return Err(ServiceError::ConversationCleanup(error.to_string()));
            }
            Ok(Ok(Err(error))) if release && !matches!(error, ServiceError::Cancelled) => {
                return Err(ServiceError::ConversationCleanup(error.to_string()));
            }
            Ok(Ok(Ok(completion))) if release && completion.exit_code != 0 => {
                return Err(ServiceError::ConversationCleanup(format!(
                    "release exited with status {}: {:?}",
                    completion.exit_code, completion.error
                )));
            }
            // A cancelled ordinary invocation has no domain release to finish;
            // its result cannot be delivered on the reset stream.
            _ => {}
        }
        return outcome.and(Err(ServiceError::Cancelled));
    };
    let completion = match completion {
        Ok(Ok(completion)) => completion,
        Ok(Err(ServiceError::Cancelled)) => return Err(ServiceError::Cancelled),
        Ok(Err(error)) => Completion {
            exit_code: 1,
            error: Some(CommandError {
                code: "command_failed".into(),
                message: error.to_string(),
            }),
        },
        Err(error) if release => return Err(ServiceError::ConversationCleanup(error.to_string())),
        Err(error) => return Err(ServiceError::Task(error)),
    };
    let cleanup_failure = (release && completion.exit_code != 0).then(|| {
        format!(
            "release exited with status {}: {:?}",
            completion.exit_code, completion.error
        )
    });
    let encoded = Record::Completion(completion).encode()?;
    let sent = async {
        send_bytes(&mut stream, encoded).await?;
        stream.send_data(Bytes::new(), true)?;
        Ok(())
    }
    .await;
    if let Some(error) = cleanup_failure {
        return Err(ServiceError::ConversationCleanup(error));
    }
    sent
}
