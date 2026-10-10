//! The `file.*` operations: showing the model a file, and creating, editing
//! or patching files one mutation at a time.

use std::{
    fs,
    path::{Path, PathBuf},
};

use bytes::Bytes;
use demi_command_package_file_protocol::{EditArgsError, Operation, PatchArgs, ViewArgs};
use demi_command_protocol::{
    CommandError, Completion, MAX_MEDIUM_BYTES, begins_as_text, is_text, sniff_media_type,
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
    edit, facts,
    patch::{self, PatchError},
};

/// How much of a file over the medium bound is read to tell what it is.
const OPENING_BYTES: usize = 64 * 1024;

/// What the view lines name stdin by.
const STDIN: &str = "stdin";

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
    #[error("a text file; read it with cat {0}")]
    Text(String),
    #[error("not an image, a video or a PDF ({0} bytes)")]
    NotMedia(u64),
    #[error("{:.1} MiB; a medium is at most 16 MiB", *.0 as f64 / (1024.0 * 1024.0))]
    TooLarge(u64),
    #[error("this conversation's model, {model}, does not read {media_type} in a tool result")]
    NotViewable { model: String, media_type: String },
    #[error(
        "it is not known which files this conversation's model, {0}, reads; its provider entry can name them"
    )]
    UnknownTypes(String),
    #[error("a job on another Host shows the model nothing; pipe its bytes into demi file view in your own script")]
    NoModel,
    #[error("File has no parent directory")]
    NoParent,
    #[error("Occurrence {0} is out of range")]
    OccurrenceOutOfRange(usize),
    #[error("No match found")]
    NoMatchNearContext,
    #[error("Context line {context} is ambiguous: {candidates}")]
    AmbiguousContext { context: usize, candidates: String },
    #[error(
        "{}: --old is not in the file; it must match the file's text exactly, whitespace included, and nothing was written. {closest}",
        shown(.path).display()
    )]
    OldNoMatch { path: PathBuf, closest: String },
    #[error(
        "{}: --old occurs {count} times, at {places}; choose one with --occurrence or --context, or include more of the text, and nothing was written",
        shown(.path).display()
    )]
    OldMatchesSeveral {
        path: PathBuf,
        count: usize,
        places: String,
    },
    /// A file an edit reads, named by its full path, as the system words
    /// why it cannot be read.
    #[error("{}: {}", shown(.path).display(), reason(.error))]
    Unreadable { path: PathBuf, error: std::io::Error },
    #[error(
        "{}: block {block}: its SEARCH is not in the file; a SEARCH must match the file's text exactly, whitespace included, and nothing was written. {closest}",
        shown(.path).display()
    )]
    BlockNoMatch {
        block: usize,
        path: PathBuf,
        closest: String,
    },
    #[error(
        "{}: block {block}: its SEARCH holds only blank lines, which match too many places; include a line of text around it, and nothing was written",
        shown(.path).display()
    )]
    BlankSearch { block: usize, path: PathBuf },
    #[error(
        "{}: block {block}: its SEARCH occurs {count} times, at {places}; include more of the text around it so it occurs once, and nothing was written",
        shown(.path).display()
    )]
    BlockMatchesSeveral {
        block: usize,
        path: PathBuf,
        count: usize,
        places: String,
    },
    #[error(
        "{}: blocks {first} and {second} overlap; make them one block, and nothing was written",
        shown(.path).display()
    )]
    BlocksOverlap {
        first: usize,
        second: usize,
        path: PathBuf,
    },
    #[error(
        "{}: block {block}: the file exists; a block with an empty SEARCH creates a new file, and nothing was written",
        shown(.path).display()
    )]
    FileExists { path: PathBuf, block: usize },
    #[error(
        "{}: block {block}: a block with an empty SEARCH creates the file, so it is the file's only block, and nothing was written",
        shown(.path).display()
    )]
    CreateWithOthers { path: PathBuf, block: usize },
    /// `file.edit`'s arguments are refused, a path they name shown as
    /// [`edit_args`] resolves it.
    #[error(transparent)]
    EditArgs(EditArgsError),
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
        Operation::View(args) => return view(context, &args).await,
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

/// Shows the model each path, or stdin for `-` and when it names none, in
/// order (`runtime.md` § What `demi file view` shows). One that fails writes
/// `<command>: <path>: <reason>` to stderr, and the next one is viewed; the
/// command prints nothing to stdout and exits 1 when any failed.
async fn view(mut context: InvocationContext, args: &ViewArgs) -> Result<Completion, ServiceError> {
    let command = context.request.command.clone();
    let named = args.path.as_deref().unwrap_or_default();
    // Stdin that is the job's own input would wait until the job ends.
    if named.is_empty() && context.request.live_input == Some(true) {
        context
            .output
            .stderr(Bytes::from(format!(
                "{command}: no file named, and stdin is the job's input; name a file or pipe one in\n"
            )))
            .await?;
        return Ok(Completion {
            exit_code: 1,
            error: None,
        });
    }
    let paths: Vec<&str> = if named.is_empty() {
        vec!["-"]
    } else {
        named.iter().map(String::as_str).collect()
    };
    let mut failed = false;
    for path in paths {
        let (shown, result) = if path == "-" {
            (STDIN, view_stdin(&mut context).await)
        } else {
            (path, view_file(&context, path).await)
        };
        match result {
            Ok(()) => {}
            Err(FileError::Cancelled) => return Err(ServiceError::Cancelled),
            Err(FileError::Output(error)) => return Err(error),
            Err(error) => {
                failed = true;
                context
                    .output
                    .stderr(Bytes::from(format!("{command}: {shown}: {error}\n")))
                    .await?;
            }
        }
    }
    Ok(Completion {
        exit_code: u8::from(failed),
        error: None,
    })
}

/// Shows the model the file at `path`. A file over the medium bound is
/// judged by its first bytes, so a large text file still names `cat`.
async fn view_file(context: &InvocationContext, path: &str) -> Result<(), FileError> {
    let resolved = demi_command_sdk::paths::resolve(&context.request.cwd, path)?;
    let mut file = tokio::fs::File::open(resolved).await?;
    let size = file.metadata().await?.len();
    if size > MAX_MEDIUM_BYTES {
        let mut opening = vec![0; OPENING_BYTES];
        let count = tokio::select! {
            _ = context.cancellation.cancelled() => return Err(FileError::Cancelled),
            result = file.read(&mut opening) => result?,
        };
        return Err(oversized(path, &opening[..count], size));
    }
    let mut bytes = Vec::new();
    tokio::select! {
        _ = context.cancellation.cancelled() => return Err(FileError::Cancelled),
        result = file.read_to_end(&mut bytes) => result?,
    };
    let name = Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned());
    show(context, path, name, Bytes::from(bytes)).await
}

/// Shows the model what stdin holds. Stdin over the medium bound is read to
/// its end, so the writer is not cut off and the line names its size.
async fn view_stdin(context: &mut InvocationContext) -> Result<(), FileError> {
    if context.request.live_input == Some(true) {
        return Err(FileError::Io(std::io::Error::other(
            "stdin is the job's input; pipe a file in",
        )));
    }
    let mut bytes = Vec::new();
    let mut size = 0u64;
    loop {
        let chunk = tokio::select! {
            _ = context.cancellation.cancelled() => return Err(FileError::Cancelled),
            chunk = context.input.next() => chunk.map_err(FileError::Output)?,
        };
        let Some(chunk) = chunk else {
            break;
        };
        size += chunk.len() as u64;
        if size <= MAX_MEDIUM_BYTES || bytes.len() < OPENING_BYTES {
            bytes.extend_from_slice(&chunk);
        }
    }
    if size > MAX_MEDIUM_BYTES {
        return Err(oversized(STDIN, &bytes, size));
    }
    show(context, STDIN, None, Bytes::from(bytes)).await
}

/// Why `named`, `size` bytes beginning with `opening`, more than a medium
/// may be, is not shown: a text file names `cat`, and anything else its size.
fn oversized(named: &str, opening: &[u8], size: u64) -> FileError {
    if begins_as_text(opening) {
        FileError::Text(quoted(named))
    } else if sniff_media_type(opening).is_some() {
        FileError::TooLarge(size)
    } else {
        FileError::NotMedia(size)
    }
}

/// Returns `bytes`, named `named` in a line and `name` to the model when it
/// is a document's file name, as a medium of the job when they are a medium
/// the job's model reads in a tool result, with the facts its header gives.
async fn show(
    context: &InvocationContext,
    named: &str,
    name: Option<String>,
    bytes: Bytes,
) -> Result<(), FileError> {
    let Some(media_type) = sniff_media_type(&bytes) else {
        return Err(if !bytes.is_empty() && is_text(&bytes) {
            FileError::Text(quoted(named))
        } else {
            FileError::NotMedia(bytes.len() as u64)
        });
    };
    let Some(viewable) = &context.request.viewable else {
        return Err(FileError::NoModel);
    };
    let Some(media_types) = &viewable.media_types else {
        return Err(FileError::UnknownTypes(viewable.model.clone()));
    };
    if !media_types.iter().any(|viewed| viewed == media_type) {
        return Err(FileError::NotViewable {
            model: viewable.model.clone(),
            media_type: media_type.to_owned(),
        });
    }
    let facts = facts::facts(&bytes, media_type, name);
    context.output.medium(facts, bytes).await?;
    Ok(())
}

/// `path` as a shell word, as the line that names `cat` writes it.
fn quoted(path: &str) -> String {
    shlex::try_quote(path).map_or_else(|_| path.into(), |quoted| quoted.into_owned())
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

/// The full path an error names for the resolved `path`: its parent
/// directory as the system resolves it, `..` and links included, with the
/// file's name, when that directory exists; otherwise `path` itself.
pub(crate) fn shown(path: &Path) -> PathBuf {
    let canonical = path
        .parent()
        .zip(path.file_name())
        .and_then(|(parent, name)| Some(fs::canonicalize(parent).ok()?.join(name)));
    canonical.unwrap_or_else(|| path.to_owned())
}

/// The failure of `file.edit`'s refused arguments, a path they name
/// resolved against `cwd` and shown in full.
pub(crate) fn edit_args(cwd: &str, error: EditArgsError) -> FileError {
    FileError::EditArgs(match error {
        EditArgsError::Sections {
            path,
            block,
            search,
            replace,
        } => EditArgsError::Sections {
            // A path that does not resolve is named as given.
            path: resolve(cwd, &path)
                .map_or(path, |resolved| shown(&resolved).display().to_string()),
            block,
            search,
            replace,
        },
        other => other,
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
