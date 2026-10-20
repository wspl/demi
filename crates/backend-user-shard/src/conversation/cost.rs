//! A conversation's cost as its open pages follow it
//! (`crates/backend-database/src/usage.rs`).
//!
//! 1. The control store keeps each agent's cost as it writes a ledger row, and
//!    announces the row with what its agent costs now.
//! 2. The backend hands the row to its user's shard.
//! 3. The shard tells the conversation's live tree, whose agent entry of that
//!    agent changes (`crates/agent-server/src/server/tree/agents.rs`); a row
//!    of no agent, such as a title's, changes the conversation's metadata.
//!
//! Nothing reads the ledger to announce a row, and a conversation whose tree
//! is not live is told nothing: its tree reads what is kept when it opens.

use std::rc::Rc;

use demi_backend_database::usage::UsageWritten;
use demi_backend_host_access::root_of;
use demi_backend_page_sync::Part;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;

use crate::shard::{Shard, Shards};

impl Shard {
    /// Tells the conversation of the row `written` what its agent costs now.
    pub(crate) async fn announce_cost(self: Rc<Self>, written: UsageWritten) {
        match written.agent {
            Some(agent) => {
                if let Some(tree) = self.agent().tree(&root_of(&written.conversation)) {
                    tree.cost_changed(agent, written.cost);
                }
            }
            None => self.mark(Part::Metadata(written.conversation)),
        }
    }
}

/// Hands each ledger row from `written` to its user's shard, which announces
/// it to the conversation's pages, until the backend closes. A follower that
/// fell behind misses the rows it lost, which it logs.
pub async fn follow_usage(mut written: broadcast::Receiver<UsageWritten>, shards: Shards) {
    loop {
        match written.recv().await {
            Ok(row) => {
                let user = row.user.clone();
                if shards.of(&user).adopt(move |shard| shard.announce_cost(row)).await.is_err() {
                    // The backend is shutting down.
                    return;
                }
            }
            Err(RecvError::Lagged(missed)) => {
                tracing::warn!(missed, "ledger rows were not announced to their conversations");
            }
            Err(RecvError::Closed) => return,
        }
    }
}
