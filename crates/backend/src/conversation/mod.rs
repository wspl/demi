//! Conversations on the user's shard (`backend.md` § Request paths and
//! responsibilities): the agent server that hosts each conversation's tree
//! over the conversation's database, the conversation socket, the provider
//! runtimes the sessions infer with, the conversations' summaries, and the
//! failure facts of their history.

mod failure_facts;
mod fork;
mod harness;
pub(crate) mod host_access;
mod providers;
pub(crate) mod remote_files;
mod socket;
mod summary;
pub(crate) mod target;
pub(crate) mod transfer;
mod transition;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use demi_agent::{AgentServer, AgentTreeStore, RandomIds, ServerConfig, ServerDeps, TreeStores};
use demi_core::NodeId;
use demi_web_api::ids::{ConversationId, UserId};

pub(crate) use self::failure_facts::failure_facts;
pub(crate) use self::fork::{ForkRefusal, recover_forks};
pub(crate) use self::harness::ConversationHarness;
use self::harness::NoShellEnvironments;
use self::providers::ConversationProviders;
use crate::backend::Services;
use crate::storage::tree::SqliteTreeStore;
use crate::usage::rate_limit::RequestRateLimit;

/// The agent server of a user's shard: its trees keep their state in the
/// conversations' databases, and their sessions infer with the user's
/// providers under the user's rate limit.
pub(crate) fn agent_server(
    user: UserId,
    services: Arc<Services>,
    http: reqwest::Client,
    rate_limit: Rc<RefCell<RequestRateLimit>>,
) -> Rc<AgentServer<ConversationHarness>> {
    let stores: TreeStores = {
        let services = services.clone();
        Rc::new(move |root: &NodeId| {
            let db = services.conversations.db(&conversation_of(root));
            Rc::new(SqliteTreeStore::new(db)) as Rc<dyn AgentTreeStore>
        })
    };
    let config = ServerConfig {
        outbox_frames: services.conversation_tuning.outbox_frames,
        ..ServerConfig::default()
    };
    AgentServer::new(ServerDeps {
        harness: Rc::new(ConversationHarness::new()),
        providers: Rc::new(ConversationProviders::new(user, services.clone(), http, rate_limit)),
        shells: Rc::new(NoShellEnvironments),
        stores,
        clock: services.clock.clone(),
        ids: Rc::new(RandomIds),
        config,
    })
}

/// The root node of a conversation's tree, whose id is the conversation's in
/// the spelling the index keeps.
pub(crate) fn root_of(conversation: &ConversationId) -> NodeId {
    NodeId::try_from(conversation.as_str()).expect("a conversation id is never empty")
}

/// The conversation a tree's root belongs to. The shard opens trees only
/// under the roots `root_of` names, so every root is a conversation id.
pub(crate) fn conversation_of(root: &NodeId) -> ConversationId {
    ConversationId::try_from(root.as_str()).expect("the shard opens trees only for conversations")
}
