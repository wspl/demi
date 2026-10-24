//! One conversation browser's environment: the Chrome process tree, its event
//! task, its two directories and the tab registry, from launch to retirement.
//!
//! ```text
//! /tmp/demi-browser-a1B2c3/                 the runtime directory
//!   demi-profile.lock                       the environment's lock
//!   profile -> /var/tmp/demi-profile-a1B2c3
//!   org.chromium.Chromium.d4E5f6/SingletonSocket
//! /var/tmp/demi-profile-a1B2c3/             the profile: Chrome's user data
//!   downloads/  uploads/
//! ```
//!
//! # Directories
//!
//! - The runtime directory is Chrome's temporary directory. Its singleton
//!   socket's path has a limit (108 bytes on Linux), so it lives in the short
//!   `/tmp`, not in the resident's `TMPDIR`, which can be any length.
//! - The profile can grow large, so it lives on disk (`TMPDIR` when absolute,
//!   else `/var/tmp`), never a memory-backed `/tmp`: the Cloud's is 256 MiB of
//!   its memory.
//! - Directories are made runtime first (with its lock), then the link, then
//!   the profile, and removed in reverse, so a running profile always has its
//!   runtime directory. Each is readable only by its user.
//! - Two stores stay in the user's home on purpose: Linux's NSS certificate
//!   database (moving it would also move the user's fonts) and macOS's Crashpad
//!   reports. Linux crash reports follow the profile through
//!   `CHROME_CONFIG_HOME`.
//!
//! # Cleanup after a crash
//!
//! No Chrome outlives its resident. When a resident ends without retiring (crash,
//! kill, missed shutdown deadline), three rules cover it:
//!
//! 1. The lock file, which the operating system releases whenever the resident
//!    ends.
//! 2. On Linux, the parent-death signal (`driver::process`).
//! 3. A sweep at the next start, limited to its own user's directories.
//!
//! The sweep works as follows:
//!
//! - It first takes the lock of each runtime directory. One it can take belongs
//!   to no running resident, so its marked processes, profile and directory are
//!   removed. A held one belongs to another resident (an upgrade's previous
//!   release) and is never touched.
//! - Then it removes profiles whose runtime directory is gone.
//! - A lock file takes its name only once it is held, so a sweep in another
//!   resident finds either no lock or a held one.

use std::{
    future::Future,
    io,
    ops::Deref,
    path::{Path, PathBuf},
    sync::{Arc, Weak},
    time::Duration,
};

use chromiumoxide::{Browser, BrowserConfig, handler::viewport::Viewport};
use futures_util::StreamExt;
use tokio::sync::watch;
use tokio_util::{
    sync::CancellationToken,
    task::{AbortOnDropHandle, TaskTracker, task_tracker::TaskTrackerToken},
};

use crate::driver::capture::CaptureChannel;
use demi_shared_recording::Live;
use crate::driver::{
    installation::Installation,
    numbers::TabNumbers,
    operation::{BrowserError, CONTROL_TIMEOUT, Operation, Result, after_cleanup},
    process::ChromeProcess,
};
use demi_command_protocol::{ColorScheme, CommandLocale};

use crate::tabs::{
    protocol::{BrowserCreatedBy, BrowserViewport, TabId},
    registry::{self, Hold, Snapshot, Tabs},
    tab::BrowserTab,
};

/// Chrome answers a browser call within this, or chromiumoxide fails the
/// call.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// The environment's one browser connection, which commands share without
/// taking turns. The environment
/// owns the `Browser`; a handle reaches it for one call at a time, and only
/// until the environment ends, so no handle keeps Chrome running.
#[derive(Clone)]
pub struct BrowserHandle {
    browser: Weak<Browser>,
    /// Browser calls in flight. Retirement waits for them before it takes
    /// the browser back to close it.
    calls: TaskTracker,
    ended: CancellationToken,
}

/// The browser, for one call.
pub struct BrowserCall {
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

/// The user a resident sweeps browser directories for: its own. Other
/// users' directories in the shared bases are neither opened nor reported.
#[derive(Clone, Copy, Debug)]
struct Owner {
    #[cfg(unix)]
    uid: u32,
}

impl Owner {
    /// The user this resident runs as.
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
/// place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectoryBases {
    /// The environments' runtime directories, each Chrome's temporary
    /// directory with the environment's lock: short, since Chrome makes its
    /// process-singleton socket there and the socket's path has a limit.
    pub runtime: PathBuf,
    /// The profiles, with their downloads and uploads: on disk, since they
    /// can grow large.
    pub profiles: PathBuf,
}

impl DirectoryBases {
    /// This Host's directory bases.
    ///
    /// - On Unix the runtime directories go in `/tmp`, whatever the resident's
    ///   own temporary directory is. The profiles go in the resident's `TMPDIR`
    ///   when it names a directory, otherwise in `/var/tmp`, which is on disk
    ///   on Linux, macOS and the Cloud.
    /// - On Windows both are the system's temporary directory, where one
    ///   directory holds a whole environment.
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
/// [`Self::keep_until_retired`].
///
/// - The runtime directory is Chrome's temporary directory. It holds the
///   environment's lock and, on Unix, a link to the profile.
/// - On Unix the profile is a directory of its own with Chrome's user data, the
///   downloads and the uploads.
/// - On Windows the runtime directory is the profile.
struct EnvironmentDirectories {
    runtime: tempfile::TempDir,
    profile: Option<tempfile::TempDir>,
    /// Held until retirement has removed both, however this resident ends.
    lock: std::fs::File,
}

impl EnvironmentDirectories {
    /// Makes an environment's directories in `bases`, each readable only by its
    /// user, since every user shares the bases on Unix.
    ///
    /// 1. The runtime directory comes first with its lock held, whose file
    ///    takes the name the sweep opens only once it is held, so that a sweep
    ///    running meanwhile, in another resident of the same user, leaves it
    ///    alone.
    /// 2. On Unix the link comes next.
    /// 3. Then comes the profile the link names, so that a running
    ///    environment's profile always has its runtime directory.
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

/// The caller supplies the installed, verified release, never a PATH lookup.
pub struct LaunchOptions {
    /// The executable, and on Linux the Chrome runtime it starts with.
    pub installation: Installation,
    /// The release's version, for the user agent pages see.
    version: String,
    /// The user's time zone and languages; they do not change while the
    /// environment lives.
    pub locale: CommandLocale,
    /// The user's color scheme, which every page renders in; it does not
    /// change while the environment lives.
    pub color_scheme: ColorScheme,
}

impl LaunchOptions {
    /// The pinned release installed as `installation`, started in `locale`
    /// and `color_scheme`.
    pub fn pinned(
        installation: Installation,
        locale: CommandLocale,
        color_scheme: ColorScheme,
    ) -> Result<Self> {
        Ok(Self {
            installation,
            version: crate::driver::installation::pinned_version()?,
            locale,
            color_scheme,
        })
    }
}

#[derive(Clone)]
pub struct BrowserEnvironment {
    pub(crate) browser: BrowserHandle,
    pub(crate) ended: CancellationToken,
    tabs: Tabs,
    /// Why the browser connection ended, once it ended on its own.
    report_failure: watch::Sender<Option<String>>,
    pub(crate) tasks: TaskTracker,
    /// The recordings of the browser's tabs that still run, which end with
    /// every frame kept before the browser does.
    recordings: Recordings,
    pub(crate) download_directory: PathBuf,
    /// Files the user chose in the live view, until the browser retires.
    pub(crate) upload_directory: PathBuf,
    /// The capture extension's connection.
    pub(crate) captures: CaptureChannel,
    /// Counts the changes to what the browser shows: its tabs, their titles
    /// and URLs, and their viewports; counted on from the conversation's
    /// browsers before it ([`TabNumbers::browser_changes`]).
    pub(crate) changes: watch::Sender<u64>,
    /// The viewport a tab the user opens takes: the one the live view would
    /// give a tab its viewers watch, once one of them has a panel; none
    /// before.
    pub(crate) opening: watch::Sender<Option<BrowserViewport>>,
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
    let Installation {
        executable,
        runtime,
    } = options.installation;
    if !executable.is_absolute() {
        return Err(BrowserError::Configuration(
            "Chrome executable must be absolute".into(),
        ));
    }
    let mut directories = EnvironmentDirectories::create(&DirectoryBases::host())?;
    let mut process = ChromeProcess::new(directories.runtime(), &executable);
    let download_directory = directories.profile().join("downloads");
    tokio::fs::create_dir(&download_directory).await?;
    let upload_directory = directories.profile().join("uploads");
    tokio::fs::create_dir(&upload_directory).await?;
    let builder = BrowserConfig::builder()
        .respect_https_errors()
        .surface_invalid_messages()
        .chrome_executable(executable)
        .user_data_dir(directories.profile())
        .viewport(Viewport {
            width: crate::tabs::viewport::UNWATCHED.width,
            height: crate::tabs::viewport::UNWATCHED.height,
            ..Viewport::default()
        })
        .launch_timeout(Duration::from_secs(60))
        .request_timeout(REQUEST_TIMEOUT);
    let capture = crate::driver::capture::CaptureServer::bind().await?;
    let config = crate::driver::launch::configure(
        builder,
        directories.profile(),
        &options.version,
        &options.locale,
        options.color_scheme,
        runtime.as_ref(),
        &capture.address()?,
    )
    .await?
    .build()
    .map_err(BrowserError::Configuration)?;
    directories.keep_until_retired();
    let profile = directories.profile().to_owned();
    let launched = tokio::select! {
        biased;
        _ = stop.cancelled() => Err(BrowserError::Cancelled),
        result = Browser::launch_with(config, |command| {
            let command = crate::driver::launch::complete(command, &options.locale, runtime.as_ref());
            process.spawn(command, &profile)
        }) => result.map_err(BrowserError::from),
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
    let recordings = Recordings::default();
    let calls = TaskTracker::new();
    let handle = BrowserHandle {
        browser: Arc::downgrade(&browser),
        calls: calls.clone(),
        ended: ended.clone(),
    };
    let _end_on_drop = ended.clone().drop_guard();
    let extension = crate::driver::launch::capture_extension(&profile)
        .to_string_lossy()
        .into_owned();
    let recreation = handle.clone();
    let captures = CaptureChannel::open(
        &observers,
        ended.clone(),
        Box::new(move || {
            use chromiumoxide::cdp::browser_protocol::extensions::LoadUnpackedParams;
            let browser = recreation.clone();
            let extension = extension.clone();
            Box::pin(async move {
                // Loading the extension Chrome already has replaces it.
                browser
                    .call()?
                    .execute(LoadUnpackedParams::new(extension))
                    .await?;
                Ok(())
            })
        }),
    );
    capture.serve(captures.clone(), &observers, ended.clone());
    let changes = numbers.browser_changes();
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
            let (opening, opened) = watch::channel(None);
            let tabs = registry::start(
                handle.clone(),
                numbers,
                ended.clone(),
                &observers,
                failure.clone(),
                changes.clone(),
                opened,
            )
            .await?;
            let environment = BrowserEnvironment {
                browser: handle.clone(),
                ended: ended.clone(),
                tabs,
                report_failure: failure.clone(),
                tasks: observers.clone(),
                recordings: recordings.clone(),
                download_directory,
                upload_directory,
                captures,
                changes,
                opening,
            };
            work(environment).await
        } => result,
    };
    // The recordings end first, while the browser still answers: each takes
    // what the capture extension holds, and its files are complete before the
    // browser goes, whatever ends it.
    recordings.finish().await;
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

/// Removes what browsers left behind when their resident ended without
/// retiring them, in two passes over this
/// user's directories; another user's are left alone, without a word.
/// First the runtime directories: one whose lock this resident can take
/// belongs to no running resident, so its marked Chrome processes end the
/// way retirement ends them, then the profile its link names goes, then the
/// directory. One whose lock is held, or that has no lock yet, is left
/// alone. Then the profiles whose runtime directory is gone, which happens
/// when a restart emptied a `/tmp` held in memory: no running environment
/// has one.
///
/// Without the installed Chrome's location the sweep cannot find an orphan's
/// processes, so it leaves the runtime directories to the next resident:
/// removing a profile under a Chrome that still runs would leave both.
pub async fn sweep_orphans(chrome: &crate::driver::installation::Chrome) {
    let installations = chrome.roots().await;
    sweep_orphans_in(&DirectoryBases::host(), installations, Owner::current()).await;
}

async fn sweep_orphans_in(bases: &DirectoryBases, installations: Option<Vec<PathBuf>>, owner: Owner) {
    let runtimes = match &installations {
        Some(_) => owned_directories(&bases.runtime, RUNTIME_PREFIX, owner).await,
        None => Vec::new(),
    };
    for runtime in runtimes {
        let installations = installations.clone().unwrap_or_default();
        if let Err(error) = sweep_orphan(&runtime, installations, owner).await {
            tracing::warn!(
                event = "browser_environment.orphan_remove_failed",
                what = "environment",
                directory = ?runtime.file_name().unwrap_or_default(),
                %error,
                "an orphaned browser directory could not be removed"
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
        // the first pass or its running resident looks after.
        let gone = matches!(
            tokio::fs::symlink_metadata(&runtime).await,
            Err(error) if error.kind() == io::ErrorKind::NotFound
        );
        if gone && let Err(error) = remove_present(&profile).await {
            tracing::warn!(
                event = "browser_environment.orphan_remove_failed",
                what = "profile",
                directory = ?profile.file_name().unwrap_or_default(),
                %error,
                "an orphaned browser directory could not be removed"
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
        // A running resident's environment holds it.
        Err(std::fs::TryLockError::WouldBlock) => return Ok(()),
        Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
    }
    ChromeProcess::marked(runtime, installations)
        .terminate()
        .await?;
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
        // Its resident ended before it made the link, and so the profile.
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

/// The recordings of an environment's tabs that run.
#[derive(Clone, Default)]
struct Recordings(Arc<std::sync::Mutex<Vec<Live>>>);

impl Recordings {
    fn add(&self, live: Live) {
        let mut recordings = self.0.lock().expect("the recordings are intact");
        recordings.retain(|live| !live.ended().is_cancelled());
        recordings.push(live);
    }

    /// Ends every recording that runs, for the browser closing, and answers
    /// once their files are complete.
    async fn finish(&self) {
        let running: Vec<Live> = self.0.lock().expect("the recordings are intact").drain(..).collect();
        for live in running {
            let _ended = live.stop(demi_shared_recording_protocol::EndReason::Closed).await;
        }
    }
}

impl BrowserEnvironment {
    /// The browser connection, which commands share.
    pub fn browser(&self) -> &BrowserHandle {
        &self.browser
    }

    /// Cancelled once the environment ends.
    pub fn ended(&self) -> &CancellationToken {
        &self.ended
    }

    /// The environment's tasks, which retirement joins: work that serves the
    /// environment and no command waits for runs on them.
    pub fn tasks(&self) -> &TaskTracker {
        &self.tasks
    }

    /// `live` records one of the environment's tabs: the browser does not end
    /// before it has, with every frame kept.
    pub fn recording(&self, live: Live) {
        self.recordings.add(live);
    }

    /// Where Chrome saves downloads.
    pub fn download_directory(&self) -> &Path {
        &self.download_directory
    }

    /// A new path for a download saved without `--output`: in the download
    /// directory, on disk with the profile, which retirement removes.
    pub fn saved_download(&self) -> Result<PathBuf> {
        Ok(self
            .download_directory
            .join(crate::driver::handles::fresh("demi-download")?))
    }

    /// Where the files the user chose in the live view stay until the
    /// browser retires.
    pub fn upload_directory(&self) -> &Path {
        &self.upload_directory
    }

    /// The capture extension's connection.
    pub fn captures(&self) -> &CaptureChannel {
        &self.captures
    }

    /// Counts the changes to what the browser shows: its tabs, their titles
    /// and URLs, and their viewports.
    pub fn changes(&self) -> &watch::Sender<u64> {
        &self.changes
    }

    /// A new empty tab, made by `created_by`: `tab.new`, whose caller opens
    /// a page in it next.
    pub async fn new_tab(
        &self,
        created_by: BrowserCreatedBy,
        cancellation: &CancellationToken,
        deadline: tokio::time::Instant,
    ) -> Result<BrowserTab> {
        let operation = Operation::until(&self.ended, cancellation, deadline);
        self.create(created_by, &operation).await
    }

    /// Creates `count` temporary tabs, and keeps the environment from
    /// retiring until the batch is closed.
    pub async fn temporary_tabs(
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
    /// operation's end is reported, so a later listing does not show it; a
    /// creation that failed has no tab to close.
    async fn create(
        &self,
        created_by: BrowserCreatedBy,
        operation: &Operation<'_>,
    ) -> Result<BrowserTab> {
        let mut creation = operation.run(self.tabs.create(created_by)).await?;
        match operation.run(creation.tab()).await {
            Ok(tab) => Ok(tab),
            Err(error) if creation.answered() => Err(error),
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
    pub fn debugging_callers(&self, id: &str, caller: Option<u64>) -> Vec<u64> {
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

    /// The tabs as [`Self::listed`] lists them, with the list's number in
    /// the Host's sequence: the count of changes read before the listing, so
    /// a list never claims to be newer than it is.
    pub async fn numbered(
        &self,
        cancellation: &CancellationToken,
        timeout: Duration,
    ) -> Result<(u64, Arc<Snapshot>)> {
        let number = *self.changes.borrow();
        Ok((number, self.listed(cancellation, timeout).await?))
    }

    /// [`Self::numbered`] with no time limit, for the report of a conversation's
    /// tabs, which waits for the browser as long as it takes.
    pub async fn numbered_unbounded(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<(u64, Arc<Snapshot>)> {
        let number = *self.changes.borrow();
        let listing = Operation::unbounded(&self.ended, cancellation)
            .run(self.tabs.listing())
            .await?;
        Ok((number, listing))
    }

    /// The tabs in the order the browser created them, with their titles and
    /// URLs, as the registry lists them.
    pub async fn listed(
        &self,
        cancellation: &CancellationToken,
        timeout: Duration,
    ) -> Result<Arc<Snapshot>> {
        Operation::new(&self.ended, cancellation, timeout)
            .run(self.tabs.listing())
            .await
    }

    /// The tab with public ID `id`, from the registry's snapshot.
    pub async fn tab(
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
    pub fn emptied(&self) -> &CancellationToken {
        self.tabs.emptied()
    }

    /// Why the browser connection ended on its own, if it did.
    pub fn failure(&self) -> Option<String> {
        self.report_failure.borrow().clone()
    }

    /// Whether both are the same browser generation.
    pub fn same(&self, other: &Self) -> bool {
        self.browser.same(&other.browser)
    }
}

/// Temporary tabs in use; until they are closed, the environment does not
/// retire.
pub struct TemporaryBatch {
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
        tracing::warn!(event = "browser.helpers_unobserved", %error, "Chrome's helpers were not observed before retirement");
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
            tracing::warn!(
                event = "browser.call_outlived",
                "a browser call outlived its environment; Chrome's process tree ends without closing it"
            );
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
