//! A machine manager the tests script (`scenarios.md` § System under
//! test): it serves the manager's socket with `machines-protocol`, as the
//! real manager does, and runs each Cloud's sandbox as a real runner
//! process whose home survives a stop, a wake and a reset. Operations of one
//! device run one at a time, in arrival order; a device's first wake makes
//! its storage. It records every call, and a test can hold a reset, fail
//! one, keep a wake from starting its runner, or kill a runner as a crash
//! would, which the manager reports as a death to every connection. It
//! mounts no image and isolates nothing: a reset clears the runner's state
//! and keeps its home.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use demi_host_remote::testing::{RunnerProcess, RunnerProcessOptions};
use demi_machines_protocol::{
    BaseVersion, GenerationId, MachineCall, MachineImageState, MachineResponse, RuntimeState, decode_request,
    encode_line,
};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Notify, Semaphore, broadcast, mpsc};
use tokio_util::task::AbortOnDropHandle;

/// The base every device boots from.
pub const BASE: &str = "test-base";

/// The capacity of each of a device's filesystems until it grows.
const VOLUME_BYTES: u64 = 1 << 30;

/// What the manager does differently, as a test sets it.
#[derive(Default)]
pub struct Script {
    /// Fail the next reset with this message.
    pub fail_reset: Option<String>,
    /// Hold each reset at its disk step: the first is notified once one
    /// arrives there, and the reset goes on once the second is.
    pub hold_reset: Option<(Arc<Notify>, Arc<Notify>)>,
    /// Wakes start no runner, as a guest that never connects.
    pub silent_wake: bool,
    /// Fail the next hibernate with this message.
    pub fail_hibernate: Option<String>,
}

/// A device's storage and its sandbox.
struct Guest {
    runner: Option<RunnerProcess>,
    image: MachineImageState,
    generations: u64,
}

#[derive(Default)]
struct State {
    calls: Vec<String>,
    guests: HashMap<String, Guest>,
    /// One operation at a time per device.
    workers: HashMap<String, Arc<Semaphore>>,
    script: Script,
}

struct Shared {
    state: Mutex<State>,
    deaths: broadcast::Sender<String>,
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
    shared: Arc<Shared>,
    _directory: tempfile::TempDir,
    _server: AbortOnDropHandle<()>,
}

impl ScriptedManager {
    pub fn start() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("machines.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let (deaths, _) = broadcast::channel(64);
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            deaths,
        });
        let server = tokio::spawn(serve(listener, shared.clone()));
        Self {
            socket,
            shared,
            _directory: directory,
            _server: AbortOnDropHandle::new(server),
        }
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// Every call so far, such as `wake:<device>`, in arrival order.
    pub fn calls(&self) -> Vec<String> {
        self.shared.lock().calls.clone()
    }

    /// How many calls so far are `call`.
    pub fn count(&self, call: &str) -> usize {
        self.calls().iter().filter(|recorded| *recorded == call).count()
    }

    /// Changes what the manager does from now on.
    pub fn script(&self, change: impl FnOnce(&mut Script)) {
        change(&mut self.shared.lock().script);
    }

    /// The devices whose storage the manager made.
    pub fn devices(&self) -> Vec<String> {
        let mut devices: Vec<String> = self.shared.lock().guests.keys().cloned().collect();
        devices.sort();
        devices
    }

    /// Whether the device's runner runs.
    pub fn running(&self, device: &str) -> bool {
        self.shared
            .lock()
            .guests
            .get_mut(device)
            .and_then(|guest| guest.runner.as_mut())
            .is_some_and(RunnerProcess::running)
    }

    /// The home the device's runner reports, where the Cloud's files are.
    pub fn home(&self, device: &str) -> String {
        self.shared.lock().guests[device]
            .runner
            .as_ref()
            .expect("the device booted")
            .home()
            .to_owned()
    }

    /// Kills the device's runner as a crash would, and reports its death.
    pub async fn kill(&self, device: &str) {
        let worker = self.shared.worker(device);
        let _turn = worker.acquire().await.unwrap();
        let taken = self
            .shared
            .lock()
            .guests
            .get_mut(device)
            .and_then(|guest| guest.runner.take());
        let Some(mut runner) = taken else {
            return;
        };
        runner.kill().await;
        if let Some(guest) = self.shared.lock().guests.get_mut(device) {
            guest.runner = Some(runner);
        }
        // Every connection hears it; none may be open.
        let _ = self.shared.deaths.send(device.to_owned());
    }

    /// Stops the device's runner the way a manager restart does: without a
    /// death event.
    pub async fn stop_quietly(&self, device: &str) {
        let worker = self.shared.worker(device);
        let _turn = worker.acquire().await.unwrap();
        stop_runner(&self.shared, device).await;
    }
}

async fn serve(listener: UnixListener, shared: Arc<Shared>) {
    let mut connections = Vec::new();
    while let Ok((stream, _)) = listener.accept().await {
        connections.push(AbortOnDropHandle::new(tokio::spawn(connection(stream, shared.clone()))));
    }
}

/// One backend connection: requests run concurrently and are answered as
/// they finish; deaths go out as they happen. A line that is not a request
/// drops the connection, as the real manager does.
async fn connection(stream: UnixStream, shared: Arc<Shared>) {
    let (read, mut write) = stream.into_split();
    let (lines, mut outgoing) = mpsc::channel::<Vec<u8>>(64);
    let mut deaths = shared.deaths.subscribe();
    let writer = AbortOnDropHandle::new(tokio::spawn(async move {
        loop {
            let line = tokio::select! {
                line = outgoing.recv() => match line {
                    Some(line) => line,
                    None => return,
                },
                death = deaths.recv() => match death {
                    Ok(device) => encode_line(&MachineResponse::Death { device_id: device }),
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return,
                },
            };
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
            let response = match handle(&shared, request.call).await {
                Ok(result) => MachineResponse::Ok { id: request.id, result },
                Err(message) => MachineResponse::Error { id: request.id, message },
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

async fn handle(shared: &Arc<Shared>, call: MachineCall) -> Result<serde_json::Value, String> {
    let json = |value: serde_json::Value| Ok(value);
    match call {
        MachineCall::Reconcile(_) => {
            shared.lock().calls.push("reconcile".into());
            let devices: Vec<String> = shared.lock().guests.keys().cloned().collect();
            for device in devices {
                let worker = shared.worker(&device);
                let _turn = worker.acquire().await.unwrap();
                stop_runner(shared, &device).await;
            }
            json(serde_json::Value::Null)
        }
        MachineCall::CurrentBaseVersion(_) => json(serde_json::json!(BASE)),
        MachineCall::ImageState(params) => {
            let image = shared.lock().guests.get(&params.device_id).map(|guest| guest.image.clone());
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
            let state = if running { RuntimeState::Running } else { RuntimeState::Stopped };
            json(serde_json::to_value(state).unwrap())
        }
        MachineCall::Wake(params) => {
            let device = params.device_id;
            let worker = shared.worker(&device);
            let _turn = worker.acquire().await.unwrap();
            shared.lock().calls.push(format!("wake:{device}"));
            let (silent, taken) = {
                let mut state = shared.lock();
                let silent = state.script.silent_wake;
                let guest = state.guests.entry(device.clone()).or_insert_with(|| Guest {
                    runner: None,
                    image: image(1, None, VOLUME_BYTES, VOLUME_BYTES),
                    generations: 1,
                });
                (silent, guest.runner.take())
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
                // A guest that never connects: its runner does not start.
                taken if silent => taken.map(|(runner, _)| runner),
                Some((mut stopped, false)) => {
                    stopped.start_again_with_token(&backend, &token);
                    Some(stopped)
                }
                None => {
                    let options = RunnerProcessOptions {
                        name: "cloud".into(),
                        token: Some(token),
                        managed: true,
                        ..RunnerProcessOptions::default()
                    };
                    Some(RunnerProcess::start(&backend, options))
                }
            };
            shared.lock().guests.get_mut(&device).unwrap().runner = runner;
            json(serde_json::Value::Null)
        }
        MachineCall::Hibernate(params) => {
            let device = params.device_id;
            let worker = shared.worker(&device);
            let _turn = worker.acquire().await.unwrap();
            shared.lock().calls.push(format!("hibernate:{device}"));
            if let Some(message) = shared.lock().script.fail_hibernate.take() {
                return Err(message);
            }
            stop_runner(shared, &device).await;
            json(serde_json::Value::Null)
        }
        MachineCall::Checkpoint(params) => {
            let device = params.device_id;
            let worker = shared.worker(&device);
            let _turn = worker.acquire().await.unwrap();
            shared.lock().calls.push(format!("checkpoint:{device}"));
            json(serde_json::Value::Null)
        }
        MachineCall::GrowVolume(params) => {
            let device = params.device_id;
            let worker = shared.worker(&device);
            let _turn = worker.acquire().await.unwrap();
            shared
                .lock()
                .calls
                .push(format!("grow:{device}:{}:{}", params.volume, params.bytes));
            let mut state = shared.lock();
            let guest = state.guests.get_mut(&device).ok_or("the device has no storage")?;
            if params.bytes.get() > guest.image.bytes(params.volume).get() {
                guest.image = guest.image.clone().with_bytes(params.volume, params.bytes);
            }
            json(serde_json::Value::Null)
        }
        MachineCall::Reset(params) => {
            let device = params.device_id;
            let worker = shared.worker(&device);
            let _turn = worker.acquire().await.unwrap();
            shared
                .lock()
                .calls
                .push(format!("reset:{device}:{}:{}", params.operation_id, params.base_version));
            let hold = shared.lock().script.hold_reset.clone();
            if let Some((rebuilding, proceed)) = hold {
                rebuilding.notify_one();
                proceed.notified().await;
            }
            if let Some(message) = shared.lock().script.fail_reset.take() {
                return Err(message);
            }
            stop_runner(shared, &device).await;
            let taken = shared.lock().guests.get_mut(&device).and_then(|guest| guest.runner.take());
            if let Some(mut runner) = taken {
                runner.clear_state();
                shared.lock().guests.get_mut(&device).unwrap().runner = Some(runner);
            }
            let mut state = shared.lock();
            if let Some(guest) = state.guests.get_mut(&device) {
                guest.generations += 1;
                guest.image = image(
                    guest.generations,
                    Some(params.operation_id),
                    guest.image.system_bytes.get(),
                    guest.image.home_bytes.get(),
                );
            }
            json(serde_json::Value::Null)
        }
    }
}

/// Stops the device's runner, if it runs; the caller holds the device's
/// turn.
async fn stop_runner(shared: &Shared, device: &str) {
    let taken = shared.lock().guests.get_mut(device).and_then(|guest| guest.runner.take());
    let Some(mut runner) = taken else {
        return;
    };
    runner.stop().await;
    if let Some(guest) = shared.lock().guests.get_mut(device) {
        guest.runner = Some(runner);
    }
}

fn image(generation: u64, reset: Option<String>, system: u64, home: u64) -> MachineImageState {
    MachineImageState {
        generation: GenerationId::parse(format!("gen-{generation}")).unwrap(),
        base_version: BaseVersion::parse(BASE).unwrap(),
        reset_id: reset,
        system_bytes: system.try_into().unwrap(),
        home_bytes: home.try_into().unwrap(),
    }
}
