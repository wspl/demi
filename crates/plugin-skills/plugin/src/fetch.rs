//! A source's fetch and the check of its commit.
//!
//! - The fetch is a shallow fetch of the repository's default branch into a
//!   temporary directory. It stops as soon as it has received 64 MiB, and
//!   returns the skills of the commit it pins.
//! - The check finds the commit that branch points to now.
//!
//! The server's fetch and check run here, with gix, and block, so they run on
//! the blocking pool. A device's run there, with its own `git`, and answer a
//! tree whose skills [`from_device`] reads by the same rules.

use std::collections::BTreeSet;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use gix::bstr::ByteSlice;
use gix::progress::{Count, Id, MessageLevel, NestedProgress, Progress, Step, StepShared, Unit};

use crate::skill::{self, Parsed, SkipProblem};

/// The most bytes a fetch receives: they are held in memory while they are
/// unpacked, and a repository has no size of its own.
pub(crate) const FETCH_MAX_BYTES: usize = 64 * 1024 * 1024;

/// The most bytes the files of one source's skills hold together: they are held
/// in memory and written to the Host's disk.
pub(crate) const FILES_MAX_BYTES: usize = 16 * 1024 * 1024;

/// What gix-pack names the progress of the pack bytes it reads.
const READ_PACK_BYTES: Id = *b"BWRB";

/// The skills of a fetched commit.
#[derive(Debug)]
pub(crate) struct Fetched {
    pub(crate) commit: String,
    pub(crate) skills: Vec<FetchedSkill>,
    pub(crate) skipped: Vec<Skipped>,
}

/// A skill of the commit with its files' bytes.
#[derive(Debug)]
pub(crate) struct FetchedSkill {
    pub(crate) parsed: Parsed,
    /// Its directory in the repository, empty at the root.
    pub(crate) directory: String,
    pub(crate) files: Vec<FetchedFile>,
}

#[derive(Debug)]
pub(crate) struct FetchedFile {
    /// Relative to the skill's directory.
    pub(crate) path: String,
    pub(crate) executable: bool,
    pub(crate) bytes: Vec<u8>,
}

/// A `SKILL.md` that is not a skill.
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct Skipped {
    /// Its path in the repository.
    pub path: String,
    /// Why, in English.
    pub reason: String,
    /// Why, as the page words it; none in a value a fetch before codes
    /// recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<SkipProblem>,
}

/// Why a fetch gave no skills, as the page words it: one of Demi's limits or
/// finds, or a failure in git's or the system's words, which the failure's
/// message keeps.
#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(tag = "code", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum FetchProblem {
    /// The repository is larger than `limit_bytes`.
    RepositoryTooLarge { limit_bytes: u32 },
    NoCommit,
    /// Its skills' files hold more than `limit_bytes`.
    FilesTooLarge { limit_bytes: u32 },
    NoSkill,
    Failed,
}

/// Why a fetch gave no skills.
#[derive(Debug, thiserror::Error)]
pub(crate) enum FetchError {
    /// The instance ended, as with the backend's shutdown: nothing is
    /// recorded.
    #[error("the fetch was stopped")]
    Stopped,
    #[error("{message}")]
    Failed {
        message: String,
        problem: FetchProblem,
    },
}

impl FetchError {
    /// A failure in git's or the system's words.
    fn failed(error: impl std::fmt::Display) -> Self {
        Self::problem(FetchProblem::Failed, error.to_string())
    }

    fn problem(problem: FetchProblem, message: impl Into<String>) -> Self {
        Self::Failed {
            message: message.into(),
            problem,
        }
    }
}

/// Fetches `url` and reads the skills of its default branch's newest
/// commit; `stop` ends the fetch, as the instance's end does. `repository`
/// names a skill at the root whose front matter names none.
pub(crate) fn fetch(
    url: &str,
    repository: &str,
    stop: &Arc<AtomicBool>,
) -> Result<Fetched, FetchError> {
    let directory = tempfile::tempdir()
        .map_err(|error| FetchError::failed(format!("no temporary directory: {error}")))?;
    let progress = Bounded::new(stop.clone());
    let exceeded = progress.exceeded.clone();
    let failed = |error: &dyn std::fmt::Display| {
        if exceeded.load(Ordering::Relaxed) {
            FetchError::problem(
                FetchProblem::RepositoryTooLarge {
                    limit_bytes: FETCH_MAX_BYTES as u32,
                },
                "the repository is larger than 64 MiB",
            )
        } else if stop.load(Ordering::Relaxed) {
            FetchError::Stopped
        } else {
            FetchError::failed(error)
        }
    };
    // Isolated: no configuration of the machine's, so no credential either.
    let mut prepare = gix::clone::PrepareFetch::new(
        url,
        directory.path(),
        gix::create::Kind::Bare,
        gix::create::Options::default(),
        gix::open::Options::isolated(),
    )
    .map_err(|error| failed(&error))?
    .with_shallow(gix::remote::fetch::Shallow::DepthAtRemote(
        NonZeroU32::new(1).expect("one is not zero"),
    ));
    let (repository_on_disk, _) = prepare
        .fetch_only(progress, stop)
        .map_err(|error| failed(&error))?;
    let commit = repository_on_disk
        .head_commit()
        .map_err(|_| FetchError::problem(FetchProblem::NoCommit, "the repository holds no commit"))?;
    let tree = commit.tree().map_err(|error| failed(&error))?;
    let mut recorder = gix::traverse::tree::Recorder::default();
    tree.traverse()
        .breadthfirst(&mut recorder)
        .map_err(|error| failed(&error))?;
    // Each file by its path as the skills read it, which a name that is
    // not UTF-8 reaches in its lossy form: the same form `read` looks up.
    let objects: std::collections::HashMap<String, gix::ObjectId> = recorder
        .records
        .iter()
        .filter(|entry| entry.mode.is_blob())
        .map(|entry| (entry.filepath.to_str_lossy().into_owned(), entry.oid))
        .collect();
    let files: Vec<TreeFile> = recorder
        .records
        .iter()
        .filter(|entry| entry.mode.is_blob())
        .map(|entry| TreeFile {
            path: entry.filepath.to_str_lossy().into_owned(),
            executable: entry.mode.is_executable(),
        })
        .collect();
    let mut read = |path: &str| -> Result<Vec<u8>, FetchError> {
        let object = objects
            .get(path)
            .ok_or_else(|| FetchError::failed(format!("the tree lists no {path}")))?;
        Ok(repository_on_disk
            .find_object(*object)
            .map_err(FetchError::failed)?
            .detach()
            .data)
    };
    let fetched = skills_of(&files, &mut read, repository)?;
    Ok(Fetched {
        commit: commit.id.to_string(),
        skills: fetched.0,
        skipped: fetched.1,
    })
}

/// The commit `url`'s default branch points to now, read from the remote's
/// ref advertisement without fetching any object. It blocks, so it runs on
/// the blocking pool.
pub(crate) fn remote_head(url: &str) -> Result<String, String> {
    // A remote needs a repository; an empty one, isolated as a fetch's is.
    let directory =
        tempfile::tempdir().map_err(|error| format!("no temporary directory: {error}"))?;
    let repository = gix::ThreadSafeRepository::init_opts(
        directory.path(),
        gix::create::Kind::Bare,
        gix::create::Options::default(),
        gix::open::Options::isolated(),
    )
    .map_err(|error| error.to_string())?
    .to_thread_local();
    let remote = repository
        .remote_at(url)
        .map_err(|error| error.to_string())?
        .with_fetch_tags(gix::remote::fetch::Tags::None);
    let head = gix::refspec::parse("HEAD".into(), gix::refspec::parse::Operation::Fetch)
        .expect("HEAD is a refspec")
        .to_owned();
    let (refs, _) = remote
        .connect(gix::remote::Direction::Fetch)
        .map_err(|error| error.to_string())?
        .ref_map(
            gix::progress::Discard,
            gix::remote::ref_map::Options {
                extra_refspecs: vec![head],
                ..Default::default()
            },
        )
        .map_err(|error| error.to_string())?;
    refs.remote_refs
        .iter()
        .find_map(|advertised| {
            let (name, target, peeled) = advertised.unpack();
            (name == "HEAD").then(|| peeled.or(target)).flatten()
        })
        .map(|commit| commit.to_string())
        .ok_or_else(|| "the repository holds no commit".to_owned())
}

/// A file of a commit's tree.
pub(crate) struct TreeFile {
    /// In the repository, with `/` between its parts.
    pub(crate) path: String,
    pub(crate) executable: bool,
}

/// The skills of the commit `commit` that a device's `git` read, `files`
/// every file of its tree and `bytes` those of the skills' directories;
/// `repository` names a skill at the root whose front matter names none.
pub(crate) fn from_device(
    commit: String,
    files: Vec<TreeFile>,
    bytes: std::collections::BTreeMap<String, Vec<u8>>,
    repository: &str,
) -> Result<Fetched, FetchError> {
    let mut read = |path: &str| {
        bytes
            .get(path)
            .cloned()
            .ok_or_else(|| FetchError::failed(format!("the device read no {path}")))
    };
    let (skills, skipped) = skills_of(&files, &mut read, repository)?;
    Ok(Fetched { commit, skills, skipped })
}

/// The skills among `files`, the commit's whole tree, whose bytes `read`
/// gives, and the `SKILL.md` files that are not skills.
fn skills_of(
    files: &[TreeFile],
    read: &mut dyn FnMut(&str) -> Result<Vec<u8>, FetchError>,
    root_name: &str,
) -> Result<(Vec<FetchedSkill>, Vec<Skipped>), FetchError> {
    let directories: BTreeSet<String> = files
        .iter()
        .map(|file| file.path.clone())
        .filter_map(|path| match path.rsplit_once('/') {
            Some((directory, "SKILL.md")) => Some(directory.to_owned()),
            None if path == "SKILL.md" => Some(String::new()),
            _ => None,
        })
        .collect();
    let mut skills = Vec::new();
    let mut skipped = Vec::new();
    let mut bytes = 0;
    for directory in &directories {
        let manifest = if directory.is_empty() {
            "SKILL.md".to_owned()
        } else {
            format!("{directory}/SKILL.md")
        };
        let name = directory.rsplit('/').next().filter(|name| !name.is_empty());
        let text = read(&manifest)?;
        let parsed = match skill::parse(name.unwrap_or(root_name), &text) {
            Ok(parsed) => parsed,
            Err(not) => {
                skipped.push(Skipped {
                    path: manifest,
                    reason: not.reason,
                    problem: Some(not.problem),
                });
                continue;
            }
        };
        let mut skill_files = Vec::new();
        for file in files {
            let path = &file.path;
            let Some(relative) = within(directory, path) else {
                continue;
            };
            // A directory below that is a skill of its own keeps its files.
            let nested = directories.iter().any(|other| {
                other != directory
                    && within(directory, other).is_some()
                    && within(other, path).is_some()
            });
            if nested {
                continue;
            }
            let content = read(path)?;
            bytes += content.len();
            if bytes > FILES_MAX_BYTES {
                return Err(FetchError::problem(
                    FetchProblem::FilesTooLarge {
                        limit_bytes: FILES_MAX_BYTES as u32,
                    },
                    "the skills' files hold more than 16 MiB",
                ));
            }
            skill_files.push(FetchedFile {
                path: relative.to_owned(),
                executable: file.executable,
                bytes: content,
            });
        }
        skills.push(FetchedSkill {
            parsed,
            directory: directory.clone(),
            files: skill_files,
        });
    }
    if skills.is_empty() {
        return Err(FetchError::problem(FetchProblem::NoSkill, "the repository holds no skill"));
    }
    Ok((skills, skipped))
}

/// `path` relative to `directory`, if it is below it; every path is below
/// the root, the empty directory.
fn within<'a>(directory: &str, path: &'a str) -> Option<&'a str> {
    if directory.is_empty() {
        return Some(path);
    }
    path.strip_prefix(directory)?.strip_prefix('/')
}

/// The fetch's progress, which counts the pack bytes it reads and stops the
/// fetch once they pass [`FETCH_MAX_BYTES`]; it shows nothing.
#[derive(Clone)]
struct Bounded {
    id: Id,
    received: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    exceeded: Arc<AtomicBool>,
}

impl Bounded {
    fn new(stop: Arc<AtomicBool>) -> Self {
        Self {
            id: gix::progress::UNKNOWN,
            received: Arc::default(),
            stop,
            exceeded: Arc::default(),
        }
    }

    fn child(&self, id: Id) -> Self {
        Self { id, ..self.clone() }
    }
}

impl Count for Bounded {
    fn set(&self, _: Step) {}

    fn step(&self) -> Step {
        0
    }

    fn inc_by(&self, step: Step) {
        if self.id != READ_PACK_BYTES {
            return;
        }
        let received = self.received.fetch_add(step, Ordering::Relaxed) + step;
        if received > FETCH_MAX_BYTES {
            self.exceeded.store(true, Ordering::Relaxed);
            self.stop.store(true, Ordering::Relaxed);
        }
    }

    fn counter(&self) -> StepShared {
        // gix-pack counts the pack's bytes through `inc_by`; this counter
        // shows nothing.
        StepShared::default()
    }
}

impl Progress for Bounded {
    fn init(&mut self, _: Option<Step>, _: Option<Unit>) {}

    fn set_name(&mut self, _: String) {}

    fn name(&self) -> Option<String> {
        None
    }

    fn id(&self) -> Id {
        self.id
    }

    fn message(&self, _: MessageLevel, _: String) {}
}

impl NestedProgress for Bounded {
    type SubProgress = Bounded;

    fn add_child(&mut self, _: impl Into<String>) -> Self::SubProgress {
        self.child(gix::progress::UNKNOWN)
    }

    fn add_child_with_id(&mut self, _: impl Into<String>, id: Id) -> Self::SubProgress {
        self.child(id)
    }
}
