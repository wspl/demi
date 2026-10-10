//! Conversations on the user's shard (`backend.md` § Request paths and
//! responsibilities): the agent server that hosts each conversation's tree
//! over the conversation's database, the conversation socket, the provider
//! runtimes the sessions infer with, the Claude Code CLI's work on the
//! user's Cloud and the provider test, the conversations' summaries, and the
//! failure facts of their history, their deletion, and the search index
//! that follows them.

mod announcement;
pub mod claude_cli;
mod cloud_workspace;
mod connection_test;
mod creation;
mod deletion;
mod failure_facts;
mod fork;
mod instructions;
pub(crate) mod organize;
pub(crate) mod pending;
pub(crate) mod product;
mod providers;
pub mod search;
pub mod settings;
mod socket;
mod summary;
pub mod titles;
pub mod transition;

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use demi_agent_server::{AgentServer, ServerConfig, ServerDeps, TreeStores};
use demi_agent_store::AgentTreeStore;
use demi_agent_tools::ContextSource;
use instructions::Instructions;
use demi_agent_transcript::RandomIds;
use demi_agent_store::media::BlobStore;
use demi_backend_database::tree::SqliteTreeStore;
use demi_backend_host_access::shells::ShardShellEnvironments;
use demi_backend_host_access::{HostShard, conversation_of};
use demi_backend_page_sync::Part;
use demi_backend_providers::usage::rate_limit::RequestRateLimit;
use demi_shared_types::NodeId;
use demi_web_api_protocol::ids::UserId;

pub use self::deletion::finish_deletions;
pub use self::failure_facts::failure_facts;
pub use self::fork::{ForkRefusal, recover_forks};
pub use self::pending::settle_pending;
pub(crate) use self::product::ShardHosts;
use self::product::{
    ExecutionContext, HARNESS_GUIDE, INSTRUCTIONS, PluginContext, ShardSubagents, ShardToolsets,
};
use self::providers::ConversationProviders;
use self::titles::Titles;
use crate::services::Services;
use crate::shard::Shard;

/// What a shard's conversations run on: the agent server and the title
/// requests, which infer with the same providers.
pub(crate) struct ConversationParts {
    pub agent: Rc<AgentServer<ShardHosts>>,
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
        let blobs: Arc<dyn BlobStore> = Arc::new(services.blobs.for_user(&user));
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
        interval_floor_ms: services.conversation_tuning.interval_floor_ms,
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
    let hosts: Weak<dyn HostShard> = shard.clone();
    let agent = AgentServer::new(ServerDeps {
        toolsets: Rc::new(ShardToolsets {
            shard: shard.clone(),
        }),
        subagents: Rc::new(ShardSubagents {
            shard: shard.clone(),
        }),
        instructions: Rc::from(INSTRUCTIONS),
        guide: Rc::from(HARNESS_GUIDE),
        hosts: Rc::new(ShardHosts {
            shard: shard.clone(),
        }),
        context: context_sources(&shard, &services),
        providers,
        shells: Rc::new(ShardShellEnvironments::new(
            hosts,
            services.native.catalog(&services.public_url),
        )),
        stores,
        clock: services.clock.clone(),
        ids: Rc::new(RandomIds),
        config,
        status_changed: {
            let shard = shard.clone();
            Rc::new(move |root: &NodeId| {
                let conversation = conversation_of(root);
                if let Some(shard) = shard.upgrade() {
                    shard.poke_pending(&conversation);
                }
                marks.mark(Part::Conversation(conversation));
            })
        },
    });
    ConversationParts { agent, titles }
}

/// The context sources a node asks before each provider request, in order:
/// the product's execution context, the instructions, then each plugin that
/// is one, in registration order.
fn context_sources(shard: &Weak<Shard>, services: &Services) -> Rc<[Rc<dyn ContextSource>]> {
    let execution = Rc::new(ExecutionContext {
        shard: shard.clone(),
    }) as Rc<dyn ContextSource>;
    let instructions = Rc::new(Instructions::new(shard.clone())) as Rc<dyn ContextSource>;
    let plugins = services
        .plugins
        .context_sources()
        .into_iter()
        .map(|plugin| {
            Rc::new(PluginContext {
                shard: shard.clone(),
                plugin,
            }) as Rc<dyn ContextSource>
        });
    [execution, instructions].into_iter().chain(plugins).collect()
}
