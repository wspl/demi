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
use demi_backend_blobs::store::{self as objects, S3Config, S3ConfigError};
use demi_backend_cloud::CloudServices;
use demi_backend_cloud::client::MachinesClient;
use demi_backend_cloud::reset::recover_resets;
use demi_backend_database::StorageError;
use demi_backend_http::{AppState, Edge, Site};
use demi_backend_user_shard::conversation::{rearm_wakeups, recover_forks};
use demi_backend_user_shard::lifecycle::retention;
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
    /// Runs the daily retention pass, when the configuration schedules it.
    retention: Option<AbortOnDropHandle<()>>,
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
    #[error("DEMI_OBJECT_STORE_CONFIG cannot be used: {0}")]
    ObjectStore(S3ConfigError),
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
        let s3 = match &config.object_store {
            Some(path) => Some(
                S3Config::read(path)
                    .await
                    .map_err(StartError::ObjectStore)?,
            ),
            None => None,
        };
        // The object store: the S3 bucket the configuration names, or the
        // data directory.
        let objects = objects::open(&data_dir, s3.as_ref()).await?;
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
        let settings = ServiceSettings {
            mode: config.mode,
            mail: config.account_mail,
            runners: config.runners,
            conversations: config.conversations,
            pages: config.pages,
            native: config.native,
            plugins: config.plugins,
            cloud: CloudServices::new(machines, config.cloud),
            lifecycle: config.lifecycle,
            expose_domain: config.expose_domain,
            exposes: config.exposes,
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
        // earlier backend left, which stops every Cloud and so ends their
        // exposes, and resets it left unfinished commit their disks.
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
        let state = AppState {
            services: services.clone(),
            shards: shards.shards(),
            site: Arc::new(Site {
                public_url: config.public_url,
                runner_releases: config.runner_releases,
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
        // The first pass reads the references once the recovery above has
        // published every Fork destination whose root committed.
        let retention = services.lifecycle.retention_interval.map(|interval| {
            AbortOnDropHandle::new(tokio::spawn(retention::schedule(
                services.clone(),
                shards.shards(),
                interval,
            )))
        });
        Ok(Self {
            local_addr: edge.local_addr(),
            storage,
            services,
            shards,
            edge,
            deaths,
            retention,
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

    /// A new expose of `address` on the user's `device` for an hour, as the
    /// `expose` plugin makes it: a scenario's way to an expose without a
    /// turn of the agent.
    #[cfg(feature = "testing")]
    pub async fn create_expose(
        &self,
        user: &demi_web_api_protocol::ids::UserId,
        device: &demi_web_api_protocol::ids::DeviceId,
        address: &str,
    ) -> Result<demi_backend_expose::records::Expose, demi_backend_expose::records::ExposeError>
    {
        let device = device.clone();
        let address = demi_web_api_protocol::exposes::ExposeAddress::try_from(address.to_owned())
            .expect("a scenario exposes a valid address");
        self.shards
            .shards()
            .of(user)
            .call(move |shard, _| async move {
                shard
                    .expose_shard()
                    .add_expose(&device, address, jiff::SignedDuration::from_hours(1))
                    .await
            })
            .await
            .expect("the user's shard serves while the backend runs")
    }

    /// Runs the user's retention pass at once (`storage.md` § The retention
    /// pass) and answers once it has ended.
    #[cfg(feature = "testing")]
    pub async fn run_retention(&self, user: &demi_web_api_protocol::ids::UserId) {
        self.shards
            .shards()
            .of(user)
            .call(|shard, _| async move { shard.retention_pass().await })
            .await
            .expect("the user's shard serves while the backend runs");
    }

    /// Shuts the backend down. The listener closes first, so no new work
    /// starts and a new request on an open connection answers 503
    /// `backend_closing`; every step runs even when an earlier one fails, and
    /// the failures are reported together.
    pub async fn close(self) -> Result<(), ShutdownErrors> {
        let mut failures = Vec::new();
        self.edge.stop_accepting();
        // No pass starts from now on; one under way ends with its shard.
        drop(self.retention);
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
