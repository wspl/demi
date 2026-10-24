//! Deleting a conversation (`crates/backend-database/src/deletions.rs`).
//!
//! Nothing refuses a deletion. Its work stops as Stop stops it, its subagents
//! and commands included, and its streams and Host operations end as an archive
//! ends them. Then, in order:
//!
//! 1. One transaction of the control database removes its records and records
//!    the deletion as pending.
//! 2. Its tree closes.
//! 3. Its connected Hosts are told to let go of it, which closes its tabs in
//!    the conversation browser; the others are told by their next `hello`,
//!    which names the conversations they hold.
//! 4. Its database goes.
//! 5. The pending record goes last.
//!
//! The owner's blob namespace is collected after it (`lifecycle::collection`).

use demi_backend_database::StorageError;
use demi_backend_database::control::ControlService;
use demi_backend_host_access::root_of;
use demi_backend_page_sync::Part;
use demi_web_api_protocol::ids::ConversationId;

use crate::shard::{Shard, Shards};

/// Finishes each deletion an earlier backend left pending
/// (`crates/backend-database/src/deletions.rs`), in its owner's shard, and
/// collects the owner's blobs after it. A deletion that fails again stays
/// pending for the next start; only listing them fails the call.
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
                tracing::error!(event = "conversation_deletion.failed", conversation = %id, stage = "finish", outcome = "next_start", error = &error as &dyn std::error::Error, "a pending deletion was not finished");
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
        let host = self.host_shard();
        let devices = host.release_devices(&record).await?;
        // Commands running on a Host whose runner is away are left to it: a
        // Host that may never return holds no file gate, and the deletion
        // waits for no stop that only that runner could report. The runner
        // stops them when it is back, as it lists jobs the backend has no
        // record of.
        for device in &devices {
            if !self.devices().connected(device) {
                slot.leave_jobs_on(device);
            }
        }
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
        // Its schedules go with its records; the shard forgets them after.
        let schedules = self.conversation_schedule_ids(&record.id).await;
        // Step 1: from this commit on, no request finds the conversation.
        let Some(id) = services
            .control
            .delete_conversation(self.user().clone(), record.id)
            .await?
        else {
            return Ok(false);
        };
        self.mark(Part::Conversation(id.clone()));
        // What the shard holds of its schedules stops.
        self.conversation_deleted_schedules(&schedules);
        self.agent().close_tree(&root).await;
        drop(stopped);
        self.titles().abort(&id);
        self.stop_idle(&id);
        // Step 3: its Hosts let go of what they hold for it, its Chrome with
        // its tabs among them; nothing waits for them.
        host.tell_release(&id, &devices, true);
        // The conversation is gone for every request already, so a failure
        // of what is left refuses nothing: the deletion stays pending.
        if let Err(error) = self.finish_deletion(&id).await {
            tracing::error!(event = "conversation_deletion.failed", conversation = %id, stage = "step_3", outcome = "pending", error = &error as &dyn std::error::Error, "the deletion stays pending until the next start");
            return Ok(true);
        }
        self.collect_blobs();
        Ok(true)
    }

    /// Steps 3 and 4 of the deletion of `id`, whose records are gone: its
    /// database goes, then the pending record.
    ///
    /// Its rows in the search index go with the summary mark step 1 raised,
    /// since the indexer drops the rows of a conversation it no longer finds
    /// (`search.rs`). A start runs them for each deletion it finds pending; a Host
    /// that was not told of the deletion lets go of the conversation at its
    /// next `hello`, which no longer lists it.
    pub async fn finish_deletion(&self, id: &ConversationId) -> Result<(), StorageError> {
        let services = self.services();
        services.conversations.remove(id).await?;
        services.control.finish_deletion(id.clone()).await
    }
}
