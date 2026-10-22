//! Generated conversation titles: one small request beside the first turn, on
//! the user's small model, which writes the title only while it is still the one
//! the request began from.
//!
//! A side question's card is titled the same way, after its first question. An
//! archive or the shard's close aborts them.
//!
//! The request runs beside the first turn instead of as a copy of the session,
//! as a compaction's summary does. That keeps the agent's system prompt, tools
//! and history out of a job that needs one message, and makes the title
//! independent of whether the first turn succeeds.
//!
//! The page learns the title as it learns a rename, from the conversation's
//! summary; there is no title event of its own.

use tracing::Instrument as _;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use demi_agent_server::title::{title_from_message, title_from_response, title_request};
use demi_backend_database::StorageError;
use demi_backend_database::control::ControlService;
use demi_backend_database::conversation_index::ConversationRecord;
use demi_backend_page_sync::{Part, UserMarks};
use demi_conversation_socket_protocol::ClientContent;
use demi_shared_types::{ModelSelection, NodeId};
use demi_web_api_protocol::ids::ConversationId;
use futures_util::future::LocalBoxFuture;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::small_requests::{SmallRequestFailure, SmallRequests};
use crate::shard::Shard;

/// What one request reads, and the state it began from.
pub(crate) struct TitleRequest {
    /// The text of the first message.
    pub message: String,
    /// The title as the request begins; only that title is replaced.
    pub from: String,
}

/// The title requests of a shard's conversations.
pub(crate) struct Titles {
    small_requests: Rc<SmallRequests>,
    control: ControlService,
    /// The user's pages, which learn the title a request wrote.
    marks: UserMarks,
    /// Off, no request starts: titles stay the ones the first messages give.
    enabled: bool,
    /// What stops each request in flight. A conversation has at most one,
    /// since only its first message's send starts one.
    in_flight: Rc<RefCell<HashMap<ConversationId, CancellationToken>>>,
    /// What stops each side question's request in flight, by its
    /// conversation and its node.
    sides_in_flight: Rc<RefCell<HashMap<(ConversationId, NodeId), CancellationToken>>>,
}

/// What writes a side question's generated title, and answers whether it
/// wrote it.
pub(crate) type WriteSideTitle = Box<dyn FnOnce(String) -> LocalBoxFuture<'static, Result<(), StorageError>>>;

impl Titles {
    pub(super) fn new(
        small_requests: Rc<SmallRequests>,
        control: ControlService,
        marks: UserMarks,
        enabled: bool,
    ) -> Self {
        Self {
            small_requests,
            control,
            marks,
            enabled,
            in_flight: Rc::default(),
            sides_in_flight: Rc::default(),
        }
    }

    /// Starts a request titling the side question `side` of `conversation`
    /// after `message`, the words it asks about with its question, on
    /// `tasks`, and returns at once; `write` writes the title the request
    /// brought. A failure is logged and writes nothing.
    pub(crate) fn start_side(
        &self,
        tasks: &TaskTracker,
        conversation: ConversationId,
        side: NodeId,
        selection: ModelSelection,
        message: String,
        write: WriteSideTitle,
    ) {
        if !self.enabled {
            return;
        }
        let Some(small) = title_request(&message) else {
            return;
        };
        let cancel = CancellationToken::new();
        let key = (conversation.clone(), side);
        self.sides_in_flight.borrow_mut().insert(key.clone(), cancel.clone());
        let small_requests = self.small_requests.clone();
        let in_flight = self.sides_in_flight.clone();
        tasks.spawn_local(async move {
            let started = std::time::Instant::now();
            let titled = async {
                let answer = small_requests.run(&conversation, &selection, small, cancel.clone()).await?;
                Ok::<&'static str, TitleFailure>(match answer.as_deref().and_then(title_from_response) {
                    Some(_) if cancel.is_cancelled() => "aborted",
                    Some(title) => {
                        write(title).await?;
                        "set"
                    }
                    None => "empty",
                })
            };
            let titled = titled.await;
            log_title(&conversation, Some(&key.1), &selection, started, &titled);
            in_flight.borrow_mut().remove(&key);
        }
        .in_current_span());
    }

    /// Starts a request of the conversation `id` on `tasks` and returns at
    /// once. A failure is logged and writes nothing; either way the
    /// request's end marks the conversation, so its pages read the title it
    /// ended with.
    pub fn start(
        &self,
        tasks: &TaskTracker,
        id: ConversationId,
        selection: ModelSelection,
        request: TitleRequest,
    ) {
        if !self.enabled {
            return;
        }
        let cancel = CancellationToken::new();
        self.in_flight.borrow_mut().insert(id.clone(), cancel.clone());
        let small_requests = self.small_requests.clone();
        let control = self.control.clone();
        let marks = self.marks.clone();
        let in_flight = self.in_flight.clone();
        // The title logs under what asked for it, such as a socket's send.
        tasks.spawn_local(async move {
            let started = std::time::Instant::now();
            let titled = generate(&small_requests, &control, &id, &selection, request, &cancel).await;
            log_title(&id, None, &selection, started, &titled);
            in_flight.borrow_mut().remove(&id);
            marks.mark(Part::Conversation(id));
        }
        .in_current_span());
    }

    /// Aborts the requests of the conversation `id`, as its archive does.
    pub fn abort(&self, id: &ConversationId) {
        if let Some(cancel) = self.in_flight.borrow().get(id) {
            cancel.cancel();
        }
        for ((conversation, _), cancel) in self.sides_in_flight.borrow().iter() {
            if conversation == id {
                cancel.cancel();
            }
        }
    }

    /// Aborts every request, as the shard's close does.
    pub(crate) fn abort_all(&self) {
        for cancel in self.in_flight.borrow().values() {
            cancel.cancel();
        }
        for cancel in self.sides_in_flight.borrow().values() {
            cancel.cancel();
        }
    }
}

/// Why a request wrote nothing.
#[derive(Debug, thiserror::Error)]
enum TitleFailure {
    #[error(transparent)]
    Request(#[from] SmallRequestFailure),
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// The line that ends a title request: whether it set a title, found no text
/// to title or an answer to take one from, was stopped, or failed.
fn log_title(
    conversation: &ConversationId,
    side: Option<&NodeId>,
    selection: &ModelSelection,
    started: std::time::Instant,
    titled: &Result<&'static str, TitleFailure>,
) {
    let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let side = side.map(tracing::field::display);
    let (outcome, error) = match titled {
        Ok(outcome) => (*outcome, None),
        Err(failure) => ("failed", Some(failure.to_string())),
    };
    let provider = selection.provider_id.as_str();
    let model = selection.model.id.as_str();
    if error.is_some() {
        tracing::warn!(event = "title.ended", %conversation, side, provider, model, ms, outcome, error, "a title request ended");
    } else {
        tracing::info!(event = "title.ended", %conversation, side, provider, model, ms, outcome, "a title request ended");
    }
}

/// Asks the small model for a title of the conversation `id`, which infers
/// with `selection`, and writes it, unless the request is stopped first.
/// Answers how it ended: `set`, `empty` for a message or an answer with no
/// title in it, or `aborted`.
async fn generate(
    small_requests: &SmallRequests,
    control: &ControlService,
    id: &ConversationId,
    selection: &ModelSelection,
    request: TitleRequest,
    cancel: &CancellationToken,
) -> Result<&'static str, TitleFailure> {
    // A message without text leaves nothing to title.
    let Some(small) = title_request(&request.message) else {
        return Ok("empty");
    };
    let answer = small_requests
        .run(id, selection, small, cancel.clone())
        .await?;
    let Some(title) = answer.as_deref().and_then(title_from_response) else {
        return Ok("empty");
    };
    if cancel.is_cancelled() {
        return Ok("aborted");
    }
    control
        .generated_title(id.clone(), title, request.from)
        .await?;
    Ok("set")
}

impl Shard {
    /// Titles the user's conversation `record` after its first message, when
    /// its title is still the placeholder: the message's start at once, and
    /// a request beside the first turn to the user's small model.
    pub(crate) async fn title_first_message(
        &self,
        record: &ConversationRecord,
        content: &[ClientContent],
    ) -> Result<(), StorageError> {
        let text = content
            .iter()
            .find_map(|part| match part {
                ClientContent::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .unwrap_or_default();
        let title = title_from_message(text);
        // A first message without text starts none; the placeholder stays.
        if title.is_empty() {
            return Ok(());
        }
        let titled = self
            .services()
            .control
            .title_from_first_message(record.id.clone(), title.clone())
            .await?;
        if let (true, Some(selection)) = (titled, &record.model) {
            let request = TitleRequest {
                message: text.to_owned(),
                from: title,
            };
            self.titles()
                .start(self.tasks(), record.id.clone(), selection.clone(), request);
        }
        Ok(())
    }
}
