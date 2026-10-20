//! The blocks before a session's window.
//!
//! A work that concerns them reads them from the store while it holds the
//! session's save order, under which no row before the window changes and the
//! window keeps its start.
//!
//! The window is the only copy of the transcript the live process keeps, so a
//! tree's memory grows with its sessions' windows, never with the length of
//! their conversations. Whatever concerns a block before it (a page reading
//! history, an edit, a Fork) reads the rows it needs from the store and holds
//! them no longer than that work.

use std::collections::BTreeMap;

use demi_agent_store::{Checkpoint, Kind, StoreError};
use demi_agent_transcript::{Earlier, TranscriptLog};
use demi_conversation_socket_protocol::{BlockVersions, Versions, latest_page_start, light};
use demi_shared_types::{Block, BlockId};

use super::SessionShared;

/// Holds the blocks from the last `compaction_boundary` at or before `position`
/// on, so that a rewrite of the history from there sees them.
///
/// Those before the window are read from the store. The caller holds the
/// session's save order; the next save that leaves nothing unsaved before the
/// last boundary lets them go again.
pub(crate) async fn reach(s: &SessionShared, position: usize) -> Result<(), StoreError> {
    let start = s.read(|core| core.transcript.start());
    if position >= start {
        return Ok(());
    }
    let from = s.store.last_of(Kind::CompactionBoundary, position + 1).await?.unwrap_or(0);
    let rows = s.store.blocks(from..start).await?;
    let earlier = Earlier::read(&*s.store, from).await?;
    s.update(|core| core.transcript.extend_back(from, rows.blocks, rows.versions, earlier));
    Ok(())
}

/// Holds one compaction more of the history before the window, when there
/// is one: answers false when the window starts at the path's start.
pub(crate) async fn reach_further(s: &SessionShared) -> Result<bool, StoreError> {
    let start = s.read(|core| core.transcript.start());
    if start == 0 {
        return Ok(false);
    }
    reach(s, start - 1).await?;
    Ok(true)
}

/// The position of the block `id` on the current path: among the blocks
/// held, else as the store finds it.
pub(crate) async fn position(s: &SessionShared, id: &BlockId) -> Result<Option<usize>, StoreError> {
    if let Some(position) = s.read(|core| core.transcript.position(id)) {
        return Ok(Some(position));
    }
    s.store.position(id).await
}

/// The transcript a reset sends: its blocks from `start` to the end in their
/// light form, the transcript's length and the versions among them.
#[derive(Debug, Clone, PartialEq)]
pub struct Reset {
    pub start: usize,
    pub length: usize,
    pub blocks: Vec<Block>,
    pub versions: Vec<BlockVersions>,
    pub version: demi_conversation_socket_protocol::Version,
}

/// The reset of the session's transcript for a page that holds its blocks up to
/// `from`, whose block there it holds as `edge`.
///
/// The reset starts from `from` while that block is still `edge` and the
/// blocks from there fit in the latest page, else from the start of the latest
/// page, which the window holds ([`latest_page`]): a page away for long gets
/// one page, and what it held before stays a hole it reads when it shows it.
/// It is read at once, so that it follows every frame sent before it.
pub(crate) fn reset(s: &SessionShared, from: Option<u32>, edge: Option<&BlockId>) -> Reset {
    s.read(|core| {
        let transcript = &core.transcript;
        held_reset(
            transcript.start(),
            transcript.blocks(),
            transcript.versions(),
            transcript.version(),
            from,
            edge,
        )
    })
}

/// The reset of a closed node's transcript from its last checkpoint, at the
/// node's `version`, for a page that holds it up to `from` with `edge` there,
/// by the rules of a live session's ([`reset`]).
pub fn checkpoint_reset(
    checkpoint: &Checkpoint,
    version: demi_conversation_socket_protocol::Version,
    from: Option<u32>,
    edge: Option<&BlockId>,
) -> Reset {
    held_reset(
        checkpoint.start,
        &checkpoint.transcript,
        &checkpoint.versions,
        version,
        from,
        edge,
    )
}

/// The reset of the window of `blocks` from the position `start` on, with
/// the `versions` of its places, at `version`: from `from` while the block
/// there is still `edge` and is within the latest page, else from the latest
/// page's start.
///
/// Its blocks are light but the last, which the stream may still be writing:
/// the patches that follow add to it, so it comes whole, and a page that
/// reads a light block whole reads the stored one, which a block being
/// written is not yet.
fn held_reset(
    start: usize,
    blocks: &[Block],
    versions: &BTreeMap<usize, Versions>,
    version: demi_conversation_socket_protocol::Version,
    from: Option<u32>,
    edge: Option<&BlockId>,
) -> Reset {
    let latest = start + latest_page_start(blocks);
    let at = |position: usize| blocks.get(position.checked_sub(start)?);
    let from = from
        .and_then(|from| usize::try_from(from).ok())
        .zip(edge)
        .filter(|(from, edge)| *from >= latest && at(*from).is_some_and(|block| block.id() == *edge))
        .map_or(latest, |(from, _)| from);
    let mut light = light(&blocks[from - start..]);
    if let (Some(last), Some(whole)) = (light.last_mut(), blocks.last()) {
        *last = whole.clone();
    }
    Reset {
        start: from,
        length: start + blocks.len(),
        versions: versions
            .range(from..)
            .filter_map(|(at, held)| Some(BlockVersions::new(light.get(at - from)?.id().clone(), *held)))
            .collect(),
        blocks: light,
        version,
    }
}

/// Where the latest page of a session's transcript starts, which its
/// window holds: the root's reset and a child's or a side question's
/// start there alike.
pub fn latest_page(transcript: &TranscriptLog) -> usize {
    transcript.start() + latest_page_start(transcript.blocks())
}
