//! The backend's configuration (`backend.md` § Configuration).

use std::net::{Ipv4Addr, SocketAddr};
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use demi_core::{Clock, SystemClock};
use demi_web_api::settings::InstanceMode;
use url::Url;

use demi_provider::models_dev::ModelsDevClient;

use crate::auth::email_change::AccountMail;
use crate::llm::families::FamilyRegistry;
use crate::shard::ShardPlacement;
use crate::vault::logins::LoginTiming;
use crate::vault::secret::InstanceSecret;

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
    #[arg(long, env = "DEMI_BACKEND_PUBLIC_URL", value_name = "DEMI_BACKEND_PUBLIC_URL")]
    pub public_url: Url,
    /// The machine manager's Unix socket
    #[arg(long, env = "DEMI_MACHINES_SOCKET", value_name = "DEMI_MACHINES_SOCKET")]
    pub machines_socket: PathBuf,
    /// The native command releases and the object storage they are published to
    #[arg(long, env = "DEMI_NATIVE_CONFIG", value_name = "DEMI_NATIVE_CONFIG")]
    pub native_config: PathBuf,
    /// A JSON file that puts the object store in an S3 bucket
    #[arg(long, env = "DEMI_CHANGE_STORE_CONFIG", value_name = "DEMI_CHANGE_STORE_CONFIG")]
    pub change_store_config: Option<PathBuf>,
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
    pub expose_domain: Option<String>,
    /// A built browser directory to serve beside the API
    #[arg(long, env = "DEMI_WEB_DIRECTORY", value_name = "DEMI_WEB_DIRECTORY")]
    pub web_directory: Option<PathBuf>,
    /// The runner releases the installer routes serve
    #[arg(long, env = "DEMI_RUNNER_RELEASE_DIR", value_name = "DEMI_RUNNER_RELEASE_DIR")]
    pub runner_release_dir: Option<PathBuf>,
}

/// A configuration value clap cannot check by itself.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("DEMI_BACKEND_DATA is not set and the home directory is unknown")]
    NoDataDirectory,
    /// The value itself is secret, so the error leaves it out.
    #[error("DEMI_INSTANCE_SECRET must be 64 hexadecimal digits")]
    InstanceSecret,
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
            .map(|text| text.parse::<InstanceSecret>().map_err(|_| ConfigError::InstanceSecret))
            .transpose()?;
        let mut config = BackendConfig::new(
            data_dir,
            SocketAddr::from((Ipv4Addr::UNSPECIFIED, self.port)),
            self.mode,
        );
        config.instance_secret = instance_secret;
        config.web_directory = self.web_directory.clone();
        Ok(config)
    }
}

/// Everything `Backend::start` needs: the configured values, and the parts a
/// test replaces.
pub struct BackendConfig {
    /// The data directory (`storage.md` § Ownership and layout).
    pub data_dir: PathBuf,
    /// Where the listener binds; port 0 picks a free port.
    pub address: SocketAddr,
    /// Who configures providers (`product.md` § Instance mode).
    pub mode: InstanceMode,
    /// A built browser directory served beside the API.
    pub web_directory: Option<PathBuf>,
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
    /// Where the models.dev document is read.
    pub models_dev_url: Url,
    /// How long a device login waits for its user, and how long its result
    /// is kept.
    pub logins: LoginTiming,
    /// How runner connections are timed and pairing is limited.
    pub runners: RunnerTuning,
    /// How conversations are served and their inference limited.
    pub conversations: ConversationTuning,
}

/// How the backend serves conversations (`runtime.md` § Order and delivery,
/// `usage-and-quota.md` § Rate limit). Tests lower the bounds.
#[derive(Debug, Clone, Copy)]
pub struct ConversationTuning {
    /// The most frames a conversation socket's outbox holds; a client that
    /// falls this far behind is disconnected as lagging.
    pub outbox_frames: usize,
    /// The provider requests a user's conversations may start in any minute.
    pub requests_per_minute: usize,
}

impl Default for ConversationTuning {
    fn default() -> Self {
        Self {
            outbox_frames: demi_agent::ServerConfig::default().outbox_frames,
            requests_per_minute: crate::usage::rate_limit::REQUESTS_PER_WINDOW,
        }
    }
}

/// How the backend treats runner connections (`runner.md` § Connection and
/// identity, `backend.md` § Authentication and ownership). Tests shorten
/// the times.
#[derive(Debug, Clone, Copy)]
pub struct RunnerTuning {
    /// A connection that sends no hello within this is closed.
    pub hello_deadline: Duration,
    /// How long a pairing code lives before its waiting runner gets a new one.
    pub claim_lifetime: Duration,
    /// How many pairing codes one user may try within a minute.
    pub claims_per_minute: usize,
    /// How often a connected runner is asked whether it is there; none turns
    /// liveness off.
    pub ping: Option<Duration>,
}

impl Default for RunnerTuning {
    fn default() -> Self {
        Self {
            hello_deadline: Duration::from_secs(30),
            claim_lifetime: Duration::from_secs(10 * 60),
            claims_per_minute: 10,
            ping: Some(demi_host_remote::PING_INTERVAL),
        }
    }
}

impl BackendConfig {
    /// A configuration on the system clock with one shard thread, the
    /// built-in families, the published models.dev document, no web
    /// directory and no mail sender.
    pub fn new(data_dir: PathBuf, address: SocketAddr, mode: InstanceMode) -> Self {
        Self {
            data_dir,
            address,
            mode,
            web_directory: None,
            instance_secret: None,
            account_mail: None,
            clock: Arc::new(SystemClock),
            shards: ShardPlacement::Threads(NonZeroUsize::MIN),
            families: FamilyRegistry::builtin(),
            models_dev_url: ModelsDevClient::DEFAULT_URL.parse().expect("the models.dev address parses"),
            logins: LoginTiming::default(),
            runners: RunnerTuning::default(),
            conversations: ConversationTuning::default(),
        }
    }
}
