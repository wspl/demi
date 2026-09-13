//! Bounded edit snapshots shared by a runner and its native command services.
//!
//! One OS lock covers capture, mutation and publication. It is never held while
//! waiting for command input or while running another command.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use crate::protocol::{
    EDIT_FILE_BYTES, EDIT_JOB_BYTES, EDIT_JOB_FILES, EDIT_JOB_SEGMENTS, EditContext, EditCopies,
    EditFile, EditJournal,
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

    /// Called after all writers have stopped; contents come only from snapshots.
    pub fn report(&self) -> io::Result<EditJournal> {
        let mut recording = self.locked()?;
        recording.journal.files.retain(|file| {
            !file.edits.is_empty()
                && match fs::metadata(&file.path) {
                    Ok(metadata) => metadata.is_file(),
                    Err(error) => error.kind() != io::ErrorKind::NotFound,
                }
        });
        Ok(recording.journal.clone())
    }

    fn locked(&self) -> io::Result<Recording> {
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&self.context.lock)?;
        lock.lock()?;
        let directory = PathBuf::from(&self.context.directory);
        let journal = match fs::read(directory.join("journal.json")) {
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
                Err(error) if error.kind() == io::ErrorKind::NotFound => Contents::Missing,
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
                        "added".into()
                    } else {
                        "modified".into()
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
        let result = serde_json::to_vec(&self.journal)
            .map_err(io::Error::other)
            .and_then(|bytes| publish_file(&self.directory.join("journal.json"), &bytes));
        if let Err(error) = result {
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

impl Contents {
    fn read(path: &Path) -> Self {
        let stamp = match fs::metadata(path) {
            Ok(metadata) if !metadata.is_file() => return Self::NotFile,
            Ok(metadata) if metadata.len() > EDIT_FILE_BYTES as u64 => {
                return Self::Unavailable(FileStamp::read(&metadata));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Self::Missing,
            Err(_) => return Self::Unavailable(None),
            Ok(metadata) => FileStamp::read(&metadata),
        };
        let bytes = File::open(path).and_then(|file| {
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
            Self::Bytes(bytes, _) => !bytes.contains(&0) && std::str::from_utf8(bytes).is_ok(),
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

fn publish_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::other("snapshot has no parent"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    temporary.write_all(bytes)?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

fn normalize(path: &Path) -> PathBuf {
    // Preserve `..`: collapsing it lexically changes paths through symlinks.
    std::path::absolute(path).unwrap_or_else(|_| path.to_owned())
}

fn diagnostic(error: &io::Error) {
    eprintln!("edit recording failed: {error}");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recorder(root: &Path, job: &str) -> Recorder {
        Recorder::new(EditContext {
            directory: root.join(job).to_string_lossy().into_owned(),
            lock: root.join("edits.lock").to_string_lossy().into_owned(),
        })
        .unwrap()
    }

    fn contents(edit: &EditCopies) -> (String, String) {
        (
            edit.original
                .as_ref()
                .map(|path| fs::read_to_string(path).unwrap())
                .unwrap_or_default(),
            fs::read_to_string(edit.modified.as_ref().unwrap()).unwrap(),
        )
    }

    #[test]
    fn interleaved_jobs_keep_their_own_segments() {
        let root = tempfile::tempdir().unwrap();
        let a = recorder(root.path(), "a");
        let b = recorder(root.path(), "b");
        let path = root.path().join("file");
        fs::write(&path, "x=0\ny=0\n").unwrap();
        a.record(&path, || fs::write(&path, "x=1\ny=0\n")).unwrap();
        b.record(&path, || fs::write(&path, "x=1\ny=1\n")).unwrap();
        a.record(&path, || fs::write(&path, "x=2\ny=1\n")).unwrap();
        let report = a.report().unwrap();
        let edits = &report.files[0].edits;
        assert_eq!(edits.len(), 2);
        assert_eq!(
            contents(&edits[0]),
            ("x=0\ny=0\n".into(), "x=1\ny=0\n".into())
        );
        assert_eq!(
            contents(&edits[1]),
            ("x=1\ny=1\n".into(), "x=2\ny=1\n".into())
        );
    }

    #[test]
    fn separate_recorder_handles_share_the_job_journal() {
        let root = tempfile::tempdir().unwrap();
        let first = recorder(root.path(), "job");
        let native = recorder(root.path(), "job");
        let path = root.path().join("file");
        first.record(&path, || fs::write(&path, "one")).unwrap();
        native.record(&path, || fs::write(&path, "two")).unwrap();
        let report = first.report().unwrap();
        assert_eq!(report.files[0].kind, "added");
        assert_eq!(report.files[0].edits.len(), 1);
        assert_eq!(
            contents(&report.files[0].edits[0]),
            ("".into(), "two".into())
        );
    }

    #[test]
    fn restored_and_removed_files_are_omitted() {
        let root = tempfile::tempdir().unwrap();
        let recorder = recorder(root.path(), "job");
        let path = root.path().join("file");
        fs::write(&path, "before").unwrap();
        recorder
            .record(&path, || fs::write(&path, "after"))
            .unwrap();
        recorder
            .record(&path, || fs::write(&path, "before"))
            .unwrap();
        assert!(recorder.report().unwrap().files.is_empty());
        recorder
            .record(&path, || fs::write(&path, "again"))
            .unwrap();
        fs::remove_file(&path).unwrap();
        assert!(recorder.report().unwrap().files.is_empty());
    }

    #[test]
    fn an_error_can_leave_a_real_edit() {
        let root = tempfile::tempdir().unwrap();
        let recorder = recorder(root.path(), "job");
        let path = root.path().join("file");
        let result: io::Result<()> = recorder.record(&path, || {
            fs::write(&path, "partial")?;
            Err(io::Error::other("failed after writing"))
        });
        assert!(result.is_err());
        assert_eq!(
            contents(&recorder.report().unwrap().files[0].edits[0]).1,
            "partial"
        );
    }

    #[test]
    fn failed_open_of_large_file_is_not_an_edit() {
        let root = tempfile::tempdir().unwrap();
        let recorder = recorder(root.path(), "job");
        let path = root.path().join("large");
        File::create(&path)
            .unwrap()
            .set_len(EDIT_FILE_BYTES as u64 + 1)
            .unwrap();
        let result = recorder.record(&path, || File::create_new(&path));
        assert!(result.is_err());
        assert!(recorder.report().unwrap().files.is_empty());
        recorder
            .record(&path, || {
                OpenOptions::new()
                    .append(true)
                    .open(&path)
                    .unwrap()
                    .write_all(b"x")
            })
            .unwrap();
        let report = recorder.report().unwrap();
        assert_eq!(report.files.len(), 1);
        assert!(report.files[0].edits[0].modified.is_none());
    }

    #[test]
    fn binary_edits_have_no_contents() {
        let root = tempfile::tempdir().unwrap();
        let recorder = recorder(root.path(), "job");
        let path = root.path().join("binary");
        recorder
            .record(&path, || fs::write(&path, [0, 1, 2]))
            .unwrap();
        let report = recorder.report().unwrap();
        assert_eq!(report.files[0].kind, "added");
        assert!(report.files[0].edits[0].modified.is_none());
    }
}
