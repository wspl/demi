use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use demi_command_service::protocol::Invocation;
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use crate::patch;

pub fn resolve_path(cwd: &str, path: &str) -> Result<PathBuf, String> {
    if path.is_empty() || path.contains('\0') {
        return Err("File path must be nonempty and contain no NUL byte".into());
    }
    let path = Path::new(path);
    Ok(if path.is_absolute() {
        path.to_owned()
    } else {
        Path::new(cwd).join(path)
    })
}

pub fn check_cancelled(cancellation: &CancellationToken) -> Result<(), String> {
    if cancellation.is_cancelled() {
        return Err("Command cancelled".into());
    }
    Ok(())
}

pub fn atomic_write(path: &Path, bytes: &[u8], create: bool) -> Result<(), String> {
    let destination = if !create && path.is_symlink() {
        fs::canonicalize(path).map_err(|error| error.to_string())?
    } else {
        path.to_owned()
    };
    let parent = destination.parent().ok_or("File has no parent directory")?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let mut builder = tempfile::Builder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // The OS applies the runner user's umask to newly created files.
        builder.permissions(fs::Permissions::from_mode(0o666));
    }
    let mut temporary = builder
        .tempfile_in(parent)
        .map_err(|error| error.to_string())?;
    if !create {
        let metadata = fs::metadata(&destination).map_err(|error| error.to_string())?;
        temporary
            .as_file()
            .set_permissions(metadata.permissions())
            .map_err(|error| error.to_string())?;
    }
    temporary
        .write_all(bytes)
        .map_err(|error| error.to_string())?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| error.to_string())?;
    if create {
        temporary
            .persist_noclobber(&destination)
            .map_err(|error| error.to_string())?;
    } else {
        temporary
            .persist(&destination)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub fn mutate(request: &Invocation, cancellation: &CancellationToken) -> Result<String, String> {
    check_cancelled(cancellation)?;
    match request.operation.as_str() {
        "file.create" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                path: String,
                content: String,
            }
            let args: Args =
                serde_json::from_value(request.args.clone()).map_err(|error| error.to_string())?;
            let path = resolve_path(&request.cwd, &args.path)?;
            check_cancelled(cancellation)?;
            atomic_write(&path, args.content.as_bytes(), true)?;
            Ok(format!("Created {}\n", args.path))
        }
        "file.edit" => edit(request, cancellation),
        "file.patch" => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                patch: String,
            }
            let args: Args =
                serde_json::from_value(request.args.clone()).map_err(|error| error.to_string())?;
            patch::apply(&request.cwd, &args.patch, cancellation)
        }
        _ => Err("Unknown builtin operation".into()),
    }
}

fn edit(request: &Invocation, cancellation: &CancellationToken) -> Result<String, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        path: String,
        old: String,
        new: String,
        occurrence: Option<usize>,
        context: Option<usize>,
    }
    let args: Args =
        serde_json::from_value(request.args.clone()).map_err(|error| error.to_string())?;
    if args.old.is_empty() || args.occurrence == Some(0) || args.context == Some(0) {
        return Err("Old text must be nonempty; occurrence and context must be positive".into());
    }
    let path = resolve_path(&request.cwd, &args.path)?;
    let content = fs::read_to_string(&path).map_err(|error| error.to_string())?;
    let matches: Vec<_> = content
        .match_indices(&args.old)
        .map(|(index, _)| index)
        .collect();
    check_cancelled(cancellation)?;
    let index = if let Some(occurrence) = args.occurrence {
        *matches
            .get(occurrence - 1)
            .ok_or_else(|| format!("Occurrence {occurrence} is out of range"))?
    } else if let Some(context) = args.context {
        let mut ranked: Vec<_> = matches
            .iter()
            .map(|&index| {
                let line = content[..index]
                    .bytes()
                    .filter(|&byte| byte == b'\n')
                    .count()
                    + 1;
                (line.abs_diff(context), index)
            })
            .collect();
        ranked.sort_unstable();
        let &(distance, index) = ranked.first().ok_or("No match found")?;
        if ranked.get(1).is_some_and(|next| next.0 == distance) {
            return Err(format!("Context line {context} is ambiguous"));
        }
        index
    } else {
        match matches.as_slice() {
            [index] => *index,
            [] => return Err(format!("No match found in {}", args.path)),
            _ => {
                return Err(format!(
                    "Multiple matches in {}; specify --occurrence or --context",
                    args.path
                ));
            }
        }
    };
    let mut updated = content;
    updated.replace_range(index..index + args.old.len(), &args.new);
    check_cancelled(cancellation)?;
    atomic_write(&path, updated.as_bytes(), false)?;
    Ok(format!("Edited {}\n", args.path))
}
