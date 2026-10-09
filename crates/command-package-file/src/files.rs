//! The `file.*` operations: reading a file, and creating, editing or patching
//! files one mutation at a time.

use std::{
    fs,
    path::{Path, PathBuf},
};

use bytes::Bytes;
use demi_command_package_file_protocol::{Operation, PatchArgs, ReadArgs};
use demi_command_protocol::{
    CommandError, Completion, MAX_MEDIUM_BYTES, StdoutTarget, is_text, sniff_media_type,
};
use demi_command_sdk::{
    InvocationContext, ServiceError,
    errors::reason,
    edits::{Recorder, Recording},
};
use demi_shared_artifacts::{Mode, Permissions, Publication};
use demi_shared_gates::SerialGate;
use demi_shared_types::DecodeError;
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;

use crate::{
    edit,
    patch::{self, PatchError},
};

/// How much of a file one read sends on.
const READ_BYTES: usize = 64 * 1024;

/// Why a file operation failed; its message is what the agent reads.
#[derive(Debug, thiserror::Error)]
pub enum FileError {
    #[error("Command cancelled")]
    Cancelled,
    #[error(transparent)]
    Arguments(#[from] DecodeError),
    #[error("{}", reason(.0))]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Publication(#[from] demi_shared_artifacts::Error),
    #[error(
        "a binary file ({size} bytes) that is not an image or video; redirect it to copy it: {copy}"
    )]
    Binary { size: usize, copy: String },
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
    #[error("{name}, block {block}: its SEARCH matches no lines. {closest}")]
    BlockNoMatch {
        block: usize,
        name: String,
        closest: String,
    },
    #[error(
        "{name}, block {block}: its SEARCH matches at {places}; add a line around it so it matches one place"
    )]
    BlockMatchesSeveral {
        block: usize,
        name: String,
        places: String,
    },
    #[error("{name}: blocks {first} and {second} overlap; make them one block")]
    BlocksOverlap {
        first: usize,
        second: usize,
        name: String,
    },
    #[error("{name}, block {block}: the file exists; a block with an empty SEARCH creates a new file")]
    FileExists { name: String, block: usize },
    #[error("{name}: the file does not exist; a block with an empty SEARCH creates it")]
    FileMissing { name: String },
    #[error(
        "{name}, block {block}: a block with an empty SEARCH creates the file, so it is the file's only block"
    )]
    CreateWithOthers { name: String, block: usize },
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
        Operation::Read(args) => return read(&context, &args).await,
        Operation::Edit(args) => {
            mutate(&context, &mutations, move |cwd, cancellation, recording| {
                edit::edit(cwd, &args, cancellation, recording)
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

/// Reads each file in order (`commands.md` § File commands). A file that
/// cannot be read writes `<command>: <path>: <reason>` to stderr, as `cat`
/// does, and the next file is read; the command exits 1 when any failed.
async fn read(context: &InvocationContext, args: &ReadArgs) -> Result<Completion, ServiceError> {
    let mut failed = false;
    for path in &args.path {
        match read_one(context, path).await {
            Ok(()) => {}
            Err(FileError::Cancelled) => return Err(ServiceError::Cancelled),
            Err(FileError::Output(error)) => return Err(error),
            Err(error) => {
                failed = true;
                context
                    .output
                    .stderr(Bytes::from(format!(
                        "{}: {path}: {error}\n",
                        context.request.command
                    )))
                    .await?;
            }
        }
    }
    Ok(Completion {
        exit_code: u8::from(failed),
        error: None,
    })
}

/// Streams the file at `path` to stdout. A job's command reading a regular
/// file of at most 16 MiB whose bytes are an image or video a model reads
/// returns it as a medium instead, and one that is neither text nor a
/// medium fails, since its bytes would mean nothing in the job's output.
async fn read_one(context: &InvocationContext, path: &str) -> Result<(), FileError> {
    let resolved = demi_command_sdk::paths::resolve(&context.request.cwd, path)?;
    let mut file = tokio::fs::File::open(resolved).await?;
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
        } else if context.request.stdout == Some(StdoutTarget::Job) && !is_text(&bytes) {
            return Err(FileError::Binary {
                size: bytes.len(),
                copy: copy_command(&context.request.command, path),
            });
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

/// The command line that copies the file at `path` by redirecting
/// `command`'s stdout, such as `demi file read data.bin > copy.bin`.
fn copy_command(command: &str, path: &str) -> String {
    let copy = match Path::new(path).extension().and_then(|extension| extension.to_str()) {
        Some(extension) => format!("copy.{extension}"),
        None => "copy".to_owned(),
    };
    let quoted = shlex::try_quote(path).map_or_else(|_| path.into(), |quoted| quoted.into_owned());
    format!("{command} {quoted} > {copy}")
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

pub(crate) fn resolve(cwd: &str, path: &str) -> Result<PathBuf, FileError> {
    Ok(demi_command_sdk::paths::resolve(cwd, path)?)
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
