//! What the model reads of a session's media (`runtime.md` § Media): the
//! replayed blocks with the bytes the session holds in the references'
//! place, once the session read the blobs of those it holds nothing for.

use demi_core::Block;

use super::{SessionShared, TurnError, cancel::TurnCancel, core::SessionCore};
use crate::store::media;

/// The replayed blocks as the model receives them, and where they start in
/// the transcript.
pub(super) struct ModelView {
    pub(super) start: usize,
    pub(super) blocks: Vec<Block>,
}

/// The model's view of the replayed blocks. First the session lets go of
/// the media nothing references any more, and reads the blobs of the
/// replayed media it holds nothing for, which after a restore is every one.
/// A stop ends the reads; what they found is dropped, and read again.
pub(super) async fn model_view(s: &SessionShared, cancel: &TurnCancel) -> Result<ModelView, TurnError> {
    let unheld = s.update(|core| {
        core.release_media();
        core.unheld_media()
    });
    if !unheld.is_empty() {
        let found = cancel.guard(media::read(s.store.blobs(), unheld)).await??;
        s.update(|core| core.media.absorb(found));
    }
    Ok(s.read(SessionCore::model_view))
}
