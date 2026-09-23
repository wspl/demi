//! Recovery of what a stopped manager left behind (`managed-hosts.md` §
//! Startup and recovery): staging directories are removed, every recorded
//! sandbox is fenced, and every working pair is published. It runs at
//! startup, on `reconcile`, and in the stop-post recovery.

use std::io;

use demi_machines_protocol::{DeviceId, IdError};

use crate::{
    blocking,
    manager::Core,
    sandbox::{Sandbox, SandboxError, files::SandboxRecord},
    storage::{
        durable::{create_private, remove_tree},
        working::{SaveError, WorkingPair},
    },
};

#[derive(Debug, thiserror::Error)]
pub enum RecoveryError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("a working pair is not named by a device id: {0}")]
    Name(#[from] IdError),
    #[error("{0} is not a valid runtime record: {1}")]
    Record(String, serde_json::Error),
    #[error("Existing Cloud slot exceeds configured pool")]
    Slot,
    #[error(transparent)]
    Sandbox(#[from] SandboxError),
    #[error(transparent)]
    Save(#[from] SaveError),
}

/// Fences each recorded sandbox and saves each working pair. Every device
/// operation has finished and none can start: the caller holds the whole
/// admission gate, or no manager serves.
pub async fn fence_and_save(core: &Core) -> Result<(), RecoveryError> {
    let working = core.config.working();
    let runsc = core.runsc.root().to_owned();
    let entries = blocking::run({
        let working = working.clone();
        move |off| -> Result<Vec<String>, RecoveryError> {
            create_private(off, &working)?;
            create_private(off, &runsc)?;
            let mut names = Vec::new();
            for entry in fs_err::read_dir(&working)? {
                let name = entry?.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    // A stage has replaced neither a working nor a committed pair.
                    remove_tree(off, &working.join(&name))?;
                } else {
                    names.push(name);
                }
            }
            names.sort();
            Ok(names)
        }
    })
    .await?;
    for name in entries {
        let device = DeviceId::parse(name)?;
        let pair = WorkingPair::new(&working, &device);
        let record_path = pair.sandbox_record();
        let record = blocking::run(move |_| -> Result<Option<SandboxRecord>, RecoveryError> {
            let bytes = match fs_err::read(&record_path) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error.into()),
            };
            serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|error| RecoveryError::Record(record_path.display().to_string(), error))
        })
        .await?;
        if let Some(record) = record {
            if !core.slots.contains(record.slot) {
                return Err(RecoveryError::Slot);
            }
            Sandbox::recorded(core, record).close(core, &pair).await?;
        }
        pair.save(&core.tools, &core.store, &device).await?;
    }
    Ok(())
}
