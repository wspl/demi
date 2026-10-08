use std::{collections::HashSet, fs, path::PathBuf, sync::LazyLock};

use regex::Regex;
use tokio_util::sync::CancellationToken;

use crate::files::{FileError, Unchosen, atomic_write, check_cancelled, nearest, resolve};

/// Why a patch does not apply; its message is what the agent reads.
#[derive(Debug, thiserror::Error)]
pub enum PatchError {
    #[error("Patch has no file path")]
    NoPath,
    #[error(transparent)]
    NotText(#[from] std::str::Utf8Error),
    #[error("Patch does not apply to {label}: {source}")]
    DoesNotApply {
        label: String,
        source: Box<PatchError>,
    },
    #[error("Delete patch leaves file content")]
    DeleteLeavesContent,
    #[error("Patch destination already exists")]
    DestinationExists,
    #[error("Patch changes the same path more than once")]
    PathTwice,
    #[error("hunk {hunk} ({header}) inserts after line {line}, past the file's end")]
    HunkOutOfRange {
        hunk: usize,
        header: String,
        line: usize,
    },
    #[error("hunk {hunk} ({header}) matches no lines")]
    HunkNoMatch { hunk: usize, header: String },
    #[error(
        "hunk {hunk} ({header}) matches at lines {lines}; give its header's line numbers or more context"
    )]
    HunkAmbiguous {
        hunk: usize,
        header: String,
        lines: String,
    },
    #[error(
        "hunk {hunk} ({header}) has no context or removed lines, so it needs its header's line numbers"
    )]
    HunkUnanchored { hunk: usize, header: String },
    #[error("New header precedes old header")]
    NewBeforeOld,
    #[error("Invalid patch hunk header")]
    HunkHeader,
    #[error("Invalid hunk count")]
    HunkCount,
    #[error("Hunk before file header")]
    HunkBeforeHeader,
    #[error("Invalid hunk start")]
    HunkStart,
    #[error("Newline marker without patch line")]
    NewlineMarker,
    #[error("Invalid patch line: {0}")]
    Line(String),
    #[error("Invalid patch: missing file headers or hunks")]
    Incomplete,
}

struct FilePatch {
    old: Option<String>,
    new: Option<String>,
    hunks: Vec<Hunk>,
}

/// A hunk: its header as written, the old start and line count the
/// header gives, when it gives them, and its lines.
struct Hunk {
    header: String,
    start: Option<usize>,
    old_count: Option<usize>,
    lines: Vec<Line>,
    /// How many of the last lines were empty lines of the diff: empty
    /// context lines inside the hunk, but at its end the blank lines that
    /// separate it from what follows.
    trailing_blank: usize,
}

impl Hunk {
    /// The hunk's lines without the blank lines after it.
    fn lines(&self) -> &[Line] {
        &self.lines[..self.lines.len() - self.trailing_blank]
    }
}

struct Line {
    kind: u8,
    text: String,
    newline: bool,
}

struct Change {
    path: PathBuf,
    before: Option<Vec<u8>>,
    after: Option<Vec<u8>>,
    permissions: Option<fs::Permissions>,
}

pub fn apply(
    cwd: &str,
    diff: &str,
    cancellation: &CancellationToken,
    mut recording: Option<&mut demi_command_sdk::edits::Recording>,
) -> Result<String, FileError> {
    let patches = parse(diff, cancellation)?;
    let mut changes = Vec::new();
    let mut touched = HashSet::new();
    for patch in &patches {
        check_cancelled(cancellation)?;
        let old_path = patch
            .old
            .as_ref()
            .map(|path| resolve(cwd, path))
            .transpose()?;
        let new_path = patch
            .new
            .as_ref()
            .map(|path| resolve(cwd, path))
            .transpose()?;
        let before = old_path.as_ref().map(fs::read).transpose()?;
        let original =
            std::str::from_utf8(before.as_deref().unwrap_or_default()).map_err(PatchError::from)?;
        let label = patch
            .old
            .as_deref()
            .or(patch.new.as_deref())
            .ok_or(PatchError::NoPath)?;
        let updated =
            apply_hunks(original, &patch.hunks, cancellation).map_err(|error| match error {
                FileError::Patch(source) => FileError::Patch(PatchError::DoesNotApply {
                    label: label.to_owned(),
                    source: Box::new(source),
                }),
                error => error,
            })?;
        if new_path.is_none() && !updated.is_empty() {
            return Err(PatchError::DeleteLeavesContent.into());
        }
        if new_path != old_path
            && new_path
                .as_ref()
                .is_some_and(|path| fs::symlink_metadata(path).is_ok())
        {
            return Err(PatchError::DestinationExists.into());
        }
        if let Some(path) = new_path.clone() {
            let same = new_path == old_path;
            let permissions = if same {
                Some(fs::metadata(&path)?.permissions())
            } else {
                None
            };
            changes.push(Change {
                path,
                before: if same { before.clone() } else { None },
                after: Some(updated.into_bytes()),
                permissions,
            });
        }
        if old_path != new_path
            && let Some(path) = old_path
        {
            let permissions = Some(fs::metadata(&path)?.permissions());
            changes.push(Change {
                path,
                before,
                after: None,
                permissions,
            });
        }
    }
    for change in &changes {
        if !touched.insert(change.path.clone()) {
            return Err(PatchError::PathTwice.into());
        }
    }
    changes.retain(|change| change.before != change.after);
    if let Some(recording) = &mut recording {
        for change in &changes {
            recording.track(&change.path);
        }
    }
    commit_changes(&changes, cancellation, write_change, recording)?;
    Ok(format!("Patched {} file(s)\n", patches.len()))
}

fn commit_changes(
    changes: &[Change],
    cancellation: &CancellationToken,
    mut write: impl FnMut(&Change) -> Result<(), FileError>,
    mut recording: Option<&mut demi_command_sdk::edits::Recording>,
) -> Result<(), FileError> {
    for (index, change) in changes.iter().enumerate() {
        let result = check_cancelled(cancellation).and_then(|()| write(change));
        if let Err(error) = result {
            let mut rollbacks = Vec::new();
            for change in changes[..index].iter().rev() {
                match restore(change) {
                    Err(rollback) => rollbacks.push(rollback),
                    Ok(()) => {
                        if let Some(recording) = &mut recording {
                            recording.restored(&change.path);
                        }
                    }
                }
            }
            if rollbacks.is_empty() {
                return Err(error);
            }
            return Err(FileError::Rollback {
                error: Box::new(error),
                rollbacks,
            });
        }
    }
    Ok(())
}

fn write_change(change: &Change) -> Result<(), FileError> {
    match &change.after {
        Some(bytes) => atomic_write(&change.path, bytes, change.before.is_none()),
        None => Ok(fs::remove_file(&change.path)?),
    }
}

fn restore(change: &Change) -> Result<(), FileError> {
    match &change.before {
        Some(bytes) => {
            atomic_write(&change.path, bytes, change.after.is_none())?;
            if let Some(permissions) = &change.permissions {
                fs::set_permissions(&change.path, permissions.clone())?;
            }
            Ok(())
        }
        None => Ok(fs::remove_file(&change.path)?),
    }
}

/// `original` with `hunks` applied in order. A hunk's header gives no line
/// counts that matter, as `git apply --recount` reads it: the hunk goes where
/// its context and removed lines match, the only place or the one nearest
/// its header's line (`commands.md` § File commands).
fn apply_hunks(
    original: &str,
    hunks: &[Hunk],
    cancellation: &CancellationToken,
) -> Result<String, FileError> {
    let mut lines: Vec<String> = original.split_inclusive('\n').map(String::from).collect();
    // How far the lines moved by the hunks applied before, against the
    // header's numbers.
    let mut offset = 0isize;
    for (index, hunk) in hunks.iter().enumerate() {
        check_cancelled(cancellation)?;
        let number = index + 1;
        let old: Vec<_> = hunk
            .lines()
            .iter()
            .filter(|line| line.kind != b'+')
            .map(line_text)
            .collect();
        let new: Vec<_> = hunk
            .lines()
            .iter()
            .filter(|line| line.kind != b'-')
            .map(line_text)
            .collect();
        // The 0-based line the header names: its first old line, or for a
        // header whose old count is 0, the line it inserts after.
        let expected = hunk.start.map(|start| {
            let first = if hunk.old_count == Some(0) {
                start
            } else {
                start.saturating_sub(1)
            };
            first.saturating_add_signed(offset)
        });
        let at = if old.is_empty() {
            match expected {
                _ if lines.is_empty() => 0,
                Some(line) if line <= lines.len() => line,
                Some(line) => {
                    return Err(PatchError::HunkOutOfRange {
                        hunk: number,
                        header: hunk.header.clone(),
                        line,
                    }
                    .into());
                }
                None => {
                    return Err(PatchError::HunkUnanchored {
                        hunk: number,
                        header: hunk.header.clone(),
                    }
                    .into());
                }
            }
        } else {
            let matches: Vec<usize> = lines
                .windows(old.len())
                .enumerate()
                .filter(|(_, window)| *window == old.as_slice())
                .map(|(first, _)| first)
                .collect();
            choose(&matches, expected).map_err(|several| match several {
                None => PatchError::HunkNoMatch {
                    hunk: number,
                    header: hunk.header.clone(),
                },
                Some(lines) => PatchError::HunkAmbiguous {
                    hunk: number,
                    header: hunk.header.clone(),
                    lines,
                },
            })?
        };
        offset += new.len() as isize - old.len() as isize;
        lines.splice(at..at + old.len(), new);
    }
    Ok(lines.concat())
}

/// The match to use of `matches`: the only one, or the one nearest
/// `expected`. Without one, the error is `None` when nothing matches and
/// the 1-based lines of the matches it cannot choose among otherwise.
fn choose(matches: &[usize], expected: Option<usize>) -> Result<usize, Option<String>> {
    let chosen = match (matches, expected) {
        ([], _) => return Err(None),
        ([only], _) => Ok(*only),
        (_, None) => Err(Unchosen::Tie),
        (_, Some(expected)) => nearest(matches, |first| first.abs_diff(expected)),
    };
    chosen.map_err(|_| {
        Some(
            matches
                .iter()
                .map(|first| (first + 1).to_string())
                .collect::<Vec<_>>()
                .join(", "),
        )
    })
}

fn line_text(line: &Line) -> String {
    if line.newline {
        format!("{}\n", line.text)
    } else {
        line.text.clone()
    }
}

static HEADER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^@@ -(\d+)(?:,(\d+))? \+\d+(?:,\d+)? @@").unwrap());
static DATE_SUFFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\s+\d{4}-\d{2}-\d{2}(?:[ T]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:\s+[+-]\d{4})?)?$")
        .unwrap()
});

/// The file patches of `diff`. Hunk line counts are not trusted, so inside
/// a hunk a `--- ` line starts the next file only when a `+++ ` line and an
/// `@@` line follow it, and is a removed line otherwise; an empty line is an
/// empty context line, as `git apply` reads it.
fn parse(diff: &str, cancellation: &CancellationToken) -> Result<Vec<FilePatch>, FileError> {
    let mut patches: Vec<FilePatch> = Vec::new();
    let mut pending_old = None;
    // The text after the last line ending is no line.
    let lines: Vec<&str> = diff.strip_suffix('\n').unwrap_or(diff).split('\n').collect();
    for (index, &line) in lines.iter().enumerate() {
        check_cancelled(cancellation)?;
        let in_hunk = pending_old.is_none()
            && patches
                .last()
                .is_some_and(|patch| !patch.hunks.is_empty());
        let old_header = line.starts_with("--- ")
            && lines
                .get(index + 1)
                .is_some_and(|next| next.starts_with("+++ "))
            && lines
                .get(index + 2)
                .is_some_and(|next| next.starts_with("@@"));
        if let Some(path) = line.strip_prefix("--- ")
            && (old_header || !in_hunk)
        {
            pending_old = Some(parse_path(path));
        } else if let Some(path) = line.strip_prefix("+++ ")
            && (pending_old.is_some() || !in_hunk)
        {
            patches.push(FilePatch {
                old: pending_old.take().ok_or(PatchError::NewBeforeOld)?,
                new: parse_path(path),
                hunks: Vec::new(),
            });
        } else if line.starts_with("@@") {
            let hunk = match HEADER.captures(line) {
                Some(header) => Hunk {
                    header: line.to_owned(),
                    start: Some(header[1].parse().map_err(|_| PatchError::HunkStart)?),
                    old_count: header
                        .get(2)
                        .map(|count| count.as_str().parse().map_err(|_| PatchError::HunkCount))
                        .transpose()?,
                    lines: Vec::new(),
                    trailing_blank: 0,
                },
                // A header without numbers, such as a bare `@@`.
                None if !line.starts_with("@@ -") => Hunk {
                    header: line.to_owned(),
                    start: None,
                    old_count: None,
                    lines: Vec::new(),
                    trailing_blank: 0,
                },
                None => return Err(PatchError::HunkHeader.into()),
            };
            patches
                .last_mut()
                .ok_or(PatchError::HunkBeforeHeader)?
                .hunks
                .push(hunk);
        } else if let Some(hunk) = patches.last_mut().and_then(|patch| patch.hunks.last_mut()) {
            match line.as_bytes().first() {
                Some(kind @ (b' ' | b'-' | b'+')) => {
                    hunk.lines.push(Line {
                        kind: *kind,
                        text: line[1..].into(),
                        newline: true,
                    });
                    hunk.trailing_blank = 0;
                }
                None => {
                    hunk.lines.push(Line {
                        kind: b' ',
                        text: String::new(),
                        newline: true,
                    });
                    hunk.trailing_blank += 1;
                }
                _ if line == "\\ No newline at end of file" => {
                    hunk.lines
                        .last_mut()
                        .ok_or(PatchError::NewlineMarker)?
                        .newline = false;
                }
                _ if line.starts_with("diff ") || line.starts_with("index ") => {}
                _ => return Err(PatchError::Line(line.to_owned()).into()),
            }
        }
    }
    if patches.is_empty()
        || pending_old.is_some()
        || patches
            .iter()
            .any(|patch| patch.hunks.is_empty() || (patch.old.is_none() && patch.new.is_none()))
    {
        return Err(PatchError::Incomplete.into());
    }
    Ok(patches)
}

fn parse_path(value: &str) -> Option<String> {
    let value = value.trim();
    let value = value.split('\t').next().unwrap();
    let value = DATE_SUFFIX.replace(value, "");
    if value == "/dev/null" {
        return None;
    }
    Some(
        value
            .strip_prefix("a/")
            .or_else(|| value.strip_prefix("b/"))
            .unwrap_or(&value)
            .into(),
    )
}
