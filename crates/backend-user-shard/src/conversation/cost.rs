//! A conversation's cost as its open pages follow it (`usage-and-quota.md`
//! § Reading it): the control store announces each ledger row it writes,
//! the backend hands the row to its user's shard, and the shard sends what
//! the row adds to the cost as a `cost` frame to the conversation's
//! attached connections. A conversation nobody shows is sent nothing.

use std::rc::Rc;

use demi_backend_database::tree;
use demi_backend_database::usage::UsageWritten;
use demi_backend_host_access::root_of;
use demi_conversation_socket_protocol::ServerFrame;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;

use crate::shard::{Shard, Shards};

impl Shard {
    /// Sends what the row `written` adds to its conversation's cost to the
    /// conversation's attached connections. A failed read is logged and the
    /// frame is not sent: the page finds the row missing from the next
    /// frame's count and reads the cost again.
    pub(crate) async fn announce_cost(self: Rc<Self>, written: UsageWritten) {
        let shown = self
            .agent()
            .tree(&root_of(&written.conversation))
            .is_some_and(|tree| tree.is_attached());
        if !shown {
            return;
        }
        let services = self.services();
        let addition = async {
            let labels = services
                .conversations
                .read(&written.conversation, tree::agent_labels)
                .await?
                .unwrap_or_default();
            services
                .control
                .cost_addition(written.conversation.clone(), written.id.clone(), labels)
                .await
        };
        let addition = match addition.await {
            Ok(Some(addition)) => addition,
            // The row is another conversation's, which cannot be.
            Ok(None) => return,
            Err(error) => {
                tracing::warn!(
                    error = &error as &dyn std::error::Error,
                    conversation = %written.conversation,
                    "a conversation's cost frame was not sent"
                );
                return;
            }
        };
        // The tree may have closed while the rows were read.
        if let Some(tree) = self.agent().tree(&root_of(&written.conversation)) {
            tree.announce(ServerFrame::Cost {
                rows: addition.rows,
                totals: addition.totals,
                agent: addition.agent,
                call: addition.call,
            });
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
