//! The backend's management socket (`crates/server/src/main.rs`).
//!
//! It is a Unix socket only root may open, on which the next release's
//! `demi-server` names the runner release that comes next. The backend has its
//! connected paired devices' runners download it ahead and answers how far they
//! are, a line of JSON each time one moves on, until none is downloading or the
//! asker stops waiting.
//!
//! The socket takes requests without credentials and acts on every user's
//! devices, so its directory is the backend unit's second runtime directory,
//! apart from the one with the runner socket, which every Cloud mounts: a
//! Cloud's root must never reach it.

use std::io;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::sync::Arc;

use demi_backend_runners::prefetch::{NextRelease, progress};
use demi_runner_protocol::release::{PrefetchAnswer, PrefetchRequest};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::AppState;

/// The longest request line read.
const REQUEST_BYTES: u64 = 64 * 1024;

/// Opens the management socket at `path`, replacing the file a previous
/// backend left, for its owner alone: the backend's user, root on a server.
/// None when the socket's directory does not exist, as in development: no
/// upgrade then tells this backend what comes next.
pub fn manage_socket(path: &Path) -> io::Result<Option<UnixListener>> {
    let directory = path.parent().unwrap_or(Path::new("/"));
    if !directory.is_dir() {
        tracing::info!(
            "no upgrade can announce the next runner release: the management socket's directory {} does not exist",
            directory.display()
        );
        return Ok(None);
    }
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(Some(listener))
}

/// Answers each connection to `listener` until `stop`, then waits for the
/// answers still going.
pub(crate) async fn serve(listener: UnixListener, state: AppState, stop: CancellationToken) {
    let answers = TaskTracker::new();
    loop {
        let accepted = tokio::select! {
            () = stop.cancelled() => break,
            accepted = listener.accept() => accepted,
        };
        match accepted {
            Ok((stream, _)) => {
                answers.spawn(answer(stream, state.clone(), stop.clone()));
            }
            Err(error) => tracing::warn!("a management connection was not accepted: {error}"),
        }
    }
    answers.close();
    answers.wait().await;
}

/// Reads one request and answers it until every device it counts is done,
/// the asker goes away, or the backend stops.
async fn answer(stream: UnixStream, state: AppState, stop: CancellationToken) {
    let (read, mut write) = stream.into_split();
    let mut read = BufReader::new(read.take(REQUEST_BYTES));
    let mut line = String::new();
    let request = tokio::select! {
        () = stop.cancelled() => return,
        read = read.read_line(&mut line) => read,
    };
    if let Err(error) = request {
        tracing::warn!("a management request was not read: {error}");
        return;
    }
    let next = match announce(&state, &line).await {
        Ok(next) => next,
        Err(error) => {
            tracing::warn!("the next runner release was not announced: {error}");
            // An asker that went away needs no refusal.
            let _ = send(&mut write, &PrefetchAnswer::Refused { error }).await;
            return;
        }
    };
    let mut devices = next.progress();
    let mut rest = [0; 64];
    loop {
        let progress = progress(&devices.borrow_and_update());
        let finished = progress.finished();
        if send(&mut write, &PrefetchAnswer::Progress(progress)).await.is_err() || finished {
            return;
        }
        tokio::select! {
            () = stop.cancelled() => return,
            // The asker stopped waiting: what its devices download goes on.
            _ = read.read(&mut rest) => return,
            changed = devices.changed() => {
                if changed.is_err() {
                    return;
                }
            }
        }
    }
}

/// Announces the release `request` names as next, and has every connected
/// paired device's runner download it: each is counted before this answers.
async fn announce(state: &AppState, request: &str) -> Result<Arc<NextRelease>, String> {
    let request = PrefetchRequest::decode(request.as_bytes()).map_err(|error| error.to_string())?;
    let next = state
        .services
        .native
        .next_release(&request.release, &request.root)
        .await?;
    let next = Arc::new(next);
    state.services.prefetches.announce(next.clone());
    tracing::info!("runner release {} comes next; the connected devices download it ahead", request.release);
    let users = state
        .services
        .control
        .paired_owners()
        .await
        .map_err(|error| error.to_string())?;
    for user in users {
        let next = next.clone();
        let asked = state
            .shards
            .of(&user)
            .call(move |shard, _| async move { shard.prefetch(next).await })
            .await;
        if asked.is_err() {
            return Err("the backend is shutting down".into());
        }
    }
    Ok(next)
}

/// Writes `answer` as a line.
async fn send(write: &mut (impl tokio::io::AsyncWrite + Unpin), answer: &PrefetchAnswer) -> io::Result<()> {
    let mut line = serde_json::to_vec(answer).map_err(io::Error::other)?;
    line.push(b'\n');
    write.write_all(&line).await?;
    write.flush().await
}
