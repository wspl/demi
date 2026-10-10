//! Deleting a conversation (`storage.md` § Deleting a conversation,
//! `web-api.md` § Sidebar mutations, read state and page synchronization):
//! nothing refuses it. Its work stops as Stop stops it, its subagents and
//! commands included, and its streams and Host operations end as an archive
//! ends them; then one transaction of the control database removes its
//! records and records the deletion as pending, its tree closes, its Hosts
//! hear the conversation release, which closes its tabs in the conversation
//! browser, its database goes, and the pending record last. The owner's blob
//! namespace is collected after it (`lifecycle::collection`).

use demi_backend_database::StorageError;
use demi_backend_database::control::ControlService;
use demi_backend_host_access::root_of;
use demi_backend_page_sync::Part;
use demi_web_api_protocol::ids::ConversationId;

use crate::shard::{Shard, Shards};

/// Finishes each deletion an earlier backend left pending (`storage.md`
/// § Deleting a conversation), in its owner's shard, and collects the
/// owner's blobs after it. A deletion that fails again stays pending for the
/// next start; only listing them fails the call.
pub async fn finish_deletions(control: &ControlService, shards: &Shards) -> Result<(), StorageError> {
    for pending in control.pending_deletions().await? {
        let id = pending.id;
        let finished = shards
            .of(&pending.owner)
            .call(move |shard, _| async move {
                let finished = shard.finish_deletion(&id).await;
                if finished.is_ok() {
                    shard.collect_blobs();
                }
                finished.map_err(|error| (id, error))
            })
            .await;
        match finished {
            Ok(Ok(())) => {}
            Ok(Err((id, error))) => {
                tracing::error!(conversation = %id, error = &error as &dyn std::error::Error, "a pending deletion was not finished");
            }
            // The backend is shutting down; the next start finishes the rest.
            Err(_) => return Ok(()),
        }
    }
    Ok(())
}

impl Shard {
    /// Deletes the user's conversation `id`; false when the user has no such
    /// conversation, a deletion already begun included. A failure before its
    /// records go leaves the conversation as it was; one after leaves the
    /// deletion pending, and the next start finishes it.
    pub async fn delete_conversation(&self, id: &ConversationId) -> Result<bool, StorageError> {
        let services = self.services();
        let Some(record) = services
            .control
            .conversation(id.clone())
            .await?
            .filter(|record| record.owner == *self.user())
        else {
            return Ok(false);
        };
        let root = root_of(&record.id);
        let slot = self.conversations().slot(&record.id);
        // The work stops first: the tree takes no action, the root's stops as
        // Stop stops it, its children are aborted and its commands end, so
        // the file gate their jobs hold comes free.
        let mut stopped = match self.agent().tree(&root) {
            Some(tree) => Some(tree.interrupt("the conversation is being deleted").await),
            None => None,
        };
        let _transfers = slot.transfers.close().await;
        // Every Host operation and frame ahead of it ends; those that arrive
        // later wait behind it and then find no conversation.
        let _files = slot.file_gate().reserve().await;
        // A frame ahead of the reservation may have opened the tree.
        if stopped.is_none()
            && let Some(tree) = self.agent().tree(&root)
        {
            stopped = Some(tree.interrupt("the conversation is being deleted").await);
        }
        let host = self.host_shard();
        let devices = host.release_devices(&record).await?;
        // Step 1: from this commit on, no request finds the conversation.
        let Some(id) = services
            .control
            .delete_conversation(self.user().clone(), record.id)
            .await?
        else {
            return Ok(false);
        };
        self.mark(Part::Conversation(id.clone()));
        self.agent().close_tree(&root).await;
        drop(stopped);
        self.titles().abort(&id);
        self.stop_idle(&id);
        // Step 2: its Hosts let go of what they hold for it, its Chrome with
        // its tabs among them.
        host.release_on_each(&id, &devices).await;
        // The conversation is gone for every request already, so a failure
        // of what is left refuses nothing: the deletion stays pending.
        if let Err(error) = self.finish_deletion(&id).await {
            tracing::error!(conversation = %id, error = &error as &dyn std::error::Error, "the deletion stays pending until the next start");
            return Ok(true);
        }
        self.collect_blobs();
        Ok(true)
    }

    /// Steps 3 and 4 of the deletion of `id`, whose records are gone: its
    /// database goes, then the pending record. Its rows in the search index
    /// go with the summary mark step 1 raised, since the indexer drops the
    /// rows of a conversation it no longer finds (`search.rs`). A start runs them for each deletion it finds pending; the
    /// Hosts need no release then, since a runner that lost its connection
    /// to the backend ended what it held for every conversation
    /// (`resource-lifecycle.md` § Conversation release).
    pub async fn finish_deletion(&self, id: &ConversationId) -> Result<(), StorageError> {
        let services = self.services();
        services.conversations.remove(id).await?;
        services.control.finish_deletion(id.clone()).await
    }
}
