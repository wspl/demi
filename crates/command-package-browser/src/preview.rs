//! The web preview's stream, `browser.preview` (`preview.md` § The stream):
//! one engine for every conversation of the Host, opened with the first
//! stream, whose cookie jar this program keeps in the data directory its
//! runner instance names (`native-runtime.md` § Invoke and retire a
//! service). Page states move over the stream between the jar and the
//! stream's conversation's browser (`preview.md` § Page state), and
//! `browser.handover` opens the tab of the agent's browser one of them was
//! kept for.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use demi_command_package_browser_chrome::page::state::PageCookie;
use demi_command_package_browser_preview::{CookieSameSite, Engine, JarCookie, PageStates, StreamError, TakenState};
use demi_command_package_browser_protocol::preview::{HandoverInput, PageStorage};
use demi_command_protocol::{CommandError, Completion};
use futures_util::future::BoxFuture;
use url::Url;

use crate::conversations::Conversations;
use demi_command_sdk::{InvocationContext, ServiceError};
use tokio::sync::OnceCell;
use tokio_util::sync::CancellationToken;
use tokio_util::task::{AbortOnDropHandle, TaskTracker};

/// The jar's file in the data directory.
const JAR_FILE: &str = "preview-cookies.json";
/// How long a page state the user's browser handed over waits for its tab
/// of the agent's browser.
const KEPT_FOR: Duration = Duration::from_secs(120);
/// How long `browser.handover` waits for its page state, which the user's
/// browser reads while the tab already shows it opening; after that the tab
/// opens its address alone.
const KEEP_WAIT: Duration = Duration::from_secs(10);

/// A page state of the user's browser, until `browser.handover` takes it.
struct Kept {
    sites: Vec<String>,
    storage: Option<PageStorage>,
    at: Instant,
}

/// The page states kept, by token, and the tabs of the agent's browser that
/// wait for theirs.
#[derive(Default)]
struct KeptStates {
    states: Mutex<HashMap<String, Kept>>,
    arrived: tokio::sync::Notify,
}

impl KeptStates {
    fn keep(&self, token: String, kept: Kept) {
        let mut states = self.states.lock().expect("the kept page states");
        states.retain(|_, state| state.at.elapsed() < KEPT_FOR);
        states.insert(token, kept);
        drop(states);
        self.arrived.notify_waiters();
    }

    /// The page state kept under `token`, waiting up to [`KEEP_WAIT`] for it
    /// to arrive; none after.
    async fn take(&self, token: &str) -> Option<Kept> {
        let deadline = tokio::time::Instant::now() + KEEP_WAIT;
        loop {
            // Listening before looking, so a state kept in between wakes the wait.
            let arrived = self.arrived.notified();
            tokio::pin!(arrived);
            arrived.as_mut().enable();
            let kept = {
                let mut states = self.states.lock().expect("the kept page states");
                states.retain(|_, state| state.at.elapsed() < KEPT_FOR);
                states.remove(token)
            };
            if kept.is_some() {
                return kept;
            }
            if tokio::time::timeout_at(deadline, arrived).await.is_err() {
                return None;
            }
        }
    }
}

/// The preview engine of the program and its open streams.
pub(crate) struct Previews {
    /// Where the engine keeps its jar; none when the runner named no data
    /// directory.
    directory: Option<PathBuf>,
    engine: OnceCell<Arc<Engine>>,
    kept: Arc<KeptStates>,
    streams: TaskTracker,
    stopping: CancellationToken,
}

impl Previews {
    pub fn new(directory: Option<PathBuf>) -> Self {
        Self {
            directory,
            engine: OnceCell::new(),
            kept: Arc::default(),
            streams: TaskTracker::new(),
            stopping: CancellationToken::new(),
        }
    }

    async fn engine(&self) -> Result<Arc<Engine>, ServiceError> {
        self.engine
            .get_or_try_init(|| async {
                let directory = self
                    .directory
                    .as_ref()
                    .ok_or_else(|| ServiceError::failed(std::io::Error::other("the runner named no data directory for the preview's cookie jar")))?;
                Engine::open(directory.join(JAR_FILE)).map_err(ServiceError::failed)
            })
            .await
            .cloned()
    }

    /// Serves one stream until the page ends it, the invocation is
    /// cancelled, or the program stops; its page states are the
    /// conversation's browser's in `browsers`.
    pub async fn serve(&self, context: InvocationContext, browsers: Arc<Conversations>) -> Result<Completion, ServiceError> {
        let engine = self.engine().await?;
        let states = Arc::new(BrowserStates {
            conversation: context.request.context.conversation.clone(),
            browsers,
            engine: engine.clone(),
            kept: self.kept.clone(),
        });
        let _open = self.streams.token();
        let InvocationContext {
            input,
            output,
            cancellation,
            ..
        } = context;
        let stop = self.stopping.child_token();
        let _cancelled = AbortOnDropHandle::new(tokio::spawn({
            let (cancellation, stop) = (cancellation.clone(), stop.clone());
            async move {
                cancellation.cancelled().await;
                stop.cancel();
            }
        }));
        let input = Box::pin(futures_util::stream::unfold(input, |mut input| async move {
            match input.next().await {
                Ok(Some(bytes)) => Some((Ok(bytes), input)),
                Ok(None) => None,
                Err(error) => Some((Err(std::io::Error::other(error)), input)),
            }
        }));
        let output = Box::pin(futures_util::sink::unfold(output, |output, bytes: Bytes| async move {
            output.stdout(bytes).await.map_err(std::io::Error::other)?;
            Ok::<_, std::io::Error>(output)
        }));
        let result = demi_command_package_browser_preview::serve(engine, states, input, output, stop).await;
        if cancellation.is_cancelled() {
            return Err(ServiceError::Cancelled);
        }
        match result {
            Ok(()) => Ok(Completion {
                exit_code: 0,
                error: None,
            }),
            Err(StreamError::Protocol(message)) => Ok(Completion {
                exit_code: 2,
                error: Some(CommandError {
                    code: "invalid_input".into(),
                    message,
                }),
            }),
            Err(error) => Err(ServiceError::failed(error)),
        }
    }

    /// `browser.handover`: a tab of the conversation's browser on the
    /// address, with the page state kept under the input's token: the jar's
    /// cookies of its sites and its storage. The user's browser keeps it
    /// while the tab already shows it opening, so it may arrive after this
    /// asks; one that does not arrive in [`KEEP_WAIT`], or waited too long
    /// before, opens the address alone.
    pub async fn handover(
        &self,
        context: InvocationContext,
        input: &HandoverInput,
        browsers: Arc<Conversations>,
    ) -> Result<Completion, ServiceError> {
        let engine = self.engine().await?;
        let kept = self.kept.take(&input.state).await;
        let (cookies, storage) = match kept {
            Some(Kept { sites, storage, .. }) => {
                let addresses: Vec<Url> = sites.iter().filter_map(|site| Url::parse(site).ok()).collect();
                let cookies = engine.cookies(&addresses).into_iter().map(page_cookie).collect();
                (cookies, storage)
            }
            None => (Vec::new(), None),
        };
        browsers.handover(context, &input.url, input.mobile, cookies, storage.as_ref()).await
    }

    /// Ends every stream, then writes the jar's latest changes.
    pub async fn close(&self) -> Result<(), ServiceError> {
        self.stopping.cancel();
        self.streams.close();
        self.streams.wait().await;
        match self.engine.get() {
            Some(engine) => engine.close().await.map_err(ServiceError::failed),
            None => Ok(()),
        }
    }
}

/// `browser.preview_open`: the top-level label of the address the user
/// opens, as JSON on standard output.
pub(crate) async fn open(
    context: InvocationContext,
    input: &demi_command_package_browser_protocol::preview::PreviewOpenInput,
) -> Result<Completion, ServiceError> {
    match demi_command_package_browser_preview::opening(input) {
        Ok(opened) => {
            let json = serde_json::to_vec(&opened).map_err(ServiceError::failed)?;
            context.output.stdout(Bytes::from(json)).await?;
            Ok(Completion {
                exit_code: 0,
                error: None,
            })
        }
        Err(message) => Ok(Completion {
            exit_code: 2,
            error: Some(CommandError {
                code: "invalid_input".into(),
                message,
            }),
        }),
    }
}

/// The stream's conversation's browser, as page states move.
struct BrowserStates {
    conversation: String,
    browsers: Arc<Conversations>,
    engine: Arc<Engine>,
    kept: Arc<KeptStates>,
}

impl PageStates for BrowserStates {
    fn take(&self, tab: String) -> BoxFuture<'static, Result<TakenState, String>> {
        let (conversation, browsers, engine) = (self.conversation.clone(), self.browsers.clone(), self.engine.clone());
        Box::pin(async move {
            let state = browsers.page_state(&conversation, &tab).await?;
            let storage = state.storage.ok_or("The page has no web address to open.")?;
            let cookies: Vec<(Url, String)> = state.cookies.iter().filter_map(set_cookie).collect();
            engine.take_cookies(cookies.iter().map(|(url, line)| (url, line.as_str())));
            Ok(TakenState {
                url: state.url,
                title: state.title,
                mobile: state.mobile,
                storage,
            })
        })
    }

    fn keep(&self, token: String, sites: Vec<String>, storage: Option<PageStorage>) {
        self.kept.keep(token, Kept { sites, storage, at: Instant::now() });
    }
}

/// A cookie of the agent's browser as a `Set-Cookie` value, with the address
/// its site would set it from, `HttpOnly` included; none for a domain no
/// address can name.
fn set_cookie(cookie: &PageCookie) -> Option<(Url, String)> {
    let scheme = if cookie.secure { "https" } else { "http" };
    let url = Url::parse(&format!("{scheme}://{}{}", cookie.domain, cookie.path)).ok()?;
    let mut line = format!("{}={}; Path={}", cookie.name, cookie.value, cookie.path);
    if !cookie.host_only {
        line.push_str(&format!("; Domain={}", cookie.domain));
    }
    if let Some(expires) = cookie.expires {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs_f64();
        line.push_str(&format!("; Max-Age={}", (expires - now).max(0.0).floor()));
    }
    if cookie.http_only {
        line.push_str("; HttpOnly");
    }
    if cookie.secure {
        line.push_str("; Secure");
    }
    if let Some(same_site) = cookie.same_site {
        line.push_str(&format!("; SameSite={same_site}"));
    }
    Some((url, line))
}

/// A cookie of the jar as the agent's browser takes it.
fn page_cookie(cookie: JarCookie) -> PageCookie {
    PageCookie {
        name: cookie.name,
        value: cookie.value,
        domain: cookie.domain,
        host_only: cookie.host_only,
        path: cookie.path,
        secure: cookie.secure,
        http_only: cookie.http_only,
        same_site: cookie.same_site.map(|same_site| match same_site {
            CookieSameSite::Strict => "Strict",
            CookieSameSite::Lax => "Lax",
            CookieSameSite::None => "None",
        }),
        expires: cookie.expires.map(|expires| expires as f64),
    }
}
