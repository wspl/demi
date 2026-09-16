use std::{collections::HashSet, future::Future, pin::Pin, sync::Arc, time::Duration};

use bytes::Bytes;
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

use crate::protocol::{
    CONVERSATION_PATH, CommandError, Completion, ConversationRequest, INFO_PATH, INVOKE_PATH,
    Invocation, MAX_INVOCATIONS, MAX_METADATA_BYTES, MAX_RECORD_BYTES, Record, SHUTDOWN_PATH,
    ServiceInfo, VERSION,
};
use crate::{Input, ServiceError, stream::send_bytes};

const OUTPUT_QUEUE_RECORDS: usize = 4;
const METADATA_TIMEOUT: Duration = Duration::from_secs(10);
const CANCEL_TIMEOUT: Duration = Duration::from_secs(5);

pub struct InvocationContext {
    pub request: Invocation,
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
    fn operations(&self) -> Vec<String>;
    fn invoke(
        &self,
        context: InvocationContext,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>>;

    /// Trusted parent-only lifecycle calls, never declared CLI operations.
    fn conversation(
        &self,
        context: ConversationContext,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>> {
        Box::pin(async move {
            let body = if context.request.operation == "status" {
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
}

#[derive(Clone)]
pub struct Output {
    sender: mpsc::Sender<Record>,
    cancellation: CancellationToken,
}

impl Output {
    pub fn channel(cancellation: CancellationToken) -> (Self, mpsc::Receiver<Record>) {
        let (sender, receiver) = mpsc::channel(OUTPUT_QUEUE_RECORDS);
        (
            Self {
                sender,
                cancellation,
            },
            receiver,
        )
    }

    pub(crate) async fn pull(&self) -> Result<(), ServiceError> {
        tokio::select! {
            biased;
            _ = self.cancellation.cancelled() => Err(ServiceError::Cancelled),
            result = self.sender.send(Record::InputPull) => result.map_err(|_| ServiceError::Cancelled),
        }
    }

    pub async fn stdout(&self, bytes: Bytes) -> Result<(), ServiceError> {
        self.write(bytes, Record::Stdout).await
    }

    pub async fn stderr(&self, bytes: Bytes) -> Result<(), ServiceError> {
        self.write(bytes, Record::Stderr).await
    }

    async fn write(
        &self,
        mut bytes: Bytes,
        record: fn(Bytes) -> Record,
    ) -> Result<(), ServiceError> {
        while !bytes.is_empty() {
            let count = bytes.len().min(MAX_RECORD_BYTES);
            let item = record(bytes.split_to(count));
            tokio::select! {
                biased;
                _ = self.cancellation.cancelled() => return Err(ServiceError::Cancelled),
                result = self.sender.send(item) => {
                    result.map_err(|_| ServiceError::Cancelled)?;
                }
            }
        }
        Ok(())
    }
}

/// Serves one parent-owned connection and joins cooperative invocation tasks.
/// A cancellation deadline fault requires the process owner to retire the service.
pub async fn serve<T>(io: T, handler: Arc<dyn Handler>) -> Result<(), ServiceError>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    serve_cancellable(io, handler, CancellationToken::new()).await
}

/// Stops accepting requests on owner cancellation, then cancels and joins handlers.
pub async fn serve_cancellable<T>(
    io: T,
    handler: Arc<dyn Handler>,
    owner: CancellationToken,
) -> Result<(), ServiceError>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    let operations = handler.operations();
    let unique: HashSet<_> = operations.iter().collect();
    if operations.is_empty()
        || operations.iter().any(String::is_empty)
        || unique.len() != operations.len()
    {
        return Err(ServiceError::InvalidCatalog);
    }
    let info = Bytes::from(serde_json::to_vec(&ServiceInfo {
        protocol_version: VERSION,
        operations: operations.clone(),
    })?);
    if info.len() > MAX_METADATA_BYTES {
        return Err(ServiceError::InvalidCatalog);
    }
    let mut builder = h2::server::Builder::new();
    builder
        .max_concurrent_streams(MAX_INVOCATIONS as u32)
        .max_header_list_size(16 * 1024)
        .initial_window_size(MAX_RECORD_BYTES as u32)
        .initial_connection_window_size((MAX_RECORD_BYTES * MAX_INVOCATIONS) as u32);
    let mut connection = tokio::select! {
        _ = owner.cancelled() => return Err(ServiceError::Cancelled),
        result = tokio::time::timeout(METADATA_TIMEOUT, builder.handshake(io)) => result.map_err(|_| ServiceError::HandshakeTimeout)??,
    };
    let cancellation = owner.child_token();
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
                            if draining && error.get_io().is_some_and(|io| matches!(
                                io.kind(), std::io::ErrorKind::BrokenPipe
                                    | std::io::ErrorKind::ConnectionReset
                                    | std::io::ErrorKind::UnexpectedEof
                            )) {
                                break Ok(());
                            }
                            break Err(error.into());
                        }
                        None => break Ok(()),
                    };
                    match (request.method(), request.uri().path()) {
                        (&Method::GET, INFO_PATH) if !draining && tasks.len() < MAX_INVOCATIONS => {
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
                            response.send_response(Response::new(()), true)?;
                            connection.graceful_shutdown();
                        }
                        (&Method::POST, INVOKE_PATH | CONVERSATION_PATH) if !draining && tasks.len() < MAX_INVOCATIONS => {
                            let conversation = request.uri().path() == CONVERSATION_PATH;
                            tasks.spawn(invoke(
                                request, response, handler.clone(), operations.clone(),
                                cancellation.child_token(), conversation,
                            ));
                        }
                        _ => {
                            let status = if draining || tasks.len() >= MAX_INVOCATIONS {
                                StatusCode::SERVICE_UNAVAILABLE
                            } else {
                                StatusCode::NOT_FOUND
                            };
                            reject(&mut response, status)?;
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

async fn invoke(
    request: Request<h2::RecvStream>,
    mut response: SendResponse<Bytes>,
    handler: Arc<dyn Handler>,
    operations: Vec<String>,
    cancellation: CancellationToken,
    conversation: bool,
) -> Result<(), ServiceError> {
    let _cancel_on_drop = cancellation.drop_guard_ref();
    let mut input = Input::new(request.into_body());
    enum Call {
        Invocation(Box<Invocation>),
        Conversation(ConversationRequest),
    }
    let metadata = async {
        if conversation {
            let request: ConversationRequest = input.metadata().await?;
            request.validate()?;
            Ok::<_, ServiceError>(Call::Conversation(request))
        } else {
            let request: Invocation = input.metadata().await?;
            request.validate()?;
            Ok(Call::Invocation(Box::new(request)))
        }
    };
    let metadata = tokio::select! {
        _ = cancellation.cancelled() => return Err(ServiceError::Cancelled),
        result = tokio::time::timeout(METADATA_TIMEOUT, metadata) => result,
    };
    let call = match metadata {
        Ok(Ok(value)) => value,
        _ => return reject(&mut response, StatusCode::BAD_REQUEST),
    };
    if let Call::Invocation(request) = &call
        && !operations.contains(&request.operation)
    {
        return reject(&mut response, StatusCode::NOT_FOUND);
    }
    let release = matches!(&call, Call::Conversation(request) if request.operation == "release");
    let mut stream = response.send_response(Response::new(()), false)?;
    let (output, mut receiver) = Output::channel(cancellation.clone());
    let work = match call {
        Call::Conversation(request) => handler.conversation(ConversationContext {
            request,
            output,
            cancellation: cancellation.clone(),
        }),
        Call::Invocation(request) => {
            input.set_output(output.clone());
            handler.invoke(InvocationContext {
                request: *request,
                input,
                output,
                cancellation: cancellation.clone(),
            })
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
        Err(error) => return Err(ServiceError::Handler(error.to_string())),
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
