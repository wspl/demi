//! Readying the next request's runtime ahead (`claude-code.md` § Preparing
//! a process ahead): once the session is idle, the worker builds the request
//! the next one is but for its input, as the selection that request lands
//! with sees the history, and hands it to the runtime that request lands
//! on, which readies what it needs. It takes no admission: the step is
//! quick, and what it starts goes on by itself. A request that cannot be
//! built now readies nothing, and the request says why when it is sent.
//! Readying reads no blob: opening a conversation reads none
//! (`runtime.md` § Media), so a history whose media the session does not
//! hold yet, as after a restore, readies nothing until a request read them.

use demi_provider_common::Ahead;

use super::SessionShared;

/// Readies the runtime the next request lands on for that request.
pub(super) async fn prepare(s: &SessionShared, ahead: Ahead) {
    let model = s.read(|core| core.landing_selection().clone());
    // A prompt that cannot be rendered fails the request, which records it.
    let Ok(prompt) = s.runtime.system_prompt(&model).await else {
        return;
    };
    let request_id = s.ids.next_id();
    let taken = s.update(|core| {
        // An action that started meanwhile sends the request itself.
        if !core.idle_with_runtime() {
            return None;
        }
        let view = core.model_view().ok()?;
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
