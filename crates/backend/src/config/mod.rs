//! The backend's configuration (`backend.md` § Configuration), and the
//! instance secret with the keys derived from it.

pub(crate) mod secret;

use std::net::{Ipv4Addr, SocketAddr};
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;

use demi_shared_types::{Clock, SystemClock};
use demi_web_api_protocol::settings::InstanceMode;
use tracing_subscriber::filter::Targets;
use url::Url;

use demi_backend_accounts::email_change::AccountMail;
use demi_backend_cloud::tuning::CloudTuning;
use demi_backend_expose::domain::ExposeDomain;
use demi_backend_providers::llm::claude_releases::DEFAULT_RELEASES_URL;
use demi_backend_providers::llm::families::FamilyRegistry;
use demi_backend_providers::vault::logins::LoginTiming;
use demi_backend_runners::native::NativeCatalog;
use demi_plugin_interface::PluginFactory;
use demi_provider_common::models_dev::ModelsDevClient;

use demi_backend_user_shard::shard::ShardPlacement;
use demi_backend_user_shard::tuning::{
    ConversationTuning, ExposeTuning, LifecycleTuning, PageTuning, RunnerTuning,
};

use self::secret::InstanceSecret;

/// The configuration as flags or `DEMI_*` variables. Each value's name in
/// `--help` and in every error is its variable, so an unusable value stops
/// startup naming the variable.
#[derive(clap::Parser)]
#[command(name = "demi-backend", version, about = "The Demi product server")]
pub struct Config {
    /// The data directory [default: ~/.demi/backend]
    #[arg(long, env = "DEMI_BACKEND_DATA", value_name = "DEMI_BACKEND_DATA")]
    pub data: Option<PathBuf>,
    /// The TCP port the backend listens on, 1 to 65535
    #[arg(
        long,
        env = "DEMI_BACKEND_PORT",
        value_name = "DEMI_BACKEND_PORT",
        default_value_t = 3271,
        value_parser = clap::value_parser!(u16).range(1..)
    )]
    pub port: u16,
    /// `shared` or `isolated`: who configures providers
    #[arg(long, env = "DEMI_INSTANCE_MODE", value_name = "DEMI_INSTANCE_MODE")]
    pub mode: InstanceMode,
    /// The URL runners and Cloud guests connect to
    #[arg(
        long,
        env = "DEMI_BACKEND_PUBLIC_URL",
        value_name = "DEMI_BACKEND_PUBLIC_URL"
    )]
    pub public_url: Url,
    /// The machine manager's Unix socket
    #[arg(
        long,
        env = "DEMI_MACHINE_MANAGER_SOCKET",
        value_name = "DEMI_MACHINE_MANAGER_SOCKET"
    )]
    pub machines_socket: PathBuf,
    /// The native command releases and the object storage they are published to
    #[arg(long, env = "DEMI_NATIVE_CONFIG", value_name = "DEMI_NATIVE_CONFIG")]
    pub native_config: PathBuf,
    /// A JSON file that puts the object store in an S3 bucket
    #[arg(
        long,
        env = "DEMI_OBJECT_STORE_CONFIG",
        value_name = "DEMI_OBJECT_STORE_CONFIG"
    )]
    pub object_store_config: Option<PathBuf>,
    /// The instance secret as 64 hexadecimal digits [default: generated into the data directory]
    #[arg(
        long,
        env = "DEMI_INSTANCE_SECRET",
        value_name = "DEMI_INSTANCE_SECRET",
        hide_env_values = true
    )]
    pub instance_secret: Option<String>,
    /// The domain of expose hostnames; without it, exposes are unavailable
    #[arg(long, env = "DEMI_EXPOSE_DOMAIN", value_name = "DEMI_EXPOSE_DOMAIN")]
    pub expose_domain: Option<ExposeDomain>,
    /// The web app build's directory, to serve beside the API
    #[arg(long, env = "DEMI_WEB_DIRECTORY", value_name = "DEMI_WEB_DIRECTORY")]
    pub web_directory: Option<PathBuf>,
    /// The runner releases the installer routes serve
    #[arg(
        long,
        env = "DEMI_RUNNER_RELEASE_DIR",
        value_name = "DEMI_RUNNER_RELEASE_DIR"
    )]
    pub runner_release_dir: Option<PathBuf>,
    /// The Claude Code distribution whose newest release the CLI on each Cloud follows
    #[arg(
        long,
        env = "DEMI_CLAUDE_RELEASES_URL",
        value_name = "DEMI_CLAUDE_RELEASES_URL",
        default_value = DEFAULT_RELEASES_URL
    )]
    pub claude_releases_url: Url,
    /// What the backend logs: a level, and a level per target, comma-separated,
    /// such as `info,demi::provider::claude_code::wire=trace`
    #[arg(
        long,
        env = "DEMI_LOG",
        value_name = "DEMI_LOG",
        default_value = "info"
    )]
    pub log: Targets,
}

/// A configuration value clap cannot check by itself.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("DEMI_BACKEND_DATA is not set and the home directory is unknown")]
    NoDataDirectory,
    /// The value itself is secret, so the error leaves it out.
    #[error("DEMI_INSTANCE_SECRET must be 64 hexadecimal digits")]
    InstanceSecret,
    #[error(
        "DEMI_BACKEND_PUBLIC_URL must be an HTTP or HTTPS URL without a user, a password, a query or a fragment"
    )]
    PublicUrl,
}

impl Config {
    /// What `Backend::start` takes, for this configuration on the system
    /// clock and one shard thread.
    pub fn backend(&self) -> Result<BackendConfig, ConfigError> {
        let data_dir = match &self.data {
            Some(data) => data.clone(),
            None => std::env::home_dir()
                .ok_or(ConfigError::NoDataDirectory)?
                .join(".demi")
                .join("backend"),
        };
        let instance_secret = self
            .instance_secret
            .as_deref()
            .map(|text| {
                text.parse::<InstanceSecret>()
                    .map_err(|_| ConfigError::InstanceSecret)
            })
            .transpose()?;
        let mut config = BackendConfig::new(
            data_dir,
            SocketAddr::from((Ipv4Addr::UNSPECIFIED, self.port)),
            self.mode,
            self.machines_socket.clone(),
        );
        config.instance_secret = instance_secret;
        config.web_directory = self.web_directory.clone();
        config.expose_domain = self.expose_domain.clone();
        let public_url = demi_backend_runners::install::backend_url(&self.public_url)
            .map_err(|_| ConfigError::PublicUrl)?;
        config.public_url = Some(public_url);
        config.runner_releases = self.runner_release_dir.clone();
        config.object_store = self.object_store_config.clone();
        config.claude_releases = self.claude_releases_url.clone();
        Ok(config)
    }
}

/// Everything `Backend::start` needs: the configured values, and the parts a
/// test replaces.
pub struct BackendConfig {
    /// The data directory (`storage.md` § Ownership and layout).
    pub data_dir: PathBuf,
    /// The machine manager's Unix socket (`managed-hosts.md` § Control and
    /// ownership): every deployment has Cloud.
    pub machines_socket: PathBuf,
    /// Where the listener binds; port 0 picks a free port.
    pub address: SocketAddr,
    /// Who configures providers (`product.md` § Instance mode).
    pub mode: InstanceMode,
    /// The web app build's directory, served beside the API.
    pub web_directory: Option<PathBuf>,
    /// The URL runners connect to, which the installers name; without it,
    /// the origin an installer was requested from.
    pub public_url: Option<Url>,
    /// The domain expose hostnames live under; without it, exposes are
    /// unavailable (`expose.md` § Deployment).
    pub expose_domain: Option<ExposeDomain>,
    /// The runner releases the installer routes serve; without them, the
    /// installers answer 503.
    pub runner_releases: Option<PathBuf>,
    /// The JSON file that puts the object store in an S3 bucket; without it,
    /// the data directory holds it.
    pub object_store: Option<PathBuf>,
    /// The instance secret; without it, the one in the data directory, which
    /// the first start creates.
    pub instance_secret: Option<InstanceSecret>,
    /// Delivers email verification codes; without it, an email change answers
    /// `mail_unavailable`.
    pub account_mail: Option<Arc<dyn AccountMail>>,
    pub clock: Arc<dyn Clock>,
    pub shards: ShardPlacement,
    /// The provider families entries are assembled with.
    pub families: FamilyRegistry,
    /// The plugins, in their order of registration.
    pub plugins: Vec<Box<dyn PluginFactory>>,
    /// Where the models.dev document is read.
    pub models_dev_url: Url,
    /// The Claude Code distribution whose newest release the CLI on each
    /// Cloud follows (`claude-code.md` § Which version).
    pub claude_releases: Url,
    /// How long a device login waits for its user, and how long its result
    /// is kept.
    pub logins: LoginTiming,
    /// How runner connections are timed and pairing is limited.
    pub runners: RunnerTuning,
    /// How conversations are served and their inference limited.
    pub conversations: ConversationTuning,
    /// How the sockets to a page are timed.
    pub pages: PageTuning,
    /// The command packages the conversations' commands bind to.
    pub native: NativeCatalog,
    /// When a conversation's Host resources are reclaimed.
    pub lifecycle: LifecycleTuning,
    /// How the Cloud is run.
    pub cloud: CloudTuning,
    /// How the public relay treats its connections.
    pub exposes: ExposeTuning,
    /// Counts what reaches the object store, for the scenarios that prove
    /// what the backend reads and writes there.
    #[cfg(feature = "testing")]
    pub object_counts: Option<demi_backend_blobs::counting::ObjectCounts>,
}

impl BackendConfig {
    /// A configuration on the system clock with one shard thread, the
    /// built-in families, the published models.dev document, no web
    /// directory and no mail sender, whose Clouds the machine manager at
    /// `machines_socket` runs.
    pub fn new(
        data_dir: PathBuf,
        address: SocketAddr,
        mode: InstanceMode,
        machines_socket: PathBuf,
    ) -> Self {
        Self {
            data_dir,
            machines_socket,
            address,
            mode,
            web_directory: None,
            public_url: None,
            expose_domain: None,
            runner_releases: None,
            object_store: None,
            instance_secret: None,
            account_mail: None,
            clock: Arc::new(SystemClock),
            shards: ShardPlacement::Threads(NonZeroUsize::MIN),
            families: crate::families::builtin(),
            plugins: crate::plugins::builtin(),
            models_dev_url: ModelsDevClient::DEFAULT_URL
                .parse()
                .expect("the models.dev address parses"),
            claude_releases: DEFAULT_RELEASES_URL
                .parse()
                .expect("the Claude Code distribution's address parses"),
            logins: LoginTiming::default(),
            runners: RunnerTuning::default(),
            conversations: ConversationTuning::default(),
            pages: PageTuning::default(),
            // The product's start publishes the releases `DEMI_NATIVE_CONFIG`
            // names and sets the catalog of them.
            native: NativeCatalog::unpublished(),
            lifecycle: LifecycleTuning::default(),
            cloud: CloudTuning::default(),
            exposes: ExposeTuning::default(),
            #[cfg(feature = "testing")]
            object_counts: None,
        }
    }
}
