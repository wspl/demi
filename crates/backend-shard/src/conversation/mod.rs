//! Conversations on the user's shard (`backend.md` § Request paths and
//! responsibilities): the agent server that hosts each conversation's tree
//! over the conversation's database, the conversation socket, the provider
//! runtimes the sessions infer with, the Claude Code CLI's work on the
//! user's Cloud and the provider test, the conversations' summaries, and the
//! failure facts of their history.

mod announcement;
pub mod claude_cli;
mod cloud_workspace;
mod connection_test;
mod failure_facts;
mod fork;
mod harness;
mod providers;
pub mod settings;
mod socket;
mod summary;
pub mod titles;
pub mod transition;

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use demi_agent::{AgentServer, ServerConfig, ServerDeps, TreeStores};
use demi_agent_store::AgentTreeStore;
use demi_agent_transcript::RandomIds;
use demi_backend_host_access::blobs::ConversationBlobs;
use demi_backend_host_access::shells::ShardShellEnvironments;
use demi_backend_host_access::{HostShard, conversation_of};
use demi_backend_providers::usage::rate_limit::RequestRateLimit;
use demi_backend_storage::blob_refs::OwnerBlobs;
use demi_backend_storage::tree::SqliteTreeStore;
use demi_backend_sync::Part;
use demi_core::NodeId;
use demi_web_api::ids::UserId;

pub use self::failure_facts::failure_facts;
pub use self::fork::{ForkRefusal, recover_forks};
pub(crate) use self::harness::ConversationHarness;
use self::harness::conversation_harness;
use self::providers::ConversationProviders;
use self::titles::Titles;
use crate::services::Services;
use crate::shard::Shard;

/// What a shard's conversations run on: the agent server and the title
/// requests, which infer with the same providers.
pub(crate) struct ConversationParts {
    pub agent: Rc<AgentServer<ConversationHarness>>,
    pub titles: Titles,
}

/// The conversation parts of `shard`, the user's. The agent server's trees
/// keep their state in the conversations' databases, their sessions infer
/// with the user's providers under the user's rate limit, as the title
/// requests do, and their nodes' shells run on the conversations' Hosts
/// through the shard's host access.
pub(crate) fn conversation_parts(
    shard: Weak<Shard>,
    user: UserId,
    services: Arc<Services>,
    http: reqwest::Client,
    rate_limit: Rc<RefCell<RequestRateLimit>>,
) -> ConversationParts {
    let marks = services.sync.of(&user);
    let stores: TreeStores = {
        let services = services.clone();
        let marks = marks.clone();
        // The shard's user owns every conversation it hosts.
        let blobs: Arc<dyn OwnerBlobs> = Arc::new(ConversationBlobs(services.blobs.for_user(&user)));
        Rc::new(move |root: &NodeId| {
            let conversation = conversation_of(root);
            let db = services.conversations.db(&conversation);
            // The summary reads the root's checkpoint, which each save of it
            // changes.
            let saved = {
                let marks = marks.clone();
                let root = root.clone();
                Rc::new(move |node: &NodeId| {
                    if *node == root {
                        marks.mark(Part::Conversation(conversation.clone()));
                    }
                })
            };
            Rc::new(SqliteTreeStore::new(db, blobs.clone(), saved)) as Rc<dyn AgentTreeStore>
        })
    };
    let config = ServerConfig {
        outbox_frames: services.conversation_tuning.outbox_frames,
        ..ServerConfig::default()
    };
    let providers = Rc::new(ConversationProviders::new(
        shard.clone(),
        user,
        services.clone(),
        http,
        rate_limit,
    ));
    let titles = Titles::new(
        providers.clone(),
        services.control.clone(),
        marks.clone(),
        services.conversation_tuning.titles,
    );
    let disposals = shard.clone();
    let hosts: Weak<dyn HostShard> = shard.clone();
    let agent = AgentServer::new(ServerDeps {
        harness: Rc::new(conversation_harness(shard.clone(), &services.native)),
        providers,
        shells: Rc::new(ShardShellEnvironments::new(hosts, services.native.catalog(&services.public_url))),
        stores,
        clock: services.clock.clone(),
        ids: Rc::new(RandomIds),
        config,
        status_changed: Rc::new(move |root: &NodeId| {
            let conversation = conversation_of(root);
            marks.mark(Part::Conversation(conversation.clone()));
            // A tree the server no longer holds was disposed, and its
            // conversation's tool media may be retired now (`storage.md`
            // § Retiring tool media).
            if let Some(shard) = disposals.upgrade()
                && shard.agent().tree(root).is_none()
            {
                shard.tree_disposed(&conversation);
            }
        }),
    });
    ConversationParts { agent, titles }
}
