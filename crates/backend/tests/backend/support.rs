//! The test backend: a data directory, a clock the test moves, a mailbox
//! that captures verification codes, and an HTTP client that sends a
//! session's cookie.

use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use demi_backend::{AccountMail, Backend, BackendConfig, Clock, MailError, VerificationMail};
use demi_web_api::auth::{Identity, UserDto};
use demi_web_api::error::{ErrorBody, ErrorCode};
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
    fn now(&self) -> Timestamp {
        *self.0.lock().unwrap()
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
        }
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
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
        let mut config = BackendConfig::new(self.data.path().join("backend"), address);
        config.clock = self.clock.clone();
        config.web_directory = self.web_directory.clone();
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
