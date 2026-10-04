//! What the model reads of a session's media (`runtime.md` § Media): the
//! replayed blocks with what the session holds for their media, once the
//! session read the blobs of those it holds nothing for.

use demi_agent_store::media::{self, ModelView};

use super::{SessionShared, TurnError, cancel::TurnCancel, core::SessionCore};

/// The model's view of the replayed blocks. First the session lets go of
/// the media nothing references any more; then it reads the blobs of the
/// replayed media it holds nothing for, which after a restore is every one.
/// A stop ends the reads, and what they found is dropped.
///
/// Inside an action one read covers the view: only the running action
/// changes the transcript, and the action that asked for the view is
/// waiting here. A view asked for outside an action, the usage a page
/// opens with, may see an action change the transcript during its read; it
/// then reads what is still missing.
pub(super) async fn model_view(
    s: &SessionShared,
    cancel: &TurnCancel,
) -> Result<ModelView, TurnError> {
    s.update(SessionCore::release_media);
    loop {
        let unheld = match s.read(SessionCore::model_view) {
            Ok(view) => return Ok(view),
            Err(unheld) => unheld,
        };
        let found = cancel.guard(media::read(s.store.blobs(), unheld)).await??;
        s.update(|core| core.media.absorb(found));
    }
}
