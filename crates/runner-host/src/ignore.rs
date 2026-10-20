//! What git ignores in a watched tree (`crates/runner-host/src/subscriptions.rs`): the
//! rules of the repository the tree lies in, with a cache of the folders it
//! ignores whole, so a path under `target/` costs one lookup of `target`.
//! A watch's thread asks it of each path it receives; an ignored path
//! changes no `worktree` subscription and is not among the working tree's
//! changed paths.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use gix::bstr::BString;

/// The most folders the cache keeps; past it, it starts over.
const CACHED_FOLDERS: usize = 65_536;

/// The rules of one repository and the folders already judged. It lives on
/// the one thread of its watch.
pub(crate) struct IgnoreFilter {
    repo: gix::Repository,
    /// Canonical, as the platform reports paths.
    workdir: PathBuf,
    git_dir: PathBuf,
    folders: Folders,
    /// The rules could not be read once: said once in the log.
    warned: bool,
}

/// The folders already judged.
#[derive(Default)]
struct Folders {
    /// Ignored with everything below them.
    ignored: HashSet<PathBuf>,
    /// Not ignored.
    kept: HashSet<PathBuf>,
}

impl Folders {
    fn clear(&mut self) {
        self.ignored.clear();
        self.kept.clear();
    }
}

/// The exclude rules as they stand, read once for the paths of one report.
type Excludes<'repo> = Option<gix::AttributeStack<'repo>>;

impl IgnoreFilter {
    /// The rules of the repository the canonical `path` lies in; none
    /// outside a repository with a work tree.
    pub(crate) fn of(path: &Path) -> Option<Self> {
        let repo = match gix::discover(path) {
            Ok(repo) => repo,
            Err(gix::discover::Error::Discover(_)) => return None,
            Err(error) => {
                // Without the rules nothing counts as ignored, as before they
                // existed: a page lists the changes once too often.
                tracing::warn!("the repository of a watch could not be opened: {error}");
                return None;
            }
        };
        let workdir = std::fs::canonicalize(repo.workdir()?).ok()?;
        let git_dir = std::fs::canonicalize(repo.git_dir()).ok()?;
        Some(Self {
            repo,
            workdir,
            git_dir,
            folders: Folders::default(),
            warned: false,
        })
    }

    /// Whether git ignores each of the canonical `paths`, each with whether
    /// it is a folder when the caller knows: untracked, and matched by an
    /// exclude rule of its own or of a folder above it. A path in the
    /// repository's git directory or outside its work tree is not ignored.
    /// A changed rules file among them makes the cache start over first.
    pub(crate) fn ignored(&mut self, paths: &[(&Path, Option<bool>)]) -> Vec<bool> {
        if paths.iter().any(|(path, _)| self.rules_file(path))
            || self.folders.ignored.len() + self.folders.kept.len() > CACHED_FOLDERS
        {
            self.folders.clear();
        }
        let index = match self.repo.index_or_empty() {
            Ok(index) => index,
            Err(error) => {
                self.warn(&error.to_string());
                return vec![false; paths.len()];
            }
        };
        let mut excludes: Excludes<'_> = None;
        let mut failure = None;
        let mut judged = Vec::with_capacity(paths.len());
        for (path, folder) in paths {
            let judge = Judge {
                repo: &self.repo,
                index: &index,
                workdir: &self.workdir,
                git_dir: &self.git_dir,
            };
            match judge.ignored(path, *folder, &mut self.folders, &mut excludes) {
                Ok(ignored) => judged.push(ignored),
                Err(error) => {
                    failure.get_or_insert(error);
                    judged.push(false);
                }
            }
        }
        if let Some(error) = failure {
            self.warn(&error);
        }
        judged
    }

    fn warn(&mut self, error: &str) {
        if !self.warned {
            self.warned = true;
            tracing::warn!("which changed paths git ignores could not be told: {error}");
        }
    }

    /// Up to `most` folders git ignores among the entries of `root` and of
    /// its folders that it does not, the shallowest first: the folders a
    /// tree's watch leaves out, such as `target` and `node_modules`.
    pub(crate) fn large_ignored(&mut self, root: &Path, most: usize) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let mut level = vec![root.to_path_buf()];
        for _ in 0..2 {
            let mut next = Vec::new();
            for folder in &level {
                let Ok(entries) = std::fs::read_dir(folder) else {
                    continue;
                };
                let folders: Vec<PathBuf> = entries
                    .filter_map(Result::ok)
                    .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                    .map(|entry| entry.path())
                    .filter(|path| {
                        !path.starts_with(&self.git_dir) && path.file_name() != Some(".git".as_ref())
                    })
                    .collect();
                let asked: Vec<(&Path, Option<bool>)> =
                    folders.iter().map(|path| (path.as_path(), Some(true))).collect();
                let judged = self.ignored(&asked);
                for (path, ignored) in folders.iter().zip(judged) {
                    if !ignored {
                        next.push(path.clone());
                        continue;
                    }
                    found.push(path.clone());
                    if found.len() == most {
                        return found;
                    }
                }
            }
            level = next;
        }
        found
    }

    /// Whether git reads `path` to decide what it ignores: a `.gitignore`,
    /// or the repository's `info/exclude` or `config`.
    fn rules_file(&self, path: &Path) -> bool {
        path.file_name() == Some(".gitignore".as_ref())
            || path == self.git_dir.join("info/exclude")
            || path == self.git_dir.join("config")
    }
}

/// What judging one path reads.
struct Judge<'a> {
    repo: &'a gix::Repository,
    index: &'a gix::worktree::Index,
    workdir: &'a Path,
    git_dir: &'a Path,
}

impl<'a> Judge<'a> {
    fn ignored(
        &self,
        path: &Path,
        folder: Option<bool>,
        folders: &mut Folders,
        excludes: &mut Excludes<'a>,
    ) -> Result<bool, String> {
        if path.starts_with(self.git_dir) {
            return Ok(false);
        }
        let Ok(relative) = path.strip_prefix(self.workdir) else {
            return Ok(false);
        };
        if relative.as_os_str().is_empty() {
            return Ok(false);
        }
        // A tracked file is never ignored, whatever the rules say.
        let rela = rela(relative);
        if self.index.entry_by_path(rela.as_ref()).is_some() {
            return Ok(false);
        }
        // The folders above it, the top first: one of them ignored is enough.
        let mut above = PathBuf::new();
        let parts: Vec<_> = relative.components().collect();
        for part in &parts[..parts.len() - 1] {
            above.push(part);
            if self.folder_ignored(&above, folders, excludes)? {
                return Ok(true);
            }
        }
        let folder = folder
            .unwrap_or_else(|| std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir()));
        if folder {
            return self.folder_ignored(relative, folders, excludes);
        }
        self.excluded(relative, None, excludes)
    }

    /// A folder is ignored whole when an exclude rule matches it and the
    /// index tracks nothing under it; each folder is judged once.
    fn folder_ignored(
        &self,
        relative: &Path,
        folders: &mut Folders,
        excludes: &mut Excludes<'a>,
    ) -> Result<bool, String> {
        let whole = self.workdir.join(relative);
        if folders.ignored.contains(&whole) {
            return Ok(true);
        }
        if folders.kept.contains(&whole) {
            return Ok(false);
        }
        let rela = rela(relative);
        let ignored = !self.index.path_is_directory(rela.as_ref())
            && self.excluded(relative, Some(gix::index::entry::Mode::DIR), excludes)?;
        if ignored {
            folders.ignored.insert(whole);
        } else {
            folders.kept.insert(whole);
        }
        Ok(ignored)
    }

    fn excluded(
        &self,
        relative: &Path,
        mode: Option<gix::index::entry::Mode>,
        excludes: &mut Excludes<'a>,
    ) -> Result<bool, String> {
        if excludes.is_none() {
            let stack = self
                .repo
                .excludes(
                    self.index,
                    None,
                    gix::worktree::stack::state::ignore::Source::WorktreeThenIdMappingIfNotSkipped,
                )
                .map_err(|error| error.to_string())?;
            *excludes = Some(stack);
        }
        let stack = excludes.as_mut().expect("the rules were just read");
        let platform = stack.at_path(relative, mode).map_err(|error| error.to_string())?;
        Ok(platform.is_excluded())
    }
}

fn rela(relative: &Path) -> BString {
    gix::path::to_unix_separators_on_windows(gix::path::into_bstr(relative)).into_owned()
}
