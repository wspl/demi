//! Readying the next request's runtime ahead
//! (`crates/provider-claude-code/src/prepared.rs`).
//!
//! Once the session is idle, its runtime first takes the account the next
//! request runs on, which the session keeps and the pages' context usage names
//! (`crates/provider-registry/src/rotation.rs`). Then the
//! worker builds the request the next one is but for its input, as the
//! selection that request lands with sees the history, and hands it to that
//! runtime, which readies what it needs.
//!
//! - It takes no admission: what the runtime starts goes on by itself.
//! - Building the request reads the blobs of the media its history replays that
//!   the session holds nothing for, as after a restore. It runs after a page's
//!   open was answered, so the open itself reads none.
//! - A runtime that readies nothing gets no request, and no blob is read for
//!   it.
//! - A request that cannot be built now readies nothing, and the request says
//!   why when it is sent.

use demi_provider_common::Ahead;

use super::{SessionShared, cancel::TurnCancel, compaction::report_context_usage, media::model_view};

/// Readies the runtime the next request lands on for that request.
pub(super) async fn prepare(s: &SessionShared, ahead: Ahead) {
    next_account(s).await;
    if !s.read(|core| core.landing_prepares_ahead()) {
        return;
    }
    let model = s.read(|core| core.landing_selection().clone());
    // A prompt that cannot be rendered fails the request, which records it.
    let Ok(prompt) = s.runtime.system_prompt(&model).await else {
        return;
    };
    // Outside an action nothing stops the reads.
    let unstoppable = TurnCancel::new();
    let Ok(view) = model_view(s, &unstoppable).await else {
        return;
    };
    let request_id = s.ids.next_id();
    let taken = s.update(|core| {
        // An action that started meanwhile sends the request itself, and
        // only an action changes the history the view was read from.
        if !core.idle_with_runtime() {
            return None;
        }
        let request = core.ahead_request(&view, prompt.text(), request_id);
        let from_switch = core
            .switch
            .as_ref()
            .is_some_and(|switch| switch.runtime.is_some());
        core.take_landing_runtime()
            .map(|runtime| (request, runtime, from_switch))
    });
    let Some((request, mut runtime, from_switch)) = taken else {
        return;
    };
    runtime.prepare(request, ahead).await;
    let unused = s.update(|core| core.return_landing_runtime(runtime, from_switch));
    s.runtime_returned.notify_waiters();
    if let Some(mut unused) = unused {
        unused.close().await;
    }
}

/// Keeps the account the next request runs on, as the runtime in the slot
/// takes it now, and tells the pages of a change with the context usage.
///
/// - While a switch waits, nothing is taken: the account the session keeps is
///   the one of the runtime in the slot, which a compaction before the switch
///   still uses, and the switch's request takes its own.
/// - An action that starts meanwhile takes its account with its request, so
///   the choice gives way to it, as to a stop, and changes nothing.
async fn next_account(s: &SessionShared) {
    let taken = s.update(|core| {
        if !core.idle_with_runtime() || core.switch.is_some() {
            return None;
        }
        let model = core.model.clone();
        let account = core.account.clone();
        core.take_landing_runtime()
            .map(|runtime| (model, account, runtime))
    });
    let Some((model, account, mut runtime)) = taken else {
        return;
    };
    let next = tokio::select! {
        next = runtime.next_account(&model, account.as_ref()) => next,
        () = s.work.notified() => None,
    };
    let unused = s.update(|core| {
        if let Some(next) = &next {
            core.take_account(next.clone());
        }
        core.return_landing_runtime(runtime, false)
    });
    s.runtime_returned.notify_waiters();
    if let Some(mut unused) = unused {
        unused.close().await;
    }
    if next.is_some() {
        // Outside an action nothing stops the reads.
        report_context_usage(s, &TurnCancel::new()).await;
    }
}
