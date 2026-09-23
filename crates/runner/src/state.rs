//! The installation's state (`runner.md` § Connection and identity): its
//! configuration, device token and lock, and the record of the runner that
//! holds it. Each file is checked as it is read.

use demi_runner_protocol::values::{BackendUrl, DeviceToken};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io,
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RunnerConfig {
    pub backend_url: BackendUrl,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<DeviceId>,
}

/// The id the backend gave the device; never empty.
#[derive(Clone, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DeviceId(String);

impl TryFrom<String> for DeviceId {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, &'static str> {
        if value.is_empty() {
            return Err("empty device ID in runner config");
        }
        Ok(Self(value))
    }
}

impl From<DeviceId> for String {
    fn from(id: DeviceId) -> Self {
        id.0
    }
}

/// The runner that holds the installation, for `status` and `drain`: its
/// local endpoint, the secret its management requests carry, and its release.
#[derive(Clone, Serialize, Deserialize)]
#[serde(try_from = "Active")]
pub struct ActiveRunner {
    pub endpoint: String,
    pub secret: String,
    pub release: String,
}

/// An active record as the file holds it, before its check.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Active {
    endpoint: String,
    secret: String,
    release: String,
}

impl TryFrom<Active> for ActiveRunner {
    type Error = &'static str;

    fn try_from(active: Active) -> Result<Self, &'static str> {
        let secret = active.secret.len() == 32
            && active
                .secret
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if active.endpoint.is_empty() || active.release.is_empty() || !secret {
            return Err("invalid active runner record");
        }
        Ok(Self {
            endpoint: active.endpoint,
            secret: active.secret,
            release: active.release,
        })
    }
}

pub struct RunnerState {
    pub root: PathBuf,
}

pub struct StateLease {
    file: std::fs::File,
    active: Option<PathBuf>,
}

impl RunnerState {
    pub async fn open(root: PathBuf) -> io::Result<Self> {
        tokio::fs::create_dir_all(&root).await?;
        crate::fs::chmod(&root, 0o700).await?;
        Ok(Self { root })
    }

    pub fn lock(&self) -> io::Result<StateLease> {
        self.try_lock()?
            .ok_or_else(|| io::Error::other("runner already active for this installation"))
    }

    pub fn try_lock(&self) -> io::Result<Option<StateLease>> {
        let mut options = std::fs::OpenOptions::new();
        // Windows file locking requires read or write access, not append alone.
        options.create(true).read(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(self.root.join("runner.lock"))?;
        match file.try_lock() {
            Ok(()) => Ok(Some(StateLease { file, active: None })),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(error)) => Err(error),
        }
    }

    pub async fn config(&self) -> io::Result<Option<RunnerConfig>> {
        let Some(bytes) = read_optional(&self.root.join("runner.json")).await? else {
            return Ok(None);
        };
        let config = serde_json::from_slice(&bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        Ok(Some(config))
    }

    pub async fn write_config(&self, config: &RunnerConfig) -> io::Result<()> {
        write_private(
            self.root.join("runner.json"),
            serde_json::to_vec_pretty(config).map_err(io::Error::other)?,
        )
        .await
    }

    /// The device token the file keeps, one line.
    pub async fn token(&self) -> io::Result<Option<DeviceToken>> {
        let Some(bytes) = read_optional(&self.root.join("runner-token")).await? else {
            return Ok(None);
        };
        let token = String::from_utf8(bytes)
            .map_err(io::Error::other)?
            .trim()
            .to_owned();
        let token = DeviceToken::try_from(token)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        Ok(Some(token))
    }

    pub async fn write_token(&self, token: &DeviceToken) -> io::Result<()> {
        write_private(
            self.root.join("runner-token"),
            format!("{}\n", token.expose()).into_bytes(),
        )
        .await
    }

    pub async fn active(&self) -> io::Result<ActiveRunner> {
        let bytes = tokio::fs::read(self.root.join("active.json")).await?;
        serde_json::from_slice(&bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }
}

impl StateLease {
    pub async fn publish(&mut self, state: &RunnerState, active: &ActiveRunner) -> io::Result<()> {
        let path = state.root.join("active.json");
        write_private(
            path.clone(),
            serde_json::to_vec(active).map_err(io::Error::other)?,
        )
        .await?;
        self.active = Some(path);
        Ok(())
    }

    pub fn release(mut self) -> io::Result<()> {
        if let Some(path) = self.active.take() {
            std::fs::remove_file(path)?;
        }
        self.file.unlock()
    }
}

impl Drop for StateLease {
    fn drop(&mut self) {
        if let Some(path) = self.active.take()
            && let Err(error) = std::fs::remove_file(path)
            && error.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!("failed to remove active state: {error}");
        }
        // Closing the file releases the operating-system lock on every exit path.
    }
}

/// The name of the installation that belongs to `backend`.
pub fn instance_id(backend: &BackendUrl) -> String {
    format!("{:x}", Sha256::digest(backend.as_str().as_bytes()))
}

async fn read_optional(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match tokio::fs::read(path).await {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Replaces the state file at `path` with `bytes` and a final newline,
/// readable by the owner alone and on disk before the rename.
pub(crate) async fn write_private(path: PathBuf, mut bytes: Vec<u8>) -> io::Result<()> {
    if !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    let publication = demi_artifact::Publication {
        mode: demi_artifact::Mode::Replace,
        permissions: demi_artifact::Permissions::Private,
        durable: true,
    };
    demi_artifact::publish_bytes(&path, &bytes, publication)
        .await
        .map_err(io_error)
}

/// An artifact failure as the IO error it is, or wraps.
pub(crate) fn io_error(error: demi_artifact::Error) -> io::Error {
    match error {
        demi_artifact::Error::Io(error) => error,
        error => io::Error::other(error),
    }
}
