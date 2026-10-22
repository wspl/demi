//! `xtask vendor diff` (`ARCHITECTURE.md`): how
//! each vendored crate differs from the upstream release it was taken from.
//! It downloads that release from crates.io through `artifact`, checks it
//! against the SHA-256 the crates.io index records, and compares it with the
//! vendored copy file by file, so an upgrade or a review sees every local
//! change, not only the ones `[package.metadata.demi]` describes.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use similar::{ChangeTag, TextDiff};
use tokio_util::sync::CancellationToken;

/// Where crates.io serves each release's `.crate` archive.
const ARCHIVES: &str = "https://static.crates.io/crates";
/// Where crates.io serves its sparse index.
const INDEX: &str = "https://index.crates.io";
/// The most bytes a crate's index file or archive may have.
///
/// Bounded because the bytes are read from the network into memory and a server
/// may send without end.
const DOWNLOAD_BYTES: u64 = 64 * 1024 * 1024;

/// The upstream files and directories a vendored copy leaves out: packaging
/// residue, a lockfile Cargo ignores in a dependency, upstream's CI, and
/// targets a dependency never builds.
const LEFT_OUT: [&str; 8] = [
    ".cargo-ok",
    "Cargo.lock",
    "Cargo.toml.orig",
    ".github",
    "tests",
    "benches",
    "fuzz",
    "examples",
];

#[derive(clap::Subcommand)]
pub enum Command {
    /// Lists each vendored crate's changes from its upstream release.
    Diff {
        /// Prints the unified diff of this crate instead of every crate's summary.
        #[arg(long, value_name = "CRATE")]
        patch: Option<String>,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Artifact(#[from] demi_shared_artifacts::Error),
    #[error("cargo metadata: {0}")]
    Metadata(String),
    #[error("the crates.io index of {name}: {reason}")]
    Index { name: String, reason: String },
    #[error("{name} {version} does not match the SHA-256 the crates.io index records")]
    Checksum { name: String, version: String },
    #[error("no vendored crate is named {0}")]
    Unknown(String),
}

/// One line of a crate's file in the crates.io sparse index.
#[derive(Deserialize)]
struct IndexEntry {
    vers: String,
    cksum: String,
}

/// A vendored crate: its upstream release and its directory.
struct Vendored {
    name: String,
    version: String,
    directory: PathBuf,
}

/// How a vendored copy differs from its upstream release.
#[derive(Debug, Default, PartialEq, Eq)]
struct Changes {
    /// Files in both whose contents differ, with the lines added and removed.
    changed: Vec<(PathBuf, usize, usize)>,
    /// Files only the vendored copy has.
    added: Vec<PathBuf>,
    /// Files only upstream has, other than those [`LEFT_OUT`] names.
    removed: Vec<PathBuf>,
    /// Upstream files under [`LEFT_OUT`] that the copy leaves out.
    left_out: usize,
}

pub fn run(command: Command) -> Result<(), Error> {
    let Command::Diff { patch } = command;
    let repository = crate::repository();
    let mut vendored = vendored(&repository)?;
    if let Some(name) = &patch {
        vendored.retain(|crate_| &crate_.name == name);
        if vendored.is_empty() {
            return Err(Error::Unknown(name.clone()));
        }
    }
    crate::interruptible(|cancel| async move {
        let client = demi_shared_artifacts::download_client()?;
        let scratch = tempfile::tempdir()?;
        for crate_ in &vendored {
            let upstream = upstream(&client, crate_, scratch.path(), &cancel).await?;
            let relative = crate_
                .directory
                .strip_prefix(&repository)
                .unwrap_or(&crate_.directory);
            if patch.is_some() {
                print_patch(&upstream, &crate_.directory)?;
            } else {
                let changes = compare(&upstream, &crate_.directory)?;
                print_summary(&relative.display().to_string(), &crate_.version, &changes);
            }
        }
        Ok(())
    })?
}

/// The vendored crates, from the workspace's `cargo metadata`.
fn vendored(repository: &Path) -> Result<Vec<Vendored>, Error> {
    let vendor = repository.join("vendor");
    let mut vendored: Vec<Vendored> = crate::packages(repository)
        .map_err(Error::Metadata)?
        .into_iter()
        .filter(|package| package.manifest_path.starts_with(&vendor))
        .map(|package| Vendored {
            directory: package
                .manifest_path
                .parent()
                .expect("a manifest is in a directory")
                .to_owned(),
            name: package.name,
            version: package.version,
        })
        .collect();
    vendored.sort_by(|a, b| a.directory.cmp(&b.directory));
    Ok(vendored)
}

/// The sparse index's path of a crate's file.
fn index_path(name: &str) -> String {
    let name = name.to_lowercase();
    match name.len() {
        1 => format!("1/{name}"),
        2 => format!("2/{name}"),
        3 => format!("3/{}/{name}", &name[..1]),
        _ => format!("{}/{}/{name}", &name[..2], &name[2..4]),
    }
}

/// Downloads and unpacks `crate_`'s upstream release below `scratch`, after
/// checking it against the index, and returns its directory.
async fn upstream(
    client: &demi_shared_artifacts::Client,
    crate_: &Vendored,
    scratch: &Path,
    cancel: &CancellationToken,
) -> Result<PathBuf, Error> {
    let url = format!("{INDEX}/{}", index_path(&crate_.name));
    let (received, index) = demi_shared_artifacts::download_bytes(
        client,
        demi_shared_artifacts::Download {
            limit: DOWNLOAD_BYTES,
            ..demi_shared_artifacts::Download::new(&url)
        },
        cancel,
    )
    .await?;
    received.plain()?;
    let mut recorded = None;
    for line in index
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let entry: IndexEntry = serde_json::from_slice(line).map_err(|error| Error::Index {
            name: crate_.name.clone(),
            reason: error.to_string(),
        })?;
        if entry.vers == crate_.version {
            recorded = Some(entry.cksum);
        }
    }
    let recorded = recorded.ok_or_else(|| Error::Index {
        name: crate_.name.clone(),
        reason: format!("it lists no version {}", crate_.version),
    })?;
    let url = format!("{ARCHIVES}/{0}/{0}-{1}.crate", crate_.name, crate_.version);
    let path = scratch.join(format!("{}-{}.crate", crate_.name, crate_.version));
    let request = demi_shared_artifacts::Download {
        limit: DOWNLOAD_BYTES,
        ..demi_shared_artifacts::Download::new(&url)
    };
    demi_shared_artifacts::download(client, request, &path, cancel, |_, _| {})
        .await?
        .plain()?;
    let digest = demi_shared_artifacts::digest(&path, DOWNLOAD_BYTES, cancel).await?;
    let archive = tokio::fs::read(&path).await?;
    if digest.sha256 != recorded {
        return Err(Error::Checksum {
            name: crate_.name.clone(),
            version: crate_.version.clone(),
        });
    }
    tar::Archive::new(flate2::read::GzDecoder::new(archive.as_slice())).unpack(scratch)?;
    Ok(scratch.join(format!("{}-{}", crate_.name, crate_.version)))
}

/// Every file below `root`, relative to it.
fn files(root: &Path) -> std::io::Result<BTreeSet<PathBuf>> {
    let mut found = BTreeSet::new();
    let mut pending = vec![root.to_owned()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                pending.push(entry.path());
            } else {
                let path = entry.path();
                found.insert(path.strip_prefix(root).expect("below the root").to_owned());
            }
        }
    }
    Ok(found)
}

/// Whether upstream's `path` is one a vendored copy leaves out.
fn left_out(path: &Path) -> bool {
    path.components()
        .next()
        .is_some_and(|first| LEFT_OUT.iter().any(|name| first.as_os_str() == *name))
}

/// How `vendored` differs from `upstream`.
fn compare(upstream: &Path, vendored: &Path) -> std::io::Result<Changes> {
    let theirs = files(upstream)?;
    let ours = files(vendored)?;
    let mut changes = Changes::default();
    for path in theirs.difference(&ours) {
        if left_out(path) {
            changes.left_out += 1;
        } else {
            changes.removed.push(path.clone());
        }
    }
    changes.added = ours.difference(&theirs).cloned().collect();
    for path in theirs.intersection(&ours) {
        let before = std::fs::read(upstream.join(path))?;
        let after = std::fs::read(vendored.join(path))?;
        if before == after {
            continue;
        }
        let before = String::from_utf8_lossy(&before);
        let after = String::from_utf8_lossy(&after);
        let diff = TextDiff::from_lines(before.as_ref(), after.as_ref());
        let mut inserted = 0;
        let mut deleted = 0;
        for change in diff.iter_all_changes() {
            match change.tag() {
                ChangeTag::Insert => inserted += 1,
                ChangeTag::Delete => deleted += 1,
                ChangeTag::Equal => {}
            }
        }
        changes.changed.push((path.clone(), inserted, deleted));
    }
    Ok(changes)
}

fn print_summary(directory: &str, version: &str, changes: &Changes) {
    let inserted: usize = changes
        .changed
        .iter()
        .map(|(_, inserted, _)| inserted)
        .sum();
    let deleted: usize = changes.changed.iter().map(|(_, _, deleted)| deleted).sum();
    println!(
        "{directory} {version}: {} changed (+{inserted} -{deleted}), {} added, {} removed, {} left out",
        changes.changed.len(),
        changes.added.len(),
        changes.removed.len(),
        changes.left_out,
    );
    for (path, inserted, deleted) in &changes.changed {
        println!("  changed {} (+{inserted} -{deleted})", path.display());
    }
    for path in &changes.added {
        println!("  added   {}", path.display());
    }
    for path in &changes.removed {
        println!("  removed {}", path.display());
    }
}

/// Prints the unified diff of every changed, added and removed file.
fn print_patch(upstream: &Path, vendored: &Path) -> std::io::Result<()> {
    let changes = compare(upstream, vendored)?;
    let read = |root: &Path, path: &Path| -> std::io::Result<String> {
        Ok(String::from_utf8_lossy(&std::fs::read(root.join(path))?).into_owned())
    };
    let changed = changes.changed.iter().map(|(path, _, _)| path);
    for path in changed.chain(&changes.added).chain(&changes.removed) {
        let before = if upstream.join(path).is_file() {
            read(upstream, path)?
        } else {
            String::new()
        };
        let after = if vendored.join(path).is_file() {
            read(vendored, path)?
        } else {
            String::new()
        };
        let name = path.display().to_string();
        let diff = TextDiff::from_lines(&before, &after);
        print!(
            "{}",
            diff.unified_diff()
                .header(&format!("a/{name}"), &format!("b/{name}"))
        );
    }
    Ok(())
}
