//! The retention pass (`storage.md` § Retention): once a day, each user's
//! shard removes the expired command outputs of the user's conversations
//! and retires the expired tool media of those whose trees are not live,
//! then collects the user's blobs that nothing references. A tree that is disposed has its conversation retired at once.
//! The agent owns the rule (`runtime.md` § Retired tool media); the store
//! applies it in one transaction per conversation, under the conversation's
//! file gate, so no tree opens meanwhile.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use demi_agent_store::COMMAND_OUTPUT_DAYS;
use demi_agent_transcript::retire::{KEPT, Retirement};
use demi_backend_storage::{blob_refs, command_outputs};
use demi_core::{BlobRef, Timestamp};
use demi_gates::Reservation;
use demi_web_api::ids::ConversationId;
use jiff::SignedDuration;

use crate::backend::Services;
use crate::conversation::root_of;
use crate::shard::{Shard, Shards};
use crate::conversation::ConversationBlobs;

/// How long an unreferenced blob, and its last use, must be old before a
/// collection deletes it: longer than a medium that was put waits for the
/// row that names it.
pub(crate) const GRACE: SignedDuration = SignedDuration::from_hours(24);

/// Runs the retention pass of every user, one user after another in the
/// user's shard: the first pass at once, the next ones every `interval`.
/// It ends when it is dropped, or once the shards close.
pub(crate) async fn schedule(services: Arc<Services>, shards: Shards, interval: Duration) {
    loop {
        match services.control.users().await {
            Ok(users) => {
                for user in users {
                    let passed = shards
                        .of(&user.id)
                        .call(|shard, _| async move { shard.retention_pass().await })
                        .await;
                    if passed.is_err() {
                        // The backend is shutting down.
                        return;
                    }
                }
            }
            Err(error) => {
                tracing::error!(error = &error as &dyn std::error::Error, "the retention pass cannot list the users");
            }
        }
        tokio::time::sleep(interval).await;
    }
}

impl Shard {
    /// The user's retention pass: each conversation whose tree is not live
    /// has its expired tool media retired, one whose tree is live is
    /// recorded as live, and then the user's blobs are collected.
    pub(crate) async fn retention_pass(&self) {
        let control = &self.services().control;
        let conversations = match control.conversation_order(self.user().clone()).await {
            Ok(conversations) => conversations,
            Err(error) => {
                tracing::error!(
                    user = %self.user(),
                    error = &error as &dyn std::error::Error,
                    "the retention pass cannot list the conversations"
                );
                return;
            }
        };
        for id in conversations {
            // A closing shard ends the pass; the next backend's first pass
            // does what is left.
            if self.is_closing() {
                return;
            }
            if let Err(error) = self.remove_command_outputs(&id).await {
                tracing::warn!(conversation = %id, "command outputs not removed: {error}");
            }
            let retired = if self.agent().tree(&root_of(&id)).is_some() {
                let now = self.services().clock.now();
                control.mark_live(id.clone(), now).await.map(|()| 0).map_err(|error| error.to_string())
            } else {
                self.retire_tool_media(&id, false).await
            };
            if let Err(error) = retired {
                tracing::warn!(conversation = %id, "tool media not retired: {error}");
            }
        }
        match self.collect_blobs().await {
            Ok(0) => {}
            Ok(deleted) => tracing::info!(user = %self.user(), deleted, "unreferenced blobs deleted"),
            Err(why) => tracing::warn!(user = %self.user(), "the collection deleted nothing: {why}"),
        }
    }

    /// Records that the conversation's tree was disposed now and, unless the
    /// shard is closing, retires the conversation's expired tool media as
    /// soon as its file gate is free.
    pub(crate) fn tree_disposed(&self, id: &ConversationId) {
        let shard = self.this();
        let id = id.clone();
        let disposed = self.services().clock.now();
        self.tasks().spawn_local(async move {
            if let Err(error) = shard.services().control.mark_live(id.clone(), disposed).await {
                tracing::warn!(conversation = %id, error = &error as &dyn std::error::Error, "the disposal was not recorded");
            }
            if shard.is_closing() {
                return;
            }
            if let Err(error) = shard.retire_tool_media(&id, true).await {
                tracing::warn!(conversation = %id, "tool media not retired: {error}");
            }
        });
    }

    /// Retires the conversation's expired tool media (`storage.md` § Retiring
    /// tool media), holding the conversation as a transition does: its file
    /// gate reserved, so no tree opens, and its tree not live. A gate that is
    /// busy leaves the conversation to the next pass, or, when `wait`, is
    /// waited for. The conversation's databases are read before the gate is
    /// taken, and written only when the rule retires something. Answers how
    /// many blocks changed.
    async fn retire_tool_media(&self, id: &ConversationId, wait: bool) -> Result<usize, String> {
        let services = self.services();
        let retirable = {
            let retirement = self.retirement(id).await?;
            services
                .conversations
                .read(id, move |connection| blob_refs::retirable(connection, retirement))
                .await
                .map_err(|error| error.to_string())?
        };
        if !retirable.is_some_and(|retired| !retired.is_empty()) {
            return Ok(0);
        }
        let reservation = if wait {
            self.free_gate(id).await
        } else {
            self.conversations().slot(id).files.gate().try_reserve()
        };
        let Some(_reservation) = reservation else {
            return Ok(0);
        };
        if self.agent().tree(&root_of(id)).is_some() {
            return Ok(0);
        }
        // Read under the reservation: no tree can have been live since.
        let retirement = self.retirement(id).await?;
        let blobs = ConversationBlobs(services.blobs.for_user(self.user()));
        services
            .conversations
            .db(id)
            .call(move |connection| blob_refs::retire(connection, &blobs, retirement))
            .await
            .map_err(|error| error.to_string())?
            .map_err(|refused| refused.to_string())
    }

    /// Marks removed the conversation's command outputs whose command ended
    /// more than [`COMMAND_OUTPUT_DAYS`] days ago (`storage.md` § Removing
    /// command outputs). No session holds these rows, so it needs neither
    /// the conversation's file gate nor a stored tree. The database is read
    /// first, and written only when some are due. Answers how many it
    /// removed.
    async fn remove_command_outputs(&self, id: &ConversationId) -> Result<usize, String> {
        let services = self.services();
        let now = services.clock.now();
        // Thirty days before any time a clock gives is a time too.
        let expired = now
            .to_jiff()
            .checked_sub(SignedDuration::from_hours(24 * COMMAND_OUTPUT_DAYS))
            .map_err(|error| error.to_string())?;
        let expired = Timestamp::truncate(expired);
        let due = services
            .conversations
            .read(id, move |connection| command_outputs::expired(connection, expired))
            .await
            .map_err(|error| error.to_string())?;
        if due != Some(true) {
            return Ok(0);
        }
        let blobs = ConversationBlobs(services.blobs.for_user(self.user()));
        services
            .conversations
            .db(id)
            .call(move |connection| command_outputs::remove_expired(connection, &blobs, expired, now))
            .await
            .map_err(|error| error.to_string())?
            .map_err(|refused| refused.to_string())
    }

    /// The rule as it applies to the conversation now: it has been idle for
    /// 30 days when its tree was last seen live that long ago.
    async fn retirement(&self, id: &ConversationId) -> Result<Retirement, String> {
        let now = self.services().clock.now();
        let live_at = self
            .services()
            .control
            .live_at(id.clone())
            .await
            .map_err(|error| error.to_string())?
            .ok_or("the conversation is gone")?;
        Ok(Retirement {
            now,
            idle: elapsed(live_at, now) >= KEPT,
        })
    }

    /// The conversation's file gate, reserved once nothing holds or waits
    /// for it; none once its tree is live again or the shard closes.
    async fn free_gate(&self, id: &ConversationId) -> Option<Reservation> {
        let files = self.conversations().slot(id).files.gate().clone();
        loop {
            if self.is_closing() || self.agent().tree(&root_of(id)).is_some() {
                return None;
            }
            // Subscribed before the attempt, so a lease that ends after it
            // wakes the wait.
            let mut changes = files.subscribe();
            if let Some(reservation) = files.try_reserve() {
                return Some(reservation);
            }
            tokio::select! {
                changed = changes.changed() => {
                    // The slot keeps the gate while the shard lives.
                    if changed.is_err() {
                        return None;
                    }
                }
                () = self.closed() => return None,
            }
        }
    }

    /// Collects the user's blobs (`storage.md` § Collecting blobs): each
    /// blob that no reference names, whose object and last use are older than
    /// [`GRACE`], is deleted. It fails closed: when one of the user's
    /// reference sources cannot be read, it deletes nothing and answers
    /// which. Answers how many blobs it deleted.
    pub(crate) async fn collect_blobs(&self) -> Result<usize, String> {
        let services = self.services();
        let blobs = services.blobs.for_user(self.user());
        blobs.forget_uses(GRACE);
        let now = services.clock.now();
        let stored = blobs
            .list()
            .await
            .map_err(|error| format!("the blob namespace cannot be listed: {error}"))?;
        let old: Vec<BlobRef> = stored
            .into_iter()
            .filter(|stored| elapsed(stored.written, now) > GRACE)
            .map(|stored| stored.blob)
            .collect();
        if old.is_empty() {
            return Ok(0);
        }
        let referenced = self.references().await?;
        let mut deleted = 0;
        for blob in old.iter().filter(|blob| !referenced.contains(*blob)) {
            if self.is_closing() {
                break;
            }
            match blobs.delete_unused(blob, GRACE).await {
                Ok(true) => deleted += 1,
                Ok(false) => {}
                Err(error) => {
                    // The next pass tries again.
                    tracing::warn!(user = %self.user(), %blob, error = &error as &dyn std::error::Error, "a blob was not deleted");
                }
            }
        }
        Ok(deleted)
    }

    /// Every blob the user's reference sources name: the upload records,
    /// which cover drafts, and each conversation's database, archived ones
    /// and Fork destinations not published yet included. The first source
    /// that cannot be read fails it.
    async fn references(&self) -> Result<BTreeSet<BlobRef>, String> {
        let services = self.services();
        let control = &services.control;
        let mut references: BTreeSet<BlobRef> = control
            .upload_blobs(self.user().clone())
            .await
            .map_err(|error| format!("the upload records cannot be read: {error}"))?
            .into_iter()
            .collect();
        let mut conversations = control
            .conversation_order(self.user().clone())
            .await
            .map_err(|error| format!("the conversations cannot be listed: {error}"))?;
        let forks = control
            .pending_forks()
            .await
            .map_err(|error| format!("the Forks under way cannot be listed: {error}"))?;
        conversations.extend(
            forks
                .into_iter()
                .filter(|fork| fork.owner == *self.user())
                .map(|fork| fork.id),
        );
        for id in conversations {
            let named = services
                .conversations
                .read(&id, blob_refs::references)
                .await
                .map_err(|error| format!("the database of conversation {id} cannot be read: {error}"))?;
            references.extend(named.into_iter().flatten());
        }
        Ok(references)
    }
}

/// How long after `earlier` `now` is.
fn elapsed(earlier: Timestamp, now: Timestamp) -> SignedDuration {
    now.to_jiff().duration_since(earlier.to_jiff())
}
