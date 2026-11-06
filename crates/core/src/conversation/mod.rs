//! Conversations on the user's core (`crates/backend/src/lib.rs`).
//!
//! The module holds:
//!
//! - the agent server that hosts each conversation's tree over the
//!   conversation's database;
//! - the conversation lane;
//! - the provider runtimes the sessions infer with;
//! - the Claude Code CLI's work on the user's Cloud, and the provider test;
//! - the conversations' summaries and the failure facts of their history;
//! - the resume of a failed turn at its retry time;
//! - their deletion, and the search index that follows them.

mod announcement;
pub mod claude_cli;
mod cloud_project;
pub use cloud_project::CloudProject;
mod connection_test;
pub mod cost;
mod cost_report;
mod creation;
mod deletion;
mod attachment_facts;
mod failure_facts;
mod history;
pub(crate) mod idle_compaction;
mod fork;
mod instructions;
mod live_reads;
pub(crate) mod notifications;
pub(crate) mod project_files;
pub(crate) mod organize;
pub(crate) mod product;
pub mod providers;
pub(crate) mod restart_resume;
pub(crate) mod retry_resume;
mod runs_on;
pub mod side_questions;
pub mod search;
pub mod settings;
mod lane;
mod summary;
mod turn_timing;
pub(crate) mod small_requests;
pub(crate) mod suggested_replies;
pub(crate) mod titles;
pub mod transition;
pub mod workflows;

use demi_core_store::store::CoreStore;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use demi_agent_server::{AgentServer, ServerConfig, ServerDeps, TreeStores};
use demi_agent_store::AgentTreeStore;
use demi_agent_tools::ContextSource;
use instructions::Instructions;
use demi_agent_transcript::RandomIds;
use demi_agent_store::media::BlobStore;
use demi_core_store::search::TranscriptChange;
use demi_core_store::tree::SqliteTreeStore;
use demi_core_host_access::shells::CoreShellEnvironments;
use demi_core_host_access::{HostCore, conversation_of};
use demi_core_sync::Part;
use demi_shared_types::NodeId;
use demi_web_api_protocol::ids::UserId;

pub use self::attachment_facts::{attachment_facts, named_blocks, replies, reply_block_facts};
pub use self::failure_facts::failure_facts;
pub use self::fork::{ForkRefusal, recover_forks};
pub use self::announcement::AgentsError;
pub(crate) use self::product::CoreHosts;
pub use self::providers::{lent_limits, lent_runtime};
use self::product::{
    ExecutionContext, HARNESS_GUIDE, INSTRUCTIONS, PluginContext, CoreAgents, CoreReplyFiles, CoreSubagents, CoreToolsets,
};
use self::providers::ConversationProviders;
use self::small_requests::SmallRequests;
use self::suggested_replies::SuggestedReplies;
use self::titles::Titles;
use crate::services::CoreServices;
use crate::Core;

/// What a core's conversations run on: the agent server, and the title and
/// suggested replies requests, small requests that infer with the same
/// providers.
pub(crate) struct ConversationParts {
    pub agent: Rc<AgentServer<CoreHosts>>,
    pub titles: Titles,
    pub suggested_replies: SuggestedReplies,
}

/// The conversation parts of `core`, the user's.
///
/// The agent server's trees keep their state in the conversations' databases.
/// Their sessions infer with the user's providers under the user's rate limit,
/// as the title requests do, and their nodes' shells run on the conversations'
/// Hosts through the core's host access.
pub(crate) fn conversation_parts(
    core: Weak<Core>,
    user: UserId,
    store: CoreStore,
    services: Arc<CoreServices>,
    http: reqwest::Client,
) -> ConversationParts {
    let marks = services.sync.of(&user);
    let stores: TreeStores = {
        let services = services.clone();
        let marks = marks.clone();
        let core = core.clone();
        let databases = store.databases().clone();
        // The core's user owns every conversation it hosts.
        let blobs: Arc<dyn BlobStore> = Arc::new(services.blobs.for_user(&user));
        Rc::new(move |root: &NodeId| {
            let conversation = conversation_of(root);
            let db = databases.db(&conversation);
            // The summary reads the root's checkpoint, which each save of it
            // changes, and the search index follows what each save of any
            // agent wrote.
            let saved = {
                let marks = marks.clone();
                let root = root.clone();
                let core = core.clone();
                Rc::new(move |node: &NodeId, change: Option<TranscriptChange>| {
                    if let (Some(change), Some(core)) = (change, core.upgrade()) {
                        core.follow_save(conversation.clone(), change);
                    }
                    if *node == root {
                        marks.mark(Part::Conversation(conversation.clone()));
                    }
                })
            };
            Rc::new(SqliteTreeStore::new(db, blobs.clone(), saved)) as Rc<dyn AgentTreeStore>
        })
    };
    let config = ServerConfig {
        outbox_steps: services.conversation_tuning.outbox_steps,
        call_window_ms: services.conversation_tuning.call_window_ms,
        session: demi_agent_session::SessionConfig {
            silence: services.conversation_tuning.silence,
            ..demi_agent_session::SessionConfig::default()
        },
        ..ServerConfig::default()
    };
    let providers = Rc::new(ConversationProviders::new(
        core.clone(),
        user.clone(),
        store.clone(),
        services.clone(),
        http,
    ));
    let small_requests = Rc::new(SmallRequests::new(providers.clone(), user.clone(), services.clone()));
    let suggested_replies = SuggestedReplies::new(small_requests.clone(), store.clone(), marks.clone());
    let titles = Titles::new(
        small_requests,
        store.clone(),
        marks.clone(),
        services.conversation_tuning.titles,
    );
    let hosts: Weak<dyn HostCore> = core.clone();
    let agent = AgentServer::new(ServerDeps {
        toolsets: Rc::new(CoreToolsets {
            core: core.clone(),
        }),
        subagents: Rc::new(CoreSubagents {
            core: core.clone(),
        }),
        instructions: Rc::from(INSTRUCTIONS),
        guide: Rc::from(HARNESS_GUIDE),
        hosts: Rc::new(CoreHosts {
            core: core.clone(),
        }),
        context: context_sources(&core, &services),
        providers,
        shells: Rc::new(CoreShellEnvironments::new(
            hosts,
            services.packages.catalog(),
        )),
        stores,
        clock: services.clock.clone(),
        ids: Rc::new(RandomIds),
        config,
        status_changed: {
            let core = core.clone();
            Rc::new(move |root: &NodeId| {
                let conversation = conversation_of(root);
                if let Some(core) = core.upgrade() {
                    core.follow_runs(&conversation);
                    core.follow_failed_turn(&conversation);
                    core.follow_turn_end(&conversation);
                    core.follow_suggested_replies(&conversation);
                    core.follow_idle_compaction_end(&conversation);
                    // A conversation no longer loaded keeps no project files.
                    if core.agent().tree(root).is_none() {
                        core.project_files().release(&conversation);
                    }
                }
                marks.mark(Part::Conversation(conversation));
            })
        },
        agents: Rc::new(CoreAgents { core: core.clone() }),
        reply_files: Rc::new(CoreReplyFiles { core: core.clone() }),
        idle_compaction_due: {
            let core = core.clone();
            Rc::new(move |root: &NodeId, due| {
                if let Some(core) = core.upgrade() {
                    core.follow_idle_compaction(&conversation_of(root), due);
                }
            })
        },
    });
    ConversationParts { agent, titles, suggested_replies }
}

/// The context sources a node asks before each provider request, in order:
/// the product's execution context, the instructions, the side questions,
/// then each plugin that is one, in registration order.
fn context_sources(core: &Weak<Core>, services: &CoreServices) -> Rc<[Rc<dyn ContextSource>]> {
    let execution = Rc::new(ExecutionContext {
        core: core.clone(),
    }) as Rc<dyn ContextSource>;
    let instructions = Rc::new(Instructions::new(core.clone())) as Rc<dyn ContextSource>;
    let side_questions = Rc::new(side_questions::Told::new(core.clone())) as Rc<dyn ContextSource>;
    let plugins = services
        .plugins
        .context_sources()
        .into_iter()
        .map(|plugin| {
            Rc::new(PluginContext {
                core: core.clone(),
                plugin,
            }) as Rc<dyn ContextSource>
        });
    [execution, instructions, side_questions].into_iter().chain(plugins).collect()
}
