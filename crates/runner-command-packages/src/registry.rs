//! The resident registry: one owner task keeps a registration's resident
//! residents by artifact digest, counts the leases on each, and decides when a
//! resident ends.
//!
//! A lease holder tells the registry when it takes its lease. The registry sees
//! the lease end when its holder drops it. Nobody waits for the decision: the
//! connection's message handling never blocks on it.
//!
//! A resident stays while anything leases it:
//!
//! - a live execution context whose pinned manifest names its artifact;
//! - an open user stream running in it;
//! - a start in progress, for a call waiting until the resident is ready;
//! - the connection's installed manifests, so consecutive jobs reuse the
//!   resident: for each package, the release the latest installed manifest
//!   naming it names (an older release keeps none, so after the startup catalog
//!   moves a package, the old resident ends with its last job);
//! - the release the connection's user streams bound last, one per package,
//!   until the connection ends or a stream binds another release.
//!
//! Callers that want one resident while it starts share its start, which
//! locates its executable through whichever of them is still waiting
//! (`askers`): the caller that asked first may leave, as a stream its page
//! gave up does, and the start goes on for the others.
//!
//! A resident whose leases all ended is asked which conversations it holds:
//!
//! - An answer counts only when no lease came or went and no release ended
//!   while it was on its way. The resident answers from what it holds at that
//!   moment: a check that follows one release can be answered while a second
//!   release still runs, and trusting it would keep a resident that holds
//!   nothing.
//! - A check that fails or takes over 5 seconds keeps the resident and is
//!   logged. A resident that cannot say what it holds is not one that holds
//!   nothing, and stopping it would end every conversation it serves.
//!
//! Against its leases a resident is retired only as faulty: it exits, breaks the
//! protocol, fails a conversation release, or keeps a handler past the
//! cancellation grace. Losing the backend connection stops every resident, and
//! the backend stops a package's residents when the user turns its plugin off
//! (`crates/plugin-host/src/lib.rs`).
//!
//! For example, a job whose manifest names `demi.browser` holds a lease on its
//! artifact while the job runs, and the installed manifest holds another. When
//! the connection installs a manifest with a newer `demi.browser`, the old
//! artifact's last lease ends when its last job does. The registry then asks
//! the old resident which conversations it holds: it stops a resident that holds
//! none, and keeps one that holds some or cannot say.

use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use demi_command_protocol::{
    ArtifactForm, ArtifactProgress, ConversationRequest, ConversationStatus, PackageDescriptor,
    Record, ResidentInfo, megabytes,
};
use demi_command_sdk::{Client, ResidentError};
use demi_runner_process::backend::Backend;
use demi_runner_protocol::wire::HostArtifact;
use tokio::{
    sync::{mpsc, oneshot, watch},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

use demi_command_protocol::host_target;

/// The line name of a package's executable (`crates/runner-command-packages/src/cache.rs`).
const PROGRAM: &str = "program";

use crate::{
    askers::{Askers, Asking},
    ArtifactResolver, ResidentBackend, RuntimeError,
    cache::{ArtifactCache, Downloading, Wanted},
    invocations::{Invocations, Invoking, ResidentArtifacts},
    process::ResidentProcess,
};

/// How long a resident has to say which conversations it holds.
const STATUS_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a resident has to release a conversation: long enough for a
/// browser to close its tabs and remove its profile.
const RELEASE_TIMEOUT: Duration = Duration::from_secs(360);
/// The most a conversation operation's answer may hold: it is held whole in
/// memory, and a resident may answer without end.
const ANSWER_BYTES: usize = 1024 * 1024;
/// Requests waiting for the owner; a sender waits for room, so that a slow
/// owner does not grow memory.
const REQUESTS: usize = 64;

/// The registry's owner task and the handle to it.
pub struct ResidentRegistry {
    handle: ResidentHandle,
    contents: watch::Receiver<Vec<HostArtifact>>,
    downloading: watch::Receiver<Vec<Downloading>>,
    owner: tokio::task::JoinHandle<()>,
}

impl ResidentRegistry {
    /// Residents start in `cwd` with exactly `env`; their artifacts are
    /// cached in `cache`, or taken from the copies the Host's image
    /// preinstalled in `image`, when given, and those the backend serves
    /// itself come the way `backend` reaches it.
    pub async fn new(
        cache: PathBuf,
        image: Option<PathBuf>,
        backend: Option<Backend>,
        cwd: PathBuf,
        env: BTreeMap<String, String>,
    ) -> Result<Self, RuntimeError> {
        let cache = Arc::new(ArtifactCache::new(cache, image, backend).await?);
        let contents = cache.contents();
        let downloading = cache.downloading();
        let invocations = Invocations::default();
        let (requests, receiver) = mpsc::channel(REQUESTS);
        let owner = Owner {
            cache: cache.clone(),
            invocations: invocations.clone(),
            cwd,
            env,
            entries: HashMap::new(),
            leases: JoinSet::new(),
            generation: 0,
            lifecycles: JoinSet::new(),
            checks: JoinSet::new(),
            work: JoinSet::new(),
            stop: CancellationToken::new(),
        };
        Ok(Self {
            handle: ResidentHandle {
                requests,
                invocations,
                cache,
            },
            contents,
            downloading,
            owner: tokio::spawn(owner.run(receiver)),
        })
    }

    pub fn handle(&self) -> ResidentHandle {
        self.handle.clone()
    }

    /// What the artifact cache holds, and each later list
    /// (`crates/runner-command-packages/src/cache.rs`).
    pub fn installed(&self) -> watch::Receiver<Vec<HostArtifact>> {
        self.contents.clone()
    }

    /// What the artifact cache is downloading, and each later list.
    pub fn downloading(&self) -> watch::Receiver<Vec<Downloading>> {
        self.downloading.clone()
    }

    /// Stops every resident and waits until each has ended. The owner also
    /// ends by itself once every handle is gone.
    pub async fn close(self) {
        // An owner that has ended already has nothing left to stop.
        let _closed = self.handle.requests.send(Request::Close).await;
        self.owner
            .await
            .expect("the resident registry does not panic");
    }
}

enum Request {
    /// A lease on `digest` that lasts until `alive` closes.
    Lease {
        digest: String,
        alive: oneshot::Receiver<()>,
    },
    Acquire {
        descriptor: PackageDescriptor,
        resolver: Arc<dyn ArtifactResolver>,
        backend: Arc<dyn ResidentBackend>,
        /// The caller's cancellation, which takes it off the start's askers.
        cancel: CancellationToken,
        reply: oneshot::Sender<Result<Acquired, RuntimeError>>,
    },
    Release {
        conversation: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Stops the residents of the packages named, or every resident.
    Stop {
        packages: Option<Vec<String>>,
        reply: oneshot::Sender<()>,
    },
    Close,
}

/// A resident's life as its callers see it.
#[derive(Clone)]
enum State {
    /// Its executable is being installed, with how far the install got,
    /// or, without one, the program is starting.
    Starting(Option<Installing>),
    Ready {
        client: Client,
        info: Arc<ResidentInfo>,
    },
    Ended(Arc<RuntimeError>),
}

/// How far the install of a resident's executable got: none before its
/// download reported anything.
#[derive(Clone, Copy, Default)]
struct Installing {
    progress: Option<ArtifactProgress>,
}

/// How long a resident's install goes unsaid before its caller's command
/// tells of it (`crates/runner-command-packages/src/cache.rs`).
const QUIET_INSTALL: Duration = Duration::from_secs(2);

struct Acquired {
    /// Held while the caller waits for the start.
    _waiting: ResidentLease,
    /// The caller's place among the start's askers while it waits.
    _asking: Asking,
    state: watch::Receiver<State>,
}

/// Sends requests to the registry. Cloning shares the one registry.
#[derive(Clone)]
pub struct ResidentHandle {
    requests: mpsc::Sender<Request>,
    invocations: Invocations,
    cache: Arc<ArtifactCache>,
}

/// Which programs [`ResidentHandle::keep_programs`] put in the cache, and
/// which it could not, with why.
#[derive(Debug, Default)]
pub struct ProgramsKept {
    pub kept: Vec<String>,
    pub failed: Vec<(String, String)>,
}

impl ResidentHandle {
    /// Puts the program of each of `packages` for this Host's target in the
    /// artifact cache, located by `resolver`, one at a time, so a resident's
    /// start finds it there. Ahead of an update (`ahead`) the programs the
    /// runner runs now stay, and those of a release downloaded ahead before
    /// go. A package without a program for this target
    /// is passed over; one that fails does not stop the others.
    pub async fn keep_programs(
        &self,
        packages: &[PackageDescriptor],
        resolver: Arc<dyn ArtifactResolver>,
        ahead: bool,
        cancel: &CancellationToken,
    ) -> ProgramsKept {
        let mut kept = ProgramsKept::default();
        let mut release = std::collections::HashSet::new();
        for descriptor in packages {
            let Some(artifact) = descriptor.targets.get(host_target()) else {
                continue;
            };
            release.insert(artifact.sha256.clone());
            let wanted = Wanted {
                package: &descriptor.id,
                name: PROGRAM,
                version: &descriptor.version,
                artifact: artifact.clone(),
                form: &ArtifactForm::File,
            };
            let placed = if ahead {
                self.cache.prefetch(&wanted, resolver.clone(), cancel).await
            } else {
                self.cache.install(&wanted, resolver.clone(), &|_| {}, cancel).await
            };
            match placed {
                Ok(_) => kept.kept.push(descriptor.id.clone()),
                Err(RuntimeError::Cancelled) => break,
                Err(error) => kept.failed.push((descriptor.id.clone(), error.to_string())),
            }
        }
        // The programs of a release downloaded ahead before this one go,
        // once this one's are in place.
        if ahead && !cancel.is_cancelled() {
            self.cache.keep_ahead(&release).await;
        }
        kept
    }

    /// Registers `invocation`, which the caller starts in a resident of
    /// `package` for the work `resolver` locates artifacts for, until the
    /// returned guard drops: the artifacts the program asks for during it
    /// are located for that work (`crates/command-protocol/src/artifacts.rs`).
    pub fn invoking(
        &self,
        invocation: &str,
        package: &str,
        resolver: Arc<dyn ArtifactResolver>,
    ) -> Invoking {
        self.invocations.register(invocation, package, resolver)
    }

    /// A lease that keeps the resident of `digest` alive until it drops,
    /// whether or not that resident runs yet.
    pub async fn lease(&self, digest: String) -> ResidentLease {
        let (lease, alive) = ResidentLease::new();
        // A closed registry keeps nothing; the lease then means nothing.
        let _closed = self.requests.send(Request::Lease { digest, alive }).await;
        lease
    }

    /// The resident of `descriptor`'s artifact for this host, started when
    /// none runs, with its requests to the backend going to `backend`. Callers asking
    /// at once share one start.
    pub async fn acquire(
        &self,
        descriptor: &PackageDescriptor,
        resolver: Arc<dyn ArtifactResolver>,
        backend: Arc<dyn ResidentBackend>,
        cancel: &CancellationToken,
    ) -> Result<Resident, Arc<RuntimeError>> {
        self.acquire_telling(descriptor, resolver, backend, cancel, |_| {})
            .await
    }

    /// [`Self::acquire`] for a command, which `tell`s its caller of an
    /// install of the resident's executable that takes longer than two
    /// seconds, in one line that names it, so that what the command prints
    /// after it is not pushed out of a reader that keeps only the first
    /// lines.
    pub async fn acquire_telling(
        &self,
        descriptor: &PackageDescriptor,
        resolver: Arc<dyn ArtifactResolver>,
        backend: Arc<dyn ResidentBackend>,
        cancel: &CancellationToken,
        mut tell: impl FnMut(String) + Send,
    ) -> Result<Resident, Arc<RuntimeError>> {
        let (reply, answer) = oneshot::channel();
        self.requests
            .send(Request::Acquire {
                descriptor: descriptor.clone(),
                resolver,
                backend,
                cancel: cancel.clone(),
                reply,
            })
            .await
            .map_err(|_| Arc::new(RuntimeError::Cancelled))?;
        let acquired = tokio::select! {
            _ = cancel.cancelled() => return Err(Arc::new(RuntimeError::Cancelled)),
            answer = answer => answer.map_err(|_| Arc::new(RuntimeError::Cancelled))?.map_err(Arc::new)?,
        };
        let mut state = acquired.state;
        let quiet = tokio::time::sleep(QUIET_INSTALL);
        tokio::pin!(quiet);
        // Whether the caller was told of the install, or nothing installs.
        let mut told = false;
        loop {
            let current = state.borrow_and_update().clone();
            match current {
                State::Starting(_) => {}
                State::Ready { client, info } => {
                    if !descriptor.serves(&info) {
                        return Err(Arc::new(RuntimeError::CatalogMismatch));
                    }
                    return Ok(Resident { client, state });
                }
                State::Ended(error) => return Err(error),
            }
            tokio::select! {
                _ = cancel.cancelled() => return Err(Arc::new(RuntimeError::Cancelled)),
                () = &mut quiet, if !told => {
                    told = true;
                    let State::Starting(Some(installing)) = *state.borrow() else {
                        // Nothing installs: nothing is ever told.
                        continue;
                    };
                    let size = match installing.progress {
                        Some(ArtifactProgress::Download { total, .. }) => total,
                        _ => descriptor
                            .targets
                            .get(host_target())
                            .map_or(0, |artifact| artifact.size),
                    };
                    tell(format!(
                        "Installing {} {} ({} MB)…\n",
                        descriptor.id,
                        descriptor.version,
                        megabytes(size)
                    ));
                }
                changed = state.changed() => {
                    if changed.is_err() {
                        return Err(Arc::new(RuntimeError::Cancelled));
                    }
                }
            }
        }
    }

    /// Releases `conversation` in every running resident, all at once, and
    /// retires each resident whose release fails.
    pub async fn release_conversation(&self, conversation: &str) -> Result<(), String> {
        let (reply, answer) = oneshot::channel();
        self.requests
            .send(Request::Release {
                conversation: conversation.to_owned(),
                reply,
            })
            .await
            .map_err(|_| "the resident registry is closed".to_owned())?;
        answer
            .await
            .map_err(|_| "the resident registry is closed".to_owned())?
    }

    /// Stops every resident, as losing the backend connection does
    /// (`crates/runner/src/connection/kept.rs`), and waits until each has ended.
    pub async fn stop_all(&self) {
        self.stop(None).await;
    }

    /// Stops the residents of `packages`, by their ids, and waits until each
    /// has ended: a resident that stops releases every conversation it
    /// holds, and the next caller of a package starts it again.
    pub async fn stop_packages(&self, packages: Vec<String>) {
        self.stop(Some(packages)).await;
    }

    async fn stop(&self, packages: Option<Vec<String>>) {
        let (reply, answer) = oneshot::channel();
        if self.requests.send(Request::Stop { packages, reply }).await.is_ok() {
            // A closed registry has stopped everything already.
            let _closed = answer.await;
        }
    }
}

/// One claim on a digest's resident; dropping it ends the claim, which the
/// registry sees as the closing of its `alive` channel.
pub struct ResidentLease {
    _alive: oneshot::Sender<()>,
}

impl ResidentLease {
    fn new() -> (Self, oneshot::Receiver<()>) {
        let (alive, watched) = oneshot::channel();
        (Self { _alive: alive }, watched)
    }
}

/// A running resident a caller invokes.
pub struct Resident {
    client: Client,
    state: watch::Receiver<State>,
}

impl Resident {
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// What a call that failed with `error` reports. When the call failed
    /// because its resident went away, that is how the resident ended, with
    /// its exit status and the end of its standard error.
    pub async fn failure(&mut self, error: ResidentError) -> String {
        let connection_lost = match &error {
            ResidentError::Http2(error) => error.is_io() || error.is_go_away(),
            ResidentError::Io(_) => true,
            _ => false,
        };
        if connection_lost {
            // The resident's owner publishes its end once it has reaped it.
            let ended = self
                .state
                .wait_for(|state| matches!(state, State::Ended(_)))
                .await;
            if let Ok(state) = ended
                && let State::Ended(ending) = &*state
            {
                return ending.to_string();
            }
        }
        error.to_string()
    }
}

struct Owner {
    cache: Arc<ArtifactCache>,
    invocations: Invocations,
    cwd: PathBuf,
    env: BTreeMap<String, String>,
    entries: HashMap<String, Entry>,
    /// One watch per lease, ending with its digest when the lease drops.
    leases: JoinSet<String>,
    /// Numbers the residents started, so a late report about one that has
    /// ended is not taken for its successor.
    generation: u64,
    lifecycles: JoinSet<(String, u64)>,
    checks: JoinSet<Checked>,
    work: JoinSet<Work>,
    /// Stops every resident; each one's own token is a child of it.
    stop: CancellationToken,
}

/// One artifact digest: the leases on it and its current resident.
#[derive(Default)]
struct Entry {
    leases: usize,
    /// Counts the lease changes and the releases that ended, so a status
    /// answer from before the last of them is asked again.
    changes: u64,
    current: Option<Current>,
}

struct Current {
    generation: u64,
    id: String,
    stop: CancellationToken,
    state: watch::Sender<State>,
    checking: bool,
    /// The callers waiting for its start.
    askers: Askers,
}

struct Checked {
    digest: String,
    generation: u64,
    changes: u64,
    holds: Result<bool, String>,
}

/// What a release task hands back to the owner.
enum Work {
    /// Residents whose release failed, to retire before `reply` goes out.
    Released {
        failed: Vec<(String, u64)>,
        errors: Vec<String>,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Retirements that have ended; idle residents are reconsidered.
    Retired {
        result: Result<(), String>,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Stopped {
        reply: oneshot::Sender<()>,
    },
}

impl Owner {
    async fn run(mut self, mut requests: mpsc::Receiver<Request>) {
        loop {
            tokio::select! {
                request = requests.recv() => match request {
                    Some(Request::Close) | None => break,
                    Some(request) => self.request(request),
                },
                Some(ended) = self.leases.join_next() => {
                    let digest = ended.expect("lease watches do not panic");
                    self.unlease(&digest);
                }
                Some(ended) = self.lifecycles.join_next() => {
                    let (digest, generation) = ended.expect("resident lifecycles do not panic");
                    self.ended(&digest, generation);
                }
                Some(checked) = self.checks.join_next() => {
                    self.checked(checked.expect("status checks do not panic"));
                }
                Some(work) = self.work.join_next() => {
                    self.worked(work.expect("release work does not panic"));
                }
            }
        }
        self.stop.cancel();
        self.leases.shutdown().await;
        self.checks.shutdown().await;
        self.work.shutdown().await;
        while self.lifecycles.join_next().await.is_some() {}
    }

    /// Counts a lease on `digest` until `alive` closes.
    fn lease(&mut self, digest: String, alive: oneshot::Receiver<()>) {
        let entry = self.entries.entry(digest.clone()).or_default();
        entry.leases += 1;
        entry.changes += 1;
        self.leases.spawn(async move {
            // The holder never sends; the channel closes when it drops.
            let _closed = alive.await;
            digest
        });
    }

    fn unlease(&mut self, digest: &str) {
        if let Some(entry) = self.entries.get_mut(digest) {
            entry.leases -= 1;
            entry.changes += 1;
        }
        self.consider(digest);
    }

    fn request(&mut self, request: Request) {
        match request {
            Request::Lease { digest, alive } => self.lease(digest, alive),
            Request::Acquire {
                descriptor,
                resolver,
                backend,
                cancel,
                reply,
            } => {
                let acquired = self.acquire(descriptor, resolver, backend, &cancel);
                // A caller that gave up drops its lease with the answer.
                let _gave_up = reply.send(acquired);
            }
            Request::Release {
                conversation,
                reply,
            } => self.release(conversation, reply),
            Request::Stop { packages, reply } => {
                let ending = self.retire_where(|_, current| {
                    packages.as_ref().is_none_or(|packages| packages.contains(&current.id))
                });
                self.work.spawn(async move {
                    for mut state in ending {
                        let _ended = state
                            .wait_for(|state| matches!(state, State::Ended(_)))
                            .await;
                    }
                    Work::Stopped { reply }
                });
            }
            Request::Close => unreachable!("the owner loop ends on close"),
        }
    }

    fn acquire(
        &mut self,
        descriptor: PackageDescriptor,
        resolver: Arc<dyn ArtifactResolver>,
        backend: Arc<dyn ResidentBackend>,
        cancel: &CancellationToken,
    ) -> Result<Acquired, RuntimeError> {
        if self.stop.is_cancelled() {
            return Err(RuntimeError::Cancelled);
        }
        let artifact = descriptor
            .targets
            .get(host_target())
            .ok_or(RuntimeError::CatalogMismatch)?
            .clone();
        let digest = artifact.sha256.clone();
        // The waiting caller's lease counts at once, so a start never begins
        // with nothing holding it.
        let (waiting, alive) = ResidentLease::new();
        self.lease(digest.clone(), alive);
        let entry = self
            .entries
            .get_mut(&digest)
            .expect("the lease made the entry");
        // A resident that has just ended, before its owner reported it, is
        // one to start again.
        if let Some(current) = &entry.current
            && !matches!(&*current.state.borrow(), State::Ended(_))
        {
            return Ok(Acquired {
                _waiting: waiting,
                _asking: current.askers.join(resolver, cancel),
                state: current.state.subscribe(),
            });
        }
        self.generation += 1;
        let generation = self.generation;
        let (state, receiver) = watch::channel(State::Starting(None));
        let stop = self.stop.child_token();
        let askers = Askers::default();
        let asking = askers.join(resolver, cancel);
        entry.current = Some(Current {
            generation,
            id: descriptor.id.clone(),
            stop: stop.clone(),
            state: state.clone(),
            checking: false,
            askers: askers.clone(),
        });
        let cache = self.cache.clone();
        let cwd = self.cwd.clone();
        let env = self.env.clone();
        let invocations = self.invocations.clone();
        self.lifecycles.spawn(async move {
            let started = Start {
                artifact: &artifact,
                descriptor: &descriptor,
                invocations,
                resolver: Arc::new(askers),
                backend,
                cwd: &cwd,
                env: &env,
            };
            live(&cache, started, stop, state).await;
            (artifact.sha256, generation)
        });
        Ok(Acquired {
            _waiting: waiting,
            _asking: asking,
            state: receiver,
        })
    }

    /// Decides about a digest whose leases changed: a start nothing waits
    /// for any longer stops, and a running resident without leases is asked
    /// what it holds.
    fn consider(&mut self, digest: &str) {
        let Some(entry) = self.entries.get_mut(digest) else {
            return;
        };
        if entry.leases > 0 {
            return;
        }
        let Some(current) = &mut entry.current else {
            self.entries.remove(digest);
            return;
        };
        let client = match &*current.state.borrow() {
            State::Starting(_) => None,
            State::Ready { client, .. } => Some(client.clone()),
            State::Ended(_) => return,
        };
        let Some(client) = client else {
            tracing::info!(
                event = "resident.state_changed",
                resident = current.id.as_str(),
                from = "starting",
                to = "stopped",
                cause = "not_needed",
                "a resident is no longer needed and its start stops"
            );
            current.stop.cancel();
            self.entries.remove(digest);
            return;
        };
        if current.checking {
            return;
        }
        current.checking = true;
        let checked = Checked {
            digest: digest.to_owned(),
            generation: current.generation,
            changes: entry.changes,
            holds: Ok(false),
        };
        self.checks.spawn(async move {
            let holds = status(&client).await;
            Checked { holds, ..checked }
        });
    }

    fn checked(&mut self, checked: Checked) {
        let Some(entry) = self.entries.get_mut(&checked.digest) else {
            return;
        };
        let Some(current) = entry
            .current
            .as_mut()
            .filter(|current| current.generation == checked.generation)
        else {
            return;
        };
        current.checking = false;
        if entry.leases > 0 {
            return;
        }
        // Leases came and went while the resident answered: what it holds
        // may have changed since.
        if entry.changes != checked.changes {
            self.consider(&checked.digest);
            return;
        }
        match checked.holds {
            Ok(true) => {}
            Ok(false) => {
                tracing::info!(
                    event = "resident.state_changed",
                    resident = current.id.as_str(),
                    from = "ready",
                    to = "stopped",
                    cause = "no_lease_no_conversation",
                    "a resident holds no lease or conversation and stops"
                );
                current.stop.cancel();
                self.entries.remove(&checked.digest);
            }
            // A resident that cannot say what it holds is not one that holds
            // nothing: stopping it would end every conversation it serves.
            Err(error) => {
                tracing::warn!(
                    event = "resident.state_changed",
                    resident = current.id.as_str(),
                    from = "ready",
                    to = "ready",
                    cause = "holds_unknown",
                    %error,
                    "a resident did not say which conversations it holds, so it stays"
                );
            }
        }
    }

    /// A resident ended by itself or after it was stopped.
    fn ended(&mut self, digest: &str, generation: u64) {
        let Some(entry) = self.entries.get_mut(digest) else {
            return;
        };
        if entry
            .current
            .as_ref()
            .is_some_and(|current| current.generation == generation)
        {
            entry.current = None;
            if entry.leases == 0 {
                self.entries.remove(digest);
            }
        }
    }

    fn release(&mut self, conversation: String, reply: oneshot::Sender<Result<(), String>>) {
        // A release never starts a resident (`crates/backend-user-shard/src/lifecycle/conversations.rs`): only running ones hold conversations.
        let running: Vec<_> = self
            .entries
            .iter()
            .filter_map(|(digest, entry)| {
                let current = entry.current.as_ref()?;
                let State::Ready { client, .. } = &*current.state.borrow() else {
                    return None;
                };
                Some((digest.clone(), current.generation, client.clone()))
            })
            .collect();
        self.work.spawn(async move {
            let request = ConversationRequest::Release { conversation };
            let results = futures_util::future::join_all(running.into_iter().map(
                |(digest, generation, client)| {
                    let request = &request;
                    async move {
                        let result = conversation_call(&client, request)
                            .await
                            .and_then(|answer| {
                                if answer == serde_json::json!({}) {
                                    Ok(())
                                } else {
                                    Err("invalid conversation release acknowledgement".into())
                                }
                            });
                        (digest, generation, result)
                    }
                },
            ))
            .await;
            let mut failed = Vec::new();
            let mut errors = Vec::new();
            for (digest, generation, result) in results {
                if let Err(error) = result {
                    failed.push((digest, generation));
                    errors.push(error);
                }
            }
            Work::Released {
                failed,
                errors,
                reply,
            }
        });
    }

    fn worked(&mut self, work: Work) {
        match work {
            Work::Released {
                failed,
                errors,
                reply,
            } => {
                // Failed cleanup retires the faulty resident before the
                // release is answered (`crates/command-protocol/src/conversation.rs`).
                let ending = self.retire_where(|digest, current| {
                    failed.contains(&(digest.to_owned(), current.generation))
                });
                let result = if errors.is_empty() {
                    Ok(())
                } else {
                    Err(errors.join("; "))
                };
                self.work.spawn(async move {
                    for mut state in ending {
                        let _ended = state
                            .wait_for(|state| matches!(state, State::Ended(_)))
                            .await;
                    }
                    Work::Retired { result, reply }
                });
            }
            Work::Retired { result, reply } => {
                // The caller may have gone; the release happened regardless.
                let _gone = reply.send(result);
                // A released conversation may leave a resident holding none,
                // and an answer about what it holds that was given before the
                // release ended may still name the conversation: the release
                // is a change, after which such an answer is asked again.
                for entry in self.entries.values_mut() {
                    entry.changes += 1;
                }
                let digests: Vec<_> = self.entries.keys().cloned().collect();
                for digest in digests {
                    self.consider(&digest);
                }
            }
            Work::Stopped { reply } => {
                let _gone = reply.send(());
            }
        }
    }

    /// Stops the current residents `selected` picks by digest and resident
    /// and returns what to watch for their ends. Their leases stay: the next
    /// caller starts a new resident.
    fn retire_where(
        &mut self,
        selected: impl Fn(&str, &Current) -> bool,
    ) -> Vec<watch::Receiver<State>> {
        let mut ending = Vec::new();
        for (digest, entry) in &mut self.entries {
            let Some(current) = entry
                .current
                .take_if(|current| selected(digest.as_str(), current))
            else {
                continue;
            };
            current.stop.cancel();
            ending.push(current.state.subscribe());
        }
        self.entries
            .retain(|_, entry| entry.leases > 0 || entry.current.is_some());
        ending
    }
}

/// What a resident starts from: its executable and descriptor, where the
/// executable comes from, the invocations its program's artifacts are
/// located for, where its requests to the backend go, and its working
/// directory and environment.
struct Start<'a> {
    artifact: &'a demi_command_protocol::PackageArtifact,
    descriptor: &'a PackageDescriptor,
    invocations: Invocations,
    resolver: Arc<dyn ArtifactResolver>,
    backend: Arc<dyn ResidentBackend>,
    cwd: &'a std::path::Path,
    env: &'a BTreeMap<String, String>,
}

/// One resident's life: install its executable, start it, publish it ready,
/// and publish how it ended. The executable is held while the resident
/// lives, so no newer version of the package removes it.
async fn live(
    cache: &Arc<ArtifactCache>,
    start: Start<'_>,
    stop: CancellationToken,
    state: watch::Sender<State>,
) {
    let descriptor = start.descriptor;
    let began = std::time::Instant::now();
    let _held = cache.holds().hold(&start.artifact.sha256);
    // How long the executable took to be had, which the start's line splits
    // from the process's own start.
    let mut install_ms = 0u64;
    let started = async {
        let wanted = Wanted {
            package: &descriptor.id,
            name: PROGRAM,
            version: &descriptor.version,
            artifact: start.artifact.clone(),
            form: &ArtifactForm::File,
        };
        state.send_replace(State::Starting(Some(Installing::default())));
        let progress = |progress| {
            state.send_modify(|state| {
                if let State::Starting(Some(installing)) = state {
                    installing.progress = Some(progress);
                }
            });
        };
        let executable = cache.install(&wanted, start.resolver, &progress, &stop).await;
        install_ms = began.elapsed().as_millis() as u64;
        state.send_replace(State::Starting(None));
        let executable = executable?;
        let artifacts = Arc::new(ResidentArtifacts {
            cache: cache.clone(),
            package: descriptor.id.clone(),
            invocations: start.invocations,
            held: std::sync::Mutex::default(),
        });
        ResidentProcess::start(
            &executable,
            descriptor,
            start.cwd,
            start.env,
            start.backend.clone(),
            artifacts,
            stop.clone(),
        )
        .await
    }
    .await;
    let resident = match started {
        Ok(resident) => resident,
        Err(error) => {
            if !matches!(error, RuntimeError::Cancelled) {
                tracing::warn!(
                    event = "resident.state_changed",
                    resident = descriptor.id.as_str(),
                    from = "starting",
                    to = "failed",
                    ms = began.elapsed().as_millis() as u64,
                    install_ms,
                    %error,
                    "a resident did not start"
                );
            }
            state.send_replace(State::Ended(Arc::new(error)));
            return;
        }
    };
    tracing::info!(
        event = "resident.state_changed",
        resident = descriptor.id.as_str(),
        from = "starting",
        to = "ready",
        ms = began.elapsed().as_millis() as u64,
        install_ms,
        pid = resident.pid(),
        "a resident started"
    );
    let ready = std::time::Instant::now();
    state.send_replace(State::Ready {
        client: resident.client().clone(),
        info: Arc::new(resident.info().clone()),
    });
    let ended = resident.ended().await;
    let Some(reason) = ended.reason else {
        // A resident asked to stop released its conversations first.
        tracing::info!(
            event = "resident.state_changed",
            resident = descriptor.id.as_str(),
            from = "ready",
            to = "stopped",
            cause = "asked",
            ms = ready.elapsed().as_millis() as u64,
            "a resident stopped"
        );
        state.send_replace(State::Ended(Arc::new(RuntimeError::Stopped)));
        return;
    };
    let exit = crate::ResidentExit {
        resident: descriptor.id.clone(),
        reason,
        stderr: ended.stderr,
    };
    // Its standard error is in the log already, line by line.
    tracing::warn!(
        event = "resident.state_changed",
        resident = exit.resident.as_str(),
        from = "ready",
        to = "exited",
        ms = ready.elapsed().as_millis() as u64,
        reason = %exit.reason,
        "a resident ended by itself"
    );
    state.send_replace(State::Ended(Arc::new(exit.into())));
    // What it kept for its conversations went with it, unsaid
    // (`crates/command-protocol/src/messages.rs`).
    start.backend.lost(descriptor.id.clone()).await;
}

/// Whether the resident holds any conversation (`crates/command-protocol/src/conversation.rs`).
async fn status(client: &Client) -> Result<bool, String> {
    let answer = tokio::time::timeout(
        STATUS_TIMEOUT,
        conversation_call(client, &ConversationRequest::Status {}),
    )
    .await
    .map_err(|_| format!("no answer within {} seconds", STATUS_TIMEOUT.as_secs()))??;
    let status: ConversationStatus =
        serde_json::from_value(answer).map_err(|error| error.to_string())?;
    status.validate().map_err(|error| error.to_string())?;
    Ok(!status.conversations.is_empty())
}

/// One conversation operation and its bounded JSON answer.
async fn conversation_call(
    client: &Client,
    request: &ConversationRequest,
) -> Result<serde_json::Value, String> {
    let exchange = async {
        let (_input, mut output) = client
            .conversation(request)
            .await
            .map_err(|error| error.to_string())?;
        let mut bytes = Vec::new();
        let mut completed = false;
        while let Some(record) = output.next().await.map_err(|error| error.to_string())? {
            match record {
                Record::Stdout(chunk) => {
                    if bytes.len() + chunk.len() > ANSWER_BYTES {
                        return Err("the conversation answer exceeds 1 MiB".into());
                    }
                    bytes.extend_from_slice(&chunk);
                }
                Record::Completion(completion)
                    if completion.exit_code == 0 && completion.error.is_none() =>
                {
                    completed = true
                }
                Record::Stderr(chunk) => tracing::debug!(
                    event = "conversation_call.stderr",
                    call = match request {
                        ConversationRequest::Status {} => "status",
                        ConversationRequest::Release { .. } => "release",
                    },
                    bytes = chunk.len(),
                    "a conversation call wrote to its standard error"
                ),
                _ => return Err("the conversation operation failed".into()),
            }
        }
        if !completed {
            return Err("the conversation operation has no completion".into());
        }
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())
    };
    match request {
        ConversationRequest::Status {} => exchange.await,
        ConversationRequest::Release { .. } => tokio::time::timeout(RELEASE_TIMEOUT, exchange)
            .await
            .map_err(|_| "the conversation release did not finish in time".to_owned())?,
    }
}
