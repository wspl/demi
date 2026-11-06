//! The command packages of the process's cores: those of the release
//! `DEMI_RELEASE` names, whose executables the process serves its runners
//! from the release's `files/` on their channels, at
//! [`ARTIFACT_ROUTE`]`/<sha256>`, as a server serves its local store's.
//! Without a release, the cores have none.
//!
//! The release may lie in several roots, a list of paths as `PATH` writes
//! one: their `commands/` together name the packages, and each one's
//! `files/` holds the executables of some targets, as an SDK's platform
//! packages each hold their own (`packages/sdk/src/binaries.ts`). An
//! executable no root holds is not offered to a runner.
//!
//! A runner checks each download against the size and SHA-256 its catalog
//! names, so the process serves the files as they are.

use std::io::SeekFrom;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use axum::Extension;
use axum::body::Body;
use axum::extract::Path;
use axum::http::header::{ACCEPT_RANGES, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum_extra::TypedHeader;
use axum_extra::headers::Range;
use demi_command_protocol::{ArtifactLocation, ArtifactUrl, PackageArtifact, PackageDescriptor};
use demi_core_hosts::packages::{ARTIFACT_ROUTE, Packages, ReleaseError, ReleasePackages, ReleaseArtifact};
use demi_runner_client::{ArtifactResolver, CommandCatalog};
use demi_runner_protocol::release::{COMMANDS, FILES};
use futures_util::future::LocalBoxFuture;
use tokio::io::AsyncSeekExt as _;
use tokio_util::io::ReaderStream;
use tokio_util::sync::CancellationToken;

use crate::error::ApiError;
use crate::runners::range_start;
use demi_web_api_protocol::error::ErrorCode;

/// The release's packages, and its files.
#[derive(Default)]
pub(super) struct ReleaseFiles {
    packages: ReleasePackages,
    /// Each root's `files/` that exists.
    files: Vec<PathBuf>,
}

impl ReleaseFiles {
    /// The packages of the release in `roots`, none without a root. A
    /// package two roots name is refused.
    pub(super) async fn read(roots: Vec<PathBuf>) -> Result<Self, ReleaseError> {
        let mut release = Self::default();
        for root in roots {
            let commands = root.join(COMMANDS);
            if tokio::fs::try_exists(&commands).await.unwrap_or(false) {
                let read = ReleasePackages::read(&commands).await?;
                for descriptor in read.descriptors {
                    if release.packages.descriptors.iter().any(|known| known.id == descriptor.id) {
                        return Err(ReleaseError::Commands {
                            directory: commands,
                            reason: format!("the package {} is in another root too", descriptor.id),
                        });
                    }
                    release.packages.descriptors.push(descriptor);
                }
                release.packages.executables.extend(read.executables);
            }
            let files = root.join(FILES);
            if tokio::fs::try_exists(&files).await.unwrap_or(false) {
                release.files.push(files);
            }
        }
        Ok(release)
    }

    /// The file of the executable `sha256`, in the first root that holds
    /// it, with what it stands for.
    fn file(&self, sha256: &str) -> Option<(PathBuf, &ReleaseArtifact)> {
        let wanted = self.packages.executables.get(sha256)?;
        self.files
            .iter()
            .map(|files| files.join(&wanted.file))
            .find(|path| path.is_file())
            .map(|path| (path, wanted))
    }
}

/// The process's cores' packages.
pub(super) struct ServedPackages(pub(super) Arc<ReleaseFiles>);

impl Packages for ServedPackages {
    fn package(&self, id: &str) -> Option<&PackageDescriptor> {
        self.0.packages.descriptors.iter().find(|descriptor| descriptor.id == id)
    }

    fn resolver(&self) -> Rc<dyn ArtifactResolver> {
        Rc::new(Resolver(self.0.clone()))
    }

    fn catalog(&self) -> CommandCatalog {
        CommandCatalog::new(self.0.packages.descriptors.clone(), self.resolver())
            .expect("the release's packages were checked when it was read")
    }
}

/// Where a runner downloads an executable of the release: the process's
/// path of it, which its channel reaches.
struct Resolver(Arc<ReleaseFiles>);

impl ArtifactResolver for Resolver {
    fn resolve(
        &self,
        artifact: &PackageArtifact,
        _target: &str,
        _cancel: CancellationToken,
    ) -> LocalBoxFuture<'static, Result<ArtifactLocation, String>> {
        let held = self
            .0
            .file(&artifact.sha256)
            .is_some_and(|(_, wanted)| wanted.artifact.size == artifact.size);
        let location = match held {
            true => Ok(ArtifactLocation::Url(ArtifactUrl {
                url: format!("{ARTIFACT_ROUTE}/{}", artifact.sha256),
                expires_at: None,
            })),
            false => Err("the release holds no file of the artifact for this target".to_owned()),
        };
        Box::pin(async move { location })
    }
}

/// `GET /native-artifacts/{sha256}`: the release's executable of that
/// digest, its compressed copy in the content coding, from a byte on for a
/// Range request, as a download that broke off resumes
/// (`crates/runner-command-packages/src/cache.rs`).
pub(super) async fn artifact(
    Extension(release): Extension<Arc<ReleaseFiles>>,
    Path(sha256): Path<String>,
    range: Option<TypedHeader<Range>>,
) -> Result<Response, ApiError> {
    let not_found = || ApiError::new(StatusCode::NOT_FOUND, ErrorCode::NotFound, "No such native artifact");
    let Some((path, wanted)) = release.file(&sha256) else {
        return Err(not_found());
    };
    let mut file = tokio::fs::File::open(&path)
        .await
        .map_err(|error| ApiError::internal_message(format!("{}: {error}", wanted.file)))?;
    let size = file.metadata().await.map_err(|error| ApiError::internal(&error))?.len();
    let from = range.map_or(0, |TypedHeader(range)| range_start(&range));
    if from > 0 && from >= size {
        return Ok((StatusCode::RANGE_NOT_SATISFIABLE, [(CONTENT_RANGE, format!("bytes */{size}"))]).into_response());
    }
    file.seek(SeekFrom::Start(from)).await.map_err(|error| ApiError::internal(&error))?;
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));
    headers.insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    headers.insert(CONTENT_LENGTH, HeaderValue::from(size - from));
    if wanted.encoded {
        headers.insert(CONTENT_ENCODING, HeaderValue::from_static(demi_shared_artifacts::CONTENT_CODING));
    }
    let status = if from > 0 {
        let whole = format!("bytes {from}-{}/{size}", size - 1);
        headers.insert(CONTENT_RANGE, HeaderValue::from_str(&whole).map_err(|error| ApiError::internal(&error))?);
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    };
    Ok((status, headers, Body::from_stream(ReaderStream::new(file))).into_response())
}
