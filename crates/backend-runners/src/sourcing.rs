//! Artifacts sourced on demand (`native-runtime.md` § Publish packages, then
//! source artifacts on demand): the first time something needs an artifact
//! of the server release, a runner asking where a package's executable
//! downloads from or an installer asking for a runner executable, it is
//! taken from the store when it is there, and otherwise from the release's
//! files, a directory on this machine or an HTTPS location, checked against
//! the size and SHA-256 the release's records give, and stored. Needs of one
//! artifact wait for each other, so they share one fetch; a fetch that
//! failed or was abandoned leaves nothing stored, and the next need tries
//! again.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use demi_command_protocol::PackageArtifact;
use demi_runner_protocol::release::FilesLocation;
use demi_shared_artifacts::Digest;
use object_store::ObjectStore;
use tokio_util::sync::CancellationToken;

use crate::publication::{PublicationError, blob, in_place, put_immutable};

/// An artifact of the release: the file of the release's files that holds
/// it, what it stands for, and whether the file is its compressed copy,
/// which is stored as it is, in the content coding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseArtifact {
    pub file: String,
    pub artifact: PackageArtifact,
    pub encoded: bool,
}

/// How much larger than what it decodes to a compressed copy may be: zstd
/// adds a little to bytes it cannot compress.
const ENCODED_MARGIN: u64 = 1 << 20;

/// The store and the release's files it takes artifacts from.
pub struct Sourcing {
    store: Arc<dyn ObjectStore>,
    files: FilesLocation,
    client: demi_shared_artifacts::Client,
    /// One turn per artifact, by SHA-256. A digest keeps its entry: the
    /// release names a few dozen artifacts.
    turns: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl Sourcing {
    pub fn new(store: Arc<dyn ObjectStore>, files: FilesLocation) -> Result<Self, PublicationError> {
        let client = demi_shared_artifacts::client_following_redirects().map_err(|error| {
            PublicationError::Files(format!("no client for the release's files: {error}"))
        })?;
        Ok(Self {
            store,
            files,
            client,
            turns: Mutex::default(),
        })
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
        match &self.files {
            FilesLocation::Directory(directory) => {
                let path = directory.join(&wanted.file);
                tokio::select! {
                    () = cancel.cancelled() => Err(abandoned()),
                    read = tokio::fs::read(&path) => read.map(Bytes::from).map_err(|error| error.to_string()),
                }
            }
            FilesLocation::Url(base) => {
                let url = base.join(&wanted.file).map_err(|error| error.to_string())?;
                let limit = if wanted.encoded {
                    wanted.artifact.size.saturating_add(ENCODED_MARGIN)
                } else {
                    wanted.artifact.size
                };
                tracing::info!(file = %wanted.file, "downloading a file of the server release");
                let mut bytes = Vec::new();
                demi_shared_artifacts::download_measured(
                    &self.client,
                    url.as_str(),
                    limit,
                    &mut bytes,
                    cancel,
                )
                .await
                .map_err(|error| error.to_string())?;
                Ok(Bytes::from(bytes))
            }
        }
    }
}

fn abandoned() -> String {
    "the work that needed the artifact no longer runs".to_owned()
}
