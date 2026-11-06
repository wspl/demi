//! The services every core of a process shares.
//!
//! A program that runs one core gives it one set; the server gives one set
//! to all of its users' cores. They span the cores, such as the vault over
//! each core's store, the model catalogs and the accounts' rotation.

use std::sync::Arc;

use demi_core_blobs::blobs::BlobStores;
use demi_core_host_access::stream::UserStreams;
use demi_core_sync::SyncRegistry;
use demi_plugin_host::Registry;
use demi_provider_registry::registry::ProviderRegistry;
use demi_provider_claude_code::releases::ClaudeReleases;
use demi_core_providers::vault::entries::Vault;
use demi_core_providers::vault::logins::LoginFlows;
use demi_core_providers::vault::operations::ProviderOperations;
use demi_core_hosts::devices::Returning;
use demi_core_hosts::packages::Packages;
use demi_core_hosts::public_url::PublicUrl;
use demi_runner_protocol::direct::StunUrl;
use demi_provider_common::media_catalog::MediaCatalog;
use demi_shared_types::Clock;

use crate::conversation::claude_cli::{CliInstalls, MachineSlot};
use crate::tuning::{ConversationTuning, LifecycleTuning, MediaTuning, RunnerTuning};

/// The services every core of this process shares: one set for each core
/// a program runs, one set for all of a server's users' cores.
pub struct CoreServices {
    /// Where runners and Cloud guests reach this backend, and where a
    /// borrowing server reaches an entry this one lends.
    pub public_url: PublicUrl,
    /// The wall clock the backend reads times from.
    pub clock: Arc<dyn Clock>,
    pub blobs: BlobStores,
    pub vault: Vault,
    pub provider_registry: Arc<ProviderRegistry>,
    /// The catalog of image and video models (`crates/provider-common/src/media_catalog.rs`).
    pub media: MediaCatalog,
    /// The vendor's Claude Code releases, which the CLI on the Claude Code machine follows.
    pub claude_releases: ClaudeReleases,
    /// The outcome of the CLI installs no conversation asked for.
    pub cli_installs: CliInstalls,
    /// The Claude Code machine, set as the shards start.
    pub claude_machine: MachineSlot,
    pub operations: Arc<ProviderOperations>,
    pub logins: Arc<LoginFlows>,
    /// The calls the schedules' URLs took this minute.
    pub hook_limits: crate::schedules::HookLimits,
    pub runners: RunnerTuning,
    pub conversation_tuning: ConversationTuning,
    /// How a video's job is waited for.
    pub media_tuning: MediaTuning,
    /// The same packages as the shards run them, at this backend's address.
    pub packages: Arc<dyn Packages>,
    /// The backend's plugins, checked at startup (`crates/plugin-host/src/lib.rs`).
    pub plugins: Arc<Registry>,
    /// The user streams a page may open.
    pub user_streams: UserStreams,
    /// The STUN servers pages and runners ask for their public address
    /// (`crates/runner-direct/src/lib.rs`).
    pub stun: Vec<StunUrl>,
    /// This run of the backend, chosen when the services start: the
    /// revisions counted in memory compare only within one run.
    pub run: String,
    /// When a conversation's Host resources are reclaimed.
    pub lifecycle: LifecycleTuning,
    /// Each user's open synchronization channels, which every change a
    /// page shows marks.
    pub sync: SyncRegistry,
    /// The devices the backend's last shutdown disconnected, which Host
    /// operations wait for in the grace after the start
    /// (`crates/runner-client/src/device_jobs.rs`).
    pub returning: Arc<Returning>,
}
