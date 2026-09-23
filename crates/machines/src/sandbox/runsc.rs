//! The runsc commands the manager drives (`managed-hosts.md` § Isolation and
//! joining): every call names the manager's state root and the one runtime
//! profile, and the pinned release must be the one installed.

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

/// The runtime profile: systrap, gVisor's own network stack in the slot's
/// namespace, no temporary root overlay (system writes land in the device's
/// image), shared file access so the images can be saved while the sandbox
/// is paused, and setuid programs for sudo.
pub const PROFILE_FLAGS: [&str; 7] = [
    "--platform=systrap",
    "--network=sandbox",
    "--overlay2=none",
    "--file-access=shared",
    "--file-access-mounts=shared",
    "--allow-suid=true",
    "--directfs=true",
];

/// A runsc command that does not finish in this time has hung.
const DEADLINE: Duration = Duration::from_secs(60);

/// The runtime release the manager is built with (`runtime/release.json`).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeRelease {
    pub upstream: String,
    pub commit: String,
    pub arm64_version: String,
    pub arm64_patch_sha256: String,
    pub amd64_archive_sha512: String,
    pub bazel: String,
    pub bazel_arm64_sha256: String,
}

impl RuntimeRelease {
    // The release manifest stays with the runtime's build inputs until they
    // move into this crate with the deployment scripts.
    const MANIFEST: &str = include_str!("../../../../packages/machines/runtime/release.json");

    pub fn pinned() -> Self {
        serde_json::from_str(Self::MANIFEST).expect("the pinned runtime release is valid")
    }

    /// The version runsc reports on this architecture: arm64 runs the build
    /// with the seccomp trap fix, amd64 the upstream release.
    pub fn version(&self) -> String {
        if cfg!(target_arch = "aarch64") {
            self.arm64_version.clone()
        } else {
            format!("release-{}", self.upstream)
        }
    }
}

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
    pub fn new(tools: Tools, runtime: &Path) -> Self {
        Self {
            tools,
            root: runtime.join("runsc"),
        }
    }

    /// Where runsc keeps its containers' state.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The arguments of `command`: the state root, the profile, the command.
    pub fn args<'a>(&self, command: impl IntoIterator<Item = &'a OsStr>) -> Vec<OsString> {
        let mut args = vec![OsString::from(format!("--root={}", self.root.display()))];
        args.extend(PROFILE_FLAGS.map(OsString::from));
        args.extend(command.into_iter().map(OsStr::to_owned));
        args
    }

    async fn run(&self, command: &[&OsStr]) -> Result<Output, ToolError> {
        self.tools.run(Tool::Runsc, self.args(command.iter().copied()), Some(DEADLINE)).await
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
        let output = self.run(&[OsStr::new("list"), OsStr::new("--format=json")]).await?;
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
        Err(RunscError::Start(crate::tools::tail(&text, 8 * 1024).to_owned()))
    }

    /// Waits for container `id` to exit, as long as it runs.
    pub async fn wait(&self, id: &SandboxId) -> Result<(), ToolError> {
        let output = self
            .tools
            .output(Tool::Runsc, self.args([OsStr::new("wait"), OsStr::new(id.as_str())]), None)
            .await?;
        Tools::accept(Tool::Runsc, output, &[0]).map(drop)
    }

    pub async fn pause(&self, id: &SandboxId) -> Result<(), ToolError> {
        self.run(&[OsStr::new("pause"), OsStr::new(id.as_str())]).await.map(drop)
    }

    pub async fn resume(&self, id: &SandboxId) -> Result<(), ToolError> {
        self.run(&[OsStr::new("resume"), OsStr::new(id.as_str())]).await.map(drop)
    }

    /// Asks every process of container `id` to terminate.
    pub async fn terminate(&self, id: &SandboxId) -> Result<Output, ToolError> {
        self.tools
            .output(
                Tool::Runsc,
                self.args([OsStr::new("kill"), OsStr::new("--all"), OsStr::new(id.as_str()), OsStr::new("TERM")]),
                Some(DEADLINE),
            )
            .await
    }

    /// Deletes container `id`, killing what still runs.
    pub async fn delete(&self, id: &SandboxId) -> Result<(), ToolError> {
        self.run(&[OsStr::new("delete"), OsStr::new("--force"), OsStr::new(id.as_str())])
            .await
            .map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_must_match_exactly() {
        let version = RuntimeRelease::pinned().version();
        assert!(reports_version(&format!("runsc version {version}\nspec: 1.1.0\n"), &version));
        assert!(!reports_version(&format!("runsc version {version}.1\n"), &version));
        assert!(!reports_version("runsc version release-20260914.0-demi.1\n", "release-20260914.0"));
        assert!(!reports_version(&format!("spec: 1.1.0\nrunsc version {version}\n"), &version));
    }

    #[test]
    fn a_listing_names_each_containers_status() {
        let id: SandboxId = serde_json::from_str(r#""demi-a""#).unwrap();
        assert_eq!(status_in("null", &id).unwrap(), None);
        assert_eq!(status_in("[]", &id).unwrap(), None);
        let listing = r#"[{"id":"demi-b","pid":7,"status":"running"},{"id":"demi-a","pid":9,"status":"paused","bundle":"/x"}]"#;
        assert_eq!(status_in(listing, &id).unwrap(), Some(Status::Paused));
        assert!(status_in(r#"[{"id":"demi-a","status":"exploded"}]"#, &id).is_err());
    }

    #[test]
    fn every_command_names_the_state_root_and_the_profile() {
        let runsc = Runsc::new(Tools::placeholder(), Path::new("/run/demi-machines"));
        let args = runsc.args([OsStr::new("pause"), OsStr::new("demi-a")]);
        let args: Vec<_> = args.iter().map(|arg| arg.to_str().unwrap()).collect();
        assert_eq!(
            args,
            [
                "--root=/run/demi-machines/runsc",
                "--platform=systrap",
                "--network=sandbox",
                "--overlay2=none",
                "--file-access=shared",
                "--file-access-mounts=shared",
                "--allow-suid=true",
                "--directfs=true",
                "pause",
                "demi-a",
            ]
        );
    }
}
