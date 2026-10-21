//! The composition root: `Backend::start` opens storage, starts the shared
//! services and the shard threads, and opens the listener;
//! `Backend::close` shuts them down in order.
//!
//! # Start
//!
//! Start recovers before it serves:
//!
//! - The machine manager settles what an earlier backend left and keeps every
//!   running Cloud and its sandbox.
//! - An interrupted Cloud reset finishes committing its disks and is marked
//!   failed, so that a retry starts the Cloud.
//! - The Fork creations whose destination root committed are published.
//! - Running commands are taken up as their runners connect, and a command
//!   that reports restores its conversation's tree.
//! - The manager's first message names its wire version, so a manager of
//!   another release stops the start here.
//! - The turns a graceful shutdown interrupted resume in the background, each
//!   owner's on its shard, and the start waits for none of them.
//!
//! The listener opens last, so nothing reaches a half-recovered backend.
//!
//! # Shutdown
//!
//! Shutdown closes the listener first, so no new work starts and no runner
//! reconnects into a backend that is closing:
//!
//! - A request on a connection already open answers 503 `backend_closing`.
//! - A pipe a runner would open is refused, so shutdown reads nothing from a
//!   Host.
//! - It stops nothing that runs on a Host: commands and Clouds go on, and the
//!   next start takes them up.
//!
//! Every step runs even when an earlier one fails, the failures are reported
//! together, and the process exits with a failure status.
//!
//! A page cannot hold up shutdown, nor can a Host:
//!
//! - A page socket's close waits at most a second for the page to take the
//!   close frame.
//! - A runner connection's end waits at most five seconds for the work it
//!   carries.

use std::fmt;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use demi_backend_blobs::ObjectError;
use demi_backend_blobs::store as objects;
use demi_backend_runners::install::{RunnerReleases, read_runner_release};
use demi_backend_runners::native::NativeCatalog;
use demi_runner_protocol::release::RunnerRelease;
use demi_backend_runners::publication::{PublicationError, publish_native, release_files};
use demi_backend_cloud::CloudServices;
use demi_backend_cloud::client::MachinesClient;
use demi_backend_cloud::reset::recover_clouds;
use demi_backend_database::StorageError;
use demi_backend_database::server_log::ServerLog;
use demi_backend_http::{AppState, Edge, Site, WebBuildError, manage_socket, runner_socket, web_build};
use demi_backend_user_shard::conversation::cost::follow_usage;
use demi_backend_user_shard::conversation::search::{follow_resets, index_at_start};
use demi_backend_user_shard::conversation::{
    finish_deletions, recover_forks, resume_after_restart, settle_pending,
};
use demi_backend_user_shard::shard::deliver_decisions;
use demi_backend_user_shard::services::{
    CloseError, ProviderSetup, ServiceKeys, ServiceSettings, Services, ServicesError, Storage,
};
use demi_backend_user_shard::shard::ShardPool;
use demi_backend_user_shard::shard::cloud::route_deaths;
use tokio_util::task::AbortOnDropHandle;

use crate::config::BackendConfig;
use crate::config::secret::{InstanceSecret, SecretError};

/// A running backend.
pub struct Backend {
    local_addr: SocketAddr,
    storage: Storage,
    services: Arc<Services>,
    shards: ShardPool,
    edge: Edge,
    /// Routes the machine manager's death events to their owners' shards.
    deaths: AbortOnDropHandle<()>,
    /// Has the users whose search index is reset index their conversations
    /// again (`crates/backend-database/src/search.rs`).
    resets: AbortOnDropHandle<()>,
    /// Announces each ledger row to its conversation's pages.
    usage: AbortOnDropHandle<()>,
    /// Sources the runner executables of the paired devices' systems
    /// (`crates/backend-http/src/install.rs`); a close ends it.
    runners: Option<AbortOnDropHandle<()>>,
    /// Stores the package executables ahead for the targets of the Hosts
    /// (`crates/backend-runners/src/publication.rs`); a close ends it.
    ahead: AbortOnDropHandle<()>,
    /// The hold `BackendConfig::migrations_held` put on the migrations as
    /// the stores opened, until a test takes it.
    #[cfg(feature = "testing")]
    held_migrations: std::sync::Mutex<Option<demi_backend_database::conversations::MigrationHold>>,
}

/// Why the backend did not start.
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error("the data directory {} cannot be created: {source}", path.display())]
    DataDirectory { path: PathBuf, source: io::Error },
    #[error(transparent)]
    Secret(#[from] SecretError),
    #[error("storage cannot be opened: {0}")]
    Storage(#[from] StorageError),
    #[error("the object store cannot be opened: {0}")]
    Objects(#[from] ObjectError),
    #[error(transparent)]
    Publication(#[from] PublicationError),
    #[error("the release's runner release cannot be read: {0}")]
    RunnerRelease(String),
    #[error(transparent)]
    Services(#[from] ServicesError),
    #[error("the shard threads cannot start: {0}")]
    Shards(io::Error),
    #[error("the backend cannot listen on {address}: {source}")]
    Listen {
        address: SocketAddr,
        source: io::Error,
    },
    #[error("the backend cannot listen on its runner socket {}: {source}", path.display())]
    RunnerSocket {
        path: std::path::PathBuf,
        source: io::Error,
    },
    #[error("the backend cannot listen on its management socket {}: {source}", path.display())]
    ManageSocket {
        path: std::path::PathBuf,
        source: io::Error,
    },
    /// The machine manager did not reconcile, or an interrupted reset could
    /// not be finished.
    #[error("the Clouds cannot be recovered: {0}")]
    Cloud(String),
    #[error(transparent)]
    WebBuild(#[from] WebBuildError),
}

/// A shutdown step that failed; the steps after it ran all the same.
#[derive(Debug, thiserror::Error)]
pub enum ShutdownError {
    #[error("the listener did not stop cleanly: {0}")]
    Edge(io::Error),
    #[error(transparent)]
    Storage(#[from] CloseError),
}

/// Every shutdown step that failed.
#[derive(Debug)]
pub struct ShutdownErrors(pub Vec<ShutdownError>);

impl fmt::Display for ShutdownErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let steps: Vec<String> = self.0.iter().map(ToString::to_string).collect();
        write!(f, "shutdown failed: {}", steps.join("; "))
    }
}

impl std::error::Error for ShutdownErrors {}

/// Hands each running Cloud to its owner's shard, which takes it over
/// (`crates/machine-manager-protocol/src/lib.rs`).
async fn take_over_clouds(
    running: Vec<demi_backend_database::devices::DeviceRecord>,
    shards: &demi_backend_user_shard::shard::Shards,
) -> Result<(), String> {
    for device in running {
        let owner = device.user.clone();
        shards
            .of(&owner)
            .call(move |shard, _| async move { shard.cloud_shard().take_over_cloud(&device).await })
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Forgets each Cloud the last shutdown disconnected that does not run, as
/// after an upgrade stopped it: no operation waits for its runner, which
/// does not come back by itself (`crates/backend-remote-host/src/device_jobs.rs`).
async fn forget_stopped_clouds(
    running: &[demi_backend_database::devices::DeviceRecord],
    services: &demi_backend_user_shard::services::Services,
) -> Result<(), demi_backend_database::StorageError> {
    for device in services.returning.devices() {
        if running.iter().any(|running| running.id == device) {
            continue;
        }
        let managed = services
            .control
            .device(device.clone())
            .await?
            .is_some_and(|record| record.kind == demi_web_api_protocol::devices::DeviceKind::Managed);
        if managed {
            services.returning.forget(&device);
        }
    }
    Ok(())
}

/// Restores the trees whose commands ran on a Cloud that does not run, other
/// than the `taken` ones, so their agents learn what became of them.
async fn take_up_stopped_clouds(
    taken: &[demi_web_api_protocol::ids::DeviceId],
    control: &demi_backend_database::control::ControlService,
    shards: &demi_backend_user_shard::shard::Shards,
) -> Result<(), String> {
    let stopped = control
        .clouds_with_running_jobs()
        .await
        .map_err(|error| error.to_string())?;
    for (device, owner) in stopped {
        if taken.contains(&device) {
            continue;
        }
        shards
            .of(&owner)
            .call(move |shard, _| async move { shard.take_up_stopped_cloud(device).await })
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Publishes the command packages of the server release whose root is
/// `release`: its `commands/`, whose executables come from the files its
/// `release.json` names, into the object store `config` names, before the
/// backend that serves them starts (`crates/backend-runners/src/publication.rs`); `cancel` interrupts it.
pub async fn publish_commands(
    config: &BackendConfig,
    release: &std::path::Path,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<NativeCatalog, StartError> {
    let files = release_files(release).await?;
    let objects = objects::open(&config.data_dir, &config.storage).await?;
    let commands = release.join("commands");
    let runners = read_runner_release(&release.join("runners/manifest.json"), RunnerRelease::decode)
        .await
        .map_err(StartError::RunnerRelease)?;
    Ok(publish_native(
        &commands,
        files,
        demi_shared_artifacts::WORKSPACE_VERSION,
        runners.as_ref(),
        &objects,
        cancel,
    )
    .await?)
}

impl Backend {
    /// Starts the backend. It serves once this returns: the data directory
    /// and the instance secret, then the databases and the object store, the
    /// shared services and the shard threads, and the listener last.
    pub async fn start(config: BackendConfig) -> Result<Self, StartError> {
        let data_dir = config.data_dir.clone();
        tokio::fs::create_dir_all(&data_dir)
            .await
            .map_err(|source| StartError::DataDirectory {
                path: data_dir.clone(),
                source,
            })?;
        let secret = match &config.instance_secret {
            Some(secret) => secret.clone(),
            None => InstanceSecret::load_or_create(&data_dir).await?,
        };
        // The object store: the data directory or the S3 bucket.
        let objects = objects::open(&data_dir, &config.storage).await?.store;
        #[cfg(feature = "testing")]
        let objects = match &config.object_counts {
            Some(counts) => counts.observe(objects),
            None => objects,
        };
        let storage = Storage::open(&data_dir, config.clock.clone(), objects, secret.vault_key()).await?;
        #[cfg(feature = "testing")]
        let held_migrations = config
            .migrations_held
            .then(|| storage.conversations.hold_migrations());
        let started = Self::serve(config, storage.clone(), &secret).await;
        #[cfg(feature = "testing")]
        if let Ok(backend) = &started {
            *backend
                .held_migrations
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = held_migrations;
        }
        if started.is_err() {
            for failure in storage.close().await {
                tracing::error!(
                    error = &failure as &dyn std::error::Error,
                    "storage did not close after a failed start"
                );
            }
        }
        started
    }

    async fn serve(
        config: BackendConfig,
        storage: Storage,
        secret: &InstanceSecret,
    ) -> Result<Self, StartError> {
        let providers = ProviderSetup {
            families: config.families,
            models_dev_url: config.models_dev_url,
            media_catalog_url: config.media_catalog_url,
            media_catalog: config.media_catalog,
            claude_releases: config.claude_releases,
            logins: config.logins,
            clock: config.clock,
            master_account: config.master_account,
        };
        let (machines, deaths) = MachinesClient::new(config.machines_socket);
        let keys = ServiceKeys {
            vault: secret.vault_key(),
            email_codes: secret.email_code_key(),
            push: secret.push_key(),
        };
        let web_build = config
            .web_directory
            .as_deref()
            .map(web_build)
            .transpose()?;
        let settings = ServiceSettings {
            mode: config.mode,
            web_build,
            runner_releases: RunnerReleases::new(config.runner_releases),
            stun: config.stun,
            mail: config.account_mail,
            runners: config.runners,
            conversations: config.conversations,
            pages: config.pages,
            media_tuning: config.media,
            native: config.native,
            plugins: config.plugins,
            cloud: CloudServices::new(machines, config.cloud),
            lifecycle: config.lifecycle,
            #[cfg(feature = "testing")]
            web_roots: config.web_roots,
            #[cfg(feature = "testing")]
            push_transport: config.push_transport,
        };
        let services = Services::start(storage.clone(), keys, providers, settings);
        let services = Arc::new(services.await?);
        let shards = match ShardPool::start(config.shards, services.clone()).await {
            Ok(shards) => shards,
            Err(error) => {
                services.close_providers().await;
                return Err(StartError::Shards(error));
            }
        };
        let deaths = AbortOnDropHandle::new(tokio::spawn(route_deaths(
            deaths,
            services.clone(),
            shards.shards(),
        )));
        let resets = AbortOnDropHandle::new(tokio::spawn(follow_resets(
            services.search.resets(),
            services.control.clone(),
            shards.shards(),
        )));
        let usage = AbortOnDropHandle::new(tokio::spawn(follow_usage(
            services.control.usage_written(),
            shards.shards(),
        )));
        // Before the backend serves, the machine manager settles what an
        // earlier backend left, which keeps every running Cloud, and resets
        // it left unfinished commit their disks; each running Cloud's shard
        // takes it over.
        let running = match recover_clouds(&services.control, &services.cloud).await {
            Ok(running) => running,
            Err(error) => {
                shards.close().await;
                services.close_providers().await;
                return Err(StartError::Cloud(error.to_string()));
            }
        };
        if let Err(error) = take_over_clouds(running.clone(), &shards.shards()).await {
            shards.close().await;
            services.close_providers().await;
            return Err(StartError::Cloud(error));
        }
        // A Cloud the manager does not run has no runner to wait for.
        if let Err(error) = forget_stopped_clouds(&running, &services).await {
            shards.close().await;
            services.close_providers().await;
            return Err(StartError::Storage(error));
        }
        // Fork destinations whose root committed before their publication are
        // published before the backend serves.
        if let Err(error) = recover_forks(&services.control, &services.conversations).await {
            shards.close().await;
            services.close_providers().await;
            return Err(StartError::Storage(error));
        }
        // Each deletion an earlier backend left pending is finished before
        // the backend serves. A failure does not stop the start: the next
        // start finishes it, and no request finds its conversation meanwhile.
        if let Err(error) = finish_deletions(&services.control, &shards.shards()).await {
            tracing::error!(
                error = &error as &dyn std::error::Error,
                "the pending deletions cannot be listed"
            );
        }
        // Each detach an agent asked for that a restart cut off is made,
        // before a page opens its tree
        // (`crates/backend-user-shard/src/conversation/pending.rs`). A failure
        // does not stop the start: the next start makes it.
        if let Err(error) = settle_pending(&services.control, &shards.shards()).await {
            tracing::error!(
                error = &error as &dyn std::error::Error,
                "the pending detaches cannot be listed"
            );
        }
        // A credential request's command ended with the run before, so no one waits
        // for its answer; and a credential whose time came while the backend was down
        // is removed.
        //
        // A failure does not stop the start: the user's shard removes what is past due
        // when it starts, and the next start withdraws the requests.
        if let Err(error) = services.control.withdraw_credential_requests().await {
            tracing::error!(
                error = &error as &dyn std::error::Error,
                "the credential requests of the run before cannot be withdrawn"
            );
        }
        if let Err(error) = services.control.expire_all_user_credentials().await {
            tracing::error!(
                error = &error as &dyn std::error::Error,
                "the expired credentials cannot be removed"
            );
        }
        // Each permission decision whose message a restart cut off reaches
        // the agent that asked. A failure does not stop the start: the next
        // start delivers it.
        if let Err(error) = deliver_decisions(&services.control, &shards.shards()).await {
            tracing::error!(
                error = &error as &dyn std::error::Error,
                "the undelivered permission decisions cannot be listed"
            );
        }
        // Each root's turn the last shutdown interrupted goes on, in the
        // background, as the user's Resume would, unless the user turned that
        // off. A failure does not stop the start: the marks stay for the next
        // start, and meanwhile the turns wait for the user's Resume.
        if let Err(error) = resume_after_restart(&services.control, &shards.shards()).await {
            tracing::error!(
                error = &error as &dyn std::error::Error,
                "the turns the last shutdown interrupted cannot be listed"
            );
        }
        // Each user's automations fire what a backend that was down missed
        // and wait for their next times. A failure does not stop the start: a
        // user's next call starts them.
        if let Err(error) =
            demi_backend_user_shard::automations::start_schedulers(&services.control, &shards.shards()).await
        {
            tracing::error!(
                error = &error as &dyn std::error::Error,
                "the users with automations cannot be listed"
            );
        }
        // Each user's search index catches up with the conversations, in the
        // background (`crates/backend-database/src/search.rs`). A failure
        // does not stop the start: a conversation not indexed here is
        // indexed by its next change.
        if let Err(error) = index_at_start(&services.control, &shards.shards()).await {
            tracing::error!(
                error = &error as &dyn std::error::Error,
                "the conversations to index for search cannot be listed"
            );
        }
        let runners = services.runner_releases.root().map(|releases| {
            let releases = releases.to_owned();
            let services = services.clone();
            AbortOnDropHandle::new(tokio::spawn(async move {
                // The handle's drop aborts the task, which drops its need.
                let cancel = tokio_util::sync::CancellationToken::new();
                services
                    .native
                    .source_paired_runners(&releases, &services.control, &cancel)
                    .await;
            }))
        });
        services.native.foresee(services.cloud.target());
        let ahead = {
            let services = services.clone();
            AbortOnDropHandle::new(tokio::spawn(async move {
                // The handle's drop aborts the task, which drops its need.
                let cancel = tokio_util::sync::CancellationToken::new();
                let native = &services.native;
                tokio::join!(native.foresee_paired(&services.control), native.store_ahead(&cancel));
            }))
        };
        let state = AppState {
            services: services.clone(),
            shards: shards.shards(),
            site: Arc::new(Site {
                public_url: config.public_url,
                origin_dropped: AtomicBool::new(false),
            }),
            log: ServerLog::of(&config.data_dir),
        };
        let runner_socket = match config.runner_socket.as_deref().map(runner_socket).transpose() {
            Ok(socket) => socket.flatten(),
            Err(source) => {
                shards.close().await;
                services.close_providers().await;
                return Err(StartError::RunnerSocket {
                    path: config.runner_socket.unwrap_or_default(),
                    source,
                });
            }
        };
        let manage_socket = match config.manage_socket.as_deref().map(manage_socket).transpose() {
            Ok(socket) => socket.flatten(),
            Err(source) => {
                shards.close().await;
                services.close_providers().await;
                return Err(StartError::ManageSocket {
                    path: config.manage_socket.unwrap_or_default(),
                    source,
                });
            }
        };
        let edge = match Edge::start(
            config.address,
            runner_socket,
            manage_socket,
            state,
            config.web_directory,
        )
        .await
        {
            Ok(edge) => edge,
            Err(source) => {
                shards.close().await;
                services.close_providers().await;
                return Err(StartError::Listen {
                    address: config.address,
                    source,
                });
            }
        };
        // A Cloud that does not run although commands ran on it, as after an
        // upgrade, starts again once a runner can reach the backend.
        let taken: Vec<_> = running.iter().map(|device| device.id.clone()).collect();
        if let Err(error) = take_up_stopped_clouds(&taken, &services.control, &shards.shards()).await {
            tracing::error!("the stopped Clouds' commands cannot be taken up: {error}");
        }
        Ok(Self {
            local_addr: edge.local_addr(),
            storage,
            services,
            shards,
            edge,
            deaths,
            resets,
            usage,
            runners,
            ahead,
            #[cfg(feature = "testing")]
            held_migrations: std::sync::Mutex::new(None),
        })
    }

    /// The address the listener is bound to.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Holds every commit of a conversation's checkpoint or stored
    /// attachment from now on, until the hold is released.
    #[cfg(feature = "testing")]
    pub fn hold_commits(&self) -> demi_backend_database::conversations::CommitHold {
        self.services.conversations.hold_commits()
    }

    /// How much work the users' search indexes have done so far.
    #[cfg(feature = "testing")]
    pub fn search_work(&self) -> demi_backend_database::search::SearchWork {
        self.services.search.work()
    }

    /// Overwrites the user's search index with bytes that do not open, in
    /// the index's own turn for the user.
    #[cfg(feature = "testing")]
    pub async fn corrupt_search_index(&self, user: &demi_web_api_protocol::ids::UserId) {
        self.services.search.corrupt(user).await.expect("the search index is overwritten");
    }

    /// The hold on every migration of a conversation's database that the
    /// backend started with (`BackendConfig::migrations_held`), until it is
    /// released.
    ///
    /// # Panics
    ///
    /// Panics if the backend started without one, or it was taken already.
    #[cfg(feature = "testing")]
    pub fn held_migrations(&self) -> demi_backend_database::conversations::MigrationHold {
        self.held_migrations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .expect("the backend started with its migrations held")
    }

    /// Holds every runner's hello at `step` from now on, until the hold is
    /// released.
    #[cfg(feature = "testing")]
    pub fn hold_hellos(
        &self,
        step: demi_backend_user_shard::holds::HelloStep,
    ) -> demi_backend_user_shard::holds::StepHold {
        self.services.hellos.hold(step)
    }

    /// Holds every failed turn's resume at its retry time at `step` from now
    /// on, until the hold is released.
    #[cfg(feature = "testing")]
    pub fn hold_retries(
        &self,
        step: demi_backend_user_shard::holds::RetryStep,
    ) -> demi_backend_user_shard::holds::StepHold {
        self.services.retries.hold(step)
    }

    /// Holds every change watch's check at `step` from now on, until the
    /// hold is released.
    #[cfg(feature = "testing")]
    pub fn hold_watches(
        &self,
        step: demi_backend_user_shard::holds::WatchStep,
    ) -> demi_backend_user_shard::holds::StepHold {
        self.services.watches.hold(step)
    }

    /// Holds every call's permission check at `step` from now on, until the
    /// hold is released.
    #[cfg(feature = "testing")]
    pub fn hold_permissions(
        &self,
        step: demi_backend_user_shard::holds::PermissionStep,
    ) -> demi_backend_user_shard::holds::StepHold {
        self.services.permissions.hold(step)
    }

    /// Downloads the media catalog now, as its daily download does.
    #[cfg(feature = "testing")]
    pub async fn refresh_media_catalog(&self) {
        self.services.media.refresh().await;
    }

    /// Holds every page's synchronization channel at `step` from now on,
    /// until the hold is released.
    #[cfg(feature = "testing")]
    pub fn hold_sync(
        &self,
        step: demi_backend_user_shard::sync::SyncStep,
    ) -> demi_backend_user_shard::holds::StepHold {
        self.services.syncs.hold(step)
    }

    /// Waits until `user`'s login `login` has ended, as the vault announces
    /// it, whatever a page's channel is held at.
    #[cfg(feature = "testing")]
    pub async fn until_login_ended(
        &self,
        user: &demi_web_api_protocol::ids::UserId,
        login: &demi_web_api_protocol::ids::LoginId,
    ) {
        use demi_web_api_protocol::providers::LoginState;
        let marks = self.services.sync.watch(user);
        while matches!(
            self.services.logins.state(login, user),
            Some(LoginState::Pending { .. })
        ) {
            marks.marked().await;
            marks.take();
        }
    }

    /// The file gate of the user's conversation `conversation`: every
    /// operation on the conversation's Host holds a lease of it, a
    /// transition reserves it, and a lease the caller takes is activity
    /// (`crates/backend-host-access/src/access.rs`).
    #[cfg(feature = "testing")]
    pub async fn file_gate(
        &self,
        user: &demi_web_api_protocol::ids::UserId,
        conversation: &demi_web_api_protocol::ids::ConversationId,
    ) -> demi_shared_gates::ActivityGate {
        let conversation = conversation.clone();
        self.shards
            .shards()
            .of(user)
            .call(move |shard, _| async move {
                shard
                    .conversations()
                    .slot(&conversation)
                    .file_gate()
                    .clone()
            })
            .await
            .expect("the user's shard serves while the backend runs")
    }

    /// How many operations of `user` wait for a returning device's runner
    /// now (`crates/backend-runners/src/devices.rs`).
    #[cfg(feature = "testing")]
    pub async fn return_waits(&self, user: &demi_web_api_protocol::ids::UserId) -> tokio::sync::watch::Receiver<usize> {
        self.shards
            .shards()
            .of(user)
            .call(move |shard, _| async move { demi_backend_host_access::HostShard::devices(&*shard).return_waits() })
            .await
            .expect("the user's shard serves while the backend runs")
    }

    /// What `user`'s device `device` is subscribed to on its runner now, by
    /// every holder (`crates/runner-host/src/subscriptions.rs`).
    #[cfg(feature = "testing")]
    pub async fn subscribed(
        &self,
        user: &demi_web_api_protocol::ids::UserId,
        device: &demi_web_api_protocol::ids::DeviceId,
    ) -> Vec<demi_runner_protocol::files::Subject> {
        let device = device.clone();
        self.shards
            .shards()
            .of(user)
            .call(move |shard, _| async move {
                let Some(link) = demi_backend_host_access::HostShard::devices(&*shard).link(&device) else {
                    return Vec::new();
                };
                link.subscribed()
            })
            .await
            .expect("the user's shard serves while the backend runs")
    }

    /// Resolves once no collection of `user`'s blob namespace runs or is to
    /// follow (`crates/backend-database/src/deletions.rs`).
    #[cfg(feature = "testing")]
    pub async fn until_collected(&self, user: &demi_web_api_protocol::ids::UserId) {
        self.shards
            .shards()
            .of(user)
            .call(|shard, _| async move { shard.until_collected().await })
            .await
            .expect("the user's shard serves while the backend runs");
    }

    /// Resolves once `count` Claude Code processes began to be prepared
    /// ahead on `user`'s Cloud (`crates/provider-claude-code/src/prepared.rs`).
    #[cfg(feature = "testing")]
    pub async fn until_prepared(&self, user: &demi_web_api_protocol::ids::UserId, count: u64) {
        self.shards
            .shards()
            .of(user)
            .call(move |shard, _| async move { shard.until_prepared(count).await })
            .await
            .expect("the user's shard serves while the backend runs");
    }

    /// Stores `secret`, a secret document of `family`, as the one account of
    /// a new entry of `owner`'s labelled `label`, as a sign-in would store
    /// it, for a suite whose vendor sign-in cannot run. The account is known
    /// by `identity`.
    #[cfg(feature = "testing")]
    pub async fn seed_subscription(
        &self,
        owner: &demi_web_api_protocol::ids::UserId,
        family: &str,
        label: &str,
        identity: &str,
        secret: String,
    ) -> demi_web_api_protocol::ids::ProviderId {
        use demi_backend_providers::vault::entries::SeededAccount;
        use demi_provider_common::credentials::{AccountLabel, NewAccount};
        let seeded = SeededAccount {
            family: family.to_owned(),
            label: label.to_owned(),
            account: NewAccount {
                secret: Some(secret),
                label: AccountLabel {
                    label: label.to_owned(),
                    detail: None,
                    identity_key: Some(identity.to_owned()),
                },
            },
            source: "login:code".into(),
        };
        self.services
            .vault
            .seed(owner.clone(), &seeded, self.services.clock.now())
            .await
            .expect("the vault stores the entry")
            .expect("the owner has no entry of the family")
            .id
    }

    /// Shuts the backend down. The listener closes first, so no new work
    /// starts and a new request on an open connection answers 503
    /// `backend_closing`; every step runs even when an earlier one fails, and
    /// the failures are reported together.
    pub async fn close(self) -> Result<(), ShutdownErrors> {
        let mut failures = Vec::new();
        self.edge.stop_accepting();
        self.services.logins.close().await;
        self.shards.close().await;
        self.services.claims.close();
        // No shard is left to route a death to.
        drop(self.deaths);
        drop(self.resets);
        drop(self.usage);
        // A runner or package executable still being sourced is no longer
        // needed.
        drop(self.runners);
        drop(self.ahead);
        self.services.cloud.machines.close().await;
        if let Err(error) = self.edge.close().await {
            failures.push(ShutdownError::Edge(error));
        }
        self.services.assembly.close().await;
        // A refresh that asked its vendor stores the answer before the
        // database closes (`crates/provider-common/src/credentials/refresh.rs`).
        self.services.vault.gates().close().await;
        failures.extend(
            self.storage
                .close()
                .await
                .into_iter()
                .map(ShutdownError::Storage),
        );
        if failures.is_empty() {
            Ok(())
        } else {
            Err(ShutdownErrors(failures))
        }
    }
}
