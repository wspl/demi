//! What a page reads of a conversation's live tree: a node's latest page, with
//! the output of its request and its cache chart.
//!
//! A live node's latest page is read from its session's window, whole in
//! memory, so it holds what no checkpoint has stored yet and is at the version
//! of the patches that follow it. The store adds what the window does not
//! keep: the entries of the newest instructions block and the summary size of
//! each compaction marker.

use demi_backend_database::StorageError;
use demi_backend_database::tree::{self, Transcript};
use demi_conversation_lane_protocol::{CacheChart, ShellStatus, index_u32};
use demi_shared_types::NodeId;
use demi_web_api_protocol::conversations::TranscriptPage;
use demi_web_api_protocol::ids::ConversationId;

use crate::shard::Shard;
use demi_backend_host_access::root_of;

impl Shard {
    /// The latest `count` blocks, a page at most, of the live node `node` of
    /// the conversation `id`, read from its window now, with the transcript's version and the view of each
    /// live command of the node and its cache chart, without the failure facts
    /// and attachments its route adds; none when the node is not live.
    pub async fn live_transcript(
        &self,
        id: &ConversationId,
        node: &NodeId,
        count: usize,
    ) -> Result<Option<TranscriptPage>, StorageError> {
        let Some(tree) = self.agent().tree(&root_of(id)) else {
            return Ok(None);
        };
        // Read at once, nothing awaited, so the patches that follow begin at
        // the version it names.
        let Some((page, commands, request_output)) = tree.latest_page(node, count) else {
            return Ok(None);
        };
        let cache_chart = tree.cache_chart(node);
        let transcript = Transcript::Agent(node.clone());
        let blocks = page.blocks.clone();
        let (instructions, summaries) = self
            .services()
            .conversations
            .read(id, move |connection| tree::read_context(connection, &transcript, &blocks))
            .await?
            .unwrap_or_default();
        Ok(Some(TranscriptPage {
            start: index_u32(page.start),
            length: index_u32(page.length),
            blocks: page.blocks,
            failures: None,
            instructions,
            summaries,
            versions: page.versions,
            attachments: Vec::new(),
            version: Some(page.version),
            commands,
            request_output,
            cache_chart,
        }))
    }

    /// The view of each command a closed subagent's shells hold while they
    /// stop, for the page of a node that is not live.
    pub fn stopping_commands(&self, id: &ConversationId, node: &NodeId) -> Vec<ShellStatus> {
        self.agent()
            .tree(&root_of(id))
            .map(|tree| tree.stopping_commands(node))
            .unwrap_or_default()
    }

    /// The cache chart of the node `node` of the conversation `id`; none for a
    /// node that has none.
    pub async fn cache_chart(&self, id: &ConversationId, node: &NodeId) -> Result<Option<CacheChart>, demi_agent_store::StoreError> {
        self.agent().cache_chart(&root_of(id), node).await
    }

}
