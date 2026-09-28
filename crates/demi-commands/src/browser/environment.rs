use std::{
    future::Future,
    io,
    ops::Deref,
    path::{Path, PathBuf},
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
    numbers::TabNumbers,
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

/// An environment's runtime directory's name starts with this, in the
/// runtime base; the rest is the environment's identity.
const RUNTIME_PREFIX: &str = "demi-browser-";
/// An environment's profile's name starts with this, in the profile base;
/// the rest is its environment's identity.
const PROFILE_PREFIX: &str = "demi-profile-";
/// The file an environment's lock is held on while the environment exists,
/// in its runtime directory.
const LOCK: &str = "demi-profile.lock";
/// The lock file until it is held. The sweep never opens it, so it finds an
/// environment either without a lock or with a held one.
const STAGED_LOCK: &str = "demi-profile.lock.new";
/// The runtime directory's link to its profile, on Unix.
#[cfg(unix)]
const PROFILE_LINK: &str = "profile";

/// The user a service sweeps browser directories for: its own. Other
/// users' directories in the shared bases are neither opened nor reported
/// (`browser.md` § Native driver).
#[derive(Clone, Copy, Debug)]
struct Owner {
    #[cfg(unix)]
    uid: u32,
}

impl Owner {
    /// The user this service runs as.
    fn current() -> Self {
        Self {
            // SAFETY: `geteuid` has no preconditions and cannot fail.
            #[cfg(unix)]
            uid: unsafe { libc::geteuid() },
        }
    }

    /// Whether `path` is a directory of this user's, not following a
    /// symbolic link. On Windows every user has a temporary directory of
    /// their own, so every directory there is.
    fn owns_directory(self, path: &Path) -> bool {
        let Ok(metadata) = std::fs::symlink_metadata(path) else {
            return false;
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            metadata.is_dir() && metadata.uid() == self.uid
        }
        #[cfg(not(unix))]
        {
            metadata.is_dir()
        }
    }
}

/// Where browser environments keep their directories, computed in this one
/// place (`browser.md` § Native driver).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectoryBases {
    /// The environments' runtime directories, each Chrome's temporary
    /// directory with the environment's lock: short, since Chrome makes its
    /// process-singleton socket there and the socket's path has a limit.
    pub runtime: PathBuf,
    /// The profiles, with their downloads and uploads, and the downloads
    /// saved without `--output`: on disk, since they can grow large.
    pub profiles: PathBuf,
}

impl DirectoryBases {
    /// This Host's. On Unix the runtime directories go in `/tmp`, whatever
    /// the service's own temporary directory is, and the profiles in the
    /// service's `TMPDIR` when it names a directory, otherwise in `/var/tmp`,
    /// which is on disk on Linux, macOS and the Cloud. On Windows both are
    /// the system's temporary directory, where one directory holds a whole
    /// environment.
    pub fn host() -> Self {
        if cfg!(unix) {
            let profiles = std::env::var_os("TMPDIR")
                .map(PathBuf::from)
                .filter(|directory| directory.is_absolute())
                .unwrap_or_else(|| PathBuf::from("/var/tmp"));
            Self {
                runtime: PathBuf::from("/tmp"),
                profiles,
            }
        } else {
            let temporary = std::env::temp_dir();
            Self {
                runtime: temporary.clone(),
                profiles: temporary,
            }
        }
    }
}

/// One environment's directories, removed when dropped until
/// [`Self::keep_until_retired`]: the runtime directory, Chrome's temporary
/// directory, which holds the environment's lock and on Unix a link to the
/// profile; and on Unix the profile, a directory of its own with Chrome's
/// user data, the downloads and the uploads. On Windows the runtime
/// directory is the profile.
struct EnvironmentDirectories {
    runtime: tempfile::TempDir,
    profile: Option<tempfile::TempDir>,
    /// Held until retirement has removed both, however this service ends.
    lock: std::fs::File,
}

impl EnvironmentDirectories {
    /// Makes an environment's directories in `bases`, each readable only by
    /// its user, since every user shares the bases on Unix. The runtime
    /// directory comes first with its lock held, whose file takes the name
    /// the sweep opens only once it is held, so that a sweep running
    /// meanwhile, in another service of the same user, leaves it alone. On
    /// Unix the link comes next, then the profile it names, so that a
    /// running environment's profile always has its runtime directory.
    fn create(bases: &DirectoryBases) -> Result<Self> {
        let runtime = private_directory(&bases.runtime, RUNTIME_PREFIX, 6)?;
        let staged = runtime.path().join(STAGED_LOCK);
        let lock = std::fs::File::create_new(&staged)?;
        lock.try_lock().map_err(|error| match error {
            std::fs::TryLockError::Error(error) => BrowserError::Io(error),
            // Nothing else opens the staged file; a lock there is a defect.
            std::fs::TryLockError::WouldBlock => BrowserError::Io(io::Error::other(
                "a new browser environment is already locked",
            )),
        })?;
        std::fs::rename(&staged, runtime.path().join(LOCK))?;
        #[cfg(unix)]
        let profile = {
            let identity = runtime
                .path()
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_prefix(RUNTIME_PREFIX))
                .expect("a runtime directory is named by its prefix and an identity");
            let name = format!("{PROFILE_PREFIX}{identity}");
            std::os::unix::fs::symlink(
                bases.profiles.join(&name),
                runtime.path().join(PROFILE_LINK),
            )?;
            Some(private_directory(&bases.profiles, &name, 0)?)
        };
        #[cfg(not(unix))]
        let profile = None;
        Ok(Self {
            runtime,
            profile,
            lock,
        })
    }

    /// Chrome's temporary directory, which names the environment.
    fn runtime(&self) -> &Path {
        self.runtime.path()
    }

    /// Chrome's user data directory, which holds the downloads and uploads.
    fn profile(&self) -> &Path {
        self.profile.as_ref().unwrap_or(&self.runtime).path()
    }

    /// From here only retirement removes the directories: Chrome may be
    /// writing into them.
    fn keep_until_retired(&mut self) {
        self.runtime.disable_cleanup(true);
        if let Some(profile) = &mut self.profile {
            profile.disable_cleanup(true);
        }
    }

    /// Removes the profile, then the runtime directory, once `retired` says
    /// that Chrome's processes are gone, and then lets the lock go. A
    /// failure keeps what is left, for a later sweep, and names the profile.
    async fn remove(self, retired: Result<()>) -> Result<()> {
        let runtime = self.runtime.keep();
        let profile = self.profile.map(tempfile::TempDir::keep);
        let result = async {
            retired?;
            if let Some(profile) = &profile {
                remove_directory(profile).await?;
            }
            remove_directory(&runtime).await
        }
        .await;
        drop(self.lock);
        result.map_err(|source| BrowserError::ProfileRetained {
            path: profile.unwrap_or(runtime),
            source: Box::new(source),
        })
    }
}

/// A new directory in `base` that only its user can read, named `prefix`
/// and `random` random characters.
fn private_directory(base: &Path, prefix: &str, random: usize) -> io::Result<tempfile::TempDir> {
    let mut directory = tempfile::Builder::new();
    directory.prefix(prefix).rand_bytes(random);
    #[cfg(unix)]
    directory.permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700));
    directory.tempdir_in(base)
}

/// Removes `directory`, retrying only "directory not empty" for at most
/// 300 ms: Chrome's last writes can land just after its processes are gone.
async fn remove_directory(directory: &Path) -> Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(300);
    loop {
        match tokio::fs::remove_dir_all(directory).await {
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

/// Own Chrome, its event task and its directories until the conversation work ends;
/// the tabs take their public IDs from `numbers`.
/// Completion, failure and owner cancellation share the same joined cleanup path.
/// Cancel through `stop` and await this owner. On abrupt future disposal the child
/// has kill-on-drop protection; weak session handles cannot keep Chrome alive.
pub async fn with_browser<T, F, W>(
    options: LaunchOptions,
    numbers: TabNumbers,
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
    let mut directories = EnvironmentDirectories::create(&DirectoryBases::host())?;
    let mut process = ChromeProcess::new(directories.runtime(), &options.executable);
    let download_directory = directories.profile().join("downloads");
    tokio::fs::create_dir(&download_directory).await?;
    let upload_directory = directories.profile().join("uploads");
    tokio::fs::create_dir(&upload_directory).await?;
    let builder = BrowserConfig::builder()
        .respect_https_errors()
        .surface_invalid_messages()
        .chrome_executable(options.executable)
        .user_data_dir(directories.profile())
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
        directories.profile(),
        &options.version,
        &options.locale,
        &capture.address()?,
    )
    .await?
    .build()
    .map_err(BrowserError::Configuration)?;
    let platform_arguments = super::launch::platform_arguments(&options.locale);
    directories.keep_until_retired();
    let profile = directories.profile().to_owned();
    let launched = tokio::select! {
        biased;
        _ = stop.cancelled() => Err(BrowserError::Cancelled),
        result = Browser::launch_with(config, |command| process.spawn(command.args(&platform_arguments), &profile)) => result.map_err(BrowserError::from),
    };
    let (browser, mut handler) = match launched {
        Ok(launched) => launched,
        Err(error) => {
            let cleanup = directories.remove(process.terminate().await).await;
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
                numbers,
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
    let cleanup = directories.remove(retired).await;
    after_cleanup(outcome, cleanup)
}

/// Removes what browsers left behind when their service ended without
/// retiring them (`browser.md` § Native driver), in two passes over this
/// user's directories; another user's are left alone, without a word.
/// First the runtime directories: one whose lock this service can take
/// belongs to no running service, so its marked Chrome processes end the
/// way retirement ends them, then the profile its link names goes, then the
/// directory. One whose lock is held, or that has no lock yet, is left
/// alone. Then the profiles whose runtime directory is gone, which happens
/// when a restart emptied a `/tmp` held in memory: no running environment
/// has one.
pub async fn sweep_orphans(directories: &super::BrowserDirectories) {
    sweep_orphans_in(&DirectoryBases::host(), directories.roots(), Owner::current()).await;
}

async fn sweep_orphans_in(bases: &DirectoryBases, installations: Vec<PathBuf>, owner: Owner) {
    for runtime in owned_directories(&bases.runtime, RUNTIME_PREFIX, owner).await {
        if let Err(error) = sweep_orphan(&runtime, installations.clone(), owner).await {
            tracing::warn!(
                "could not remove the orphaned browser environment {}: {error}",
                runtime.display()
            );
        }
    }
    for profile in owned_directories(&bases.profiles, PROFILE_PREFIX, owner).await {
        let identity = profile
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix(PROFILE_PREFIX));
        let Some(identity) = identity else {
            continue;
        };
        let runtime = bases.runtime.join(format!("{RUNTIME_PREFIX}{identity}"));
        // Any other answer leaves the profile to its runtime directory, which
        // the first pass or its running service looks after.
        let gone = matches!(
            tokio::fs::symlink_metadata(&runtime).await,
            Err(error) if error.kind() == io::ErrorKind::NotFound
        );
        if gone && let Err(error) = remove_present(&profile).await {
            tracing::warn!(
                "could not remove the orphaned browser profile {}: {error}",
                profile.display()
            );
        }
    }
}

/// The directories this user owns in `base` whose names start with `prefix`.
async fn owned_directories(base: &Path, prefix: &str, owner: Owner) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(mut entries) = tokio::fs::read_dir(base).await else {
        return found;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if entry.file_name().to_string_lossy().starts_with(prefix) && owner.owns_directory(&path) {
            found.push(path);
        }
    }
    found
}

async fn sweep_orphan(runtime: &Path, installations: Vec<PathBuf>, owner: Owner) -> Result<()> {
    let lock = match std::fs::File::options()
        .read(true)
        .write(true)
        .open(runtime.join(LOCK))
    {
        Ok(lock) => lock,
        // An environment being made, or one this release did not make.
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    match lock.try_lock() {
        Ok(()) => {}
        // A running service's environment holds it.
        Err(std::fs::TryLockError::WouldBlock) => return Ok(()),
        Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
    }
    ChromeProcess::marked(runtime, installations).terminate().await?;
    #[cfg(unix)]
    if let Some(profile) = linked_profile(runtime, owner).await? {
        remove_present(&profile).await?;
    }
    #[cfg(not(unix))]
    let _ = owner;
    tokio::fs::remove_dir_all(runtime).await?;
    drop(lock);
    Ok(())
}

/// The profile `runtime` links to, when the link names a profile directory
/// of this user's; whatever else a link could name is left alone.
#[cfg(unix)]
async fn linked_profile(runtime: &Path, owner: Owner) -> Result<Option<PathBuf>> {
    let profile = match tokio::fs::read_link(runtime.join(PROFILE_LINK)).await {
        Ok(profile) => profile,
        // Its service ended before it made the link, and so the profile.
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let named = profile
        .file_name()
        .is_some_and(|name| name.to_string_lossy().starts_with(PROFILE_PREFIX));
    Ok((named && owner.owns_directory(&profile)).then_some(profile))
}

/// Removes `directory` and what it holds, unless it is gone already.
async fn remove_present(directory: &Path) -> Result<()> {
    match tokio::fs::remove_dir_all(directory).await {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error.into()),
        _ => Ok(()),
    }
}

impl BrowserEnvironment {
    /// Opens `url` in a new tab, as the conversation's root agent does.
    pub async fn open(
        &self,
        url: &str,
        cancellation: &CancellationToken,
        timeout: Duration,
    ) -> Result<BrowserTab> {
        self.open_for(
            url,
            0,
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
        caller: u64,
        load: Load,
        cancellation: &CancellationToken,
        deadline: tokio::time::Instant,
    ) -> Result<(BrowserTab, String)> {
        super::navigation::validate_url(url)?;
        let operation = Operation::until(&self.ended, cancellation, deadline);
        let tab = self
            .create(
                BrowserCreatedBy::Agent { number: caller },
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
        caller: u64,
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
                let created_by = BrowserCreatedBy::Temporary { number: caller };
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
    pub(super) fn debugging_callers(&self, id: &str, caller: Option<u64>) -> Vec<u64> {
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

/// What only the Chrome tests reach: Chrome's own view of its targets, and
/// evaluation in a target no command addresses.
#[cfg(feature = "testing")]
impl BrowserEnvironment {
    /// Every target Chrome runs, the capture extension's included.
    pub async fn targets(
        &self,
    ) -> Result<Vec<chromiumoxide::cdp::browser_protocol::target::TargetInfo>> {
        use chromiumoxide::cdp::browser_protocol::target::GetTargetsParams;
        Ok(self
            .browser
            .call()?
            .execute(GetTargetsParams::default())
            .await?
            .result
            .target_infos)
    }

    /// Evaluates `expression` in `target` over a CDP connection of its own,
    /// and answers its value.
    pub async fn evaluate_in(
        &self,
        target: chromiumoxide::cdp::browser_protocol::target::TargetId,
        expression: &str,
    ) -> Result<serde_json::Value> {
        super::cdp::evaluate_in(&self.browser, target, expression).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bases of a test's own, in `temporary`.
    fn bases(temporary: &tempfile::TempDir) -> DirectoryBases {
        let bases = DirectoryBases {
            runtime: temporary.path().join("runtime"),
            profiles: temporary.path().join("profiles"),
        };
        std::fs::create_dir(&bases.runtime).unwrap();
        std::fs::create_dir(&bases.profiles).unwrap();
        bases
    }

    /// The runtime directory and the profile of the environment `identity`,
    /// linked on Unix as a service links them.
    fn environment(bases: &DirectoryBases, identity: &str) -> (PathBuf, PathBuf) {
        let runtime = bases.runtime.join(format!("{RUNTIME_PREFIX}{identity}"));
        let profile = bases.profiles.join(format!("{PROFILE_PREFIX}{identity}"));
        std::fs::create_dir(&runtime).unwrap();
        std::fs::create_dir(&profile).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&profile, runtime.join(PROFILE_LINK)).unwrap();
        (runtime, profile)
    }

    /// An environment nobody holds goes with its profile; a held one and
    /// one without a lock stay.
    #[tokio::test]
    async fn the_sweep_removes_only_the_environments_no_service_holds() {
        let temporary = tempfile::tempdir().unwrap();
        let bases = bases(&temporary);
        let (orphan, orphan_profile) = environment(&bases, "orphan");
        std::fs::File::create(orphan.join(LOCK)).unwrap();
        let (held, held_profile) = environment(&bases, "held");
        let lock = std::fs::File::create(held.join(LOCK)).unwrap();
        lock.try_lock().unwrap();
        let (unlocked, unlocked_profile) = environment(&bases, "being-made");
        let other = bases.runtime.join("not-an-environment");
        std::fs::create_dir(&other).unwrap();
        std::fs::File::create(other.join(LOCK)).unwrap();
        sweep_orphans_in(&bases, Vec::new(), Owner::current()).await;
        assert!(!orphan.exists());
        assert!(!orphan_profile.exists());
        assert!(held.exists() && held_profile.exists());
        assert!(unlocked.exists() && unlocked_profile.exists());
        assert!(other.exists());
    }

    /// A restart that empties a `/tmp` held in memory takes the runtime
    /// directories and leaves the profiles on disk. A profile whose runtime
    /// directory is gone goes; one whose runtime directory is there stays
    /// with it.
    #[tokio::test]
    async fn the_sweep_removes_a_profile_whose_runtime_directory_is_gone() {
        let temporary = tempfile::tempdir().unwrap();
        let bases = bases(&temporary);
        let (emptied, left) = environment(&bases, "emptied");
        std::fs::remove_dir_all(&emptied).unwrap();
        let (held, held_profile) = environment(&bases, "held");
        let lock = std::fs::File::create(held.join(LOCK)).unwrap();
        lock.try_lock().unwrap();
        let other = bases.profiles.join("not-a-profile");
        std::fs::create_dir(&other).unwrap();
        sweep_orphans_in(&bases, Vec::new(), Owner::current()).await;
        assert!(!left.exists());
        assert!(held_profile.exists());
        assert!(other.exists());
    }

    /// An orphan's link is followed only to a profile directory of this
    /// user's. What a link names otherwise stays, while the orphan goes: a
    /// directory of another name, a symbolic link named as a profile, and,
    /// which only root can arrange, another user's profile.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_sweep_follows_an_orphans_link_only_to_a_profile_of_this_users() {
        let temporary = tempfile::tempdir().unwrap();
        let bases = bases(&temporary);
        let orphan = |identity: &str, target: &Path| {
            let runtime = bases.runtime.join(format!("{RUNTIME_PREFIX}{identity}"));
            std::fs::create_dir(&runtime).unwrap();
            std::fs::File::create(runtime.join(LOCK)).unwrap();
            std::os::unix::fs::symlink(target, runtime.join(PROFILE_LINK)).unwrap();
            runtime
        };
        let elsewhere = temporary.path().join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        let symbolic = bases.profiles.join(format!("{PROFILE_PREFIX}symbolic"));
        std::os::unix::fs::symlink(&elsewhere, &symbolic).unwrap();
        let mut orphans = vec![orphan("elsewhere", &elsewhere), orphan("symbolic", &symbolic)];
        let mut named = vec![elsewhere, symbolic];
        let this_user = Owner::current();
        if this_user.uid == 0 {
            let foreign = bases.profiles.join(format!("{PROFILE_PREFIX}foreign"));
            std::fs::create_dir(&foreign).unwrap();
            std::os::unix::fs::chown(&foreign, Some(65534), Some(65534)).unwrap();
            orphans.push(orphan("foreign", &foreign));
            named.push(foreign);
        }
        sweep_orphans_in(&bases, Vec::new(), this_user).await;
        for orphan in orphans {
            assert!(!orphan.exists(), "{} stayed", orphan.display());
        }
        for path in named {
            assert!(path.symlink_metadata().is_ok(), "{} went", path.display());
        }
    }

    /// Every user's environments share `/tmp` and `/var/tmp`, and a service
    /// sweeps only its own user's: another user's orphan and profile are
    /// neither opened nor reported. Here they are this test's, and the first
    /// sweep runs for another user.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_sweep_leaves_other_users_environments_alone() {
        let temporary = tempfile::tempdir().unwrap();
        let bases = bases(&temporary);
        let (orphan, profile) = environment(&bases, "orphan");
        std::fs::File::create(orphan.join(LOCK)).unwrap();
        let (emptied, left) = environment(&bases, "emptied");
        std::fs::remove_dir_all(&emptied).unwrap();
        let this_user = Owner::current();
        let another_user = Owner {
            uid: this_user.uid.wrapping_add(1),
        };
        sweep_orphans_in(&bases, Vec::new(), another_user).await;
        assert!(orphan.exists() && profile.exists() && left.exists());
        sweep_orphans_in(&bases, Vec::new(), this_user).await;
        assert!(!orphan.exists() && !profile.exists() && !left.exists());
    }

    /// Another service of the same user may sweep at any moment, also while
    /// this one makes an environment's directories, and takes none: a new
    /// environment's lock is held before the sweep can find it. Three sweeps
    /// race the making of 4,000 environments; with the lock file named
    /// before it was held, they took one in each of 20 runs, and with only
    /// one sweep and 2,000 environments in 13 of 20. About 0.4 s.
    #[tokio::test(flavor = "multi_thread", worker_threads = 3)]
    async fn a_sweep_takes_no_environment_being_made() {
        let temporary = tempfile::tempdir().unwrap();
        let bases = bases(&temporary);
        let made = CancellationToken::new();
        let sweeps: Vec<_> = (0..3)
            .map(|_| {
                tokio::spawn({
                    let bases = bases.clone();
                    let made = made.clone();
                    async move {
                        while !made.is_cancelled() {
                            sweep_orphans_in(&bases, Vec::new(), Owner::current()).await;
                        }
                    }
                })
            })
            .collect();
        let making = tokio::task::spawn_blocking({
            let bases = bases.clone();
            move || {
                for _ in 0..4000 {
                    let directories = EnvironmentDirectories::create(&bases)
                        .expect("the sweep took the lock of an environment being made");
                    assert!(
                        directories.runtime().join(LOCK).is_file() && directories.profile().is_dir(),
                        "the sweep removed an environment being made"
                    );
                    // Retirement removes the profile, then the runtime
                    // directory, and then lets the lock go.
                    let EnvironmentDirectories {
                        runtime,
                        profile,
                        lock,
                    } = directories;
                    drop(profile);
                    drop(runtime);
                    drop(lock);
                }
            }
        })
        .await;
        made.cancel();
        for sweep in sweeps {
            sweep.await.unwrap();
        }
        making.unwrap();
    }

    #[tokio::test]
    async fn failed_process_retirement_retains_the_directories_and_cause() {
        let temporary = tempfile::tempdir().unwrap();
        let bases = bases(&temporary);
        let mut directories = EnvironmentDirectories::create(&bases).unwrap();
        directories.keep_until_retired();
        let runtime = directories.runtime().to_owned();
        let profile = directories.profile().to_owned();
        let result = directories.remove(Err(BrowserError::Timeout)).await;
        assert!(
            matches!(&result, Err(BrowserError::ProfileRetained { path, source }) if *path == profile && matches!(**source, BrowserError::Timeout)),
            "{result:?}"
        );
        assert!(runtime.is_dir() && profile.is_dir());
    }
}
