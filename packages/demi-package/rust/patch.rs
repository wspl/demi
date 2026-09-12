use std::{collections::HashSet, fs, path::PathBuf, sync::LazyLock};

use regex::Regex;
use tokio_util::sync::CancellationToken;

use crate::files::{atomic_write, check_cancelled, resolve_path};

struct FilePatch {
    old: Option<String>,
    new: Option<String>,
    hunks: Vec<Hunk>,
}

struct Hunk {
    start: usize,
    old_count: usize,
    new_count: usize,
    lines: Vec<Line>,
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

pub fn apply(cwd: &str, diff: &str, cancellation: &CancellationToken) -> Result<String, String> {
    let patches = parse(diff, cancellation)?;
    let mut changes = Vec::new();
    let mut touched = HashSet::new();
    for patch in &patches {
        check_cancelled(cancellation)?;
        let old_path = patch
            .old
            .as_ref()
            .map(|path| resolve_path(cwd, path))
            .transpose()?;
        let new_path = patch
            .new
            .as_ref()
            .map(|path| resolve_path(cwd, path))
            .transpose()?;
        let before = old_path
            .as_ref()
            .map(fs::read)
            .transpose()
            .map_err(|error| error.to_string())?;
        let original = std::str::from_utf8(before.as_deref().unwrap_or_default())
            .map_err(|error| error.to_string())?;
        let updated = apply_hunks(original, &patch.hunks, cancellation)?;
        if new_path.is_none() && !updated.is_empty() {
            return Err("Delete patch leaves file content".into());
        }
        if new_path != old_path
            && new_path
                .as_ref()
                .is_some_and(|path| fs::symlink_metadata(path).is_ok())
        {
            return Err("Patch destination already exists".into());
        }
        if let Some(path) = new_path.clone() {
            let same = new_path == old_path;
            let permissions = if same {
                Some(
                    fs::metadata(&path)
                        .map_err(|error| error.to_string())?
                        .permissions(),
                )
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
            let permissions = Some(
                fs::metadata(&path)
                    .map_err(|error| error.to_string())?
                    .permissions(),
            );
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
            return Err("Patch changes the same path more than once".into());
        }
    }
    for (index, change) in changes.iter().enumerate() {
        let result = check_cancelled(cancellation).and_then(|()| write_change(change));
        if let Err(mut error) = result {
            for change in changes[..index].iter().rev() {
                if let Err(rollback) = restore(change) {
                    error.push_str(&format!("\nRollback failed: {rollback}"));
                }
            }
            return Err(error);
        }
    }
    Ok(format!("Patched {} file(s)\n", patches.len()))
}

fn write_change(change: &Change) -> Result<(), String> {
    match &change.after {
        Some(bytes) => atomic_write(&change.path, bytes, change.before.is_none()),
        None => fs::remove_file(&change.path).map_err(|error| error.to_string()),
    }
}

fn restore(change: &Change) -> Result<(), String> {
    match &change.before {
        Some(bytes) => {
            atomic_write(&change.path, bytes, change.after.is_none())?;
            if let Some(permissions) = &change.permissions {
                fs::set_permissions(&change.path, permissions.clone())
                    .map_err(|error| error.to_string())?;
            }
            Ok(())
        }
        None => fs::remove_file(&change.path).map_err(|error| error.to_string()),
    }
}

fn apply_hunks(
    original: &str,
    hunks: &[Hunk],
    cancellation: &CancellationToken,
) -> Result<String, String> {
    let mut lines: Vec<String> = original.split_inclusive('\n').map(String::from).collect();
    let mut offset = 0isize;
    for hunk in hunks {
        check_cancelled(cancellation)?;
        let original_start = if hunk.old_count == 0 {
            hunk.start
        } else {
            hunk.start.saturating_sub(1)
        };
        let start = isize::try_from(original_start)
            .ok()
            .and_then(|start| start.checked_add(offset))
            .ok_or("Patch hunk position is out of range")?;
        if start < 0 {
            return Err("Patch hunk starts before file".into());
        }
        let start = start as usize;
        let old: Vec<_> = hunk
            .lines
            .iter()
            .filter(|line| line.kind != b'+')
            .map(line_text)
            .collect();
        let new: Vec<_> = hunk
            .lines
            .iter()
            .filter(|line| line.kind != b'-')
            .map(line_text)
            .collect();
        if old.len() != hunk.old_count || new.len() != hunk.new_count {
            return Err("Patch hunk line counts do not match header".into());
        }
        let end = start
            .checked_add(old.len())
            .ok_or("Patch hunk position is out of range")?;
        if lines.get(start..end) != Some(old.as_slice()) {
            return Err(format!("Patch does not apply at line {}", hunk.start));
        }
        offset += new.len() as isize - old.len() as isize;
        lines.splice(start..end, new);
    }
    Ok(lines.concat())
}

fn line_text(line: &Line) -> String {
    if line.newline {
        format!("{}\n", line.text)
    } else {
        line.text.clone()
    }
}

static HEADER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^@@ -(\d+)(?:,(\d+))? \+\d+(?:,(\d+))? @@").unwrap());
static DATE_SUFFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\s+\d{4}-\d{2}-\d{2}(?:[ T]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:\s+[+-]\d{4})?)?$")
        .unwrap()
});

fn parse(diff: &str, cancellation: &CancellationToken) -> Result<Vec<FilePatch>, String> {
    let mut patches: Vec<FilePatch> = Vec::new();
    let mut pending_old = None;
    for line in diff.split('\n') {
        check_cancelled(cancellation)?;
        if pending_old.is_none()
            && (line.starts_with("--- ") || line.starts_with("+++ "))
            && let Some(hunk) = patches.last_mut().and_then(|patch| patch.hunks.last_mut())
        {
            let old = hunk.lines.iter().filter(|line| line.kind != b'+').count();
            let new = hunk.lines.iter().filter(|line| line.kind != b'-').count();
            if old < hunk.old_count || new < hunk.new_count {
                hunk.lines.push(Line {
                    kind: line.as_bytes()[0],
                    text: line[1..].into(),
                    newline: true,
                });
                continue;
            }
        }
        if let Some(path) = line.strip_prefix("--- ") {
            pending_old = Some(parse_path(path));
        } else if let Some(path) = line.strip_prefix("+++ ") {
            patches.push(FilePatch {
                old: pending_old.take().ok_or("New header precedes old header")?,
                new: parse_path(path),
                hunks: Vec::new(),
            });
        } else if line.starts_with("@@ ") {
            let header = HEADER.captures(line).ok_or("Invalid patch hunk header")?;
            let parse_count = |index| -> Result<usize, String> {
                header.get(index).map_or(Ok(1), |value| {
                    value
                        .as_str()
                        .parse()
                        .map_err(|_| "Invalid hunk count".into())
                })
            };
            patches
                .last_mut()
                .ok_or("Hunk before file header")?
                .hunks
                .push(Hunk {
                    start: header[1].parse().map_err(|_| "Invalid hunk start")?,
                    old_count: parse_count(2)?,
                    new_count: parse_count(3)?,
                    lines: Vec::new(),
                });
        } else if let Some(hunk) = patches.last_mut().and_then(|patch| patch.hunks.last_mut()) {
            match line.as_bytes().first() {
                Some(kind @ (b' ' | b'-' | b'+')) => hunk.lines.push(Line {
                    kind: *kind,
                    text: line[1..].into(),
                    newline: true,
                }),
                _ if line == "\\ No newline at end of file" => {
                    hunk.lines
                        .last_mut()
                        .ok_or("Newline marker without patch line")?
                        .newline = false;
                }
                _ if line.is_empty() || line.starts_with("diff ") || line.starts_with("index ") => {
                }
                _ => return Err(format!("Invalid patch line: {line}")),
            }
        }
    }
    if patches.is_empty()
        || pending_old.is_some()
        || patches
            .iter()
            .any(|patch| patch.hunks.is_empty() || (patch.old.is_none() && patch.new.is_none()))
    {
        return Err("Invalid patch: missing file headers or hunks".into());
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
