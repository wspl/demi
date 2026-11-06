//! The download of the next runner release ahead, by the runners of the
//! user's connected devices.

use std::rc::Rc;
use std::sync::Arc;

use demi_backend_runners::prefetch::NextRelease;
use demi_core::Shard;
use demi_runner_protocol::release::PrefetchState;

/// The download of the next runner release, on the user's core.
#[expect(
    async_fn_in_trait,
    reason = "a core runs on one thread: its futures are never sent to another"
)]
pub trait RunnerPrefetch {
    /// Has the runner of each of the user's paired devices that is
    /// connected download `next` ahead while it serves (`crates/runner/src/update.rs`): each is counted at once, and `next` hears how its
    /// download went, a connection that ends failing it.
    async fn prefetch(self: &Rc<Self>, next: Arc<NextRelease>);
}

impl RunnerPrefetch for Shard {
    async fn prefetch(self: &Rc<Self>, next: Arc<NextRelease>) {
        let devices = match self.store().paired_devices().await {
            Ok(devices) => devices,
            Err(error) => {
                // Their runners download after the switch.
                tracing::warn!(event = "runner_release.devices_list_failed", error = &error as &dyn std::error::Error, "the devices to download the next runner cannot be listed");
                return;
            }
        };
        for device in devices {
            let Some(link) = self.devices().link(&device.id) else {
                continue;
            };
            next.begin(&device.id, &device.name);
            let target = self.devices().target(&device.id);
            let artifact = target
                .as_deref()
                .and_then(|target| next.release.targets.get(target))
                .cloned();
            let programs = next.programs(&self.services().public_url);
            let next = next.clone();
            self.tasks().spawn_local(async move {
                let state = match artifact {
                    None => PrefetchState::Failed {
                        reason: format!(
                            "the release has no runner for {}",
                            target.as_deref().unwrap_or("the device's system")
                        ),
                    },
                    Some(artifact) => match link
                        .prefetch(&next.release.release, &artifact.sha256, artifact.size, programs)
                        .await
                    {
                        Ok(()) => PrefetchState::Done,
                        Err(error) => PrefetchState::Failed {
                            reason: error.to_string(),
                        },
                    },
                };
                next.end(&device.id, &device.name, state);
            });
        }
    }
}
