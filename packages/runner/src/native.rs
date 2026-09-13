//! Resident native services shared by one runner registration and security context.

use demi_command_protocol::PackageDescriptor;
use demi_command_runtime::{ArtifactCache, ArtifactResolver, ResidentService, RuntimeError};
use demi_command_service::Client;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};
use tokio::sync::{Mutex, watch};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

type Ready = Result<Arc<Client>, Arc<RuntimeError>>;
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
    pub async fn new(
        directory: PathBuf,
        target: String,
        cwd: PathBuf,
        env: BTreeMap<String, String>,
    ) -> Result<Arc<Self>, RuntimeError> {
        if !demi_command_protocol::TARGETS.contains(&target.as_str()) {
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
                    let outcome = match started {
                        Ok(mut service) => {
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
                    if !matches!(&*error, RuntimeError::Cancelled) {
                        eprintln!("demi-runner: native service retired: {error}");
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

    /// Call after manifest installation or job release; active snapshots retain their artifacts.
    pub async fn retain(&self, digests: &HashSet<String>) {
        self.slots.lock().await.retain(|digest, slot| {
            if digests.contains(digest) {
                return true;
            }
            slot.stop.cancel();
            false
        });
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

pub fn target() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return "aarch64-apple-darwin";
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return "x86_64-apple-darwin";
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    return "aarch64-unknown-linux-musl";
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return "x86_64-unknown-linux-musl";
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    return "aarch64-pc-windows-msvc";
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    return "x86_64-pc-windows-msvc";
    #[cfg(not(any(
        all(
            target_os = "macos",
            any(target_arch = "aarch64", target_arch = "x86_64")
        ),
        all(
            target_os = "linux",
            any(target_arch = "aarch64", target_arch = "x86_64")
        ),
        all(
            target_os = "windows",
            any(target_arch = "aarch64", target_arch = "x86_64")
        )
    )))]
    compile_error!("unsupported runner target");
}
