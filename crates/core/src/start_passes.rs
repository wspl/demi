//! What a core takes up before it serves, of what an earlier run left in its
//! store: the server's start runs it over all of its users' cores, and a
//! program's process over each core as it opens it. Each pass opens the core
//! of every store that holds such work, which the core then takes up in the
//! background.

use std::fmt;
use std::rc::Rc;

use demi_core_permissions::{PermissionCore, deliver};
use demi_core_store::StorageError;
use demi_core_store::store::CoreStore;
use demi_web_api_protocol::ids::UserId;

use crate::Core;
use crate::conversation::recover_forks;
use crate::pool::Shards;

/// Takes up what an earlier run left in the stores `cores`, before their
/// cores serve. Only the forks' recovery fails it: a pass that fails after it
/// is logged, the work it missed waits for the next run, and the rest go on.
pub async fn take_up(cores: &[(UserId, CoreStore)], shards: &Shards) -> Result<(), StorageError> {
    // Fork destinations whose root committed before their publication are
    // published before the cores serve.
    recover_forks(cores).await?;
    // Each deletion an earlier run left pending is finished. A failure does
    // not stop the start: the next run finishes it, and no request finds
    // its conversation meanwhile.
    start_step("pending_deletions", finish_deletions(cores, shards)).await;
    // A credential request's command ended with the run before, so no one
    // waits for its answer; and a credential whose time came while the core
    // was not running is removed. A failure does not stop the start: the core
    // removes what is past due when it starts, and the next run withdraws
    // the requests.
    for (_, store) in cores {
        start_step("credential_requests", store.withdraw_credential_requests()).await;
        start_step("expired_credentials", store.expire_all_user_credentials()).await;
    }
    // Each permission decision whose message a stop cut off reaches the
    // agent that asked. A failure does not stop the start: the next run
    // delivers it.
    start_step("permission_decisions", deliver_decisions(cores, shards)).await;
    // Each root's turn the last stop interrupted goes on, in the background,
    // as the user's Resume would, unless the user turned that off. A failure
    // does not stop the start: the marks stay for the next run, and
    // meanwhile the turns wait for the user's Resume.
    start_step("interrupted_turns", resume_after_restart(cores, shards)).await;
    // Idle conversations compact at the times their last requests set, which
    // a stop keeps. A failure does not stop the start: the next answered
    // request sets its conversation's time again.
    start_step("idle_compactions", arm_idle_compactions(cores, shards)).await;
    // The schedules fire what they missed while the core was not running,
    // and wait for their next times. A failure does not stop the start: the
    // core's next call starts them.
    start_step("schedule_users", start_schedulers(cores, shards)).await;
    // Each media job a vendor took before the stop is followed to its end,
    // whether or not its call comes back. A failure does not stop the start:
    // the core's next call follows them.
    start_step("media_jobs", start_media_followers(cores, shards)).await;
    // The search index catches up with the conversations, in the background
    // (`crates/core-store/src/search.rs`). A failure does not stop the start:
    // a conversation not indexed here is indexed by its next change.
    start_step("search_index", index_at_start(cores, shards)).await;
    Ok(())
}

/// Runs the start step `step`, whose failure does not stop the start: its end
/// is logged with the time it took, and a failure with its error.
pub async fn start_step<T, E: fmt::Display>(step: &'static str, work: impl Future<Output = Result<T, E>>) {
    let started = std::time::Instant::now();
    let done = work.await;
    let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    match done {
        Ok(_) => tracing::info!(event = "startup.step_ended", step, ms, outcome = "ok", "a start step ended"),
        Err(error) => tracing::error!(event = "startup.step_failed", step, ms, %error, "a start step failed; the start goes on"),
    }
}

/// Finishes each deletion an earlier backend left pending
/// (`crates/core-store/src/deletions.rs`), in its owner's core, and
/// collects the owner's blobs after it. A deletion that fails again stays
/// pending for the next start; only listing them fails the call.
async fn finish_deletions(cores: &[(UserId, CoreStore)], shards: &Shards) -> Result<(), StorageError> {
    for (owner, store) in cores {
        for id in store.pending_deletions().await? {
            let finished = shards
                .of(owner)
                .call(move |core, _| async move {
                    let finished = core.finish_deletion(&id).await;
                    if finished.is_ok() {
                        core.collect_blobs();
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

/// Opens at start the core of each user with a stored idle compaction time,
/// which arms its timers in the background, so the start waits for none.
async fn arm_idle_compactions(cores: &[(UserId, CoreStore)], shards: &Shards) -> Result<(), StorageError> {
    for (owner, store) in cores {
        if store.idle_compactions().await?.is_empty() {
            continue;
        }
        let handed = shards.of(owner).adopt(Core::arm_idle_compactions).await;
        if handed.is_err() {
            // The backend is shutting down; the rows stay for the next start.
            return Ok(());
        }
    }
    Ok(())
}

/// Takes up at start the turns the last shutdown interrupted: each owner's
/// core resumes its own in the background, so the start waits for none.
async fn resume_after_restart(cores: &[(UserId, CoreStore)], shards: &Shards) -> Result<(), StorageError> {
    for (owner, store) in cores {
        if store.interrupted_turns().await?.is_empty() {
            continue;
        }
        let handed = shards.of(owner).adopt(Core::resume_interrupted_turns).await;
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
/// their core index it in the background, while the backend serves. Answers
/// once each core has its task.
async fn index_at_start(cores: &[(UserId, CoreStore)], shards: &Shards) -> Result<(), StorageError> {
    for (owner, store) in cores {
        if store.conversations(false).await?.is_empty() && store.conversations(true).await?.is_empty() {
            continue;
        }
        let handed = shards.of(owner).adopt(Core::catch_up_everything).await;
        if handed.is_err() {
            // The backend is shutting down; the next start indexes what is
            // left.
            return Ok(());
        }
    }
    Ok(())
}

/// Delivers at start each decision whose message a restart cut off: each
/// user's core sends it to the agent that asked, opening its conversation's
/// tree. Answers once each core has its tasks.
async fn deliver_decisions(cores: &[(UserId, CoreStore)], shards: &Shards) -> Result<(), StorageError> {
    for (owner, store) in cores {
        for request in store.undelivered_permissions().await? {
            let handed = shards
                .of(owner)
                .adopt(move |core| {
                    let core: Rc<dyn PermissionCore> = core;
                    deliver(core, request)
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

/// Opens the core of every user who has a schedule, whose scheduler
/// then fires what a backend that was down missed and waits for the next
/// times.
async fn start_schedulers(
    cores: &[(UserId, CoreStore)],
    shards: &Shards,
) -> Result<(), StorageError> {
    for (user, store) in cores {
        if !store.has_schedules().await? {
            continue;
        }
        // The core's first call starts its scheduler; a core that cannot
        // be reached starts it on its next call.
        if let Err(error) = shards.of(user).call(|_, _| async {}).await {
            tracing::warn!(event = "schedule_scheduler.start_failed", %user, %error, "the schedules' scheduler did not start");
        }
    }
    Ok(())
}

/// Starts the core of each user with a running job, whose start follows
/// them.
async fn start_media_followers(
    cores: &[(UserId, CoreStore)],
    shards: &Shards,
) -> Result<(), StorageError> {
    for (user, store) in cores {
        if !store.has_running_media_jobs().await? {
            continue;
        }
        // A core that cannot be reached follows them at its next call.
        if let Err(error) = shards.of(user).call(|_, _| async {}).await {
            tracing::warn!(event = "media_job.start_failed", %user, %error, "the media jobs were not followed");
        }
    }
    Ok(())
}
