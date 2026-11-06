//! The passes of the server's start over its users' cores: each opens the
//! core of every user whose store holds work an earlier run left, which the
//! core then takes up in the background.

use std::rc::Rc;

use demi_core::shard::Shard;
use demi_core_permissions::{PermissionShard, deliver};
use demi_core_store::StorageError;
use demi_core_store::store::CoreStore;
use demi_web_api_protocol::ids::UserId;

use crate::pool::Shards;

/// Finishes each deletion an earlier backend left pending
/// (`crates/core-store/src/deletions.rs`), in its owner's shard, and
/// collects the owner's blobs after it. A deletion that fails again stays
/// pending for the next start; only listing them fails the call.
pub async fn finish_deletions(cores: &[(UserId, CoreStore)], shards: &Shards) -> Result<(), StorageError> {
    for (owner, store) in cores {
        for id in store.pending_deletions().await? {
            let finished = shards
                .of(owner)
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
    }
    Ok(())
}

/// Opens at start the shard of each user with a stored idle compaction time,
/// which arms its timers in the background, so the start waits for none.
pub async fn arm_idle_compactions(cores: &[(UserId, CoreStore)], shards: &Shards) -> Result<(), StorageError> {
    for (owner, store) in cores {
        if store.idle_compactions().await?.is_empty() {
            continue;
        }
        let handed = shards.of(owner).adopt(Shard::arm_idle_compactions).await;
        if handed.is_err() {
            // The backend is shutting down; the rows stay for the next start.
            return Ok(());
        }
    }
    Ok(())
}

/// Takes up at start the turns the last shutdown interrupted: each owner's
/// shard resumes its own in the background, so the start waits for none.
pub async fn resume_after_restart(cores: &[(UserId, CoreStore)], shards: &Shards) -> Result<(), StorageError> {
    for (owner, store) in cores {
        if store.interrupted_turns().await?.is_empty() {
            continue;
        }
        let handed = shards.of(owner).adopt(Shard::resume_interrupted_turns).await;
        if handed.is_err() {
            // The backend is shutting down; the next start takes up what is
            // left.
            return Ok(());
        }
    }
    Ok(())
}

/// Brings each user's search index in step with the conversations at start
/// (`crates/core-store/src/search.rs`): every user who has a conversation has
/// their shard index it in the background, while the backend serves. Answers
/// once each shard has its task.
pub async fn index_at_start(cores: &[(UserId, CoreStore)], shards: &Shards) -> Result<(), StorageError> {
    for (owner, store) in cores {
        if store.conversations(false).await?.is_empty() && store.conversations(true).await?.is_empty() {
            continue;
        }
        let handed = shards.of(owner).adopt(Shard::catch_up_everything).await;
        if handed.is_err() {
            // The backend is shutting down; the next start indexes what is
            // left.
            return Ok(());
        }
    }
    Ok(())
}

/// Delivers at start each decision whose message a restart cut off: each
/// user's shard sends it to the agent that asked, opening its conversation's
/// tree. Answers once each shard has its tasks.
pub async fn deliver_decisions(cores: &[(UserId, CoreStore)], shards: &Shards) -> Result<(), StorageError> {
    for (owner, store) in cores {
        for request in store.undelivered_permissions().await? {
            let handed = shards
                .of(owner)
                .adopt(move |shard| {
                    let shard: Rc<dyn PermissionShard> = shard;
                    deliver(shard, request)
                })
                .await;
            if handed.is_err() {
                // The backend is shutting down; the next start delivers what
                // is left.
                return Ok(());
            }
        }
    }
    Ok(())
}

/// Opens the shard of every user who has a schedule, whose scheduler
/// then fires what a backend that was down missed and waits for the next
/// times.
pub async fn start_schedulers(
    cores: &[(UserId, CoreStore)],
    shards: &Shards,
) -> Result<(), StorageError> {
    for (user, store) in cores {
        if !store.has_schedules().await? {
            continue;
        }
        // The shard's first call starts its scheduler; a shard that cannot
        // be reached starts it on its next call.
        if let Err(error) = shards.of(user).call(|_, _| async {}).await {
            tracing::warn!(event = "schedule_scheduler.start_failed", %user, %error, "the schedules' scheduler did not start");
        }
    }
    Ok(())
}

/// Starts the shard of each user with a running job, whose start follows
/// them.
pub async fn start_media_followers(
    cores: &[(UserId, CoreStore)],
    shards: &Shards,
) -> Result<(), StorageError> {
    for (user, store) in cores {
        if !store.has_running_media_jobs().await? {
            continue;
        }
        // A shard that cannot be reached follows them at its next call.
        if let Err(error) = shards.of(user).call(|_, _| async {}).await {
            tracing::warn!(event = "media_job.start_failed", %user, %error, "the media jobs were not followed");
        }
    }
    Ok(())
}
