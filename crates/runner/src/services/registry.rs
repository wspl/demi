//! The service registry (`native-runtime.md` § Keep a service resident). One
//! owner task keeps a registration's resident services by artifact digest,
//! counts the leases on each, and decides when a service ends. A lease holder
//! sends one message when it takes its lease and one when it drops it, and
//! never waits for the decision.
//!
//! For example, a job whose manifest names `demi.builtin` holds a lease on
//! its artifact while the job runs, and the installed manifest holds another.
//! When the connection installs a manifest with a newer `demi.builtin`, the
//! old artifact's last lease ends when its last job does. The registry then
//! asks the old service which conversations it holds: it stops a service that
//! holds none, and keeps one that holds some or cannot say.

use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use demi_command_service::{
    Client, ServiceError,
    protocol::{ConversationRequest, ConversationStatus, PackageDescriptor, Record, ServiceInfo},
};
use tokio::{
    sync::{mpsc, oneshot, watch},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

use super::{
    ArtifactResolver, RuntimeError, cache::ArtifactCache, process::ResidentService, target,
};

/// How long a service has to say which conversations it holds.
const STATUS_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a service has to release a conversation: long enough for a
/// browser to close its tabs and remove its profile.
const RELEASE_TIMEOUT: Duration = Duration::from_secs(360);
/// The most a conversation operation's answer may hold.
const ANSWER_BYTES: usize = 1024 * 1024;

/// The registry's owner task and the handle to it.
pub struct ServiceRegistry {
    handle: ServiceHandle,
    owner: tokio::task::JoinHandle<()>,
}

impl ServiceRegistry {
    /// Services start in `cwd` with exactly `env`; their executables are
    /// cached in `cache`.
    pub async fn new(
        cache: PathBuf,
        cwd: PathBuf,
        env: BTreeMap<String, String>,
    ) -> Result<Self, RuntimeError> {
        let cache = Arc::new(ArtifactCache::new(cache).await?);
        // Unbounded, but bounded by construction: each lease sends two
        // messages, and each other request comes from a caller awaiting it.
        let (requests, receiver) = mpsc::unbounded_channel();
        let owner = Owner {
            cache,
            cwd,
            env,
            requests: requests.downgrade(),
            entries: HashMap::new(),
            generation: 0,
            lifecycles: JoinSet::new(),
            checks: JoinSet::new(),
            work: JoinSet::new(),
            stop: CancellationToken::new(),
        };
        Ok(Self {
            handle: ServiceHandle { requests },
            owner: tokio::spawn(owner.run(receiver)),
        })
    }

    pub fn handle(&self) -> ServiceHandle {
        self.handle.clone()
    }

    /// Stops every service and waits until each has ended. The owner also
    /// ends by itself once every handle and lease is gone.
    pub async fn close(self) {
        // An owner that has ended already has nothing left to stop.
        let _closed = self.handle.requests.send(Request::Close);
        self.owner.await.expect("the service registry does not panic");
    }
}

enum Request {
    Lease(String),
    Unlease(String),
    Acquire {
        descriptor: PackageDescriptor,
        resolver: Arc<dyn ArtifactResolver>,
        reply: oneshot::Sender<Result<Acquired, RuntimeError>>,
    },
    Release {
        conversation: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    StopAll {
        reply: oneshot::Sender<()>,
    },
    Close,
}

/// A service's life as its callers see it.
#[derive(Clone)]
enum State {
    Starting,
    Ready {
        client: Client,
        info: Arc<ServiceInfo>,
    },
    Ended(Arc<RuntimeError>),
}

struct Acquired {
    /// Held while the caller waits for the start.
    _waiting: ServiceLease,
    state: watch::Receiver<State>,
}

/// Sends requests to the registry. Cloning shares the one registry.
#[derive(Clone)]
pub struct ServiceHandle {
    requests: mpsc::UnboundedSender<Request>,
}

impl ServiceHandle {
    /// A lease that keeps the service of `digest` resident until it drops,
    /// whether or not that service runs yet.
    pub fn lease(&self, digest: String) -> ServiceLease {
        // A closed registry keeps nothing; the lease then means nothing.
        let _closed = self.requests.send(Request::Lease(digest.clone()));
        ServiceLease {
            requests: self.requests.clone(),
            digest,
        }
    }

    /// The service of `descriptor`'s artifact for this host, started when
    /// none runs. Callers asking at once share one start.
    pub async fn acquire(
        &self,
        descriptor: &PackageDescriptor,
        resolver: Arc<dyn ArtifactResolver>,
        cancel: &CancellationToken,
    ) -> Result<Resident, Arc<RuntimeError>> {
        let (reply, answer) = oneshot::channel();
        self.requests
            .send(Request::Acquire {
                descriptor: descriptor.clone(),
                resolver,
                reply,
            })
            .map_err(|_| Arc::new(RuntimeError::Cancelled))?;
        let acquired = tokio::select! {
            _ = cancel.cancelled() => return Err(Arc::new(RuntimeError::Cancelled)),
            answer = answer => answer.map_err(|_| Arc::new(RuntimeError::Cancelled))?.map_err(Arc::new)?,
        };
        let mut state = acquired.state;
        loop {
            let current = state.borrow_and_update().clone();
            match current {
                State::Starting => {}
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
                changed = state.changed() => {
                    if changed.is_err() {
                        return Err(Arc::new(RuntimeError::Cancelled));
                    }
                }
            }
        }
    }

    /// Releases `conversation` in every running service, all at once, and
    /// retires each service whose release fails.
    pub async fn release_conversation(&self, conversation: &str) -> Result<(), String> {
        let (reply, answer) = oneshot::channel();
        self.requests
            .send(Request::Release {
                conversation: conversation.to_owned(),
                reply,
            })
            .map_err(|_| "the service registry is closed".to_owned())?;
        answer
            .await
            .map_err(|_| "the service registry is closed".to_owned())?
    }

    /// Stops every service, as losing the backend connection does
    /// (`runner.md` § Command lifetime), and waits until each has ended.
    pub async fn stop_all(&self) {
        let (reply, answer) = oneshot::channel();
        if self.requests.send(Request::StopAll { reply }).is_ok() {
            // A closed registry has stopped everything already.
            let _closed = answer.await;
        }
    }
}

/// One claim on a digest's service; dropping it ends the claim.
pub struct ServiceLease {
    requests: mpsc::UnboundedSender<Request>,
    digest: String,
}

impl Drop for ServiceLease {
    fn drop(&mut self) {
        // A closed registry has no count to lower.
        let _closed = self
            .requests
            .send(Request::Unlease(std::mem::take(&mut self.digest)));
    }
}

/// A running service a caller invokes.
pub struct Resident {
    client: Client,
    state: watch::Receiver<State>,
}

impl Resident {
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// What a call that failed with `error` reports. When the call failed
    /// because its service went away, that is how the service ended, with
    /// its exit status and the end of its standard error.
    pub async fn failure(&mut self, error: ServiceError) -> String {
        let connection_lost = match &error {
            ServiceError::Http2(error) => error.is_io() || error.is_go_away(),
            ServiceError::Io(_) => true,
            _ => false,
        };
        if connection_lost {
            // The service's owner publishes its end once it has reaped it.
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
    cwd: PathBuf,
    env: BTreeMap<String, String>,
    /// For the leases the owner hands out itself; weak, so the owner ends
    /// when nothing else can reach it.
    requests: mpsc::WeakUnboundedSender<Request>,
    entries: HashMap<String, Entry>,
    /// Numbers the services started, so a late report about one that has
    /// ended is not taken for its successor.
    generation: u64,
    lifecycles: JoinSet<(String, u64)>,
    checks: JoinSet<Checked>,
    work: JoinSet<Work>,
    /// Stops every service; each one's own token is a child of it.
    stop: CancellationToken,
}

/// One artifact digest: the leases on it and its current service.
#[derive(Default)]
struct Entry {
    leases: usize,
    /// Counts lease changes, so a status answer from before the last change
    /// is asked again.
    changes: u64,
    current: Option<Current>,
}

struct Current {
    generation: u64,
    id: String,
    stop: CancellationToken,
    state: watch::Sender<State>,
    checking: bool,
}

struct Checked {
    digest: String,
    generation: u64,
    changes: u64,
    holds: Result<bool, String>,
}

/// What a release task hands back to the owner.
enum Work {
    /// Services whose release failed, to retire before `reply` goes out.
    Released {
        failed: Vec<(String, u64)>,
        errors: Vec<String>,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Retirements that have ended; idle services are reconsidered.
    Retired {
        result: Result<(), String>,
        reply: oneshot::Sender<Result<(), String>>,
    },
    StoppedAll {
        reply: oneshot::Sender<()>,
    },
}

impl Owner {
    async fn run(mut self, mut requests: mpsc::UnboundedReceiver<Request>) {
        loop {
            tokio::select! {
                request = requests.recv() => match request {
                    Some(Request::Close) | None => break,
                    Some(request) => self.request(request),
                },
                Some(ended) = self.lifecycles.join_next() => {
                    let (digest, generation) = ended.expect("service lifecycles do not panic");
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
        self.checks.shutdown().await;
        self.work.shutdown().await;
        while self.lifecycles.join_next().await.is_some() {}
    }

    fn request(&mut self, request: Request) {
        match request {
            Request::Lease(digest) => {
                let entry = self.entries.entry(digest).or_default();
                entry.leases += 1;
                entry.changes += 1;
            }
            Request::Unlease(digest) => {
                if let Some(entry) = self.entries.get_mut(&digest) {
                    entry.leases -= 1;
                    entry.changes += 1;
                }
                self.consider(&digest);
            }
            Request::Acquire {
                descriptor,
                resolver,
                reply,
            } => {
                let acquired = self.acquire(descriptor, resolver);
                // A caller that gave up drops its lease with the answer.
                let _gave_up = reply.send(acquired);
            }
            Request::Release {
                conversation,
                reply,
            } => self.release(conversation, reply),
            Request::StopAll { reply } => {
                let ending = self.retire_where(|_| true);
                self.work.spawn(async move {
                    for mut state in ending {
                        let _ended = state.wait_for(|state| matches!(state, State::Ended(_))).await;
                    }
                    Work::StoppedAll { reply }
                });
            }
            Request::Close => unreachable!("the owner loop ends on close"),
        }
    }

    fn acquire(
        &mut self,
        descriptor: PackageDescriptor,
        resolver: Arc<dyn ArtifactResolver>,
    ) -> Result<Acquired, RuntimeError> {
        if self.stop.is_cancelled() {
            return Err(RuntimeError::Cancelled);
        }
        let artifact = descriptor
            .targets
            .get(target())
            .ok_or(RuntimeError::CatalogMismatch)?
            .clone();
        let digest = artifact.sha256.clone();
        // The waiting caller's lease counts at once, so a start never begins
        // with nothing holding it.
        let waiting = ServiceLease {
            requests: self.requests.upgrade().ok_or(RuntimeError::Cancelled)?,
            digest: digest.clone(),
        };
        let entry = self.entries.entry(digest).or_default();
        entry.leases += 1;
        entry.changes += 1;
        // A service that has just ended, before its owner reported it, is
        // one to start again.
        if let Some(current) = &entry.current
            && !matches!(&*current.state.borrow(), State::Ended(_))
        {
            return Ok(Acquired {
                _waiting: waiting,
                state: current.state.subscribe(),
            });
        }
        self.generation += 1;
        let generation = self.generation;
        let (state, receiver) = watch::channel(State::Starting);
        let stop = self.stop.child_token();
        entry.current = Some(Current {
            generation,
            id: descriptor.id.clone(),
            stop: stop.clone(),
            state: state.clone(),
            checking: false,
        });
        let cache = self.cache.clone();
        let cwd = self.cwd.clone();
        let env = self.env.clone();
        self.lifecycles.spawn(async move {
            live(&cache, &artifact, &descriptor, resolver.as_ref(), &cwd, &env, stop, state).await;
            (artifact.sha256, generation)
        });
        Ok(Acquired {
            _waiting: waiting,
            state: receiver,
        })
    }

    /// Decides about a digest whose leases changed: a start nothing waits
    /// for any longer stops, and a running service without leases is asked
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
            State::Starting => None,
            State::Ready { client, .. } => Some(client.clone()),
            State::Ended(_) => return,
        };
        let Some(client) = client else {
            tracing::info!(
                "service {} is no longer needed and its start stops",
                current.id
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
        // Leases came and went while the service answered: what it holds
        // may have changed since.
        if entry.changes != checked.changes {
            self.consider(&checked.digest);
            return;
        }
        match checked.holds {
            Ok(true) => {}
            Ok(false) => {
                tracing::info!(
                    "service {} holds no lease or conversation and stops",
                    current.id
                );
                current.stop.cancel();
                self.entries.remove(&checked.digest);
            }
            // A service that cannot say what it holds is not one that holds
            // nothing: stopping it would end every conversation it serves.
            Err(error) => tracing::warn!(
                "service {} did not say which conversations it holds ({error}); it stays",
                current.id
            ),
        }
    }

    /// A service ended by itself or after it was stopped.
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
        // A release never starts a service (`resource-lifecycle.md`
        // § Conversation release): only running ones hold conversations.
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
                // Failed cleanup retires the faulty service before the
                // release is answered (`native-runtime.md`
                // § Conversation-scoped state).
                let ending = self.retire_where(|(digest, generation)| {
                    failed.contains(&(digest.to_owned(), generation))
                });
                let result = if errors.is_empty() {
                    Ok(())
                } else {
                    Err(errors.join("; "))
                };
                self.work.spawn(async move {
                    for mut state in ending {
                        let _ended = state.wait_for(|state| matches!(state, State::Ended(_))).await;
                    }
                    Work::Retired { result, reply }
                });
            }
            Work::Retired { result, reply } => {
                // The caller may have gone; the release happened regardless.
                let _gone = reply.send(result);
                // A released conversation may leave a service holding none.
                let digests: Vec<_> = self.entries.keys().cloned().collect();
                for digest in digests {
                    self.consider(&digest);
                }
            }
            Work::StoppedAll { reply } => {
                let _gone = reply.send(());
            }
        }
    }

    /// Stops the current services `selected` picks by digest and generation
    /// and returns what to watch for their ends. Their leases stay: the next
    /// caller starts a new service.
    fn retire_where(
        &mut self,
        selected: impl Fn((&str, u64)) -> bool,
    ) -> Vec<watch::Receiver<State>> {
        let mut ending = Vec::new();
        for (digest, entry) in &mut self.entries {
            let Some(current) = entry
                .current
                .take_if(|current| selected((digest.as_str(), current.generation)))
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

/// One service's life: install its executable, start it, publish it ready,
/// and publish how it ended.
async fn live(
    cache: &ArtifactCache,
    artifact: &demi_command_service::protocol::PackageArtifact,
    descriptor: &PackageDescriptor,
    resolver: &dyn ArtifactResolver,
    cwd: &std::path::Path,
    env: &BTreeMap<String, String>,
    stop: CancellationToken,
    state: watch::Sender<State>,
) {
    let started = async {
        let executable = cache.install(artifact, resolver, &stop).await?;
        ResidentService::start(&executable, descriptor, cwd, env, stop.clone()).await
    }
    .await;
    let service = match started {
        Ok(service) => service,
        Err(error) => {
            if !matches!(error, RuntimeError::Cancelled) {
                tracing::warn!("service {} did not start: {error}", descriptor.id);
            }
            state.send_replace(State::Ended(Arc::new(error)));
            return;
        }
    };
    tracing::info!(
        "service {} started (pid {})",
        descriptor.id,
        service.pid()
    );
    state.send_replace(State::Ready {
        client: service.client().clone(),
        info: Arc::new(service.info().clone()),
    });
    let ended = service.ended().await;
    let error = match ended.reason {
        None => {
            tracing::info!("service {} stopped", descriptor.id);
            RuntimeError::Stopped
        }
        Some(reason) => {
            let exit = super::ServiceExit {
                service: descriptor.id.clone(),
                reason,
                stderr: ended.stderr,
            };
            // Its standard error is in the log already, line by line.
            tracing::warn!(
                "service {} {}",
                exit.service, exit.reason
            );
            exit.into()
        }
    };
    state.send_replace(State::Ended(Arc::new(error)));
}

/// Whether the service holds any conversation (`native-runtime.md`
/// § Conversation-scoped state).
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
                Record::Stderr(chunk) => tracing::warn!(
                    "conversation {request:?}: {}",
                    String::from_utf8_lossy(&chunk).trim_end()
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
