//! A conversation's cost as its open pages follow it
//! (`crates/core-store/src/usage.rs`).
//!
//! 1. The control store keeps each agent's cost as it writes a ledger row, and
//!    announces the row with what its agent costs now.
//! 2. The backend hands the row to its user's shard.
//! 3. The shard tells the conversation's live tree, whose agent entry of that
//!    agent changes, with its cache chart for a turn call
//!    (`crates/agent-server/src/server/tree/agents.rs`); a row of no agent,
//!    such as a title's, changes the conversation's metadata, as does the
//!    conversation's first row, which gives it its first call's time.
//!
//! Nothing reads the ledger again to announce a row: the chart is counted in
//! the row's write. A conversation whose tree is not live is told nothing:
//! its tree reads what is kept when it opens.

use std::rc::Rc;

use demi_core_store::usage::UsageWritten;
use demi_backend_host_access::root_of;
use demi_backend_page_sync::Part;
use tokio::sync::broadcast::error::RecvError;

use crate::shard::Shard;

impl Shard {
    /// Tells the conversation of the row `written` what its agent costs now.
    pub(crate) async fn announce_cost(self: Rc<Self>, written: UsageWritten) {
        if written.agent.is_none() || written.first {
            self.mark(Part::Metadata(written.conversation.clone()));
        }
        if let Some(agent) = written.agent
            && let Some(tree) = self.agent().tree(&root_of(&written.conversation))
        {
            tree.cost_changed(agent, written.cost, written.chart);
        }
    }
}

impl Shard {
    /// Announces each ledger row of the user's to the conversation's pages,
    /// until the shard closes. A follower that fell behind misses the rows it
    /// lost, which it logs.
    pub(crate) async fn follow_usage(self: Rc<Self>) {
        let mut written = self.store().usage_written();
        loop {
            tokio::select! {
                () = self.closed() => return,
                row = written.recv() => match row {
                    Ok(row) => self.clone().announce_cost(row).await,
                    Err(RecvError::Lagged(missed)) => {
                        tracing::warn!(event = "usage_ledger.announce_missed", missed, "ledger rows were not announced to their conversations");
                    }
                    Err(RecvError::Closed) => return,
                },
            }
        }
    }
}
