//! What an upgrade tells the running backend of how far it is, over the
//! backend's management socket, for the administrators' pages
//! (`demi_runner_protocol::release::UpgradeNotice`).
//!
//! Each notice is a connection of its own, written and answered within
//! seconds. A backend that is not there, or of a release that reads no
//! notice, is told nothing, and the upgrade goes on: the notices only
//! report it. While the release downloads or prepares, the last notice is
//! repeated every 10 seconds, so a backend that hears nothing for longer
//! knows the upgrade stopped.

use std::{
    collections::VecDeque,
    io::{self, BufRead as _, BufReader, Write as _},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, PoisonError},
    thread::JoinHandle,
    time::{Duration, Instant},
};

use demi_runner_protocol::release::UpgradeNotice;

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

/// The notices of one `demi-server` run.
pub struct Notices {
    socket: PathBuf,
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
    /// The notices to the backend the configuration names.
    pub fn new(layout: &Layout) -> Self {
        let socket = Settings::read(&layout.config())
            .ok()
            .and_then(|settings| settings.manage_socket())
            .unwrap_or_else(|| layout.manage_socket());
        let repeated = Arc::new(Repeated::default());
        let repeating = {
            let (repeated, socket) = (repeated.clone(), socket.clone());
            std::thread::spawn(move || repeat(&repeated, &socket))
        };
        Self {
            socket,
            repeated,
            repeating: Some(repeating),
            downloads: VecDeque::new(),
        }
    }

    /// Tells `notice` now, and again every 10 seconds when it is a step that
    /// takes long.
    pub fn tell(&mut self, notice: UpgradeNotice) {
        let lasts = matches!(notice, UpgradeNotice::Downloading { .. } | UpgradeNotice::Preparing { .. });
        // Sent holding the lock, as the repeating thread sends, so the
        // backend hears the notices in the order they were told: a repeated
        // step never arrives after the restart or the failure that ended it.
        let mut state = self.repeated.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.0 = lasts.then(|| notice.clone());
        self.repeated.changed.notify_all();
        send(&self.socket, &notice);
    }

    /// Tells the bytes downloaded of `to` and the rate of the last seconds,
    /// at most once a second and always once all are done.
    pub fn downloaded(&mut self, to: &semver::Version, bytes_done: u64, bytes: u64) {
        let now = Instant::now();
        if self.downloads.back().is_some_and(|(last, _)| now - *last < DOWNLOAD_EVERY) && bytes_done < bytes {
            return;
        }
        while self.downloads.front().is_some_and(|(at, _)| now - *at > RATE_OVER) {
            self.downloads.pop_front();
        }
        let bytes_per_second = match self.downloads.front() {
            Some((at, done)) if now > *at => (bytes_done.saturating_sub(*done) as f64 / (now - *at).as_secs_f64()) as u64,
            _ => 0,
        };
        self.downloads.push_back((now, bytes_done));
        self.tell(UpgradeNotice::Downloading {
            to: to.to_string(),
            bytes_done,
            bytes,
            bytes_per_second,
        });
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
