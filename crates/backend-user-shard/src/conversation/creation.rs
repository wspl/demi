//! A conversation's creation (`web-api.md` § Conversation creation and
//! Fork): one request makes it with what it starts with, its settings and
//! the devices it attaches, each checked as its own change would check it
//! before anything is created, so a refused one creates nothing.

use demi_backend_database::conversation_index::{AttachedHostRecord, ConversationStart, Creation};
use demi_backend_host_access::transition::ChangeRefusal;
use demi_backend_page_sync::Part;
use demi_web_api_protocol::conversations::{ConversationTarget, CreateConversation, ModelSettings};

use crate::shard::Shard;

impl Shard {
    /// Creates the user's conversation `request.id` as the request says. The
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
        let start = self.conversation_start(request).await?;
        let created = control
            .create_conversation(self.user().clone(), id, start)
            .await?;
        if let Creation::Created(record) = &created {
            self.mark(Part::Conversation(record.id.clone()));
            self.mark(Part::ConversationOrder);
        }
        Ok(created)
    }

    /// What the conversation starts with, each part checked as a patch or an
    /// attach checks it.
    async fn conversation_start(
        &self,
        request: CreateConversation,
    ) -> Result<ConversationStart, ChangeRefusal> {
        let owner = self.user();
        let host = self.host_shard();
        let target = request
            .target
            .unwrap_or(ConversationTarget::Cloud { path: None });
        host.check_destination(owner, &target).await?;
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
        let primary = host
            .resolve(&request.id, owner, &target)
            .await?
            .device()
            .cloned();
        let control = &self.services().control;
        let mut hosts = Vec::with_capacity(request.hosts.len());
        for new in request.hosts {
            let device = control
                .device(new.device_id)
                .await?
                .filter(|device| device.user == *owner)
                .ok_or(ChangeRefusal::DeviceNotFound)?;
            if primary.as_ref() == Some(&device.id) {
                return Err(ChangeRefusal::HostIsPrimary);
            }
            hosts.push(AttachedHostRecord {
                name: new.name.map_or(device.name, |name| name.into_string()),
                device: device.id,
                cwd: None,
            });
        }
        Ok(ConversationStart {
            title: request.title.map(|title| title.into_string()),
            pinned: request.pinned.unwrap_or(false),
            target,
            model,
            hosts,
        })
    }
}
