//! Conversations on the user's shard (`backend.md` § Request paths and
//! responsibilities): the agent server that hosts each conversation's tree
//! over the conversation's database, the conversation socket, the provider
//! runtimes the sessions infer with, the conversations' summaries, and the
//! failure facts of their history.

mod announcement;
mod cloud_workspace;
mod failure_facts;
mod fork;
mod harness;
pub(crate) mod host_access;
mod providers;
pub(crate) mod remote_files;
pub(crate) mod settings;
mod shells;
mod socket;
pub(crate) mod stream;
mod summary;
pub(crate) mod target;
pub(crate) mod transfer;
pub(crate) mod titles;
pub(crate) mod transition;
mod uploads;

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use demi_agent::{AgentServer, AgentTreeStore, RandomIds, ServerConfig, ServerDeps, TreeStores};
use demi_core::NodeId;
use demi_web_api::ids::{ConversationId, UserId};

pub(crate) use self::failure_facts::failure_facts;
pub(crate) use self::fork::{ForkRefusal, recover_forks};
pub(crate) use self::harness::ConversationHarness;
use self::harness::conversation_harness;
use self::providers::ConversationProviders;
use self::shells::ShardShellEnvironments;
use self::titles::Titles;
use crate::backend::Services;
use crate::shard::Shard;
use crate::storage::tree::SqliteTreeStore;
use crate::sync::Part;
use crate::usage::rate_limit::RequestRateLimit;

/// What a shard's conversations run on: the agent server and the title
/// requests, which infer with the same providers.
pub(crate) struct ConversationParts {
    pub(crate) agent: Rc<AgentServer<ConversationHarness>>,
    pub(crate) titles: Titles,
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
        let blobs = services.blobs.for_user(&user);
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
    let agent = AgentServer::new(ServerDeps {
        harness: Rc::new(conversation_harness(shard.clone(), &services.native)),
        providers,
        shells: Rc::new(ShardShellEnvironments::new(shard, services.native.catalog(&services.public_url))),
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
