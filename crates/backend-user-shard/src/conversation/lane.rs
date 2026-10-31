//! The conversation lane: a conversation's lane of the page connection, which
//! the user's shard serves once it opened.
//!
//! The lane decodes each message into a client frame and hands it to the
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
//! A message that is not JSON closes the lane.
//!
//! # The conversation's own frames
//!
//! Beside the agents' frames, the lane sends what the backend keeps of the
//! conversation itself (`ConversationFrame`) as it changes, never as the
//! answer to an `open`: its metadata, whole; its saved messages, its workflows
//! and its shares, each whole, with the entity tag of its route's answer; its
//! draft when newer than the revision the page last held; and the description
//! of an attachment an agent uploaded since the page's `open`, as a reply may
//! name it before it exists. The page reads each by its route, then follows it
//! by the `open` and `follow` frames, naming the tag it holds; the lane
//! reads the value and sends it when its tag differs, and at each change.
//! A value the page does not follow is not sent. Only a lane of the
//! conversation hears of them; the sidebar's channel sends the conversation's
//! entry alone.
//!
//! # A conversation's first message
//!
//! The request that creates a conversation may carry its first message
//! (`admit_first_message`). A connection of its own then takes the frames a
//! page's lane would send, an `open` and the `send`, through the same
//! handling, so the message is admitted as any `send` is: its title, its files
//! resolved in the conversation, its id checked against the messages already
//! taken. The request answers from what the connection's outbox holds, and the
//! connection goes; the page's lane opens afterwards, and the page reads the
//! reply so far with the root's latest page.
//!
//! # Attachments
//!
//! A transcript frame carries the attachments its blocks and its appended
//! text name, whichever agent stored them, and the facts of the plugins'
//! reply blocks they name (`attachment_facts`), read as it is sent, as its
//! failure facts are. A reply streams in pieces, so a frame that
//! completes a tag in the text so far names it, whichever piece it ends in.
//!
//! # Frames the lane rewrites
//!
//! - An `open` takes the conversation's settings order, so the tree opens with
//!   the model selection the record holds.
//! - A `send_saved` becomes the `send` or `steer` of the message saved for later
//!   it names, as the list holds it or as an edit changed it, and the message
//!   leaves the list as the session takes it. One the list no longer holds is
//!   answered with a `rejected` frame (`saved_not_found`), unless an edit
//!   changed it: the edit holds the whole message, which goes as it is.

use std::rc::{Rc, Weak};

use axum::extract::ws::close_code;
use demi_agent_server::{ContentError, ContentResolver, FileReference, Outgoing, ResolvedFiles};
use demi_agent_store::media::HeldMedia;
use demi_backend_database::StorageError;
use demi_backend_database::conversation_index::ConversationRecord;
use demi_backend_page_sync::Part;
use demi_backend_permissions::Stops;
use demi_backend_database::conversation_attachments::uploaded_after;
use demi_conversation_lane_protocol::{
    SteerOutcome, Follow, Followed,
    ClientContent, ClientFrame, ClientFrameKind, EditOutcome, FrameError, Refusal,
    RemoveOutcome, SavedDelivery, SendMessage, ServerFrame, TranscriptPatch, Version, decode_client_frame,
};
use demi_shared_gates::Purpose;
use demi_shared_types::{Block, BlockId, NodeId, TurnId, UserContentBlock};
use demi_web_api_protocol::conversations::{ConversationFrame, FirstMessageAnswer};
use demi_web_api_protocol::drafts::{ATTACHMENT_MARK, DraftFile};
use demi_web_api_protocol::saved::SavedMessage;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::{ConversationId, ProviderId};
use demi_web_api_protocol::permissions::PermissionRequestId;
use futures_util::future::LocalBoxFuture;

use super::attachment_facts::StreamedText;
use super::failure_facts;
use crate::sync::etag_of;
use crate::shard::{Lane, LaneClose, LaneEnd, LaneMessage, LaneReader, Shard};
use demi_backend_host_access::remote_files::{RemoteFile, RemoteFileRefusal};
use demi_backend_host_access::root_of;

/// The close code of a lane whose page fell a full outbox behind; the page
/// reconnects and adopts the running tree.
const LAGGED: u16 = 4001;

/// How a conversation lane ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ending {
    /// The page closed the lane, or it broke.
    Gone,
    /// The page sent a message that is not JSON.
    NotJson,
    /// The page fell a full outbox behind.
    Lagged,
    /// The backend is shutting down.
    Closing,
    /// The agent connection's outbox closed, so nothing more reaches the
    /// page; the lane goes without a close frame.
    Detached,
}

impl Ending {
    /// How the lane ends: with the close the backend sends, or without one.
    fn lane_end(self) -> LaneEnd {
        let close = |code, reason| LaneClose { code, reason };
        match self {
            Self::Gone => LaneEnd::Gone,
            Self::NotJson => LaneEnd::Close(close(close_code::INVALID, "not_json")),
            Self::Lagged => LaneEnd::Fail(close(LAGGED, "lagged")),
            Self::Closing => LaneEnd::Close(close(close_code::AWAY, "backend_closing")),
            Self::Detached => LaneEnd::Dropped("agent connection closed"),
        }
    }
}

/// What handling one message came to.
enum Handled {
    /// The frame reached its session, or the backend refused it with these
    /// frames.
    Replies(Vec<ServerFrame>),
    /// A frame that follows or ends following values: the lane applies
    /// `own`, what it follows itself, then `frame`, the rest, is handled.
    Following { own: OwnFollows, frame: Option<ClientFrame> },
    /// An `open` reached the tree, which answers it with `opened`.
    Opened { replies: Vec<ServerFrame> },
    /// The backend refused an `open` with these frames, as a conversation
    /// with no model yet is: the lane answers `opened` after them, since
    /// the page still reads the conversation and follows its changes.
    OpenRefused { replies: Vec<ServerFrame> },
    /// The message is not JSON: the lane closes.
    NotJson,
}

/// What a frame changes of what the lane follows itself: the conversation's
/// metadata and its lists, which the agents' connection knows nothing of.
#[derive(Default)]
struct OwnFollows {
    /// The revision of the draft the page holds, when the frame is its `open`.
    opening: Option<u64>,
    follow: Vec<Follow>,
    unfollow: Vec<Followed>,
}

impl OwnFollows {
    /// Splits `follows` into what the lane follows itself, and the rest,
    /// which the agents' connection follows.
    fn split(follows: Vec<Follow>) -> (Vec<Follow>, Vec<Follow>) {
        follows.into_iter().partition(|follow| is_own(&follow.value))
    }
}

/// Whether the lane follows `value` itself: the agents' connection follows
/// the agents and the transcripts.
fn is_own(value: &Followed) -> bool {
    matches!(value, Followed::Metadata | Followed::Saved | Followed::Workflows | Followed::Shares)
}

/// What a lane follows of the conversation itself, each value with the
/// entity tag the page holds of it, which is what a value is told against: a
/// value whose tag differs is sent whole, with its tag. A value none is not
/// followed.
#[derive(Default)]
struct Shown {
    metadata: Option<String>,
    saved: Option<String>,
    workflows: Option<String>,
    shares: Option<String>,
    /// The number of the latest attachment the page holds, whose uploads since
    /// go to it with their descriptions; none before its open.
    attached_through: Option<u64>,
    /// The revision of the draft the page holds; none before its open.
    draft: Option<u64>,
}

impl Shown {
    /// Whether the page follows anything of the conversation itself.
    fn follows_any(&self) -> bool {
        self.draft.is_some()
            || [&self.metadata, &self.saved, &self.workflows, &self.shares]
                .into_iter()
                .any(Option::is_some)
    }

    fn slot(&mut self, value: &Followed) -> Option<&mut Option<String>> {
        match value {
            Followed::Metadata => Some(&mut self.metadata),
            Followed::Saved => Some(&mut self.saved),
            Followed::Workflows => Some(&mut self.workflows),
            Followed::Shares => Some(&mut self.shares),
            Followed::Agents | Followed::Transcript { .. } => None,
        }
    }

    fn apply(&mut self, own: OwnFollows) {
        if let Some(draft) = own.opening {
            self.draft = Some(draft);
        }
        for Follow { value, holds } in own.follow {
            let tag = match holds {
                demi_conversation_lane_protocol::Held::Tag { tag } => tag,
                // A version names no whole value: the page holds none of it.
                demi_conversation_lane_protocol::Held::Version { .. } => String::new(),
            };
            if let Some(slot) = self.slot(&value) {
                *slot = Some(tag);
            }
        }
        for value in own.unfollow {
            if let Some(slot) = self.slot(&value) {
                *slot = None;
            }
        }
    }
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
    /// Serves a lane of `conversation` until either end closes it or the shard
    /// closes.
    ///
    /// - A frame the lane has handed to the agent is handled to its end, even
    ///   when the shard closes meanwhile; until then, the outbox's frames keep
    ///   flowing to the page.
    /// - The lane ends with its connection, which ends with the session it
    ///   opened with.
    pub async fn serve_conversation_lane(
        self: Rc<Self>,
        conversation: ConversationRecord,
        mut page: Lane,
        mut stream: LaneReader,
    ) {
        // Counted before any wait, so the shard's close waits for this
        // lane; one adopted as the close begins ends at once.
        let _lane = self.tree_openers().token();
        if self.is_closing() {
            return;
        }
        let id = conversation.id;
        let watch = self
            .services()
            .sync
            .watch_conversation(self.user(), id.clone());
        let resolver = Rc::new(ConversationFiles {
            shard: Rc::downgrade(&self),
            conversation: id.clone(),
        });
        let (connection, mut frames) = self.agent().connect(root_of(&id), resolver.clone());
        let mut handling: Option<LocalBoxFuture<'_, Handled>> = None;
        let mut shown = Shown::default();
        let mut streamed = StreamedText::default();
        let relay = async {
            'relay: loop {
                let busy = handling.is_some();
                // The page's own frames come first, in this order, so a
                // burst of what the agent says never holds back what the page
                // asked for.
                tokio::select! {
                    biased;
                    handled = async { handling.as_mut().expect("the branch runs only while a frame is handled").await },
                        if handling.is_some() =>
                    {
                        handling = None;
                        let replies = match handled {
                            Handled::Replies(replies) => replies,
                            Handled::Following { own, frame } => {
                                let opening = own.opening.is_some();
                                shown.apply(own);
                                // The values the page follows are compared with
                                // what it holds before the frame is handled, and
                                // the attachments its open holds are noted.
                                if !self.show_conversation(&id, &mut page, &mut shown, opening).await {
                                    break 'relay Ending::Gone;
                                }
                                let Some(frame) = frame else {
                                    continue 'relay;
                                };
                                let shard = self.clone();
                                let connection = &connection;
                                let id = id.clone();
                                handling = Some(Box::pin(async move {
                                    shard.handle_frame(&id, connection, frame).await
                                }));
                                continue 'relay;
                            }
                            Handled::Opened { replies } => replies,
                            Handled::OpenRefused { mut replies } => {
                                replies.push(ServerFrame::Opened);
                                replies
                            }
                            Handled::NotJson => break Ending::NotJson,
                        };
                        for reply in replies {
                            if !self.send_frame(&id, &mut page, &mut streamed, reply).await {
                                break 'relay Ending::Gone;
                            }
                        }
                    }
                    // While a frame is handled, the page's next message
                    // waits for its turn in the lane's credit.
                    message = stream.next_unless_busy(busy) => match message {
                        Some(LaneMessage::Text(text)) => {
                            let shard = self.clone();
                            let connection = &connection;
                            let id = id.clone();
                            handling = Some(Box::pin(async move {
                                shard.handle_message(&id, connection, text.as_str()).await
                            }));
                        }
                        Some(LaneMessage::Binary(_)) => break Ending::NotJson,
                        None => break Ending::Gone,
                    },
                    // What changed in the conversation's metadata comes before
                    // the agent's frames, which never go quiet while it says
                    // something: the marks coalesce, so this takes the agent no
                    // more than one pass each time.
                    () = watch.marked() => {
                        watch.take();
                        if shown.follows_any() && !self.show_conversation(&id, &mut page, &mut shown, false).await {
                            break Ending::Gone;
                        }
                    }
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
                            if !self.send_frames(&id, &mut page, &mut streamed, batch).await {
                                break Ending::Gone;
                            }
                            if let Some(ending) = after {
                                break ending;
                            }
                        }
                        Outgoing::Lagged => break Ending::Lagged,
                        Outgoing::Closed => break Ending::Detached,
                    },
                }
            }
        };
        // The shard's close ends the relay wherever it waits, a send to a
        // page that stopped reading included
        // (`crates/backend/src/backend.rs`).
        let ending = tokio::select! {
            biased;
            () = self.closed() => Ending::Closing,
            ending = relay => ending,
        };
        // A frame that reached the agent is handled to its end: its session
        // may be in the middle of taking it. What it replies goes nowhere.
        if let Some(handled) = handling.take() {
            handled.await;
        }
        page.end(stream, ending.lane_end());
        // Dropping the connection, last, detaches it from its tree, whose
        // turns go on.
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
                &ClientFrame::Open { draft: None, follows: Vec::new() },
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

    /// Restores the conversation's tree for a read that names its version,
    /// which a tree that is not live has only as the stored one: the version
    /// a page follows must be the live tree's own, or its first follow is
    /// stale. A conversation whose tree cannot open is read as it is stored.
    pub async fn restore_for_read(&self, conversation: &ConversationId) {
        if self.agent().tree(&root_of(conversation)).is_some() {
            return;
        }
        if let Err(reason) = self.restore_tree(conversation).await {
            tracing::debug!(event = "conversation.read_without_tree", %conversation, reason, "a conversation is read without its tree");
        }
    }

    /// Admits the first message of a conversation just created, as the
    /// lane would: a connection of its own opens the conversation's session
    /// and hands it the `send` frame through the admission every `send`
    /// takes, then goes. The page opens its lane afterwards, and that
    /// opening carries the reply so far
    /// (`docs/decisions/0045-a-new-conversation-sends-with-its-creation.md`).
    ///
    /// A message already taken under its id is taken again without a second
    /// turn, as the session does for a repeated `send`.
    pub(crate) async fn admit_first_message(
        self: &Rc<Self>,
        conversation: &ConversationId,
        message: SendMessage,
    ) -> FirstMessageAnswer {
        let resolver = Rc::new(ConversationFiles {
            shard: Rc::downgrade(self),
            conversation: conversation.clone(),
        });
        let (connection, mut frames) = self.agent().connect(root_of(conversation), resolver);
        let message_id = message.message_id.clone();
        let steps = [ClientFrame::Open { draft: None, follows: Vec::new() }, ClientFrame::Send(message)];
        for frame in steps {
            let sending = matches!(frame, ClientFrame::Send(_));
            let replies = match self.handle_frame(conversation, &connection, frame).await {
                Handled::Replies(replies) | Handled::Opened { replies } | Handled::OpenRefused { replies } => replies,
                Handled::Following { .. } | Handled::NotJson => Vec::new(),
            };
            // What the backend refused before the session saw the frame, then
            // what the session answered, which waits in the outbox.
            let mut answered = replies;
            while let Some(Outgoing::Frames(step)) = frames.try_recv() {
                answered.extend(step);
            }
            for reply in answered {
                match reply {
                    ServerFrame::Rejected { refusal, .. } => return FirstMessageAnswer::Refused { refusal },
                    ServerFrame::Error { message, code, .. } => {
                        let code = code.unwrap_or_else(|| ErrorCode::SendFailed.to_string());
                        return FirstMessageAnswer::Refused { refusal: Refusal::new(code, message) };
                    }
                    ServerFrame::SendResult { message_id: taken, .. } if sending && taken == message_id => {
                        return FirstMessageAnswer::Taken;
                    }
                    _ => {}
                }
            }
        }
        FirstMessageAnswer::Refused {
            refusal: Refusal::new(ErrorCode::SendFailed, "The session did not answer the message"),
        }
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
        match decode_client_frame(text) {
            Ok(ClientFrame::Open { draft, follows }) => {
                let (own, rest) = OwnFollows::split(follows);
                Handled::Following {
                    own: OwnFollows {
                        opening: Some(draft.unwrap_or(0)),
                        follow: own,
                        unfollow: Vec::new(),
                    },
                    frame: Some(ClientFrame::Open { draft, follows: rest }),
                }
            }
            Ok(ClientFrame::Follow(follow)) if is_own(&follow.value) => Handled::Following {
                own: OwnFollows {
                    opening: None,
                    follow: vec![follow],
                    unfollow: Vec::new(),
                },
                frame: None,
            },
            Ok(ClientFrame::Unfollow { value }) if is_own(&value) => Handled::Following {
                own: OwnFollows {
                    opening: None,
                    follow: Vec::new(),
                    unfollow: vec![value],
                },
                frame: None,
            },
            Ok(frame) => self.handle_frame(conversation, connection, frame).await,
            Err(FrameError::NotJson(_)) => Handled::NotJson,
            Err(FrameError::Invalid(error)) => Handled::Replies(vec![refusal(
                ErrorCode::InvalidFrame,
                format!("Invalid client frame: {error}"),
            )]),
        }
    }

    /// Handles `frame` to its end: an `open` once the page was shown the
    /// conversation (`Handled::Opening`), any other at once.
    async fn handle_frame(
        &self,
        conversation: &ConversationId,
        connection: &demi_agent_server::Connection<super::ShardHosts>,
        frame: ClientFrame,
    ) -> Handled {
        let started = std::time::Instant::now();
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
                let control = &self.services().control;
                let list = match control.saved_sharers(conversation.clone()).await {
                    Ok(sharers) => sharers.key,
                    Err(error) => return Handled::Replies(vec![send_failed(error.to_string())]),
                };
                let sending = self.conversations().saved_list(&list).acquire().await;
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
        let files = self
            .conversations()
            .slot(conversation)
            .file_gate()
            .enter(Purpose::Demand)
            .await;
        let files_pending_ms = millis(started.elapsed());
        // An open takes its turn among the changes of the model settings: the
        // tree opens with the selection the record holds, never with one a
        // change is about to replace. A delivered open keeps both gates until
        // the tree took it.
        let settings = match &frame {
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
        let sent = sent_facts(&frame);
        let opening = matches!(frame, ClientFrame::Open { .. });
        let followed = match &frame {
            ClientFrame::Open { follows, .. } => follows.clone(),
            _ => Vec::new(),
        };
        let handled = match self.prepare_frame(conversation, &frame).await {
            Ok(Prepared::Deliver) => {
                if opening {
                    connection.handle_as(frame, command).await;
                    drop((files, settings));
                    return Handled::Opened { replies: Vec::new() };
                }
                let held = self.held_by(conversation, &frame);
                // A stop is carried out whenever it names an agent that is
                // there, and the agent reads the denial in the turn the stop
                // ends: one that came after would open a turn.
                let stops = matches!(
                    frame,
                    ClientFrame::Abort {} | ClientFrame::AbortSubagent { .. } | ClientFrame::AbortSubagents {}
                );
                if stops {
                    self.deny(conversation, held.clone()).await;
                }
                let taken = connection.handle_as(frame, command).await;
                if taken && !stops {
                    self.deny(conversation, held).await;
                }
                if let (true, Some((message, attachments))) = (taken, sent) {
                    tracing::info!(
                        event = "message.accepted",
                        %conversation,
                        user = %self.user(),
                        turn = message.as_str(),
                        kind = kind_name(command),
                        attachments,
                        files_pending_ms,
                        ms_to_accept = millis(started.elapsed()),
                        "a message was taken by its conversation"
                    );
                }
                if let (true, Some((id, turn))) = (taken, saved) {
                    self.take_saved(conversation, id, turn).await;
                }
                Handled::Replies(Vec::new())
            }
            Ok(Prepared::Refused(code, message)) => {
                let replies = vec![refused_frame(frame, command, code, message)];
                // A conversation whose tree cannot open, as one with no model
                // yet, is still shown, and follows its changes.
                if opening {
                    // The tree does not take the open, so the connection
                    // follows what the page follows of it by frames of their
                    // own: what it holds of a tree that is not live is
                    // stale.
                    for follow in followed {
                        connection.handle(ClientFrame::Follow(follow)).await;
                    }
                    Handled::OpenRefused { replies }
                } else {
                    Handled::Replies(replies)
                }
            }
            Err(error) => {
                tracing::error!(event = "frame.prepare_failed", %conversation, frame_kind = kind_name(command), error = &error as &dyn std::error::Error, "a frame was not prepared");
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
        // Following and unfollowing change nothing of the conversation, and an
        // archived one is followed to hear of its restoring.
        let following = matches!(frame, ClientFrame::Follow(_) | ClientFrame::Unfollow { .. });
        if record.archived && !following && !matches!(frame, ClientFrame::Close {}) {
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
            ClientFrame::Send(SendMessage { content, .. }) => {
                self.title_first_message(&record, content).await?;
                services
                    .control
                    .touch_conversation(record.id.clone())
                    .await?;
                self.mark(Part::Conversation(record.id));
            }
            _ => {}
        }
        Ok(Prepared::Deliver)
    }

    /// The permission requests that hold a call `frame` stops: any action of
    /// the user on the agent whose call waits but Allow denies it, so a
    /// message to the agent, its edit or retry, its Stop, and the stop of
    /// the waiting command itself do.
    fn held_by(&self, conversation: &ConversationId, frame: &ClientFrame) -> Vec<PermissionRequestId> {
        let root = root_of(conversation);
        let Some(tree) = self.agent().tree(&root) else {
            return Vec::new();
        };
        let held = |stops| demi_backend_permissions::held_requests(self.permission_shard(), conversation, &stops);
        // The agents below `top`, itself included, which a stop closes with it.
        let below = |top: &NodeId| {
            let parents: Vec<_> = tree.nodes().iter().map(|node| (node.id().clone(), node.record().parent)).collect();
            let mut agents = vec![top.clone()];
            while let Some((id, _)) = parents
                .iter()
                .find(|(id, parent)| !agents.contains(id) && parent.as_ref().is_some_and(|parent| agents.contains(parent)))
            {
                agents.push(id.clone());
            }
            agents
        };
        match frame {
            ClientFrame::Send(_)
            | ClientFrame::Steer { .. }
            | ClientFrame::SteerNow { .. }
            | ClientFrame::SendQueuedMessage { .. }
            | ClientFrame::EditAndSend { .. }
            | ClientFrame::Regenerate { .. }
            | ClientFrame::Retry {}
            | ClientFrame::Abort {} => held(Stops::Agents(&[root])),
            // The server stops only a direct child of the root.
            ClientFrame::AbortSubagent { subagent_id } => {
                let child = tree
                    .node(subagent_id)
                    .is_some_and(|node| node.record().parent.as_ref() == Some(&root));
                if child { held(Stops::Agents(&below(subagent_id))) } else { Vec::new() }
            }
            ClientFrame::AbortSubagents {} => held(Stops::Agents(&below(&root)[1..])),
            ClientFrame::ShellAbort { command_id } => match tree.call_of(command_id) {
                Some(call) => held(Stops::Call(&call)),
                None => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    /// Denies the held requests `requests`: each call returns as any call
    /// does, with its command's end, and the user's own action is read after
    /// it.
    async fn deny(&self, conversation: &ConversationId, requests: Vec<PermissionRequestId>) {
        if requests.is_empty() {
            return;
        }
        if let Err(error) = demi_backend_permissions::deny(self.permission_shard(), conversation, requests).await {
            tracing::warn!(event = "permission_requests.deny_failed", %conversation, %error, "the requests that hold an agent's calls were not denied");
        }
    }

    /// Takes the saved message `id`, which the session took, off the
    /// conversation's list. One removed meanwhile is gone already. A
    /// failure leaves it listed though sent; it is logged, since the send
    /// it follows has happened and has its own answer.
    async fn take_saved(&self, conversation: &ConversationId, id: String, turn: TurnId) {
        match self.services().control.take_saved(conversation.clone(), id, turn).await {
            Ok(true) => match self.services().control.saved_sharers(conversation.clone()).await {
                Ok(sharers) => {
                    for sharer in sharers.conversations {
                        self.mark(Part::Metadata(sharer));
                    }
                    if sharers.project {
                        self.mark(Part::Projects);
                    }
                }
                Err(_) => self.mark(Part::Metadata(conversation.clone())),
            },
            Ok(false) => {}
            Err(error) => tracing::error!(
                event = "saved_message.stuck",
                %conversation,
                error = &error as &dyn std::error::Error,
                "a sent message saved for later stayed on its list"
            ),
        }
    }

    /// Sends one server frame, presented as the page receives it; false when
    /// the lane is gone.
    async fn send_frame(&self, conversation: &ConversationId, page: &mut Lane, streamed: &mut StreamedText, frame: ServerFrame) -> bool {
        self.send_frames(conversation, page, streamed, vec![frame]).await
    }

    /// Sends `frames` as one message: a frame alone as itself, several as
    /// a JSON array of them in order, which the page applies as one change. Every `error`
    /// and `rejected` frame carries the request ID of the lane's open, and every
    /// refused send of `conversation` is logged.
    async fn send_frames(
        &self,
        conversation: &ConversationId,
        page: &mut Lane,
        streamed: &mut StreamedText,
        frames: Vec<ServerFrame>,
    ) -> bool {
        let mut texts = Vec::with_capacity(frames.len());
        for frame in frames {
            let mut frame = self.present(conversation, streamed, frame).await;
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
                // the lane's request ID.
                tracing::warn!(
                    event = "message.refused",
                    %conversation,
                    user = %self.user(),
                    frame_kind = kind_name(*command),
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
        page.send(text).await.is_ok()
    }

    /// A server frame as the page receives it: a transcript frame carries
    /// the failure facts of the error blocks it brings, and the attachments
    /// and the reply blocks its blocks and its appended text name.
    /// Its blocks hold their media by reference already (§ Media by
    /// reference).
    async fn present(&self, conversation: &ConversationId, streamed: &mut StreamedText, frame: ServerFrame) -> ServerFrame {
        let services = self.services();
        let provider_registry = &services.provider_registry;
        match frame {
            ServerFrame::TranscriptPatch {
                node, patches, version, ..
            } => {
                let added = blocks_added(&patches);
                let failures = failure_facts(&services.vault, provider_registry, self.user(), &added).await;
                let (attachments, reply_blocks) = streamed
                    .named(&services.conversations, self.plugins(), conversation, &node, &patches)
                    .await;
                let attachments = attachments.unwrap_or_else(|error| {
                    tracing::warn!(
                        event = "frame_attachments.read_failed",
                        %conversation,
                        error = &error as &dyn std::error::Error,
                        "the attachments a transcript frame names were not read"
                    );
                    Vec::new()
                });
                ServerFrame::TranscriptPatch {
                    node,
                    patches,
                    version,
                    failures,
                    attachments,
                    reply_blocks,
                }
            }
            other => other,
        }
    }

    /// Sends the page what it follows of the conversation itself that differs
    /// from what it holds, in one message: its metadata and each list, whole,
    /// with their entity tags, when the tag differs from the page's; its
    /// draft when newer than the page's; and the attachments agents uploaded
    /// since its open.
    ///
    /// With `opening`, the attachment the open's moment holds is taken as the
    /// baseline. A failed read is logged and sends nothing, as the next change
    /// sends what it read; false when the lane is gone.
    async fn show_conversation(
        &self,
        conversation: &ConversationId,
        page: &mut Lane,
        shown: &mut Shown,
        opening: bool,
    ) -> bool {
        if !shown.follows_any() {
            return true;
        }
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
            let saved = match shown.saved {
                Some(_) => Some(services.control.saved_listing(conversation.clone()).await?),
                None => None,
            };
            let workflows = match shown.workflows {
                Some(_) => Some(self.conversation_workflows(conversation).await),
                None => None,
            };
            let shares = match shown.shares {
                Some(_) => Some(self.conversation_shares(conversation).await?),
                None => None,
            };
            let draft = if shown.draft.is_some_and(|held| draft_revision > held) {
                Some(services.control.draft(conversation.clone()).await?)
            } else {
                None
            };
            let uploaded = match shown.attached_through {
                Some(held) if !opening && metadata.last_attachment > held => services
                    .conversations
                    .read(conversation, move |connection| uploaded_after(connection, held))
                    .await?
                    .unwrap_or_default(),
                _ => Vec::new(),
            };
            Ok::<_, StorageError>(Some((metadata, saved, workflows, shares, draft, uploaded)))
        };
        let (metadata, saved, workflows, shares, draft, uploaded) = match read.await {
            Ok(Some(read)) => read,
            Ok(None) => return true,
            Err(error) => {
                tracing::warn!(
                    event = "conversation_view.read_failed",
                    %conversation,
                    error = &error as &dyn std::error::Error,
                    "what a page shows of a conversation was not read"
                );
                return true;
            }
        };
        let mut frames = Vec::new();
        shown.attached_through = Some(metadata.last_attachment);
        if let Some(held) = &mut shown.metadata {
            let etag = etag_of(&metadata);
            if *held != etag {
                held.clone_from(&etag);
                frames.push(ConversationFrame::Conversation {
                    metadata: Box::new(metadata),
                    etag,
                });
            }
        }
        if let (Some(held), Some(saved)) = (&mut shown.saved, saved) {
            let etag = etag_of(&saved);
            if *held != etag {
                held.clone_from(&etag);
                frames.push(ConversationFrame::Saved { saved, etag });
            }
        }
        if let (Some(held), Some(workflows)) = (&mut shown.workflows, workflows) {
            let etag = etag_of(&workflows);
            if *held != etag {
                held.clone_from(&etag);
                frames.push(ConversationFrame::Workflows { workflows, etag });
            }
        }
        if let (Some(held), Some(shares)) = (&mut shown.shares, shares) {
            let etag = etag_of(&shares);
            if *held != etag {
                held.clone_from(&etag);
                frames.push(ConversationFrame::Shares { shares, etag });
            }
        }
        if let Some(draft) = draft {
            shown.draft = Some(draft.revision);
            frames.push(ConversationFrame::Draft { draft });
        }
        if !uploaded.is_empty() {
            frames.push(ConversationFrame::Attachments {
                attachments: uploaded.into_iter().map(|row| row.description()).collect(),
            });
        }
        if frames.is_empty() {
            return true;
        }
        // The frames' types serialize as JSON objects, which serde_json
        // never refuses.
        let text = serde_json::to_string(&frames).expect("a conversation frame serializes to JSON");
        page.send(text).await.is_ok()
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
        SavedDelivery::Message | SavedDelivery::Queue => ClientFrame::Send(SendMessage {
            message_id: turn,
            content,
            comments: Vec::new(),
        }),
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

/// A frame's JSON text. A frame that may carry many blocks, a rewrite's
/// patches, is serialized on the blocking pool, since a long one
/// would hold the shard's thread; none when that pool is gone with its
/// runtime, which is shutting down.
async fn serialize(frame: ServerFrame) -> Option<String> {
    let whole = match &frame {
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
pub(super) struct ConversationFiles {
    /// Weak: the shard owns the agent server that holds this resolver.
    shard: Weak<Shard>,
    conversation: ConversationId,
}

impl ConversationFiles {
    /// The resolver of the files of `shard`'s conversation `conversation`.
    pub(super) fn of(shard: &Rc<Shard>, conversation: ConversationId) -> Self {
        Self {
            shard: Rc::downgrade(shard),
            conversation,
        }
    }
}

impl ContentResolver for ConversationFiles {
    fn resolve<'a>(
        &'a self,
        files: Vec<FileReference>,
    ) -> LocalBoxFuture<'a, Result<ResolvedFiles, ContentError>> {
        Box::pin(async move {
            // A failure inside the backend has no cause of its own for the
            // user; it is shown with the lane's request ID.
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

/// A duration as the integer `_ms` fields of the log hold it.
fn millis(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// A frame's kind as the protocol names it, such as `send` or `steer`.
fn kind_name(kind: ClientFrameKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|name| name.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// The message a frame sends and how many files it names, for a frame that
/// sends one.
fn sent_facts(frame: &ClientFrame) -> Option<(String, usize)> {
    let (message, content) = match frame {
        ClientFrame::Send(send) => (send.message_id.to_string(), &send.content),
        ClientFrame::Steer { steer_id, content, .. } => (steer_id.to_string(), content),
        _ => return None,
    };
    let files = content
        .iter()
        .filter(|part| !matches!(part, ClientContent::Text { .. }))
        .count();
    Some((message, files))
}
