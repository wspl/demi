//! The `file.*` operations: reading a file, and creating, editing or patching
//! files one mutation at a time.

use std::{
    fs,
    ops::Range,
    path::{Path, PathBuf},
};

use bytes::Bytes;
use demi_command_package_file_protocol::{Block, Change, Choice, CreateArgs, Edit, Operation, PatchArgs, ReadArgs};
use demi_command_protocol::{CommandError, Completion, MAX_MEDIUM_BYTES, sniff_media_type};
use demi_command_sdk::{
    InvocationContext, ServiceError,
    edits::{Recorder, Recording},
};
use demi_shared_artifacts::{Mode, Permissions, Publication};
use demi_shared_gates::SerialGate;
use demi_shared_types::DecodeError;
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;

use crate::patch::{self, PatchError};

/// How much of a file one read sends on.
const READ_BYTES: usize = 64 * 1024;

/// Why a file operation failed; its message is what the agent reads.
#[derive(Debug, thiserror::Error)]
pub enum FileError {
    #[error("Command cancelled")]
    Cancelled,
    #[error(transparent)]
    Arguments(#[from] DecodeError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Publication(#[from] demi_shared_artifacts::Error),
    #[error("File has no parent directory")]
    NoParent,
    #[error("Occurrence {0} is out of range")]
    OccurrenceOutOfRange(usize),
    #[error("No match found")]
    NoMatchNearContext,
    #[error("Context line {context} is ambiguous: {candidates}")]
    AmbiguousContext { context: usize, candidates: String },
    #[error("No match found in {0}")]
    NoMatch(String),
    #[error("Multiple matches in {0}; specify --occurrence or --context")]
    MultipleMatches(String),
    #[error("Block {block}'s SEARCH matches no lines of {name}. {closest}")]
    BlockNoMatch {
        block: usize,
        name: String,
        closest: String,
    },
    #[error(
        "Block {block}'s SEARCH matches {name} at {places}; add a line around it so it matches one place"
    )]
    BlockMatchesSeveral {
        block: usize,
        name: String,
        places: String,
    },
    #[error("Blocks {first} and {second} overlap in {name}; make them one block")]
    BlocksOverlap {
        first: usize,
        second: usize,
        name: String,
    },
    #[error(transparent)]
    Patch(#[from] PatchError),
    /// The result could not be sent back.
    #[error(transparent)]
    Output(#[from] ServiceError),
    /// A write failed and restoring the files written before it failed too.
    #[error("{error}{}", .rollbacks.iter().map(|failure| format!("\nRollback failed: {failure}")).collect::<String>())]
    Rollback {
        error: Box<FileError>,
        rollbacks: Vec<FileError>,
    },
}

/// Runs one `file.*` invocation: a read streams the file to stdout, and a
/// mutation plans, writes and rolls back on the blocking pool while it holds
/// `mutations`, so two mutations never interleave.
pub async fn invoke(
    context: InvocationContext,
    operation: Operation,
    mutations: SerialGate,
) -> Result<Completion, ServiceError> {
    let result = match operation {
        Operation::Read(args) => read(&context, &args).await,
        Operation::Create(args) => {
            mutate(&context, &mutations, move |cwd, cancellation, recording| {
                create(cwd, args, cancellation, recording)
            })
            .await
        }
        Operation::Edit(args) => {
            mutate(&context, &mutations, move |cwd, cancellation, recording| {
                edit(cwd, &args, cancellation, recording)
            })
            .await
        }
        Operation::Patch(PatchArgs { patch }) => {
            mutate(&context, &mutations, move |cwd, cancellation, recording| {
                patch::apply(cwd, &patch, cancellation, recording)
            })
            .await
        }
    };
    match result {
        Ok(()) => Ok(Completion {
            exit_code: 0,
            error: None,
        }),
        Err(FileError::Cancelled) => Err(ServiceError::Cancelled),
        Err(FileError::Output(error)) => Err(error),
        Err(error) => Ok(failure(&error)),
    }
}

/// The completion of a file operation that failed.
pub fn failure(error: &FileError) -> Completion {
    Completion {
        exit_code: 1,
        error: Some(CommandError {
            code: "command_failed".into(),
            message: error.to_string(),
        }),
    }
}

/// Streams the file to stdout. A job's command reading a regular file of at
/// most 16 MiB whose bytes are an image or video a model reads returns it as
/// a medium instead (`commands.md` § File commands).
async fn read(context: &InvocationContext, args: &ReadArgs) -> Result<(), FileError> {
    let path = demi_command_sdk::paths::resolve(&context.request.cwd, &args.path)?;
    let mut file = tokio::fs::File::open(path).await?;
    let metadata = file.metadata().await?;
    if context.request.stdout.is_some() && metadata.is_file() && metadata.len() <= MAX_MEDIUM_BYTES
    {
        let mut bytes = Vec::new();
        tokio::select! {
            _ = context.cancellation.cancelled() => return Err(FileError::Cancelled),
            result = file.read_to_end(&mut bytes) => result?,
        };
        let bytes = Bytes::from(bytes);
        if sniff_media_type(&bytes).is_some() {
            context.output.medium(bytes).await?;
        } else {
            context.output.stdout(bytes).await?;
        }
        return Ok(());
    }
    let mut buffer = vec![0; READ_BYTES];
    loop {
        let count = tokio::select! {
            _ = context.cancellation.cancelled() => return Err(FileError::Cancelled),
            result = file.read(&mut buffer) => result?,
        };
        if count == 0 {
            return Ok(());
        }
        context
            .output
            .stdout(Bytes::copy_from_slice(&buffer[..count]))
            .await?;
    }
}

/// Runs `mutation` on the blocking pool while it holds `mutations`, and
/// sends its message on.
async fn mutate(
    context: &InvocationContext,
    mutations: &SerialGate,
    mutation: impl FnOnce(&str, &CancellationToken, Option<&mut Recording>) -> Result<String, FileError>
    + Send
    + 'static,
) -> Result<(), FileError> {
    let permit = tokio::select! {
        _ = context.cancellation.cancelled() => return Err(FileError::Cancelled),
        permit = mutations.acquire() => permit,
    };
    let cwd = context.request.cwd.clone();
    let edits = context.request.edits.clone();
    let cancellation = context.cancellation.clone();
    let message = tokio::task::spawn_blocking(move || {
        // The permit covers planning, writes and rollback, cancellation included.
        let _permit = permit;
        let recorder = edits.and_then(|edits| match Recorder::new(edits) {
            Ok(recorder) => Some(recorder),
            Err(error) => {
                tracing::warn!("edit recording failed: {error}");
                None
            }
        });
        let mut recording = recorder.as_ref().and_then(Recorder::begin);
        check_cancelled(&cancellation)?;
        mutation(&cwd, &cancellation, recording.as_mut())
    })
    .await
    .map_err(std::io::Error::other)??;
    context.output.stdout(Bytes::from(message)).await?;
    Ok(())
}

pub(crate) fn check_cancelled(cancellation: &CancellationToken) -> Result<(), FileError> {
    if cancellation.is_cancelled() {
        return Err(FileError::Cancelled);
    }
    Ok(())
}

/// Replaces the file at `path` with `bytes`, or creates it when `create`,
/// in one step a reader never sees half done. An edit of a symbolic link
/// writes the file it points at and keeps that file's permissions.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8], create: bool) -> Result<(), FileError> {
    let destination = if !create && path.is_symlink() {
        fs::canonicalize(path)?
    } else {
        path.to_owned()
    };
    let parent = destination.parent().ok_or(FileError::NoParent)?;
    fs::create_dir_all(parent)?;
    let publication = if create {
        Publication {
            mode: Mode::CreateNew,
            permissions: Permissions::Default,
            durable: true,
        }
    } else {
        Publication {
            mode: Mode::Replace,
            permissions: Permissions::Keep,
            durable: true,
        }
    };
    demi_shared_artifacts::publish_bytes_blocking(&destination, bytes, publication)?;
    Ok(())
}

fn create(
    cwd: &str,
    CreateArgs {
        path: name,
        content,
    }: CreateArgs,
    cancellation: &CancellationToken,
    recording: Option<&mut Recording>,
) -> Result<String, FileError> {
    let path = resolve(cwd, &name)?;
    if let Some(recording) = recording {
        recording.track(&path);
    }
    check_cancelled(cancellation)?;
    atomic_write(&path, content.as_bytes(), true)?;
    Ok(format!("Created {name}\n"))
}

pub(crate) fn resolve(cwd: &str, path: &str) -> Result<PathBuf, FileError> {
    Ok(demi_command_sdk::paths::resolve(cwd, path)?)
}

/// Replaces what `args` names in its file: every replacement is planned
/// against the file as it was, and they are written together or not at all.
fn edit(
    cwd: &str,
    args: &Edit,
    cancellation: &CancellationToken,
    recording: Option<&mut Recording>,
) -> Result<String, FileError> {
    let path = resolve(cwd, &args.path)?;
    let content = fs::read_to_string(&path)?;
    let replacements = match &args.change {
        Change::Text { old, new, choice } => {
            vec![text_replacement(&content, &args.path, old, new, *choice)?]
        }
        Change::Blocks(blocks) => {
            let lines = Lines::of(&content);
            blocks
                .iter()
                .enumerate()
                .map(|(index, block)| {
                    check_cancelled(cancellation)?;
                    lines.replacement(index + 1, block, &args.path)
                })
                .collect::<Result<_, _>>()?
        }
    };
    let updated = replace_all(&content, replacements, &args.path)?;
    check_cancelled(cancellation)?;
    if updated != content {
        if let Some(recording) = recording {
            recording.track(&path);
        }
        atomic_write(&path, updated.as_bytes(), false)?;
    }
    Ok(format!("Edited {}\n", args.path))
}

/// One planned replacement: the bytes of the file it replaces, its text,
/// and the block it comes from, for a message about two that overlap.
struct Replacement {
    range: Range<usize>,
    text: String,
    block: usize,
}

/// `--old` replaced by `--new`, at the match `choice` names.
fn text_replacement(
    content: &str,
    name: &str,
    old: &str,
    new: &str,
    choice: Choice,
) -> Result<Replacement, FileError> {
    let matches: Vec<_> = content.match_indices(old).map(|(index, _)| index).collect();
    let index = match choice {
        Choice::Occurrence(occurrence) => *matches
            .get(occurrence - 1)
            .ok_or(FileError::OccurrenceOutOfRange(occurrence))?,
        Choice::Context(context) => {
            match nearest(&matches, |index| line_of(content, index).abs_diff(context)) {
                Ok(index) => index,
                Err(Unchosen::Empty) => return Err(FileError::NoMatchNearContext),
                Err(Unchosen::Tie) => {
                    let candidates = matches
                        .iter()
                        .enumerate()
                        .map(|(occurrence, &index)| {
                            format!(
                                "occurrence {} at line {}",
                                occurrence + 1,
                                line_of(content, index)
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("; ");
                    return Err(FileError::AmbiguousContext {
                        context,
                        candidates,
                    });
                }
            }
        }
        Choice::Only => match matches.as_slice() {
            [index] => *index,
            [] => return Err(FileError::NoMatch(name.to_owned())),
            _ => return Err(FileError::MultipleMatches(name.to_owned())),
        },
    };
    // `--old` is the edit's one replacement, so it overlaps no other.
    Ok(Replacement {
        range: index..index + old.len(),
        text: new.to_owned(),
        block: 1,
    })
}

/// Why [`nearest`] chose no candidate.
pub(crate) enum Unchosen {
    /// There is none.
    Empty,
    /// Two are equally near.
    Tie,
}

/// The candidate `distance` puts nearest, of several matches of one text:
/// a match nearest a line the caller names.
pub(crate) fn nearest<T: Copy>(
    candidates: &[T],
    distance: impl Fn(T) -> usize,
) -> Result<T, Unchosen> {
    let mut ranked: Vec<_> = candidates
        .iter()
        .map(|&candidate| (distance(candidate), candidate))
        .collect();
    ranked.sort_by_key(|&(distance, _)| distance);
    match ranked.as_slice() {
        [] => Err(Unchosen::Empty),
        [first, second, ..] if first.0 == second.0 => Err(Unchosen::Tie),
        [(_, candidate), ..] => Ok(*candidate),
    }
}

/// `content` with each replacement made; two that overlap fail.
fn replace_all(
    content: &str,
    mut replacements: Vec<Replacement>,
    name: &str,
) -> Result<String, FileError> {
    replacements.sort_by_key(|replacement| replacement.range.start);
    for pair in replacements.windows(2) {
        if pair[1].range.start < pair[0].range.end {
            let mut blocks = [pair[0].block, pair[1].block];
            blocks.sort_unstable();
            return Err(FileError::BlocksOverlap {
                first: blocks[0],
                second: blocks[1],
                name: name.to_owned(),
            });
        }
    }
    let mut updated = String::with_capacity(content.len());
    let mut end = 0;
    for replacement in &replacements {
        updated.push_str(&content[end..replacement.range.start]);
        updated.push_str(&replacement.text);
        end = replacement.range.end;
    }
    updated.push_str(&content[end..]);
    Ok(updated)
}

/// A file's lines: where each starts, where its text ends, and where its
/// line ending ends.
struct Lines<'a> {
    content: &'a str,
    lines: Vec<Line>,
}

struct Line {
    start: usize,
    text_end: usize,
    end: usize,
}

impl<'a> Lines<'a> {
    fn of(content: &'a str) -> Self {
        let mut lines = Vec::new();
        let mut start = 0;
        for line in content.split_inclusive('\n') {
            let end = start + line.len();
            let text = line
                .strip_suffix('\n')
                .map_or(line, |text| text.strip_suffix('\r').unwrap_or(text));
            lines.push(Line {
                start,
                text_end: start + text.len(),
                end,
            });
            start = end;
        }
        Self { content, lines }
    }

    fn text(&self, index: usize) -> &'a str {
        let line = &self.lines[index];
        &self.content[line.start..line.text_end]
    }

    fn ending(&self, index: usize) -> &'a str {
        let line = &self.lines[index];
        &self.content[line.text_end..line.end]
    }

    /// Where `block`'s SEARCH matches whole lines: the index of each
    /// match's first line.
    fn matches(&self, block: &Block) -> Vec<usize> {
        let count = block.search.len();
        (0..(self.lines.len() + 1).saturating_sub(count))
            .filter(|&first| {
                block
                    .search
                    .iter()
                    .enumerate()
                    .all(|(offset, line)| self.text(first + offset) == line)
            })
            .collect()
    }

    /// The replacement `block`, the `number`th, makes: its REPLACE in place
    /// of the lines its SEARCH matches exactly once, written with the
    /// file's own line endings.
    fn replacement(
        &self,
        number: usize,
        block: &Block,
        name: &str,
    ) -> Result<Replacement, FileError> {
        let count = block.search.len();
        let first = match self.matches(block).as_slice() {
            [first] => *first,
            [] => {
                return Err(FileError::BlockNoMatch {
                    block: number,
                    name: name.to_owned(),
                    closest: self.closest(block),
                });
            }
            several => {
                return Err(FileError::BlockMatchesSeveral {
                    block: number,
                    name: name.to_owned(),
                    places: several
                        .iter()
                        .map(|&first| line_span(first, count))
                        .collect::<Vec<_>>()
                        .join(" and "),
                });
            }
        };
        let last = first + count - 1;
        let ending = (first..=last)
            .map(|index| self.ending(index))
            .chain((0..self.lines.len()).map(|index| self.ending(index)))
            .find(|ending| !ending.is_empty())
            .unwrap_or("\n");
        let mut text = String::new();
        for (index, line) in block.replace.iter().enumerate() {
            text.push_str(line);
            // The last line keeps the matched last line's ending, none at
            // the end of a file without a final newline.
            if index + 1 < block.replace.len() || !self.ending(last).is_empty() {
                text.push_str(ending);
            }
        }
        Ok(Replacement {
            range: self.lines[first].start..self.lines[last].end,
            text,
            block: number,
        })
    }

    /// The lines of the file most like `block`'s SEARCH, numbered: as many
    /// lines as it has, from the first that matches its lines best.
    fn closest(&self, block: &Block) -> String {
        if self.lines.is_empty() {
            return "The file is empty.".to_owned();
        }
        let count = block.search.len().min(self.lines.len());
        let score = |first: usize| -> f64 {
            block
                .search
                .iter()
                .zip(first..first + count)
                .map(|(line, index)| strsim::sorensen_dice(line.trim(), self.text(index).trim()))
                .sum()
        };
        let mut best = 0;
        let mut best_score = f64::MIN;
        for first in 0..=self.lines.len() - count {
            let score = score(first);
            if score > best_score {
                best = first;
                best_score = score;
            }
        }
        let lines = (best..best + count)
            .map(|index| format!("{}: {}", index + 1, self.text(index)))
            .collect::<Vec<_>>()
            .join("\n");
        format!("The closest are {}:\n{lines}", line_span(best, count))
    }
}

/// `line 3` or `lines 3-5`, for `count` lines from the 0-based `first`.
fn line_span(first: usize, count: usize) -> String {
    if count == 1 {
        format!("line {}", first + 1)
    } else {
        format!("lines {}-{}", first + 1, first + count)
    }
}
/// The 1-based line of the byte at `index`.
fn line_of(content: &str, index: usize) -> usize {
    content[..index]
        .bytes()
        .filter(|&byte| byte == b'\n')
        .count()
        + 1
}
