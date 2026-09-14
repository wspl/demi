use std::{
    collections::HashMap,
    future::Future,
    path::PathBuf,
    sync::{Arc, Weak},
    time::Duration,
};

use chromiumoxide::{
    Browser, BrowserConfig,
    cdp::browser_protocol::target::{CreateTargetParams, TargetId},
    handler::viewport::Viewport,
};
use futures_util::StreamExt;
use tokio::sync::Mutex;
use tokio_util::{sync::CancellationToken, task::AbortOnDropHandle};

use super::{
    BrowserError, BrowserTab, Result,
    operation::{CONTROL_TIMEOUT, Operation, after_cleanup},
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
    tab_locks: Arc<Mutex<HashMap<TargetId, Weak<TabState>>>>,
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
        .chrome_executable(options.executable)
        .user_data_dir(profile.path())
        .arg("no-startup-window")
        .viewport(Viewport {
            width: 1280,
            height: 720,
            ..Viewport::default()
        })
        .request_timeout(Duration::from_secs(30))
        .build()
        .map_err(BrowserError::Configuration)?;
    let (browser, mut handler) = tokio::select! {
        biased;
        _ = stop.cancelled() => return Err(BrowserError::Cancelled),
        result = Browser::launch(config) => result?,
    };
    let browser = Arc::new(Mutex::new(browser));
    let environment = BrowserEnvironment {
        browser: Arc::downgrade(&browser),
        ended: CancellationToken::new(),
        tab_locks: Arc::new(Mutex::new(HashMap::new())),
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
        ended.cancel();
        result
    }));
    let outcome = tokio::select! {
        biased;
        _ = stop.cancelled() => Err(BrowserError::Cancelled),
        _ = environment.ended.cancelled() => Err(BrowserError::Closed),
        result = work(environment.clone()) => result,
    };
    environment.ended.cancel();
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
        let operation = Operation::new(&self.ended, cancellation, timeout);
        operation
            .run(async {
                let page = self
                    .browser
                    .upgrade()
                    .ok_or(BrowserError::Closed)?
                    .lock()
                    .await
                    .new_page(
                        CreateTargetParams::builder()
                            .url(url)
                            .new_window(true)
                            .build()
                            .map_err(BrowserError::Configuration)?,
                    )
                    .await?;
                Ok(self.tab(page).await)
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
                let pages = self
                    .browser
                    .upgrade()
                    .ok_or(BrowserError::Closed)?
                    .lock()
                    .await
                    .pages()
                    .await?;
                let mut tabs = Vec::with_capacity(pages.len());
                for page in pages {
                    tabs.push(self.tab(page).await);
                }
                Ok(tabs)
            })
            .await
    }

    /// Share the operation gate for every handle to the same live CDP target.
    async fn tab(&self, page: chromiumoxide::Page) -> BrowserTab {
        let mut locks = self.tab_locks.lock().await;
        locks.retain(|_, lock| lock.strong_count() != 0);
        let lock = locks
            .get(page.target_id())
            .and_then(Weak::upgrade)
            .unwrap_or_else(|| {
                let lock = Arc::new(TabState::default());
                locks.insert(page.target_id().clone(), Arc::downgrade(&lock));
                lock
            });
        BrowserTab::new(page, self.browser.clone(), self.ended.clone(), lock)
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
