//! Downloading ahead (`runner.md` § Runner updates): the runner release an
//! upgrade announced as next, over the management socket
//! (`upgrades.md` § Prepare), which every connected paired device's runner
//! downloads while the current release serves, and how far each device has
//! come, which the upgrade waits on. The backend serves that release's
//! executables from the next release's files, and keeps nothing of it past
//! its own run.

use std::sync::Arc;

use demi_runner_protocol::release::{DevicePrefetch, PrefetchProgress, PrefetchState, RunnerRelease};
use demi_web_api_protocol::ids::DeviceId;
use tokio::sync::watch;

use crate::sourcing::Sourcing;

/// The runner release announced as next: its record, where its executables
/// are sourced from, and each device's download of it.
pub struct NextRelease {
    pub release: RunnerRelease,
    pub(crate) sourcing: Sourcing,
    devices: watch::Sender<Vec<(DeviceId, DevicePrefetch)>>,
}

impl NextRelease {
    pub(crate) fn new(release: RunnerRelease, sourcing: Sourcing) -> Self {
        Self {
            release,
            sourcing,
            devices: watch::Sender::new(Vec::new()),
        }
    }

    /// `device`, which its user named `name`, is downloading the release.
    pub fn begin(&self, device: &DeviceId, name: &str) {
        self.set(device, name, PrefetchState::Downloading);
    }

    /// `device`'s download ended as `state` says.
    pub fn end(&self, device: &DeviceId, name: &str, state: PrefetchState) {
        self.set(device, name, state);
    }

    fn set(&self, device: &DeviceId, name: &str, state: PrefetchState) {
        let prefetch = DevicePrefetch {
            name: name.to_owned(),
            state,
        };
        self.devices.send_modify(|devices| {
            match devices.iter_mut().find(|(id, _)| id == device) {
                Some((_, current)) => *current = prefetch,
                None => devices.push((device.clone(), prefetch)),
            }
        });
    }

    /// Each device's download, from now on.
    pub fn progress(&self) -> watch::Receiver<Vec<(DeviceId, DevicePrefetch)>> {
        self.devices.subscribe()
    }
}

/// The devices' downloads as the management socket answers them.
pub fn progress(devices: &[(DeviceId, DevicePrefetch)]) -> PrefetchProgress {
    PrefetchProgress {
        devices: devices.iter().map(|(_, prefetch)| prefetch.clone()).collect(),
    }
}

/// The next runner release, once an upgrade announced one; the connected
/// paired devices follow it.
#[derive(Default)]
pub struct Prefetches {
    next: watch::Sender<Option<Arc<NextRelease>>>,
}

impl Prefetches {
    /// Announces `next`, which replaces any release announced before.
    pub fn announce(&self, next: Arc<NextRelease>) {
        self.next.send_replace(Some(next));
    }

    /// The release announced as next, if any.
    pub fn next(&self) -> Option<Arc<NextRelease>> {
        self.next.borrow().clone()
    }

    /// The release announced as next, now and from now on.
    pub fn subscribe(&self) -> watch::Receiver<Option<Arc<NextRelease>>> {
        self.next.subscribe()
    }
}
