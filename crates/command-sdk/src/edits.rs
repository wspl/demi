//! Bounded edit snapshots shared by a runner and its native command services.
//!
//! One OS lock covers capture, mutation and publication. It is never held while
//! waiting for command input or while running another command.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read},
    path::{Path, PathBuf},
};

use demi_shared_artifacts::{Mode, Permissions, Publication};

use demi_command_protocol::{
    EDIT_FILE_BYTES, EDIT_JOB_BYTES, EDIT_JOB_FILES, EDIT_JOB_SEGMENTS, EditContext, EditCopies,
    EditFile, EditJournal, EditKind,
};

#[derive(Clone)]
pub struct Recorder {
    context: EditContext,
}

impl Recorder {
    pub fn new(context: EditContext) -> io::Result<Self> {
        context.validate().map_err(io::Error::other)?;
        fs::create_dir_all(&context.directory)?;
        Ok(Self { context })
    }

    pub fn context(&self) -> &EditContext {
        &self.context
    }

    /// Failure to record must not prevent the caller's filesystem operation.
    pub fn begin(&self) -> Option<Recording> {
        match self.locked() {
            Ok(recording) => Some(recording),
            Err(error) => {
                diagnostic(&error);
                None
            }
        }
    }

    pub fn record<T>(&self, path: &Path, operation: impl FnOnce() -> T) -> T {
        // Opening a pipe can block until another command opens its other end.
        if fs::metadata(path).is_ok_and(|metadata| !metadata.is_file()) {
            return operation();
        }
        let mut recording = self.begin();
        if let Some(recording) = &mut recording {
            recording.track(path);
        }
        let result = operation();
        drop(recording);
        result
    }

    /// Records the rename of `from` to `to` that runs while the returned
    /// guard lives. A file at `to` is replaced: an edit of it with both
    /// sides, as any whole-file write. Otherwise the rename moves `from`,
    /// which carries what this job recorded under the old name, or within
    /// it, to the new name once it happened; a move of a file the job did not
    /// change records nothing (`edit-tracking.md` § Scope).
    pub fn rename(&self, from: &Path, to: &Path) -> Renaming {
        let to = normalize(to);
        let replacing = if fs::metadata(&to).is_ok_and(|metadata| metadata.is_file()) {
            self.begin().map(|mut recording| {
                recording.track(&to);
                recording
            })
        } else {
            None
        };
        Renaming {
            recorder: self.clone(),
            from: normalize(from),
            to,
            replacing,
        }
    }

    /// Called after all writers have stopped; contents come only from snapshots.
    pub fn report(&self) -> io::Result<EditJournal> {
        let mut recording = self.locked()?;
        recording.journal.files.retain(|file| {
            !file.edits.is_empty()
                && match fs::metadata(&file.path) {
                    Ok(metadata) => metadata.is_file(),
                    Err(error) => !absent(&error),
                }
        });
        Ok(recording.journal.clone())
    }

    fn locked(&self) -> io::Result<Recording> {
        // Out of open files, recording waits for one rather than leave an
        // edit out (`runner.md` § Load).
        let mut options = OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        let lock = crate::descriptors::retry_blocking(|| options.open(&self.context.lock))?;
        lock.lock()?;
        let directory = PathBuf::from(&self.context.directory);
        let journal_path = directory.join("journal.json");
        let journal = match crate::descriptors::retry_blocking(|| fs::read(&journal_path)) {
            Ok(bytes) => {
                let journal: EditJournal =
                    serde_json::from_slice(&bytes).map_err(io::Error::other)?;
                journal.validate().map_err(io::Error::other)?;
                for file in &journal.files {
                    for edit in &file.edits {
                        if edit.original.is_some() && edit.modified.is_none() {
                            return Err(io::Error::other("original snapshot has no modified side"));
                        }
                        for path in edit.original.iter().chain(edit.modified.iter()) {
                            let path = Path::new(path);
                            if path.parent() != Some(directory.as_path()) {
                                return Err(io::Error::other(
                                    "snapshot is outside its job directory",
                                ));
                            }
                        }
                    }
                }
                journal
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => EditJournal {
                files: Vec::new(),
                bytes_copied: 0,
                next_segment: 0,
                files_truncated: false,
            },
            Err(error) => return Err(error),
        };
        Ok(Recording {
            _lock: lock,
            directory,
            journal,
            before: Vec::new(),
        })
    }
}

/// Dropping the guard captures partial changes on errors and unwinding as well.
pub struct Recording {
    _lock: File,
    directory: PathBuf,
    journal: EditJournal,
    before: Vec<(PathBuf, Contents)>,
}

impl Recording {
    /// Track before touching the destination, including a backup rename.
    pub fn track(&mut self, path: &Path) {
        let path = normalize(path);
        if self.before.iter().any(|(existing, _)| existing == &path) {
            return;
        }
        if self.before.len() >= EDIT_JOB_FILES {
            self.journal.files_truncated = true;
            return;
        }
        let memory: u64 = self.before.iter().map(|(_, contents)| contents.len()).sum();
        let contents = if memory + self.journal.bytes_copied >= EDIT_JOB_BYTES {
            match fs::metadata(&path) {
                Err(error) if absent(&error) => Contents::Missing,
                Ok(metadata) if !metadata.is_file() => Contents::NotFile,
                Ok(metadata) => Contents::Unavailable(FileStamp::read(&metadata)),
                Err(_) => Contents::Unavailable(None),
            }
        } else {
            Contents::read(&path)
        };
        if !matches!(contents, Contents::NotFile) {
            self.before.push((path, contents));
        }
    }

    /// A transactional writer has restored this path to its exact prior bytes.
    pub fn restored(&mut self, path: &Path) {
        let path = normalize(path);
        self.before.retain(|(existing, _)| existing != &path);
    }

    /// Moves the entries of `from` and of the paths within it to the same
    /// names under `to`, after the rename that moved them. An entry the new
    /// name already has, from a file there that the job changed and that
    /// went, keeps its kind and takes the moved segments after its own; any
    /// other is `added`, as nothing was at the new name before the move.
    fn carry(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        let mut moved = false;
        let mut index = 0;
        while index < self.journal.files.len() {
            let Ok(within) = Path::new(&self.journal.files[index].path).strip_prefix(from) else {
                index += 1;
                continue;
            };
            // `join` of an empty path would add a trailing separator.
            let path = if within.as_os_str().is_empty() {
                to.to_owned()
            } else {
                to.join(within)
            };
            let path = path.to_string_lossy().into_owned();
            moved = true;
            match self.journal.files.iter().position(|file| file.path == path) {
                Some(existing) => {
                    let edits = self.journal.files.remove(index).edits;
                    let existing = existing - usize::from(existing > index);
                    self.journal.files[existing].edits.extend(edits);
                }
                None => {
                    let file = &mut self.journal.files[index];
                    file.path = path;
                    file.kind = EditKind::Added;
                    index += 1;
                }
            }
        }
        if moved {
            self.write_journal()?;
        }
        Ok(())
    }

    fn write_journal(&self) -> io::Result<()> {
        let bytes = serde_json::to_vec(&self.journal).map_err(io::Error::other)?;
        publish_file(&self.directory.join("journal.json"), &bytes)
    }

    fn publish(&mut self, path: &Path, before: Contents, after: Contents) -> io::Result<()> {
        if before.same(&after) || matches!(after, Contents::NotFile) {
            return Ok(());
        }
        let path = path.to_string_lossy().into_owned();
        let index = match self.journal.files.iter().position(|file| file.path == path) {
            Some(index) => index,
            // Deletions have no independent entry. A containing transaction can
            // still restore the file before this guard captures its final side.
            None if matches!(after, Contents::Missing) => return Ok(()),
            None if self.journal.files.len() >= EDIT_JOB_FILES => {
                self.journal.files_truncated = true;
                return Ok(());
            }
            None => {
                self.journal.files.push(EditFile {
                    path,
                    kind: if matches!(before, Contents::Missing) {
                        EditKind::Added
                    } else {
                        EditKind::Modified
                    },
                    edits: Vec::new(),
                });
                self.journal.files.len() - 1
            }
        };
        let previous = self.journal.files[index].edits.last().cloned();
        if previous
            .as_ref()
            .is_some_and(|edit| edit.modified.is_none())
        {
            // Once continuity is unknown, keep metadata instead of a stale diff.
            return Ok(());
        }
        let merge = previous.as_ref().is_some_and(|edit| {
            edit.modified
                .as_ref()
                .is_some_and(|path| before.same(&Contents::read(Path::new(path))))
        });
        let original = if merge {
            previous
                .as_ref()
                .and_then(|edit| edit.original.as_deref())
                .map_or(Contents::Missing, |path| Contents::read(Path::new(path)))
        } else {
            before
        };
        if merge && original.same(&after) {
            self.journal.files[index].edits.pop();
            return Ok(());
        }
        if matches!(after, Contents::Missing) {
            // The final report omits absent paths. Re-creation cannot reuse the
            // previous after side, so keep no misleading continuous segment.
            self.journal.files[index].edits.clear();
            return Ok(());
        }
        let copy_bytes = if merge { 0 } else { original.len() } + after.len();
        if !original.text_or_missing()
            || !after.text_or_missing()
            || self.journal.bytes_copied + copy_bytes > EDIT_JOB_BYTES
        {
            self.unavailable(index);
            return Ok(());
        }
        if !merge && self.journal.next_segment >= EDIT_JOB_SEGMENTS {
            self.journal.files_truncated = true;
            self.unavailable(index);
            return Ok(());
        }
        let copies = if merge {
            previous.expect("a merged segment has a predecessor")
        } else {
            let segment = self.journal.next_segment;
            self.journal.next_segment += 1;
            EditCopies {
                original: (!matches!(original, Contents::Missing)).then(|| {
                    self.directory
                        .join(format!("{segment}.original"))
                        .to_string_lossy()
                        .into_owned()
                }),
                modified: Some(
                    self.directory
                        .join(format!("{segment}.modified"))
                        .to_string_lossy()
                        .into_owned(),
                ),
            }
        };
        if !merge && let (Some(path), Contents::Bytes(bytes, _)) = (&copies.original, &original) {
            publish_file(Path::new(path), bytes)?;
            self.journal.bytes_copied += bytes.len() as u64;
        }
        if let (Some(path), Contents::Bytes(bytes, _)) = (&copies.modified, &after) {
            publish_file(Path::new(path), bytes)?;
            self.journal.bytes_copied += bytes.len() as u64;
        }
        if merge {
            *self.journal.files[index]
                .edits
                .last_mut()
                .expect("merged segment") = copies;
        } else {
            self.journal.files[index].edits.push(copies);
        }
        Ok(())
    }

    fn unavailable(&mut self, index: usize) {
        self.journal.files[index].edits = vec![EditCopies {
            original: None,
            modified: None,
        }];
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        if self.before.is_empty() {
            return;
        }
        for (path, before) in std::mem::take(&mut self.before) {
            let after = Contents::read(&path);
            if let Err(error) = self.publish(&path, before, after) {
                diagnostic(&error);
                if let Some(index) = self
                    .journal
                    .files
                    .iter()
                    .position(|file| Path::new(&file.path) == path)
                {
                    self.unavailable(index);
                }
            }
        }
        if let Err(error) = self.write_journal() {
            diagnostic(&error);
        }
    }
}

/// The guard of [`Recorder::rename`]. A replacement holds the lock across
/// the rename, as a temporary file's publication does; a move takes it only
/// afterwards, so moving a large folder across devices blocks no other
/// job's writes.
pub struct Renaming {
    recorder: Recorder,
    from: PathBuf,
    to: PathBuf,
    replacing: Option<Recording>,
}

impl Drop for Renaming {
    fn drop(&mut self) {
        if self.replacing.take().is_some() {
            // Its own drop has recorded the replaced file.
            return;
        }
        // A rename that failed left the old name in place.
        let happened = fs::symlink_metadata(&self.from).is_err_and(|error| absent(&error))
            && fs::symlink_metadata(&self.to).is_ok();
        if !happened {
            return;
        }
        if let Some(mut recording) = self.recorder.begin()
            && let Err(error) = recording.carry(&self.from, &self.to)
        {
            diagnostic(&error);
        }
    }
}

enum Contents {
    Missing,
    Bytes(Vec<u8>, Option<FileStamp>),
    Unavailable(Option<FileStamp>),
    NotFile,
}

#[derive(PartialEq)]
struct FileStamp {
    length: u64,
    modified: std::time::SystemTime,
    created: Option<std::time::SystemTime>,
}

impl FileStamp {
    fn read(metadata: &fs::Metadata) -> Option<Self> {
        Some(Self {
            length: metadata.len(),
            modified: metadata.modified().ok()?,
            created: metadata.created().ok(),
        })
    }
}

/// Whether a lookup failed because nothing is at the path: a path below a
/// file names nothing either (`edit-tracking.md` § Recording actual writes).
fn absent(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
    )
}

impl Contents {
    fn read(path: &Path) -> Self {
        let stamp = match fs::metadata(path) {
            Ok(metadata) if !metadata.is_file() => return Self::NotFile,
            Ok(metadata) if metadata.len() > EDIT_FILE_BYTES as u64 => {
                return Self::Unavailable(FileStamp::read(&metadata));
            }
            Err(error) if absent(&error) => return Self::Missing,
            Err(_) => return Self::Unavailable(None),
            Ok(metadata) => FileStamp::read(&metadata),
        };
        let bytes = crate::descriptors::retry_blocking(|| File::open(path)).and_then(|file| {
            let mut bytes = Vec::new();
            file.take((EDIT_FILE_BYTES + 1) as u64)
                .read_to_end(&mut bytes)?;
            Ok(bytes)
        });
        match bytes {
            Ok(bytes) if bytes.len() <= EDIT_FILE_BYTES => Self::Bytes(bytes, stamp),
            _ => Self::Unavailable(stamp),
        }
    }

    fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Missing, Self::Missing) => true,
            (Self::Bytes(left, _), Self::Bytes(right, _)) => left == right,
            // Failed opens and other no-ops must not invent edits when contents
            // exceed the snapshot budget. Unknown metadata proves nothing.
            (Self::Unavailable(Some(left)), Self::Unavailable(Some(right)))
            | (Self::Unavailable(Some(left)), Self::Bytes(_, Some(right)))
            | (Self::Bytes(_, Some(left)), Self::Unavailable(Some(right))) => left == right,
            _ => false,
        }
    }

    fn text_or_missing(&self) -> bool {
        match self {
            Self::Missing => true,
            Self::Bytes(bytes, _) => demi_command_protocol::is_text(bytes),
            _ => false,
        }
    }

    fn len(&self) -> u64 {
        match self {
            Self::Bytes(bytes, _) => bytes.len() as u64,
            _ => 0,
        }
    }
}

/// Publishes a snapshot or the journal at `path` through `artifact`'s
/// atomic publication, readable by its owner alone as the files it copies
/// may not be. Nothing needs it to survive a crash, so it is not synced.
fn publish_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let publication = Publication {
        mode: Mode::Replace,
        permissions: Permissions::Private,
        durable: false,
    };
    crate::descriptors::retry_blocking(|| {
        demi_shared_artifacts::publish_bytes_blocking(path, bytes, publication).map_err(|error| {
            match error {
                demi_shared_artifacts::Error::Io(error) => error,
                // A publication of bytes fails only in its file operations.
                other => io::Error::other(other),
            }
        })
    })
}

fn normalize(path: &Path) -> PathBuf {
    // Preserve `..`: collapsing it lexically changes paths through symlinks.
    std::path::absolute(path).unwrap_or_else(|_| path.to_owned())
}

/// Inside the runner the event reaches the Host log; inside a command service
/// it reaches standard error, which the runner drains into the same log
/// (`edit-tracking.md`).
fn diagnostic(error: &io::Error) {
    tracing::warn!("edit recording failed: {error}");
}
