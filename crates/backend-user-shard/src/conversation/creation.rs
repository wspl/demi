//! A conversation's creation.
//!
//! One request makes the conversation with what it starts with, its target and
//! its settings. Each is checked as its own change would check it before
//! anything is created, so a refused one creates nothing.
//!
//! A first message the request carries is admitted once the record exists
//! (`create_conversation_sending`), as the conversation socket admits a
//! `send` (`socket.rs`, `admit_first_message`): the creation does not wait
//! for the page's socket, and the conversation exists whether or not the
//! session takes the message
//! (`docs/decisions/0045-a-new-conversation-sends-with-its-creation.md`).
//!
//! Its directory is selected as a move's is, in the transaction that creates
//! it, which makes the directory's project when there is none, by the
//! directory's name on its Host (`crates/backend-database/src/selection.rs`).

use demi_backend_database::conversation_index::{ConversationStart, Creation};
use demi_backend_host_access::transition::ChangeRefusal;
use demi_backend_page_sync::Part;
use demi_web_api_protocol::conversations::{
    ConversationTarget, CreateConversation, FirstMessageAnswer, ModelSettings,
};

use crate::shard::Shard;

impl Shard {
    /// Creates the conversation as the request says, then admits the first
    /// message it carries, if any, in the session the creation opens
    /// (`admit_first_message`). The conversation exists whatever the message's
    /// answer is, and one that already existed takes the message as a repeated
    /// request would: once.
    pub async fn create_conversation_sending(
        &self,
        mut request: CreateConversation,
    ) -> Result<(Creation, Option<FirstMessageAnswer>), ChangeRefusal> {
        let first_message = request.first_message.take();
        let creation = self.create_conversation(request).await?;
        let answer = match (&creation, first_message) {
            (Creation::Created(record) | Creation::Existing(record), Some(message)) => {
                Some(self.this().admit_first_message(&record.id, message).await)
            }
            _ => None,
        };
        Ok((creation, answer))
    }

    /// Creates the user's conversation `request.id` as the request says,
    /// without its first message, which the caller admits. The
    /// user's conversation of that id answers as it is, and nothing of the
    /// request applies to it.
    pub async fn create_conversation(
        &self,
        request: CreateConversation,
    ) -> Result<Creation, ChangeRefusal> {
        let control = &self.services().control;
        if let Some(record) = control.conversation(request.id.clone()).await? {
            return Ok(if record.owner == *self.user() {
                Creation::Existing(record)
            } else {
                Creation::Unavailable
            });
        }
        let id = request.id.clone();
        let mut start = self.conversation_start(request).await?;
        // Its directory is the one its Host names (§ Resolve a target).
        start.target = self.host_shard().resolved_destination(start.target).await?;
        let (created, made) = control
            .create_conversation_selecting(self.user().clone(), id, start)
            .await?;
        if made.is_some() {
            self.mark(Part::Workspaces);
        }
        match created {
            Creation::Created(record) => {
                self.mark(Part::Conversation(record.id.clone()));
                self.mark(Part::ConversationOrder);
                Ok(Creation::Created(record))
            }
            Creation::Unselectable(reason) => Err(reason.into()),
            created => Ok(created),
        }
    }

    /// What the conversation starts with, each part checked as a patch
    /// checks it.
    async fn conversation_start(
        &self,
        request: CreateConversation,
    ) -> Result<ConversationStart, ChangeRefusal> {
        let target = request
            .target
            .unwrap_or(ConversationTarget::Cloud { path: None });
        let model = match request.model {
            Some(choice) => Some(
                self.chosen_selection(&ModelSettings {
                    provider_id: choice.provider_id,
                    model_id: choice.model_id,
                    thinking_effort: request.thinking_effort,
                    service_tier_id: request.service_tier_id.flatten(),
                })
                .await?,
            ),
            None if request.thinking_effort.is_some() || request.service_tier_id.is_some() => {
                return Err(ChangeRefusal::ModelNotSelected);
            }
            None => None,
        };
        Ok(ConversationStart {
            title: request.title.map(|title| title.into_string()),
            pinned: request.pinned.unwrap_or(false),
            target,
            model,
            workflow: request.workflow.unwrap_or_default(),
        })
    }
}
