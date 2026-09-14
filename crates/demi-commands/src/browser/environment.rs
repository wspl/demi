use std::{
    collections::HashMap,
    future::Future,
    path::PathBuf,
    sync::{Arc, Weak},
    time::Duration,
};

use chromiumoxide::{
    Browser, BrowserConfig,
    cdp::browser_protocol::target::{GetTargetInfoParams, TargetId},
    handler::viewport::Viewport,
};
use futures_util::StreamExt;
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

/// The caller supplies the installed, verified release executable, never a PATH lookup.
pub struct LaunchOptions {
    pub executable: PathBuf,
}

#[derive(Clone)]
pub struct BrowserEnvironment {
    browser: Weak<Mutex<Browser>>,
    ended: CancellationToken,
    tabs: Arc<Mutex<HashMap<TargetId, BrowserTab>>>,
    generation: Arc<str>,
    failure: watch::Receiver<Option<String>>,
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
    let (failure, failure_state) = watch::channel(None);
    let environment = BrowserEnvironment {
        browser: Arc::downgrade(&browser),
        ended: CancellationToken::new(),
        tabs: Arc::new(Mutex::new(HashMap::new())),
        generation: uuid::Uuid::new_v4().simple().to_string().into(),
        failure: failure_state,
        report_failure: failure.clone(),
        observers: TaskTracker::new(),
    };
    let ended = environment.ended.clone();
    let _end_on_drop = ended.clone().drop_guard();
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
            failure.send_replace(Some(message));
        }
        ended.cancel();
        result
    }));
    let outcome = tokio::select! {
        biased;
        _ = stop.cancelled() => Err(BrowserError::Cancelled),
        _ = environment.ended.cancelled() => Err(environment.failure().map(BrowserError::Connection).unwrap_or(BrowserError::Closed)),
        result = work(environment.clone()) => result,
    };
    environment.ended.cancel();
    environment.observers.close();
    environment.observers.wait().await;
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
    pub(super) fn failure(&self) -> Option<String> {
        self.failure.borrow().clone()
    }
    pub async fn open(
        &self,
        url: &str,
        cancellation: &CancellationToken,
        timeout: Duration,
    ) -> Result<BrowserTab> {
        self.open_for(url, "native", cancellation, timeout).await
    }

    pub(super) async fn open_for(
        &self,
        url: &str,
        caller: &str,
        cancellation: &CancellationToken,
        timeout: Duration,
    ) -> Result<BrowserTab> {
        let operation = Operation::new(&self.ended, cancellation, timeout);
        operation
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
                let created_by = serde_json::from_value(
                    serde_json::json!({ "kind": "agent", "nodeId": caller }),
                )
                .map_err(|error| BrowserError::Configuration(error.to_string()))?;
                let tab = self.tab(&mut registry, page, Some(created_by)).await?;
                tab.page.goto(url).await?;
                Ok(tab)
            })
            .await
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
                registry.retain(|target, _| live.contains(target));
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
        tabs: &mut HashMap<TargetId, BrowserTab>,
        page: chromiumoxide::Page,
        created_by: Option<BrowserCreatedBy>,
    ) -> Result<BrowserTab> {
        if let Some(tab) = tabs.get(page.target_id()) {
            return Ok(tab.clone());
        }
        let created_by = match created_by {
            Some(value) => value,
            None => {
                let info = page
                    .execute(
                        GetTargetInfoParams::builder()
                            .target_id(page.target_id().clone())
                            .build(),
                    )
                    .await?
                    .result
                    .target_info;
                let opener = info.opener_id.ok_or_else(|| {
                    BrowserError::InvalidResult("unregistered browser page has no opener".into())
                })?;
                serde_json::from_value(serde_json::json!({ "kind": "page", "opener": format!("{}-{}", self.generation, opener.as_ref()) }))
                    .map_err(|error| BrowserError::InvalidResult(error.to_string()))?
            }
        };
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
            self.generation.clone(),
            created_by,
        );
        tabs.insert(tab.page.target_id().clone(), tab.clone());
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
