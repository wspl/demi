//! Resident native services shared by one runner registration and security context.

use crate::commands::{
    cache::{ArtifactCache, ArtifactResolver, RuntimeError},
    services::ResidentService,
};
use crate::host_log;
use demi_command_service::Client;
use demi_command_service::protocol::{
    ConversationRequest, ConversationStatus, PackageDescriptor, Record,
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::sync::{Mutex, watch};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

pub(super) type Ready = Result<Arc<Client>, Arc<RuntimeError>>;
struct Slot {
    ready: watch::Sender<Option<Ready>>,
    stop: CancellationToken,
    descriptor: PackageDescriptor,
}

pub struct Services {
    cache: Arc<ArtifactCache>,
    target: String,
    cwd: PathBuf,
    env: BTreeMap<String, String>,
    slots: Arc<Mutex<HashMap<String, Arc<Slot>>>>,
    tasks: TaskTracker,
    stop: CancellationToken,
}

impl Services {
    /// Failed conversation cleanup retires its entire service before returning.
    pub(super) async fn retire(&self, digest: &str) {
        let receiver = {
            let slots = self.slots.lock().await;
            slots.get(digest).map(|slot| {
                slot.stop.cancel();
                slot.ready.subscribe()
            })
        };
        if let Some(mut receiver) = receiver {
            loop {
                if matches!(&*receiver.borrow_and_update(), Some(Err(_))) {
                    break;
                }
                if receiver.changed().await.is_err() {
                    break;
                }
            }
        }
    }
    pub async fn new(
        directory: PathBuf,
        target: String,
        cwd: PathBuf,
        env: BTreeMap<String, String>,
    ) -> Result<Arc<Self>, RuntimeError> {
        if !demi_command_service::protocol::TARGETS.contains(&target.as_str()) {
            return Err(RuntimeError::Artifact(format!(
                "unsupported native target: {target}"
            )));
        }
        Ok(Arc::new(Self {
            cache: ArtifactCache::new(directory).await?,
            target,
            cwd,
            env,
            slots: Arc::new(Mutex::new(HashMap::new())),
            tasks: TaskTracker::new(),
            stop: CancellationToken::new(),
        }))
    }

    pub async fn acquire(
        self: &Arc<Self>,
        descriptor: &PackageDescriptor,
        resolver: Arc<dyn ArtifactResolver>,
        caller: &CancellationToken,
    ) -> Ready {
        let artifact = descriptor
            .targets
            .get(&self.target)
            .ok_or_else(|| Arc::new(RuntimeError::CatalogMismatch))?
            .clone();
        let mut receiver = {
            let mut slots = self.slots.lock().await;
            if self.stop.is_cancelled() {
                return Err(Arc::new(RuntimeError::Cancelled));
            }
            if let Some(slot) = slots.get(&artifact.sha256) {
                let mut expected = descriptor.operations.clone();
                let mut available = slot.descriptor.operations.clone();
                expected.sort();
                available.sort();
                if descriptor.protocol_version != slot.descriptor.protocol_version
                    || expected != available
                {
                    return Err(Arc::new(RuntimeError::CatalogMismatch));
                }
                slot.ready.subscribe()
            } else {
                let (ready, receiver) = watch::channel(None);
                let slot = Arc::new(Slot {
                    ready,
                    stop: self.stop.child_token(),
                    descriptor: descriptor.clone(),
                });
                slots.insert(artifact.sha256.clone(), slot.clone());
                let cache = self.cache.clone();
                let service_slots = self.slots.clone();
                let cwd = self.cwd.clone();
                let env = self.env.clone();
                let descriptor = descriptor.clone();
                self.tasks.spawn(async move {
                    let digest = artifact.sha256.clone();
                    let started = async {
                        let path = cache.acquire(artifact, resolver, &slot.stop).await?;
                        ResidentService::start(&path, &descriptor, &cwd, &env)
                            .await
                            .map_err(Arc::new)
                    }
                    .await;
                    let ran = started.is_ok();
                    let outcome = match started {
                        Ok(mut service) => {
                            host_log::event(format_args!(
                                "service {} started (pid {})",
                                descriptor.id,
                                service.pid()
                            ));
                            slot.ready
                                .send_replace(Some(Ok(Arc::new(service.client().clone()))));
                            let stopped = tokio::select! {
                                _ = slot.stop.cancelled() => None,
                                result = service.wait_closed() => Some(result),
                            };
                            match stopped {
                                Some(Ok(())) => Err(Arc::new(RuntimeError::Artifact(
                                    "native service exited".into(),
                                ))),
                                Some(Err(error)) => Err(Arc::new(error)),
                                None => service.shutdown().await.map_err(Arc::new),
                            }
                        }
                        Err(error) => Err(error),
                    };
                    let mut slots = service_slots.lock().await;
                    if slots
                        .get(&digest)
                        .is_some_and(|current| Arc::ptr_eq(current, &slot))
                    {
                        slots.remove(&digest);
                    }
                    drop(slots);
                    let error = outcome
                        .err()
                        .unwrap_or_else(|| Arc::new(RuntimeError::Cancelled));
                    if matches!(&*error, RuntimeError::Cancelled) {
                        if ran {
                            host_log::event(format_args!("service {} stopped", descriptor.id));
                        }
                    } else {
                        host_log::runner(format_args!(
                            "service {} retired: {error}",
                            descriptor.id
                        ));
                    }
                    slot.ready.send_replace(Some(Err(error)));
                });
                receiver
            }
        };
        loop {
            if let Some(result) = receiver.borrow_and_update().clone() {
                return result;
            }
            tokio::select! {
                _ = caller.cancelled() => return Err(Arc::new(RuntimeError::Cancelled)),
                changed = receiver.changed() => {
                    if changed.is_err() { return Err(Arc::new(RuntimeError::Cancelled)); }
                }
            }
        }
    }

    /// Ownerless services remain resident only while they hold conversation state.
    pub async fn retain(&self, digests: &HashSet<String>) {
        let candidates: Vec<_> = self
            .slots
            .lock()
            .await
            .iter()
            .filter(|(digest, _)| !digests.contains(*digest))
            .map(|(digest, slot)| (digest.clone(), slot.ready.borrow().clone()))
            .collect();
        for (digest, ready) in candidates {
            let retained = match ready {
                Some(Ok(client)) => {
                    let status = conversation(&client, "status", None)
                        .await
                        .and_then(|value| {
                            let status = serde_json::from_value::<ConversationStatus>(value)
                                .map_err(|error| error.to_string())?;
                            status.validate().map_err(|error| error.to_string())?;
                            Ok(status)
                        });
                    match status {
                        Ok(status) => !status.conversations.is_empty(),
                        Err(error) => {
                            host_log::runner(format_args!("native status failed: {error}"));
                            false
                        }
                    }
                }
                _ => false,
            };
            if !retained {
                self.retire(&digest).await;
            }
        }
    }

    /// Release joins every resident service without starting or resolving an artifact.
    pub async fn release_conversation(&self, id: &str) -> Result<(), String> {
        let residents: Vec<_> = self
            .slots
            .lock()
            .await
            .iter()
            .map(|(digest, slot)| (digest.clone(), slot.ready.subscribe()))
            .collect();
        let results = futures_util::future::join_all(residents.into_iter().map(
            |(digest, mut ready)| async move {
                let client = loop {
                    if let Some(result) = ready.borrow_and_update().clone() {
                        break result.map_err(|error| error.to_string());
                    }
                    if ready.changed().await.is_err() {
                        break Err("native service closed".into());
                    }
                };
                let result = match client {
                    Ok(client) => {
                        conversation(&client, "release", Some(id))
                            .await
                            .and_then(|value| {
                                if value == serde_json::json!({}) {
                                    Ok(())
                                } else {
                                    Err("invalid conversation release acknowledgement".into())
                                }
                            })
                    }
                    Err(error) => Err(error),
                };
                if result.is_err() {
                    self.retire(&digest).await;
                }
                result
            },
        ))
        .await;
        let errors: Vec<_> = results.into_iter().filter_map(Result::err).collect();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }

    /// Connection loss ends all retained state while allowing the runner to reconnect.
    pub async fn disconnect(&self) {
        let digests: Vec<_> = self.slots.lock().await.keys().cloned().collect();
        futures_util::future::join_all(digests.iter().map(|digest| self.retire(digest))).await;
    }

    pub async fn close(&self) {
        self.stop.cancel();
        self.tasks.close();
        tokio::join!(self.cache.shutdown(), self.tasks.wait());
        self.slots.lock().await.clear();
    }
}

impl Drop for Services {
    fn drop(&mut self) {
        self.stop.cancel();
        self.cache.cancel();
        self.tasks.close();
    }
}

pub use demi_command_service::protocol::host_target as target;

/// Validate the bounded JSON response to a native conversation lifecycle operation.
async fn conversation(
    client: &Client,
    operation: &str,
    id: Option<&str>,
) -> Result<serde_json::Value, String> {
    let exchange = async {
        let (_input, mut output) = client
            .conversation(&ConversationRequest {
                operation: operation.into(),
                conversation: id.map(str::to_owned),
            })
            .await
            .map_err(|error| error.to_string())?;
        let mut bytes = Vec::new();
        let mut completed = false;
        while let Some(record) = output.next().await.map_err(|error| error.to_string())? {
            match record {
                Record::Stdout(chunk) => {
                    if bytes.len() + chunk.len() > 1024 * 1024 {
                        return Err("conversation response exceeds 1 MiB".into());
                    }
                    bytes.extend_from_slice(&chunk);
                }
                Record::Completion(completion)
                    if completion.exit_code == 0 && completion.error.is_none() =>
                {
                    completed = true
                }
                Record::Stderr(chunk) => host_log::runner(format_args!(
                    "conversation {operation}: {}",
                    String::from_utf8_lossy(&chunk).trim_end()
                )),
                _ => return Err("native conversation operation failed".into()),
            }
        }
        if !completed {
            return Err("native conversation operation has no completion".into());
        }
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())
    };
    tokio::time::timeout(
        Duration::from_secs(if operation == "status" { 5 } else { 360 }),
        exchange,
    )
    .await
    .map_err(|_| "native conversation operation timed out".to_owned())?
}
