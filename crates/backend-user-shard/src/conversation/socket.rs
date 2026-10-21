//! The conversation socket: `WS /api/conversations/:id/stream`, moved into the
//! user's shard once upgraded.
//!
//! The socket decodes each message into a client frame and hands it to the
//! conversation's agent connection, one at a time in arrival order. The
//! connection's outbox carries every server frame back, with the failure facts
//! of the error blocks it brings; its media are references into the owner's
//! blobs already.
//!
//! # Refused frames
//!
//! A frame the backend refuses before the agent sees it is answered with an
//! `error` frame:
//!
//! - `invalid_frame`: the message is not a valid frame.
//! - `conversation_archived`: the frame was sent while the conversation is
//!   archived.
//! - `model_not_selected`: an `open` of a conversation that has no model yet.
//! - `provider_not_found`: an `open` of a conversation whose provider the user
//!   may no longer use.
//! - `frame_delivery_failed`: storage failed to prepare the frame.
//!
//! A message that is not JSON closes the socket.
//!
//! # The conversation's own frames
//!
//! Beside the agents' frames, the socket sends what the backend keeps of the
//! conversation itself (`ConversationFrame`): its metadata once the page
//! sent its `open`, refused or not, as one of a conversation with no model
//! yet, and at each change of it, and its draft when the draft is newer than
//! the revision the page named at its open, and at each change of it.
//! Only a socket of the conversation hears of them; the sidebar's channel
//! sends the conversation's entry alone.
//!
//! # Frames the socket rewrites
//!
//! - An `open` takes the conversation's settings order, so the tree opens with
//!   the model selection the record holds.
//! - A `send_saved` becomes the `send` or `steer` of the message saved for later
//!   it names, as the list holds it or as an edit changed it, and the message
//!   leaves the list as the session takes it. One the list no longer holds is
//!   answered with a `rejected` frame (`saved_not_found`), unless an edit
//!   changed it: the edit holds the whole message, which goes as it is.

use std::rc::{Rc, Weak};
use std::time::Duration;

use axum::extract::ws::{CloseFrame, Message, Utf8Bytes, WebSocket, close_code};
use demi_agent_server::{ContentError, ContentResolver, FileReference, Outgoing, ResolvedFiles};
use demi_agent_store::media::HeldMedia;
use demi_backend_database::StorageError;
use demi_backend_database::conversation_index::ConversationRecord;
use demi_backend_page_sync::Part;
use demi_conversation_socket_protocol::{
    SteerOutcome,
    ClientContent, ClientFrame, ClientFrameKind, EditOutcome, FrameError, Refusal, RemoveOutcome,
    SavedDelivery, ServerFrame, TranscriptPatch, Version, decode_client_frame,
};
use demi_shared_gates::Purpose;
use demi_shared_types::{Block, BlockId, TurnId, UserContentBlock};
use demi_web_api_protocol::conversations::{ConversationFrame, ConversationMetadata};
use demi_web_api_protocol::drafts::{ATTACHMENT_MARK, DraftFile};
use demi_web_api_protocol::saved::SavedMessage;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::{ConversationId, ProviderId};
use futures_util::future::LocalBoxFuture;

use super::failure_facts;
use crate::shard::page_socket::{PageSocketOrigin, open_page_socket};
use crate::sync::ChannelSession;
use crate::shard::{PageEnd, PageSocket, Shard};
use demi_backend_host_access::remote_files::{RemoteFile, RemoteFileRefusal};
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
    /// An `open` reached the tree, or the backend refused it with these
    /// frames: the page holds the draft at `draft`, and follows the
    /// conversation from now on.
    Opened { draft: u64, replies: Vec<ServerFrame> },
    /// The message is not JSON: the socket closes.
    NotJson,
}

/// What a socket last sent of the conversation itself, once its page opened
/// it.
#[derive(Default)]
struct Shown {
    metadata: Option<ConversationMetadata>,
    /// The revision of the draft the page holds; none before its open.
    draft: Option<u64>,
}

/// An `error` frame the backend answers a refused frame with.
fn refusal(code: ErrorCode, message: impl Into<String>) -> ServerFrame {
    ServerFrame::Error {
        message: message.into(),
        code: Some(code.to_string()),
        diagnostics: None,
        request_id: None,
    }
}

impl Shard {
    /// Serves a socket of `conversation` until either end closes it or the shard
    /// closes.
    ///
    /// - A frame the socket has handed to the agent is handled to its end, even
    ///   when the shard closes meanwhile; until then, the outbox's frames keep
    ///   flowing to the page.
    /// - A socket that has sent nothing for the heartbeat interval sends a
    ///   `heartbeat`.
    /// - `origin` is who opened it, which its log lines name.
    /// - The session it opened with, `session`, closes it as it is signed out or
    ///   as it expires.
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
        let id = conversation.id;
        let watch = self
            .services()
            .sync
            .watch_conversation(self.user(), session.token.clone(), id.clone());
        let resolver = Rc::new(ConversationFiles {
            shard: Rc::downgrade(&self),
            conversation: id.clone(),
        });
        let (connection, mut frames) = self.agent().connect(root_of(&id), resolver.clone());
        let (mut page, mut stream) = open_page_socket(socket, self.services().pages, origin);
        let mut handling: Option<LocalBoxFuture<'_, Handled>> = None;
        let mut shown = Shown::default();
        let relay = async {
            'relay: loop {
                let busy = handling.is_some();
                tokio::select! {
                    handled = async { handling.as_mut().expect("the branch runs only while a frame is handled").await },
                        if handling.is_some() =>
                    {
                        handling = None;
                        let replies = match handled {
                            Handled::Replies(replies) => replies,
                            Handled::Opened { draft, replies } => {
                                shown.draft = Some(draft);
                                if !self.show_conversation(&id, &mut page, &mut shown).await {
                                    break 'relay Ending::Gone;
                                }
                                replies
                            }
                            Handled::NotJson => break Ending::NotJson,
                        };
                        for reply in replies {
                            if !self.send_frame(&id, &mut page, reply).await {
                                break 'relay Ending::Gone;
                            }
                        }
                    }
                    // While a frame is handled, the page's next message
                    // waits for its turn, and its pings are answered meanwhile.
                    message = stream.next_unless_busy(busy) => match message {
                        Some(Ok(Message::Text(text))) => {
                            let shard = self.clone();
                            let connection = &connection;
                            let id = id.clone();
                            handling = Some(Box::pin(async move {
                                shard.handle_message(&id, connection, text.as_str()).await
                            }));
                        }
                        Some(Ok(Message::Binary(_))) => break Ending::NotJson,
                        // Pings are answered by the socket itself.
                        Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                        Some(Ok(Message::Close(_)) | Err(_)) | None => break Ending::Gone,
                    },
                    outgoing = frames.recv() => match outgoing {
                        Outgoing::Frames(step) => {
                                // Every step the outbox holds goes in one
                                // message, never a step in two: the page
                                // applies a message as one change.
                            let mut batch = step;
                            let mut after = None;
                            while let Some(next) = frames.try_recv() {
                                match next {
                                    Outgoing::Frames(step) => batch.extend(step),
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
                            if !self.send_frames(&id, &mut page, batch).await {
                                break Ending::Gone;
                            }
                            if let Some(ending) = after {
                                break ending;
                            }
                        }
                        Outgoing::Lagged => break Ending::Lagged,
                        Outgoing::Closed => break Ending::Detached,
                    },
                    () = watch.marked() => {
                        let marked = watch.take();
                        if marked.session_ended {
                            break Ending::SessionEnded;
                        }
                        if shown.draft.is_some() && !self.show_conversation(&id, &mut page, &mut shown).await {
                            break Ending::Gone;
                        }
                    }
                    () = page.silent() => {
                        if page.send(to_text(&ServerFrame::Heartbeat)).await.is_err() {
                            break Ending::Gone;
                        }
                    }
                }
            }
        };
        // The shard's close ends the relay wherever it waits, a send to a
        // page that stopped reading included
        // (`crates/backend/src/backend.rs`).
        let ending = tokio::select! {
            biased;
            () = self.closed() => Ending::Closing,
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

    /// Restores the conversation's tree with no page attached, as an `open` does.
    ///
    /// It runs under the conversation's file gate and its settings order, with the
    /// model its record holds, and only when the backend would deliver an `open`.
    /// A tree that is live already is left as it is.
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
                    agents: None,
                    draft: None,
                },
            )
            .await
        {
            Ok(Prepared::Deliver) => {}
            Ok(Prepared::Refused(code, message)) => return Err(format!("{code}: {message}")),
            Err(error) => return Err(error.to_string()),
        }
        self.agent()
            .restore(&root_of(conversation))
            .await
            .map_err(|error| error.to_string())
    }

    /// Whether the backend would deliver the user's Resume of the
    /// conversation; it refuses it, for example, for an archived
    /// conversation.
    pub(crate) async fn resume_admitted(&self, conversation: &ConversationId) -> Result<bool, StorageError> {
        let prepared = self.prepare_frame(conversation, &ClientFrame::Resume {}).await?;
        Ok(matches!(prepared, Prepared::Deliver))
    }

    /// Decodes one message and hands the frame to the conversation's agent
    /// connection, unless the backend refuses it first.
    ///
    /// The backend refuses:
    ///
    /// - a frame that is not valid;
    /// - one sent while the conversation is archived;
    /// - an `open` of a conversation without a model or whose provider the user
    ///   may not use;
    /// - a frame storage failed to prepare.
    ///
    /// A refused edit answers its `edit_result`, and a refused removal its
    /// `remove_result`.
    async fn handle_message(
        &self,
        conversation: &ConversationId,
        connection: &demi_agent_server::Connection<super::ShardHosts>,
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
        // A message saved for later goes as the frame the composer would send, as
        // the turn the page named, one at a time. It leaves the list once the
        // session takes it, and stays when the session refuses it.
        //
        // One a send already took as that turn answers as that send did: the
        // session holds it, so nothing is sent again. An edited one another
        // page deleted or sent meanwhile goes as the user changed it, with
        // nothing to take off the list.
        let (frame, saved, from_list, _sending) = match frame {
            ClientFrame::SendSaved { id, turn_id, delivery, content } => {
                let sending = self.conversations().slot(conversation).saved.acquire().await;
                let control = &self.services().control;
                match control.saved_message(conversation.clone(), id.clone()).await {
                    Ok(Some(message)) => {
                        let content = content.unwrap_or_else(|| saved_content(message));
                        (
                            saved_frame(content, turn_id.clone(), delivery),
                            Some((id, turn_id)),
                            true,
                            Some(sending),
                        )
                    }
                    Ok(None) => match control.sent_as(conversation.clone(), id).await {
                        Ok(Some(turn)) if turn == turn_id => {
                            let at = self.agent().tree(&root_of(conversation)).map(|tree| tree.agents_version());
                            return Handled::Replies(sent_again(turn_id, delivery, at));
                        }
                        Ok(_) => match content {
                            Some(content) => (saved_frame(content, turn_id, delivery), None, true, Some(sending)),
                            None => return Handled::Replies(vec![saved_gone()]),
                        },
                        Err(error) => return Handled::Replies(vec![send_failed(error.to_string())]),
                    },
                    Err(error) => {
                        return Handled::Replies(vec![send_failed(error.to_string())]);
                    }
                }
            }
            frame => (frame, None, false, None),
        };
        // The frame is handled under the conversation's file gate, which a
        // transition holding the conversation makes it wait for, never refuse
        // (`crates/backend-cloud/src/uses.rs`). Its uploads go to no Host
        // (`crates/backend-host-access/src/uploads.rs`).
        let _files = self
            .conversations()
            .slot(conversation)
            .file_gate()
            .enter(Purpose::Demand)
            .await;
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
        // A refusal names the command the page sent: a saved message's is
        // its `send_saved`'s, whatever frame it became.
        let command = if from_list { ClientFrameKind::SendSaved } else { frame.kind() };
        let opened = match &frame {
            ClientFrame::Open { draft, .. } => Some(draft.unwrap_or(0)),
            _ => None,
        };
        let handled = match self.prepare_frame(conversation, &frame).await {
            Ok(Prepared::Deliver) => {
                let taken = connection.handle_as(frame, command).await;
                if let Some(draft) = opened {
                    return Handled::Opened { draft, replies: Vec::new() };
                }
                if let (true, Some((id, turn))) = (taken, saved) {
                    self.take_saved(conversation, id, turn).await;
                }
                Handled::Replies(Vec::new())
            }
            Ok(Prepared::Refused(code, message)) => {
                let replies = vec![refused_frame(frame, command, code, message)];
                // A conversation whose tree cannot open, as one with no model
                // yet, is still shown: its metadata and its draft follow.
                match opened {
                    Some(draft) => Handled::Opened { draft, replies },
                    None => Handled::Replies(replies),
                }
            }
            Err(error) => {
                tracing::error!(%conversation, error = &error as &dyn std::error::Error, "a frame was not prepared");
                // A send's failure inside the backend is told as one, with
                // the request ID beside it.
                let code = match command {
                    ClientFrameKind::Send | ClientFrameKind::SendSaved => ErrorCode::SendFailed,
                    _ => ErrorCode::FrameDeliveryFailed,
                };
                Handled::Replies(vec![refused_frame(frame, command, code, error.to_string())])
            }
        };
        handled
    }

    /// What the backend does before the agent sees `frame`:
    ///
    /// - The conversation must not be archived, except to close it.
    /// - An `open` needs the model the conversation's record holds, of a provider
    ///   of the user's scope.
    /// - A `send` is activity in the conversation.
    /// - A `resume` carries the reason the model reads.
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
                // The user's message lets a run's conversation into the
                // sidebar.
                self.answered_by_user(&record.id).await;
                self.mark(Part::Conversation(record.id));
            }
            ClientFrame::Steer { .. } => self.answered_by_user(&record.id).await,
            _ => {}
        }
        Ok(Prepared::Deliver)
    }

    /// Takes the saved message `id`, which the session took, off the
    /// conversation's list. One removed meanwhile is gone already. A
    /// failure leaves it listed though sent; it is logged, since the send
    /// it follows has happened and has its own answer.
    async fn take_saved(&self, conversation: &ConversationId, id: String, turn: TurnId) {
        match self.services().control.take_saved(conversation.clone(), id, turn).await {
            Ok(true) => self.mark(Part::Metadata(conversation.clone())),
            Ok(false) => {}
            Err(error) => tracing::error!(
                %conversation,
                error = &error as &dyn std::error::Error,
                "a sent message saved for later stayed on its list"
            ),
        }
    }

    /// Sends one server frame, presented as the page receives it; false when
    /// the socket is gone.
    async fn send_frame(&self, conversation: &ConversationId, page: &mut PageSocket, frame: ServerFrame) -> bool {
        self.send_frames(conversation, page, vec![frame]).await
    }

    /// Sends `frames` as one message: a frame alone as itself, several as
    /// a JSON array of them in order, which the page applies as one change. Every `error`
    /// and `rejected` frame carries the request ID of the socket's upgrade, and every
    /// refused send of `conversation` is logged.
    async fn send_frames(&self, conversation: &ConversationId, page: &mut PageSocket, frames: Vec<ServerFrame>) -> bool {
        // An open's handshake starts with `opened`; its size and how long it
        // takes to write are logged, as what a page waits for to open.
        let handshake = matches!(frames.first(), Some(ServerFrame::Opened));
        let count = frames.len();
        let building = std::time::Instant::now();
        let mut texts = Vec::with_capacity(frames.len());
        for frame in frames {
            let mut frame = self.present(frame).await;
            if let ServerFrame::Error { request_id, .. } | ServerFrame::Rejected { request_id, .. } = &mut frame {
                *request_id = Some(page.request().clone());
            }
            if let ServerFrame::Rejected {
                command: command @ (ClientFrameKind::Send | ClientFrameKind::SendSaved),
                refusal,
                ..
            } = &frame
            {
                // The backend decides a send failed, and says so here, under
                // the socket's request ID.
                tracing::warn!(
                    %conversation,
                    command = ?command,
                    code = %refusal.code,
                    cause = %refusal.reason,
                    "a send was refused"
                );
            }
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
        if !handshake {
            return page.send(text).await.is_ok();
        }
        let bytes = text.len();
        let build_ms = building.elapsed().as_millis();
        let writing = std::time::Instant::now();
        let sent = page.send(text).await.is_ok();
        tracing::info!(
            frames = count,
            bytes,
            build_ms,
            write_ms = writing.elapsed().as_millis(),
            sent,
            "a conversation's open handshake was written"
        );
        sent
    }

    /// A server frame as the page receives it: a transcript frame carries
    /// the failure facts of the error blocks it brings. Its blocks hold
    /// their media by reference already (§ Media by reference).
    async fn present(&self, frame: ServerFrame) -> ServerFrame {
        let services = self.services();
        let assembly = &services.assembly;
        match frame {
            ServerFrame::TranscriptReset {
                node,
                start,
                length,
                blocks,
                version,
                versions,
                ..
            } => {
                let failures = failure_facts(assembly, self.user(), &blocks).await;
                ServerFrame::TranscriptReset {
                    node,
                    start,
                    length,
                    blocks,
                    version,
                    failures,
                    versions,
                }
            }
            ServerFrame::TranscriptPatch {
                node, patches, revision, ..
            } => {
                let failures = failure_facts(assembly, self.user(), &blocks_added(&patches)).await;
                ServerFrame::TranscriptPatch {
                    node,
                    patches,
                    revision,
                    failures,
                }
            }
            other => other,
        }
    }

    /// Sends the page what changed of the conversation itself since `shown`:
    /// its metadata, and its draft when newer than the page's. A failed read
    /// is logged and sends nothing, as the next change sends what it read;
    /// false when the socket is gone.
    async fn show_conversation(&self, conversation: &ConversationId, page: &mut PageSocket, shown: &mut Shown) -> bool {
        let services = self.services();
        let read = async {
            let Some(record) = services
                .control
                .conversation(conversation.clone())
                .await?
                .filter(|record| record.owner == *self.user())
            else {
                return Ok(None);
            };
            let draft_revision = record.draft_revision;
            let metadata = self.conversation_metadata(record).await?;
            let draft = if shown.draft.is_some_and(|held| draft_revision > held) {
                Some(services.control.draft(conversation.clone()).await?)
            } else {
                None
            };
            Ok::<_, StorageError>(Some((metadata, draft)))
        };
        let (metadata, draft) = match read.await {
            Ok(Some(read)) => read,
            Ok(None) => return true,
            Err(error) => {
                tracing::warn!(
                    %conversation,
                    error = &error as &dyn std::error::Error,
                    "what a page shows of a conversation was not read"
                );
                return true;
            }
        };
        let mut frames = Vec::new();
        if shown.metadata.as_ref() != Some(&metadata) {
            shown.metadata = Some(metadata.clone());
            frames.push(ConversationFrame::Conversation {
                metadata: Box::new(metadata),
            });
        }
        if let Some(draft) = draft {
            shown.draft = Some(draft.revision);
            frames.push(ConversationFrame::Draft { draft });
        }
        for frame in frames {
            // The frames' types serialize as JSON objects, which serde_json
            // never refuses.
            let text = serde_json::to_string(&frame).expect("a conversation frame serializes to JSON");
            if page.send(text).await.is_err() {
                return false;
            }
        }
        true
    }
}

/// What the backend decided about a frame before the agent sees it.
enum Prepared {
    Deliver,
    Refused(ErrorCode, String),
}

/// The answer to a frame the backend refused:
///
/// - an edit's or a Regenerate's `edit_result`;
/// - a removal's `remove_result` `gone_on`, since it reached no session to hold
///   what it names;
/// - a send's `rejected` as the page's `command`;
/// - a saved message's whichever frame it became;
/// - an `error` frame for any other.
fn refused_frame(frame: ClientFrame, command: ClientFrameKind, code: ErrorCode, message: String) -> ServerFrame {
    if matches!(command, ClientFrameKind::Send | ClientFrameKind::SendSaved) {
        // A send is refused as itself, so the page marks its message not
        // delivered with the cause.
        return ServerFrame::Rejected {
            command,
            refusal: Refusal::new(code, message),
            request_id: None,
        };
    }
    match frame {
        ClientFrame::EditAndSend { request } => ServerFrame::EditResult {
            operation_id: request.operation_id,
            outcome: EditOutcome::Rejected {
                refusal: Refusal::new(code, message),
            },
        },
        ClientFrame::Regenerate { request } => ServerFrame::EditResult {
            operation_id: request.operation_id,
            outcome: EditOutcome::Rejected {
                refusal: Refusal::new(code, message),
            },
        },
        ClientFrame::CancelPendingSteer { steer_id } => ServerFrame::RemoveResult {
            id: steer_id.into_string(),
            outcome: RemoveOutcome::GoneOn,
            at: None,
        },
        ClientFrame::DequeueMessage { message_id } => ServerFrame::RemoveResult {
            id: message_id.into_string(),
            outcome: RemoveOutcome::GoneOn,
            at: None,
        },
        _ => refusal(code, message),
    }
}

/// The answer to a `send_saved` whose message the list no longer holds.
fn saved_gone() -> ServerFrame {
    ServerFrame::Rejected {
        command: ClientFrameKind::SendSaved,
        refusal: Refusal::new(ErrorCode::SavedNotFound, "No such message saved for later"),
        request_id: None,
    }
}

/// The answer to a `send_saved` the backend failed on inside, such as a
/// storage failure of the list, which has no cause of its own for the user.
fn send_failed(message: String) -> ServerFrame {
    ServerFrame::Rejected {
        command: ClientFrameKind::SendSaved,
        refusal: Refusal::new(ErrorCode::SendFailed, message),
        request_id: None,
    }
}

/// The refusal of a message whose files on devices cannot be referenced:
/// each cause the user can act on as its own code, a storage failure as a
/// failure inside the backend.
fn remote_file_refused(refusal: RemoteFileRefusal) -> ContentError {
    let code = match &refusal {
        RemoteFileRefusal::NotAccessible => ErrorCode::DeviceNotFound,
        RemoteFileRefusal::Offline(_) => ErrorCode::DeviceOffline,
        RemoteFileRefusal::Unquotable => ErrorCode::PathUnquotable,
        RemoteFileRefusal::Storage(_) => ErrorCode::SendFailed,
    };
    ContentError {
        message: refusal.to_string(),
        code: Some(code.to_string()),
    }
}

/// What a send of a saved message a send already took as `turn` answers: the
/// session holds the turn, whose frames went out with the first send and
/// come with every opening, so the message or the steer is answered as taken
/// at `at`, the agents' version now, which shows it.
fn sent_again(turn: TurnId, delivery: SavedDelivery, at: Option<Version>) -> Vec<ServerFrame> {
    match delivery {
        SavedDelivery::Message | SavedDelivery::Queue => vec![ServerFrame::SendResult { message_id: turn, at }],
        SavedDelivery::Steer => vec![ServerFrame::SteerResult {
            steer_id: BlockId::try_from(turn.into_string()).expect("a turn id is not empty"),
            outcome: SteerOutcome::Accepted { at },
        }],
    }
}

/// The content a saved message sends: its text split at the marks where its
/// files stand, as the page joins a message's content, without the blank space
/// at its ends.
fn saved_content(message: SavedMessage) -> Vec<ClientContent> {
    let mut files = message.files.into_iter().map(|file| match file {
        DraftFile::Upload {
            r#ref, file_name, ..
        } => ClientContent::Upload {
            r#ref: r#ref.into_string(),
            file_name,
        },
        DraftFile::RemoteFile { device_id, path } => ClientContent::RemoteFile { device_id, path },
    });
    let parts: Vec<&str> = message.text.split(ATTACHMENT_MARK).collect();
    let last = parts.len() - 1;
    let mut content = Vec::new();
    for (index, part) in parts.into_iter().enumerate() {
        let part = if index == 0 { part.trim_start() } else { part };
        let part = if index == last { part.trim_end() } else { part };
        if !part.trim().is_empty() {
            content.push(ClientContent::Text {
                text: part.to_owned(),
            });
        }
        if index < last {
            content.extend(files.next());
        }
    }
    // Files without a mark follow the text.
    content.extend(files);
    content
}

/// The frame the composer would send for a saved message's `content`: a
/// message, which waits in the queue while a turn runs, or a steer. The frame
/// carries `turn`, the page's id for the send, as the turn's or the steer's.
fn saved_frame(content: Vec<ClientContent>, turn: TurnId, delivery: SavedDelivery) -> ClientFrame {
    match delivery {
        SavedDelivery::Message | SavedDelivery::Queue => ClientFrame::Send {
            message_id: turn,
            content,
            comments: Vec::new(),
        },
        SavedDelivery::Steer => ClientFrame::Steer {
            comments: Vec::new(),
            steer_id: BlockId::try_from(turn.into_string()).expect("a turn id is not empty"),
            content,
        },
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
        ServerFrame::TranscriptReset { .. } => true,
        ServerFrame::TranscriptPatch { patches, .. } => patches
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

/// The files of a frame's content: an upload becomes its blocks under the
/// conversation's next attachment number, and a remote file becomes its
/// reference once its device may be read.
///
/// The uploads are numbered first, in the order of the content, so a frame whose
/// upload cannot be read grants no remote file.
///
/// A send writes nothing to the Host: most files are only for the model to read,
/// which it does from the blob, so a Host that is slow to reach, offline or a
/// stopped Cloud never holds a message. A command that needs the file fetches it
/// with `demi attachment download`.
struct ConversationFiles {
    /// Weak: the shard owns the agent server that holds this resolver.
    shard: Weak<Shard>,
    conversation: ConversationId,
}

impl ContentResolver for ConversationFiles {
    fn resolve<'a>(
        &'a self,
        files: Vec<FileReference>,
    ) -> LocalBoxFuture<'a, Result<ResolvedFiles, ContentError>> {
        Box::pin(async move {
            // A failure inside the backend has no cause of its own for the
            // user; it is shown with the socket's request ID.
            let refused = |message: String| ContentError {
                message,
                code: Some(ErrorCode::SendFailed.to_string()),
            };
            let shard = self.shard.upgrade().ok_or_else(|| ContentError {
                message: "The backend is shutting down".into(),
                code: Some(ErrorCode::BackendClosing.to_string()),
            })?;
            // Each file's blocks by its place, the remote files' once they
            // are granted together, and the bytes of the uploads' media.
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
                // One at a time, so the numbers follow the content's order.
                let host_shard = shard.host_shard();
                let mut writes = Vec::with_capacity(uploads.len());
                for (reference, file_name) in uploads {
                    writes.push(
                        host_shard
                            .resolve_upload(&self.conversation, reference, file_name)
                            .await
                            .map_err(refused)?,
                    );
                }
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
                    .map_err(remote_file_refused)?
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
