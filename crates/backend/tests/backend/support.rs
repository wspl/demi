//! The test backend: a data directory, a clock the test moves, a mailbox
//! that captures verification codes, and an HTTP client that sends a
//! session's cookie.

use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use demi_backend::{
    AccountMail, Backend, BackendConfig, FamilyRegistry, LoginTiming, MailError, RunnerTuning, VerificationMail,
};
use demi_core::Clock;
use demi_host_remote::testing::{RunnerProcess, RunnerProcessOptions};
use demi_web_api::auth::{Identity, Role, UserDto};
use demi_web_api::devices::{ClaimedDevice, DeviceDto, Devices};
use demi_web_api::error::{ErrorBody, ErrorCode};
use demi_web_api::settings::InstanceMode;
use jiff::{SignedDuration, Timestamp};
use reqwest::header::{COOKIE, HeaderMap, SET_COOKIE};
use reqwest::{Method, StatusCode};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

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
    logins: LoginTiming,
    pub runners: RunnerTuning,
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
            logins: LoginTiming::default(),
            // Liveness would ping every 30 s; the tests end runners
            // themselves.
            runners: RunnerTuning {
                ping: None,
                ..RunnerTuning::default()
            },
        }
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
        let mut config = BackendConfig::new(self.data_dir(), address, mode);
        config.clock = self.clock.clone();
        config.web_directory = self.web_directory.clone();
        config.families = self.families.clone();
        config.logins = self.logins;
        config.runners = self.runners;
        if let Some(url) = &self.models_dev_url {
            config.models_dev_url = url.parse().unwrap();
        }
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

    pub fn address(&self) -> SocketAddr {
        self.backend.local_addr()
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

    /// A runner of a new device named `name`, started, claimed by `session`
    /// and online.
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

    /// The device token the runner stored once it was claimed; it stores it
    /// a moment after the backend bound it.
    pub async fn token(&self) -> String {
        let path = self.runner.state_dir().join("runner-token");
        eventually("the runner stores its token", || {
            let stored = path.exists();
            async move { stored }
        })
        .await;
        std::fs::read_to_string(path).unwrap().trim().to_owned()
    }
}

/// Waits until `check` holds, asking every 20 ms for at most 20 s.
pub async fn eventually<F, Fut>(what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
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
        let mut request = self
            .http
            .get(format!("{}{path}", self.url))
            .header(COOKIE, &session.cookie);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        answer(request.send().await.unwrap()).await
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
