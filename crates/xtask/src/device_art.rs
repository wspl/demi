//! `xtask device-art` (`crates/plugin-mobile/command/src/android/art.rs`):
//! pins the Android device art a Host downloads from AOSP.
//!
//! It reads AOSP's `device-art-resources` at a commit, the head of Android
//! Studio's main branch unless one is named, downloads each device's folder
//! as an archive, and writes the commit and each file's SHA-256 into the
//! mobile command package's pin. A folder is a device's when it, or one of
//! its posture folders, holds a `layout`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use tokio_util::sync::CancellationToken;

/// Where AOSP serves Android Studio's repository.
const REPOSITORY: &str = "https://android.googlesource.com/platform/tools/adt/idea";
/// The branch whose head is pinned when no commit is named.
const BRANCH: &str = "refs/heads/mirror-goog-studio-main";
/// The art's folder in the repository; the pin's `ROOT` says it too.
const ROOT: &str = "artwork/resources/device-art-resources";
/// The pin, beside the code that reads it.
const PIN: &str = "crates/plugin-mobile/command/src/android/art_pin.rs";

#[derive(clap::Args)]
pub struct Options {
    /// The commit to pin; the branch's head when none.
    #[arg(long)]
    commit: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{url}: {message}")]
    Fetch { url: String, message: String },
    #[error("{url} answers what does not read: {message}")]
    Unreadable { url: String, message: String },
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// A commit as Gitiles describes it.
#[derive(Deserialize)]
struct Commit {
    commit: String,
}

/// A tree as Gitiles lists it, recursively.
#[derive(Deserialize)]
struct Tree {
    entries: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    #[serde(rename = "type")]
    kind: String,
    name: String,
}

pub fn run(options: Options) -> Result<(), Error> {
    let path = crate::repository().join(PIN);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| Error::Io { path: path.clone(), source })?;
    let pin = runtime.block_on(pin(options.commit))?;
    std::fs::write(&path, pin).map_err(|source| Error::Io { path: path.clone(), source })?;
    println!("Wrote {}", path.display());
    Ok(())
}

/// The pin's source for `commit`, or the branch's head.
async fn pin(commit: Option<String>) -> Result<String, Error> {
    let client = demi_shared_artifacts::download_client().map_err(|error| Error::Fetch {
        url: REPOSITORY.into(),
        message: error.to_string(),
    })?;
    let commit = match commit {
        Some(commit) => commit,
        None => json::<Commit>(&client, &format!("{REPOSITORY}/+/{BRANCH}?format=JSON")).await?.commit,
    };
    let tree: Tree = json(&client, &format!("{REPOSITORY}/+/{commit}/{ROOT}?format=JSON&recursive=1")).await?;
    let mut folders: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for entry in tree.entries.iter().filter(|entry| entry.kind == "blob") {
        if let Some((device, file)) = entry.name.split_once('/') {
            folders.entry(device).or_default().push(file);
        }
    }
    // A device's folder holds a layout, or a posture folder each of which does.
    folders.retain(|_, files| {
        files
            .iter()
            .any(|file| *file == "layout" || file.split_once('/').is_some_and(|(_, name)| name == "layout"))
    });
    let mut source = format!(
        "//! The Android device art a Host downloads (`art.rs`): AOSP's commit and\n\
         //! the SHA-256 of each file of each device's folder. Written by\n\
         //! `xtask device-art`; a new device's art comes with a new pin.\n\
         \n\
         /// The commit of AOSP's `platform/tools/adt/idea` the art is read at.\n\
         pub const COMMIT: &str = \"{commit}\";\n\
         \n\
         /// Each device's files, by their paths in its folder, and their SHA-256.\n\
         pub const DEVICES: &[(&str, &[(&str, &str)])] = &[\n"
    );
    for device in folders.keys() {
        let url = format!("{REPOSITORY}/+archive/{commit}/{ROOT}/{device}.tar.gz");
        let files = archive(&fetch(&client, &url).await?).map_err(|message| Error::Unreadable { url: url.clone(), message })?;
        source.push_str(&format!("    (\"{device}\", &[\n"));
        for (file, bytes) in &files {
            let sha256: String = Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect();
            source.push_str(&format!("        (\"{file}\", \"{sha256}\"),\n"));
        }
        source.push_str("    ]),\n");
        eprintln!("{device}: {} files", files.len());
    }
    source.push_str("];\n");
    Ok(source)
}

/// The files of a folder's archive, as Gitiles makes it, by their paths.
fn archive(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut files = BTreeMap::new();
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(bytes));
    for entry in archive.entries().map_err(|error| error.to_string())? {
        let mut entry = entry.map_err(|error| error.to_string())?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path().map_err(|error| error.to_string())?.to_string_lossy().into_owned();
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut bytes).map_err(|error| error.to_string())?;
        files.insert(path, bytes);
    }
    Ok(files)
}

/// The document Gitiles answers at `url`, after the prefix that keeps a
/// browser from running it.
async fn json<T: serde::de::DeserializeOwned>(client: &reqwest::Client, url: &str) -> Result<T, Error> {
    let bytes = fetch(client, url).await?;
    let body = bytes.strip_prefix(b")]}'").unwrap_or(&bytes);
    serde_json::from_slice(body).map_err(|error| Error::Unreadable {
        url: url.into(),
        message: error.to_string(),
    })
}

/// The body Gitiles answers at `url`. Gitiles answers 429 to a client that
/// asks often, so a refusal waits and asks again, a few times.
async fn fetch(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, Error> {
    let failed = |error: demi_shared_artifacts::Error| Error::Fetch {
        url: url.into(),
        message: error.to_string(),
    };
    let mut attempt = 0;
    loop {
        let download = demi_shared_artifacts::Download::new(url);
        match demi_shared_artifacts::download_bytes(client, download, &CancellationToken::new()).await {
            Ok((received, bytes)) => {
                received.plain().map_err(failed)?;
                return Ok(bytes);
            }
            Err(error) if attempt < 6 && error.to_string().contains("429") => {
                attempt += 1;
                tokio::time::sleep(std::time::Duration::from_secs(10 * attempt)).await;
            }
            Err(error) => return Err(failed(error)),
        }
    }
}
