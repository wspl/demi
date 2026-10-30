//! An update's steps (`demi_runner_protocol::release::UpdateRecord`): what
//! the move did and how far it is, which this module keeps and tells.
//!
//! - It keeps them in the backend's data directory, so the next
//!   `demi-server` of the move goes on from them, the update screen shows
//!   them while no backend runs, and the new backend adds its own as it
//!   starts.
//! - It tells them to the running backend over its management socket, for
//!   the administrators' pages and, from the restart, every page. Each
//!   notice is a connection of its own, written and answered within seconds.
//!   A backend that is not there, or of a release that reads no notice, is
//!   told nothing, and the upgrade goes on: the notices only report it.
//!   While the update runs, the last notice is repeated every 10 seconds, so
//!   a backend that hears nothing for longer knows the upgrade stopped.
//! - The work ahead of an update (`preparing`) tells its steps as notices of
//!   their own, which the backend shows as how far the release is got ready,
//!   and keeps no record: no move reads it.
use std::{
    collections::VecDeque,
    io::{self, BufRead as _, BufReader, Write as _},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, PoisonError},
    thread::JoinHandle,
    time::{Duration, Instant},
};

use demi_runner_protocol::release::{
    RestartStepKind, StepProgress, StepState, StepSummary, UPDATE_RECORD, UpdateRecord, UpgradeNotice,
    UpgradeStepKind,
};

use crate::{layout::Layout, settings::Settings};

/// How often the notice of a step that takes long is told again.
const REPEAT: Duration = Duration::from_secs(10);

/// How long a backend may take to answer a notice: the one before its stop
/// waits for every page to hear it.
const ANSWER: Duration = Duration::from_secs(5);

/// The least time between two notices of the bytes downloaded, which change
/// with every read.
const DOWNLOAD_EVERY: Duration = Duration::from_secs(1);

/// How far back the download's rate is measured: long enough that one slow
/// read does not swing it, short enough that it follows a changed link.
const RATE_OVER: Duration = Duration::from_secs(5);

/// The steps of one `demi-server` run's update, and their notices.
pub struct Notices {
    socket: PathBuf,
    /// The record's file; none without a configuration that names the
    /// backend's data.
    path: Option<PathBuf>,
    /// The update's steps, once it knows what it moves to.
    record: Option<UpdateRecord>,
    /// The steps are the work ahead of an update, not an update's.
    preparing: bool,
    repeated: Arc<Repeated>,
    repeating: Option<JoinHandle<()>>,
    /// When the downloads told of the last `RATE_OVER` were, and their bytes
    /// done: the oldest gives the rate, the newest the next notice's due.
    downloads: VecDeque<(Instant, u64)>,
}

/// The notice the repeating thread tells again, and whether it should stop.
#[derive(Default)]
struct Repeated {
    state: Mutex<(Option<UpgradeNotice>, bool)>,
    changed: Condvar,
}

impl Notices {
    /// The notices to the backend the configuration names, of no update yet.
    pub fn new(layout: &Layout) -> Self {
        let settings = Settings::read(&layout.config()).ok();
        let socket = settings
            .as_ref()
            .and_then(|settings| settings.manage_socket())
            .unwrap_or_else(|| layout.manage_socket());
        let path = settings
            .and_then(|settings| settings.backend_data().ok())
            .map(|data| data.join(UPDATE_RECORD));
        let repeated = Arc::new(Repeated::default());
        let repeating = {
            let (repeated, socket) = (repeated.clone(), socket.clone());
            std::thread::spawn(move || repeat(&repeated, &socket))
        };
        Self {
            socket,
            path,
            record: None,
            preparing: false,
            repeated,
            repeating: Some(repeating),
            downloads: VecDeque::new(),
        }
    }

    /// The notices of the work ahead of an update, which keep no record.
    pub fn preparing(layout: &Layout) -> Self {
        let mut notices = Self::new(layout);
        notices.path = None;
        notices.preparing = true;
        notices
    }

    /// The update from `from` to `to`: the steps an earlier run of the same
    /// move recorded, or none taken yet.
    pub fn moving(&mut self, from: &semver::Version, to: &semver::Version) {
        let (from, to) = (from.to_string(), to.to_string());
        let recorded = self
            .path
            .as_ref()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice::<UpdateRecord>(&bytes).ok())
            .filter(|record| record.from == from && record.to == to);
        self.record = Some(recorded.unwrap_or_else(|| UpdateRecord::new(&from, &to)));
    }

    /// The step `kind` of the preparation is now `state`.
    pub fn step(&mut self, kind: UpgradeStepKind, state: StepState) {
        if let Some(record) = &mut self.record {
            *record.upgrade_step(kind) = state;
            self.changed();
        }
    }

    /// The step `kind` from the restart on is now `state`.
    pub fn restart(&mut self, kind: RestartStepKind, state: StepState) {
        if let Some(record) = &mut self.record {
            *record.restart_step(kind) = state;
            self.changed();
        }
    }

    /// The step `kind` of the preparation, while no run took it, is not
    /// needed, for `why`.
    pub fn skip_waiting(&mut self, kind: UpgradeStepKind, why: demi_runner_protocol::release::SkipReason) {
        if let Some(record) = &mut self.record
            && *record.upgrade_step(kind) == StepState::Waiting
        {
            *record.upgrade_step(kind) = StepState::Skipped { why };
            self.changed();
        }
    }

    /// Where the steps are kept, which the update screen reads.
    pub fn record_path(&self) -> Option<PathBuf> {
        self.path.clone()
    }

    /// Starts the step `kind` of the preparation.
    pub fn start(&mut self, kind: UpgradeStepKind) {
        self.step(kind, StepState::running());
    }

    /// The running step `kind` of the preparation has done `summary`.
    pub fn done(&mut self, kind: UpgradeStepKind, summary: StepSummary) {
        if let Some(record) = &mut self.record {
            let step = record.upgrade_step(kind);
            *step = step.done(summary);
            self.changed();
        }
    }

    /// The running step `kind` of the preparation is `progress` far.
    pub fn progress(&mut self, kind: UpgradeStepKind, progress: StepProgress) {
        if let Some(record) = &mut self.record
            && let StepState::Running { progress: now, .. } = record.upgrade_step(kind)
        {
            *now = Some(progress);
            self.changed();
        }
    }

    /// Whether the update keeps the running Clouds running rather than
    /// saving them, which their step says.
    pub fn clouds_kept(&mut self, kept: bool) {
        if let Some(record) = &mut self.record {
            record.clouds_kept(kept);
            self.changed();
        }
    }

    /// Starts the step `kind` from the restart on.
    pub fn start_restart(&mut self, kind: RestartStepKind) {
        self.restart(kind, StepState::running());
    }

    /// The running step `kind` from the restart on has done `summary`.
    pub fn done_restart(&mut self, kind: RestartStepKind, summary: StepSummary) {
        if let Some(record) = &mut self.record {
            let step = record.restart_step(kind);
            *step = step.done(summary);
            self.changed();
        }
    }

    /// The running step `kind` from the restart on is `progress` far.
    pub fn progress_restart(&mut self, kind: RestartStepKind, progress: StepProgress) {
        if let Some(record) = &mut self.record
            && let StepState::Running { progress: now, .. } = record.restart_step(kind)
        {
            *now = Some(progress);
            self.changed();
        }
    }

    /// The step that runs failed for `reason`, and the server returns to the
    /// release it left, when `returning`.
    pub fn failed_step(&mut self, reason: &str, returning: bool) {
        if let Some(record) = &mut self.record {
            let failed = StepState::Failed { reason: reason.to_owned() };
            let running = |state: &StepState| matches!(state, StepState::Running { .. });
            if let Some(step) = record.restart.iter_mut().find(|step| running(&step.state)) {
                step.state = failed;
            } else if let Some(step) = record.upgrade.iter_mut().find(|step| running(&step.state)) {
                step.state = failed;
            } else if self.preparing
                && let Some(step) = record.upgrade.iter_mut().find(|step| step.state == StepState::Waiting)
            {
                // The work ahead failed before its first step: that step
                // says why.
                step.state = failed;
            }
            record.returning = returning;
            self.changed();
        }
    }

    /// Whether the move returns to the release it left.
    pub fn returning(&mut self, returning: bool) {
        if let Some(record) = &mut self.record
            && record.returning != returning
        {
            record.returning = returning;
            self.changed();
        }
    }

    /// The update did not happen for `reason`: the backend hears it, and the
    /// record goes, so no later start reads it. After the backend's stop,
    /// with nothing serving, the record stays with its failed step, which the
    /// update screen shows until the next run finishes the move.
    pub fn failed(&mut self, reason: String, stopped: bool) {
        let returning = self.record.as_ref().is_some_and(|record| record.returning);
        self.failed_step(&reason, returning);
        if self.preparing {
            // The failed step says why; the next check tries again.
            return;
        }
        let update = self.record.take();
        self.tell(UpgradeNotice::Failed { reason, update });
        if !stopped {
            self.forget();
        }
    }

    /// The update is done: the record goes, which the new backend removes
    /// once it serves, and its last notice is told no more, since the
    /// backend that serves now would take it for a restart to come.
    pub fn finished(&mut self) {
        self.record = None;
        self.repeated.state.lock().unwrap_or_else(PoisonError::into_inner).0 = None;
        self.forget();
    }

    fn forget(&self) {
        if let Some(path) = &self.path {
            let _ = std::fs::remove_file(path);
        }
    }

    /// Keeps the record whole, then tells it.
    fn changed(&mut self) {
        let Some(record) = &self.record else {
            return;
        };
        if let Some(path) = &self.path {
            let written = serde_json::to_vec(record)
                .map_err(io::Error::other)
                .and_then(|bytes| keep(path, &bytes));
            if let Err(error) = written {
                eprintln!("The update's steps were not kept in {}: {error}", path.display());
            }
        }
        let notice = if self.preparing {
            UpgradeNotice::Preparing { update: record.clone() }
        } else {
            UpgradeNotice::Progress { update: record.clone() }
        };
        self.tell(notice);
    }

    /// Tells `notice` now, and again every 10 seconds while the update runs.
    fn tell(&mut self, notice: UpgradeNotice) {
        let lasts = matches!(notice, UpgradeNotice::Progress { .. } | UpgradeNotice::Preparing { .. });
        // Sent holding the lock, as the repeating thread sends, so the
        // backend hears the notices in the order they were told: a repeated
        // step never arrives after the restart or the failure that ended it.
        let mut state = self.repeated.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.0 = lasts.then(|| notice.clone());
        self.repeated.changed.notify_all();
        send(&self.socket, &notice);
    }

    /// Tells the bytes downloaded of all and the rate of the last seconds,
    /// at most once a second and always once all are done.
    pub fn downloaded(&mut self, bytes_done: u64, bytes: u64) {
        let now = Instant::now();
        if self.downloads.back().is_some_and(|(last, _)| now - *last < DOWNLOAD_EVERY) && bytes_done < bytes {
            return;
        }
        while self.downloads.front().is_some_and(|(at, _)| now - *at > RATE_OVER) {
            self.downloads.pop_front();
        }
        let per_second = match self.downloads.front() {
            Some((at, done)) if now > *at => Some((bytes_done.saturating_sub(*done) as f64 / (now - *at).as_secs_f64()) as u64),
            _ => None,
        };
        self.downloads.push_back((now, bytes_done));
        self.progress(
            UpgradeStepKind::Download,
            StepProgress::Bytes {
                done: bytes_done,
                total: bytes,
                per_second,
                left_seconds: None,
            },
        );
    }
}

impl Drop for Notices {
    fn drop(&mut self) {
        self.repeated.state.lock().unwrap_or_else(PoisonError::into_inner).1 = true;
        self.repeated.changed.notify_all();
        if let Some(repeating) = self.repeating.take() {
            let _ = repeating.join();
        }
    }
}

/// Writes `bytes` whole to `path` in the backend's data directory, which its
/// unprivileged user owns: the staged file is created anew and never through
/// a link that user left in its place, and every user may read it, as the
/// backend's user must, even where the directory's default ACL would give a
/// new file narrower modes.
fn keep(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
    let staged = path.with_extension("new");
    match std::fs::remove_file(&staged) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(&staged)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o644))?;
    file.write_all(bytes)?;
    std::fs::rename(&staged, path)
}

/// Tells the repeated notice every `REPEAT` until told to stop.
fn repeat(repeated: &Repeated, socket: &Path) {
    let mut state = repeated.state.lock().unwrap_or_else(PoisonError::into_inner);
    loop {
        // Looked at before each wait: a stop told before this thread first
        // waited woke nobody.
        if state.1 {
            return;
        }
        let (next, timeout) = repeated
            .changed
            .wait_timeout(state, REPEAT)
            .unwrap_or_else(PoisonError::into_inner);
        state = next;
        if !state.1
            && timeout.timed_out()
            && let Some(notice) = &state.0
        {
            send(socket, notice);
        }
    }
}

/// Writes `notice` to the backend on `socket` and waits for its answer;
/// nothing fails the upgrade.
fn send(socket: &Path, notice: &UpgradeNotice) {
    let sent = (|| -> io::Result<()> {
        let mut stream = UnixStream::connect(socket)?;
        stream.set_read_timeout(Some(ANSWER))?;
        stream.set_write_timeout(Some(ANSWER))?;
        let mut line = serde_json::to_vec(notice).map_err(io::Error::other)?;
        line.push(b'\n');
        stream.write_all(&line)?;
        let mut answer = String::new();
        BufReader::new(stream).read_line(&mut answer)?;
        Ok(())
    })();
    if let Err(error) = sent
        && !matches!(error.kind(), io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused)
    {
        eprintln!("The backend was not told how far the upgrade is: {error}");
    }
}
