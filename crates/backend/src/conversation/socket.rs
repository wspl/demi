//! The conversation socket (`runtime.md` § Frame protocol, `backend.md`
//! § Browser synchronization): `WS /api/conversations/:id/stream`, moved into
//! the user's shard once upgraded. The socket decodes each message into a
//! client frame and hands it to the conversation's agent connection, one at a
//! time in arrival order; the connection's outbox carries every server frame
//! back, with the failure facts of the error blocks it brings. A frame the
//! backend refuses before the agent sees it is answered with an `error`
//! frame: one that is not a valid frame (`invalid_frame`), one sent while the
//! conversation is archived (`conversation_archived`), one naming a provider
//! the user may not use (`provider_not_found`), and one that storage failed
//! to prepare (`frame_delivery_failed`); a message that is not JSON closes
//! the socket.

use std::rc::{Rc, Weak};

use axum::extract::ws::{CloseFrame, Message, Utf8Bytes, WebSocket, close_code};
use demi_agent::{ContentError, ContentResolver, FileReference, Outgoing};
use demi_agent_protocol::{ClientFrame, EditOutcome, FrameError, ServerFrame, TranscriptPatch, decode_client_frame};
use demi_core::{Block, UserContentBlock};
use demi_gates::Purpose;
use demi_web_api::error::ErrorCode;
use demi_web_api::ids::{ConversationId, ProviderId};
use futures_util::future::LocalBoxFuture;
use futures_util::{SinkExt as _, StreamExt as _};

use super::remote_files::RemoteFile;
use super::{failure_facts, root_of};
use crate::shard::Shard;
use crate::storage::StorageError;
use crate::storage::conversation_index::{ConversationModel, ConversationRecord};

/// The close code of a socket whose page fell a full outbox behind; the page
/// reconnects and adopts the running tree.
const LAGGED: u16 = 4001;

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
}

impl Ending {
    /// The close frame the backend sends for it; none when the socket is
    /// gone already.
    fn close(self) -> Option<CloseFrame> {
        let (code, reason) = match self {
            Self::Gone => return None,
            Self::NotJson => (close_code::INVALID, "not_json"),
            Self::Lagged => (LAGGED, "lagged"),
            Self::Closing => (close_code::AWAY, "backend_closing"),
        };
        Some(CloseFrame {
            code,
            reason: Utf8Bytes::from_static(reason),
        })
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
    /// its end; while it is, the outbox's frames keep flowing to the page.
    pub(crate) async fn serve_conversation_socket(self: Rc<Self>, conversation: ConversationRecord, socket: WebSocket) {
        // Counted before any wait, so the shard's close waits for this
        // socket; one adopted as the close begins ends at once.
        let _socket = self.conversation_sockets().token();
        if self.is_closing() {
            return;
        }
        let cwd = match self.resolve_target(&conversation).await {
            Ok(target) => target.path().to_owned(),
            Err(error) => {
                tracing::error!(
                    conversation = %conversation.id,
                    error = &error as &dyn std::error::Error,
                    "the conversation's target does not resolve"
                );
                return;
            }
        };
        let id = conversation.id;
        let resolver = Rc::new(ConversationFiles {
            shard: Rc::downgrade(&self),
            conversation: id.clone(),
        });
        let (connection, mut frames) = self.agent().connect(root_of(&id), cwd, resolver);
        let (mut sink, mut stream) = socket.split();
        let mut handling: Option<LocalBoxFuture<'_, Handled>> = None;
        let mut closing = false;
        let ending = loop {
            if closing && handling.is_none() {
                break Ending::Closing;
            }
            tokio::select! {
                handled = async { handling.as_mut().expect("the branch runs only while a frame is handled").await },
                    if handling.is_some() =>
                {
                    handling = None;
                    let replies = match handled {
                        Handled::Replies(replies) => replies,
                        Handled::NotJson => break Ending::NotJson,
                    };
                    let mut sent = true;
                    for reply in replies {
                        sent = self.send_frame(&mut sink, reply).await;
                        if !sent {
                            break;
                        }
                    }
                    if !sent {
                        break Ending::Gone;
                    }
                }
                message = stream.next(), if handling.is_none() && !closing => match message {
                    Some(Ok(Message::Text(text))) => {
                        let shard = self.clone();
                        let connection = &connection;
                        let id = id.clone();
                        handling = Some(Box::pin(async move { shard.handle_message(&id, connection, text.as_str()).await }));
                    }
                    Some(Ok(Message::Binary(_))) => break Ending::NotJson,
                    // Pings are answered by the socket itself.
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_)) | Err(_)) | None => break Ending::Gone,
                },
                outgoing = frames.recv() => match outgoing {
                    Outgoing::Frame(frame) => {
                        if !self.send_frame(&mut sink, frame).await {
                            break Ending::Gone;
                        }
                    }
                    Outgoing::Lagged => break Ending::Lagged,
                    Outgoing::Closed => break Ending::Gone,
                },
                () = self.closed(), if !closing => closing = true,
            }
        };
        // A frame that reached the agent is handled to its end: its session
        // may be in the middle of taking it. What it replies goes nowhere.
        if let Some(handled) = handling.take() {
            handled.await;
        }
        if let Some(close) = ending.close() {
            // The page may be gone already; the socket closes either way.
            let _ = sink.send(Message::Close(Some(close))).await;
        }
        // Dropping the connection, last, detaches it from its tree, whose
        // turns go on.
    }

    /// Decodes one message and hands the frame to the conversation's agent
    /// connection, unless the backend refuses it first: a frame that is not
    /// valid, one sent while the conversation is archived, a provider the
    /// user may not use, or a frame storage failed to prepare. A refused edit
    /// answers its `edit_result`.
    async fn handle_message(
        &self,
        conversation: &ConversationId,
        connection: &demi_agent::Connection<super::ConversationHarness>,
        text: &str,
    ) -> Handled {
        let frame = match decode_client_frame(text) {
            Ok(frame) => frame,
            Err(FrameError::NotJson(_)) => return Handled::NotJson,
            Err(FrameError::Invalid(error)) => {
                return Handled::Replies(vec![refusal(ErrorCode::InvalidFrame, format!("Invalid client frame: {error}"))]);
            }
        };
        // The frame is handled under the conversation's file gate, which a
        // transition holding the conversation makes it wait for, never
        // refuse (`sessions-and-targets.md` § How a conversation uses a
        // device).
        let _admitted = self.conversations().slot(conversation).files.enter(Purpose::Demand).await;
        match self.prepare_frame(conversation, &frame).await {
            Ok(Prepared::Deliver) => {
                connection.handle(frame).await;
                Handled::Replies(Vec::new())
            }
            Ok(Prepared::Refused(code, message)) => Handled::Replies(vec![refused_frame(frame, code, message)]),
            Err(error) => {
                tracing::error!(%conversation, error = &error as &dyn std::error::Error, "a frame was not prepared");
                Handled::Replies(vec![refused_frame(frame, ErrorCode::FrameDeliveryFailed, error.to_string())])
            }
        }
    }

    /// What the backend does before the agent sees `frame` (`web-api.md`
    /// § Sidebar mutations, read state and page synchronization): the
    /// conversation must not be archived, except to close it; an `open` or
    /// a `set_provider` names a provider of the user's scope, and the
    /// conversation records the selection; a `send` is activity in the
    /// conversation.
    async fn prepare_frame(&self, conversation: &ConversationId, frame: &ClientFrame) -> Result<Prepared, StorageError> {
        let services = self.services();
        let record = services.control.conversation(conversation.clone()).await?;
        let Some(record) = record.filter(|record| record.owner == *self.user()) else {
            return Ok(Prepared::Refused(ErrorCode::ConversationNotFound, "No such conversation".into()));
        };
        if record.archived && !matches!(frame, ClientFrame::Close {}) {
            return Ok(Prepared::Refused(
                ErrorCode::ConversationArchived,
                "Restore the conversation before writing to it".into(),
            ));
        }
        match frame {
            ClientFrame::Open { model } | ClientFrame::SetProvider { model, .. } => {
                let visible = match ProviderId::try_from(model.provider_id.as_str()) {
                    Ok(provider) => services.vault.visible(self.user(), &provider).await?.map(|_| provider),
                    Err(_) => None,
                };
                let Some(provider) = visible else {
                    return Ok(Prepared::Refused(ErrorCode::ProviderNotFound, "No such provider".into()));
                };
                let selected = ConversationModel {
                    provider,
                    model: model.model.id.clone(),
                };
                services.control.set_conversation_model(record.id, Some(selected)).await?;
            }
            ClientFrame::Send { .. } => services.control.touch_conversation(record.id).await?,
            _ => {}
        }
        Ok(Prepared::Deliver)
    }

    /// Sends one server frame, presented as the page receives it; false when
    /// the socket is gone.
    async fn send_frame(&self, sink: &mut futures_util::stream::SplitSink<WebSocket, Message>, frame: ServerFrame) -> bool {
        let frame = self.present(frame).await;
        let Some(text) = serialize(frame).await else {
            return false;
        };
        sink.send(Message::Text(text.into())).await.is_ok()
    }

    /// A server frame as the page receives it (`backend.md` § Failure
    /// facts): a transcript frame carries the failure facts of the error
    /// blocks it brings.
    async fn present(&self, frame: ServerFrame) -> ServerFrame {
        let assembly = &self.services().assembly;
        match frame {
            ServerFrame::TranscriptReset { blocks, version, .. } => {
                let failures = failure_facts(assembly, &blocks).await;
                ServerFrame::TranscriptReset {
                    blocks,
                    version,
                    failures,
                }
            }
            ServerFrame::TranscriptPatch { patches, revision, .. } => {
                let failures = failure_facts(assembly, &blocks_added(&patches)).await;
                ServerFrame::TranscriptPatch {
                    patches,
                    revision,
                    failures,
                }
            }
            ServerFrame::SubagentTranscriptReset {
                subagent_id,
                blocks,
                revision,
                ..
            } => {
                let failures = failure_facts(assembly, &blocks).await;
                ServerFrame::SubagentTranscriptReset {
                    subagent_id,
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
    Refused(ErrorCode, String),
}

/// The answer to a frame the backend refused: an edit's `edit_result`, and
/// an `error` frame for any other.
fn refused_frame(frame: ClientFrame, code: ErrorCode, message: String) -> ServerFrame {
    match frame {
        ClientFrame::EditAndSend { request } => ServerFrame::EditResult {
            operation_id: request.operation_id,
            outcome: EditOutcome::Rejected { reason: message },
        },
        _ => refusal(code, message),
    }
}

/// The whole blocks `patches` put into a transcript.
fn blocks_added(patches: &[TranscriptPatch]) -> Vec<Block> {
    patches
        .iter()
        .flat_map(|patch| match patch {
            TranscriptPatch::Add { value, .. } | TranscriptPatch::ReplaceBlock { value, .. } => vec![value.clone()],
            TranscriptPatch::Replace { value } => value.clone(),
            TranscriptPatch::Remove { .. } | TranscriptPatch::AppendText { .. } => Vec::new(),
        })
        .collect()
}

/// A frame's JSON text. A frame that carries a whole transcript is
/// serialized on the blocking pool, since a long one would hold the shard's
/// thread; none when that pool is gone with its runtime, which is shutting
/// down.
async fn serialize(frame: ServerFrame) -> Option<String> {
    let whole = match &frame {
        ServerFrame::TranscriptReset { .. } | ServerFrame::SubagentTranscriptReset { .. } => true,
        ServerFrame::TranscriptPatch { patches, .. } | ServerFrame::SubagentTranscriptPatch { patches, .. } => patches
            .iter()
            .any(|patch| matches!(patch, TranscriptPatch::Replace { .. })),
        _ => false,
    };
    if !whole {
        return Some(to_text(&frame));
    }
    tokio::task::spawn_blocking(move || to_text(&frame)).await.ok()
}

fn to_text(frame: &ServerFrame) -> String {
    // A frame's types serialize their fields as JSON strings, numbers, arrays
    // and objects with string keys, which serde_json never refuses.
    serde_json::to_string(frame).expect("a server frame serializes to JSON")
}

/// The files of a frame's content: a remote file becomes its reference once
/// its device may be read. Interim: uploads resolve against the user's
/// attachment records and are written to the conversation's Host once those
/// land; until then a frame with an upload is refused.
struct ConversationFiles {
    /// Weak: the shard owns the agent server that holds this resolver.
    shard: Weak<Shard>,
    conversation: ConversationId,
}

impl ContentResolver for ConversationFiles {
    fn resolve<'a>(
        &'a self,
        files: Vec<FileReference>,
    ) -> LocalBoxFuture<'a, Result<Vec<Vec<UserContentBlock>>, ContentError>> {
        Box::pin(async move {
            let refused = |message: String| ContentError {
                message,
                code: Some(ErrorCode::FrameDeliveryFailed.to_string()),
            };
            let shard = self
                .shard
                .upgrade()
                .ok_or_else(|| refused("The backend is shutting down".into()))?;
            let remote = files
                .into_iter()
                .map(|file| match file {
                    FileReference::RemoteFile { device_id, path } => Ok(RemoteFile { device: device_id, path }),
                    FileReference::Upload { .. } => Err(refused("Attachments are not available on this backend yet".into())),
                })
                .collect::<Result<Vec<_>, _>>()?;
            let blocks = shard
                .reference_remote_files(&self.conversation, &remote)
                .await
                .map_err(|refusal| refused(refusal.to_string()))?;
            Ok(blocks.into_iter().map(|block| vec![block]).collect())
        })
    }
}
