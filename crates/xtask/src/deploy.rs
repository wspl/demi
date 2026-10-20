//! `xtask deploy`: moves a server that runs Demi to the next version, built on
//! GitHub. It checks that the checkout is clean and its branch even with origin's,
//! writes the next patch version into `Cargo.toml` and `Cargo.lock`, commits it as
//! `chore: release <version>` and pushes it; starts the release workflow on the
//! branch as a pre-release, checks that the run builds that commit and waits for
//! it; then, in one SSH session, runs `demi-server upgrade <version>` and
//! `demi-server status` on the server. Nothing is built here.
//!
//! Every deploy is a version of its own, so the repository always says which
//! version a server runs. A pre-release is never the latest release, which the
//! installer and `demi-server upgrade` without a version take, so only a deploy
//! installs one until it is published; a version the user is content with is
//! published by turning its pre-release into the latest release, so the files
//! users install are the ones the server ran, built once. Every version is kept,
//! published or not. A workflow run that fails ends the deploy with the failing
//! job's name and address, before the server is touched. A deploy that changes a
//! schema migrates the server's databases as an upgrade does, and the first
//! change to a schema after a deploy adds the deployed schema to the history (see
//! `backend-database`): a schema changed again after a deploy shipped it, in the
//! same unpublished batch, extends that step, and one that was changed without it
//! leaves the server's databases at a version the next build does not know, which
//! the upgrade's data check reports before anything is interrupted.

use std::io::{BufRead as _, BufReader, Write as _};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use semver::{BuildMetadata, Prerelease, Version};

/// The installed `demi-server`, by its path: sudo's search path may leave out `/usr/local/bin`.
const DEMI_SERVER: &str = "/usr/local/bin/demi-server";
/// The release workflow, which a deploy starts as a pre-release.
const WORKFLOW: &str = "release.yml";
/// The table of `Cargo.toml` whose `version` is the workspace version.
const WORKSPACE_TABLE: &str = "[workspace.package]";
/// How often `gh run watch` asks GitHub about the run, in seconds: GitHub
/// announces a run's end to nobody outside it, and a run takes many
/// minutes.
const WATCH_INTERVAL: &str = "30";
/// What a line of the remote session starts with when the session begins a
/// stage, the stage's name after it.
const STAGE_MARK: &str = "::demi-deploy:: ";
/// The remote session's last stage, whose output is the server's status.
const STATUS: &str = "status";

#[derive(clap::Args)]
pub struct Options {
    /// The SSH host of the server, which runs Demi already, as root or as a
    /// user who may use sudo without a password [default: DEMI_DEPLOY_HOST in
    /// the checkout's .env].
    #[arg(long, env = "DEMI_DEPLOY_HOST", value_name = "HOST")]
    host: Option<String>,
}

/// Why a deploy stopped.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no server to deploy to: pass --host or set DEMI_DEPLOY_HOST in the checkout's .env")]
    NoHost,
    #[error("the checkout has uncommitted changes, which a deploy never builds; commit or remove them:\n{0}")]
    Uncommitted(String),
    #[error("HEAD is on no branch: a deploy commits its version on the checkout's branch")]
    NoBranch,
    #[error("{branch} is at {head} here and {remote} on origin: a deploy starts from a branch even with GitHub's; push or pull it first")]
    NotEven {
        head: String,
        branch: String,
        remote: String,
    },
    #[error("Cargo.toml names no version in {WORKSPACE_TABLE}")]
    NoVersion,
    #[error("Cargo.toml's workspace version {0} is no version: {1}")]
    BadVersion(String, semver::Error),
    #[error("the workflow run {url} builds {built}, not {pushed}, the commit this deploy pushed: {branch} moved on GitHub since")]
    OtherCommit {
        url: String,
        built: String,
        pushed: String,
        branch: String,
    },
    #[error("the release workflow failed at {}", failed_at(.run, .jobs))]
    RunFailed { run: String, jobs: Vec<Job> },
    #[error("{what} failed: {status}")]
    Step { what: String, status: ExitStatus },
    #[error("{what}: {source}")]
    Answer {
        what: String,
        source: serde_json::Error,
    },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// The failing jobs' names and addresses, or the run's address when no job
/// says it failed, as when the run was cancelled before a job started.
fn failed_at(run: &str, jobs: &[Job]) -> String {
    if jobs.is_empty() {
        return run.to_owned();
    }
    jobs.iter()
        .map(|job| format!("{} ({})", job.name, job.html_url))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `manifest`, the workspace's `Cargo.toml`, with its workspace version
/// moved to the next patch, and that version.
fn bumped(manifest: &str) -> Result<(String, Version), Error> {
    let mut table = false;
    let mut lines: Vec<String> = Vec::new();
    let mut next = None;
    for line in manifest.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            table = trimmed == WORKSPACE_TABLE;
        }
        let current = (table && next.is_none())
            .then(|| trimmed.strip_prefix("version = \"")?.strip_suffix('"'))
            .flatten();
        let Some(current) = current else {
            lines.push(line.to_owned());
            continue;
        };
        let current = Version::parse(current).map_err(|error| Error::BadVersion(current.to_owned(), error))?;
        let version = Version {
            major: current.major,
            minor: current.minor,
            patch: current.patch + 1,
            pre: Prerelease::EMPTY,
            build: BuildMetadata::EMPTY,
        };
        lines.push(line.replacen(&current.to_string(), &version.to_string(), 1));
        next = Some(version);
    }
    let version = next.ok_or(Error::NoVersion)?;
    Ok((lines.concat(), version))
}

pub fn run(options: Options) -> Result<(), Error> {
    let host = options
        .host
        .filter(|host| !host.trim().is_empty())
        .ok_or(Error::NoHost)?;
    let mut stages = Stages::default();

    let branch = stages.time("Check the checkout", checkout)?;
    let (version, pushed) = stages.time("Commit and push the version", || {
        let manifest = crate::repository().join("Cargo.toml");
        let (bumped, version) = bumped(&std::fs::read_to_string(&manifest)?)?;
        std::fs::write(&manifest, bumped)?;
        // Cargo.lock names each workspace crate's version too.
        local(
            "updating Cargo.lock",
            Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
                .args(["update", "--workspace", "--offline"])
                .current_dir(crate::repository()),
        )?;
        git("staging the version", &["add", "Cargo.toml", "Cargo.lock"])?;
        git("committing the version", &["commit", "--quiet", "-m", &format!("chore: release {version}")])?;
        let pushed = git("reading HEAD", &["rev-parse", "HEAD"])?.trim().to_owned();
        git("pushing the version", &["push", "--quiet", "origin", &branch])?;
        Ok((version, pushed))
    })?;
    println!("Deploying {version}, {branch} at {pushed}, to {host}");

    let run = stages.time("Start the release workflow", || {
        let dispatched: Dispatched = gh_json(
            "starting the release workflow",
            &[
                "api",
                "--method",
                "POST",
                &format!("repos/{{owner}}/{{repo}}/actions/workflows/{WORKFLOW}/dispatches"),
                "-f",
                &format!("ref={branch}"),
                "-f",
                "inputs[prerelease]=true",
                "-F",
                "return_run_details=true",
            ],
        )?;
        let run: Run = gh_json(
            "reading the workflow run",
            &["api", &format!("repos/{{owner}}/{{repo}}/actions/runs/{}", dispatched.workflow_run_id)],
        )?;
        run.check(&pushed, &branch)?;
        Ok(run)
    })?;
    println!("The release workflow builds {version}: {}", run.html_url);

    stages.time("Build on GitHub", || {
        // Its exit status is not the verdict: the run's conclusion is,
        // which is read either way. A watch also ends when its connection
        // to GitHub breaks, while the run goes on, so it is watched again
        // until the run has ended.
        let ended = loop {
            let _: ExitStatus = Command::new("gh")
                .args(["run", "watch", &run.id.to_string(), "--compact", "--interval", WATCH_INTERVAL])
                .current_dir(crate::repository())
                .status()?;
            let read: Run = gh_json(
                "reading the workflow run",
                &["api", &format!("repos/{{owner}}/{{repo}}/actions/runs/{}", run.id)],
            )?;
            if read.status == "completed" {
                break read;
            }
        };
        if ended.conclusion.as_deref() == Some("success") {
            return Ok(());
        }
        let pages: Vec<Jobs> = gh_json(
            "reading the workflow run's jobs",
            &[
                "api",
                "--paginate",
                "--slurp",
                &format!("repos/{{owner}}/{{repo}}/actions/runs/{}/jobs", run.id),
            ],
        )?;
        Err(Error::RunFailed {
            run: run.html_url.clone(),
            jobs: failed(pages.into_iter().flat_map(|page| page.jobs)),
        })
    })?;

    let status = upgrade(&mut stages, &host, &version)?;

    println!();
    println!("Deployed {version} to {host}: {}", run.html_url);
    println!();
    stages.report();
    println!();
    println!("demi-server status on {host}:");
    print!("{status}");
    Ok(())
}

/// The checkout's branch, which must be clean and even with origin's.
fn checkout() -> Result<String, Error> {
    let changes = git("reading the checkout's status", &["status", "--porcelain"])?;
    if !changes.trim().is_empty() {
        return Err(Error::Uncommitted(changes.trim_end().to_owned()));
    }
    let output = Command::new("git")
        .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
        .current_dir(crate::repository())
        .output()?;
    if !output.status.success() {
        return Err(Error::NoBranch);
    }
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let head = git("reading HEAD", &["rev-parse", "HEAD"])?.trim().to_owned();
    let remote = git(
        "asking origin for the branch",
        &["ls-remote", "origin", &format!("refs/heads/{branch}")],
    )?;
    let remote = remote.split_whitespace().next().unwrap_or_default();
    if remote != head {
        return Err(Error::NotEven {
            head,
            branch,
            remote: if remote.is_empty() {
                "absent".to_owned()
            } else {
                remote.to_owned()
            },
        });
    }
    Ok(branch)
}

/// The answer to `POST .../dispatches` with `return_run_details`.
#[derive(serde::Deserialize)]
struct Dispatched {
    workflow_run_id: u64,
}

/// A workflow run, as GitHub's API describes it.
#[derive(Debug, serde::Deserialize)]
struct Run {
    id: u64,
    head_sha: String,
    html_url: String,
    status: String,
    conclusion: Option<String>,
}

impl Run {
    /// Whether the run builds `pushed`, the commit of the version the
    /// deploy pushed to `branch`.
    fn check(&self, pushed: &str, branch: &str) -> Result<(), Error> {
        if self.head_sha != pushed {
            return Err(Error::OtherCommit {
                url: self.html_url.clone(),
                built: self.head_sha.clone(),
                pushed: pushed.to_owned(),
                branch: branch.to_owned(),
            });
        }
        Ok(())
    }
}

/// A page of a run's jobs.
#[derive(serde::Deserialize)]
struct Jobs {
    jobs: Vec<Job>,
}

/// A job of a workflow run.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct Job {
    name: String,
    html_url: String,
    conclusion: Option<String>,
}

/// The jobs that failed, timed out or were cancelled, which ended the run;
/// a job skipped after them did not.
fn failed(jobs: impl IntoIterator<Item = Job>) -> Vec<Job> {
    jobs.into_iter()
        .filter(|job| matches!(job.conclusion.as_deref(), Some("failure" | "timed_out" | "cancelled")))
        .collect()
}

/// Moves the server to `version` and reads its status, in one SSH session;
/// returns the status.
fn upgrade(stages: &mut Stages, host: &str, version: &Version) -> Result<String, Error> {
    let script = format!(
        "set -eu
# root runs each step itself; another user through sudo, which fails rather
# than asks for a password.
as_root() {{ if [ \"$(id -u)\" -eq 0 ]; then \"$@\"; else sudo -n \"$@\"; fi; }}
[ -x {DEMI_SERVER} ] || {{ echo 'this machine has no {DEMI_SERVER}: a deploy moves a server that runs Demi already (installation.md)' >&2; exit 1; }}
echo '{STAGE_MARK}Upgrade the server'
as_root {DEMI_SERVER} upgrade {version}
echo '{STAGE_MARK}{STATUS}'
as_root {DEMI_SERVER} status
"
    );
    let mut session = Command::new("ssh")
        .arg(host)
        .args(["bash", "-s"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let mut input = session.stdin.take().expect("the session's input is piped");
    input.write_all(script.as_bytes())?;
    // Dropping the input ends it, so the script's shell ends with it.
    drop(input);
    let output = session.stdout.take().expect("the session's output is piped");
    let mut current: Option<(String, Instant)> = None;
    let mut status = String::new();
    for line in BufReader::new(output).lines() {
        let line = line?;
        if let Some(stage) = line.strip_prefix(STAGE_MARK) {
            if let Some((name, started)) = current.take() {
                stages.record(name, started.elapsed());
            }
            if stage != STATUS {
                println!("== {stage}");
            }
            current = Some((stage.to_owned(), Instant::now()));
        } else if current.as_ref().is_some_and(|(name, _)| name == STATUS) {
            status.push_str(&line);
            status.push('\n');
        } else {
            println!("{line}");
        }
    }
    let exit = session.wait()?;
    let ended = current.take();
    if !exit.success() {
        let what = match ended {
            Some((stage, _)) => format!("{stage} on {host}"),
            None => format!("the session on {host}"),
        };
        return Err(Error::Step { what, status: exit });
    }
    Ok(status)
}

/// Runs `command` and fails naming `what` when it fails.
fn local(what: &str, command: &mut Command) -> Result<(), Error> {
    let status = command.status()?;
    if !status.success() {
        return Err(Error::Step {
            what: what.to_owned(),
            status,
        });
    }
    Ok(())
}

/// Runs git in the repository and returns what it printed.
fn git(what: &str, arguments: &[&str]) -> Result<String, Error> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(crate::repository())
        .stderr(Stdio::inherit())
        .output()?;
    if !output.status.success() {
        return Err(Error::Step {
            what: what.to_owned(),
            status: output.status,
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Runs gh in the repository, whose origin names the GitHub repository,
/// and returns what it printed.
fn gh(what: &str, arguments: &[&str]) -> Result<Vec<u8>, Error> {
    let output = Command::new("gh")
        .args(arguments)
        .current_dir(crate::repository())
        .stderr(Stdio::inherit())
        .output()?;
    if !output.status.success() {
        return Err(Error::Step {
            what: what.to_owned(),
            status: output.status,
        });
    }
    Ok(output.stdout)
}

/// Runs gh as [`gh`] does and decodes the JSON it printed.
fn gh_json<T: serde::de::DeserializeOwned>(what: &str, arguments: &[&str]) -> Result<T, Error> {
    let output = gh(what, arguments)?;
    serde_json::from_slice(&output).map_err(|source| Error::Answer {
        what: format!("{what}: GitHub's answer"),
        source,
    })
}

/// How long each stage of the deploy took, in order.
#[derive(Default)]
struct Stages(Vec<(String, Duration)>);

impl Stages {
    fn time<T>(&mut self, stage: &str, work: impl FnOnce() -> Result<T, Error>) -> Result<T, Error> {
        println!("== {stage}");
        let started = Instant::now();
        let result = work();
        self.record(stage.to_owned(), started.elapsed());
        result
    }

    fn record(&mut self, stage: String, took: Duration) {
        self.0.push((stage, took));
    }

    fn report(&self) {
        let width = self.0.iter().map(|(stage, _)| stage.len()).max().unwrap_or(0);
        let mut total = Duration::ZERO;
        for (stage, took) in &self.0 {
            println!("  {stage:width$}  {}", duration(*took));
            total += *took;
        }
        println!("  {:width$}  {}", "Total", duration(total));
    }
}

/// `took` as minutes and seconds, or seconds to a tenth under a minute.
fn duration(took: Duration) -> String {
    let seconds = took.as_secs_f64();
    if seconds < 60.0 {
        format!("{seconds:.1} s")
    } else {
        format!("{} min {:02} s", took.as_secs() / 60, took.as_secs() % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deploy moves the workspace version, and only it, to the next
    /// patch, whatever other tables name a version.
    #[test]
    fn a_deploy_moves_the_workspace_version_to_the_next_patch() {
        let manifest = "[workspace]\nmembers = [\"crates/a\"]\n\n[workspace.package]\nedition = \"2024\"\nversion = \"0.1.21\"\nlicense = \"Apache-2.0\"\n\n[workspace.dependencies]\nsemver = { version = \"0.1.21\" }\nversion = \"0.1.21\"\n";
        let (written, version) = bumped(manifest).unwrap();
        assert_eq!(version, Version::new(0, 1, 22));
        assert_eq!(written, manifest.replacen("version = \"0.1.21\"\nlicense", "version = \"0.1.22\"\nlicense", 1));
        assert_eq!(bumped("[workspace.package]\nversion = \"0.2.9-rc.1\"\n").unwrap().1, Version::new(0, 2, 10));
        assert!(matches!(bumped("[package]\nversion = \"0.1.21\"\n"), Err(Error::NoVersion)));
        assert!(matches!(bumped("[workspace.package]\nversion = \"one\"\n"), Err(Error::BadVersion(..))));
    }

    /// The run a deploy started is followed only when it builds the commit
    /// the deploy pushed.
    #[test]
    fn the_run_is_followed_only_when_it_builds_the_pushed_commit() {
        let run = |sha: &str| -> Run {
            serde_json::from_value(serde_json::json!({
                "id": 7,
                "head_sha": sha,
                "display_title": "Release",
                "html_url": "https://github.com/wspl/demi/actions/runs/7",
                "status": "queued",
                "conclusion": null,
            }))
            .unwrap()
        };
        assert!(run("4f2c1e0a").check("4f2c1e0a", "main").is_ok());
        assert!(matches!(run("9a9a9a9a").check("4f2c1e0a", "main"), Err(Error::OtherCommit { .. })));
    }

    /// A failed run names the jobs that failed it, not the ones skipped
    /// after them, or the run itself when no job failed.
    #[test]
    fn a_failed_run_names_the_jobs_that_failed_it() {
        let page: Jobs = serde_json::from_value(serde_json::json!({ "total_count": 3, "jobs": [
            { "name": "Version", "html_url": "https://x/1", "conclusion": "success" },
            { "name": "runner x86_64-pc-windows-msvc", "html_url": "https://x/2", "conclusion": "failure" },
            { "name": "Publish", "html_url": "https://x/3", "conclusion": "skipped" },
        ]}))
        .unwrap();
        let error = Error::RunFailed {
            run: "https://x/run".to_owned(),
            jobs: failed(page.jobs),
        };
        assert_eq!(
            error.to_string(),
            "the release workflow failed at runner x86_64-pc-windows-msvc (https://x/2)"
        );
        let cancelled = Error::RunFailed {
            run: "https://x/run".to_owned(),
            jobs: Vec::new(),
        };
        assert_eq!(cancelled.to_string(), "the release workflow failed at https://x/run");
    }
}
