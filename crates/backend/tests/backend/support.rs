//! The test backend: a data directory, a clock the test moves, a mailbox
//! that captures verification codes, a machine manager the test scripts,
//! the command packages the workspace built as development releases, which
//! the backend's local store serves its runners, and an HTTP client that
//! sends a session's cookie.

use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::time::Duration;

use demi_backend::{Backend, BackendConfig, publish_commands};
use demi_backend_accounts::email_change::{AccountMail, MailError, VerificationMail};
use demi_backend_blobs::counting::ObjectCounts;
use demi_backend_cloud::tuning::CloudTuning;
use demi_backend_providers::llm::families::FamilyRegistry;
use demi_backend_remote_host::testing::{
    NativeFixture, RunnerProcess, RunnerProcessOptions, native_fixture_binary,
};
use demi_backend_user_shard::tuning::{
    ConversationTuning, LifecycleTuning, PageTuning, RunnerTuning,
};
use demi_command_declarations::NativeOperation;
use demi_command_package_browser_protocol::{
    Operation as BrowserOperation, PACKAGE as BROWSER_PACKAGE,
};
use demi_command_protocol::testing::built_program;
use demi_command_protocol::{PackageDescriptor, host_target};
use demi_runner_protocol::release::{SERVER_RELEASE, ServerRelease, compressed_file};
use demi_plugin_interface::{
    Manifest, Page, Plugin, PluginError, PluginFactory, PluginId, PluginPort, Reply, Request,
    Stream,
};
use demi_plugin_skills::testing::Repos;
pub use demi_provider_common::testing::ManualClock;
use demi_shared_types::Clock;
use demi_web_api_protocol::auth::{Identity, Role, UserDto};
use demi_web_api_protocol::devices::{DeviceAnswer, DeviceDto, DeviceState, Devices};
use demi_web_api_protocol::error::{ErrorBody, ErrorCode};
use demi_web_api_protocol::settings::InstanceMode;
use demi_web_api_protocol::state::{ProductState, SyncEvent};
use futures_util::future::LocalBoxFuture;
use futures_util::{SinkExt as _, StreamExt as _};
use reqwest::header::{COOKIE, HeaderMap, SET_COOKIE};
use reqwest::{Method, StatusCode};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::rc::Rc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_util::sync::CancellationToken;

use crate::machines::ScriptedManager;

/// Where a scenario that serves no models.dev document finds none: a port
/// nothing listens on. No scenario reaches the public models.dev; one that
/// reads the catalog serves its document with `Harness::with_models_dev`.
const NO_MODELS_DEV: &str = "http://127.0.0.1:9/api.json";

/// Where a scenario that serves no Claude Code distribution finds none,
/// likewise; one that installs the CLI serves it with
/// `Harness::with_claude_releases`.
const NO_CLAUDE_RELEASES: &str = "http://127.0.0.1:9/claude-code-releases";

pub const MASTER_EMAIL: &str = "master@example.test";
pub const MASTER_PASSWORD: &str = "master-pass-1";
pub const SESSION_COOKIE: &str = "demi_session";

/// Where the harness keeps the releases of the packages the workspace
/// built: Cargo's directory for integration tests, which outlives the test
/// process, since every backend of the process publishes the same release
/// files.
const RELEASES: &str = concat!(env!("CARGO_TARGET_TMPDIR"), "/backend-releases");

/// A command package the workspace built: its descriptor for this machine's
/// target and its program, and a server release that holds it, whose
/// descriptor every backend publishes into its own object store and whose
/// files hold the program's compressed copy, which a backend stores when a
/// runner first needs it (`native-runtime.md` § Backend deployment
/// configuration). Each test process computes the digest and writes the
/// release once: both read the whole program, most of a second for
/// `demi-browser`.
pub struct Built {
    pub descriptor: PackageDescriptor,
    pub program: PathBuf,
    release: OnceLock<PathBuf>,
}

impl Built {
    fn new<'a>(id: &str, program: PathBuf, operations: impl IntoIterator<Item = &'a str>) -> Self {
        let descriptor = NativeFixture::package(id, program.clone(), operations).descriptor;
        Self {
            descriptor,
            program,
            release: OnceLock::new(),
        }
    }

    /// The root of the server release that holds this package for this
    /// machine's target: the conversations bind to its package, and every
    /// runner, a paired device's or the Cloud's, downloads its program from
    /// the backend.
    fn release(&self) -> &std::path::Path {
        self.release.get_or_init(|| {
            let executable = self.program.file_name().unwrap().to_str().unwrap();
            write_release(executable, &[self])
        })
    }
}

/// Writes the root of the release `name` of `packages`, which holds their
/// descriptors and names their files, and the files, which hold their
/// programs' compressed copies, and answers the root. Each file is replaced
/// whole, so a backend of another test process that reads it meanwhile
/// reads it whole.
fn write_release(name: &str, packages: &[&Built]) -> PathBuf {
    let root = PathBuf::from(RELEASES).join(format!("release-{name}"));
    let files = PathBuf::from(RELEASES).join(format!("files-{name}"));
    std::fs::create_dir_all(&files).unwrap();
    for built in packages {
        let executable = built.program.file_name().unwrap().to_str().unwrap();
        let package = root.join("commands").join(executable);
        std::fs::create_dir_all(&package).unwrap();
        // A test's copy is made fast: the backend checks only that it
        // decodes to the program.
        let bytes = std::fs::read(&built.program).unwrap();
        let encoded =
            demi_shared_artifacts::encode_blocking(&bytes, demi_shared_artifacts::Effort::Fast)
                .unwrap();
        replace(&files.join(compressed_file(executable, host_target())), &encoded);
        replace(
            &package.join("descriptor.json"),
            &serde_json::to_vec(&built.descriptor).unwrap(),
        );
    }
    write_server_release(&root, &files);
    root
}

/// Writes `root`'s `release.json`, which names `files` as where the
/// release's files are.
pub fn write_server_release(root: &std::path::Path, files: &std::path::Path) {
    let record = ServerRelease {
        files: files.to_str().unwrap().to_owned(),
    };
    replace(
        &root.join(SERVER_RELEASE),
        &serde_json::to_vec(&record).unwrap(),
    );
}

/// Writes `bytes` to `path` whole: to a file of this process's beside it,
/// then renamed over it.
fn replace(path: &std::path::Path, bytes: &[u8]) {
    let staged = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&staged, bytes).unwrap();
    std::fs::rename(&staged, path).unwrap();
}

/// `demi.file`, from `demi-file`.
static FILE: LazyLock<Built> = LazyLock::new(|| {
    Built::new(
        demi_command_package_file_protocol::PACKAGE,
        built_program("demi-file"),
        demi_command_package_file_protocol::OPERATIONS
            .iter()
            .copied(),
    )
});

/// `demi.browser`, from `demi-browser`.
static BROWSER: LazyLock<Built> = LazyLock::new(|| {
    Built::new(
        BROWSER_PACKAGE,
        built_program("demi-browser"),
        BrowserOperation::names(),
    )
});

/// `demi.claude-code`, from `demi-claude-code`.
static CLAUDE: LazyLock<Built> = LazyLock::new(|| {
    Built::new(
        demi_command_package_claude_code_protocol::PACKAGE,
        built_program("demi-claude-code"),
        demi_command_package_claude_code_protocol::Operation::ALL
            .map(demi_command_package_claude_code_protocol::Operation::name),
    )
});

/// The runner's native fixture package: `echo` answers what the page
/// sends, `where` reports its context and directory.
pub static FIXTURE: LazyLock<Built> = LazyLock::new(|| Built {
    descriptor: NativeFixture::load().descriptor,
    program: native_fixture_binary(),
    release: OnceLock::new(),
});

/// A second package of the fixture plugin's user streams, which serves
/// nothing: its artifact for this machine's target is a small file, which a
/// stream of the fixture's may not install, since only the extra stream
/// binds its package.
pub static EXTRA: LazyLock<Built> = LazyLock::new(|| {
    let program = PathBuf::from(RELEASES).join("extra-artifact");
    std::fs::create_dir_all(RELEASES).unwrap();
    let bytes: Vec<u8> = (0..100_000u32).map(|index| (index * 13) as u8).collect();
    replace(&program, &bytes);
    Built::new("demicodes.runner-test-extra", program, ["extra"])
});

/// The release of the fixture's package and the extra one.
static FIXTURE_AND_EXTRA: LazyLock<PathBuf> =
    LazyLock::new(|| write_release("fixture-and-extra", &[&FIXTURE, &EXTRA]));

/// Captures verification mail, or refuses it while `failing` is set.
#[derive(Default)]
pub struct Mailbox {
    sent: Mutex<Vec<VerificationMail>>,
    pub failing: AtomicBool,
}

impl AccountMail for Mailbox {
    fn send_verification(
        &self,
        mail: VerificationMail,
    ) -> Pin<Box<dyn Future<Output = Result<(), MailError>> + Send>> {
        let delivered = if self.failing.load(Ordering::SeqCst) {
            Err(MailError("the mail transport failed".to_owned()))
        } else {
            self.sent.lock().unwrap().push(mail);
            Ok(())
        };
        Box::pin(std::future::ready(delivered))
    }
}

impl Mailbox {
    pub fn sent(&self) -> Vec<VerificationMail> {
        self.sent.lock().unwrap().clone()
    }
}

/// What a test backend starts from; it can start several times over the
/// same data directory.
pub struct Harness {
    data: tempfile::TempDir,
    pub clock: Arc<ManualClock>,
    pub mailbox: Arc<Mailbox>,
    mail: bool,
    web_directory: Option<PathBuf>,
    mode: InstanceMode,
    families: FamilyRegistry,
    models_dev_url: Option<String>,
    claude_releases: Option<String>,
    pub runners: RunnerTuning,
    pub conversations: ConversationTuning,
    pub pages: PageTuning,
    runner_releases: Option<PathBuf>,
    /// The development release of the workspace's packages the backends
    /// load.
    release: Option<fn() -> &'static std::path::Path>,
    /// The URL runners connect to; without it, the listener's own address.
    pub public_url: Option<url::Url>,
    /// The user streams of a test plugin registered beside the built-in
    /// ones.
    user_streams: Option<Vec<Stream>>,
    pub lifecycle: LifecycleTuning,
    pub cloud: CloudTuning,
    /// Runs the Clouds of every backend this harness starts, unless
    /// `machines` names a real manager.
    pub manager: ScriptedManager,
    /// A real machine manager's socket, which the backends use instead of
    /// the scripted manager's (`scenarios.md` § Cloud suite).
    pub machines: Option<PathBuf>,
    /// The root of a server release whose command packages the backends
    /// publish (`native-runtime.md` § Backend deployment configuration),
    /// instead of a package the workspace built.
    pub server_release: Option<PathBuf>,
    /// Counts what reaches the object store of every backend this harness
    /// starts.
    objects: Option<ObjectCounts>,
    /// The repositories the skills plugin fetches instead of the internet's.
    skill_repos: Option<Arc<Repos>>,
    /// Whether the backends have the `fixture-panel` plugin and its `page`
    /// kind.
    panel_fixture: bool,
}

impl Harness {
    pub fn new() -> Self {
        Self {
            data: tempfile::Builder::new()
                .prefix("demi-backend-")
                .tempdir()
                .unwrap(),
            clock: Arc::new(ManualClock::new(
                "2026-09-24T08:00:00.000Z".parse().unwrap(),
            )),
            mailbox: Arc::new(Mailbox::default()),
            mail: false,
            web_directory: None,
            mode: InstanceMode::Shared,
            families: demi_backend::families::builtin(),
            models_dev_url: None,
            claude_releases: None,
            // Liveness would ping every 30 s; the tests end runners
            // themselves.
            runners: RunnerTuning {
                ping: None,
                ..RunnerTuning::default()
            },
            // A scripted vendor answers the turns a scenario scripts; a title
            // request beside the first turn would take one of its answers.
            conversations: ConversationTuning {
                titles: false,
                ..ConversationTuning::default()
            },
            pages: PageTuning::default(),
            runner_releases: None,
            release: None,
            public_url: None,
            user_streams: None,
            lifecycle: LifecycleTuning::default(),
            cloud: CloudTuning::default(),
            manager: ScriptedManager::start(),
            machines: None,
            server_release: None,
            objects: None,
            skill_repos: None,
            panel_fixture: false,
        }
    }

    /// Backends whose skills plugin fetches `repos` for the origins
    /// `owner/repo`.
    pub fn with_skill_repos(mut self, repos: &Arc<Repos>) -> Self {
        self.skill_repos = Some(repos.clone());
        self
    }

    /// Backends with a plugin of the work panel kind `page`, which does
    /// nothing with a tab: the panel's own rules, with no plugin's work
    /// beside them.
    pub fn with_panel_fixture(mut self) -> Self {
        self.panel_fixture = true;
        self
    }

    /// Backends whose object store counts what reaches it in `counts`.
    pub fn with_object_counts(mut self, counts: &ObjectCounts) -> Self {
        self.objects = Some(counts.clone());
        self
    }

    /// Conversations whose `demi file` commands bind to the `demi.file`
    /// package the workspace built, which runners install from the backend.
    pub fn with_file_package(mut self) -> Self {
        self.release = Some(|| FILE.release());
        self
    }

    /// Conversations whose `demi browser` commands and `browser` user
    /// stream bind to the `demi.browser` package the workspace built.
    pub fn with_browser_package(mut self) -> Self {
        self.release = Some(|| BROWSER.release());
        self
    }

    /// Conversations whose user streams bind to the runner's native test
    /// fixture: `echo` answers what the page sends, `where` reports its
    /// context and directory, `retain` makes the resident service hold the
    /// conversation, as the browser service holds a conversation's Chrome
    /// until its release, `held` answers what the service holds,
    /// `stall_release` holds the conversation so that its release never ends
    /// by itself, and `stalled` ends once such a release waits.
    pub fn with_native_fixture(mut self) -> Self {
        let package = FIXTURE.descriptor.id.clone();
        let stream = |operation: &str| NativeOperation {
            package: package.clone(),
            operation: operation.into(),
        };
        self.user_streams = Some(
            [
                "echo",
                "where",
                "retain",
                "held",
                "stall_release",
                "stalled",
            ]
            .map(|name| Stream::new::<Value, Value>(name, stream(name)))
            .into(),
        );
        self.release = Some(|| FIXTURE.release());
        self
    }

    /// Beside the native fixture's streams, its `install`, which installs
    /// the artifact its arguments name and prints its path, and `extra`, a
    /// stream of the extra package, whose artifact only a stream of that
    /// name may install.
    pub fn with_extra_package(mut self) -> Self {
        let streams = self
            .user_streams
            .as_mut()
            .expect("the native fixture's streams come first");
        let stream = |package: &Built, operation: &str| {
            let binding = NativeOperation {
                package: package.descriptor.id.clone(),
                operation: operation.into(),
            };
            Stream::new::<Value, Value>(operation, binding)
        };
        streams.push(stream(&FIXTURE, "install"));
        streams.push(stream(&EXTRA, "extra"));
        self.release = Some(|| FIXTURE_AND_EXTRA.as_path());
        self
    }

    /// The server release at `root`, whose `runners/` the installer routes
    /// serve and whose files hold the runner executables.
    pub fn with_runner_releases(mut self, root: PathBuf) -> Self {
        self.runner_releases = Some(root.join("runners"));
        self.server_release = Some(root);
        self
    }

    /// The provider families the backend assembles entries with.
    pub fn with_families(mut self, families: FamilyRegistry) -> Self {
        self.families = families;
        self
    }

    /// Where the backend reads the models.dev document.
    pub fn with_models_dev(mut self, url: String) -> Self {
        self.models_dev_url = Some(url);
        self
    }

    /// Where the backend reads the Claude Code distribution.
    pub fn with_claude_releases(mut self, url: String) -> Self {
        self.claude_releases = Some(url);
        self
    }

    /// A Claude Code provider's CLI is listed and installed by the
    /// `demi.claude-code` package the workspace built, which a Cloud's runner
    /// installs from the backend.
    pub fn with_claude_package(mut self) -> Self {
        self.release = Some(|| CLAUDE.release());
        self
    }

    /// The control database, opened beside the backend's own connection.
    pub fn control_database(&self) -> rusqlite::Connection {
        let connection =
            rusqlite::Connection::open(self.data_dir().join("control.sqlite")).unwrap();
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        connection
    }

    /// An account setup did not create, written to the control database:
    /// account administration is not a route of this backend yet.
    pub fn add_user(&self, email: &str, password: &str, role: Role) {
        use argon2::password_hash::rand_core::OsRng;
        use argon2::password_hash::{PasswordHasher as _, SaltString};
        let hash = argon2::Argon2::default()
            .hash_password(password.as_bytes(), &SaltString::generate(&mut OsRng))
            .unwrap()
            .to_string();
        self.control_database()
            .execute(
                "INSERT INTO users (id, email, nickname, password_hash, role, created_at)
                 VALUES (?1, ?2, '', ?3, ?4, ?5)",
                rusqlite::params![
                    uuid::Uuid::new_v4().to_string(),
                    email,
                    hash,
                    role.to_string(),
                    self.clock.now().as_millisecond()
                ],
            )
            .unwrap();
    }

    pub fn with_mode(mut self, mode: InstanceMode) -> Self {
        self.mode = mode;
        self
    }

    /// The data directory the backend keeps its storage in.
    pub fn data_dir(&self) -> PathBuf {
        self.data.path().join("backend")
    }

    pub fn with_mail(mut self) -> Self {
        self.mail = true;
        self
    }

    /// A web app build in the data directory, served beside the API.
    pub fn with_web(mut self, files: &[(&str, &str)]) -> Self {
        let directory = self.data.path().join("web");
        std::fs::create_dir_all(&directory).unwrap();
        for (name, content) in files {
            std::fs::write(directory.join(name), content).unwrap();
        }
        self.web_directory = Some(directory);
        self
    }

    /// The web app build `with_web` wrote, which a test may replace between starts.
    pub fn web_dir(&self) -> PathBuf {
        self.web_directory
            .clone()
            .expect("the harness serves a web app build")
    }

    pub async fn start(&self) -> TestBackend {
        self.start_in_mode(self.mode).await
    }

    /// The backend over this harness's data, in `mode`.
    pub async fn start_in_mode(&self, mode: InstanceMode) -> TestBackend {
        self.launch(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)), mode)
            .await
    }

    /// A backend listening on `address`, such as the one an earlier start
    /// of the same data directory had, which its runners reconnect to: a
    /// runner's state names its backend's URL. Only a restart whose runners
    /// must come back uses it, since another test may take the port between
    /// the close and the start; one that needs only a stable URL sets
    /// `public_url` and starts on a port of its own.
    pub async fn start_at(&self, address: SocketAddr) -> TestBackend {
        self.launch(address, self.mode).await
    }

    async fn launch(&self, address: SocketAddr, mode: InstanceMode) -> TestBackend {
        let machines = self
            .machines
            .clone()
            .unwrap_or_else(|| self.manager.socket().to_owned());
        let mut config = BackendConfig::new(self.data_dir(), address, mode, machines);
        config.lifecycle = self.lifecycle;
        config.cloud = self.cloud;
        config.clock = self.clock.clone();
        config.web_directory = self.web_directory.clone();
        config.families = self.families.clone();
        config.runners = self.runners;
        config.runner_releases = self.runner_releases.clone();
        config.conversations = self.conversations;
        config.pages = self.pages;
        config.public_url = self.public_url.clone();
        config.object_counts = self.objects.clone();
        assert!(
            self.release.is_none() || self.server_release.is_none(),
            "a harness loads a workspace package or a server release, not both"
        );
        let release = match (self.release, &self.server_release) {
            (Some(release), _) => Some(release()),
            (None, release) => release.as_deref(),
        };
        if let Some(release) = release {
            config.native = publish_commands(&config, release, &CancellationToken::new())
                .await
                .unwrap();
        }
        if let Some(streams) = &self.user_streams {
            config
                .plugins
                .push(Box::new(StreamsPlugin::new(streams.clone())));
        }
        if self.panel_fixture {
            config.plugins.push(Box::new(PanelPlugin::new()));
        }
        if let Some(repos) = &self.skill_repos {
            let skills = config
                .plugins
                .iter_mut()
                .find(|plugin| plugin.manifest().id.as_str() == "skills")
                .expect("the backend has the skills plugin");
            *skills = Box::new(repos.skills());
        }
        config.models_dev_url = self
            .models_dev_url
            .as_deref()
            .unwrap_or(NO_MODELS_DEV)
            .parse()
            .unwrap();
        config.claude_releases = self
            .claude_releases
            .as_deref()
            .unwrap_or(NO_CLAUDE_RELEASES)
            .parse()
            .unwrap();
        if self.mail {
            config.account_mail = Some(self.mailbox.clone());
        }
        let backend = Backend::start(config).await.unwrap();
        TestBackend {
            url: format!("http://{}", backend.local_addr()),
            backend,
            http: reqwest::Client::builder().no_proxy().build().unwrap(),
        }
    }

    /// A started backend with the master account set up and signed in.
    pub async fn start_set_up(&self) -> (TestBackend, Session) {
        let backend = self.start().await;
        let master = backend.setup().await;
        (backend, master)
    }
}

pub struct TestBackend {
    pub url: String,
    backend: Backend,
    http: reqwest::Client,
}

/// A page's synchronization channel (`web-api.md` § Page synchronization).
pub struct SyncChannel {
    socket: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
}

impl SyncChannel {
    /// The next message, a heartbeat included.
    pub async fn next(&mut self) -> SyncEvent {
        loop {
            let message = tokio::time::timeout(PATIENCE, self.socket.next())
                .await
                .unwrap_or_else(|_| panic!("the channel sends within {PATIENCE:?}"));
            match message {
                Some(Ok(Message::Text(text))) => {
                    return serde_json::from_str(text.as_str())
                        .unwrap_or_else(|error| panic!("{error}: {text}"));
                }
                Some(Ok(Message::Close(close))) => panic!("the channel closed: {close:?}"),
                Some(Ok(_)) => {}
                other => panic!("the channel ended: {other:?}"),
            }
        }
    }

    /// The product state, the channel's first message.
    pub async fn snapshot(&mut self) -> ProductState {
        match self.next().await {
            SyncEvent::Snapshot { state } => *state,
            other => panic!("the channel's first message is not the snapshot: {other:?}"),
        }
    }

    /// The messages up to and including the first that `done` accepts.
    pub async fn until(&mut self, done: impl Fn(&SyncEvent) -> bool) -> Vec<SyncEvent> {
        let mut received = Vec::new();
        loop {
            let event = self.next().await;
            let last = done(&event);
            received.push(event);
            if last {
                return received;
            }
        }
    }

    /// The code and reason the backend closed the channel with, after the
    /// messages still on their way.
    pub async fn closed(&mut self) -> (u16, String) {
        loop {
            let message = tokio::time::timeout(PATIENCE, self.socket.next())
                .await
                .unwrap_or_else(|_| panic!("the channel closes within {PATIENCE:?}"));
            match message {
                Some(Ok(Message::Close(Some(close)))) => {
                    return (close.code.into(), close.reason.to_string());
                }
                Some(Ok(Message::Close(None))) => panic!("the channel closed without a code"),
                Some(Ok(_)) => {}
                other => panic!("the channel ended without a close: {other:?}"),
            }
        }
    }

    /// Sends the backend a text message, which a page never does.
    pub async fn send_text(&mut self, text: &str) {
        self.socket.send(Message::Text(text.into())).await.unwrap();
    }
}

/// An HTTP answer, read whole.
pub struct Answer {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl Answer {
    pub fn json<T: DeserializeOwned>(&self) -> T {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&self.body)))
    }

    pub fn error(&self) -> ErrorBody {
        self.json()
    }

    /// The answer's status and error code.
    pub fn refusal(&self) -> (StatusCode, ErrorCode) {
        (self.status, self.error().code)
    }

    /// The `Set-Cookie` values that set or clear the session cookie.
    pub fn session_cookies(&self) -> Vec<String> {
        self.headers
            .get_all(SET_COOKIE)
            .iter()
            .map(|value| value.to_str().unwrap().to_owned())
            .filter(|value| value.starts_with(&format!("{SESSION_COOKIE}=")))
            .collect()
    }
}

/// A response read whole.
pub async fn answer(response: reqwest::Response) -> Answer {
    Answer {
        status: response.status(),
        headers: response.headers().clone(),
        body: response.bytes().await.unwrap().to_vec(),
    }
}

/// A signed-in user's session, as their browser holds it: its cookie on
/// every request.
#[derive(Clone)]
pub struct Session {
    pub cookie: String,
    pub user: UserDto,
}

impl TestBackend {
    pub async fn close(self) {
        self.backend.close().await.unwrap();
    }

    /// Shuts the backend down and answers the steps that failed.
    pub async fn close_reporting(self) -> Result<(), demi_backend::ShutdownErrors> {
        self.backend.close().await
    }

    pub fn address(&self) -> SocketAddr {
        self.backend.local_addr()
    }

    /// Holds every commit of a conversation's checkpoint from now on, until
    /// the hold is released or dropped.
    pub fn hold_commits(&self) -> demi_backend_database::conversations::CommitHold {
        self.backend.hold_commits()
    }

    /// Holds every runner's hello at `step` from now on, until the hold is
    /// released or dropped.
    pub fn hold_hellos(
        &self,
        step: demi_backend_user_shard::holds::HelloStep,
    ) -> demi_backend_user_shard::holds::StepHold {
        self.backend.hold_hellos(step)
    }

    /// Holds every page's synchronization channel at `step` from now on,
    /// until the hold is released or dropped.
    pub fn hold_sync(
        &self,
        step: demi_backend_user_shard::sync::SyncStep,
    ) -> demi_backend_user_shard::holds::StepHold {
        self.backend.hold_sync(step)
    }

    /// The session's synchronization channel, opened from a page of the
    /// product; its first message is the snapshot.
    pub async fn sync(&self, session: &Session) -> SyncChannel {
        let mut request = self.ws_url("/api/sync").into_client_request().unwrap();
        let headers = request.headers_mut();
        headers.insert("cookie", session.cookie.parse().unwrap());
        headers.insert("origin", self.url.parse().unwrap());
        let (socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        SyncChannel { socket }
    }

    /// The file gate of `session`'s user's conversation `conversation`.
    pub async fn file_gate(
        &self,
        session: &Session,
        conversation: &str,
    ) -> demi_shared_gates::ActivityGate {
        let conversation =
            demi_web_api_protocol::ids::ConversationId::try_from(conversation).unwrap();
        self.backend
            .file_gate(&session.user.id, &conversation)
            .await
    }

    /// Waits until no collection of `session`'s user's blobs runs or is to
    /// follow.
    pub async fn until_collected(&self, session: &Session) {
        self.backend.until_collected(&session.user.id).await;
    }

    /// The `ws://` URL of `path`.
    pub fn ws_url(&self, path: &str) -> String {
        format!("ws://{}{path}", self.backend.local_addr())
    }

    /// The session's paired devices, as `GET /api/devices` lists them.
    pub async fn devices(&self, session: &Session) -> Vec<DeviceDto> {
        let answer = self.get("/api/devices", Some(session)).await;
        assert_eq!(
            answer.status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&answer.body)
        );
        answer.json::<Devices>().devices
    }

    /// Whether the session's device `id` is online, as the device list says.
    pub async fn online(&self, session: &Session, id: &str) -> bool {
        self.devices(session)
            .await
            .iter()
            .any(|device| device.id.as_str() == id && device.state == DeviceState::Online)
    }

    /// Waits until the device's online state is `online`.
    pub async fn until_online(&self, session: &Session, id: &str, online: bool) {
        eventually(&format!("device {id} online: {online}"), || async {
            self.online(session, id).await == online
        })
        .await;
    }

    /// A runner of a new device named `name`, started, claimed by `session`,
    /// online and holding its device token, so a test may stop it and start
    /// it again as the same device.
    pub async fn pair(&self, session: &Session, name: &str) -> Paired {
        self.pair_through(session, name, &self.url).await
    }

    /// [`TestBackend::pair`], with a runner that reaches the backend at
    /// `url`, such as an edge in front of it.
    pub async fn pair_through(&self, session: &Session, name: &str, url: &str) -> Paired {
        let runner = RunnerProcess::start(
            url,
            RunnerProcessOptions {
                name: name.into(),
                ..RunnerProcessOptions::default()
            },
        );
        let code = runner.pairing_code(0).await;
        let claimed = self
            .post("/api/devices/claim", Some(session), json!({ "code": code }))
            .await;
        assert_eq!(
            claimed.status,
            StatusCode::CREATED,
            "{}",
            String::from_utf8_lossy(&claimed.body)
        );
        let device = claimed.json::<DeviceAnswer>().device;
        self.until_online(session, device.id.as_str(), true).await;
        stored_token(&runner).await;
        Paired { runner, device }
    }
}

/// A paired device and its runner.
pub struct Paired {
    pub runner: RunnerProcess,
    pub device: DeviceDto,
}

impl Paired {
    pub fn id(&self) -> &str {
        self.device.id.as_str()
    }

    /// The device token the runner stored once it was claimed.
    pub async fn token(&self) -> String {
        stored_token(&self.runner).await
    }
}

/// The device token `runner` stored once it was claimed. The backend binds
/// the device before the runner hears its token, so the claim answers, and
/// the device is online, a moment before the runner holds the token; a
/// runner stopped in that moment comes back unpaired.
pub async fn stored_token(runner: &RunnerProcess) -> String {
    let path = runner.state_dir().join("runner-token");
    eventually("the runner stores its token", || {
        let stored = path.exists();
        async move { stored }
    })
    .await;
    std::fs::read_to_string(path).unwrap().trim().to_owned()
}

/// `length` bytes that `seed` varies, which repeat only every 64,256 bytes,
/// so that a read from a shifted offset shows. One period is made a byte at a
/// time and then copied, since making every byte one at a time is slow in a
/// debug build.
pub fn pattern(length: usize, seed: u8) -> Vec<u8> {
    const PERIOD: usize = 251 * 256;
    let period: Vec<u8> = (0..PERIOD)
        .map(|index| ((index * 31 + (index >> 8)) % 251) as u8 ^ seed)
        .collect();
    let mut bytes = Vec::with_capacity(length);
    while bytes.len() < length {
        let take = (length - bytes.len()).min(PERIOD);
        bytes.extend_from_slice(&period[..take]);
    }
    bytes
}

/// How long a scenario waits for something that should come true before it
/// fails as hung.
pub const PATIENCE: Duration = Duration::from_secs(20);

/// Waits until `check` holds, asking every 20 ms for at most [`PATIENCE`].
pub async fn eventually<F, Fut>(what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + PATIENCE;
    while !check().await {
        assert!(
            tokio::time::Instant::now() < deadline,
            "never came true: {what}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

impl TestBackend {
    pub async fn send(
        &self,
        method: Method,
        path: &str,
        cookie: Option<&str>,
        body: Option<Value>,
    ) -> Answer {
        let mut request = self.http.request(method, format!("{}{path}", self.url));
        if let Some(cookie) = cookie {
            request = request.header(COOKIE, cookie);
        }
        if let Some(body) = body {
            request = request
                .header("content-type", "application/json")
                .body(body.to_string());
        }
        answer(request.send().await.unwrap()).await
    }

    pub async fn get(&self, path: &str, session: Option<&Session>) -> Answer {
        self.send(
            Method::GET,
            path,
            session.map(|session| session.cookie.as_str()),
            None,
        )
        .await
    }

    pub async fn post(&self, path: &str, session: Option<&Session>, body: Value) -> Answer {
        self.send(
            Method::POST,
            path,
            session.map(|session| session.cookie.as_str()),
            Some(body),
        )
        .await
    }

    pub async fn patch(&self, path: &str, session: &Session, body: Value) -> Answer {
        self.send(Method::PATCH, path, Some(&session.cookie), Some(body))
            .await
    }

    pub async fn put(&self, path: &str, session: &Session, body: Value) -> Answer {
        self.send(Method::PUT, path, Some(&session.cookie), Some(body))
            .await
    }

    pub async fn delete(&self, path: &str, session: &Session) -> Answer {
        self.send(Method::DELETE, path, Some(&session.cookie), None)
            .await
    }

    /// A GET with the session's cookie and extra headers.
    pub async fn get_with(
        &self,
        path: &str,
        session: &Session,
        headers: &[(&str, &str)],
    ) -> Answer {
        answer(
            self.response(Method::GET, path, session, headers, None)
                .await,
        )
        .await
    }

    /// The response to a request with the session's cookie, extra headers
    /// and a body, before its body is read.
    pub async fn response(
        &self,
        method: Method,
        path: &str,
        session: &Session,
        headers: &[(&str, &str)],
        body: Option<reqwest::Body>,
    ) -> reqwest::Response {
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.url))
            .header(COOKIE, &session.cookie);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        if let Some(body) = body {
            request = request.body(body);
        }
        request.send().await.unwrap()
    }

    pub async fn setup(&self) -> Session {
        let answer = self
            .post(
                "/api/setup",
                None,
                json!({ "nickname": "Master", "email": MASTER_EMAIL, "password": MASTER_PASSWORD }),
            )
            .await;
        assert_eq!(
            answer.status,
            StatusCode::CREATED,
            "{}",
            String::from_utf8_lossy(&answer.body)
        );
        session_from(&answer)
    }

    pub async fn login(&self, email: &str, password: &str) -> Session {
        let answer = self
            .post(
                "/api/auth/login",
                None,
                json!({ "email": email, "password": password }),
            )
            .await;
        assert_eq!(
            answer.status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&answer.body)
        );
        session_from(&answer)
    }

    pub async fn login_answer(&self, email: &str, password: &str) -> Answer {
        self.post(
            "/api/auth/login",
            None,
            json!({ "email": email, "password": password }),
        )
        .await
    }
}

/// The session a setup or login answer signed in.
pub fn session_from(answer: &Answer) -> Session {
    let set = answer.session_cookies();
    let [cookie] = set.as_slice() else {
        panic!("expected one session cookie, got {set:?}");
    };
    let pair = cookie.split(';').next().unwrap().to_owned();
    Session {
        cookie: pair,
        user: answer.json::<Identity>().user,
    }
}

/// A plugin that declares only user streams, bound to the runner's native
/// test fixture.
struct StreamsPlugin(Manifest);

impl StreamsPlugin {
    fn new(streams: Vec<Stream>) -> Self {
        let mut manifest = Manifest::new(
            PluginId::try_from("fixture").unwrap(),
            "Fixture streams",
            "The native test fixture's user streams.",
        );
        manifest.streams = streams;
        Self(manifest)
    }
}

impl PluginFactory for StreamsPlugin {
    fn manifest(&self) -> &Manifest {
        &self.0
    }

    fn instance(&self) -> Rc<dyn Plugin> {
        Rc::new(NoRequests)
    }
}

/// A plugin of the work panel kind `page`, which does nothing with a tab.
struct PanelPlugin(Manifest);

impl PanelPlugin {
    fn new() -> Self {
        let mut manifest = Manifest::new(
            PluginId::try_from("fixture-panel").unwrap(),
            "Fixture panel",
            "A work panel kind whose tabs the scenarios change.",
        );
        manifest.page = Some(Page::new("@demicodes/plugin-fixture-panel").panel_kind("page"));
        Self(manifest)
    }
}

impl PluginFactory for PanelPlugin {
    fn manifest(&self) -> &Manifest {
        &self.0
    }

    fn instance(&self) -> Rc<dyn Plugin> {
        Rc::new(PanelTabs)
    }
}

/// An instance told of each tab its user creates or removes, which does
/// nothing with it.
struct PanelTabs;

impl Plugin for PanelTabs {
    fn call(&self, request: Request, _: PluginPort) -> LocalBoxFuture<'_, Result<Reply, PluginError>> {
        Box::pin(async move {
            match request {
                Request::PanelTab { .. } => Ok(Reply::Done),
                _ => unreachable!("a plugin of a panel kind only is told of its tabs"),
            }
        })
    }
}

/// An instance that is sent no request: the plugin declares no command and
/// no page.
struct NoRequests;

impl Plugin for NoRequests {
    fn call(&self, _: Request, _: PluginPort) -> LocalBoxFuture<'_, Result<Reply, PluginError>> {
        unreachable!("a plugin of streams only receives no request")
    }
}
