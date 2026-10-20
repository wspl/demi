//! The runners step of an upgrade's preparation: the running backend hears
//! which runner release comes next, over its management socket, and has its
//! connected paired devices download it while it still serves; this waits
//! for them, saying how many have it, and goes on once all have or after
//! its limit, naming those that have not: they download after the switch.

use std::{
    io::{self, BufRead as _, BufReader, Write},
    os::unix::net::UnixStream,
    path::Path,
    time::{Duration, Instant},
};

use demi_runner_protocol::release::{
    PrefetchAnswer, PrefetchProgress, PrefetchRequest, PrefetchState, RunnerRelease,
};

/// How long an upgrade waits for the devices' downloads.
pub const WAIT: Duration = Duration::from_secs(2 * 60);

/// Tells the backend on `socket` that the runner release of the release at
/// `root` comes next, and waits up to `wait` for its devices to download it,
/// writing to `out` how many have. A backend whose release has no such
/// socket is told nothing, nor is one when the release has no runners; its
/// devices then download after the switch. Nothing here fails the upgrade
/// but a release record that does not decode, or a failure to write to
/// `out`.
pub fn prefetch(socket: &Path, root: &Path, wait: Duration, out: &mut impl Write) -> io::Result<()> {
    let record = root.join("runners").join("manifest.json");
    let release = match std::fs::read(&record) {
        // The next release may speak other protocols than this program.
        Ok(bytes) => RunnerRelease::decode_intact(&bytes)
            .map_err(|error| io::Error::other(format!("{}: {error}", record.display())))?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let stream = match UnixStream::connect(socket) {
        Ok(stream) => stream,
        Err(error) if matches!(error.kind(), io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused) => {
            return Ok(());
        }
        Err(error) => {
            writeln!(out, "The devices download the new runner after the switch: {error}")?;
            return Ok(());
        }
    };
    let request = PrefetchRequest {
        release: release.release,
        root: root.to_owned(),
    };
    let last = match wait_for(&stream, &request, wait, out) {
        Ok(last) => last,
        Err(Waited::Out(error)) => return Err(error),
        Err(Waited::Backend(reason)) => {
            writeln!(out, "The devices download the new runner after the switch: {reason}")?;
            return Ok(());
        }
    };
    let Some(progress) = last else {
        return Ok(());
    };
    for device in &progress.devices {
        match &device.state {
            PrefetchState::Done => {}
            PrefetchState::Downloading => writeln!(
                out,
                "{} has not finished and downloads the new runner after the switch",
                device.name
            )?,
            PrefetchState::Failed { reason } => writeln!(
                out,
                "{} downloads the new runner after the switch: {reason}",
                device.name
            )?,
        }
    }
    Ok(())
}

/// Why waiting ended early: the output failed, or the backend did.
enum Waited {
    Out(io::Error),
    Backend(String),
}

/// Sends `request` on `stream` and reads the backend's answers until no
/// device is downloading, the backend goes away, or `wait` has passed,
/// writing to `out` how many devices have the new runner whenever that
/// changes. The last progress the backend answered, if any.
fn wait_for(
    stream: &UnixStream,
    request: &PrefetchRequest,
    wait: Duration,
    out: &mut impl Write,
) -> Result<Option<PrefetchProgress>, Waited> {
    let backend = |error: io::Error| Waited::Backend(error.to_string());
    let mut line = serde_json::to_vec(request).map_err(|error| Waited::Backend(error.to_string()))?;
    line.push(b'\n');
    let mut writer = stream;
    writer.write_all(&line).map_err(backend)?;
    let deadline = Instant::now() + wait;
    let mut answers = BufReader::new(stream);
    let mut last: Option<PrefetchProgress> = None;
    loop {
        let rest = deadline.saturating_duration_since(Instant::now());
        if rest.is_zero() {
            return Ok(last);
        }
        match stream.set_read_timeout(Some(rest)) {
            // macOS takes no timeout on a socket its peer closed, whose
            // reads end at once.
            Err(error) if error.kind() == io::ErrorKind::InvalidInput => {}
            other => other.map_err(backend)?,
        }
        let mut line = String::new();
        match answers.read_line(&mut line) {
            // The backend went away, as when it stops.
            Ok(0) => return Ok(last),
            Ok(_) => {}
            Err(error) if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {
                return Ok(last);
            }
            Err(error) => return Err(backend(error)),
        }
        let progress = match serde_json::from_str::<PrefetchAnswer>(&line) {
            Ok(PrefetchAnswer::Progress(progress)) => progress,
            Ok(PrefetchAnswer::Refused { error }) => return Err(Waited::Backend(error)),
            Err(error) => return Err(Waited::Backend(format!("the backend answered {error}"))),
        };
        let done = |progress: &PrefetchProgress| {
            progress
                .devices
                .iter()
                .filter(|device| device.state == PrefetchState::Done)
                .count()
        };
        let moved = last.as_ref().is_none_or(|last| done(last) != done(&progress));
        if moved && !progress.devices.is_empty() {
            let count = progress.devices.len();
            let devices = if count == 1 { "device has" } else { "devices have" };
            writeln!(out, "{} of {count} {devices} the new runner", done(&progress)).map_err(Waited::Out)?;
        }
        let finished = progress.finished();
        last = Some(progress);
        if finished {
            return Ok(last);
        }
    }
}
