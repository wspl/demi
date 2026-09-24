//! A node's command storage (`command-state-history.md` § Mutation API and
//! concurrency): reads of the current version, and compare-and-set writes
//! that commit a new version in the session's save order. A message from a
//! job whose history generation a rewrite or dispose ended cannot commit.

use demi_shell::{PortError, Revision, StorageOp, StorageReply};
use tokio_util::sync::CancellationToken;

use super::{AgentSession, core::SessionCore, persist};
use crate::store::{CommandStorageKey, CommitGuard, StoreError};

/// What a storage message says while an edit is being prepared.
const RESERVED: &str = "Command storage is reserved for a transcript edit";

/// A command-storage generation: the number a job's record holds, and the
/// lifetime a history rewrite or dispose ends. They are one value, replaced
/// together, so they never disagree.
#[derive(Debug, Clone)]
pub(super) struct Generation {
    pub(super) number: u64,
    pub(super) token: CancellationToken,
}

impl Generation {
    pub(super) fn first() -> Self {
        Self {
            number: 0,
            token: CancellationToken::new(),
        }
    }

    /// Ends this generation and starts the next.
    pub(super) fn next(&self) -> Self {
        self.token.cancel();
        Self {
            number: self.number + 1,
            token: CancellationToken::new(),
        }
    }
}

impl AgentSession {
    /// The command-storage generation current now, which a job started now
    /// is bound to.
    pub(crate) fn command_generation(&self) -> CancellationToken {
        self.shared.read(|core| core.generation.token.clone())
    }

    /// The number of the generation current now, which a job started now
    /// records.
    pub(crate) fn generation_number(&self) -> u64 {
        self.shared.read(|core| core.generation.number)
    }

    /// Serves one storage message of a job that recorded the generation
    /// `number`, while its call `call` lives: refused as no longer current
    /// once that generation ended.
    pub(crate) async fn job_storage(
        &self,
        number: u64,
        op: StorageOp,
        call: CancellationToken,
    ) -> Result<StorageReply, PortError> {
        let current = self.shared.read(|core| core.generation.clone());
        if current.number != number {
            return Err(PortError::Storage(StoreError::Invalidated.to_string()));
        }
        self.storage(op, vec![current.token, call]).await
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
