//! The command package installs in progress on this runner
//! (`native-runtime.md` § Installation progress): each artifact download
//! adds one, updates it as another hundredth of the artifact arrives and as
//! a resource's archive is unpacked, and removes it when it ends, however
//! it ends. The connection to the backend reports the list on each change.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use demi_runner_protocol::wire::{Install, InstallPhase, MAX_INSTALLS};
use tokio::sync::watch;

/// The installs in progress, which every clone shares.
#[derive(Clone)]
pub struct Installs {
    list: Arc<watch::Sender<Vec<(u64, Install)>>>,
    next: Arc<AtomicU64>,
}

impl Default for Installs {
    fn default() -> Self {
        Self {
            list: Arc::new(watch::channel(Vec::new()).0),
            next: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl Installs {
    /// The installs in progress now, and each later list.
    pub fn subscribe(&self) -> InstallsReceiver {
        InstallsReceiver(self.list.subscribe())
    }

    /// Adds the download of `package`'s artifact `name` at `version`,
    /// whose size is `total`; the returned install leaves the list when
    /// dropped.
    pub(crate) fn start(&self, package: &str, name: &str, version: &str, total: u64) -> Installing {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let install = Install {
            package: package.to_owned(),
            name: name.to_owned(),
            version: version.to_owned(),
            phase: InstallPhase::Download,
            done: 0,
            total,
        };
        self.list.send_modify(|list| list.push((id, install)));
        Installing {
            installs: self.clone(),
            id,
        }
    }
}

/// A receiver of the installs in progress.
#[derive(Clone)]
pub struct InstallsReceiver(watch::Receiver<Vec<(u64, Install)>>);

impl InstallsReceiver {
    /// The list now, which [`Self::changed`] then waits past. A list longer
    /// than a message carries keeps its oldest installs.
    pub fn current(&mut self) -> Vec<Install> {
        self.0
            .borrow_and_update()
            .iter()
            .take(MAX_INSTALLS)
            .map(|(_, install)| install.clone())
            .collect()
    }

    /// Waits for the list to change; false once no install can change it.
    pub async fn changed(&mut self) -> bool {
        self.0.changed().await.is_ok()
    }
}

/// One artifact being installed, in the list until dropped.
pub(crate) struct Installing {
    installs: Installs,
    id: u64,
}

impl Installing {
    /// `done` bytes have arrived: the list changes when they pass another
    /// hundredth of the size, or reach it.
    pub(crate) fn downloaded(&self, done: u64) {
        self.installs.list.send_if_modified(|list| {
            let Some((_, install)) = list.iter_mut().find(|(id, _)| *id == self.id) else {
                return false;
            };
            let hundredth = |bytes: u64| bytes.saturating_mul(100) / install.total;
            let done = done.min(install.total);
            let passed = hundredth(done) > hundredth(install.done) || done == install.total;
            if done == install.done || !passed {
                return false;
            }
            install.done = done;
            true
        });
    }

    /// The archive arrived whole, and is unpacked now.
    pub(crate) fn unpacking(&self) {
        self.installs.list.send_modify(|list| {
            if let Some((_, install)) = list.iter_mut().find(|(id, _)| *id == self.id) {
                install.phase = InstallPhase::Unpack;
                install.done = install.total;
            }
        });
    }
}

impl Drop for Installing {
    fn drop(&mut self) {
        self.installs
            .list
            .send_modify(|list| list.retain(|(id, _)| *id != self.id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A download changes the list once per hundredth of its size, and at
    /// its end; unpacking and the end of the install change it too.
    #[test]
    fn an_install_changes_the_list_at_each_hundredth_and_leaves_it_when_done() {
        let installs = Installs::default();
        let mut reported = installs.subscribe();
        let installing = installs.start("demi.browser", "program", "0.1.3", 1000);
        assert!(reported.0.has_changed().unwrap());
        assert_eq!(reported.current()[0].done, 0);
        installing.downloaded(9);
        assert!(
            !reported.0.has_changed().unwrap(),
            "within the first hundredth"
        );
        installing.downloaded(10);
        assert_eq!(reported.current()[0].done, 10);
        installing.downloaded(15);
        assert!(!reported.0.has_changed().unwrap());
        installing.downloaded(1000);
        assert_eq!(reported.current()[0].done, 1000);
        installing.unpacking();
        assert_eq!(reported.current()[0].phase, InstallPhase::Unpack);
        drop(installing);
        assert_eq!(reported.current(), []);
    }
}
