//! What the model reads of a session's media (`runtime.md` § Media): the
//! replayed blocks with what the session holds for their media, once the
//! session read the blobs of those it holds nothing for.

use super::{SessionShared, TurnError, cancel::TurnCancel, core::SessionCore};
use crate::store::media::{self, ModelView};

/// The model's view of the replayed blocks. First the session lets go of
/// the media nothing references any more; then it reads the blobs of the
/// replayed media it holds nothing for, which after a restore is every one.
/// A stop ends the reads, and what they found is dropped.
///
/// One read covers the view: only the running action changes the
/// transcript, and the action that asked for the view is waiting here.
/// Steers, agent messages and queued messages wait outside the transcript
/// until that action writes them, and what the session holds only grows
/// meanwhile.
pub(super) async fn model_view(s: &SessionShared, cancel: &TurnCancel) -> Result<ModelView, TurnError> {
    s.update(SessionCore::release_media);
    let unheld = match s.read(SessionCore::model_view) {
        Ok(view) => return Ok(view),
        Err(unheld) => unheld,
    };
    let found = cancel.guard(media::read(s.store.blobs(), unheld)).await??;
    Ok(s.update(|core| {
        core.media.absorb(found);
        core.model_view()
            .expect("the read covered every medium the replayed blocks reference")
    }))
}
