//! A scripted machine manager.
//!
//! It serves the manager's socket with `machine-manager-protocol`, as the real
//! manager does, and runs each Cloud's sandbox as a real runner process whose
//! home survives a stop, a wake and a reset.
//!
//! Behavior:
//!
//! - Operations of one device run one at a time, in arrival order.
//! - A device's first wake makes its storage.
//! - A call whose handling panics is answered with the panic, never left
//!   waiting.
//! - Its `reconcile` keeps the sandboxes that run, as the real manager's does.
//!
//! It mounts no image and isolates nothing:
//!
//! - An integration's shared folder is a temporary directory, linked into
//!   each runner's home where the integration's path says: `~/shared` at
//!   `shared` in the home, and a top-level directory such as `/data` at
//!   `data` in the home, since a runner here has no root of its own.
//!
//! - A stop clears the runner's state but its log and its job directories, as
//!   a Cloud's `/run/demi` goes with it while its system image, which holds
//!   those two, stays.
//! - A reset clears them too.
//! - Both keep its home.

use std::collections::{BTreeMap, HashMap};
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::runner::{RunnerProcess, RunnerProcessOptions, runner_binary};
use demi_host_interface::SpawnEnv;
use demi_machine_manager_protocol::{
    BaseVersion, GenerationId, MachineCall, MachineImageState, MachineResponse, RuntimeState, WIRE_VERSION,
    decode_request, encode_line,
};
use futures_util::FutureExt as _;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Semaphore, mpsc};
use tokio_util::task::AbortOnDropHandle;

/// The base every device boots from.
const BASE: &str = "test-base";

/// A device's storage and its sandbox.
struct Guest {
    runner: Option<RunnerProcess>,
    image: MachineImageState,
    generations: u64,
    /// The shared folder its last wake mounted.
    shared: Option<String>,
}

#[derive(Default)]
struct State {
    guests: HashMap<String, Guest>,
    /// The shared folders, by id.
    shared: HashMap<String, tempfile::TempDir>,
    /// One operation at a time per device.
    workers: HashMap<String, Arc<Semaphore>>,
}

struct Shared {
    /// The workspace's `demi-runner`, found when the manager starts, so that a
    /// missing or stale build stops it there rather than in the task of a call.
    runner: PathBuf,
    /// The artifact cache each Cloud runner uses, as `DEMI_ARTIFACTS`; none
    /// keeps its own in its state.
    artifacts: Option<String>,
    state: Mutex<State>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn worker(&self, device: &str) -> Arc<Semaphore> {
        self.lock()
            .workers
            .entry(device.to_owned())
            .or_insert_with(|| Arc::new(Semaphore::new(1)))
            .clone()
    }
}

/// The manager, listening while it lives.
pub struct ScriptedManager {
    socket: PathBuf,
    _directory: tempfile::TempDir,
    _server: AbortOnDropHandle<()>,
}

impl ScriptedManager {
    /// Starts the manager; its Cloud runners use the artifact cache
    /// `artifacts` when given.
    pub fn start(artifacts: Option<String>) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("machines.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let shared = Arc::new(Shared {
            runner: runner_binary(),
            artifacts,
            state: Mutex::new(State::default()),
        });
        let server = tokio::spawn(serve(listener, shared));
        Self {
            socket,
            _directory: directory,
            _server: AbortOnDropHandle::new(server),
        }
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }
}

async fn serve(listener: UnixListener, shared: Arc<Shared>) {
    let mut connections = Vec::new();
    while let Ok((stream, _)) = listener.accept().await {
        connections.push(AbortOnDropHandle::new(tokio::spawn(connection(
            stream,
            shared.clone(),
        ))));
    }
}

/// One backend connection: requests run concurrently and are answered as
/// they finish. A line that is not a request drops the connection, as the real
/// manager does.
async fn connection(stream: UnixStream, shared: Arc<Shared>) {
    let (read, mut write) = stream.into_split();
    let (lines, mut outgoing) = mpsc::channel::<Vec<u8>>(64);
    // A manager's first line names its wire version.
    let hello = encode_line(&MachineResponse::Hello {
        version: WIRE_VERSION,
    });
    lines.try_send(hello).expect("a new connection's queue has room");
    let writer = AbortOnDropHandle::new(tokio::spawn(async move {
        while let Some(line) = outgoing.recv().await {
            if write.write_all(&line).await.is_err() {
                return;
            }
        }
    }));
    let mut requests = Vec::new();
    let mut reader = BufReader::new(read).lines();
    while let Ok(Some(line)) = reader.next_line().await {
        if line.is_empty() {
            continue;
        }
        let Ok(request) = decode_request(&line) else {
            break;
        };
        let (shared, lines) = (shared.clone(), lines.clone());
        requests.push(AbortOnDropHandle::new(tokio::spawn(async move {
            // A call whose handling panics is answered with the panic, so
            // that the backend's wait for it ends with the reason instead of
            // never.
            let handled = AssertUnwindSafe(handle(&shared, request.call)).catch_unwind().await;
            let response = match handled {
                Ok(Ok(result)) => MachineResponse::Ok {
                    id: request.id,
                    result,
                },
                Ok(Err(message)) => MachineResponse::Error {
                    id: request.id,
                    message,
                },
                Err(panic) => MachineResponse::Error {
                    id: request.id,
                    message: panic_message(panic.as_ref()),
                },
            };
            // A connection that closed discards its replies.
            let _ = lines.send(encode_line(&response)).await;
        })));
    }
    drop(lines);
    // The replies still owed are written before the connection ends.
    for request in requests {
        let _ = request.await;
    }
    let _ = writer.await;
}

/// What a panic said: its formatted message, or its literal one.
fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|message| (*message).to_owned()))
        .unwrap_or_else(|| "the manager's handler panicked".into())
}

async fn handle(shared: &Arc<Shared>, call: MachineCall) -> Result<serde_json::Value, String> {
    let json = |value: serde_json::Value| Ok(value);
    match call {
        // The sandboxes that run stay running, and the manager reports them
        // (`crates/machine-manager-protocol/src/lib.rs`).
        MachineCall::Reconcile(_) => {
            let devices: Vec<String> = shared.lock().guests.keys().cloned().collect();
            let mut running = Vec::new();
            for device in devices {
                let worker = shared.worker(&device);
                let _turn = worker.acquire().await.unwrap();
                let runs = shared
                    .lock()
                    .guests
                    .get_mut(&device)
                    .and_then(|guest| guest.runner.as_mut())
                    .is_some_and(RunnerProcess::running);
                if runs {
                    running.push(device);
                }
            }
            json(serde_json::to_value(demi_machine_manager_protocol::Reconciled { running }).unwrap())
        }
        MachineCall::CurrentBaseVersion(_) => json(serde_json::json!(BASE)),
        MachineCall::ImageState(params) => {
            let image = shared
                .lock()
                .guests
                .get(&params.device_id)
                .map(|guest| guest.image.clone());
            json(serde_json::to_value(image).unwrap())
        }
        MachineCall::RuntimeState(params) => {
            let worker = shared.worker(&params.device_id);
            let _turn = worker.acquire().await.unwrap();
            let running = shared
                .lock()
                .guests
                .get_mut(&params.device_id)
                .and_then(|guest| guest.runner.as_mut())
                .is_some_and(RunnerProcess::running);
            let state = if running {
                RuntimeState::Running
            } else {
                RuntimeState::Stopped
            };
            json(serde_json::to_value(state).unwrap())
        }
        MachineCall::Wake(params) => {
            let device = params.device_id;
            let worker = shared.worker(&device);
            let _turn = worker.acquire().await.unwrap();
            let (env, taken) = {
                let mut state = shared.lock();
                let env = match &shared.artifacts {
                    Some(cache) => SpawnEnv::Overlay(BTreeMap::from([("DEMI_ARTIFACTS".to_owned(), Some(cache.clone()))])),
                    None => SpawnEnv::Inherit,
                };
                let capacity = params.capacity;
                let guest = state.guests.entry(device.clone()).or_insert_with(|| Guest {
                    runner: None,
                    image: image(
                        1,
                        BASE,
                        None,
                        capacity.system_bytes.get(),
                        capacity.home_bytes.get(),
                    ),
                    generations: 1,
                    shared: None,
                });
                guest.shared = params.shared.as_ref().map(|shared| shared.id.clone());
                // A smaller disk is enlarged to the capacity named, a larger
                // one kept.
                guest.image.system_bytes = guest.image.system_bytes.max(capacity.system_bytes);
                guest.image.home_bytes = guest.image.home_bytes.max(capacity.home_bytes);
                (env, guest.runner.take())
            };
            let backend = params.boot.backend_url.to_string();
            let token = params.boot.device_token.expose().to_owned();
            let taken = taken.map(|mut runner| {
                let running = runner.running();
                (runner, running)
            });
            let runner = match taken {
                // A sandbox that runs is left as it is.
                Some((running, true)) => Some(running),
                Some((mut stopped, false)) => {
                    stopped.start_again_with_token(&backend, &token);
                    Some(stopped)
                }
                None => {
                    let options = RunnerProcessOptions {
                        name: "cloud".into(),
                        env,
                        token,
                    };
                    Some(RunnerProcess::start_program(shared.runner.clone(), &backend, options))
                }
            };
            if let (Some(folder), Some(runner)) = (&params.shared, &runner) {
                mount_shared(shared, folder, runner.home());
            }
            shared.lock().guests.get_mut(&device).unwrap().runner = runner;
            json(serde_json::Value::Null)
        }
        MachineCall::Delete(params) => {
            let device = params.device_id;
            let worker = shared.worker(&device);
            let _turn = worker.acquire().await.unwrap();
            stop_runner(shared, &device).await;
            // The guest's home goes with its runner.
            shared.lock().guests.remove(&device);
            json(serde_json::Value::Null)
        }
        MachineCall::DeleteShared(params) => {
            let mut state = shared.lock();
            let mounted = state.guests.values_mut().any(|guest| {
                guest.shared.as_deref() == Some(params.id.as_str())
                    && guest.runner.as_mut().is_some_and(RunnerProcess::running)
            });
            if mounted {
                return Err(format!("the shared folder {} is mounted", params.id));
            }
            state.shared.remove(&params.id);
            json(serde_json::Value::Null)
        }
        MachineCall::Hibernate(params) => {
            let device = params.device_id;
            let worker = shared.worker(&device);
            let _turn = worker.acquire().await.unwrap();
            stop_runner(shared, &device).await;
            json(serde_json::Value::Null)
        }
        MachineCall::Checkpoint(params) => {
            let device = params.device_id;
            let worker = shared.worker(&device);
            let _turn = worker.acquire().await.unwrap();
            json(serde_json::Value::Null)
        }
        MachineCall::Reset(params) => {
            let device = params.device_id;
            let worker = shared.worker(&device);
            let _turn = worker.acquire().await.unwrap();
            stop_runner(shared, &device).await;
            let taken = shared
                .lock()
                .guests
                .get_mut(&device)
                .and_then(|guest| guest.runner.take());
            if let Some(mut runner) = taken {
                // A reset replaces the system layer, the log and the job
                // directories with it.
                runner.clear_state();
                shared.lock().guests.get_mut(&device).unwrap().runner = Some(runner);
            }
            let mut state = shared.lock();
            if let Some(guest) = state.guests.get_mut(&device) {
                guest.generations += 1;
                // A fresh system at the capacity named, the home kept.
                guest.image = image(
                    guest.generations,
                    &params.base_version,
                    Some(params.operation_id),
                    params.capacity.system_bytes.get(),
                    guest.image.home_bytes.get(),
                );
            }
            json(serde_json::Value::Null)
        }
    }
}

/// Links the shared folder `folder`, made on its first use, into the
/// runner home `home` where its path says.
fn mount_shared(shared: &Shared, folder: &demi_machine_manager_protocol::SharedFolder, home: &str) {
    let source = {
        let mut state = shared.lock();
        let directory = state
            .shared
            .entry(folder.id.clone())
            .or_insert_with(|| tempfile::tempdir().expect("a shared folder's directory"));
        directory.path().to_path_buf()
    };
    let inside = folder
        .path
        .strip_prefix(demi_machine_manager_protocol::shared::SANDBOX_HOME)
        .unwrap_or(&folder.path)
        .trim_start_matches('/');
    let target = Path::new(home).join(inside);
    if let Some(parent) = target.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // A link of an earlier wake, to the same directory or an earlier path's,
    // is replaced.
    let _ = std::fs::remove_file(&target);
    std::os::unix::fs::symlink(&source, &target).expect("a shared folder's link");
}

/// Stops the device's runner, if it runs, and clears its state but its
/// log and its job directories, as a stop takes a Cloud's temporary mounts
/// and keeps its system image; the caller holds the device's turn.
async fn stop_runner(shared: &Shared, device: &str) {
    let taken = shared
        .lock()
        .guests
        .get_mut(device)
        .and_then(|guest| guest.runner.take());
    let Some(mut runner) = taken else {
        return;
    };
    runner.stop().await;
    runner.clear_run_state();
    if let Some(guest) = shared.lock().guests.get_mut(device) {
        guest.runner = Some(runner);
    }
}

fn image(
    generation: u64,
    base: &str,
    reset: Option<String>,
    system: u64,
    home: u64,
) -> MachineImageState {
    MachineImageState {
        generation: GenerationId::parse(format!("gen-{generation}")).unwrap(),
        base_version: BaseVersion::parse(base).unwrap(),
        reset_id: reset,
        system_bytes: system.try_into().unwrap(),
        home_bytes: home.try_into().unwrap(),
    }
}
