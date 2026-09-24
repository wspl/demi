//! Generated conversation titles (`product.md` § Conversation titles): one
//! model request beside the first turn, or when the user asks, through the
//! same metered runtime a turn uses, which writes the title only while it is
//! still the one the request began from. The shard keeps one request in
//! flight per conversation; asking again while one runs joins it, and an
//! archive or the shard's close aborts it.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use demi_agent::ProviderResolver as _;
use demi_agent::title::{request_title, title_from_message};
use demi_agent_protocol::ClientContent;
use demi_core::{Block, ModelSelection, UserContentBlock};
use demi_web_api::ids::{ConversationId, ProviderId};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::providers::ConversationProviders;
use super::root_of;
use crate::shard::Shard;
use crate::storage::StorageError;
use crate::storage::control::ControlService;
use crate::storage::conversation_index::ConversationRecord;
use crate::storage::tree;

/// What one request reads, and the state it began from.
pub(crate) struct TitleRequest {
    /// The text of the user's messages, oldest first.
    pub(crate) messages: Vec<String>,
    /// The title as the request begins; only that title is replaced.
    pub(crate) from: String,
    /// How many messages the user had sent as the request begins.
    pub(crate) seen: u64,
}

/// A request in flight: which one, and what stops it.
struct InFlight {
    request: u64,
    cancel: CancellationToken,
}

/// The title requests of a shard's conversations.
pub(crate) struct Titles {
    providers: Rc<ConversationProviders>,
    control: ControlService,
    /// Off, no request starts: titles stay the ones the first messages give.
    enabled: bool,
    in_flight: Rc<RefCell<HashMap<ConversationId, InFlight>>>,
    /// Numbers the requests, so a request that ends removes only itself.
    started: Cell<u64>,
}

impl Titles {
    pub(super) fn new(providers: Rc<ConversationProviders>, control: ControlService, enabled: bool) -> Self {
        Self {
            providers,
            control,
            enabled,
            in_flight: Rc::default(),
            started: Cell::new(0),
        }
    }

    /// Whether a request of the conversation `id` is in flight.
    pub(crate) fn generating(&self, id: &ConversationId) -> bool {
        self.in_flight
            .borrow()
            .get(id)
            .is_some_and(|request| !request.cancel.is_cancelled())
    }

    /// Starts a request of the conversation `id` on `tasks` and returns at
    /// once, unless one is in flight already, which this joins. A failure
    /// is logged and writes nothing.
    pub(crate) fn start(&self, tasks: &TaskTracker, id: ConversationId, selection: ModelSelection, request: TitleRequest) {
        if !self.enabled || self.generating(&id) {
            return;
        }
        let number = self.started.get() + 1;
        self.started.set(number);
        let cancel = CancellationToken::new();
        self.in_flight.borrow_mut().insert(
            id.clone(),
            InFlight {
                request: number,
                cancel: cancel.clone(),
            },
        );
        let providers = self.providers.clone();
        let control = self.control.clone();
        let in_flight = self.in_flight.clone();
        tasks.spawn_local(async move {
            if let Err(failure) = generate(&providers, &control, &id, &selection, request, &cancel).await {
                tracing::warn!(conversation = %id, %failure, "a title request wrote nothing");
            }
            let mut in_flight = in_flight.borrow_mut();
            if in_flight.get(&id).is_some_and(|request| request.request == number) {
                in_flight.remove(&id);
            }
        });
    }

    /// Aborts the request of the conversation `id`, as its archive does.
    pub(crate) fn abort(&self, id: &ConversationId) {
        if let Some(request) = self.in_flight.borrow().get(id) {
            request.cancel.cancel();
        }
    }

    /// Aborts every request, as the shard's close does.
    pub(crate) fn abort_all(&self) {
        for request in self.in_flight.borrow().values() {
            request.cancel.cancel();
        }
    }
}

/// Why a request wrote nothing.
#[derive(Debug, thiserror::Error)]
enum TitleFailure {
    #[error(transparent)]
    Provider(#[from] demi_agent::ResolveError),
    #[error(transparent)]
    Request(#[from] demi_agent::title::TitleError),
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// Asks `selection`'s model for a title of the conversation `id` and writes
/// it, unless the request is stopped first.
async fn generate(
    providers: &ConversationProviders,
    control: &ControlService,
    id: &ConversationId,
    selection: &ModelSelection,
    request: TitleRequest,
    cancel: &CancellationToken,
) -> Result<(), TitleFailure> {
    let mut runtime = providers.runtime(&root_of(id), selection).await?;
    let request_id = uuid::Uuid::new_v4().to_string();
    let answer = request_title(
        &mut *runtime,
        id.as_str(),
        request_id,
        selection,
        &request.messages,
        cancel.clone(),
    )
    .await;
    runtime.close().await;
    let Some(title) = answer? else {
        return Ok(());
    };
    if cancel.is_cancelled() {
        return Ok(());
    }
    control.generated_title(id.clone(), title, request.from, request.seen).await?;
    Ok(())
}

/// Why a title request was not started.
#[derive(Debug, thiserror::Error)]
pub(crate) enum TitleRefusal {
    #[error("No such conversation")]
    NotFound,
    #[error("Restore the conversation before changing it")]
    Archived,
    #[error("No such provider")]
    ProviderNotFound,
    #[error("The conversation has no message to title")]
    NoMessages,
    #[error(transparent)]
    Storage(#[from] StorageError),
}

impl Shard {
    /// Titles the user's conversation `record` after its first message, when
    /// its title is still the placeholder: the message's start at once, and
    /// a request beside the first turn to the model the conversation's
    /// session infers with. `seen` counts the message.
    pub(crate) async fn title_first_message(
        &self,
        record: &ConversationRecord,
        content: &[ClientContent],
        seen: u64,
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
        let tree = self.agent().tree(&root_of(&record.id));
        if let (true, Some(tree)) = (titled, tree) {
            let request = TitleRequest {
                messages: vec![text.to_owned()],
                from: title,
                seen,
            };
            self.titles()
                .start(self.tasks(), record.id.clone(), tree.root().session().model(), request);
        }
        Ok(())
    }

    /// Asks `selection`'s model for a new title of the user's conversation
    /// `id` from every message the user sent (`POST
    /// /conversations/:id/title`); a request in flight is joined.
    pub(crate) async fn ask_title(&self, id: &ConversationId, selection: ModelSelection) -> Result<(), TitleRefusal> {
        let services = self.services();
        let record = services.control.conversation(id.clone()).await?;
        let Some(record) = record.filter(|record| record.owner == *self.user()) else {
            return Err(TitleRefusal::NotFound);
        };
        if record.archived {
            return Err(TitleRefusal::Archived);
        }
        let provider = ProviderId::try_from(selection.provider_id.as_str()).map_err(|_| TitleRefusal::ProviderNotFound)?;
        if services.vault.visible(self.user(), &provider).await?.is_none() {
            return Err(TitleRefusal::ProviderNotFound);
        }
        // The live tree's history is newer than its last save.
        let blocks = match self.agent().tree(&root_of(&record.id)) {
            Some(tree) => tree.root().session().transcript().blocks,
            None => services
                .conversations
                .read(&record.id, tree::history)
                .await?
                .unwrap_or_default()
                .blocks,
        };
        let messages: Vec<String> = blocks.iter().filter_map(message_text).collect();
        if messages.is_empty() {
            return Err(TitleRefusal::NoMessages);
        }
        let request = TitleRequest {
            messages,
            from: record.title,
            seen: record.user_messages,
        };
        self.titles().start(self.tasks(), record.id, selection, request);
        Ok(())
    }
}

/// The text of a message the user sent, without its files; none for any
/// other block, or a message without text.
fn message_text(block: &Block) -> Option<String> {
    let content = match block {
        Block::User(user) => &user.content,
        Block::Steer(steer) => &steer.content,
        _ => return None,
    };
    let texts: Vec<&str> = content
        .iter()
        .filter_map(|part| match part {
            UserContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    let text = texts.join("\n");
    (!text.trim().is_empty()).then_some(text)
}
