//! Artifacts sourced on demand (`crates/backend-runners/src/publication.rs`):
//! the first time something needs an artifact of the server release, it is made
//! available. Three things need one: a runner asking where a package's
//! executable downloads from, an installer asking for a runner executable, and
//! a paired device downloading the next release's runner ahead of an update.
//!
//! The artifact is taken from the store when it is there. Otherwise it is copied
//! from the release's files, the root's `files/`, which the release brought
//! with it, checked against the size and SHA-256 the release's records give,
//! and stored. Nothing is downloaded: the copy takes milliseconds, so nothing is
//! stored ahead of a need.
//!
//! Needs of one artifact wait for each other, so they share one fetch. A fetch
//! that failed or was abandoned leaves nothing stored, and the next need tries
//! again.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use demi_command_protocol::PackageArtifact;
use demi_shared_artifacts::Digest;
use object_store::ObjectStore;
use tokio_util::sync::CancellationToken;

use crate::publication::{blob, in_place, put_immutable};

/// An artifact of the release: the file of the release's files that holds
/// it, what it stands for, and whether the file is its compressed copy,
/// which is stored as it is, in the content coding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseArtifact {
    pub file: String,
    pub artifact: PackageArtifact,
    pub encoded: bool,
}

/// The store and the release's files it takes artifacts from.
pub struct Sourcing {
    store: Arc<dyn ObjectStore>,
    /// The release's files, its root's `files/`.
    files: PathBuf,
    /// One turn per artifact, by SHA-256. A digest keeps its entry: the
    /// release names a few dozen artifacts.
    turns: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl Sourcing {
    pub fn new(store: Arc<dyn ObjectStore>, files: PathBuf) -> Self {
        Self {
            store,
            files,
            turns: Mutex::default(),
        }
    }

    /// The store, which holds every artifact once it was sourced.
    pub fn store(&self) -> &Arc<dyn ObjectStore> {
        &self.store
    }

    /// Makes sure the store holds `wanted`, taking it from the release's
    /// files when it does not. `cancel` abandons the need.
    pub async fn ensure(
        &self,
        wanted: &ReleaseArtifact,
        cancel: &CancellationToken,
    ) -> Result<(), String> {
        let key = blob(&wanted.artifact.sha256).map_err(|error| error.to_string())?;
        let turn = self.turn(&wanted.artifact.sha256);
        let _turn = tokio::select! {
            () = cancel.cancelled() => return Err(abandoned()),
            turn = turn.lock() => turn,
        };
        let placed = in_place(&*self.store, &key, &wanted.artifact, cancel).await;
        if placed.map_err(|error| error.to_string())? {
            return Ok(());
        }
        let bytes = self
            .fetch(wanted, cancel)
            .await
            .map_err(|reason| format!("{}: {reason}", wanted.file))?;
        let checked = bytes.clone();
        let expected = Digest {
            size: wanted.artifact.size,
            sha256: wanted.artifact.sha256.clone(),
        };
        let encoded = wanted.encoded;
        tokio::task::spawn_blocking(move || {
            if encoded {
                return demi_shared_artifacts::check_encoded_blocking(&checked, &expected);
            }
            let mut verifier = demi_shared_artifacts::Verifier::new(&expected);
            verifier.update(&checked)?;
            verifier.finish()
        })
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| format!("{}: {error}", wanted.file))?;
        let coding = encoded.then_some(demi_shared_artifacts::CONTENT_CODING);
        put_immutable(&*self.store, &key, bytes, &wanted.artifact, coding, cancel)
            .await
            .map_err(|error| error.to_string())
    }

    /// The artifact's turn, which needs of it take one at a time.
    fn turn(&self, sha256: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.turns
            .lock()
            .expect("the turns are intact")
            .entry(sha256.to_owned())
            .or_default()
            .clone()
    }

    /// The bytes of `wanted`'s file, unchecked.
    async fn fetch(&self, wanted: &ReleaseArtifact, cancel: &CancellationToken) -> Result<Bytes, String> {
        let path = self.files.join(&wanted.file);
        tokio::select! {
            () = cancel.cancelled() => Err(abandoned()),
            read = tokio::fs::read(&path) => read.map(Bytes::from).map_err(|error| error.to_string()),
        }
    }
}

fn abandoned() -> String {
    "the work that needed the artifact no longer runs".to_owned()
}
