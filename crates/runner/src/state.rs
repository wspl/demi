use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io,
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RunnerConfig {
    pub backend_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveRunner {
    pub endpoint: String,
    pub secret: String,
    pub release: String,
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
        let config: RunnerConfig = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        backend_url(&config.backend_url)?;
        if config.device_id.as_ref().is_some_and(String::is_empty) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "empty device ID in runner config",
            ));
        }
        Ok(Some(config))
    }

    pub async fn write_config(&self, config: &RunnerConfig) -> io::Result<()> {
        backend_url(&config.backend_url)?;
        write_private(
            self.root.join("runner.json"),
            serde_json::to_vec_pretty(config).map_err(io::Error::other)?,
        )
        .await
    }

    pub async fn token(&self) -> io::Result<Option<String>> {
        let Some(bytes) = read_optional(&self.root.join("runner-token")).await? else {
            return Ok(None);
        };
        let token = String::from_utf8(bytes)
            .map_err(io::Error::other)?
            .trim()
            .to_owned();
        validate_token(&token)?;
        Ok(Some(token))
    }

    pub async fn write_token(&self, token: &str) -> io::Result<()> {
        validate_token(token)?;
        write_private(
            self.root.join("runner-token"),
            format!("{token}\n").into_bytes(),
        )
        .await
    }

    pub async fn active(&self) -> io::Result<ActiveRunner> {
        let bytes = tokio::fs::read(self.root.join("active.json")).await?;
        let active: ActiveRunner = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        validate_active(&active)?;
        Ok(active)
    }
}

impl StateLease {
    pub async fn publish(&mut self, state: &RunnerState, active: &ActiveRunner) -> io::Result<()> {
        validate_active(active)?;
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
            crate::host_log::runner(format_args!("failed to remove active state: {error}"));
        }
        // Closing the file releases the operating-system lock on every exit path.
    }
}

pub fn backend_url(value: &str) -> io::Result<reqwest::Url> {
    let url = reqwest::Url::parse(value).map_err(io::Error::other)?;
    if !matches!(url.scheme(), "http" | "https" | "ws" | "wss")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid backend URL",
        ));
    }
    Ok(url)
}

pub fn instance_id(url: &str) -> io::Result<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(backend_url(url)?.as_str().as_bytes())
    ))
}

async fn read_optional(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match tokio::fs::read(path).await {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

pub(crate) async fn write_private(path: PathBuf, mut bytes: Vec<u8>) -> io::Result<()> {
    if !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    tokio::task::spawn_blocking(move || {
        use std::io::Write;
        let directory = path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "state file has no directory")
        })?;
        let mut file = tempfile::NamedTempFile::new_in(directory)?;
        file.write_all(&bytes)?;
        file.as_file().sync_all()?;
        file.persist(path).map_err(|error| error.error)?;
        Ok::<_, io::Error>(())
    })
    .await
    .map_err(io::Error::other)?
}

fn validate_token(token: &str) -> io::Result<()> {
    if token.is_empty() || token.chars().any(char::is_whitespace) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid runner device token",
        ));
    }
    Ok(())
}

fn validate_active(active: &ActiveRunner) -> io::Result<()> {
    if active.endpoint.is_empty()
        || active.release.is_empty()
        || active.secret.len() != 32
        || !active
            .secret
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid active runner record",
        ));
    }
    Ok(())
}
