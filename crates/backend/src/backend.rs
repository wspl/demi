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
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use demi_backend_blobs::ObjectError;
use demi_backend_blobs::store as objects;
use demi_backend_runners::install::{RunnerReleases, read_runner_release};
use demi_backend_runners::native::NativeCatalog;
use demi_runner_protocol::release::RunnerRelease;
use demi_backend_runners::publication::{PublicationError, publish_native};
use demi_backend_cloud::CloudServices;
use demi_backend_cloud::client::MachinesClient;
use demi_backend_cloud::reset::recover_clouds;
use demi_backend_database::StorageError;
use demi_backend_database::update::Update;
use demi_backend_http::{AppState, Edge, Listening, Site, WebBuildError, manage_socket, runner_socket, web_build};
use demi_backend_user_shard::conversation::cost::follow_usage;
use demi_backend_user_shard::conversation::search::{follow_resets, index_at_start};
use demi_backend_user_shard::conversation::{
    arm_idle_compactions, finish_deletions, recover_forks, resume_after_restart,
};
use demi_backend_user_shard::shard::deliver_decisions;
use demi_backend_user_shard::services::{
    CloseError, ProviderSetup, ServiceKeys, ServiceSettings, Services, ServicesError, Storage,
};
use demi_backend_user_shard::shard::ShardPool;
use demi_backend_user_shard::shard::cloud::{route_deaths, route_statuses};
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
    /// Settles the Clouds an earlier backend left, once the machine manager
    /// answers.
    recovery: AbortOnDropHandle<()>,
    /// Routes the machine manager's death events to their owners' shards.
    deaths: AbortOnDropHandle<()>,
    /// Marks each device's Cloud status changed for its owner's pages.
    statuses: AbortOnDropHandle<()>,
    /// Has the users whose search index is reset index their conversations
    /// again (`crates/backend-database/src/search.rs`).
    resets: AbortOnDropHandle<()>,
    /// Announces each ledger row to its conversation's pages.
    usage: AbortOnDropHandle<()>,
    /// Logs the backend's resources every minute (`crate::sampling`).
    sampling: AbortOnDropHandle<()>,
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

/// Runs the start step `step`, whose failure does not stop the start: its end
/// is logged with the time it took, and a failure with its error.
async fn start_step<T, E: fmt::Display>(step: &'static str, work: impl std::future::Future<Output = Result<T, E>>) {
    let started = std::time::Instant::now();
    let done = work.await;
    let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    match done {
        Ok(_) => tracing::info!(event = "startup.step_ended", step, ms, outcome = "ok", "a start step ended"),
        Err(error) => tracing::error!(event = "startup.step_failed", step, ms, %error, "a start step failed; the start goes on"),
    }
}

/// How long the recovery waits before it asks an unreachable machine manager
/// again.
const RECOVERY_RETRY: std::time::Duration = std::time::Duration::from_secs(5);

/// Settles the Clouds an earlier backend left, in the background so that the
/// backend serves meanwhile: the machine manager keeps every running Cloud
/// and finishes the resets it left unfinished, each running Cloud's shard
/// takes it over, and the trees of a Cloud that does not run although commands
/// ran on it, as after an upgrade, are restored with their commands as last
/// recorded, which the Cloud's next hello settles. A
/// manager that cannot be reached is asked again until it answers; any other
/// failure ends the recovery and is logged, as a failed start step is.
async fn recover_in_background(
    services: Arc<demi_backend_user_shard::services::Services>,
    shards: demi_backend_user_shard::shard::Shards,
) {
    let running = loop {
        match recover_clouds(&services.control, &services.cloud).await {
            Ok(running) => break running,
            Err(demi_backend_cloud::reset::RecoveryError::Machines(
                error @ demi_backend_cloud::client::MachinesError::Unavailable { .. },
            )) => {
                tracing::warn!(event = "startup.cloud_recovery_waiting", %error, "the machine manager is unreachable; the Clouds are recovered once it answers");
                tokio::time::sleep(RECOVERY_RETRY).await;
            }
            Err(error) => {
                tracing::error!(event = "startup.step_failed", step = "cloud_recovery", %error, "the Clouds cannot be recovered");
                return;
            }
        }
    };
    start_step("cloud_take_over", take_over_clouds(running.clone(), &shards)).await;
    // A Cloud the manager does not run has no runner to wait for.
    start_step("stopped_clouds", forget_stopped_clouds(&running, &services)).await;
    let taken: Vec<_> = running.iter().map(|device| device.id.clone()).collect();
    start_step("cloud_commands", take_up_stopped_clouds(&taken, &services.control, &shards)).await;
}

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
/// `release`: its `commands/`, whose executables come from its `files/`,
/// into the object store `config` names, before the
/// backend that serves them starts (`crates/backend-runners/src/publication.rs`); `cancel` interrupts it.
pub async fn publish_commands(
    config: &BackendConfig,
    release: &std::path::Path,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<NativeCatalog, StartError> {
    let files = release.join(demi_runner_protocol::release::FILES);
    let objects = objects::open(&config.data_dir, &config.storage).await?;
    let commands = release.join("commands");
    let runners = read_runner_release(&release.join("runners/manifest.json"), RunnerRelease::decode)
        .await
        .map_err(StartError::RunnerRelease)?;
    Ok(publish_native(
        &commands,
        files,
        demi_shared_artifacts::BUILD_VERSION,
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
        // The listener answers before the data is ready: while a new release
        // migrates or converts it, every page shows how far it is.
        let update = Arc::new(Update::default());
        let listening = Edge::listen(
            config.address,
            config.public_url.as_ref(),
            config.web_directory.clone(),
            update.clone(),
        )
        .await
        .map_err(|source| StartError::Listen {
            address: config.address,
            source,
        })?;
        let opened = Self::open_storage(&config, &data_dir, &update).await;
        let (storage, secret) = match opened {
            Ok(opened) => opened,
            // The listener goes with the process, which a failed start ends.
            Err(error) => return Err(error),
        };
        let started = Self::serve(config, storage.clone(), &secret, listening, update).await;
        if started.is_err() {
            for failure in storage.close().await {
                tracing::error!(
                    event = "storage.close_failed",
                    after = "start_failed",
                    error = &failure as &dyn std::error::Error,
                    "storage did not close after a failed start"
                );
            }
        }
        started
    }

    /// Opens the data, migrating and converting what an earlier release left,
    /// as `update` tells.
    async fn open_storage(
        config: &BackendConfig,
        data_dir: &Path,
        update: &Arc<Update>,
    ) -> Result<(Storage, InstanceSecret), StartError> {
        let due = {
            let data_dir = data_dir.to_owned();
            tokio::task::spawn_blocking(move || Storage::due(&data_dir))
                .await
                .map_err(|error| StartError::Storage(StorageError::from(error)))??
        };
        if let Some(due) = due {
            tracing::info!(event = "update.started", bytes = due.all, "the backend updates its data before it serves");
            update.begin(due);
        }
        let secret = match &config.instance_secret {
            Some(secret) => secret.clone(),
            None => InstanceSecret::load_or_create(data_dir).await?,
        };
        // The object store: the data directory or the S3 bucket.
        let objects = objects::open(data_dir, &config.storage).await?.store;
        let storage = Storage::open(data_dir, config.clock.clone(), objects, secret.vault_key()).await?;
        if due.is_some() {
            update.control_finished();
            let started = std::time::Instant::now();
            storage.conversations.prepare(update.clone()).await;
            tracing::info!(
                event = "update.ended",
                ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                "the backend's data is up to date"
            );
        }
        Ok((storage, secret))
    }

    async fn serve(
        config: BackendConfig,
        storage: Storage,
        secret: &InstanceSecret,
        listening: Listening,
        update: Arc<Update>,
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
        let (machines, notices) = MachinesClient::new(config.machines_socket, config.log.clone());
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
            log: config.log,
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
            upgrade_request: config.upgrade_request,
            update: update.clone(),
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
            notices.deaths,
            services.clone(),
            shards.shards(),
        )));
        let statuses = AbortOnDropHandle::new(tokio::spawn(route_statuses(
            notices.statuses,
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
        start_step("pending_deletions", finish_deletions(&services.control, &shards.shards())).await;
        // A credential request's command ended with the run before, so no one waits
        // for its answer; and a credential whose time came while the backend was down
        // is removed.
        //
        // A failure does not stop the start: the user's shard removes what is past due
        // when it starts, and the next start withdraws the requests.
        start_step("credential_requests", services.control.withdraw_credential_requests()).await;
        start_step("expired_credentials", services.control.expire_all_user_credentials()).await;
        // Each permission decision whose message a restart cut off reaches
        // the agent that asked. A failure does not stop the start: the next
        // start delivers it.
        start_step("permission_decisions", deliver_decisions(&services.control, &shards.shards())).await;
        // Each root's turn the last shutdown interrupted goes on, in the
        // background, as the user's Resume would, unless the user turned that
        // off. A failure does not stop the start: the marks stay for the next
        // start, and meanwhile the turns wait for the user's Resume.
        start_step("interrupted_turns", resume_after_restart(&services.control, &shards.shards())).await;
        // Each user's idle conversations compact at the times their last
        // requests set, which a restart keeps. A failure does not stop the
        // start: the next answered request sets its conversation's time again.
        start_step("idle_compactions", arm_idle_compactions(&services.control, &shards.shards())).await;
        // Each user's schedules fire what a backend that was down missed
        // and wait for their next times. A failure does not stop the start: a
        // user's next call starts them.
        start_step(
            "schedule_users",
            demi_backend_user_shard::schedules::start_schedulers(&services.control, &shards.shards()),
        )
        .await;
        // Each user's search index catches up with the conversations, in the
        // background (`crates/backend-database/src/search.rs`). A failure
        // does not stop the start: a conversation not indexed here is
        // indexed by its next change.
        start_step("search_index", index_at_start(&services.control, &shards.shards())).await;
        let state = AppState {
            services: services.clone(),
            shards: shards.shards(),
            site: Arc::new(Site {
                public_url: config.public_url,
                origin_dropped: AtomicBool::new(false),
            }),
            update: update.clone(),
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
        update.ready();
        let edge = listening.open(runner_socket, manage_socket, state, config.web_directory);
        // The machine manager settles what an earlier backend left in the
        // background: no page waits for it, and a manager that starts late is
        // tried until it answers.
        let recovery = AbortOnDropHandle::new(tokio::spawn(recover_in_background(
            services.clone(),
            shards.shards(),
        )));
        let sampling = AbortOnDropHandle::new(tokio::spawn(crate::sampling::sample(
            shards.shards(),
            services.log.clone(),
        )));
        Ok(Self {
            local_addr: edge.local_addr(),
            storage,
            services,
            shards,
            edge,
            recovery,
            deaths,
            statuses,
            resets,
            usage,
            sampling,
        })
    }

    /// What the shards hold now: the live trees and the work they run.
    pub async fn census(&self) -> demi_backend_user_shard::shard::Census {
        self.shards.shards().census().await
    }

    /// The address the listener is bound to.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
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
        drop(self.recovery);
        drop(self.deaths);
        drop(self.statuses);
        drop(self.resets);
        drop(self.usage);
        drop(self.sampling);
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
