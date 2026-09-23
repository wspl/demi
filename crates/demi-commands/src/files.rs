//! The `file.*` operations: reading a file, and creating, editing or patching
//! files one mutation at a time.

use std::{
    fs,
    path::{Path, PathBuf},
};

use bytes::Bytes;
use demi_artifact::{Mode, Permissions, Publication};
use demi_builtin_protocol::{
    DecodeError,
    file::{CreateArgs, EditArgs, FileOperation, PatchArgs, ReadArgs},
};
use demi_command_service::{
    InvocationContext, ServiceError,
    edits::{Recorder, Recording},
    protocol::{CommandError, Completion},
};
use demi_gates::SerialGate;
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
    Publication(#[from] demi_artifact::Error),
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
    operation: FileOperation,
    mutations: SerialGate,
) -> Result<Completion, ServiceError> {
    let result = match operation {
        FileOperation::Read(args) => read(&context, &args).await,
        FileOperation::Create(args) => {
            mutate(&context, &mutations, move |cwd, cancellation, recording| {
                create(cwd, args, cancellation, recording)
            })
            .await
        }
        FileOperation::Edit(args) => {
            mutate(&context, &mutations, move |cwd, cancellation, recording| {
                edit(cwd, &args, cancellation, recording)
            })
            .await
        }
        FileOperation::Patch(PatchArgs { patch }) => {
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

async fn read(context: &InvocationContext, args: &ReadArgs) -> Result<(), FileError> {
    let path = demi_command_service::paths::resolve(&context.request.cwd, &args.path)?;
    let mut file = tokio::fs::File::open(path).await?;
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
    demi_artifact::publish_bytes_blocking(&destination, bytes, publication)?;
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
    Ok(demi_command_service::paths::resolve(cwd, path)?)
}

fn edit(
    cwd: &str,
    args: &EditArgs,
    cancellation: &CancellationToken,
    recording: Option<&mut Recording>,
) -> Result<String, FileError> {
    let path = resolve(cwd, &args.path)?;
    let content = fs::read_to_string(&path)?;
    let matches: Vec<_> = content
        .match_indices(&args.old)
        .map(|(index, _)| index)
        .collect();
    check_cancelled(cancellation)?;
    let index = if let Some(occurrence) = args.occurrence {
        *matches
            .get(occurrence - 1)
            .ok_or(FileError::OccurrenceOutOfRange(occurrence))?
    } else if let Some(context) = args.context {
        let mut ranked: Vec<_> = matches
            .iter()
            .map(|&index| (line_of(&content, index).abs_diff(context), index))
            .collect();
        ranked.sort_unstable();
        let &(distance, index) = ranked.first().ok_or(FileError::NoMatchNearContext)?;
        if ranked.get(1).is_some_and(|next| next.0 == distance) {
            let candidates = matches
                .iter()
                .enumerate()
                .map(|(occurrence, &index)| {
                    format!(
                        "occurrence {} at line {}",
                        occurrence + 1,
                        line_of(&content, index)
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            return Err(FileError::AmbiguousContext {
                context,
                candidates,
            });
        }
        index
    } else {
        match matches.as_slice() {
            [index] => *index,
            [] => return Err(FileError::NoMatch(args.path.clone())),
            _ => return Err(FileError::MultipleMatches(args.path.clone())),
        }
    };
    if args.old == args.new {
        return Ok(format!("Edited {}\n", args.path));
    }
    let mut updated = content;
    updated.replace_range(index..index + args.old.len(), &args.new);
    check_cancelled(cancellation)?;
    if let Some(recording) = recording {
        recording.track(&path);
    }
    atomic_write(&path, updated.as_bytes(), false)?;
    Ok(format!("Edited {}\n", args.path))
}

/// The 1-based line of the byte at `index`.
fn line_of(content: &str, index: usize) -> usize {
    content[..index].bytes().filter(|&byte| byte == b'\n').count() + 1
}
