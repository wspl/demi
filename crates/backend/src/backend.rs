//! The composition root: `Backend::start` opens storage, starts the shared
//! services and the shard threads, and opens the listener;
//! `Backend::close` shuts them down in order (`backend.md` § Startup and
//! shutdown).

use std::fmt;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use demi_backend_blobs::ObjectError;
use demi_backend_blobs::store as objects;
use demi_backend_runners::install::RunnerReleases;
use demi_backend_runners::native::NativeCatalog;
use demi_backend_runners::publication::{PublicationError, publish_native, release_files};
use demi_backend_cloud::CloudServices;
use demi_backend_cloud::client::MachinesClient;
use demi_backend_cloud::reset::recover_resets;
use demi_backend_database::StorageError;
use demi_backend_http::{AppState, Edge, Site, WebBuildError, web_build};
use demi_backend_user_shard::conversation::search::index_at_start;
use demi_backend_user_shard::conversation::{
    finish_deletions, rearm_wakeups, recover_forks, settle_pending,
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
    /// Sources the runner executables of the paired devices' systems
    /// (`native-runtime.md` § Runner releases); a close ends it.
    runners: Option<AbortOnDropHandle<()>>,
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
    #[error(transparent)]
    Services(#[from] ServicesError),
    #[error("the shard threads cannot start: {0}")]
    Shards(io::Error),
    #[error("the backend cannot listen on {address}: {source}")]
    Listen {
        address: SocketAddr,
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
    /// A user's Cloud could not be saved; the manager's reconcile saves it.
    #[error("a Cloud was not saved: {0}")]
    Cloud(String),
    /// The machine manager did not reconcile at close.
    #[error("the machine manager did not reconcile: {0}")]
    Machines(String),
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

/// Publishes the command packages of the server release whose root is
/// `release`: its `commands/`, whose executables come from the files its
/// `release.json` names, into the object store `config` names, before the
/// backend that serves them starts (`native-runtime.md` § Publish packages,
/// then source artifacts on demand); `cancel` interrupts it.
pub async fn publish_commands(
    config: &BackendConfig,
    release: &std::path::Path,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<NativeCatalog, StartError> {
    let files = release_files(release).await?;
    let objects = objects::open(&config.data_dir, &config.storage).await?;
    let commands = release.join("commands");
    Ok(publish_native(&commands, files, &objects, cancel).await?)
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
        let storage = Storage::open(&data_dir, config.clock.clone(), objects).await?;
        let started = Self::serve(config, storage.clone(), &secret).await;
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
            claude_releases: config.claude_releases,
            logins: config.logins,
            clock: config.clock,
        };
        let (machines, deaths) = MachinesClient::new(config.machines_socket);
        let keys = ServiceKeys {
            vault: secret.vault_key(),
            email_codes: secret.email_code_key(),
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
            native: config.native,
            plugins: config.plugins,
            cloud: CloudServices::new(machines, config.cloud),
            lifecycle: config.lifecycle,
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
        // Before the backend serves, the machine manager settles what an
        // earlier backend left, which stops every Cloud, and resets it left
        // unfinished commit their disks.
        if let Err(error) = recover_resets(&services.control, &services.cloud).await {
            shards.close().await;
            services.close_providers().await;
            return Err(StartError::Cloud(error.to_string()));
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
        // Each move or detach an agent asked for that a restart cut off is
        // made, before a saved wakeup opens its tree
        // (`sessions-and-targets.md` § Switch the primary target). A failure
        // does not stop the start: the next start makes it.
        if let Err(error) = settle_pending(&services.control, &shards.shards()).await {
            tracing::error!(
                error = &error as &dyn std::error::Error,
                "the pending moves cannot be listed"
            );
        }
        // Each saved wakeup is armed again, so it fires with no page open
        // (`runtime.md` § Yield wakeups). A failure does not stop the start:
        // a wakeup not armed here still fires once a page opens its
        // conversation.
        if let Err(error) = rearm_wakeups(&services.control, &shards.shards()).await {
            tracing::error!(
                error = &error as &dyn std::error::Error,
                "the saved wakeups cannot be listed"
            );
        }
        // Each permission decision whose message a restart cut off reaches
        // the agent that asked (`permissions.md` § The decision's message).
        // A failure does not stop the start: the next start delivers it.
        if let Err(error) = deliver_decisions(&services.control, &shards.shards()).await {
            tracing::error!(
                error = &error as &dyn std::error::Error,
                "the undelivered permission decisions cannot be listed"
            );
        }
        // Each user's search index catches up with the conversations, in the
        // background (`storage.md` § Search index). A failure does not stop
        // the start: a conversation not indexed here is indexed by its next
        // change.
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
        let state = AppState {
            services: services.clone(),
            shards: shards.shards(),
            site: Arc::new(Site {
                public_url: config.public_url,
                origin_dropped: AtomicBool::new(false),
            }),
        };
        let edge = match Edge::start(config.address, state, config.web_directory).await {
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
        Ok(Self {
            local_addr: edge.local_addr(),
            storage,
            services,
            shards,
            edge,
            deaths,
            runners,
        })
    }

    /// The address the listener is bound to.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Holds every commit of a conversation's checkpoint from now on, until
    /// the hold is released.
    #[cfg(feature = "testing")]
    pub fn hold_commits(&self) -> demi_backend_database::conversations::CommitHold {
        self.services.conversations.hold_commits()
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

    /// Holds every page's synchronization channel at `step` from now on,
    /// until the hold is released.
    #[cfg(feature = "testing")]
    pub fn hold_sync(
        &self,
        step: demi_backend_user_shard::sync::SyncStep,
    ) -> demi_backend_user_shard::holds::StepHold {
        self.services.syncs.hold(step)
    }

    /// The file gate of the user's conversation `conversation`: every
    /// operation on the conversation's Host holds a lease of it, a
    /// transition reserves it, and a lease the caller takes is activity
    /// (`sessions-and-targets.md` § Host operations).
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

    /// Resolves once no collection of `user`'s blob namespace runs or is to
    /// follow (`storage.md` § Deleting a conversation).
    #[cfg(feature = "testing")]
    pub async fn until_collected(&self, user: &demi_web_api_protocol::ids::UserId) {
        self.shards
            .shards()
            .of(user)
            .call(|shard, _| async move { shard.until_collected().await })
            .await
            .expect("the user's shard serves while the backend runs");
    }

    /// Stores `secret`, a secret document of `family`, as the one account of
    /// a new entry of `owner`'s labelled `label`, as a sign-in would store
    /// it, for a suite whose vendor sign-in cannot run (`scenarios.md` §
    /// Claude Code suite). The account is known by `identity`.
    #[cfg(feature = "testing")]
    pub async fn seed_subscription(
        &self,
        owner: &demi_web_api_protocol::ids::UserId,
        family: &str,
        label: &str,
        identity: &str,
        secret: String,
    ) -> demi_web_api_protocol::ids::ProviderId {
        use demi_provider_common::credentials::{
            AccountMeta, CredentialPool as _, MemoryCredentialPool, credential_id_for,
        };
        let staged = MemoryCredentialPool::new();
        let id = credential_id_for(Some(identity), label);
        let meta = AccountMeta {
            id: id.clone(),
            label: label.to_owned(),
            detail: None,
            updated_at: self.services.clock.now(),
            source: "login:code".into(),
            identity_key: Some(identity.to_owned()),
        };
        staged.write(meta, secret).await.expect("a pool in memory stores");
        staged.set_active(&id).await.expect("a pool in memory selects");
        self.services
            .vault
            .create_subscription(owner.clone(), family.to_owned(), label.to_owned(), &staged)
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
        failures.extend(
            self.shards
                .close()
                .await
                .into_iter()
                .map(ShutdownError::Cloud),
        );
        self.services.claims.close();
        // No shard is left to route a death to.
        drop(self.deaths);
        // A runner executable still being sourced is no longer needed.
        drop(self.runners);
        if let Err(error) = self.services.cloud.machines.close().await {
            failures.push(ShutdownError::Machines(error.to_string()));
        }
        if let Err(error) = self.edge.close().await {
            failures.push(ShutdownError::Edge(error));
        }
        self.services.assembly.close().await;
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
