//! What the model reads of a session's media (`runtime.md` § Media): the
//! replayed blocks with what the session holds for their media, once the
//! session read the blobs of those it holds nothing for.

use demi_agent_store::media::{self, ModelView};
use demi_shared_types::BlobRef;

use super::{SessionShared, TurnError, cancel::TurnCancel, core::SessionCore};

/// The model's view of the replayed blocks, once the session holds what
/// [`held`] reads.
pub(super) async fn model_view(
    s: &SessionShared,
    cancel: &TurnCancel,
) -> Result<ModelView, TurnError> {
    held(s, cancel, SessionCore::model_view).await
}

/// What `of` makes of the session once it holds the media `of` needs. First
/// the session lets go of the media nothing references any more; then it
/// reads the blobs of the media `of` names that it holds nothing for, which
/// after a restore is every one. A stop ends the reads, and what they found
/// is dropped.
///
/// Inside an action one read covers what `of` needs: only the running
/// action changes the transcript, and the action that asked is waiting
/// here. Outside an action, as when a `compact` frame asks for the usage,
/// an action may change the transcript during the read; the session then
/// reads what is still missing.
pub(super) async fn held<T>(
    s: &SessionShared,
    cancel: &TurnCancel,
    of: impl Fn(&SessionCore) -> Result<T, Vec<BlobRef>>,
) -> Result<T, TurnError> {
    s.update(SessionCore::release_media);
    loop {
        let unheld = match s.read(&of) {
            Ok(value) => return Ok(value),
            Err(unheld) => unheld,
        };
        let found = cancel.guard(media::read(s.store.blobs(), unheld)).await??;
        s.update(|core| core.media.absorb(found));
    }
}
