//! Git on one of the user's paired devices, which a plugin asks for through
//! device access (`crate`'s entry comment): the device runs its own `git`,
//! as its user, with that user's keys, credential helpers and network.
//!
//! - A check reads the commit a repository's default branch points to,
//!   `git ls-remote <url> HEAD`.
//! - A fetch clones the default branch's newest commit, bare and without
//!   the files' contents where the remote can leave them out, into a
//!   directory of the device's own under `~/.demi/tmp/`. It lists the
//!   commit's tree, fetches the objects of the files of every directory a
//!   file named `marker` marks in one request, and reads them: the commit's
//!   bytes, as the server's fetch reads them, with no checkout's line
//!   endings or filters. One request, since git would otherwise fetch each
//!   object on its own connection as it is read.
//!   The directory is removed afterwards, and a fetch first removes what an
//!   earlier one cut off left behind.
//!
//! `git` never waits for anyone: its standard input is closed, every way it
//! or SSH could ask for a credential is turned off, and each run has a time
//! limit, after which it is killed. A failure answers `git`'s own words.
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use futures_util::StreamExt;
use demi_host_interface::{FileKind, Host as _, ProcessEnd, RmOptions, SpawnEnv, SpawnRequest};
use demi_runner_client::RemoteHost;
use demi_shared_types::StreamKind;
use demi_web_api_protocol::devices::DeviceKind;
use demi_web_api_protocol::ids::DeviceId;

use crate::HostShard;

/// The fetches this backend started, which tells two fetches' directories
/// apart within one moment.
static FETCHES: AtomicU64 = AtomicU64::new(0);

/// What a fetch's directory is named after, followed by when it started.
const DIRECTORY_PREFIX: &str = "git-fetch-";

/// How old a fetch's directory is before a later fetch removes it: no fetch
/// runs this long, so it is one that was cut off.
const LEFT_BEHIND: Duration = Duration::from_secs(60 * 60);

/// The most bytes of one run's output Demi keeps: a large tree's listing
/// fits, and a file is read alone, within the files' limit.
const OUTPUT_MAX_BYTES: usize = 32 * 1024 * 1024;

/// How long a check may take before its `git` is killed.
const CHECK_LIMIT: Duration = Duration::from_secs(60);

/// How long a clone may take before its `git` is killed: a slow network
/// fetches a 64 MiB repository within it.
const CLONE_LIMIT: Duration = Duration::from_secs(10 * 60);

/// How long reading one object or the tree may take.
const READ_LIMIT: Duration = Duration::from_secs(2 * 60);

/// The variables that would point `git` at another repository than the one
/// a run names, which the run's environment removes.
const ROUTING_VARIABLES: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
    "GIT_CONFIG_PARAMETERS",
];

/// A commit's tree as a fetch read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitTree {
    pub commit: String,
    /// Every file of the tree, by its path in the repository.
    pub entries: Vec<GitEntry>,
    /// The bytes of the files of the directories `marker` marks.
    pub files: Vec<(String, Bytes)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitEntry {
    pub path: String,
    pub executable: bool,
}

/// Why git on a device answered nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceGitError {
    /// The device is not one of the user's paired devices.
    NotFound,
    /// Its runner is not connected, or went away during the run.
    Offline,
    /// The repository's objects hold more than `limit` bytes.
    RepositoryTooLarge { limit: usize },
    /// The marked directories' files hold more than `limit` bytes.
    FilesTooLarge { limit: usize },
    /// `git` or the device failed, in their words.
    Failed(String),
}

impl dyn HostShard + '_ {
    /// The commit `url`'s default branch points to, as `device` reads it.
    pub async fn device_git_head(&self, device: &DeviceId, url: &str) -> Result<String, DeviceGitError> {
        let host = self.git_device(device).await?;
        let listed = git(&host, None, &["ls-remote", "--", url, "HEAD"], CHECK_LIMIT, OUTPUT_MAX_BYTES).await?;
        let line = String::from_utf8_lossy(&listed);
        line.split_whitespace()
            .next()
            .map(str::to_owned)
            .ok_or_else(|| DeviceGitError::Failed("the repository holds no commit".into()))
    }

    /// The newest commit of `url`'s default branch as `device` clones it,
    /// with the files of the directories `marker` marks, which hold at most
    /// `files_max` bytes; the repository's objects at most `repository_max`.
    pub async fn device_git_fetch(
        &self,
        device: &DeviceId,
        url: &str,
        marker: &str,
        repository_max: usize,
        files_max: usize,
    ) -> Result<GitTree, DeviceGitError> {
        let host = self.git_device(device).await?;
        let paths = host.identity().paths;
        let base = paths.join(&host.identity().home_dir, ".demi/tmp");
        let base = host
            .fs()
            .directory(&base, true)
            .await
            .map_err(|error| DeviceGitError::Failed(error.to_string()))?;
        remove_left_behind(&host, &base).await;
        let started = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_millis());
        let number = FETCHES.fetch_add(1, Ordering::Relaxed);
        let directory = paths.join(&base, &format!("{DIRECTORY_PREFIX}{started}-{number}"));
        let fetched = clone_and_read(&host, url, &directory, marker, repository_max, files_max).await;
        // A removal that fails, or a fetch cut off before it, leaves a
        // directory the next fetch removes.
        if let Err(error) = host
            .fs()
            .rm(&directory, RmOptions { recursive: true, force: true })
            .await
        {
            tracing::warn!(event = "device_git.cleanup_failed", %device, %error, "a fetch's directory stayed on the device");
        }
        fetched
    }

    /// The Host of the user's paired device `device`, while it is connected.
    async fn git_device(&self, device: &DeviceId) -> Result<RemoteHost, DeviceGitError> {
        let paired = self
            .control()
            .device(device.clone())
            .await
            .map_err(|error| DeviceGitError::Failed(error.to_string()))?
            .is_some_and(|record| record.user == *self.user() && record.kind == DeviceKind::User);
        if !paired {
            return Err(DeviceGitError::NotFound);
        }
        if self.devices().link(device).is_none() {
            return Err(DeviceGitError::Offline);
        }
        self.devices().device_access(device).await.ok_or(DeviceGitError::Offline)
    }
}

/// Removes the fetches' directories in `base` older than [`LEFT_BEHIND`],
/// which fetches that were cut off left; one that fails stays for the next.
async fn remove_left_behind(host: &RemoteHost, base: &str) {
    let Ok(entries) = host.fs().read_dir(base).await else {
        return;
    };
    let limit = i64::try_from(LEFT_BEHIND.as_millis()).unwrap_or(i64::MAX);
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX));
    for entry in entries {
        if entry.kind != FileKind::Directory
            || !entry.name.starts_with(DIRECTORY_PREFIX)
            || now - entry.modified.as_millisecond() < limit
        {
            continue;
        }
        let path = host.identity().paths.join(base, &entry.name);
        let _ = host.fs().rm(&path, RmOptions { recursive: true, force: true }).await;
    }
}

async fn clone_and_read(
    host: &RemoteHost,
    url: &str,
    directory: &str,
    marker: &str,
    repository_max: usize,
    files_max: usize,
) -> Result<GitTree, DeviceGitError> {
    // Bare and without the files' contents, which the remote sends only for
    // the objects read below; a remote that cannot leave them out sends them
    // all, which the size check after it bounds.
    let clone = [
        "clone", "--bare", "--depth", "1", "--single-branch", "--no-tags", "--filter=blob:none", "--quiet", "--", url,
        directory,
    ];
    git(host, None, &clone, CLONE_LIMIT, OUTPUT_MAX_BYTES).await?;
    let commit = String::from_utf8_lossy(&git(host, Some(directory), &["rev-parse", "HEAD"], READ_LIMIT, 1024).await?)
        .trim()
        .to_owned();
    if commit.is_empty() {
        return Err(DeviceGitError::Failed("the repository holds no commit".into()));
    }
    let counted = git(host, Some(directory), &["count-objects", "-v"], READ_LIMIT, 4096).await?;
    let packed_kib: usize = String::from_utf8_lossy(&counted)
        .lines()
        .filter_map(|line| line.split_once(": "))
        .filter(|(name, _)| *name == "size" || *name == "size-pack")
        .filter_map(|(_, value)| value.trim().parse::<usize>().ok())
        .sum();
    if packed_kib.saturating_mul(1024) > repository_max {
        return Err(DeviceGitError::RepositoryTooLarge { limit: repository_max });
    }
    let listing = git(host, Some(directory), &["ls-tree", "-r", "-z", "HEAD"], READ_LIMIT, OUTPUT_MAX_BYTES).await?;
    let mut entries = Vec::new();
    let mut objects = Vec::new();
    for record in listing.split(|byte| *byte == 0).filter(|record| !record.is_empty()) {
        let record = String::from_utf8_lossy(record);
        // `<mode> <type> <object>\t<path>`
        let Some((head, path)) = record.split_once('\t') else {
            continue;
        };
        let mut fields = head.split_whitespace();
        let (Some(mode), Some(kind), Some(object)) = (fields.next(), fields.next(), fields.next()) else {
            continue;
        };
        if kind != "blob" || !(mode == "100644" || mode == "100755") {
            continue;
        }
        entries.push(GitEntry { path: path.to_owned(), executable: mode == "100755" });
        objects.push(object.to_owned());
    }
    let marked: BTreeSet<&str> = entries
        .iter()
        .filter_map(|entry| match entry.path.rsplit_once('/') {
            Some((parent, name)) if name == marker => Some(parent),
            None if entry.path == marker => Some(""),
            _ => None,
        })
        .collect();
    let below = |path: &str| {
        marked
            .iter()
            .any(|parent| parent.is_empty() || path.strip_prefix(parent).is_some_and(|rest| rest.starts_with('/')))
    };
    let wanted: Vec<(&GitEntry, &String)> = entries
        .iter()
        .zip(&objects)
        .filter(|(entry, _)| below(&entry.path))
        .collect();
    let promisor = git(host, Some(directory), &["config", "--get", "remote.origin.promisor"], READ_LIMIT, 1024)
        .await
        .is_ok_and(|value| String::from_utf8_lossy(&value).trim() == "true");
    if promisor && !wanted.is_empty() {
        let mut list = String::new();
        for (_, object) in &wanted {
            list.push_str(object);
            list.push('\n');
        }
        let fetch = [
            "-c", "fetch.negotiationAlgorithm=noop", "fetch", "--quiet", "--no-tags", "--no-write-fetch-head",
            "--recurse-submodules=no", "--filter=blob:none", "--stdin", "origin",
        ];
        run(host, Some(directory), &fetch, Some(Bytes::from(list)), CLONE_LIMIT, OUTPUT_MAX_BYTES).await?;
    }
    let mut files = Vec::new();
    let mut total = 0usize;
    for (entry, object) in wanted {
        // The object's own bytes: what the commit holds, as the server reads
        // it. A read stops at what the files may still hold.
        let left = files_max.saturating_sub(total);
        let bytes = git(host, Some(directory), &["cat-file", "blob", object], READ_LIMIT, left)
            .await
            .map_err(|error| match error {
                DeviceGitError::FilesTooLarge { .. } => DeviceGitError::FilesTooLarge { limit: files_max },
                error => error,
            })?;
        total += bytes.len();
        files.push((entry.path.clone(), Bytes::from(bytes)));
    }
    Ok(GitTree { commit, entries, files })
}

/// Runs `git args` on `host`, in `directory` or the Host's default, and
/// answers its standard output, of at most `max_output` bytes; a run that
/// writes more fails with [`DeviceGitError::FilesTooLarge`], one that takes
/// longer than `limit` is killed, and a nonzero exit fails with its
/// standard error, or its output when that is empty.
async fn git(
    host: &RemoteHost,
    directory: Option<&str>,
    args: &[&str],
    limit: Duration,
    max_output: usize,
) -> Result<Vec<u8>, DeviceGitError> {
    run(host, directory, args, None, limit, max_output).await
}

/// [`git`], with `input` written to its standard input before it closes.
async fn run(
    host: &RemoteHost,
    directory: Option<&str>,
    args: &[&str],
    input: Option<Bytes>,
    limit: Duration,
    max_output: usize,
) -> Result<Vec<u8>, DeviceGitError> {
    let mut command: Vec<String> = Vec::new();
    if let Some(directory) = directory {
        command.extend(["-C".to_owned(), directory.to_owned()]);
    }
    command.extend(args.iter().map(|arg| (*arg).to_owned()));
    let mut env: std::collections::BTreeMap<String, Option<String>> = [
        // No prompt: git's own, SSH's, or a credential manager's.
        ("GIT_TERMINAL_PROMPT", Some("0")),
        ("GCM_INTERACTIVE", Some("never")),
        ("SSH_ASKPASS_REQUIRE", Some("never")),
        ("GIT_ASKPASS", None),
        ("SSH_ASKPASS", None),
        // A read of an object the fetch did not bring fails at once rather
        // than fetching it alone; a git before 2.44 fetches it all the same.
        ("GIT_NO_LAZY_FETCH", Some("1")),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_owned(), value.map(str::to_owned)))
    .collect();
    env.extend(ROUTING_VARIABLES.iter().map(|name| ((*name).to_owned(), None)));
    let request = SpawnRequest {
        command: "git".into(),
        args: command,
        cwd: None,
        env: SpawnEnv::Overlay(env),
        descriptors: Vec::new(),
        retained: false,
    };
    let mut process = host
        .process()
        .spawn(request)
        .await
        .map_err(|error| DeviceGitError::Failed(error.to_string()))?;
    if let Some(input) = input {
        process
            .control
            .write_stdin(input)
            .await
            .map_err(|error| DeviceGitError::Failed(error.to_string()))?;
    }
    // Nothing answers a question git asks on its standard input.
    let _ = process.control.close_stdin().await;
    let run = async {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        while let Some(chunk) = process.output.next().await {
            let (into, cap) = match chunk.stream {
                StreamKind::Stdout => (&mut stdout, max_output),
                StreamKind::Stderr => (&mut stderr, 64 * 1024),
            };
            if into.len() + chunk.bytes.len() > cap {
                if matches!(chunk.stream, StreamKind::Stdout) {
                    return Err(DeviceGitError::FilesTooLarge { limit: max_output });
                }
                continue;
            }
            into.extend_from_slice(&chunk.bytes);
        }
        Ok((stdout, stderr))
    };
    // Dropping the process on any early return kills it.
    let (stdout, stderr) = tokio::time::timeout(limit, run)
        .await
        .map_err(|_| DeviceGitError::Failed(format!("git did not answer within {} seconds", limit.as_secs())))??;
    match process.exit.await {
        ProcessEnd::Exited(0) => Ok(stdout),
        ProcessEnd::Exited(_) | ProcessEnd::Signalled(_) => {
            let words = String::from_utf8_lossy(if stderr.is_empty() { &stdout } else { &stderr }).trim().to_owned();
            Err(DeviceGitError::Failed(if words.is_empty() { "git failed".into() } else { words }))
        }
        ProcessEnd::NotStarted(error) => Err(DeviceGitError::Failed(format!("git did not start: {error:?}"))),
        ProcessEnd::Lost(reason) => {
            tracing::info!(event = "device_git.lost", %reason, "a device went away during git");
            Err(DeviceGitError::Offline)
        }
    }
}
