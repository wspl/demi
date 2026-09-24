use std::{
    future::Future,
    io,
    ops::Deref,
    path::PathBuf,
    sync::{Arc, Weak},
    time::Duration,
};

use chromiumoxide::{Browser, BrowserConfig, handler::viewport::Viewport};
use demi_command_service::protocol::CommandLocale;
use futures_util::StreamExt;
use tokio::sync::watch;
use tokio_util::{
    sync::CancellationToken,
    task::{AbortOnDropHandle, TaskTracker, task_tracker::TaskTrackerToken},
};

use super::{
    BrowserError, BrowserTab, Result,
    operation::{CONTROL_TIMEOUT, Operation, after_cleanup},
    process::ChromeProcess,
    protocol::{BrowserCreatedBy, Load, TabId},
    registry::{self, Hold, Snapshot, Tabs},
};

/// Chrome answers a browser call within this, or chromiumoxide fails the
/// call.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// The environment's one browser connection, which commands share without
/// taking turns (`browser.md` § Owners inside the service). The environment
/// owns the `Browser`; a handle reaches it for one call at a time, and only
/// until the environment ends, so no handle keeps Chrome running.
#[derive(Clone)]
pub(super) struct BrowserHandle {
    browser: Weak<Browser>,
    /// Browser calls in flight. Retirement waits for them before it takes
    /// the browser back to close it.
    calls: TaskTracker,
    ended: CancellationToken,
}

/// The browser, for one call.
pub(super) struct BrowserCall {
    browser: Arc<Browser>,
    _call: TaskTrackerToken,
}

impl Deref for BrowserCall {
    type Target = Browser;

    fn deref(&self) -> &Browser {
        &self.browser
    }
}

impl BrowserHandle {
    pub fn call(&self) -> Result<BrowserCall> {
        // Counted before the end is checked: retirement ends the environment
        // before it waits for calls, so it waits for every call that saw the
        // environment running.
        let call = self.calls.token();
        if self.ended.is_cancelled() {
            return Err(BrowserError::Closed);
        }
        let browser = self.browser.upgrade().ok_or(BrowserError::Closed)?;
        Ok(BrowserCall {
            browser,
            _call: call,
        })
    }

    /// Whether both reach the same browser.
    pub fn same(&self, other: &Self) -> bool {
        self.browser.ptr_eq(&other.browser)
    }
}

/// A browser profile's directory name starts with this, in the system's
/// temporary directory.
const PROFILE_PREFIX: &str = "demi-browser-";
/// The file a profile's lock is held on while its environment exists.
const PROFILE_LOCK: &str = "demi-profile.lock";

/// The caller supplies the installed, verified release executable, never a PATH lookup.
pub struct LaunchOptions {
    pub executable: PathBuf,
    /// The release's version, for the user agent pages see.
    version: String,
    /// The user's time zone and languages; they do not change while the
    /// environment lives (`browser.md` § Native driver).
    pub locale: CommandLocale,
}

impl LaunchOptions {
    /// The pinned release installed at `executable`, started in `locale`.
    pub fn pinned(executable: PathBuf, locale: CommandLocale) -> Result<Self> {
        Ok(Self {
            executable,
            version: super::installation::pinned_version()?,
            locale,
        })
    }
}

#[derive(Clone)]
pub struct BrowserEnvironment {
    pub(super) browser: BrowserHandle,
    pub(super) ended: CancellationToken,
    tabs: Tabs,
    /// Why the browser connection ended, once it ended on its own.
    report_failure: watch::Sender<Option<String>>,
    pub(super) observers: TaskTracker,
    pub(super) download_directory: PathBuf,
    /// Files the user chose in the live view, until the browser retires.
    pub(super) upload_directory: PathBuf,
    /// The live view of this browser (`live-view.md`).
    pub(super) live: Arc<super::live::Hub>,
}

/// Own Chrome, its event task and its profile until the conversation work ends.
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
    let mut profile = tempfile::Builder::new().prefix(PROFILE_PREFIX).tempdir()?;
    // Held until the profile is removed, however this service ends; a later
    // service's sweep takes it only once this one is gone.
    let profile_lock = std::fs::File::create_new(profile.path().join(PROFILE_LOCK))?;
    profile_lock.try_lock().map_err(|error| match error {
        std::fs::TryLockError::Error(error) => BrowserError::Io(error),
        std::fs::TryLockError::WouldBlock => {
            BrowserError::Io(io::Error::other("a new browser profile is already locked"))
        }
    })?;
    let mut process = ChromeProcess::new(profile.path(), &options.executable);
    let download_directory = profile.path().join("downloads");
    tokio::fs::create_dir(&download_directory).await?;
    let upload_directory = profile.path().join("uploads");
    tokio::fs::create_dir(&upload_directory).await?;
    let builder = BrowserConfig::builder()
        .respect_https_errors()
        .surface_invalid_messages()
        .chrome_executable(options.executable)
        .user_data_dir(profile.path())
        .viewport(Viewport {
            width: super::viewport::UNWATCHED.width,
            height: super::viewport::UNWATCHED.height,
            ..Viewport::default()
        })
        .launch_timeout(Duration::from_secs(60))
        .request_timeout(REQUEST_TIMEOUT);
    let capture = super::live::capture::CaptureServer::bind().await?;
    let config = super::launch::configure(
        builder,
        profile.path(),
        &options.version,
        &options.locale,
        &capture.address()?,
    )
    .await?
    .build()
    .map_err(BrowserError::Configuration)?;
    let platform_arguments = super::launch::platform_arguments(&options.locale);
    // Only joined retirement may remove a profile once Chrome could be writing.
    profile.disable_cleanup(true);
    let launched = tokio::select! {
        biased;
        _ = stop.cancelled() => Err(BrowserError::Cancelled),
        result = Browser::launch_with(config, |command| process.spawn(command.args(&platform_arguments))) => result.map_err(BrowserError::from),
    };
    let (browser, mut handler) = match launched {
        Ok(launched) => launched,
        Err(error) => {
            let cleanup = remove_profile(profile, process.terminate().await).await;
            return after_cleanup(Err(error), cleanup);
        }
    };
    let browser = Arc::new(browser);
    let (failure, _) = watch::channel(None);
    let ended = CancellationToken::new();
    let observers = TaskTracker::new();
    let calls = TaskTracker::new();
    let handle = BrowserHandle {
        browser: Arc::downgrade(&browser),
        calls: calls.clone(),
        ended: ended.clone(),
    };
    let _end_on_drop = ended.clone().drop_guard();
    let captures = super::live::capture::CaptureChannel::open(&observers, ended.clone());
    capture.serve(captures.clone(), &observers, ended.clone());
    let live = Arc::new(super::live::Hub::new(
        captures,
        observers.clone(),
        ended.clone(),
    ));
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
        } else if !pump_ended.is_cancelled() {
            // An EOF without a protocol error is still transport loss unless
            // joined retirement already ended the environment intentionally.
            pump_failure.send_replace(Some("browser connection ended".into()));
        }
        pump_ended.cancel();
        result
    }));
    let outcome = tokio::select! {
        biased;
        _ = stop.cancelled() => Err(BrowserError::Cancelled),
        _ = ended.cancelled() => Err(failure.borrow().clone().map(BrowserError::Connection).unwrap_or(BrowserError::Closed)),
        result = async {
            use chromiumoxide::cdp::browser_protocol::browser::{SetDownloadBehaviorParams, SetDownloadBehaviorBehavior};
            handle.call()?.execute(SetDownloadBehaviorParams::builder()
                .behavior(SetDownloadBehaviorBehavior::AllowAndName)
                .download_path(download_directory.to_string_lossy().into_owned())
                .events_enabled(true)
                .build().map_err(BrowserError::Configuration)?).await?;
            // Subscribed before any caller can create a target: Chrome may
            // leave a popup's opener out of later target information once
            // the opener closed.
            let tabs = registry::start(
                handle.clone(),
                ended.clone(),
                &observers,
                failure.clone(),
                live.changes(),
            )
            .await?;
            let environment = BrowserEnvironment {
                browser: handle.clone(),
                ended: ended.clone(),
                tabs,
                report_failure: failure.clone(),
                observers: observers.clone(),
                download_directory,
                upload_directory,
                live: live.clone(),
            };
            work(environment).await
        } => result,
    };
    // Every task of the environment ends with it: the tabs' debugging
    // owners close their connections as they end.
    ended.cancel();
    observers.close();
    observers.wait().await;
    // No call starts once the environment ended, and each ends within the
    // request timeout while the pump still runs; then only this owner holds
    // the browser. A call that outlived the wait leaves the browser with it,
    // and `retire_browser` ends Chrome's process tree without it.
    calls.close();
    let _outlived = tokio::time::timeout(REQUEST_TIMEOUT, calls.wait()).await;
    let retired = retire_browser(Arc::into_inner(browser), pump, &mut process).await;
    let cleanup = remove_profile(profile, retired).await;
    drop(profile_lock);
    after_cleanup(outcome, cleanup)
}

/// Removes what browsers left behind when their service ended without
/// retiring them (`browser.md` § Native driver). A profile whose lock this
/// service can take belongs to no running service: its marked Chrome
/// processes end the way retirement ends them, then the profile goes. A
/// profile whose lock is held, or that has no lock yet, is left alone.
pub async fn sweep_orphans() {
    sweep_orphans_in(&std::env::temp_dir(), super::installation::roots()).await;
}

async fn sweep_orphans_in(directory: &std::path::Path, installations: Vec<PathBuf>) {
    let Ok(mut entries) = tokio::fs::read_dir(directory).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        if !entry.file_name().to_string_lossy().starts_with(PROFILE_PREFIX) {
            continue;
        }
        let profile = entry.path();
        if let Err(error) = sweep_orphan(&profile, installations.clone()).await {
            tracing::warn!("could not remove orphaned browser profile {}: {error}", profile.display());
        }
    }
}

async fn sweep_orphan(profile: &std::path::Path, installations: Vec<PathBuf>) -> Result<()> {
    let lock = match std::fs::File::options()
        .read(true)
        .write(true)
        .open(profile.join(PROFILE_LOCK))
    {
        Ok(lock) => lock,
        // A profile being made, or one this release did not make.
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    match lock.try_lock() {
        Ok(()) => {}
        // A running service's environment holds it.
        Err(std::fs::TryLockError::WouldBlock) => return Ok(()),
        Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
    }
    ChromeProcess::marked(profile, installations).terminate().await?;
    tokio::fs::remove_dir_all(profile).await?;
    drop(lock);
    Ok(())
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
            Load::DomContentLoaded,
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
        load: Load,
        cancellation: &CancellationToken,
        deadline: tokio::time::Instant,
    ) -> Result<(BrowserTab, String)> {
        super::navigation::validate_url(url)?;
        let operation = Operation::until(&self.ended, cancellation, deadline);
        let tab = self
            .create(
                BrowserCreatedBy::Agent {
                    node_id: caller.to_owned(),
                },
                &operation,
            )
            .await?;
        let mut session = tab.state.gate.try_checkout().ok_or(BrowserError::Busy)?;
        let url = tab
            .navigate(
                super::navigation::Navigation::Url(url.into()),
                load,
                &operation,
                &mut session.references,
            )
            .await?;
        drop(session);
        Ok((tab, url))
    }

    /// A tab the user opens in the work panel, blank or loading `url`; the
    /// panel shows its loading, so opening does not wait for it.
    pub(super) async fn open_user(
        &self,
        url: Option<&str>,
        cancellation: &CancellationToken,
        deadline: tokio::time::Instant,
    ) -> Result<BrowserTab> {
        if let Some(url) = url {
            super::navigation::validate_url(url)?;
        }
        let operation = Operation::until(&self.ended, cancellation, deadline);
        let tab = self.create(BrowserCreatedBy::User {}, &operation).await?;
        if let Some(url) = url {
            super::navigation::visit(&tab, url);
        }
        Ok(tab)
    }

    /// Creates `count` temporary tabs, and keeps the environment from
    /// retiring until the batch is closed.
    pub(super) async fn temporary_tabs(
        &self,
        caller: &str,
        count: usize,
        cancel: &CancellationToken,
        deadline: tokio::time::Instant,
    ) -> Result<TemporaryBatch> {
        let operation = Operation::until(&self.ended, cancel, deadline);
        let hold = operation.run(self.tabs.hold()).await?;
        let mut batch = TemporaryBatch {
            tabs: Vec::with_capacity(count),
            hold,
            registry: self.tabs.clone(),
        };
        let result = async {
            for _ in 0..count {
                let created_by = BrowserCreatedBy::Temporary {
                    node_id: caller.to_owned(),
                };
                batch.tabs.push(self.create(created_by, &operation).await?);
            }
            Ok(())
        }
        .await;
        if let Err(error) = result {
            return after_cleanup(Err(error), batch.close(CONTROL_TIMEOUT).await);
        }
        Ok(batch)
    }

    /// A new tab for `operation`. A creation Chrome began finishes even when
    /// the operation ends first, and its tab closes again before the
    /// operation's end is reported, so a later listing does not show it.
    async fn create(&self, created_by: BrowserCreatedBy, operation: &Operation<'_>) -> Result<BrowserTab> {
        let mut creation = operation.run(self.tabs.create(created_by)).await?;
        match operation.run(creation.tab()).await {
            Ok(tab) => Ok(tab),
            Err(error) => {
                let cleanup = tokio::time::timeout(CONTROL_TIMEOUT, async {
                    let tab = creation.tab().await?;
                    tab.close(&CancellationToken::new(), CONTROL_TIMEOUT).await
                })
                .await
                .map_err(|_| BrowserError::Timeout)
                .and_then(std::convert::identity);
                after_cleanup(Err(error), cleanup)
            }
        }
    }

    /// Snapshot another caller's debug ownership without issuing browser commands.
    pub(super) fn debugging_callers(&self, id: &str, caller: Option<&str>) -> Vec<String> {
        self.tabs
            .latest()
            .tabs
            .iter()
            .map(|listed| &listed.tab)
            .find(|tab| tab.id().as_str() == id && !tab.ended.is_cancelled())
            .map_or_else(Vec::new, |tab| tab.state.debug.other_callers(caller))
    }

    pub async fn tabs(
        &self,
        cancellation: &CancellationToken,
        timeout: Duration,
    ) -> Result<Vec<BrowserTab>> {
        Ok(self
            .listed(cancellation, timeout)
            .await?
            .tabs
            .iter()
            .map(|listed| listed.tab.clone())
            .collect())
    }

    /// The tabs in the order the browser created them, with their titles and
    /// URLs, as the registry lists them.
    pub(super) async fn listed(
        &self,
        cancellation: &CancellationToken,
        timeout: Duration,
    ) -> Result<Arc<Snapshot>> {
        Operation::new(&self.ended, cancellation, timeout)
            .run(self.tabs.listing())
            .await
    }

    /// The tab with public ID `id`, from the registry's snapshot.
    pub(super) async fn tab(
        &self,
        id: &TabId,
        cancellation: &CancellationToken,
        timeout: Duration,
    ) -> Result<BrowserTab> {
        Operation::new(&self.ended, cancellation, timeout)
            .run(self.tabs.find(id))
            .await
    }

    /// Cancelled once the last tab has closed: the environment admits no new
    /// tab, and its owner retires it.
    pub(super) fn emptied(&self) -> &CancellationToken {
        self.tabs.emptied()
    }

    /// Why the browser connection ended on its own, if it did.
    pub(super) fn failure(&self) -> Option<String> {
        self.report_failure.borrow().clone()
    }

    /// Whether both are the same browser generation.
    pub(super) fn same(&self, other: &Self) -> bool {
        self.browser.same(&other.browser)
    }
}

/// Temporary tabs in use; until they are closed, the environment does not
/// retire.
pub(super) struct TemporaryBatch {
    tabs: Vec<BrowserTab>,
    hold: Hold,
    registry: Tabs,
}

impl TemporaryBatch {
    pub fn tabs(&self) -> &[BrowserTab] {
        &self.tabs
    }

    /// Closes the batch's tabs and ends its hold. Once this returns, the
    /// environment has emptied if nothing else holds it.
    pub async fn close(self, timeout: Duration) -> Result<()> {
        let mut cleanup = Ok(());
        let cancel = CancellationToken::new();
        for tab in &self.tabs {
            let result = tab.close(&cancel, timeout).await;
            let result = match result {
                // Explicit user closure already released this temporary tab.
                Err(BrowserError::TabNotFound) => Ok(()),
                result => result,
            };
            cleanup = after_cleanup(cleanup, result);
        }
        drop(self.hold);
        let settled = match self.registry.settled().await {
            // An environment that ended has no hold left to settle.
            Err(BrowserError::Closed) => Ok(()),
            settled => settled.map(|_| ()),
        };
        after_cleanup(cleanup, settled)
    }
}

/// Stop Chrome before joining the CDP pump; it must run while close is in flight.
/// Without the browser, because a call still holds it, Chrome's process
/// tree ends without a graceful close: `ChromeProcess` stays authoritative.
async fn retire_browser(
    browser: Option<Browser>,
    mut pump: AbortOnDropHandle<chromiumoxide::Result<()>>,
    process: &mut ChromeProcess,
) -> Result<()> {
    if let Err(error) = process.observe().await {
        // The drain reads the process table again as it signals.
        tracing::warn!("Chrome's helpers were not observed before retirement: {error}");
    }
    let leader = match browser {
        Some(mut browser) => {
            let graceful = tokio::time::timeout(CONTROL_TIMEOUT, async {
                browser.close().await?;
                browser.wait().await?;
                Ok::<_, BrowserError>(())
            })
            .await;
            match graceful {
                Ok(Ok(())) => Ok(()),
                _ => {
                    // The connection can already be gone; killing the owned child is authoritative.
                    match tokio::time::timeout(CONTROL_TIMEOUT, browser.kill()).await {
                        Ok(Some(result)) => result.map_err(BrowserError::from),
                        Ok(None) => Err(BrowserError::Closed),
                        Err(_) => Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "Chrome leader did not exit",
                        )
                        .into()),
                    }
                }
            }
        }
        None => {
            tracing::warn!("a browser call outlived its environment; Chrome's process tree ends without closing it");
            Ok(())
        }
    };
    let group = process.terminate().await;
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
    leader?;
    group?;
    task
}

/// Remove Chrome's profile only after retirement, briefly retrying concurrent directory updates.
async fn remove_profile(profile: tempfile::TempDir, retired: Result<()>) -> Result<()> {
    let path = profile.keep();
    let result = async {
        retired?;
        let deadline = tokio::time::Instant::now() + Duration::from_millis(300);
        loop {
            match tokio::fs::remove_dir_all(&path).await {
                Ok(()) => return Ok(()),
                Err(error)
                    if error.kind() == io::ErrorKind::DirectoryNotEmpty
                        && tokio::time::Instant::now() < deadline =>
                {
                    tokio::time::sleep_until(
                        deadline.min(tokio::time::Instant::now() + Duration::from_millis(50)),
                    )
                    .await;
                }
                Err(error) => return Err(BrowserError::from(error)),
            }
        }
    }
    .await;
    result.map_err(|source| BrowserError::ProfileRetained {
        path,
        source: Box::new(source),
    })
}

#[cfg(test)]
mod tests {
    use chromiumoxide::cdp::browser_protocol::target::GetTargetsParams;

    use super::*;

    #[tokio::test]
    #[ignore = "requires DEMI_TEST_CHROME pointing to an installed Chrome for Testing release"]
    async fn the_capture_extension_runs_beside_the_pages_and_is_never_a_tab() {
        let executable =
            PathBuf::from(std::env::var_os("DEMI_TEST_CHROME").expect("DEMI_TEST_CHROME"));
        let locale = CommandLocale {
            time_zone: "UTC".into(),
            languages: vec!["en-US".into()],
        };
        let timeout = Duration::from_secs(60);
        with_browser(
            LaunchOptions::pinned(executable, locale).unwrap(),
            CancellationToken::new(),
            |environment| async move {
                let tab = environment
                    .open("about:blank", &CancellationToken::new(), timeout)
                    .await?;
                let prefix = format!(
                    "chrome-extension://{}/",
                    super::super::launch::CAPTURE_EXTENSION_ID
                );
                let mut loaded = false;
                for _ in 0..100 {
                    let targets = environment
                        .browser
                        .call()?
                        .execute(GetTargetsParams::default())
                        .await?
                        .result
                        .target_infos;
                    if targets.iter().any(|target| target.url.starts_with(&prefix)) {
                        loaded = true;
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                assert!(loaded, "the capture extension never ran");
                let tabs = environment.tabs(&CancellationToken::new(), timeout).await?;
                assert_eq!(
                    tabs.iter().map(BrowserTab::id).collect::<Vec<_>>(),
                    vec![tab.id()]
                );
                Ok(())
            },
        )
        .await
        .unwrap();
    }

    /// A profile nobody holds goes; a held one and one without a lock stay.
    #[tokio::test]
    async fn the_sweep_removes_only_the_profiles_no_service_holds() {
        let temporary = tempfile::tempdir().unwrap();
        let profile = |name: &str| {
            let path = temporary.path().join(format!("{PROFILE_PREFIX}{name}"));
            std::fs::create_dir(&path).unwrap();
            path
        };
        let orphan = profile("orphan");
        std::fs::File::create(orphan.join(PROFILE_LOCK)).unwrap();
        let held = profile("held");
        let lock = std::fs::File::create(held.join(PROFILE_LOCK)).unwrap();
        lock.try_lock().unwrap();
        let unlocked = profile("being-made");
        let other = temporary.path().join("not-a-profile");
        std::fs::create_dir(&other).unwrap();
        std::fs::File::create(other.join(PROFILE_LOCK)).unwrap();
        sweep_orphans_in(temporary.path(), Vec::new()).await;
        assert!(!orphan.exists());
        assert!(held.exists());
        assert!(unlocked.exists());
        assert!(other.exists());
    }

    #[tokio::test]
    async fn failed_process_retirement_retains_the_profile_and_cause() {
        let profile = tempfile::tempdir().unwrap();
        let path = profile.path().to_owned();
        let result = remove_profile(profile, Err(BrowserError::Timeout)).await;
        assert!(
            matches!(result, Err(BrowserError::ProfileRetained { source, .. }) if matches!(*source, BrowserError::Timeout))
        );
        assert!(path.is_dir());
        std::fs::remove_dir_all(path).unwrap();
    }
}
