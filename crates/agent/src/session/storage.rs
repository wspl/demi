//! A node's command storage (`command-state-history.md` § Mutation API and
//! concurrency): reads of the current version, and compare-and-set writes
//! that commit a new version in the session's save order. A message from a
//! job whose history generation a rewrite or dispose ended cannot commit.

use demi_shell::{PortError, Revision, StorageOp, StorageReply};
use tokio_util::sync::CancellationToken;

use super::{AgentSession, core::SessionCore, persist};
use crate::store::{CommandStorageKey, CommitGuard};

/// What a storage message says while an edit is being prepared.
const RESERVED: &str = "Command storage is reserved for a transcript edit";

impl AgentSession {
    /// The command-storage generation current now, which a job started now
    /// is bound to.
    pub(crate) fn command_generation(&self) -> CancellationToken {
        self.shared.read(|core| core.generation.clone())
    }

    /// Serves one storage message of a job bound to `lifetimes`: its
    /// generation and its call's cancellation. A write returns once its
    /// version is committed.
    pub(crate) async fn storage(
        &self,
        op: StorageOp,
        lifetimes: Vec<CancellationToken>,
    ) -> Result<StorageReply, PortError> {
        let guard = CommitGuard::new(lifetimes);
        let admit = |core: &SessionCore| {
            guard
                .check()
                .map_err(|error| PortError::Storage(error.to_string()))?;
            if core.preparing_edit() {
                return Err(PortError::Storage(RESERVED.to_owned()));
            }
            Ok(())
        };
        match op {
            StorageOp::Read { key } => {
                let key = storage_key(key)?;
                self.shared.read(|core| {
                    admit(core)?;
                    let (value, revision) = core.storage_value(&key);
                    Ok(StorageReply::Value {
                        value,
                        revision: Revision(revision),
                    })
                })
            }
            StorageOp::List { prefix } => {
                if !prefix.is_empty() {
                    storage_key(prefix.clone())?;
                }
                self.shared.read(|core| {
                    admit(core)?;
                    Ok(StorageReply::Keys {
                        keys: core.storage_keys(&prefix),
                    })
                })
            }
            StorageOp::WriteIf {
                key,
                value,
                expected,
            } => {
                let key = storage_key(key)?;
                // The write takes its place in the save order, and nothing
                // changes the version between its check and its commit.
                let _turn = self.shared.persist_gate.acquire().await;
                let written = self.shared.read(|core| {
                    admit(core)?;
                    let current = core.commands.revision();
                    if expected.is_some_and(|expected| expected != Revision(current)) {
                        return Ok(Err(StorageReply::Conflict {
                            revision: Revision(current),
                        }));
                    }
                    Ok(Ok((core.storage_write(key, value), current)))
                })?;
                let (version, current) = match written {
                    Ok(written) => written,
                    Err(conflict) => return Ok(conflict),
                };
                // Equal values under canonical JSON make no version.
                let Some(version) = version else {
                    return Ok(StorageReply::Committed {
                        revision: Revision(current),
                    });
                };
                persist::write_if_dirty(&self.shared, Some(&version), &guard)
                    .await
                    .map_err(|error| PortError::Storage(error.to_string()))?;
                let revision = version.revision;
                self.shared
                    .update(|core| core.accept_storage_version(version));
                Ok(StorageReply::Committed {
                    revision: Revision(revision),
                })
            }
        }
    }
}

fn storage_key(key: String) -> Result<CommandStorageKey, PortError> {
    CommandStorageKey::try_from(key).map_err(|error| PortError::Storage(error.to_string()))
}
