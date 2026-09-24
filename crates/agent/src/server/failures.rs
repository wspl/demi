//! Failure facts (`backend.md` § Failure facts): what the providers read out
//! of the failure records the error blocks keep, sent beside the blocks a
//! frame or a history carries and never stored. The backend reads a record
//! through the provider the block's model selection names; the agent finds
//! the blocks and attaches the facts.

use demi_agent_protocol::{Failures, ServerFrame, TranscriptPatch};
use demi_core::{Block, ProviderErrorDiagnostics, ProviderFailureFacts, Timestamp};

/// Where the facts of a failure record are read (`failures-and-recovery.md`
/// § Reading a failure): by the provider `provider`, from the record and
/// the moment the failure was received. None when that provider's
/// configuration is gone.
pub trait FailureReader {
    fn read(
        &self,
        provider: &str,
        diagnostics: &ProviderErrorDiagnostics,
        received_at: Timestamp,
    ) -> Option<ProviderFailureFacts>;
}

/// The facts of the error blocks among `blocks` that keep a vendor's
/// record, by block id: the one reading both the conversation socket's
/// frames and the transcript route use. None when no block yields facts.
pub fn read_failures<'a>(
    blocks: impl IntoIterator<Item = &'a Block>,
    reader: &dyn FailureReader,
) -> Option<Failures> {
    let failures: Failures = blocks
        .into_iter()
        .filter_map(|block| {
            let Block::Error(error) = block else {
                return None;
            };
            let diagnostics = error
                .diagnostics
                .as_ref()
                .filter(|diagnostics| diagnostics.upstream.is_some())?;
            let facts = reader.read(&error.model.provider_id, diagnostics, error.created_at)?;
            Some((error.id.clone(), facts))
        })
        .collect();
    (!failures.is_empty()).then_some(failures)
}

/// `frame` with the facts of the error blocks it carries: a transcript's
/// blocks, or the whole blocks its patches put in, of the root or of a
/// subagent.
pub(crate) fn present(frame: ServerFrame, reader: &dyn FailureReader) -> ServerFrame {
    match frame {
        ServerFrame::TranscriptReset {
            blocks, version, ..
        } => ServerFrame::TranscriptReset {
            failures: read_failures(&blocks, reader),
            blocks,
            version,
        },
        ServerFrame::SubagentTranscriptReset {
            subagent_id,
            blocks,
            revision,
            ..
        } => ServerFrame::SubagentTranscriptReset {
            failures: read_failures(&blocks, reader),
            subagent_id,
            blocks,
            revision,
        },
        ServerFrame::TranscriptPatch {
            patches, revision, ..
        } => ServerFrame::TranscriptPatch {
            failures: read_failures(patched(&patches), reader),
            patches,
            revision,
        },
        ServerFrame::SubagentTranscriptPatch {
            subagent_id,
            patches,
            revision,
            ..
        } => ServerFrame::SubagentTranscriptPatch {
            failures: read_failures(patched(&patches), reader),
            subagent_id,
            patches,
            revision,
        },
        frame => frame,
    }
}

/// The whole blocks a list of patches puts into a transcript.
fn patched(patches: &[TranscriptPatch]) -> impl Iterator<Item = &Block> {
    patches.iter().flat_map(|patch| match patch {
        TranscriptPatch::Add { value, .. } | TranscriptPatch::ReplaceBlock { value, .. } => {
            std::slice::from_ref(value)
        }
        TranscriptPatch::Replace { value } => value.as_slice(),
        TranscriptPatch::Remove { .. } | TranscriptPatch::AppendText { .. } => &[],
    })
}
