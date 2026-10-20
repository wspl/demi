//! Readying the next request's runtime ahead
//! (`crates/provider-claude-code/src/prepared.rs`).
//!
//! Once the session is idle, the worker builds the request the next one is but
//! for its input, as the selection that request lands with sees the history. It
//! hands the request to the runtime that request lands on, which readies what
//! it needs.
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

use super::{SessionShared, cancel::TurnCancel, media::model_view};

/// Readies the runtime the next request lands on for that request.
pub(super) async fn prepare(s: &SessionShared, ahead: Ahead) {
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
