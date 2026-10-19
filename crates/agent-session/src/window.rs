//! The blocks before a session's window (`runtime.md` § What a session
//! holds): what a work that concerns them reads from the store while it
//! holds the session's save order, under which no row before the window
//! changes and the window keeps its start.

use std::collections::BTreeMap;

use demi_agent_store::{Kind, Rows, StoreError};
use demi_agent_transcript::{Earlier, TranscriptLog};
use demi_conversation_socket_protocol::{BlockVersions, Versions, latest_page_start_within};
use demi_shared_types::{Block, BlockId};

use super::SessionShared;

/// Holds the blocks from the last `compaction_boundary` at or before
/// `position` on, reading those before the window from the store, so that
/// a rewrite of the history from there sees them. The caller holds the
/// session's save order; the next save that leaves nothing unsaved before
/// the last boundary lets them go again.
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

/// The transcript a reset sends (`rendering.md` § Where a reset starts):
/// its blocks from `start` to the end, the transcript's length and the
/// versions among them.
#[derive(Debug, Clone, PartialEq)]
pub struct Reset {
    pub start: usize,
    pub length: usize,
    pub blocks: Vec<Block>,
    pub versions: Vec<BlockVersions>,
    pub version: demi_conversation_socket_protocol::TranscriptVersion,
}

/// The reset of the session's transcript for a page that holds its blocks
/// up to `from`, whose block there it holds as `edge`: from `from` while
/// that block is still `edge`, else from the start of the latest page,
/// which the window holds ([`latest_page_start_within`]). A reset within
/// the window is read at once, so that it follows every frame sent before
/// it even while a save waits to commit; one from a place before the
/// window holds the session's save order and reads the blocks the page
/// holds from there from the store.
pub(crate) async fn reset(s: &SessionShared, from: Option<u32>, edge: Option<&BlockId>) -> Result<Reset, StoreError> {
    let held = from.and_then(|from| usize::try_from(from).ok());
    let before = match (held, edge) {
        (Some(from), Some(edge)) => s
            .read(|core| from < core.transcript.start())
            .then(|| (from, edge.clone())),
        _ => None,
    };
    let Some((from, edge)) = before else {
        return Ok(s.read(|core| {
            let transcript = &core.transcript;
            let from = match (held, edge) {
                (Some(from), Some(edge)) if transcript.get(from).is_some_and(|block| block.id() == edge) => from,
                _ => latest_page(transcript),
            };
            window_reset(transcript, from, Rows::default())
        }));
    };
    let _turn = s.persist_gate.acquire().await;
    let start = s.read(|core| core.transcript.start());
    let still = from < start && {
        let read = s.store.blocks(from..from + 1).await?;
        read.blocks.first().is_some_and(|block| block.id() == &edge)
    };
    let earlier = if still { s.store.blocks(from..start).await? } else { Rows::default() };
    Ok(s.read(|core| {
        let transcript = &core.transcript;
        let from = if still { from } else { latest_page(transcript) };
        window_reset(transcript, from, earlier)
    }))
}

/// Where the latest page of a session's transcript starts, which its
/// window holds: the root's reset and a child's or a side question's
/// start there alike.
pub fn latest_page(transcript: &TranscriptLog) -> usize {
    transcript.start() + latest_page_start_within(transcript.blocks(), transcript.start() == 0)
}

/// The reset from `from`: `earlier`, the rows before the window from
/// there, then the window's blocks from there on.
pub(crate) fn window_reset(transcript: &TranscriptLog, from: usize, earlier: Rows) -> Reset {
    let mut versions: BTreeMap<usize, Versions> = earlier.versions;
    versions.extend(transcript.versions().range(from..).map(|(at, held)| (*at, *held)));
    let mut blocks = earlier.blocks;
    blocks.extend_from_slice(transcript.from(from));
    Reset {
        start: from,
        length: transcript.len(),
        versions: versions
            .iter()
            .filter_map(|(at, held)| Some(BlockVersions::new(blocks.get(at - from)?.id().clone(), *held)))
            .collect(),
        blocks,
        version: transcript.version(),
    }
}
