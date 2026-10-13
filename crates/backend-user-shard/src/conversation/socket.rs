//! The conversation socket (`runtime.md` § Frame protocol, `backend.md`
//! § Page synchronization): `WS /api/conversations/:id/stream`, moved into
//! the user's shard once upgraded. The socket decodes each message into a
//! client frame and hands it to the conversation's agent connection, one at a
//! time in arrival order; the connection's outbox carries every server frame
//! back, with the failure facts of the error blocks it brings; its media are
//! references into the owner's blobs already (`runtime.md` § Media). A frame the
//! backend refuses before the agent sees it is answered with an `error`
//! frame: one that is not a valid frame (`invalid_frame`), one sent while the
//! conversation is archived (`conversation_archived`), an `open` of a
//! conversation that has no model yet (`model_not_selected`) or whose
//! provider the user may no longer use (`provider_not_found`), and one that
//! storage failed to prepare (`frame_delivery_failed`); a message that is not
//! JSON closes the socket. An `open` takes the conversation's settings order,
//! so the tree opens with the model selection the record holds.

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::time::Duration;

use axum::extract::ws::{CloseFrame, Message, Utf8Bytes, WebSocket, close_code};
use demi_agent_server::{ContentError, ContentResolver, FileReference, Outgoing, ResolvedFiles};
use demi_agent_store::media::{HeldMedia, READS_AT_ONCE};
use demi_backend_database::StorageError;
use demi_backend_database::conversation_index::ConversationRecord;
use demi_backend_page_sync::Part;
use demi_conversation_socket_protocol::{
    ClientContent, ClientFrame, EditOutcome, FrameError, RemoveOutcome, ServerFrame,
    TranscriptPatch, decode_client_frame,
};
use demi_shared_gates::{GateLease, Purpose};
use demi_shared_types::{Block, UserContentBlock};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::{ConversationId, ProviderId};
use futures_util::{StreamExt as _, TryStreamExt as _};
use futures_util::future::LocalBoxFuture;
use tokio_util::sync::CancellationToken;

use super::failure_facts;
use crate::shard::page_socket::{PageSocketOrigin, open_page_socket};
use crate::sync::ChannelSession;
use crate::shard::{PageEnd, PageSocket, Shard};
use demi_backend_host_access::access::{Admitted, ConversationHost, Waits};
use demi_backend_host_access::remote_files::RemoteFile;
use demi_backend_host_access::root_of;

/// The close code of a socket whose page fell a full outbox behind; the page
/// reconnects and adopts the running tree.
const LAGGED: u16 = 4001;

/// The close code of a socket whose session was signed out, the
/// synchronization channel's.
const SESSION_ENDED: u16 = 4002;

/// How a conversation socket ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ending {
    /// The page closed the socket, or it broke.
    Gone,
    /// The page sent a message that is not JSON.
    NotJson,
    /// The page fell a full outbox behind.
    Lagged,
    /// The backend is shutting down.
    Closing,
    /// The session the socket opened with was signed out, or expired.
    SessionEnded,
    /// The session could not be read at its expiry.
    SessionUnread,
    /// The agent connection's outbox closed, so nothing more reaches the
    /// page; the socket goes without a close frame.
    Detached,
}

impl Ending {
    /// How the socket ends: with the close frame the backend sends, or
    /// without one.
    fn page_end(self) -> PageEnd {
        let frame = |code, reason| CloseFrame {
            code,
            reason: Utf8Bytes::from_static(reason),
        };
        match self {
            Self::Gone => PageEnd::Gone,
            Self::NotJson => PageEnd::Close(frame(close_code::INVALID, "not_json")),
            Self::Lagged => PageEnd::Fail(frame(LAGGED, "lagged")),
            Self::Closing => PageEnd::Close(frame(close_code::AWAY, "backend_closing")),
            Self::SessionEnded => PageEnd::Close(frame(SESSION_ENDED, "session_ended")),
            Self::SessionUnread => PageEnd::Fail(frame(close_code::ERROR, "internal_error")),
            Self::Detached => PageEnd::Dropped("agent connection closed"),
        }
    }
}

/// What handling one message came to.
enum Handled {
    /// The frame reached its session, or the backend refused it with these
    /// frames.
    Replies(Vec<ServerFrame>),
    /// The message is not JSON: the socket closes.
    NotJson,
}

/// An `error` frame the backend answers a refused frame with.
fn refusal(code: ErrorCode, message: impl Into<String>) -> ServerFrame {
    ServerFrame::Error {
        message: message.into(),
        code: Some(code.to_string()),
        diagnostics: None,
    }
}

impl Shard {
    /// Serves a socket of `conversation` until either end closes it or the
    /// shard closes. A frame the socket has handed to the agent is handled to
    /// its end, even when the shard closes meanwhile; until then, the
    /// outbox's frames keep flowing to the page. A socket that has sent
    /// nothing for the heartbeat interval sends a `heartbeat`. `origin` is
    /// who opened it, which its log lines name. The session it opened with,
    /// `session`, closes it as it is signed out or as it expires.
    pub async fn serve_conversation_socket(
        self: Rc<Self>,
        conversation: ConversationRecord,
        socket: WebSocket,
        origin: PageSocketOrigin,
        session: ChannelSession,
    ) {
        // Counted before any wait, so the shard's close waits for this
        // socket; one adopted as the close begins ends at once.
        let _socket = self.tree_openers().token();
        if self.is_closing() {
            return;
        }
        let signed_out = self.services().sync.watch_session(self.user(), session.token.clone());
        let id = conversation.id;
        let resolver = Rc::new(ConversationFiles {
            shard: Rc::downgrade(&self),
            conversation: id.clone(),
            host: RefCell::new(None),
        });
        let (connection, mut frames) = self.agent().connect(root_of(&id), resolver.clone());
        let (mut page, mut stream) = open_page_socket(socket, self.services().pages, origin);
        let mut handling: Option<LocalBoxFuture<'_, Handled>> = None;
        let relay = async {
            'relay: loop {
                tokio::select! {
                    handled = async { handling.as_mut().expect("the branch runs only while a frame is handled").await },
                        if handling.is_some() =>
                    {
                        handling = None;
                        let replies = match handled {
                            Handled::Replies(replies) => replies,
                            Handled::NotJson => break Ending::NotJson,
                        };
                        for reply in replies {
                            if !self.send_frame(&mut page, reply).await {
                                break 'relay Ending::Gone;
                            }
                        }
                    }
                    message = stream.next(), if handling.is_none() => match message {
                        Some(Ok(Message::Text(text))) => {
                            let shard = self.clone();
                            let connection = &connection;
                            let resolver = &resolver;
                            let id = id.clone();
                            handling = Some(Box::pin(async move {
                                shard.handle_message(&id, connection, resolver, text.as_str()).await
                            }));
                        }
                        Some(Ok(Message::Binary(_))) => break Ending::NotJson,
                        // Pings are answered by the socket itself.
                        Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                        Some(Ok(Message::Close(_)) | Err(_)) | None => break Ending::Gone,
                    },
                    outgoing = frames.recv() => match outgoing {
                        Outgoing::Frame(frame) => {
                            // The frames the outbox holds at once, as a
                            // turn's end, go in one message, which the page
                            // applies as one change (`product.md`
                            // § Following the agent's work).
                            let mut batch = vec![frame];
                            let mut after = None;
                            while let Some(next) = frames.try_recv() {
                                match next {
                                    Outgoing::Frame(frame) => batch.push(frame),
                                    Outgoing::Lagged => {
                                        after = Some(Ending::Lagged);
                                        break;
                                    }
                                    Outgoing::Closed => {
                                        after = Some(Ending::Detached);
                                        break;
                                    }
                                }
                            }
                            if !self.send_frames(&mut page, batch).await {
                                break Ending::Gone;
                            }
                            if let Some(ending) = after {
                                break ending;
                            }
                        }
                        Outgoing::Lagged => break Ending::Lagged,
                        Outgoing::Closed => break Ending::Detached,
                    },
                    () = page.silent() => {
                        if page.send(to_text(&ServerFrame::Heartbeat)).await.is_err() {
                            break Ending::Gone;
                        }
                    }
                }
            }
        };
        // The shard's close ends the relay wherever it waits, a send to a
        // page that stopped reading included (`backend.md` § Startup and
        // shutdown).
        let ending = tokio::select! {
            biased;
            () = self.closed() => Ending::Closing,
            () = signed_out.marked() => Ending::SessionEnded,
            ending = self.session_expiry(session) => ending,
            ending = relay => ending,
        };
        // A frame that reached the agent is handled to its end: its session
        // may be in the middle of taking it. What it replies goes nowhere.
        if let Some(handled) = handling.take() {
            handled.await;
        }
        page.end(stream, ending.page_end()).await;
        // Dropping the connection, last, detaches it from its tree, whose
        // turns go on.
    }

    /// Resolves as the socket's session expires: at the expiry the socket
    /// holds, it looks at the session again, since a request may have
    /// renewed it, and waits for the new expiry if one did. One timer at a
    /// time, which dropping the future releases.
    async fn session_expiry(&self, mut session: ChannelSession) -> Ending {
        let services = self.services();
        loop {
            let left = session.expires_at.to_jiff().duration_since(services.clock.now().to_jiff());
            // An expiry already past is due at once.
            tokio::time::sleep(Duration::try_from(left).unwrap_or(Duration::ZERO)).await;
            match services.sessions.check(&session.token).await {
                Ok(Some(renewed)) if renewed.expires_at > session.expires_at => {
                    session.expires_at = renewed.expires_at;
                }
                // A session the clock has not yet passed is checked again
                // at the expiry it holds.
                Ok(Some(_)) => {}
                Ok(None) => return Ending::SessionEnded,
                Err(error) => {
                    tracing::error!(
                        error = &error as &dyn std::error::Error,
                        "a conversation socket's session could not be read"
                    );
                    return Ending::SessionUnread;
                }
            }
        }
    }

    /// Restores the conversation's tree with no page attached, as an `open`
    /// does: under the conversation's file
    /// gate and its settings order, with the model its record holds, and
    /// only when the backend would deliver an `open`. A tree that is live
    /// already is left as it is.
    pub(crate) async fn restore_tree(&self, conversation: &ConversationId) -> Result<(), String> {
        let services = self.services();
        services
            .control
            .conversation(conversation.clone())
            .await
            .map_err(|error| error.to_string())?
            .filter(|record| record.owner == *self.user())
            .ok_or("the conversation is gone")?;
        let slot = self.conversations().slot(conversation);
        let _files = slot.file_gate().enter(Purpose::Demand).await;
        let _settings = slot.settings.acquire().await;
        match self
            .prepare_frame(
                conversation,
                &ClientFrame::Open {
                    from: None,
                    edge: None,
                },
            )
            .await
        {
            Ok(Prepared::Deliver) => {}
            Ok(Prepared::Resume(_)) => unreachable!("an open is prepared as an open"),
            Ok(Prepared::Refused(code, message)) => return Err(format!("{code}: {message}")),
            Err(error) => return Err(error.to_string()),
        }
        self.agent()
            .restore(&root_of(conversation))
            .await
            .map_err(|error| error.to_string())
    }

    /// Decodes one message and hands the frame to the conversation's agent
    /// connection, unless the backend refuses it first: a frame that is not
    /// valid, one sent while the conversation is archived, an `open` of a
    /// conversation without a model or whose provider the user may not use,
    /// or a frame storage failed to prepare. A refused edit answers its
    /// `edit_result`, and a refused removal its `remove_result`.
    async fn handle_message(
        &self,
        conversation: &ConversationId,
        connection: &demi_agent_server::Connection<super::ShardHosts>,
        files: &ConversationFiles,
        text: &str,
    ) -> Handled {
        let frame = match decode_client_frame(text) {
            Ok(frame) => frame,
            Err(FrameError::NotJson(_)) => return Handled::NotJson,
            Err(FrameError::Invalid(error)) => {
                return Handled::Replies(vec![refusal(
                    ErrorCode::InvalidFrame,
                    format!("Invalid client frame: {error}"),
                )]);
            }
        };
        // The frame is handled under the conversation's file gate, which a
        // transition holding the conversation makes it wait for, never
        // refuse (`sessions-and-targets.md` § How a conversation uses a
        // device). A frame whose uploads go to the Host is admitted there,
        // once, and its resolver writes through that admission: nothing
        // enters the file gate while holding it (§ Host operations).
        let _admitted = if carries_uploads(&frame) {
            match self
                .host_shard()
                .admit_host(
                    conversation,
                    None,
                    Waits::request(&CancellationToken::new()),
                )
                .await
            {
                Ok(admitted) => {
                    files.host.replace(Some(admitted.host.clone()));
                    Admission::Host(admitted)
                }
                Err(error) => {
                    let (code, _) = error.code();
                    return Handled::Replies(vec![refused_frame(frame, code, error.to_string())]);
                }
            }
        } else {
            Admission::Files(
                self.conversations()
                    .slot(conversation)
                    .file_gate()
                    .enter(Purpose::Demand)
                    .await,
            )
        };
        // An open takes its turn among the changes of the model settings: the
        // tree opens with the selection the record holds, never with one a
        // change is about to replace.
        let _settings = match &frame {
            ClientFrame::Open { .. } => Some(
                self.conversations()
                    .slot(conversation)
                    .settings
                    .acquire()
                    .await,
            ),
            _ => None,
        };
        let handled = match self.prepare_frame(conversation, &frame).await {
            Ok(Prepared::Resume(reason)) => {
                connection.resume(reason).await;
                Handled::Replies(Vec::new())
            }
            Ok(Prepared::Deliver) => {
                connection.handle(frame).await;
                Handled::Replies(Vec::new())
            }
            Ok(Prepared::Refused(code, message)) => {
                Handled::Replies(vec![refused_frame(frame, code, message)])
            }
            Err(error) => {
                tracing::error!(%conversation, error = &error as &dyn std::error::Error, "a frame was not prepared");
                Handled::Replies(vec![refused_frame(
                    frame,
                    ErrorCode::FrameDeliveryFailed,
                    error.to_string(),
                )])
            }
        };
        files.host.replace(None);
        handled
    }

    /// What the backend does before the agent sees `frame` (`web-api.md`
    /// § Sidebar mutations, read state and page synchronization): the
    /// conversation must not be archived, except to close it; an `open` needs
    /// the model the conversation's record holds, of a provider of the user's
    /// scope; a `send` is activity in the conversation; a `resume` carries
    /// the reason the model reads.
    async fn prepare_frame(
        &self,
        conversation: &ConversationId,
        frame: &ClientFrame,
    ) -> Result<Prepared, StorageError> {
        let services = self.services();
        let record = services.control.conversation(conversation.clone()).await?;
        let Some(record) = record.filter(|record| record.owner == *self.user()) else {
            return Ok(Prepared::Refused(
                ErrorCode::ConversationNotFound,
                "No such conversation".into(),
            ));
        };
        if record.archived && !matches!(frame, ClientFrame::Close {}) {
            return Ok(Prepared::Refused(
                ErrorCode::ConversationArchived,
                "Restore the conversation before writing to it".into(),
            ));
        }
        match frame {
            ClientFrame::Open { .. } => {
                let Some(model) = &record.model else {
                    return Ok(Prepared::Refused(
                        ErrorCode::ModelNotSelected,
                        "Choose a model for the conversation before opening it".into(),
                    ));
                };
                let visible = match ProviderId::try_from(model.provider_id.as_str()) {
                    Ok(provider) => services
                        .vault
                        .visible(self.user(), &provider)
                        .await?
                        .is_some(),
                    Err(_) => false,
                };
                if !visible {
                    return Ok(Prepared::Refused(
                        ErrorCode::ProviderNotFound,
                        "No such provider".into(),
                    ));
                }
            }
            ClientFrame::Send { content, .. } => {
                self.title_first_message(&record, content).await?;
                services
                    .control
                    .touch_conversation(record.id.clone())
                    .await?;
                self.mark(Part::Conversation(record.id));
            }
            // The user's Resume carries where the turn now runs, which only
            // the backend knows (`failures-and-recovery.md` § The unfinished
            // turn).
            ClientFrame::Resume {} => {
                return Ok(Prepared::Resume(self.resume_reason(&record).await?));
            }
            _ => {}
        }
        Ok(Prepared::Deliver)
    }

    /// Sends one server frame, presented as the page receives it; false when
    /// the socket is gone.
    async fn send_frame(&self, page: &mut PageSocket, frame: ServerFrame) -> bool {
        self.send_frames(page, vec![frame]).await
    }

    /// Sends `frames` as one message: a frame alone as itself, several as
    /// a JSON array of them in order, which the page applies as one change
    /// (`product.md` § Following the agent's work).
    async fn send_frames(&self, page: &mut PageSocket, frames: Vec<ServerFrame>) -> bool {
        let mut texts = Vec::with_capacity(frames.len());
        for frame in frames {
            let frame = self.present(frame).await;
            let Some(text) = serialize(frame).await else {
                return false;
            };
            texts.push(text);
        }
        let text = if texts.len() == 1 {
            texts.remove(0)
        } else {
            format!("[{}]", texts.join(","))
        };
        page.send(text).await.is_ok()
    }

    /// A server frame as the page receives it: a transcript frame carries
    /// the failure facts of the error blocks it brings (`backend.md`
    /// § Failure facts). Its blocks hold their media by reference already
    /// (§ Media by reference).
    async fn present(&self, frame: ServerFrame) -> ServerFrame {
        let services = self.services();
        let assembly = &services.assembly;
        match frame {
            ServerFrame::TranscriptReset {
                start,
                length,
                blocks,
                version,
                ..
            } => {
                let failures = failure_facts(assembly, &blocks).await;
                ServerFrame::TranscriptReset {
                    start,
                    length,
                    blocks,
                    version,
                    failures,
                }
            }
            ServerFrame::TranscriptPatch {
                patches, revision, ..
            } => {
                let failures = failure_facts(assembly, &blocks_added(&patches)).await;
                ServerFrame::TranscriptPatch {
                    patches,
                    revision,
                    failures,
                }
            }
            ServerFrame::SubagentTranscriptReset {
                subagent_id,
                start,
                length,
                blocks,
                revision,
                ..
            } => {
                let failures = failure_facts(assembly, &blocks).await;
                ServerFrame::SubagentTranscriptReset {
                    subagent_id,
                    start,
                    length,
                    blocks,
                    revision,
                    failures,
                }
            }
            ServerFrame::SubagentTranscriptPatch {
                subagent_id,
                patches,
                revision,
                ..
            } => {
                let failures = failure_facts(assembly, &blocks_added(&patches)).await;
                ServerFrame::SubagentTranscriptPatch {
                    subagent_id,
                    patches,
                    revision,
                    failures,
                }
            }
            other => other,
        }
    }
}

/// What the backend decided about a frame before the agent sees it.
enum Prepared {
    Deliver,
    /// Deliver the user's Resume with the reason the backend composed.
    Resume(Option<String>),
    Refused(ErrorCode, String),
}

/// How a frame is admitted while it is handled.
enum Admission {
    /// Under a lease of the conversation's file gate.
    Files(#[expect(dead_code, reason = "held for its drop")] GateLease),
    /// On the conversation's primary Host, for its uploads.
    Host(#[expect(dead_code, reason = "held for its drop")] Admitted),
}

/// Whether the frame's content names an upload, which is written to the
/// conversation's Host before the session sees it.
fn carries_uploads(frame: &ClientFrame) -> bool {
    let content = match frame {
        ClientFrame::Send { content, .. } | ClientFrame::Steer { content, .. } => content,
        ClientFrame::EditAndSend { request } => &request.content,
        _ => return false,
    };
    content
        .iter()
        .any(|part| matches!(part, ClientContent::Upload { .. }))
}

/// The answer to a frame the backend refused: an edit's `edit_result`, a
/// removal's `remove_result` `gone_on`, since it reached no session to hold
/// what it names (`runtime.md` § Pending steers), and an `error` frame for
/// any other.
fn refused_frame(frame: ClientFrame, code: ErrorCode, message: String) -> ServerFrame {
    match frame {
        ClientFrame::EditAndSend { request } => ServerFrame::EditResult {
            operation_id: request.operation_id,
            outcome: EditOutcome::Rejected { reason: message },
        },
        ClientFrame::CancelPendingSteer { steer_id } => ServerFrame::RemoveResult {
            id: steer_id.into_string(),
            outcome: RemoveOutcome::GoneOn,
        },
        ClientFrame::DequeueMessage { message_id } => ServerFrame::RemoveResult {
            id: message_id.into_string(),
            outcome: RemoveOutcome::GoneOn,
        },
        _ => refusal(code, message),
    }
}

/// The whole blocks `patches` put into a transcript.
fn blocks_added(patches: &[TranscriptPatch]) -> Vec<Block> {
    patches
        .iter()
        .flat_map(|patch| match patch {
            TranscriptPatch::Add { value, .. } | TranscriptPatch::ReplaceBlock { value, .. } => {
                vec![value.clone()]
            }
            TranscriptPatch::AppendText { .. } | TranscriptPatch::Truncate { .. } => Vec::new(),
        })
        .collect()
}

/// A frame's JSON text. A frame that may carry many blocks, a reset or a
/// rewrite's patches, is serialized on the blocking pool, since a long one
/// would hold the shard's thread; none when that pool is gone with its
/// runtime, which is shutting down.
async fn serialize(frame: ServerFrame) -> Option<String> {
    let whole = match &frame {
        ServerFrame::TranscriptReset { .. } | ServerFrame::SubagentTranscriptReset { .. } => true,
        ServerFrame::TranscriptPatch { patches, .. }
        | ServerFrame::SubagentTranscriptPatch { patches, .. } => patches
            .iter()
            .any(|patch| matches!(patch, TranscriptPatch::Truncate { .. })),
        _ => false,
    };
    if !whole {
        return Some(to_text(&frame));
    }
    tokio::task::spawn_blocking(move || to_text(&frame))
        .await
        .ok()
}

fn to_text(frame: &ServerFrame) -> String {
    // A frame's types serialize their fields as JSON strings, numbers, arrays
    // and objects with string keys, which serde_json never refuses.
    serde_json::to_string(frame).expect("a server frame serializes to JSON")
}

/// The files of a frame's content: an upload is written to the
/// conversation's Host and becomes its blocks, and a remote file becomes its
/// reference once its device may be read. The uploads are written first, so a
/// frame whose upload cannot be written grants no remote file.
struct ConversationFiles {
    /// Weak: the shard owns the agent server that holds this resolver.
    shard: Weak<Shard>,
    conversation: ConversationId,
    /// The Host the frame being handled was admitted on, while its uploads
    /// are written there.
    host: RefCell<Option<ConversationHost>>,
}

impl ContentResolver for ConversationFiles {
    fn resolve<'a>(
        &'a self,
        files: Vec<FileReference>,
    ) -> LocalBoxFuture<'a, Result<ResolvedFiles, ContentError>> {
        Box::pin(async move {
            let refused = |message: String| ContentError {
                message,
                code: Some(ErrorCode::FrameDeliveryFailed.to_string()),
            };
            let shard = self
                .shard
                .upgrade()
                .ok_or_else(|| refused("The backend is shutting down".into()))?;
            // Each file's blocks by its place, the remote files' once they
            // are granted together, and the bytes of the uploads' media. The
            // uploads are written together, as many at once as blobs are
            // read, since each reads its own: each write takes a free name
            // itself, so two of one name never meet.
            let mut uploads = Vec::new();
            let mut remote = Vec::new();
            for file in &files {
                match file {
                    FileReference::Upload { r#ref, file_name } => uploads.push((r#ref, file_name)),
                    FileReference::RemoteFile { device_id, path } => remote.push(RemoteFile {
                        device: device_id.clone(),
                        path: path.clone(),
                    }),
                }
            }
            let mut written = Vec::new().into_iter();
            if !uploads.is_empty() {
                // A frame with uploads is admitted on its Host first.
                let host = self.host.borrow().clone();
                let host =
                    host.ok_or_else(|| refused("The frame's Host was not admitted".into()))?;
                let host_shard = shard.host_shard();
                let writes = uploads.into_iter().map(|(reference, file_name)| {
                    host_shard.resolve_upload(&self.conversation, &host, reference, file_name)
                });
                let writes: Vec<_> = futures_util::stream::iter(writes)
                    .buffered(READS_AT_ONCE)
                    .try_collect()
                    .await
                    .map_err(|error| refused(error.to_string()))?;
                written = writes.into_iter();
            }
            let mut media = HeldMedia::default();
            let mut resolved: Vec<Option<Vec<UserContentBlock>>> = Vec::with_capacity(files.len());
            for file in &files {
                match file {
                    FileReference::Upload { .. } => {
                        let (blocks, held) = written.next().expect("one answer per upload");
                        resolved.push(Some(blocks));
                        media.absorb(held);
                    }
                    FileReference::RemoteFile { .. } => resolved.push(None),
                }
            }
            let mut references = if remote.is_empty() {
                Vec::new().into_iter()
            } else {
                shard
                    .host_shard()
                    .reference_remote_files(&self.conversation, &remote)
                    .await
                    .map_err(|refusal| refused(refusal.to_string()))?
                    .into_iter()
            };
            let blocks = resolved
                .into_iter()
                .map(|blocks| blocks.unwrap_or_else(|| references.next().into_iter().collect()))
                .collect();
            Ok(ResolvedFiles { blocks, media })
        })
    }
}
