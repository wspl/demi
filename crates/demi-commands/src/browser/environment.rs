use std::{
    collections::HashMap,
    future::Future,
    path::PathBuf,
    sync::{Arc, Weak},
    time::Duration,
};

use chromiumoxide::{
    Browser, BrowserConfig,
    cdp::browser_protocol::target::{EventTargetCreated, TargetId},
    handler::viewport::Viewport,
};
use futures_util::{FutureExt, StreamExt};
use tokio::sync::{Mutex, watch};
use tokio_util::{
    sync::CancellationToken,
    task::{AbortOnDropHandle, TaskTracker},
};

use super::{
    BrowserError, BrowserTab, Result,
    operation::{CONTROL_TIMEOUT, Operation, after_cleanup},
    protocol::BrowserCreatedBy,
    tab::TabState,
};

struct TabRegistry {
    created: chromiumoxide::listeners::EventStream<EventTargetCreated>,
    openers: HashMap<TargetId, TargetId>,
    live: HashMap<TargetId, BrowserTab>,
    public_ids: HashMap<TargetId, Arc<str>>,
}

impl TabRegistry {
    /// Consume creation metadata before reconciling live pages, even if an opener closed.
    fn observe_targets(&mut self) -> Result<()> {
        while let Some(event) = self.created.next().now_or_never() {
            let event = event.ok_or(BrowserError::Closed)??;
            let info = &event.target_info;
            self.public_id(&info.target_id)?;
            if let Some(opener) = &info.opener_id {
                self.public_id(opener)?;
                self.openers.insert(info.target_id.clone(), opener.clone());
            }
        }
        Ok(())
    }

    /// Retain browser target identities even after their live tab is pruned.
    fn public_id(&mut self, target: &TargetId) -> Result<Arc<str>> {
        if let Some(id) = self.public_ids.get(target) {
            return Ok(id.clone());
        }
        let id: Arc<str> = super::handles::fresh("t")?.into();
        self.public_ids.insert(target.clone(), id.clone());
        Ok(id)
    }
}

/// The caller supplies the installed, verified release executable, never a PATH lookup.
pub struct LaunchOptions {
    pub executable: PathBuf,
}

#[derive(Clone)]
pub struct BrowserEnvironment {
    browser: Weak<Mutex<Browser>>,
    ended: CancellationToken,
    tabs: Arc<Mutex<TabRegistry>>,
    report_failure: watch::Sender<Option<String>>,
    observers: TaskTracker,
}

/// Own Chrome, its event task and its profile until the retained-resource work ends.
/// Completion, failure and owner cancellation share the same joined cleanup path.
/// Cancel through `stop` and await this owner. On abrupt future disposal the child
/// has kill-on-drop protection; weak session handles cannot keep Chrome alive.
pub async fn with_browser<T, F, W>(
    options: LaunchOptions,
    stop: CancellationToken,
    work: F,
) -> Result<T>
where
    F: FnOnce(BrowserEnvironment) -> W,
    W: Future<Output = Result<T>>,
{
    if !options.executable.is_absolute() {
        return Err(BrowserError::Configuration(
            "Chrome executable must be absolute".into(),
        ));
    }
    let profile = tempfile::Builder::new().prefix("demi-browser-").tempdir()?;
    let config = BrowserConfig::builder()
        .respect_https_errors()
        .surface_invalid_messages()
        .chrome_executable(options.executable)
        .user_data_dir(profile.path())
        .arg("no-startup-window")
        // Headless automation has no browser toolbar or omnibox. Avoid their
        // WebUI renderers and preload work, including Chrome's overhead trial.
        .arg((
            "disable-features",
            "InitialWebUI,WebUIToolbarProcessOverheadExperiment,PreloadTopChromeWebUI,WebUIOmniboxPopup,WebUIOmniboxAimPopup",
        ))
        .viewport(Viewport {
            width: 1280,
            height: 720,
            ..Viewport::default()
        })
        .launch_timeout(Duration::from_secs(60))
        .request_timeout(Duration::from_secs(30))
        .build()
        .map_err(BrowserError::Configuration)?;
    let (browser, mut handler) = tokio::select! {
        biased;
        _ = stop.cancelled() => return Err(BrowserError::Cancelled),
        result = Browser::launch(config) => result?,
    };
    let browser = Arc::new(Mutex::new(browser));
    let (failure, _) = watch::channel(None);
    let ended = CancellationToken::new();
    let observers = TaskTracker::new();
    let _end_on_drop = ended.clone().drop_guard();
    let pump_ended = ended.clone();
    let pump_failure = failure.clone();
    let pump = AbortOnDropHandle::new(tokio::spawn(async move {
        let result = async {
            while let Some(event) = handler.next().await {
                event?;
            }
            Ok::<_, chromiumoxide::error::CdpError>(())
        }
        .await;
        if let Err(error) = &result {
            let message = match error {
                chromiumoxide::error::CdpError::InvalidMessage(message, source) => {
                    let diagnostic =
                        serde_json::from_str::<chromiumoxide::cdp::CdpEventMessage>(message)
                            .err()
                            .map(|error| error.to_string())
                            .unwrap_or_else(|| source.to_string());
                    format!("browser protocol decoding failed: {diagnostic}")
                }
                _ => format!("browser connection ended: {error}"),
            };
            pump_failure.send_replace(Some(message));
        }
        pump_ended.cancel();
        result
    }));
    let outcome = tokio::select! {
        biased;
        _ = stop.cancelled() => Err(BrowserError::Cancelled),
        _ = ended.cancelled() => Err(failure.borrow().clone().map(BrowserError::Connection).unwrap_or(BrowserError::Closed)),
        result = async {
            // Subscribe before any caller can create a target. Chrome may discard
            // openerId from later TargetInfo responses after the opener closes.
            let created = browser.lock().await.event_listener::<EventTargetCreated>().await?;
            let environment = BrowserEnvironment {
                browser: Arc::downgrade(&browser),
                ended: ended.clone(),
                tabs: Arc::new(Mutex::new(TabRegistry {
                    live: HashMap::new(),
                    public_ids: HashMap::new(),
                    openers: HashMap::new(),
                    created,
                })),
                report_failure: failure.clone(),
                observers: observers.clone(),
            };
            work(environment).await
        } => result,
    };
    ended.cancel();
    observers.close();
    observers.wait().await;
    let cleanup = match retire_browser(&browser, pump).await {
        Ok(()) => profile.close().map_err(BrowserError::from),
        Err(source) => Err(BrowserError::ProfileRetained {
            path: profile.keep(),
            source: Box::new(source),
        }),
    };
    after_cleanup(outcome, cleanup)
}

impl BrowserEnvironment {
    pub async fn open(
        &self,
        url: &str,
        cancellation: &CancellationToken,
        timeout: Duration,
    ) -> Result<BrowserTab> {
        self.open_for(
            url,
            "native",
            "domcontentloaded",
            cancellation,
            tokio::time::Instant::now() + timeout,
        )
        .await
        .map(|(tab, _)| tab)
    }

    pub(super) async fn open_for(
        &self,
        url: &str,
        caller: &str,
        load: &str,
        cancellation: &CancellationToken,
        deadline: tokio::time::Instant,
    ) -> Result<(BrowserTab, String)> {
        let operation = Operation::until(&self.ended, cancellation, deadline);
        let tab = operation
            .run(async {
                let mut registry = self.tabs.lock().await;
                let page = self
                    .browser
                    .upgrade()
                    .ok_or(BrowserError::Closed)?
                    .lock()
                    .await
                    .new_page("about:blank")
                    .await?;
                let created_by =
                    serde_json::from_value(serde_json::json!({"kind": "agent", "nodeId": caller}))
                        .map_err(|error| BrowserError::Configuration(error.to_string()))?;
                self.tab(&mut registry, page, Some(created_by)).await
            })
            .await?;
        let mut references = tab
            .state
            .operations
            .try_lock()
            .map_err(|_| BrowserError::Busy)?;
        let url = tab
            .navigate(
                super::navigation::Navigation::Url(url.into()),
                load,
                &operation,
                &mut references,
            )
            .await?;
        drop(references);
        Ok((tab, url))
    }

    pub async fn tabs(
        &self,
        cancellation: &CancellationToken,
        timeout: Duration,
    ) -> Result<Vec<BrowserTab>> {
        let operation = Operation::new(&self.ended, cancellation, timeout);
        operation
            .run(async {
                let mut registry = self.tabs.lock().await;
                let pages = self
                    .browser
                    .upgrade()
                    .ok_or(BrowserError::Closed)?
                    .lock()
                    .await
                    .pages()
                    .await?;
                let live: Vec<_> = pages.iter().map(|page| page.target_id().clone()).collect();
                registry.live.retain(|target, _| live.contains(target));
                let mut tabs = Vec::with_capacity(pages.len());
                for page in pages {
                    tabs.push(self.tab(&mut registry, page, None).await?);
                }
                Ok(tabs)
            })
            .await
    }

    /// Reconcile pages into the one registry, retaining their operation gates and refs.
    async fn tab(
        &self,
        tabs: &mut TabRegistry,
        page: chromiumoxide::Page,
        created_by: Option<BrowserCreatedBy>,
    ) -> Result<BrowserTab> {
        tabs.observe_targets()?;
        if let Some(tab) = tabs.live.get(page.target_id()) {
            return Ok(tab.clone());
        }
        let created_by = match created_by {
            Some(value) => value,
            None => {
                let opener = tabs.openers.get(page.target_id()).cloned().ok_or_else(|| {
                    BrowserError::InvalidResult(
                        "unregistered browser page has no recorded opener".into(),
                    )
                })?;
                serde_json::from_value(serde_json::json!({ "kind": "page", "opener": tabs.public_id(&opener)?.as_ref() }))
                    .map_err(|error| BrowserError::InvalidResult(error.to_string()))?
            }
        };
        let id = tabs.public_id(page.target_id())?;
        let state = TabState::observe(
            &page,
            self.ended.clone(),
            &self.observers,
            self.report_failure.clone(),
        )
        .await?;
        let tab = BrowserTab::new(
            page,
            self.browser.clone(),
            self.ended.clone(),
            state,
            id,
            created_by,
        );
        tabs.live.insert(tab.page.target_id().clone(), tab.clone());
        Ok(tab)
    }
}

/// Stop Chrome before joining the CDP pump; it must run while close is in flight.
async fn retire_browser(
    browser: &Mutex<Browser>,
    mut pump: AbortOnDropHandle<chromiumoxide::Result<()>>,
) -> Result<()> {
    let mut browser = browser.lock().await;
    let graceful = tokio::time::timeout(CONTROL_TIMEOUT, async {
        browser.close().await?;
        browser.wait().await?;
        Ok::<_, BrowserError>(())
    })
    .await;
    let process = match graceful {
        Ok(Ok(())) => Ok(()),
        _ => {
            // The connection can already be gone; killing the owned child is authoritative.
            match browser.kill().await {
                Some(result) => result.map_err(BrowserError::from),
                None => Err(BrowserError::Closed),
            }
        }
    };
    let joined = tokio::time::timeout(CONTROL_TIMEOUT, &mut pump).await;
    let task = match joined {
        Ok(result) => result.map_err(BrowserError::from).map(|_| ()),
        Err(_) => {
            pump.abort();
            match pump.await {
                Err(error) if error.is_cancelled() => Ok(()),
                Err(error) => Err(BrowserError::Task(error)),
                Ok(_) => Ok(()),
            }
        }
    };
    // A CDP error after requested shutdown is expected; child exit, not the
    // WebSocket closing handshake, establishes resource retirement.
    process?;
    task
}
