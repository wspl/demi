//! Generated conversation titles (`conversations.md` § Conversation titles): one
//! model request beside the first turn, through the same metered runtime a
//! turn uses, which writes the title only while it is still the one the
//! request began from. An archive or the shard's close aborts it.

use tracing::Instrument as _;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use demi_agent_server::ProviderResolver as _;
use demi_agent_server::title::{request_title, title_from_message};
use demi_backend_database::StorageError;
use demi_backend_database::control::ControlService;
use demi_backend_database::conversation_index::ConversationRecord;
use demi_backend_page_sync::{Part, UserMarks};
use demi_conversation_socket_protocol::ClientContent;
use demi_shared_types::ModelSelection;
use demi_web_api_protocol::ids::ConversationId;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::providers::ConversationProviders;
use crate::shard::Shard;
use demi_backend_host_access::root_of;

/// What one request reads, and the state it began from.
pub(crate) struct TitleRequest {
    /// The text of the first message.
    pub message: String,
    /// The title as the request begins; only that title is replaced.
    pub from: String,
}

/// The title requests of a shard's conversations.
pub(crate) struct Titles {
    providers: Rc<ConversationProviders>,
    control: ControlService,
    /// The user's pages, which learn the title a request wrote.
    marks: UserMarks,
    /// Off, no request starts: titles stay the ones the first messages give.
    enabled: bool,
    /// What stops each request in flight. A conversation has at most one,
    /// since only its first message's send starts one.
    in_flight: Rc<RefCell<HashMap<ConversationId, CancellationToken>>>,
}

impl Titles {
    pub(super) fn new(
        providers: Rc<ConversationProviders>,
        control: ControlService,
        marks: UserMarks,
        enabled: bool,
    ) -> Self {
        Self {
            providers,
            control,
            marks,
            enabled,
            in_flight: Rc::default(),
        }
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
        let providers = self.providers.clone();
        let control = self.control.clone();
        let marks = self.marks.clone();
        let in_flight = self.in_flight.clone();
        // The title logs under what asked for it, such as a socket's send.
        tasks.spawn_local(async move {
            if let Err(failure) =
                generate(&providers, &control, &id, &selection, request, &cancel).await
            {
                tracing::warn!(conversation = %id, %failure, "a title request wrote nothing");
            }
            in_flight.borrow_mut().remove(&id);
            marks.mark(Part::Conversation(id));
        }
        .in_current_span());
    }

    /// Aborts the request of the conversation `id`, as its archive does.
    pub fn abort(&self, id: &ConversationId) {
        if let Some(cancel) = self.in_flight.borrow().get(id) {
            cancel.cancel();
        }
    }

    /// Aborts every request, as the shard's close does.
    pub(crate) fn abort_all(&self) {
        for cancel in self.in_flight.borrow().values() {
            cancel.cancel();
        }
    }
}

/// Why a request wrote nothing.
#[derive(Debug, thiserror::Error)]
enum TitleFailure {
    #[error(transparent)]
    Provider(#[from] demi_agent_server::ResolveError),
    #[error(transparent)]
    Request(#[from] demi_agent_server::title::TitleError),
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
        &request.message,
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
    control
        .generated_title(id.clone(), title, request.from)
        .await?;
    Ok(())
}

impl Shard {
    /// Titles the user's conversation `record` after its first message, when
    /// its title is still the placeholder: the message's start at once, and
    /// a request beside the first turn to the conversation's model.
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
