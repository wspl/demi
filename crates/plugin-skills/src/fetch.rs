//! A source's fetch (`skills.md` § User skills): a shallow fetch of the
//! repository's default branch into a temporary directory, which stops as
//! soon as it has received 64 MiB, and the skills of the commit it pins;
//! and the check of the commit that branch points to now. Both block, so
//! they run on the blocking pool.

use std::collections::BTreeSet;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use gix::bstr::ByteSlice;
use gix::progress::{Count, Id, MessageLevel, NestedProgress, Progress, Step, StepShared, Unit};

use crate::skill::{self, Parsed};

/// The most bytes a fetch receives.
const FETCH_MAX_BYTES: usize = 64 * 1024 * 1024;

/// The most skills one source has.
pub(crate) const SOURCE_MAX_SKILLS: usize = 100;

/// The most bytes the files of one source's skills hold together.
const FILES_MAX_BYTES: usize = 16 * 1024 * 1024;

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
    pub reason: String,
}

/// Why a fetch gave no skills.
#[derive(Debug, thiserror::Error)]
pub(crate) enum FetchError {
    /// The instance ended, as with the backend's shutdown: nothing is
    /// recorded.
    #[error("the fetch was stopped")]
    Stopped,
    #[error("{0}")]
    Failed(String),
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
        .map_err(|error| FetchError::Failed(format!("no temporary directory: {error}")))?;
    let progress = Bounded::new(stop.clone());
    let exceeded = progress.exceeded.clone();
    let failed = |error: &dyn std::fmt::Display| {
        if exceeded.load(Ordering::Relaxed) {
            FetchError::Failed("the repository is larger than 64 MiB".into())
        } else if stop.load(Ordering::Relaxed) {
            FetchError::Stopped
        } else {
            FetchError::Failed(error.to_string())
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
        .map_err(|_| FetchError::Failed("the repository holds no commit".into()))?;
    let tree = commit.tree().map_err(|error| failed(&error))?;
    let mut recorder = gix::traverse::tree::Recorder::default();
    tree.traverse()
        .breadthfirst(&mut recorder)
        .map_err(|error| failed(&error))?;
    let fetched = skills_of(&repository_on_disk, &recorder.records, repository)?;
    Ok(Fetched {
        commit: commit.id.to_string(),
        skills: fetched.0,
        skipped: fetched.1,
    })
}

/// The commit `url`'s default branch points to now, read from the remote's
/// ref advertisement without fetching any object (`skills.md` § Updates
/// available). It blocks, so it runs on the blocking pool.
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

/// The skills among `entries`, the commit's whole tree, and the `SKILL.md`
/// files that are not skills.
fn skills_of(
    repository: &gix::Repository,
    entries: &[gix::traverse::tree::recorder::Entry],
    root_name: &str,
) -> Result<(Vec<FetchedSkill>, Vec<Skipped>), FetchError> {
    let path_of =
        |entry: &gix::traverse::tree::recorder::Entry| entry.filepath.to_str_lossy().into_owned();
    let directories: BTreeSet<String> = entries
        .iter()
        .filter(|entry| entry.mode.is_blob())
        .map(path_of)
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
        let text = read_blob(repository, entries, &manifest)?;
        let parsed = match skill::parse(name.unwrap_or(root_name), &text) {
            Ok(parsed) => parsed,
            Err(reason) => {
                skipped.push(Skipped {
                    path: manifest,
                    reason,
                });
                continue;
            }
        };
        if skills.len() == SOURCE_MAX_SKILLS {
            return Err(FetchError::Failed(
                "the repository has more than 100 skills".into(),
            ));
        }
        let mut files = Vec::new();
        for entry in entries.iter().filter(|entry| entry.mode.is_blob()) {
            let path = path_of(entry);
            let Some(relative) = within(directory, &path) else {
                continue;
            };
            // A directory below that is a skill of its own keeps its files.
            let nested = directories.iter().any(|other| {
                other != directory
                    && within(directory, other).is_some()
                    && within(other, &path).is_some()
            });
            if nested {
                continue;
            }
            let content = repository
                .find_object(entry.oid)
                .map_err(|error| FetchError::Failed(error.to_string()))?
                .detach()
                .data;
            bytes += content.len();
            if bytes > FILES_MAX_BYTES {
                return Err(FetchError::Failed(
                    "the skills' files hold more than 16 MiB".into(),
                ));
            }
            files.push(FetchedFile {
                path: relative.to_owned(),
                executable: entry.mode.is_executable(),
                bytes: content,
            });
        }
        skills.push(FetchedSkill {
            parsed,
            directory: directory.clone(),
            files,
        });
    }
    if skills.is_empty() {
        return Err(FetchError::Failed("the repository holds no skill".into()));
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

/// The bytes of the blob at `path`.
fn read_blob(
    repository: &gix::Repository,
    entries: &[gix::traverse::tree::recorder::Entry],
    path: &str,
) -> Result<Vec<u8>, FetchError> {
    let entry = entries
        .iter()
        .find(|entry| entry.filepath == path)
        .expect("the path was listed");
    Ok(repository
        .find_object(entry.oid)
        .map_err(|error| FetchError::Failed(error.to_string()))?
        .detach()
        .data)
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
