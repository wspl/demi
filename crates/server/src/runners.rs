//! The devices' download of the next runner: the running backend hears which
//! runner release comes next, over its management socket, and has its
//! connected paired devices download it while the preparation goes on.
//!
//! Nothing waits for them. A thread of its own reads the backend's answers
//! only to say how many devices have the new runner, and ends once all have
//! or once the backend closes the socket as it stops; a device that has not
//! finished downloads after the restart, and shows as updating until it has.

use std::{
    io::{self, BufRead as _, BufReader, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
};

use demi_runner_protocol::release::{PrefetchAnswer, PrefetchProgress, PrefetchRequest, PrefetchState, RunnerRelease};

/// Tells the backend on `socket` that the runner release of the release at
/// `root` comes next, and leaves its devices to download it. A backend whose
/// release has no such socket is told nothing, nor is one when the release
/// has no runners; its devices then download after the restart. Nothing
/// here fails the upgrade but a release record that does not decode.
pub fn start_prefetch(socket: &Path, root: &Path) -> io::Result<()> {
    let record = root.join("runners").join("manifest.json");
    let release = match std::fs::read(&record) {
        // The next release may speak other protocols than this program.
        Ok(bytes) => RunnerRelease::decode_intact(&bytes)
            .map_err(|error| io::Error::other(format!("{}: {error}", record.display())))?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let request = PrefetchRequest {
        release: release.release,
        root: root.to_owned(),
    };
    let socket = socket.to_owned();
    std::thread::Builder::new()
        .name("prefetch".to_owned())
        .spawn(move || follow(&socket, &request, &mut io::stdout()))?;
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
