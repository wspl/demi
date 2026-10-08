//! The `preview` stream (`preview.md` § The stream): the relay's requests and
//! WebSockets in, the engine's answers out, over one byte stream with the
//! live view's framing. Several requests share the stream, each with the id
//! the relay chose. A request's body moves one chunk per pull; an answer's
//! goes [`BODY_WINDOW`] chunks ahead of the relay's pulls, so a small
//! answer takes one round trip, and a slow page holds the Host back rather
//! than filling memory. Page states move
//! over it too, between the user's browser and the conversation's
//! ([`PageStates`]).

use std::collections::HashMap;
use std::sync::Arc;

use bytes::{Buf, BufMut, Bytes, BytesMut};
use demi_command_package_browser_protocol::preview::{
    BODY_CHUNK_BYTES, BODY_WINDOW, BodyHeader, CHUNK_FRAME, CONTROL_FRAME, MAX_FRAME_BYTES, MAX_STORAGE_BYTES,
    PageStorage, PreviewClient, PreviewEngineMessage, PreviewEnvironment, PreviewHeader,
    PreviewRelayMessage, PreviewRequest, REQUEST_BODY_FRAME, SOCKET_MESSAGE_FRAME, SocketHeader,
};
use futures_util::future::BoxFuture;
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use tokio::sync::{Semaphore, mpsc};
use tokio::task::{AbortHandle, JoinSet};
use tokio_util::sync::CancellationToken;
use wreq::ws::message::{CloseCode, CloseFrame, Message, Utf8Bytes};

use crate::engine::{AnswerBody, Engine, Fetch, Place, environment_of, preview_environment};
use crate::socket;

/// How many frames wait for the output at most; a slow page stops the
/// requests that write.
const OUTPUT_FRAMES: usize = 16;
/// How many of the page's messages wait for one upstream socket.
const SOCKET_MESSAGES: usize = 64;
/// The close code of a socket that ended without a close frame.
const ABNORMAL_CLOSURE: u16 = 1006;
/// The close code of a close frame without one.
const NO_STATUS: u16 = 1005;

/// Why a stream ended other than by its relay or its cancellation.
#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    /// The relay sent a frame the protocol refuses (`live-view.md` § Framing
    /// and versions): a defect of the page, which ends the stream.
    #[error("invalid preview frame: {0}")]
    Protocol(String),
    #[error("the preview stream's input failed: {0}")]
    Input(std::io::Error),
    #[error("the preview stream's output failed: {0}")]
    Output(std::io::Error),
}

/// A page of the agent's browser, as `state_take` moves it: its address,
/// title, and top-level origin's storage.
#[derive(Debug, Clone, PartialEq)]
pub struct TakenState {
    pub url: String,
    pub title: String,
    /// The tab shows the page in Mobile.
    pub mobile: bool,
    pub storage: PageStorage,
}

/// The conversation's browser, as the stream moves page states between it
/// and the user's browser (`preview.md` § Page state); the program that
/// composes the engine provides it for the stream's conversation.
pub trait PageStates: Send + Sync {
    /// Moves the cookies of the agent's tab `tab` into the jar, and answers
    /// its page; why not, as a sentence, when it cannot.
    fn take(&self, tab: String) -> BoxFuture<'static, Result<TakenState, String>>;
    /// Keeps the jar's cookies of `sites` and `storage` under `token`, for the
    /// tab of the agent's browser `browser.handover` opens with it.
    fn keep(&self, token: String, sites: Vec<String>, storage: Option<PageStorage>);
}

/// Serves one `preview` stream: the relay's frames from `input`, the
/// engine's to `output`, until the relay ends its side or `cancel` is
/// cancelled. Every request and socket of the stream ends with it.
pub async fn serve<I, O>(
    engine: Arc<Engine>,
    states: Arc<dyn PageStates>,
    input: I,
    output: O,
    cancel: CancellationToken,
) -> Result<(), StreamError>
where
    I: Stream<Item = std::io::Result<Bytes>> + Send + Unpin,
    O: Sink<Bytes, Error = std::io::Error> + Send + Unpin + 'static,
{
    let (frames, queued) = mpsc::channel(OUTPUT_FRAMES);
    let mut writing = tokio::spawn(write(queued, output));
    let mut stream = Served {
        engine,
        states,
        place: None,
        output: frames,
        work: JoinSet::new(),
        requests: HashMap::new(),
        sockets: HashMap::new(),
        generation: 0,
    };
    let mut reader = Reader {
        input,
        pending: BytesMut::new(),
    };
    let result = loop {
        tokio::select! {
            () = cancel.cancelled() => break Ok(()),
            written = &mut writing => break match written {
                Ok(result) => result.map_err(StreamError::Output),
                Err(joined) => Err(StreamError::Output(std::io::Error::other(joined))),
            },
            Some(ended) = stream.work.join_next() => {
                // A task that panicked or was aborted was removed already.
                if let Ok(ended) = ended {
                    stream.ended(ended);
                }
            }
            frame = reader.next() => match frame {
                Ok(Some(frame)) => {
                    if let Err(error) = stream.receive(frame).await {
                        break Err(error);
                    }
                }
                Ok(None) => break Ok(()),
                Err(error) => break Err(error),
            },
        }
    };
    // The requests and sockets end first, then what they wrote is
    // delivered; a page that no longer reads gives up the rest with the
    // cancellation.
    drop(stream);
    if !writing.is_finished() {
        tokio::select! {
            () = cancel.cancelled() => writing.abort(),
            // The output's failure after the end changes nothing.
            _ = &mut writing => {}
        }
    }
    result
}

/// Writes the queued frames to the output, then ends it.
async fn write<O>(mut queued: mpsc::Receiver<Bytes>, mut output: O) -> std::io::Result<()>
where
    O: Sink<Bytes, Error = std::io::Error> + Unpin,
{
    while let Some(frame) = queued.recv().await {
        output.send(frame).await?;
    }
    output.close().await
}

/// A frame of the relay.
enum Inbound {
    Control(PreviewRelayMessage),
    RequestBody { id: u32, data: Bytes },
    SocketMessage { id: u32, binary: bool, data: Bytes },
}

/// Splits the relay's bytes into frames as they arrive.
struct Reader<I> {
    input: I,
    pending: BytesMut,
}

impl<I: Stream<Item = std::io::Result<Bytes>> + Unpin> Reader<I> {
    /// The next frame, or none once the relay ended its side.
    async fn next(&mut self) -> Result<Option<Inbound>, StreamError> {
        loop {
            if self.pending.len() >= 4 {
                let length = u32::from_be_bytes(self.pending[..4].try_into().expect("four bytes"));
                let length = usize::try_from(length).unwrap_or(usize::MAX);
                if length == 0 || length > MAX_FRAME_BYTES {
                    return Err(StreamError::Protocol("a frame is empty or too large".into()));
                }
                if self.pending.len() >= 4 + length {
                    self.pending.advance(4);
                    let frame = self.pending.split_to(length).freeze();
                    return decode(frame).map(Some).map_err(StreamError::Protocol);
                }
            }
            match self.input.next().await {
                Some(Ok(chunk)) => self.pending.extend_from_slice(&chunk),
                Some(Err(error)) => return Err(StreamError::Input(error)),
                None if self.pending.is_empty() => return Ok(None),
                None => return Err(StreamError::Protocol("the stream ended inside a frame".into())),
            }
        }
    }
}

fn decode(mut frame: Bytes) -> Result<Inbound, String> {
    match frame.get_u8() {
        CONTROL_FRAME => PreviewRelayMessage::decode(&frame)
            .map(Inbound::Control)
            .map_err(|error| error.to_string()),
        REQUEST_BODY_FRAME => {
            let (header, data) = BodyHeader::split(&frame).map_err(|error| error.to_string())?;
            Ok(Inbound::RequestBody {
                id: header.id,
                data: frame.slice_ref(data),
            })
        }
        SOCKET_MESSAGE_FRAME => {
            let (header, data) = SocketHeader::split(&frame).map_err(|error| error.to_string())?;
            Ok(Inbound::SocketMessage {
                id: header.id,
                binary: header.binary,
                data: frame.slice_ref(data),
            })
        }
        kind => Err(format!("the relay sends no frame of kind {kind}")),
    }
}

fn framed(kind: u8, payload_length: usize, write: impl FnOnce(&mut BytesMut)) -> Bytes {
    let mut bytes = BytesMut::with_capacity(5 + payload_length);
    bytes.put_u32(u32::try_from(1 + payload_length).expect("preview frames are bounded"));
    bytes.put_u8(kind);
    write(&mut bytes);
    bytes.freeze()
}

fn control(message: &PreviewEngineMessage) -> Bytes {
    let json = serde_json::to_vec(message).expect("preview messages serialize");
    framed(CONTROL_FRAME, json.len(), |bytes| bytes.extend_from_slice(&json))
}

fn chunk(id: u32, data: &[u8]) -> Bytes {
    let mut head = Vec::with_capacity(BodyHeader::BYTES);
    BodyHeader { id }.write(&mut head);
    framed(CHUNK_FRAME, head.len() + data.len(), |bytes| {
        bytes.extend_from_slice(&head);
        bytes.extend_from_slice(data);
    })
}

fn socket_message(id: u32, binary: bool, data: &[u8]) -> Bytes {
    let mut head = Vec::with_capacity(SocketHeader::BYTES);
    SocketHeader { id, binary }.write(&mut head);
    framed(SOCKET_MESSAGE_FRAME, head.len() + data.len(), |bytes| {
        bytes.extend_from_slice(&head);
        bytes.extend_from_slice(data);
    })
}

/// A request in progress: the relay's pulls of its body, the chunks of its
/// own body while the engine reads it, and its task.
struct Open {
    generation: u64,
    pulls: Arc<Semaphore>,
    body: Option<mpsc::Sender<Bytes>>,
    task: AbortHandle,
}

/// A socket in progress: the page's messages for it, and its task.
struct Socket {
    generation: u64,
    inbound: mpsc::Sender<FromPage>,
    task: AbortHandle,
}

enum FromPage {
    Message { binary: bool, data: Bytes },
    Close { code: u16, reason: String },
}

/// What a request's or a socket's task answers when it ends; a page
/// state's question leaves nothing to forget.
enum Ended {
    Request { id: u32, generation: u64 },
    Socket { id: u32, generation: u64 },
    Answered,
}

/// One stream's state: where its previews live, and its open requests and
/// sockets. Dropping it ends them.
struct Served {
    engine: Arc<Engine>,
    states: Arc<dyn PageStates>,
    place: Option<Arc<Place>>,
    output: mpsc::Sender<Bytes>,
    work: JoinSet<Ended>,
    requests: HashMap<u32, Open>,
    sockets: HashMap<u32, Socket>,
    generation: u64,
}

fn refused(message: impl Into<String>) -> StreamError {
    StreamError::Protocol(message.into())
}

impl Served {
    fn ended(&mut self, ended: Ended) {
        match ended {
            Ended::Request { id, generation } => {
                if self.requests.get(&id).is_some_and(|open| open.generation == generation) {
                    self.requests.remove(&id);
                }
            }
            Ended::Socket { id, generation } => {
                if self.sockets.get(&id).is_some_and(|socket| socket.generation == generation) {
                    self.sockets.remove(&id);
                }
            }
            Ended::Answered => {}
        }
    }

    fn place(&self) -> Result<Arc<Place>, StreamError> {
        self.place.clone().ok_or_else(|| refused("the relay's first message is not hello"))
    }

    async fn receive(&mut self, frame: Inbound) -> Result<(), StreamError> {
        match frame {
            Inbound::Control(PreviewRelayMessage::Hello {
                scheme,
                domain,
                namespace,
                host,
            }) => {
                if self.place.is_some() {
                    return Err(refused("a second hello"));
                }
                self.place = Some(Arc::new(Place {
                    scheme: scheme.to_string(),
                    domain,
                    namespace,
                    host,
                }));
            }
            Inbound::Control(PreviewRelayMessage::Request {
                id,
                environment,
                request,
                client,
            }) => {
                let place = self.place()?;
                if self.requests.get(&id).is_some_and(|open| !open.task.is_finished()) {
                    return Err(refused(format!("request {id} is open already")));
                }
                self.generation += 1;
                let generation = self.generation;
                // The answer's first chunks go without a pull.
                let pulls = Arc::new(Semaphore::new(BODY_WINDOW));
                let (body, chunks) = if request.body {
                    let (body, chunks) = mpsc::channel(1);
                    (Some(body), Some(chunks))
                } else {
                    (None, None)
                };
                let task = self.work.spawn(answer(
                    self.engine.clone(),
                    place,
                    Requested {
                        id,
                        environment,
                        request,
                        client,
                    },
                    chunks,
                    pulls.clone(),
                    self.output.clone(),
                    generation,
                ));
                self.requests.insert(
                    id,
                    Open {
                        generation,
                        pulls,
                        body,
                        task,
                    },
                );
            }
            Inbound::Control(PreviewRelayMessage::Pull { id }) => {
                self.place()?;
                // A pull of a request that ended meanwhile asks nothing.
                if let Some(open) = self.requests.get(&id) {
                    open.pulls.add_permits(1);
                }
            }
            Inbound::Control(PreviewRelayMessage::Cancel { id }) => {
                self.place()?;
                if let Some(open) = self.requests.remove(&id) {
                    open.task.abort();
                }
            }
            Inbound::RequestBody { id, data } => {
                self.place()?;
                let Some(open) = self.requests.get(&id) else {
                    // The request was cancelled while its chunk was under way.
                    return Ok(());
                };
                let Some(body) = &open.body else {
                    return Err(refused(format!("request {id} has no body")));
                };
                match body.try_send(data) {
                    // The request has read its body, or failed meanwhile.
                    Ok(()) | Err(mpsc::error::TrySendError::Closed(_)) => {}
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        return Err(refused(format!("a chunk of request {id}'s body the engine did not pull")));
                    }
                }
            }
            Inbound::Control(PreviewRelayMessage::SocketOpen {
                id,
                environment,
                url,
                protocols,
                client,
            }) => {
                self.place()?;
                if self.sockets.get(&id).is_some_and(|socket| !socket.task.is_finished()) {
                    return Err(refused(format!("socket {id} is open already")));
                }
                self.generation += 1;
                let generation = self.generation;
                let (inbound, from_page) = mpsc::channel(SOCKET_MESSAGES);
                let task = self.work.spawn(relay_socket(
                    self.engine.clone(),
                    id,
                    environment,
                    url,
                    protocols,
                    client,
                    from_page,
                    self.output.clone(),
                    generation,
                ));
                self.sockets.insert(
                    id,
                    Socket {
                        generation,
                        inbound,
                        task,
                    },
                );
            }
            Inbound::SocketMessage { id, binary, data } => {
                self.place()?;
                if let Some(socket) = self.sockets.get(&id) {
                    // A socket that closed meanwhile drops what reaches it
                    // late, as a closed socket does. Waiting for room holds
                    // the stream while upstream reads slowly.
                    let _late = socket.inbound.send(FromPage::Message { binary, data }).await;
                }
            }
            Inbound::Control(PreviewRelayMessage::Labels { id, environments }) => {
                let place = self.place()?;
                let labels = crate::engine::labels_of(&place, environments);
                // It fails only once the writer ended, which the serving loop notices.
                let _closed = self.output.send(control(&PreviewEngineMessage::Labels { id, labels })).await;
            }
            Inbound::Control(PreviewRelayMessage::StateTake { id, tab }) => {
                self.place()?;
                let taking = self.states.take(tab);
                let output = self.output.clone();
                self.work.spawn(async move {
                    let message = match taking.await {
                        Ok(taken) => taken_state(id, taken),
                        Err(reason) => PreviewEngineMessage::Failed { id, reason },
                    };
                    // It fails only once the writer ended, which the serving loop notices.
                    let _closed = output.send(control(&message)).await;
                    Ended::Answered
                });
            }
            Inbound::Control(PreviewRelayMessage::StateKeep { id, token, sites, storage }) => {
                self.place()?;
                self.states.keep(token, sites, storage);
                // It fails only once the writer ended, which the serving loop notices.
                let _closed = self.output.send(control(&PreviewEngineMessage::StateKept { id })).await;
            }
            Inbound::Control(PreviewRelayMessage::SocketClose { id, code, reason }) => {
                self.place()?;
                if let Some(socket) = self.sockets.get(&id) {
                    let _late = socket.inbound.send(FromPage::Close { code, reason }).await;
                }
            }
        }
        Ok(())
    }
}

/// `state` of a taken page, its storage left out when it is larger than a
/// page state moves.
fn taken_state(id: u32, taken: TakenState) -> PreviewEngineMessage {
    let size = serde_json::to_vec(&taken.storage).map_or(usize::MAX, |bytes| bytes.len());
    let too_large = size > MAX_STORAGE_BYTES;
    PreviewEngineMessage::State {
        id,
        url: taken.url,
        title: taken.title,
        mobile: taken.mobile,
        storage: (!too_large).then_some(taken.storage),
        too_large,
    }
}

/// A request as the relay sent it.
struct Requested {
    id: u32,
    environment: PreviewEnvironment,
    request: PreviewRequest,
    client: PreviewClient,
}

/// Answers one request: reads its body on the engine's pulls, fetches it,
/// and sends its body as far ahead of the relay's pulls as the window
/// lets it.
async fn answer(
    engine: Arc<Engine>,
    place: Arc<Place>,
    requested: Requested,
    chunks: Option<mpsc::Receiver<Bytes>>,
    pulls: Arc<Semaphore>,
    output: mpsc::Sender<Bytes>,
    generation: u64,
) -> Ended {
    let id = requested.id;
    let ended = Ended::Request { id, generation };
    let mut body = BytesMut::new();
    if let Some(mut chunks) = chunks {
        loop {
            if output.send(control(&PreviewEngineMessage::Pull { id })).await.is_err() {
                return ended;
            }
            match chunks.recv().await {
                Some(chunk) if chunk.is_empty() => break,
                Some(chunk) => body.extend_from_slice(&chunk),
                // The stream ended.
                None => return ended,
            }
        }
    }
    let fetch = Fetch {
        environment: environment_of(&requested.environment),
        request: requested.request,
        client: requested.client,
        body: body.freeze(),
    };
    let answer = match engine.fetch(&place, fetch).await {
        Ok(answer) => answer,
        Err(failure) => {
            // The stream's end is the only reason a send fails, and then
            // nobody waits for this answer.
            let _ = output
                .send(control(&PreviewEngineMessage::Failed { id, reason: failure.0 }))
                .await;
            return ended;
        }
    };
    let head = PreviewEngineMessage::Response {
        id,
        status: answer.status,
        headers: answer
            .headers
            .into_iter()
            .map(|(name, value)| PreviewHeader { name, value })
            .collect(),
        labels: answer
            .labels
            .into_iter()
            .map(|(label, environment)| (label, preview_environment(environment)))
            .collect(),
    };
    if output.send(control(&head)).await.is_err() {
        return ended;
    }
    let mut source = Source::of(answer.body);
    loop {
        let Ok(permit) = pulls.acquire().await else {
            return ended;
        };
        permit.forget();
        match source.next().await {
            Ok(data) => {
                let last = data.is_empty();
                if output.send(chunk(id, &data)).await.is_err() || last {
                    return ended;
                }
            }
            Err(reason) => {
                // As above: only the stream's end fails a send.
                let _ = output.send(control(&PreviewEngineMessage::Failed { id, reason })).await;
                return ended;
            }
        }
    }
}

/// An answer's body, one chunk of at most [`BODY_CHUNK_BYTES`] at a time.
struct Source {
    pending: Bytes,
    upstream: Option<std::pin::Pin<Box<dyn Stream<Item = wreq::Result<Bytes>> + Send>>>,
}

impl Source {
    fn of(body: AnswerBody) -> Self {
        match body {
            AnswerBody::Full(bytes) => Self {
                pending: bytes,
                upstream: None,
            },
            AnswerBody::Upstream(response) => Self {
                pending: Bytes::new(),
                upstream: Some(Box::pin(response.bytes_stream())),
            },
        }
    }

    /// The next chunk; an empty one once the body ended.
    async fn next(&mut self) -> Result<Bytes, String> {
        while self.pending.is_empty() {
            let Some(upstream) = &mut self.upstream else {
                return Ok(Bytes::new());
            };
            match upstream.next().await {
                Some(Ok(bytes)) => self.pending = bytes,
                Some(Err(error)) => return Err(error.to_string()),
                None => self.upstream = None,
            }
        }
        let count = self.pending.len().min(BODY_CHUNK_BYTES);
        Ok(self.pending.split_to(count))
    }
}

/// Opens one socket upstream and relays its messages both ways until either
/// side closes it.
async fn relay_socket(
    engine: Arc<Engine>,
    id: u32,
    environment: PreviewEnvironment,
    url: String,
    protocols: Vec<String>,
    client: PreviewClient,
    mut from_page: mpsc::Receiver<FromPage>,
    output: mpsc::Sender<Bytes>,
    generation: u64,
) -> Ended {
    let ended = Ended::Socket { id, generation };
    let closed = |code: u16, reason: String| control(&PreviewEngineMessage::SocketClose { id, code, reason });
    let opened = socket::connect(&engine, &environment_of(&environment), &url, protocols, &client).await;
    let mut upstream = match opened {
        Ok(opened) => {
            let message = PreviewEngineMessage::SocketOpened {
                id,
                protocol: opened.protocol,
                extensions: opened.extensions,
            };
            if output.send(control(&message)).await.is_err() {
                return ended;
            }
            opened.socket
        }
        Err(reason) => {
            tracing::debug!(url, reason, "a preview socket did not open");
            // Only the stream's end fails a send, and then nobody waits.
            let _ = output.send(closed(ABNORMAL_CLOSURE, String::new())).await;
            return ended;
        }
    };
    // The page's close goes upstream; the socket then ends with upstream's
    // answer, as the page's close handshake does.
    let mut closing = false;
    loop {
        tokio::select! {
            message = from_page.recv(), if !closing => {
                let sent = match message {
                    Some(FromPage::Message { binary: true, data }) => upstream.send(Message::Binary(data)).await,
                    Some(FromPage::Message { binary: false, data }) => match Utf8Bytes::try_from(data) {
                        Ok(text) => upstream.send(Message::Text(text)).await,
                        // The frame's UTF-8 was checked when it arrived.
                        Err(_) => continue,
                    },
                    Some(FromPage::Close { code, reason }) => {
                        closing = true;
                        upstream
                            .send(Message::Close(Some(CloseFrame {
                                code: CloseCode::from(code),
                                reason: Utf8Bytes::from(reason),
                            })))
                            .await
                    }
                    // The stream ended.
                    None => return ended,
                };
                if sent.is_err() {
                    let _ = output.send(closed(ABNORMAL_CLOSURE, String::new())).await;
                    return ended;
                }
            }
            message = upstream.next() => match message {
                Some(Ok(Message::Text(text))) => {
                    if output.send(socket_message(id, false, text.as_str().as_bytes())).await.is_err() {
                        return ended;
                    }
                }
                Some(Ok(Message::Binary(data))) => {
                    if output.send(socket_message(id, true, &data)).await.is_err() {
                        return ended;
                    }
                }
                Some(Ok(Message::Close(frame))) => {
                    let (code, reason) = frame.map_or((NO_STATUS, String::new()), |frame| {
                        (u16::from(frame.code), frame.reason.as_str().to_owned())
                    });
                    let _ = output.send(closed(code, reason)).await;
                    return ended;
                }
                // Pings and pongs are each side's own; neither is relayed.
                Some(Ok(_)) => {}
                Some(Err(_)) | None => {
                    let _ = output.send(closed(ABNORMAL_CLOSURE, String::new())).await;
                    return ended;
                }
            },
        }
    }
}
