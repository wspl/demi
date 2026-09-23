//! The external programs the manager runs (`managed-hosts.md` § Linux
//! control): `runsc`, `mke2fs`, `e2fsck`, `resize2fs`, `bsdtar` and `nft`.
//! Each starts from the loop, which only spawns it and waits for its exit,
//! with the C locale's UTF-8 variant so its messages have one form. Dropping
//! a running tool kills it.

use std::{
    ffi::OsStr,
    fmt,
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
    time::Duration,
};

/// The programs the manager runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Runsc,
    Mke2fs,
    E2fsck,
    Resize2fs,
    Bsdtar,
    Nft,
}

impl Tool {
    /// The programs found on `PATH`; runsc is configured.
    const ON_PATH: [Tool; 5] = [Self::Mke2fs, Self::E2fsck, Self::Resize2fs, Self::Bsdtar, Self::Nft];

    pub fn name(self) -> &'static str {
        match self {
            Self::Runsc => "runsc",
            Self::Mke2fs => "mke2fs",
            Self::E2fsck => "e2fsck",
            Self::Resize2fs => "resize2fs",
            Self::Bsdtar => "bsdtar",
            Self::Nft => "nft",
        }
    }
}

impl fmt::Display for Tool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// The resolved programs.
#[derive(Debug, Clone)]
pub struct Tools {
    runsc: PathBuf,
    mke2fs: PathBuf,
    e2fsck: PathBuf,
    resize2fs: PathBuf,
    bsdtar: PathBuf,
    nft: PathBuf,
}

/// Programs that are not installed.
#[derive(Debug, thiserror::Error)]
#[error("Cloud manager needs: {}", .0.join(", "))]
pub struct MissingTools(Vec<&'static str>);

/// A program that could not run or did not succeed. The message carries the
/// end of its output, which names what went wrong.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("{tool} could not start: {source}")]
    Spawn { tool: Tool, source: std::io::Error },
    #[error("{tool} did not finish within {} s", deadline.as_secs())]
    Deadline { tool: Tool, deadline: Duration },
    #[error("{tool} exited {}: {}", describe(*status), output.message())]
    Failed { tool: Tool, status: ExitStatus, output: Output },
}

/// What a program wrote, decoded lossily: tools write text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    /// The error output, or the standard output when there is none, trimmed
    /// and bounded to its last 8 KiB.
    pub fn message(&self) -> &str {
        let text = if self.stderr.trim().is_empty() {
            self.stdout.trim()
        } else {
            self.stderr.trim()
        };
        tail(text, 8 * 1024)
    }
}

/// The last `bytes` of `text`, cut at a character boundary.
pub fn tail(text: &str, bytes: usize) -> &str {
    let mut start = text.len().saturating_sub(bytes);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

fn describe(status: ExitStatus) -> String {
    match status.code() {
        Some(code) => code.to_string(),
        None => "by signal".into(),
    }
}

impl Tools {
    /// Finds every program: `runsc` at its configured path, the others on
    /// `PATH`.
    pub fn resolve(runsc: &Path) -> Result<Self, MissingTools> {
        let mut missing = Vec::new();
        let mut find = |tool: Tool| match which::which(tool.name()) {
            Ok(path) => path,
            Err(_) => {
                missing.push(tool.name());
                PathBuf::new()
            }
        };
        let [mke2fs, e2fsck, resize2fs, bsdtar, nft] = Tool::ON_PATH.map(&mut find);
        if !runsc.is_file() {
            missing.push(Tool::Runsc.name());
        }
        if !missing.is_empty() {
            return Err(MissingTools(missing));
        }
        Ok(Self {
            runsc: runsc.to_owned(),
            mke2fs,
            e2fsck,
            resize2fs,
            bsdtar,
            nft,
        })
    }

    /// Programs with no paths, for tests that only build command lines.
    #[cfg(test)]
    pub(crate) fn placeholder() -> Self {
        Self {
            runsc: PathBuf::new(),
            mke2fs: PathBuf::new(),
            e2fsck: PathBuf::new(),
            resize2fs: PathBuf::new(),
            bsdtar: PathBuf::new(),
            nft: PathBuf::new(),
        }
    }

    /// The programs on `PATH`, for tests that do not run runsc.
    #[cfg(all(test, target_os = "linux"))]
    pub(crate) fn on_path() -> Self {
        let find = |tool: Tool| which::which(tool.name()).unwrap_or_else(|_| panic!("{tool} is not installed"));
        Self {
            runsc: PathBuf::from("runsc"),
            mke2fs: find(Tool::Mke2fs),
            e2fsck: find(Tool::E2fsck),
            resize2fs: find(Tool::Resize2fs),
            bsdtar: find(Tool::Bsdtar),
            nft: find(Tool::Nft),
        }
    }

    pub fn path(&self, tool: Tool) -> &Path {
        match tool {
            Tool::Runsc => &self.runsc,
            Tool::Mke2fs => &self.mke2fs,
            Tool::E2fsck => &self.e2fsck,
            Tool::Resize2fs => &self.resize2fs,
            Tool::Bsdtar => &self.bsdtar,
            Tool::Nft => &self.nft,
        }
    }

    /// A command for `tool` with the manager's environment and no input; it
    /// is killed when dropped.
    pub fn command(&self, tool: Tool) -> tokio::process::Command {
        // The one place the manager starts a program other than itself.
        let mut command = tokio::process::Command::new(self.path(tool));
        command
            .env("LC_ALL", "C.UTF-8")
            .stdin(Stdio::null())
            .kill_on_drop(true);
        command
    }

    /// Runs `tool` to its end, within `deadline` when one is given, and
    /// returns its output whatever its exit status.
    pub async fn output<A: AsRef<OsStr>>(
        &self,
        tool: Tool,
        args: impl IntoIterator<Item = A>,
        deadline: Option<Duration>,
    ) -> Result<Output, ToolError> {
        let mut command = self.command(tool);
        command.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
        let child = command
            .spawn()
            .map_err(|source| ToolError::Spawn { tool, source })?;
        let finished = child.wait_with_output();
        let output = match deadline {
            Some(deadline) => tokio::time::timeout(deadline, finished)
                .await
                .map_err(|_| ToolError::Deadline { tool, deadline })?,
            None => finished.await,
        }
        .map_err(|source| ToolError::Spawn { tool, source })?;
        Ok(Output {
            status: output.status,
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    /// Runs `tool` and requires it to exit 0.
    pub async fn run<A: AsRef<OsStr>>(
        &self,
        tool: Tool,
        args: impl IntoIterator<Item = A>,
        deadline: Option<Duration>,
    ) -> Result<Output, ToolError> {
        let output = self.output(tool, args, deadline).await?;
        if !output.status.success() {
            return Err(ToolError::Failed {
                tool,
                status: output.status,
                output,
            });
        }
        Ok(output)
    }

    /// Requires `output` of `tool` to have exited with one of `codes`.
    pub fn accept(tool: Tool, output: Output, codes: &[i32]) -> Result<Output, ToolError> {
        if output.status.code().is_some_and(|code| codes.contains(&code)) {
            return Ok(output);
        }
        Err(ToolError::Failed {
            tool,
            status: output.status,
            output,
        })
    }
}
