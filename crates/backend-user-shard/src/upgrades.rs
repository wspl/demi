//! Settings › Updates: the release the server runs, the newest of
//! its channel on GitHub, and an update an administrator runs.
//!
//! ```text
//! Update Now ─► request file ─► demi-upgrade.service (root): demi-server
//!                                    │ a line per step, on the management
//!                                    ▼ socket (`backend-http`'s `manage`)
//!     pages ◄── the state channel ◄── heard(notice)
//! ```
//!
//! Every check that finds a newer release has the server get it ready ahead:
//! the backend writes its version to the prepare request beside the update's,
//! whose appearance has systemd start `demi-server prepare --requested`, and
//! shows the administrators how far the release is got ready.
//!
//! The backend cannot update the server itself: it runs as an unprivileged
//! user, and the update stops it. It writes the version to the request file,
//! whose appearance has systemd start `demi-server upgrade --requested` as
//! root, and shows the administrators what `demi-server` tells it of each
//! step. Before the stop it tells every page, so each shows the update's
//! screen until the new release serves.
//!
//! The check and the run are held in memory: a start checks again, and an
//! update that failed is shown until the next one or the next start. Only a
//! server whose backend unit names the request file offers updates.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use demi_backend_database::StorageError;
use demi_backend_database::control::ControlService;
use demi_backend_database::update::Update;
use demi_backend_page_sync::{Part, SyncRegistry};
use demi_runner_protocol::release::{
    PREPARE_REQUEST_NAME, StepProgress, StepState, UpdateRecord, UpgradeNotice, UpgradeStepKind,
};
use demi_shared_artifacts::releases::{self, Channel};
use demi_shared_types::Clock;
use demi_web_api_protocol::server_upgrade::{ServerUpgrade, UpgradeChannel, UpgradeCheck, UpgradePreparation, UpgradeRun};
use demi_web_api_protocol::state::UpdateState;
use semver::Version;

/// How often the backend looks for a newer release of `channel` by itself,
/// which the server then gets ready.
///
/// - Canary publishes every push to main that passes, so a fix reaches the
///   administrators within the hour; its list of every release spends a page
///   of 100 at a time of GitHub's 60 unauthenticated requests an hour per
///   address.
/// - Beta publishes only a Canary someone promoted, which a day's wait does
///   not hold back.
///
/// A start, a change of channel and Check Now look at once besides.
fn check_every(channel: UpgradeChannel) -> Duration {
    match channel {
        UpgradeChannel::Canary => Duration::from_secs(60 * 60),
        UpgradeChannel::Beta => Duration::from_secs(24 * 60 * 60),
    }
}

/// The steps the server takes ahead of Update Now.
const AHEAD: [UpgradeStepKind; 4] = [
    UpgradeStepKind::Download,
    UpgradeStepKind::Unpack,
    UpgradeStepKind::DownloadGvisor,
    UpgradeStepKind::InstallImage,
];

/// How long a step of an update may go unheard before the backend takes the
/// update as stopped: `demi-server` repeats its step every 10 seconds.
const UNHEARD: Duration = Duration::from_secs(60);

/// How often the backend looks whether an update went unheard.
const WATCH_EVERY: Duration = Duration::from_secs(10);

/// How long the backend gives the pages to hear that it stops before it
/// lets `demi-server` stop it.
const PAGES_HEAR: Duration = Duration::from_secs(1);

/// Why an update a page offered was not run.
#[derive(Debug, thiserror::Error)]
pub enum RunRefused {
    #[error("the server offers no updates")]
    NotOffered,
    #[error("an update runs already")]
    Running,
    #[error("{0} is not newer than the release the server runs")]
    NotNewer(String),
    #[error("{0} is not a release's version")]
    NotAVersion(String),
    #[error("the request was not written: {0}")]
    Unwritten(#[from] std::io::Error),
}

/// The server's updates.
pub struct Upgrades {
    /// The file Update Now writes the version to; none when the server
    /// offers no updates.
    request: Option<PathBuf>,
    running: Version,
    control: ControlService,
    sync: SyncRegistry,
    update: Arc<Update>,
    clock: Arc<dyn Clock>,
    state: Mutex<State>,
    /// One check at a time.
    checking: tokio::sync::Mutex<()>,
    /// A check ended or the channel changed: the periodic checks work out
    /// when the next is due again.
    due_changed: Arc<tokio::sync::Notify>,
}

struct State {
    channel: UpgradeChannel,
    check: UpgradeCheck,
    preparation: UpgradePreparation,
    run: UpgradeRun,
    /// When `demi-server` last told how far the run is.
    heard: Instant,
    /// Whether the run's own `demi-server` told how far it is: until it
    /// does, as while it waits for the work ahead, the run shows that work.
    told: bool,
    /// When the last check ended; none before the first.
    checked: Option<Instant>,
}

impl Upgrades {
    /// The updates of a server whose backend unit names `request`, on the
    /// channel the control database keeps; it checks now and then at the
    /// channel's cadence. Without `request` it offers none and checks nothing.
    pub async fn start(
        request: Option<PathBuf>,
        installed: UpgradeChannel,
        control: ControlService,
        sync: SyncRegistry,
        update: Arc<Update>,
        clock: Arc<dyn Clock>,
    ) -> Result<Arc<Self>, StorageError> {
        let channel = match request {
            Some(_) => control.upgrade_channel(installed).await?,
            None => UpgradeChannel::Beta,
        };
        let upgrades = Arc::new(Self {
            request,
            running: Version::parse(demi_shared_artifacts::BUILD_VERSION).expect("the build version is a version"),
            control,
            sync,
            update,
            clock,
            state: Mutex::new(State {
                channel,
                check: UpgradeCheck::Never,
                preparation: UpgradePreparation::None,
                run: UpgradeRun::None,
                heard: Instant::now(),
                told: false,
                checked: None,
            }),
            checking: tokio::sync::Mutex::new(()),
            due_changed: Arc::new(tokio::sync::Notify::new()),
        });
        if upgrades.offered() {
            tokio::spawn(checks(Arc::downgrade(&upgrades), upgrades.due_changed.clone()));
            tokio::spawn(watch(Arc::downgrade(&upgrades)));
        }
        Ok(upgrades)
    }

    fn locked(&self) -> std::sync::MutexGuard<'_, State> {
        // Nothing panics while it holds the lock.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether the server offers updates.
    pub fn offered(&self) -> bool {
        self.request.is_some()
    }

    /// Settings › Updates now; none when the server offers no
    /// updates.
    pub fn snapshot(&self) -> Option<ServerUpgrade> {
        self.offered().then(|| {
            let state = self.locked();
            ServerUpgrade {
                running: self.running.to_string(),
                channel: state.channel,
                check: state.check.clone(),
                preparation: state.preparation.clone(),
                run: state.run.clone(),
            }
        })
    }

    /// What `GET /api/update` answers, which the pages hear when the
    /// backend stops for an update.
    pub fn update_state(&self) -> UpdateState {
        self.update.state()
    }

    /// Every master's and administrator's page hears the change.
    fn changed(&self) {
        self.sync.mark_everyone(&Part::ServerUpgrade);
    }

    /// Changes the channel and looks for the newest of it, unless a check
    /// that ended since did.
    pub async fn set_channel(&self, channel: UpgradeChannel) -> Result<(), StorageError> {
        self.control.set_upgrade_channel(channel).await?;
        let switched = {
            let mut state = self.locked();
            state.channel = channel;
            Instant::now()
        };
        self.due_changed.notify_one();
        self.changed();
        if !self.offered() {
            return Ok(());
        }
        // A periodic check that took the lock first looked at the new
        // channel already.
        let _one = self.checking.lock().await;
        if self.locked().checked.is_none_or(|checked| checked < switched) {
            self.check_held().await;
        }
        Ok(())
    }

    /// Looks for the newest release of the channel on GitHub.
    pub async fn check(&self) {
        if !self.offered() {
            return;
        }
        let _one = self.checking.lock().await;
        self.check_held().await;
    }

    /// When the periodic check is due: none before the first check, which is
    /// due at once, and then the channel's cadence after the last one ended.
    fn due(&self) -> Option<Instant> {
        let state = self.locked();
        state.checked.map(|checked| checked + check_every(state.channel))
    }

    /// Whether the periodic check is due now.
    fn is_due(&self) -> bool {
        self.due().is_none_or(|due| Instant::now() >= due)
    }

    /// Checks when the periodic check is due, as one check does; nothing when
    /// another check, such as a change of channel's, ended meanwhile.
    async fn check_if_due(&self) {
        let _one = self.checking.lock().await;
        if self.is_due() {
            self.check_held().await;
        }
    }

    /// [`Self::check`], with the one check at a time held.
    async fn check_held(&self) {
        let channel = {
            let mut state = self.locked();
            state.check = UpgradeCheck::Checking;
            state.channel
        };
        self.changed();
        let found = releases::newest(match channel {
            UpgradeChannel::Beta => Channel::Beta,
            UpgradeChannel::Canary => Channel::Canary,
        })
        .await;
        let at = self.clock.now();
        let check = match found {
            Ok(newest) => {
                let newer = newest.filter(|newest| newest > &self.running);
                UpgradeCheck::Checked {
                    at,
                    notes_url: newer.as_ref().map(releases::page_url),
                    newer: newer.map(|newer| newer.to_string()),
                }
            }
            Err(error) => {
                tracing::warn!(event = "upgrade.check_failed", %error, "the newest release could not be read from GitHub");
                UpgradeCheck::Failed {
                    at,
                    reason: error.to_string(),
                }
            }
        };
        let prepare = {
            let mut state = self.locked();
            // A change of channel meanwhile checks again: this check looked
            // at the channel before.
            if state.channel != channel {
                None
            } else {
                state.checked = Some(Instant::now());
                let newer = match &check {
                    UpgradeCheck::Checked { newer, .. } => newer.clone(),
                    _ => None,
                };
                state.check = check;
                newer.filter(|_| !matches!(state.run, UpgradeRun::Updating { .. }))
            }
        };
        self.changed();
        self.due_changed.notify_one();
        if let (Some(newer), Some(request)) = (prepare, &self.request) {
            let prepare = request.with_file_name(PREPARE_REQUEST_NAME);
            match write_request(&prepare, &newer) {
                Ok(()) => tracing::info!(event = "upgrade.prepare_requested", version = %newer, "the server gets the newest release ready ahead"),
                Err(error) => tracing::warn!(event = "upgrade.prepare_unrequested", version = %newer, %error, "the newest release was not asked to be got ready"),
            }
        }
    }

    /// Runs the update to `version`: writes the request that has systemd
    /// start `demi-server`. The page names the version it offered, which
    /// must be newer than the running one.
    pub fn run(&self, version: &str) -> Result<(), RunRefused> {
        let request = self.request.as_ref().ok_or(RunRefused::NotOffered)?;
        let to = Version::parse(version).map_err(|_| RunRefused::NotAVersion(version.to_owned()))?;
        if to <= self.running {
            return Err(RunRefused::NotNewer(version.to_owned()));
        }
        {
            let mut state = self.locked();
            if matches!(state.run, UpgradeRun::Updating { .. }) {
                return Err(RunRefused::Running);
            }
            write_request(request, &to.to_string())?;
            // The download starts as `demi-server` does, which says how far
            // it is from its first bytes.
            let mut record = UpdateRecord::new(&self.running.to_string(), &to.to_string());
            *record.upgrade_step(UpgradeStepKind::Download) = StepState::running();
            state.run = UpgradeRun::Updating {
                to: to.to_string(),
                steps: record.upgrade,
            };
            state.heard = Instant::now();
            state.told = false;
        }
        tracing::info!(event = "upgrade.requested", to = %to, "an administrator asked for an update of the server");
        self.changed();
        Ok(())
    }

    /// What `demi-server` told of the update, which the administrators'
    /// pages show; before the stop every page hears it, and this returns
    /// once they had the time to.
    pub async fn heard(&self, notice: UpgradeNotice) {
        tracing::info!(event = "upgrade.heard", ?notice, "demi-server told how far the update of the server is");
        let restart = {
            let mut state = self.locked();
            state.heard = Instant::now();
            match notice {
                UpgradeNotice::Preparing { update } => {
                    state.preparation = preparation_of(&update);
                    // A run that waits for the work ahead shows it, and
                    // hears from it, until its own `demi-server` tells.
                    let told = state.told;
                    let mut shown = false;
                    if let UpgradeRun::Updating { to, steps } = &mut state.run
                        && *to == update.to
                        && !told
                    {
                        for step in steps.iter_mut().filter(|step| AHEAD.contains(&step.kind)) {
                            if let Some(ahead) = update.upgrade.iter().find(|ahead| ahead.kind == step.kind) {
                                step.state = ahead.state.clone();
                            }
                        }
                        shown = true;
                    }
                    if shown {
                        state.heard = Instant::now();
                    }
                    // The work ahead says nothing of a restart.
                    drop(state);
                    self.changed();
                    return;
                }
                UpgradeNotice::Progress { update } => {
                    state.told = true;
                    let restart = update.restarting().then(|| update.clone());
                    state.run = UpgradeRun::Updating {
                        to: update.to,
                        steps: update.upgrade,
                    };
                    restart
                }
                UpgradeNotice::Failed { reason, update } => {
                    state.run = match update {
                        Some(update) => UpgradeRun::Failed {
                            to: update.to,
                            reason,
                            steps: update.upgrade,
                        },
                        // A run that failed before it read the request
                        // names nothing, and keeps the steps of one that
                        // ran.
                        None => match std::mem::replace(&mut state.run, UpgradeRun::None) {
                            UpgradeRun::Updating { to, steps } => UpgradeRun::Failed { to, reason, steps },
                            UpgradeRun::Failed { to, steps, .. } => UpgradeRun::Failed { to, reason, steps },
                            UpgradeRun::None => UpgradeRun::Failed {
                                to: String::new(),
                                reason,
                                steps: Vec::new(),
                            },
                        },
                    };
                    None
                }
            }
        };
        let restarting = restart.is_some();
        self.update.restarting(restart);
        self.changed();
        if restarting {
            self.sync.mark_everyone(&Part::Update);
            tokio::time::sleep(PAGES_HEAR).await;
        }
    }

    /// Takes a run whose step went unheard for a minute as stopped. A
    /// restart stops this backend within seconds, so a backend that still
    /// runs a minute after it heard of the restart was never stopped, and
    /// every page leaves the update's screen.
    fn unheard(&self) {
        let restarting = {
            let mut state = self.locked();
            let UpgradeRun::Updating { to, steps } = &state.run else {
                return;
            };
            if state.heard.elapsed() < UNHEARD {
                return;
            }
            let (to, mut steps) = (to.clone(), steps.clone());
            let restarting = steps
                .iter()
                .any(|step| step.kind == UpgradeStepKind::Restart && step.state != StepState::Waiting);
            tracing::warn!(event = "upgrade.unheard", to = %to, "the update of the server went unheard and is taken as stopped");
            let reason = "The update stopped without saying why. On the server, journalctl -u demi-upgrade shows what happened.";
            // The step that ran is the one it stopped at.
            for step in &mut steps {
                if matches!(step.state, StepState::Running { .. }) {
                    step.state = StepState::Failed { reason: reason.into() };
                }
            }
            state.run = UpgradeRun::Failed {
                to,
                reason: reason.into(),
                steps,
            };
            restarting
        };
        if restarting {
            self.update.restarting(None);
            self.sync.mark_everyone(&Part::Update);
        }
        self.changed();
    }
}

/// How far the work ahead that `update` tells has got its release ready.
fn preparation_of(update: &UpdateRecord) -> UpgradePreparation {
    let version = update.to.clone();
    let ahead = || update.upgrade.iter().filter(|step| AHEAD.contains(&step.kind));
    // Any step that failed ended the work, one it failed before too.
    if let Some(reason) = update.upgrade.iter().find_map(|step| match &step.state {
        StepState::Failed { reason } => Some(reason.clone()),
        _ => None,
    }) {
        return UpgradePreparation::Failed { version, reason };
    }
    if ahead().all(|step| matches!(step.state, StepState::Done { .. } | StepState::Skipped { .. })) {
        return UpgradePreparation::Ready { version };
    }
    let download = ahead().find(|step| step.kind == UpgradeStepKind::Download).map(|step| &step.state);
    match download {
        Some(StepState::Running {
            progress: Some(StepProgress::Bytes { done, total, .. }),
            ..
        }) => UpgradePreparation::Downloading {
            version,
            done: *done,
            total: *total,
        },
        Some(StepState::Running { .. } | StepState::Waiting) => UpgradePreparation::Downloading {
            version,
            done: 0,
            total: 0,
        },
        _ => UpgradePreparation::Installing { version },
    }
}

/// Writes `version` to the request file whole, so the path unit that waits
/// for the file never reads half of it.
fn write_request(request: &std::path::Path, version: &str) -> std::io::Result<()> {
    let staged = request.with_extension("new");
    std::fs::write(&staged, version.to_string())?;
    std::fs::rename(&staged, request)
}

/// Checks now and then at the channel's cadence while the backend runs
/// (`check_every`), from the end of the last check, whichever made it: a
/// change of channel or Check Now starts the wait again.
async fn checks(upgrades: Weak<Upgrades>, due_changed: Arc<tokio::sync::Notify>) {
    loop {
        let Some(held) = upgrades.upgrade() else {
            return;
        };
        let Some(due) = held.due().filter(|due| Instant::now() < *due) else {
            held.check_if_due().await;
            continue;
        };
        drop(held);
        tokio::select! {
            () = tokio::time::sleep_until(due.into()) => {}
            () = due_changed.notified() => {}
        }
    }
}

/// Looks every few seconds whether a run went unheard.
async fn watch(upgrades: Weak<Upgrades>) {
    loop {
        tokio::time::sleep(WATCH_EVERY).await;
        let Some(upgrades) = upgrades.upgrade() else {
            return;
        };
        upgrades.unheard();
    }
}
