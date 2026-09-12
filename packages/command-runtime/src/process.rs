use std::{collections::BTreeMap, path::Path, process::Stdio, sync::Arc, time::Duration};

use demi_command_protocol::PackageDescriptor;
use demi_command_service::Client;
use tokio::{
    io::AsyncReadExt,
    process::Command,
    sync::Mutex,
    task::{JoinHandle, JoinSet},
};
use tokio_util::sync::CancellationToken;

use crate::RuntimeError;

const START_TIMEOUT: Duration = Duration::from_secs(10);
const STOP_TIMEOUT: Duration = Duration::from_secs(6);
const STDERR_LIMIT: usize = 16 * 1024;

/// One resident executable owned by one runner registration and artifact digest.
/// The catalog retains this object while environments or calls reference it.
pub struct ResidentService {
    client: Client,
    cancel: CancellationToken,
    owner: Option<JoinHandle<Result<(), RuntimeError>>>,
    diagnostics: Arc<Mutex<Vec<u8>>>,
    pid: u32,
}

impl ResidentService {
    pub async fn start(
        executable: &Path,
        descriptor: &PackageDescriptor,
        cwd: &Path,
        env: &BTreeMap<String, String>,
    ) -> Result<Self, RuntimeError> {
        let mut command = Command::new(executable);
        command
            .arg("--command-service")
            .current_dir(cwd)
            .env_clear()
            .envs(env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn()?;
        let pid = child.id().expect("newly spawned process has an ID");
        let input = child.stdin.take().expect("piped stdin");
        let output = child.stdout.take().expect("piped stdout");
        let mut stderr = child.stderr.take().expect("piped stderr");
        let (client, connection) = match tokio::time::timeout(
            START_TIMEOUT,
            Client::connect(tokio::io::join(output, input)),
        )
        .await
        {
            Ok(Ok(result)) => result,
            result => {
                child.kill().await?;
                return Err(match result {
                    Ok(Err(error)) => error.into(),
                    Err(_) => RuntimeError::Deadline("handshake"),
                    Ok(Ok(_)) => unreachable!(),
                });
            }
        };
        let diagnostics = Arc::new(Mutex::new(Vec::new()));
        let diagnostics_writer = diagnostics.clone();
        let cancel = CancellationToken::new();
        let owner_cancel = cancel.clone();
        let shutdown_client = client.clone();
        let owner = tokio::spawn(async move {
            let mut tasks = JoinSet::new();
            tasks.spawn(async move {
                connection.await.map_err(|error| {
                    RuntimeError::Artifact(format!("native connection: {error}"))
                })?;
                Ok(true)
            });
            tasks.spawn(async move {
                let mut buffer = [0; 4096];
                loop {
                    let count = stderr.read(&mut buffer).await?;
                    if count == 0 {
                        return Ok(false);
                    }
                    let mut tail = diagnostics_writer.lock().await;
                    let remove = (tail.len() + count).saturating_sub(STDERR_LIMIT);
                    tail.drain(..remove);
                    tail.extend_from_slice(&buffer[..count]);
                }
            });
            let result = loop {
                tokio::select! {
                    _ = owner_cancel.cancelled() => {
                        let graceful = tokio::time::timeout(STOP_TIMEOUT, async {
                            shutdown_client.shutdown().await?;
                            child.wait().await?;
                            Ok::<_, RuntimeError>(())
                        }).await;
                        break match graceful {
                            Ok(result) => result,
                            Err(_) => Err(RuntimeError::Deadline("shutdown")),
                        };
                    }
                    status = child.wait() => {
                        break match status {
                            Ok(status) if status.success() => Ok(()),
                            Ok(status) => Err(RuntimeError::Artifact(format!("native process exited with {status}"))),
                            Err(error) => Err(error.into()),
                        };
                    }
                    task = tasks.join_next(), if !tasks.is_empty() => {
                        match task {
                            Some(Ok(Ok(true))) => break Err(RuntimeError::Artifact("native connection closed".into())),
                            Some(Ok(Ok(false))) => {},
                            Some(Ok(Err(error))) => break Err(error),
                            Some(Err(error)) => break Err(RuntimeError::Artifact(format!("native transport task: {error}"))),
                            None => {},
                        }
                        if tasks.is_empty() {
                            break Err(RuntimeError::Artifact("native service closed its transports".into()));
                        }
                    }
                }
            };
            // Reap even after transport failure; kill_on_drop also covers owner cancellation.
            let reaped = if child.try_wait()?.is_none() {
                child.kill().await
            } else {
                Ok(())
            };
            tasks.abort_all();
            while tasks.join_next().await.is_some() {}
            reaped?;
            result
        });
        let service = Self {
            client,
            cancel,
            owner: Some(owner),
            diagnostics,
            pid,
        };
        let info = tokio::time::timeout(START_TIMEOUT, service.client.info()).await;
        let info = match info {
            Ok(result) => result?,
            Err(_) => return Err(RuntimeError::Deadline("catalog")),
        };
        let mut actual = info.operations;
        let mut expected = descriptor.operations.clone();
        actual.sort();
        expected.sort();
        if u64::from(info.protocol_version) != descriptor.protocol_version || actual != expected {
            return Err(RuntimeError::CatalogMismatch);
        }
        Ok(service)
    }

    pub fn client(&self) -> &Client {
        &self.client
    }
    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub async fn diagnostics(&self) -> Vec<u8> {
        self.diagnostics.lock().await.clone()
    }

    pub async fn shutdown(mut self) -> Result<(), RuntimeError> {
        self.cancel.cancel();
        self.owner
            .take()
            .expect("service owner exists")
            .await
            .map_err(|error| RuntimeError::Artifact(format!("native process owner: {error}")))?
    }
}

impl Drop for ResidentService {
    fn drop(&mut self) {
        // The owner continues only long enough to drain or kill and reap the child.
        self.cancel.cancel();
    }
}
