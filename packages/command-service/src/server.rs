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
    Completion, INFO_PATH, INVOKE_PATH, Invocation, MAX_METADATA_BYTES, MAX_RECORD_BYTES, Record,
    SHUTDOWN_PATH, ServiceInfo, VERSION,
};
use crate::{Input, ServiceError, stream::send_bytes};

const MAX_INVOCATIONS: usize = 32;
const OUTPUT_QUEUE_RECORDS: usize = 4;
const METADATA_TIMEOUT: Duration = Duration::from_secs(10);
const CANCEL_TIMEOUT: Duration = Duration::from_secs(5);

pub struct InvocationContext {
    pub request: Invocation,
    pub input: Input,
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
}

#[derive(Clone)]
pub struct Output {
    sender: mpsc::Sender<Record>,
    cancellation: CancellationToken,
}

impl Output {
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
    let mut connection = tokio::time::timeout(METADATA_TIMEOUT, builder.handshake(io))
        .await
        .map_err(|_| ServiceError::HandshakeTimeout)??;
    let cancellation = CancellationToken::new();
    let mut tasks = JoinSet::new();
    let mut draining = false;
    let _cancel_on_drop = cancellation.drop_guard_ref();
    let mut outcome = async {
        loop {
            tokio::select! {
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
                        (&Method::POST, INVOKE_PATH) if !draining && tasks.len() < MAX_INVOCATIONS => {
                            tasks.spawn(invoke(
                                request, response, handler.clone(), operations.clone(),
                                cancellation.child_token(),
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
                    if matches!(finished, Some(Ok(Err(ServiceError::CancellationDeadline)))) {
                        break Err(ServiceError::CancellationDeadline);
                    }
                    // Invocation failures are represented by stream reset; other calls remain usable.
                }
            }
        }
    }.await;
    cancellation.cancel();
    drop(connection);
    while let Some(result) = tasks.join_next().await {
        if matches!(result, Ok(Err(ServiceError::CancellationDeadline))) {
            outcome = Err(ServiceError::CancellationDeadline);
        }
        // Ordinary invocation errors already fail their individual HTTP/2 streams.
    }
    outcome
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
) -> Result<(), ServiceError> {
    let _cancel_on_drop = cancellation.drop_guard_ref();
    let mut input = Input::new(request.into_body());
    let metadata = tokio::select! {
        _ = cancellation.cancelled() => return Err(ServiceError::Cancelled),
        result = tokio::time::timeout(METADATA_TIMEOUT, input.invocation()) => result,
    };
    let invocation = match metadata {
        Ok(Ok(value)) => value,
        _ => return reject(&mut response, StatusCode::BAD_REQUEST),
    };
    if !operations.contains(&invocation.operation) {
        return reject(&mut response, StatusCode::NOT_FOUND);
    }
    let mut stream = response.send_response(Response::new(()), false)?;
    let (sender, mut receiver) = mpsc::channel(OUTPUT_QUEUE_RECORDS);
    let output = Output {
        sender,
        cancellation: cancellation.clone(),
    };
    let context = InvocationContext {
        request: invocation,
        input,
        output,
        cancellation: cancellation.clone(),
    };
    let mut task = AbortOnDropHandle::new(tokio::spawn(handler.invoke(context)));
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
        if tokio::time::timeout(CANCEL_TIMEOUT, &mut task)
            .await
            .is_err()
        {
            task.abort();
            // Rust cannot stop non-cooperative native code. Report a fatal service
            // fault so the process owner retires it; do not wait forever or claim
            // that aborting its async task proves the native work has stopped.
            return Err(ServiceError::CancellationDeadline);
        }
        return outcome.and(Err(ServiceError::Cancelled));
    };
    let completion = completion.map_err(|error| ServiceError::Handler(error.to_string()))??;
    let encoded = Record::Completion(completion).encode()?;
    send_bytes(&mut stream, encoded).await?;
    stream.send_data(Bytes::new(), true)?;
    Ok(())
}
