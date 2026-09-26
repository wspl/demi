//! The test backend: a data directory, a clock the test moves, a mailbox
//! that captures verification codes, a machine manager the test scripts,
//! and an HTTP client that sends a session's cookie.

use std::collections::BTreeMap;
use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use demi_backend::{
    AccountMail, Backend, BackendConfig, CloudTuning, ConversationTuning, ExposeDomain, ExposeTuning, FamilyRegistry,
    LifecycleTuning, LoginTiming, MailError, NativeCatalog, RunnerTuning, VerificationMail,
};
use demi_builtin_protocol::{Operation, PACKAGE as BUILTIN_PACKAGE};
use demi_command_service::testing::built_program;
use demi_command_tree::NativeOperation;
use demi_core::Clock;
use demi_host_remote::testing::{NativeFixture, RunnerProcess, RunnerProcessOptions};
use demi_web_api::auth::{Identity, Role, UserDto};
use demi_web_api::devices::{ClaimedDevice, DeviceDto, Devices};
use demi_web_api::error::{ErrorBody, ErrorCode};
use demi_web_api::settings::InstanceMode;
use jiff::{SignedDuration, Timestamp};
use reqwest::header::{COOKIE, HeaderMap, SET_COOKIE};
use reqwest::{Method, StatusCode};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

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

/// Wall-clock time the test sets.
pub struct ManualClock(Mutex<Timestamp>);

impl Clock for ManualClock {
    fn now(&self) -> demi_core::Timestamp {
        demi_core::Timestamp::truncate(*self.0.lock().unwrap())
    }
}

impl ManualClock {
    pub fn advance(&self, by: SignedDuration) {
        let mut now = self.0.lock().unwrap();
        *now = now.checked_add(by).unwrap();
    }
}

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
    logins: LoginTiming,
    pub runners: RunnerTuning,
    pub conversations: ConversationTuning,
    runner_releases: Option<PathBuf>,
    native: Option<NativeCatalog>,
    user_streams: Option<BTreeMap<String, NativeOperation>>,
    pub lifecycle: LifecycleTuning,
    pub cloud: CloudTuning,
    /// Runs the Clouds of every backend this harness starts.
    pub manager: ScriptedManager,
    expose_domain: Option<ExposeDomain>,
    pub exposes: ExposeTuning,
}

impl Harness {
    pub fn new() -> Self {
        Self {
            data: tempfile::Builder::new().prefix("demi-backend-").tempdir().unwrap(),
            clock: Arc::new(ManualClock(Mutex::new(
                "2026-09-24T08:00:00Z".parse::<Timestamp>().unwrap(),
            ))),
            mailbox: Arc::new(Mailbox::default()),
            mail: false,
            web_directory: None,
            mode: InstanceMode::Shared,
            families: FamilyRegistry::builtin(),
            models_dev_url: None,
            claude_releases: None,
            logins: LoginTiming::default(),
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
            runner_releases: None,
            native: None,
            user_streams: None,
            lifecycle: LifecycleTuning::default(),
            cloud: CloudTuning::default(),
            manager: ScriptedManager::start(),
            expose_domain: None,
            exposes: ExposeTuning::default(),
        }
    }

    /// Exposes under `domain`, whose hostnames the backend answers with the
    /// public relay.
    pub fn with_expose_domain(mut self, domain: &str) -> Self {
        self.expose_domain = Some(domain.parse().unwrap());
        self
    }

    /// Conversations whose commands bind to the `demi.builtin` package the
    /// workspace built, which a runner on this machine installs from where
    /// it was built.
    pub fn with_builtin_package(mut self) -> Self {
        let builtin = Arc::new(NativeFixture::package(
            BUILTIN_PACKAGE,
            built_program("demi-commands"),
            Operation::names(),
        ));
        let packages = vec![builtin.descriptor.clone()];
        self.native = Some(NativeCatalog::new(packages, move || builtin.resolver()).unwrap());
        self
    }

    /// Conversations whose user streams bind to the runner's native test
    /// fixture: `echo` answers what the page sends, `where` reports its
    /// context and directory.
    pub fn with_native_fixture(mut self) -> Self {
        let fixture = Arc::new(NativeFixture::load());
        let package = fixture.descriptor.id.clone();
        let stream = |operation: &str| NativeOperation {
            package: package.clone(),
            operation: operation.into(),
        };
        self.user_streams = Some(BTreeMap::from([
            ("echo".to_owned(), stream("echo")),
            ("where".to_owned(), stream("where")),
        ]));
        let packages = vec![fixture.descriptor.clone()];
        self.native = Some(NativeCatalog::new(packages, move || fixture.resolver()).unwrap());
        self
    }

    /// The runner releases the installer routes serve.
    pub fn with_runner_releases(mut self, directory: PathBuf) -> Self {
        self.runner_releases = Some(directory);
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
    /// `demi.claude` package the workspace built, which a Cloud's runner on
    /// this machine runs from where it was built.
    pub fn with_claude_package(mut self) -> Self {
        let claude = Arc::new(NativeFixture::package(
            demi_claude_protocol::PACKAGE,
            built_program("demi-claude"),
            demi_claude_protocol::Operation::ALL.map(demi_claude_protocol::Operation::name),
        ));
        let packages = vec![claude.descriptor.clone()];
        self.native = Some(NativeCatalog::new(packages, move || claude.resolver()).unwrap());
        self
    }

    pub fn with_logins(mut self, logins: LoginTiming) -> Self {
        self.logins = logins;
        self
    }

    /// The control database, opened beside the backend's own connection.
    pub fn control_database(&self) -> rusqlite::Connection {
        let connection = rusqlite::Connection::open(self.data_dir().join("control.sqlite")).unwrap();
        connection.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
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

    /// A browser build in the data directory, served beside the API.
    pub fn with_web(mut self, files: &[(&str, &str)]) -> Self {
        let directory = self.data.path().join("web");
        std::fs::create_dir_all(&directory).unwrap();
        for (name, content) in files {
            std::fs::write(directory.join(name), content).unwrap();
        }
        self.web_directory = Some(directory);
        self
    }

    pub async fn start(&self) -> TestBackend {
        self.start_in_mode(self.mode).await
    }

    /// The backend over this harness's data, in `mode`.
    pub async fn start_in_mode(&self, mode: InstanceMode) -> TestBackend {
        self.launch(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)), mode).await
    }

    /// A backend listening on `address`, such as the one an earlier start
    /// of the same data directory had, which its runners reconnect to.
    pub async fn start_at(&self, address: SocketAddr) -> TestBackend {
        self.launch(address, self.mode).await
    }

    async fn launch(&self, address: SocketAddr, mode: InstanceMode) -> TestBackend {
        let mut config = BackendConfig::new(self.data_dir(), address, mode, self.manager.socket().to_owned());
        config.lifecycle = self.lifecycle;
        config.cloud = self.cloud;
        config.clock = self.clock.clone();
        config.web_directory = self.web_directory.clone();
        config.expose_domain = self.expose_domain.clone();
        config.exposes = self.exposes;
        config.families = self.families.clone();
        config.logins = self.logins;
        config.runners = self.runners;
        config.runner_releases = self.runner_releases.clone();
        config.conversations = self.conversations;
        if let Some(native) = &self.native {
            config.native = native.clone();
        }
        if let Some(streams) = &self.user_streams {
            config.user_streams = streams.clone();
        }
        config.models_dev_url = self.models_dev_url.as_deref().unwrap_or(NO_MODELS_DEV).parse().unwrap();
        config.claude_releases = self.claude_releases.as_deref().unwrap_or(NO_CLAUDE_RELEASES).parse().unwrap();
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

/// An HTTP answer, read whole.
pub struct Answer {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl Answer {
    pub fn json<T: DeserializeOwned>(&self) -> T {
        serde_json::from_slice(&self.body).unwrap_or_else(|error| {
            panic!("{error}: {}", String::from_utf8_lossy(&self.body))
        })
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

/// A signed-in browser: its cookie on every request.
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
    pub fn hold_commits(&self) -> demi_backend::CommitHold {
        self.backend.hold_commits()
    }

    /// The `ws://` URL of `path`.
    pub fn ws_url(&self, path: &str) -> String {
        format!("ws://{}{path}", self.backend.local_addr())
    }

    /// The session's paired devices, as `GET /api/devices` lists them.
    pub async fn devices(&self, session: &Session) -> Vec<DeviceDto> {
        let answer = self.get("/api/devices", Some(session)).await;
        assert_eq!(answer.status, StatusCode::OK, "{}", String::from_utf8_lossy(&answer.body));
        answer.json::<Devices>().devices
    }

    /// Whether the session's device `id` is online, as the device list says.
    pub async fn online(&self, session: &Session, id: &str) -> bool {
        self.devices(session)
            .await
            .iter()
            .any(|device| device.id.as_str() == id && device.online)
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
        let runner = RunnerProcess::start(
            &self.url,
            RunnerProcessOptions {
                name: name.into(),
                ..RunnerProcessOptions::default()
            },
        );
        let code = runner.pairing_code(0).await;
        let claimed = self.post("/api/devices/claim", Some(session), json!({ "code": code })).await;
        assert_eq!(claimed.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&claimed.body));
        let device = claimed.json::<ClaimedDevice>().device;
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
        assert!(tokio::time::Instant::now() < deadline, "never came true: {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

impl TestBackend {
    pub async fn send(&self, method: Method, path: &str, cookie: Option<&str>, body: Option<Value>) -> Answer {
        let mut request = self.http.request(method, format!("{}{path}", self.url));
        if let Some(cookie) = cookie {
            request = request.header(COOKIE, cookie);
        }
        if let Some(body) = body {
            request = request.header("content-type", "application/json").body(body.to_string());
        }
        answer(request.send().await.unwrap()).await
    }

    pub async fn get(&self, path: &str, session: Option<&Session>) -> Answer {
        self.send(Method::GET, path, session.map(|session| session.cookie.as_str()), None)
            .await
    }

    pub async fn post(&self, path: &str, session: Option<&Session>, body: Value) -> Answer {
        self.send(Method::POST, path, session.map(|session| session.cookie.as_str()), Some(body))
            .await
    }

    pub async fn patch(&self, path: &str, session: &Session, body: Value) -> Answer {
        self.send(Method::PATCH, path, Some(&session.cookie), Some(body)).await
    }

    pub async fn put(&self, path: &str, session: &Session, body: Value) -> Answer {
        self.send(Method::PUT, path, Some(&session.cookie), Some(body)).await
    }

    pub async fn delete(&self, path: &str, session: &Session) -> Answer {
        self.send(Method::DELETE, path, Some(&session.cookie), None).await
    }

    /// A GET with the session's cookie and extra headers.
    pub async fn get_with(&self, path: &str, session: &Session, headers: &[(&str, &str)]) -> Answer {
        answer(self.response(Method::GET, path, session, headers, None).await).await
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
            .post("/api/setup", None, json!({ "email": MASTER_EMAIL, "password": MASTER_PASSWORD }))
            .await;
        assert_eq!(answer.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&answer.body));
        session_from(&answer)
    }

    pub async fn login(&self, email: &str, password: &str) -> Session {
        let answer = self
            .post("/api/auth/login", None, json!({ "email": email, "password": password }))
            .await;
        assert_eq!(answer.status, StatusCode::OK, "{}", String::from_utf8_lossy(&answer.body));
        session_from(&answer)
    }

    pub async fn login_answer(&self, email: &str, password: &str) -> Answer {
        self.post("/api/auth/login", None, json!({ "email": email, "password": password }))
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
