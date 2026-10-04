//! Git repositories a test makes, and a factory that fetches them, for the
//! plugin's tests and the backend's scenarios.

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use demi_shared_types::{Clock, Timestamp};

use crate::Skills;

/// The time every fetch ends at, until the test lets time pass.
pub const FETCHED_AT: i64 = 1_790_000_000_000;

/// The plugin's clock, which stands still until the test moves it.
struct Still(AtomicI64);

impl Clock for Still {
    fn now(&self) -> Timestamp {
        Timestamp::from_millisecond(self.0.load(Ordering::Relaxed)).unwrap()
    }
}

/// Repositories at `<root>/<owner>/<repo>`, which the factory fetches for
/// the origin `owner/repo`, and the clock of the plugins it makes.
pub struct Repos {
    root: tempfile::TempDir,
    clock: Arc<Still>,
}

/// A file of a commit: its path, its text, and whether it is executable.
pub type File<'a> = (&'a str, &'a str, bool);

impl Default for Repos {
    fn default() -> Self {
        Self {
            root: tempfile::tempdir().unwrap(),
            clock: Arc::new(Still(AtomicI64::new(FETCHED_AT))),
        }
    }
}

impl Repos {
    pub fn new() -> Self {
        Self::default()
    }

    /// Commits `files` as the whole tree of `owner/repo`, made on first use.
    pub fn commit(&self, name: &str, files: &[File<'_>]) {
        let repo = self.root.path().join(name);
        if !repo.exists() {
            std::fs::create_dir_all(&repo).unwrap();
            git(&repo, &["init", "-q", "-b", "main"]);
        }
        for entry in std::fs::read_dir(&repo).unwrap() {
            let entry = entry.unwrap();
            if entry.file_name() != ".git" {
                let path = entry.path();
                if path.is_dir() {
                    std::fs::remove_dir_all(path).unwrap();
                } else {
                    std::fs::remove_file(path).unwrap();
                }
            }
        }
        for (path, text, executable) in files {
            write(&repo.join(path), text.as_bytes(), *executable);
        }
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "--allow-empty", "-m", "skills"]);
    }

    /// Writes `bytes` into `owner/repo`'s tree and commits it with the rest.
    pub fn commit_bytes(&self, name: &str, path: &str, bytes: &[u8]) {
        let repo = self.root.path().join(name);
        write(&repo.join(path), bytes, false);
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "bytes"]);
    }

    /// Moves the plugins' clock `duration` on.
    pub fn pass(&self, duration: Duration) {
        let milliseconds = i64::try_from(duration.as_millis()).unwrap();
        self.clock.0.fetch_add(milliseconds, Ordering::Relaxed);
    }

    /// The plugin, fetching these repositories.
    pub fn skills(&self) -> Skills {
        let root = self.root.path().to_owned();
        let resolve = move |url: &str| {
            let name = url.strip_prefix("https://github.com/").unwrap_or(url);
            format!("file://{}", root.join(name).display())
        };
        Skills::resolving(Arc::new(resolve), self.clock.clone())
    }
}

fn write(path: &Path, bytes: &[u8], executable: bool) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if executable { 0o755 } else { 0o644 };
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }
}

fn git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args([
            "-c",
            "user.name=test",
            "-c",
            "user.email=test@example.test",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(repo)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

/// A `SKILL.md` with `front` as its front matter.
pub fn skill_md(front: &str) -> String {
    format!("---\n{front}\n---\n\nFollow these steps.\n")
}
