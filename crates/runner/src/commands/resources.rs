//! Retained native owners scoped to one authenticated backend connection.

use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
    time::Duration,
};

use demi_command_service::{
    Client,
    protocol::{
        ArtifactLocation, Invocation, NativeResourceGrant, NativeResourceScope,
        NativeResourceStatus, PackageArtifact, PackageDescriptor, Record,
    },
};
use futures_util::future::BoxFuture;
use tokio::sync::{mpsc, watch};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use super::{
    cache::{ArtifactResolver, ArtifactSource, RuntimeError},
    native::Services,
};
use crate::connection::wire::{self, Inbound, ResourceResultResult};

type Ready = Result<Arc<Client>, Arc<RuntimeError>>;
type Released = Result<(), Arc<RuntimeError>>;

enum Ownership {
    Acquiring(usize),
    Retained,
}

struct Resource {
    owner: String,
    grant: NativeResourceGrant,
    digest: String,
    ready: watch::Sender<Option<Ready>>,
    released: watch::Sender<Option<Released>>,
    stop: CancellationToken,
    ownership: Mutex<Ownership>,
}

struct Claim(Arc<Resource>);
impl Drop for Claim {
    fn drop(&mut self) {
        let mut owner = self.0.ownership.lock().unwrap();
        if let Ownership::Acquiring(count) = &mut *owner {
            *count -= 1;
            if *count == 0 {
                self.0.stop.cancel();
            }
        }
    }
}

#[derive(Default)]
struct State {
    output: Option<(mpsc::Sender<wire::Outbound>, CancellationToken)>,
    entries: HashMap<String, Arc<Resource>>,
    requests: HashMap<String, CancellationToken>,
}

pub struct Resources {
    services: Arc<Services>,
    state: Arc<Mutex<State>>,
    tasks: TaskTracker,
}

struct Location(ArtifactLocation);
impl ArtifactResolver for Location {
    fn resolve<'a>(
        &'a self,
        _: &'a PackageArtifact,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<ArtifactSource, RuntimeError>> {
        Box::pin(async move {
            if cancel.is_cancelled() {
                return Err(RuntimeError::Cancelled);
            }
            ArtifactSource::from_location(self.0.clone())
        })
    }
}

impl Resources {
    pub fn new(services: Arc<Services>) -> Arc<Self> {
        Arc::new(Self {
            services,
            state: Arc::new(Mutex::new(State::default())),
            tasks: TaskTracker::new(),
        })
    }

    pub fn attach(&self, output: mpsc::Sender<wire::Outbound>, connection: CancellationToken) {
        self.tasks.reopen();
        self.state.lock().unwrap().output = Some((output, connection));
    }

    /// A job receives only live grants from this connection and its exact manifest.
    pub fn bindings(
        &self,
        ids: &[String],
        packages: &BTreeMap<String, PackageDescriptor>,
    ) -> Result<Vec<NativeResourceGrant>, RuntimeError> {
        let state = self.state.lock().unwrap();
        let mut bindings: Vec<NativeResourceGrant> = Vec::new();
        for id in ids {
            let resource = state
                .entries
                .get(id)
                .filter(|entry| !entry.stop.is_cancelled())
                .ok_or_else(|| RuntimeError::Artifact("resource grant is no longer live".into()))?;
            if !matches!(&*resource.ownership.lock().unwrap(), Ownership::Retained)
                || !packages.contains_key(&resource.grant.descriptor_hash)
                || bindings.iter().any(|grant| {
                    grant.descriptor_hash == resource.grant.descriptor_hash
                        && grant.scope.kind == resource.grant.scope.kind
                })
            {
                return Err(RuntimeError::Artifact(
                    "resource does not match this job's package catalog".into(),
                ));
            }
            bindings.push(resource.grant.clone());
        }
        Ok(bindings)
    }

    /// Resource requests never block the connection's command or cancellation reader.
    pub fn handle(self: &Arc<Self>, message: Inbound) -> Result<(), RuntimeError> {
        if let Inbound::ResourceCancel { id } = message {
            if let Some(cancel) = self.state.lock().unwrap().requests.get(&id) {
                cancel.cancel();
            }
            return Ok(());
        }
        let id = match &message {
            Inbound::ResourceAcquire { id, .. }
            | Inbound::ResourceRelease { id, .. }
            | Inbound::ResourceStatus { id, .. } => id.clone(),
            _ => return Err(RuntimeError::Artifact("unexpected resource message".into())),
        };
        let cancel = CancellationToken::new();
        let (output, connection) = {
            let mut state = self.state.lock().unwrap();
            if state.requests.contains_key(&id) || state.requests.len() >= 32 {
                return Err(RuntimeError::Artifact(
                    "resource request limit or duplicate id".into(),
                ));
            }
            let output = state.output.clone().ok_or(RuntimeError::Cancelled)?;
            state.requests.insert(id.clone(), cancel.clone());
            output
        };
        let resources = self.clone();
        self.tasks.spawn(async move {
            let result = match message {
                Inbound::ResourceAcquire {
                    owner,
                    kind,
                    descriptor,
                    location,
                    ..
                } => resources
                    .acquire(owner, kind, descriptor, location, &cancel)
                    .await
                    .map(ResourceResultResult::Variant0),
                Inbound::ResourceRelease { grant_id, .. } => resources
                    .release(&grant_id)
                    .await
                    .map(|()| ResourceResultResult::Variant2(())),
                Inbound::ResourceStatus { grant_id, .. } => resources
                    .status(&grant_id, &cancel)
                    .await
                    .map(ResourceResultResult::Variant1),
                _ => unreachable!(),
            };
            resources.state.lock().unwrap().requests.remove(&id);
            let (value, error) = match result {
                Ok(value) => (value, None),
                Err(error) => (ResourceResultResult::Variant2(()), Some(error.to_string())),
            };
            match wire::resource_result(id, value, error) {
                Ok(message) => {
                    // Socket teardown also releases every grant; no orphan survives a failed reply.
                    if let Err(error) = output.send(message).await {
                        eprintln!("demi-runner: resource reply connection closed: {error}");
                    }
                }
                Err(error) => {
                    eprintln!("demi-runner: resource reply encoding failed: {error}");
                    connection.cancel();
                }
            }
        });
        Ok(())
    }

    async fn acquire(
        self: &Arc<Self>,
        owner: String,
        kind: String,
        descriptor: PackageDescriptor,
        location: ArtifactLocation,
        cancel: &CancellationToken,
    ) -> Result<NativeResourceGrant, RuntimeError> {
        let descriptor_hash = descriptor
            .digest()
            .map_err(|error| RuntimeError::Artifact(error.to_string()))?;
        let digest = descriptor
            .targets
            .get(super::native::target())
            .ok_or(RuntimeError::CatalogMismatch)?
            .sha256
            .clone();
        let (resource, mut ready) = {
            let mut state = self.state.lock().unwrap();
            if state.output.is_none() || cancel.is_cancelled() {
                return Err(RuntimeError::Cancelled);
            }
            if let Some(resource) = state.entries.values().find(|entry| {
                entry.owner == owner
                    && entry.grant.scope.kind == kind
                    && entry.grant.descriptor_hash == descriptor_hash
                    && !entry.stop.is_cancelled()
            }) {
                let mut ownership = resource.ownership.lock().unwrap();
                if let Ownership::Acquiring(count) = &mut *ownership {
                    *count += 1;
                }
                (resource.clone(), resource.ready.subscribe())
            } else {
                if state.entries.len() >= 256 {
                    return Err(RuntimeError::Artifact(
                        "retained resource limit reached".into(),
                    ));
                }
                let id = uuid::Uuid::new_v4().simple().to_string();
                let (ready, receiver) = watch::channel(None);
                let (released, _) = watch::channel(None);
                let resource = Arc::new(Resource {
                    owner,
                    grant: NativeResourceGrant {
                        scope: NativeResourceScope {
                            id: id.clone(),
                            kind,
                        },
                        descriptor_hash,
                    },
                    digest,
                    ready,
                    released,
                    stop: CancellationToken::new(),
                    ownership: Mutex::new(Ownership::Acquiring(1)),
                });
                state.entries.insert(id, resource.clone());
                let resources = self.clone();
                let owned = resource.clone();
                self.tasks.spawn(async move {
                    resources.own(owned, descriptor, location).await;
                });
                (resource, receiver)
            }
        };
        let _claim = Claim(resource.clone());
        loop {
            if let Some(result) = ready.borrow_and_update().clone() {
                result.map_err(|error| RuntimeError::Artifact(error.to_string()))?;
                if cancel.is_cancelled() || resource.stop.is_cancelled() {
                    return Err(RuntimeError::Cancelled);
                }
                *resource.ownership.lock().unwrap() = Ownership::Retained;
                return Ok(resource.grant.clone());
            }
            tokio::select! {
                _ = cancel.cancelled() => return Err(RuntimeError::Cancelled),
                _ = resource.stop.cancelled() => return Err(RuntimeError::Cancelled),
                changed = ready.changed() => changed.map_err(|_| RuntimeError::Cancelled)?,
            }
        }
    }

    async fn own(
        &self,
        resource: Arc<Resource>,
        descriptor: PackageDescriptor,
        location: ArtifactLocation,
    ) {
        let client = self
            .services
            .acquire(&descriptor, Arc::new(Location(location)), &resource.stop)
            .await;
        let result = match client {
            Ok(client) => {
                let started =
                    resource_call(&client, &resource.grant.scope, "acquire", &resource.stop).await;
                resource
                    .ready
                    .send_replace(Some(started.map(|_| client.clone()).map_err(Arc::new)));
                if matches!(&*resource.ready.borrow(), Some(Ok(_))) {
                    if let Some(mut service) = self.services.observe(&resource.digest).await {
                        loop {
                            if matches!(&*service.borrow_and_update(), Some(Err(_))) {
                                break;
                            }
                            tokio::select! {
                                _ = resource.stop.cancelled() => break,
                                changed = service.changed() => if changed.is_err() { break; },
                            }
                        }
                    }
                }
                let cleanup = resource_call(
                    &client,
                    &resource.grant.scope,
                    "release",
                    &CancellationToken::new(),
                )
                .await
                .map(|_| ());
                if cleanup.is_err() {
                    self.services.retire(&resource.digest).await;
                }
                cleanup.map_err(Arc::new)
            }
            Err(error) => {
                resource.ready.send_replace(Some(Err(error.clone())));
                Err(error)
            }
        };
        let stopped = resource.stop.is_cancelled();
        resource.stop.cancel();
        let output = {
            let mut state = self.state.lock().unwrap();
            state.entries.remove(&resource.grant.scope.id);
            state.output.clone()
        };
        if !stopped && let Some((output, connection)) = output {
            let reason = result
                .as_ref()
                .err()
                .map(ToString::to_string)
                .unwrap_or_else(|| "native resource lost".into());
            match wire::resource_lost(resource.grant.scope.id.clone(), reason) {
                Ok(message) => {
                    // The backend's connection teardown independently invalidates the same grant.
                    if let Err(error) = output.send(message).await {
                        eprintln!("demi-runner: resource loss delivery failed: {error}");
                    }
                }
                Err(error) => {
                    eprintln!("demi-runner: resource loss encoding failed: {error}");
                    connection.cancel();
                }
            }
        }
        resource.released.send_replace(Some(result));
    }

    async fn release(&self, id: &str) -> Result<(), RuntimeError> {
        let resource = self.state.lock().unwrap().entries.get(id).cloned();
        let Some(resource) = resource else {
            return Ok(());
        };
        let mut released = resource.released.subscribe();
        resource.stop.cancel();
        loop {
            if let Some(result) = released.borrow_and_update().clone() {
                return result.map_err(|error| RuntimeError::Artifact(error.to_string()));
            }
            released
                .changed()
                .await
                .map_err(|_| RuntimeError::Cancelled)?;
        }
    }

    async fn status(
        &self,
        id: &str,
        cancel: &CancellationToken,
    ) -> Result<NativeResourceStatus, RuntimeError> {
        let resource = self.state.lock().unwrap().entries.get(id).cloned();
        let Some(resource) = resource else {
            return Ok(NativeResourceStatus {
                state: "released".into(),
            });
        };
        let client = resource
            .ready
            .borrow()
            .clone()
            .and_then(Result::ok)
            .ok_or(RuntimeError::Cancelled)?;
        serde_json::from_slice(
            &resource_call(&client, &resource.grant.scope, "status", cancel).await?,
        )
        .map_err(|error| RuntimeError::Artifact(error.to_string()))
    }

    pub fn digests(&self) -> std::collections::HashSet<String> {
        self.state
            .lock()
            .unwrap()
            .entries
            .values()
            .map(|entry| entry.digest.clone())
            .collect()
    }

    pub async fn detach(&self) {
        {
            let mut state = self.state.lock().unwrap();
            state.output = None;
            for cancel in state.requests.values() {
                cancel.cancel();
            }
            for resource in state.entries.values() {
                resource.stop.cancel();
            }
        }
        self.tasks.close();
        self.tasks.wait().await;
    }
}

/// Exchange one bounded resource lifecycle request on its resident service.
async fn resource_call(
    client: &Client,
    scope: &NativeResourceScope,
    operation: &str,
    cancel: &CancellationToken,
) -> Result<Vec<u8>, RuntimeError> {
    let request = Invocation {
        caller: None,
        operation: operation.into(),
        invocation_id: uuid::Uuid::new_v4().simple().to_string(),
        resource: Some(scope.clone()),
        json: Some(true),
        args: serde_json::json!({}),
        cwd: "/".into(),
        env: BTreeMap::new(),
        edits: None,
    };
    let exchange = async {
        let (_input, mut output) = client.resource(&request).await?;
        // Lifecycle handlers never read stdin and can finish before this await
        // returns. Keep the request alive without writing to a completed stream.
        let mut bytes = Vec::new();
        let mut completion = None;
        while let Some(record) = output.next().await? {
            match record {
                Record::Stdout(chunk) => {
                    if bytes.len() + chunk.len() > 64 * 1024 {
                        return Err(RuntimeError::Artifact(
                            "resource result exceeds 64 KiB".into(),
                        ));
                    }
                    bytes.extend_from_slice(&chunk);
                }
                Record::Completion(value) => completion = Some(value),
                Record::Stderr(_) => {
                    return Err(RuntimeError::Artifact("unexpected resource stderr".into()));
                }
                Record::InputPull => {
                    return Err(RuntimeError::Artifact(
                        "resource lifecycle cannot read stdin".into(),
                    ));
                }
            }
        }
        let completion = completion
            .ok_or_else(|| RuntimeError::Artifact("resource completion missing".into()))?;
        if completion.exit_code != 0 {
            return Err(RuntimeError::Artifact(
                completion
                    .error
                    .map(|error| error.message)
                    .unwrap_or_else(|| "resource request failed".into()),
            ));
        }
        Ok::<_, RuntimeError>(bytes)
    };
    tokio::select! {
        _ = cancel.cancelled() => Err(RuntimeError::Cancelled),
        result = tokio::time::timeout(Duration::from_secs(10), exchange) => result.map_err(|_| RuntimeError::Deadline("resource lifecycle"))?,
    }
}
