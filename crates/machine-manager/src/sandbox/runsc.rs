//! The runsc commands the manager drives: every call names the manager's
//! state root and the one runtime profile, and the pinned release must be
//! the one installed.

use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use serde::Deserialize;

use super::files::SandboxId;
use crate::{
    blocking,
    tools::{Output, Tool, ToolError, Tools},
};

/// The runtime profile:
///
/// - systrap.
/// - gVisor's own network stack in the slot's namespace.
/// - No temporary root overlay (system writes land in the device's image).
/// - Shared file access, so the images can be saved while the sandbox is
///   paused.
/// - Setuid programs for sudo.
/// - Only the sidecar programs beside runsc. Without the strict policy runsc
///   downloads a missing one or falls back to a deprecated embedded copy, where
///   a Cloud runs only the programs `demi-server` fetched and checked.
///
/// The sandbox may connect to the host's Unix sockets its mounts show, of which
/// there is one: the backend's runner socket.
///
/// Shared file access, with no forced private page cache, is a deliberate
/// persistence choice whose performance must be measured.
pub const PROFILE_FLAGS: [&str; 9] = [
    "--platform=systrap",
    "--network=sandbox",
    "--overlay2=none",
    "--file-access=shared",
    "--file-access-mounts=shared",
    "--allow-suid=true",
    "--directfs=true",
    "--sidecar-usage-policy=STRICT",
    "--host-uds=open",
];

/// A runsc command that does not finish in this time has hung.
const DEADLINE: Duration = Duration::from_secs(60);

/// Whether `runsc --version` printed exactly `version` on its first line.
pub fn reports_version(output: &str, version: &str) -> bool {
    output.lines().next() == Some(&format!("runsc version {version}"))
}

/// A container's state, as `runsc list` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Creating,
    Created,
    Running,
    Paused,
    Stopped,
}

#[derive(Debug, Deserialize)]
struct Container {
    id: String,
    status: Status,
}

/// The status of container `id` in `runsc list --format=json` output, which
/// is `null` when there are no containers.
pub fn status_in(listing: &str, id: &SandboxId) -> Result<Option<Status>, serde_json::Error> {
    let containers: Option<Vec<Container>> = serde_json::from_str(listing)?;
    Ok(containers
        .unwrap_or_default()
        .into_iter()
        .find(|container| container.id == id.as_str())
        .map(|container| container.status))
}

/// runsc, with its state under the manager's runtime directory.
#[derive(Debug, Clone)]
pub struct Runsc {
    tools: Tools,
    root: PathBuf,
    /// Whether runsc sets up each sandbox's cgroup: only with the resource
    /// limits on.
    cgroups: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum RunscError {
    #[error(transparent)]
    Tool(#[from] ToolError),
    #[error("Cannot inspect Cloud runtimes: {0}")]
    List(String),
    #[error("Cloud start failed: {0}")]
    Start(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl Runsc {
    pub fn new(tools: Tools, runtime: &Path, cgroups: bool) -> Self {
        Self {
            tools,
            root: runtime.join("runsc"),
            cgroups,
        }
    }

    /// Where runsc keeps its containers' state.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The arguments of `command`: the state root, the profile, the command.
    /// Without cgroups the profile says so: runsc gives a configuration
    /// without a cgroup path one of its own.
    pub fn args<'a>(&self, command: impl IntoIterator<Item = &'a OsStr>) -> Vec<OsString> {
        let mut args = vec![OsString::from(format!("--root={}", self.root.display()))];
        args.extend(PROFILE_FLAGS.map(OsString::from));
        if !self.cgroups {
            args.push(OsString::from("--ignore-cgroups"));
        }
        args.extend(command.into_iter().map(OsStr::to_owned));
        args
    }

    async fn run(&self, command: &[&OsStr]) -> Result<Output, ToolError> {
        self.tools
            .run(
                Tool::Runsc,
                self.args(command.iter().copied()),
                Some(DEADLINE),
            )
            .await
    }

    /// The installed runsc's version line.
    pub async fn version(&self) -> Result<String, ToolError> {
        let output = self
            .tools
            .run(Tool::Runsc, ["--version"], Some(DEADLINE))
            .await?;
        Ok(output.stdout)
    }

    /// The status of container `id`, `None` when runsc does not know it.
    pub async fn status(&self, id: &SandboxId) -> Result<Option<Status>, RunscError> {
        let output = self
            .run(&[OsStr::new("list"), OsStr::new("--format=json")])
            .await?;
        status_in(&output.stdout, id).map_err(|error| RunscError::List(error.to_string()))
    }

    /// Starts container `id` from `bundle`, detached. runsc and the sandbox
    /// write to `log`, the bundle's log file: a detached sandbox keeps its
    /// standard output, so a pipe would never reach its end.
    pub async fn start(
        &self,
        id: &SandboxId,
        bundle: &Path,
        log: (std::fs::File, PathBuf),
    ) -> Result<(), RunscError> {
        let (file, path) = log;
        let mut command = self.tools.command(Tool::Runsc);
        command
            .args(self.args([
                OsStr::new("run"),
                OsStr::new("--detach"),
                OsStr::new("--bundle"),
                bundle.as_os_str(),
                OsStr::new(id.as_str()),
            ]))
            .stdout(Stdio::from(file.try_clone()?))
            .stderr(Stdio::from(file));
        let mut child = command.spawn().map_err(|source| ToolError::Spawn {
            tool: Tool::Runsc,
            source,
        })?;
        let status = tokio::time::timeout(DEADLINE, child.wait())
            .await
            .map_err(|_| ToolError::Deadline {
                tool: Tool::Runsc,
                deadline: DEADLINE,
            })?
            .map_err(|source| ToolError::Spawn {
                tool: Tool::Runsc,
                source,
            })?;
        if status.success() {
            return Ok(());
        }
        let written = blocking::run(move |_| fs_err::read(path)).await?;
        let text = String::from_utf8_lossy(&written);
        Err(RunscError::Start(
            crate::tools::tail(&text, 8 * 1024).to_owned(),
        ))
    }

    /// Waits for container `id` to exit, as long as it runs.
    pub async fn wait(&self, id: &SandboxId) -> Result<(), ToolError> {
        let output = self
            .tools
            .output(
                Tool::Runsc,
                self.args([OsStr::new("wait"), OsStr::new(id.as_str())]),
                None,
            )
            .await?;
        Tools::accept(Tool::Runsc, output, &[0]).map(drop)
    }

    pub async fn pause(&self, id: &SandboxId) -> Result<(), ToolError> {
        self.run(&[OsStr::new("pause"), OsStr::new(id.as_str())])
            .await
            .map(drop)
    }

    pub async fn resume(&self, id: &SandboxId) -> Result<(), ToolError> {
        self.run(&[OsStr::new("resume"), OsStr::new(id.as_str())])
            .await
            .map(drop)
    }

    /// Asks every process of container `id` to terminate.
    pub async fn terminate(&self, id: &SandboxId) -> Result<Output, ToolError> {
        self.tools
            .output(
                Tool::Runsc,
                self.args([
                    OsStr::new("kill"),
                    OsStr::new("--all"),
                    OsStr::new(id.as_str()),
                    OsStr::new("TERM"),
                ]),
                Some(DEADLINE),
            )
            .await
    }

    /// Deletes container `id`, killing what still runs.
    pub async fn delete(&self, id: &SandboxId) -> Result<(), ToolError> {
        self.run(&[
            OsStr::new("delete"),
            OsStr::new("--force"),
            OsStr::new(id.as_str()),
        ])
        .await
        .map(drop)
    }
}
