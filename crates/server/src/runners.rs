//! The devices' download of the next runner: the running backend hears which
//! runner release comes next, over its management socket, and has its
//! connected paired devices download it, once the server got the release
//! ready ahead of an update and again as the update's preparation starts.
//!
//! Nothing waits for them. A thread of its own reads the backend's answers
//! only to say how many devices have the new runner, and ends once all have
//! or once the backend closes the socket as it stops; a device that has not
//! finished downloads after the restart, and shows as updating until it has.

use std::{
    io::{self, BufRead as _, BufReader, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    time::Duration,
};

use demi_runner_protocol::release::{PrefetchAnswer, PrefetchProgress, PrefetchRequest, PrefetchState, RunnerRelease};

/// How long the work ahead of an update waits for the backend to answer its
/// announcement of the next runner release.
const ANNOUNCE_WAIT: Duration = Duration::from_secs(60);

/// The request that names the runner release of the release at `root` as
/// next; none when the release has no runners. A release record that does
/// not decode is an error.
fn request(root: &Path) -> io::Result<Option<PrefetchRequest>> {
    let record = root.join("runners").join("manifest.json");
    let release = match std::fs::read(&record) {
        // The next release may speak other protocols than this program.
        Ok(bytes) => RunnerRelease::decode_intact(&bytes)
            .map_err(|error| io::Error::other(format!("{}: {error}", record.display())))?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    Ok(Some(PrefetchRequest {
        release: release.release,
        root: root.to_owned(),
    }))
}

/// Tells the backend on `socket` that the runner release of the release at
/// `root` comes next, and leaves its devices to download it. A backend whose
/// release has no such socket is told nothing, nor is one when the release
/// has no runners; its devices then download after the restart. Nothing
/// here fails the upgrade but a release record that does not decode.
pub fn start_prefetch(socket: &Path, root: &Path) -> io::Result<()> {
    let Some(request) = request(root)? else {
        return Ok(());
    };
    let socket = socket.to_owned();
    std::thread::Builder::new()
        .name("prefetch".to_owned())
        .spawn(move || follow(&socket, &request, &mut io::stdout()))?;
    Ok(())
}

/// Tells the backend on `socket` that the runner release of the release at
/// `root`, which the server got ready ahead of an update, comes next, and
/// returns once it has asked its connected devices to download it: they do
/// while nothing waits. The backend that is not there, or refuses, is told
/// nothing, and the update's own preparation tells it again.
pub fn announce(socket: &Path, root: &Path) -> io::Result<()> {
    let Some(request) = request(root)? else {
        return Ok(());
    };
    let Ok(stream) = UnixStream::connect(socket) else {
        return Ok(());
    };
    // A backend that does not answer holds the work ahead, and the server's
    // lock with it, no longer than this: an update waits for that lock. The
    // backend asks every connected device before it answers, within seconds.
    stream.set_read_timeout(Some(ANNOUNCE_WAIT))?;
    stream.set_write_timeout(Some(ANNOUNCE_WAIT))?;
    let mut line = serde_json::to_vec(&request).map_err(io::Error::other)?;
    line.push(b'\n');
    let mut writer = &stream;
    if writer.write_all(&line).is_err() {
        return Ok(());
    }
    // The first answer comes once every device was asked; closing then
    // leaves their downloads going.
    let mut answer = String::new();
    let _ = BufReader::new(&stream).read_line(&mut answer);
    Ok(())
}

/// Sends `request` to the backend on `socket` and writes to `out` how far its
/// devices are, until the backend stops answering.
fn follow(socket: &PathBuf, request: &PrefetchRequest, out: &mut impl Write) {
    let stream = match UnixStream::connect(socket) {
        Ok(stream) => stream,
        Err(error) if matches!(error.kind(), io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused) => return,
        Err(error) => {
            let _ = writeln!(out, "The devices download the new runner after the restart: {error}");
            return;
        }
    };
    let last = match wait_for(&stream, request, out) {
        Ok(last) => last,
        Err(reason) => {
            let _ = writeln!(out, "The devices download the new runner after the restart: {reason}");
            return;
        }
    };
    let Some(progress) = last else {
        return;
    };
    for device in &progress.devices {
        let _ = match &device.state {
            PrefetchState::Done => Ok(()),
            PrefetchState::Downloading => writeln!(
                out,
                "{} has not finished and downloads the new runner after the restart",
                device.name
            ),
            PrefetchState::Failed { reason } => writeln!(
                out,
                "{} downloads the new runner after the restart: {reason}",
                device.name
            ),
        };
    }
}

/// Sends `request` on `stream` and reads the backend's answers until no
/// device is downloading or the backend goes away, writing to `out` how many
/// devices have the new runner whenever that changes. The last progress the
/// backend answered, if any; an error says why the backend refused.
fn wait_for(
    stream: &UnixStream,
    request: &PrefetchRequest,
    out: &mut impl Write,
) -> Result<Option<PrefetchProgress>, String> {
    let mut line = serde_json::to_vec(request).map_err(|error| error.to_string())?;
    line.push(b'\n');
    let mut writer = stream;
    writer.write_all(&line).map_err(|error| error.to_string())?;
    let mut answers = BufReader::new(stream);
    let mut last: Option<PrefetchProgress> = None;
    loop {
        let mut line = String::new();
        match answers.read_line(&mut line) {
            // The backend went away, as when it stops.
            Ok(0) | Err(_) => return Ok(last),
            Ok(_) => {}
        }
        let progress = match serde_json::from_str::<PrefetchAnswer>(&line) {
            Ok(PrefetchAnswer::Progress(progress)) => progress,
            Ok(PrefetchAnswer::Refused { error }) => return Err(error),
            Err(error) => return Err(format!("the backend answered {error}")),
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
            let _ = writeln!(out, "{} of {count} {devices} the new runner", done(&progress));
        }
        let finished = progress.finished();
        last = Some(progress);
        if finished {
            return Ok(last);
        }
    }
}
