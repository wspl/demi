//! Suggested replies of the user's conversations
//! (`demi_agent_server::suggested_replies`): as a root rests after work that
//! ended a turn with an answer written while it ran, one small request on
//! the user's small model asks for the replies the user is likely to send
//! next.
//!
//! An answer written before the work started asks for none: a tree only
//! opened, as a page opening an old conversation opens it; a version of an
//! answer brought back; a compaction; a turn that ended without text. What a
//! request answered is kept in memory with the answer it follows, for the
//! conversation's metadata, until the next answer's replaces it, an archive
//! or a deletion; a restart of the backend forgets it, as the next answer
//! brings new ones, and the page shows it only under that answer. Work that
//! starts while a request runs cancels it, as does the shard's close. The
//! pages learn what a request kept from the conversation's metadata.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use demi_agent_server::small_request::SmallRequest;
use demi_agent_server::suggested_replies::{answer_of, suggestion_request, suggestions_from_response};
use demi_backend_database::StorageError;
use demi_backend_database::control::ControlService;
use demi_backend_host_access::root_of;
use demi_backend_page_sync::{Part, UserMarks};
use demi_shared_types::{BlockId, ModelSelection, Timestamp};
use demi_web_api_protocol::conversations::SuggestedReplies as Kept;
use demi_web_api_protocol::ids::{ConversationId, UserId};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::Instrument as _;

use super::small_requests::{SmallRequestFailure, SmallRequests};
use crate::shard::Shard;

/// The suggested replies of a shard's conversations.
pub(crate) struct SuggestedReplies {
    small_requests: Rc<SmallRequests>,
    control: ControlService,
    /// The user's pages, which learn what a request kept.
    marks: UserMarks,
    user: UserId,
    /// When each running tree started to run: only an answer written since
    /// asks for suggestions.
    working: RefCell<HashMap<ConversationId, Timestamp>>,
    /// What each conversation's latest answer offers.
    kept: Rc<RefCell<HashMap<ConversationId, Kept>>>,
    /// Each conversation's request in flight, by its number, which tells it
    /// from a request it replaced.
    in_flight: Rc<RefCell<HashMap<ConversationId, (u64, CancellationToken)>>>,
    /// The number of the next request.
    next: Cell<u64>,
}

impl SuggestedReplies {
    pub(super) fn new(small_requests: Rc<SmallRequests>, control: ControlService, marks: UserMarks, user: UserId) -> Self {
        Self {
            small_requests,
            control,
            marks,
            user,
            working: RefCell::default(),
            kept: Rc::default(),
            in_flight: Rc::default(),
            next: Cell::new(0),
        }
    }

    /// What the conversation `id`'s latest answer offers, for its metadata.
    pub(crate) fn kept(&self, id: &ConversationId) -> Option<Kept> {
        self.kept.borrow().get(id).cloned()
    }

    /// Forgets what the conversation `id` offered and stops its request, as
    /// its archive or its deletion does.
    pub(crate) fn abort(&self, id: &ConversationId) {
        self.stop(id);
        self.kept.borrow_mut().remove(id);
    }

    /// Stops the conversation `id`'s request, as work that starts does.
    fn stop(&self, id: &ConversationId) {
        if let Some((_, cancel)) = self.in_flight.borrow_mut().remove(id) {
            cancel.cancel();
        }
    }

    /// Stops every request, as the shard's close does.
    pub(crate) fn abort_all(&self) {
        for (_, cancel) in self.in_flight.borrow_mut().drain().map(|(_, request)| request) {
            cancel.cancel();
        }
    }

    /// Starts the request for the answer `answer` of the conversation `id`,
    /// which infers with `selection`, on `tasks`, and returns at once. The
    /// user's switch is read as it starts; off, it asks nothing. A failure
    /// is logged and keeps none; either way its end marks the conversation.
    fn start(&self, tasks: &TaskTracker, id: ConversationId, answer: BlockId, selection: ModelSelection, request: SmallRequest) {
        let cancel = CancellationToken::new();
        let number = self.next.get();
        self.next.set(number + 1);
        if let Some((_, earlier)) = self.in_flight.borrow_mut().insert(id.clone(), (number, cancel.clone())) {
            earlier.cancel();
        }
        let small_requests = self.small_requests.clone();
        let control = self.control.clone();
        let user = self.user.clone();
        let marks = self.marks.clone();
        let kept = self.kept.clone();
        let in_flight = self.in_flight.clone();
        tasks.spawn_local(
            async move {
                let started = std::time::Instant::now();
                let ended = async {
                    if !control.preferences(user).await?.suggests_replies() {
                        return Ok::<_, Failure>("off");
                    }
                    let response = small_requests.run(&id, &selection, request, cancel.clone()).await?;
                    // Stopped, or replaced by a later request: what it brought is stale.
                    let current = in_flight.borrow().get(&id).is_some_and(|(of, _)| *of == number);
                    if cancel.is_cancelled() || !current {
                        return Ok("aborted");
                    }
                    let replies = response.as_deref().map(suggestions_from_response).unwrap_or_default();
                    let outcome = if replies.is_empty() { "none" } else { "kept" };
                    kept.borrow_mut().insert(id.clone(), Kept { block_id: answer.clone(), replies });
                    Ok(outcome)
                }
                .await;
                log(&id, &selection, started, &ended);
                let mut in_flight = in_flight.borrow_mut();
                if in_flight.get(&id).is_some_and(|(of, _)| *of == number) {
                    in_flight.remove(&id);
                }
                marks.mark(Part::Conversation(id));
            }
            .in_current_span(),
        );
    }
}

/// Why a request kept nothing.
#[derive(Debug, thiserror::Error)]
enum Failure {
    #[error(transparent)]
    Request(#[from] SmallRequestFailure),
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// The line that ends a request: whether it kept suggestions, found none,
/// was off or stopped, or failed.
fn log(conversation: &ConversationId, selection: &ModelSelection, started: std::time::Instant, ended: &Result<&'static str, Failure>) {
    let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let provider = selection.provider_id.as_str();
    let model = selection.model.id.as_str();
    match ended {
        Ok(outcome) => tracing::info!(event = "suggested_replies.ended", %conversation, provider, model, ms, outcome, "a suggested replies request ended"),
        Err(error) => tracing::warn!(event = "suggested_replies.ended", %conversation, provider, model, ms, outcome = "failed", error = %error, "a suggested replies request ended"),
    }
}

impl Shard {
    /// Follows the conversation's status: as its tree starts to run, stops
    /// its request and notes when; as it rests, asks for suggestions when the
    /// latest turn ended with an answer written since. Called on each change
    /// of the tree's status.
    pub(crate) fn follow_suggested_replies(&self, conversation: &ConversationId) {
        if self.is_closing() {
            return;
        }
        let replies = self.suggested_replies();
        let Some(tree) = self.agent().tree(&root_of(conversation)) else {
            replies.working.borrow_mut().remove(conversation);
            return;
        };
        if tree.is_running() {
            replies.stop(conversation);
            let now = self.services().clock.now();
            replies.working.borrow_mut().entry(conversation.clone()).or_insert(now);
            return;
        }
        // Only a rest after work: a tree only opened asks for none.
        let Some(started) = replies.working.borrow_mut().remove(conversation) else {
            return;
        };
        let session = tree.root().session();
        let transcript = session.transcript();
        let Some(answer) = answer_of(&transcript.blocks).filter(|text| text.created_at >= started) else {
            return;
        };
        let Some(request) = suggestion_request(&transcript.blocks, &answer.id) else {
            return;
        };
        replies.start(self.tasks(), conversation.clone(), answer.id.clone(), session.model(), request);
    }
}
