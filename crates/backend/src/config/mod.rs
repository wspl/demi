//! The backend's configuration (`backend.md` § Configuration), and the
//! instance secret with the keys derived from it.

pub(crate) mod secret;

use std::net::SocketAddr;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use demi_shared_types::{Clock, SystemClock};
use demi_web_api_protocol::settings::InstanceMode;
use tracing_subscriber::filter::Targets;
use url::Url;

use demi_backend_accounts::email_change::AccountMail;
use demi_backend_blobs::store::{S3Config, Storage};
use demi_backend_cloud::tuning::CloudTuning;
use demi_backend_providers::llm::claude_releases::DEFAULT_RELEASES_URL;
use demi_backend_providers::llm::families::FamilyRegistry;
use demi_backend_providers::vault::logins::LoginTiming;
use demi_backend_runners::native::NativeCatalog;
use demi_plugin_interface::PluginFactory;
use demi_provider_common::models_dev::ModelsDevClient;

use demi_backend_user_shard::preview::{PageOrigin, PreviewDomainName, PreviewSettings};
use demi_backend_user_shard::shard::ShardPlacement;
use demi_backend_user_shard::tuning::{
    ConversationTuning, LifecycleTuning, PageTuning, PreviewTuning, RunnerTuning,
};

use self::secret::InstanceSecret;

/// The configuration as flags or `DEMI_*` variables. Each value's name in
/// `--help` and in every error is its variable, so an unusable value stops
/// startup naming the variable.
#[derive(clap::Parser)]
#[command(name = "demi-backend", version, about = "The Demi product server")]
pub struct Config {
    /// Validate the configuration as a start would, then exit
    #[arg(long)]
    pub check_config: bool,
    /// The server release root [default: the directory above the one that
    /// holds this executable]
    #[arg(long, env = "DEMI_RELEASE", value_name = "DEMI_RELEASE")]
    pub release: Option<PathBuf>,
    /// The data directory [default: ~/.demi/backend]
    #[arg(long, env = "DEMI_BACKEND_DATA", value_name = "DEMI_BACKEND_DATA")]
    pub data: Option<PathBuf>,
    /// The address and port the backend listens on
    #[arg(
        long,
        env = "DEMI_BACKEND_LISTEN",
        value_name = "DEMI_BACKEND_LISTEN",
        default_value = "0.0.0.0:3271"
    )]
    pub listen: SocketAddr,
    /// `shared` or `isolated`: who configures providers
    #[arg(long, env = "DEMI_INSTANCE_MODE", value_name = "DEMI_INSTANCE_MODE")]
    pub mode: InstanceMode,
    /// The URL browsers, runners and Cloud guests reach the backend at
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
        value_name = "DEMI_MACHINE_MANAGER_SOCKET",
        default_value = "/run/demi-cloud/machines.sock"
    )]
    pub machines_socket: PathBuf,
    /// Where the one object store lives: `local`, the data directory, or `s3`
    #[arg(
        long,
        env = "DEMI_STORAGE",
        value_name = "DEMI_STORAGE",
        default_value = "local"
    )]
    pub storage: StorageKind,
    /// The S3 store's bucket
    #[arg(long, env = "DEMI_S3_BUCKET", value_name = "DEMI_S3_BUCKET")]
    pub s3_bucket: Option<String>,
    /// The S3 store's region
    #[arg(long, env = "DEMI_S3_REGION", value_name = "DEMI_S3_REGION")]
    pub s3_region: Option<String>,
    /// An HTTPS endpoint of an S3-compatible service [default: the region's AWS endpoint]
    #[arg(long, env = "DEMI_S3_ENDPOINT", value_name = "DEMI_S3_ENDPOINT")]
    pub s3_endpoint: Option<Url>,
    /// `true` names the bucket in the request path instead of the host name [default: false]
    #[arg(
        long,
        env = "DEMI_S3_FORCE_PATH_STYLE",
        value_name = "DEMI_S3_FORCE_PATH_STYLE",
        action = clap::ArgAction::Set
    )]
    pub s3_force_path_style: Option<bool>,
    /// The instance secret as 64 hexadecimal digits [default: generated into the data directory]
    #[arg(
        long,
        env = "DEMI_INSTANCE_SECRET",
        value_name = "DEMI_INSTANCE_SECRET",
        hide_env_values = true
    )]
    pub instance_secret: Option<String>,
    /// The Claude Code distribution whose newest release the CLI on each Cloud follows
    #[arg(
        long,
        env = "DEMI_CLAUDE_RELEASES_URL",
        value_name = "DEMI_CLAUDE_RELEASES_URL",
        default_value = DEFAULT_RELEASES_URL
    )]
    pub claude_releases_url: Url,
    /// The preview domain the backend registers its namespace with and the
    /// page embeds previews from; one under .localhost may carry a port
    #[arg(
        long,
        env = "DEMI_PREVIEW_DOMAIN",
        value_name = "DEMI_PREVIEW_DOMAIN",
        default_value = "demi-preview.dev"
    )]
    pub preview_domain: PreviewDomainName,
    /// Origins besides the public URL's that serve the web app, such as a
    /// development server's, comma-separated: the preview namespace admits
    /// them too
    #[arg(
        long,
        env = "DEMI_PREVIEW_ORIGINS",
        value_name = "DEMI_PREVIEW_ORIGINS",
        value_delimiter = ','
    )]
    pub preview_origins: Vec<PageOrigin>,
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

/// Where the one object store lives (`storage.md` § The object store).
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum StorageKind {
    Local,
    S3,
}

/// A configuration value clap cannot check by itself.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("DEMI_BACKEND_DATA is not set and the home directory is unknown")]
    NoDataDirectory,
    #[error("DEMI_RELEASE is not set and the executable's directory is unknown: {0}")]
    NoRelease(std::io::Error),
    #[error("DEMI_BACKEND_LISTEN must name a port, not 0")]
    ListenPort,
    /// The value itself is secret, so the error leaves it out.
    #[error("DEMI_INSTANCE_SECRET must be 64 hexadecimal digits")]
    InstanceSecret,
    #[error(
        "DEMI_BACKEND_PUBLIC_URL must be an HTTP or HTTPS URL without a user, a password, a query or a fragment"
    )]
    PublicUrl,
    #[error("DEMI_STORAGE=s3 needs {0}")]
    S3Missing(&'static str),
    #[error("{0} names an S3 bucket, and DEMI_STORAGE is local")]
    S3Unused(&'static str),
    #[error("{0}")]
    S3(#[from] demi_backend_blobs::store::S3ConfigError),
}

impl Config {
    /// The server release root (`builds-and-releases.md` § Server release):
    /// `DEMI_RELEASE`, or the directory above the one that holds this
    /// executable.
    pub fn release(&self) -> Result<PathBuf, ConfigError> {
        if let Some(release) = &self.release {
            return Ok(release.clone());
        }
        let executable = std::env::current_exe().map_err(ConfigError::NoRelease)?;
        executable
            .parent()
            .and_then(Path::parent)
            .map(Path::to_owned)
            .ok_or_else(|| {
                ConfigError::NoRelease(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "the executable lies in no directory of a release",
                ))
            })
    }

    /// The object store the `DEMI_STORAGE` and `DEMI_S3_*` settings name.
    fn storage(&self) -> Result<Storage, ConfigError> {
        let named = [
            ("DEMI_S3_BUCKET", self.s3_bucket.is_some()),
            ("DEMI_S3_REGION", self.s3_region.is_some()),
            ("DEMI_S3_ENDPOINT", self.s3_endpoint.is_some()),
            ("DEMI_S3_FORCE_PATH_STYLE", self.s3_force_path_style.is_some()),
        ];
        match self.storage {
            StorageKind::Local => match named.iter().find(|(_, set)| *set) {
                Some((name, _)) => Err(ConfigError::S3Unused(name)),
                None => Ok(Storage::Local),
            },
            StorageKind::S3 => {
                let config = S3Config {
                    bucket: self
                        .s3_bucket
                        .clone()
                        .ok_or(ConfigError::S3Missing("DEMI_S3_BUCKET"))?,
                    region: self
                        .s3_region
                        .clone()
                        .ok_or(ConfigError::S3Missing("DEMI_S3_REGION"))?,
                    endpoint: self.s3_endpoint.clone(),
                    force_path_style: self.s3_force_path_style.unwrap_or(false),
                };
                config.check()?;
                Ok(Storage::S3(config))
            }
        }
    }

    /// What `Backend::start` takes, for this configuration and its server
    /// release root `release`, on the system clock and one shard thread.
    pub fn backend(&self, release: &Path) -> Result<BackendConfig, ConfigError> {
        let data_dir = match &self.data {
            Some(data) => data.clone(),
            None => std::env::home_dir()
                .ok_or(ConfigError::NoDataDirectory)?
                .join(".demi")
                .join("backend"),
        };
        if self.listen.port() == 0 {
            return Err(ConfigError::ListenPort);
        }
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
            self.listen,
            self.mode,
            self.machines_socket.clone(),
        );
        config.instance_secret = instance_secret;
        config.storage = self.storage()?;
        // A root serves the parts it holds: a developer's has no web app,
        // which Vite serves instead.
        let web = release.join("web");
        config.web_directory = web.is_dir().then_some(web);
        let runners = release.join("runners");
        config.runner_releases = runners.is_dir().then_some(runners);
        let public_url = demi_backend_runners::install::backend_url(&self.public_url)
            .map_err(|_| ConfigError::PublicUrl)?;
        config.public_url = Some(public_url);
        config.claude_releases = self.claude_releases_url.clone();
        config.preview = Some(PreviewSettings {
            domain: self.preview_domain.clone(),
            origins: self.preview_origins.clone(),
            tuning: PreviewTuning::default(),
        });
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
    /// The runner releases the installer routes serve; without them, the
    /// installers answer 503.
    pub runner_releases: Option<PathBuf>,
    /// Where the one object store lives.
    pub storage: Storage,
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
    /// The preview domain the backend keeps its namespace at; without it,
    /// none is registered and the pages are told none, as in a test that
    /// serves no preview domain.
    pub preview: Option<PreviewSettings>,
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
            runner_releases: None,
            storage: Storage::Local,
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
            // The product's start publishes its server release's
            // `commands/` and sets the catalog of them.
            native: NativeCatalog::unpublished(),
            lifecycle: LifecycleTuning::default(),
            cloud: CloudTuning::default(),
            preview: None,
            #[cfg(feature = "testing")]
            object_counts: None,
        }
    }
}
