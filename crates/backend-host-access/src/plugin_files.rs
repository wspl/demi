//! What the user's plugins reach on a Host (`plugins.md` § Host
//! directories, § Reading a conversation's files): reads of a
//! conversation's primary Host in the form that never wakes it, and the
//! plugins' directories, which each job of the user's installs before it
//! starts, once per runner connection.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};

use demi_backend_blobs::blobs::UserBlobs;
use demi_backend_remote_host::{DirectoryFile, Link, Look, LookAt, RemoteHost, WeakLink};
use demi_host_interface::{FileKind, HostError, HostErrorKind};
use demi_plugin_interface::{EntryKind, HostDirectory, HostEntry, HostFile, HostRead, PluginId};
use demi_shared_types::B64Bytes;
use demi_web_api_protocol::ids::{ConversationId, DeviceId};
use tokio_util::sync::CancellationToken;

use crate::HostShard;
use crate::access::{Attention, ConversationHost, HostAccessError, Refusal, Waits};

/// The Host directories of the user's plugins: every plugin with its set,
/// an empty one for a plugin the user has off, whose directories a Host
/// loses at its next installation.
pub type DirectorySets = Vec<(PluginId, Vec<HostDirectory>)>;

/// What the shard remembers of its users' Hosts' directories.
#[derive(Default)]
pub struct PluginInstalls {
    /// Per device, the connection its Host was last brought to the user's
    /// set on, and that set's revision; a new connection checks again.
    synced: RefCell<HashMap<DeviceId, (WeakLink, String)>>,
    /// One installation at a time, so two never share a temporary
    /// directory.
    turn: tokio::sync::Mutex<()>,
}

impl PluginInstalls {
    fn synced(&self, device: &DeviceId, link: &Link, revision: &str) -> bool {
        self.synced
            .borrow()
            .get(device)
            .is_some_and(|(synced, at)| synced.is(link) && at == revision)
    }
}

/// Why a read of a conversation's Host did not answer.
#[derive(Debug, thiserror::Error)]
pub enum ReadFilesError {
    /// The Host is not running, and a read never wakes it.
    #[error("the conversation's Host is not running")]
    NotRunning,
    #[error(transparent)]
    Access(HostAccessError),
}

impl From<HostAccessError> for ReadFilesError {
    fn from(error: HostAccessError) -> Self {
        match error {
            HostAccessError::Refused(Refusal::Stopped) => Self::NotRunning,
            HostAccessError::Host(error) if not_running(&error) => Self::NotRunning,
            error => Self::Access(error),
        }
    }
}

impl dyn HostShard + '_ {
    /// Reads `reads` on the conversation's primary Host, admitted as a look:
    /// a stopped Cloud or an offline device is not running and is not
    /// woken, the read is no activity, and a transition ends it instead of
    /// waiting for it. A path the Host cannot read answers as unreadable;
    /// the rest answer in the order of `reads`.
    pub async fn read_files(
        &self,
        id: &ConversationId,
        reads: &[HostRead],
        cancel: &CancellationToken,
    ) -> Result<Vec<HostFile>, ReadFilesError> {
        let access = self.admit_stream(id, Attention::Looks, cancel).await?;
        let waits = Waits::until_ended(cancel, &access.open.ended);
        let found = waits.wait(look(&access.host.host, reads)).await?;
        Ok(found.map_err(HostAccessError::Host)?)
    }

    /// Brings `host`, admitted on `device` for a job, to the user's
    /// directory sets: once per runner connection and set, it lists every
    /// plugin's directory on the Host with one request, removes what the
    /// sets no longer name with one more, and writes each directory that is
    /// missing with one request each. A failure fails the job.
    pub(crate) async fn install_directories(
        &self,
        device: &DeviceId,
        host: &ConversationHost,
    ) -> Result<(), HostError> {
        let Some(link) = self.devices().link(device) else {
            // The job's own start reports the connection's end.
            return Ok(());
        };
        let sets = self
            .directory_sets()
            .await
            .map_err(|error| HostError::failed(None, error))?;
        let revision = revision_of(&sets);
        let installs = self.plugin_installs();
        if installs.synced(device, &link, &revision) {
            return Ok(());
        }
        let _turn = installs.turn.lock().await;
        if installs.synced(device, &link, &revision) {
            return Ok(());
        }
        let home = host.home.as_deref().ok_or_else(|| {
            HostError::failed(
                None,
                "the Host reported no home directory for the plugins' files",
            )
        })?;
        sync_directories(&host.host, home, &sets, &self.blobs()).await?;
        installs
            .synced
            .borrow_mut()
            .insert(device.clone(), (link.downgrade(), revision));
        Ok(())
    }
}

/// Brings the plugins' directories on `host` to `sets`: every plugin's
/// directory listed with one request, what no set names removed with one,
/// and each directory missing written with one, all at once.
async fn sync_directories(
    host: &RemoteHost,
    home: &str,
    sets: &DirectorySets,
    blobs: &UserBlobs,
) -> Result<(), HostError> {
    let base = |plugin: &PluginId| format!("{home}/.demi/plugins/{plugin}");
    let listed: Vec<LookAt> = sets
        .iter()
        .map(|(plugin, _)| LookAt {
            path: base(plugin),
            limit: 0,
        })
        .collect();
    let found = if listed.is_empty() {
        Vec::new()
    } else {
        host.look(&listed).await?
    };
    let mut removed = Vec::new();
    let mut missing = Vec::new();
    for ((plugin, set), look) in sets.iter().zip(found) {
        let present: Vec<String> = match look {
            Look::Directory(entries) => entries.into_iter().map(|entry| entry.name).collect(),
            Look::Missing => Vec::new(),
            Look::Unreadable(error) => return Err(error),
            Look::File { .. } | Look::Other => {
                return Err(HostError::failed(
                    None,
                    format!("{} is not a directory", base(plugin)),
                ));
            }
        };
        let wanted: Vec<(String, &HostDirectory)> = set
            .iter()
            .map(|directory| (directory.host_name(), directory))
            .collect();
        let names: BTreeSet<&str> = wanted.iter().map(|(name, _)| name.as_str()).collect();
        removed.extend(
            present
                .iter()
                .filter(|name| !names.contains(name.as_str()))
                .map(|name| format!("{}/{name}", base(plugin))),
        );
        missing.extend(
            wanted
                .into_iter()
                .filter(|(name, _)| !present.contains(name))
                .map(|(name, directory)| (format!("{}/{name}", base(plugin)), directory)),
        );
    }
    if !removed.is_empty() {
        host.remove_all(&removed).await?;
    }
    let installs = missing
        .into_iter()
        .map(|(path, directory)| async move { install(host, blobs, &path, directory).await });
    futures_util::future::try_join_all(installs).await?;
    Ok(())
}

/// Writes `directory` at `path` with one request: its files read-only,
/// each keeping its executable bit, and every directory in it read-only
/// too, so a directory that exists is complete.
async fn install(
    host: &RemoteHost,
    blobs: &UserBlobs,
    path: &str,
    directory: &HostDirectory,
) -> Result<(), HostError> {
    let mut files = Vec::with_capacity(directory.files.len());
    for file in &directory.files {
        let bytes = blobs
            .get(&file.blob)
            .await
            .map_err(|error| HostError::failed(None, error.to_string()))?
            .ok_or_else(|| {
                HostError::failed(
                    None,
                    format!("the file {} of {path} is not stored", file.path),
                )
            })?;
        files.push(DirectoryFile {
            path: file.path.clone(),
            mode: if file.executable { 0o555 } else { 0o444 },
            bytes,
        });
    }
    host.write_directory(path, files, 0o555).await
}

/// What each of `reads` finds on `host`, with one request to the Host
/// (`runner.md` § Host operations): a failure on the Host about a path
/// answers for the path; any other, such as the connection's end, fails
/// the read.
async fn look(host: &RemoteHost, reads: &[HostRead]) -> Result<Vec<HostFile>, HostError> {
    let paths: Vec<LookAt> = reads
        .iter()
        .map(|read| LookAt {
            path: read.path.clone(),
            limit: read.limit,
        })
        .collect();
    let found = host.look(&paths).await?;
    Ok(found
        .into_iter()
        .map(|look| match look {
            Look::Missing => HostFile::Missing,
            Look::Directory(entries) => HostFile::Directory {
                entries: entries
                    .into_iter()
                    .map(|entry| HostEntry {
                        name: entry.name,
                        kind: entry_kind(entry.kind),
                    })
                    .collect(),
            },
            Look::File { bytes, size } => HostFile::File {
                bytes: B64Bytes::new(bytes),
                size,
            },
            Look::Other => HostFile::Other,
            Look::Unreadable(error) => HostFile::Unreadable {
                message: error.message,
            },
        })
        .collect())
}

fn entry_kind(kind: FileKind) -> EntryKind {
    match kind {
        FileKind::File => EntryKind::File,
        FileKind::Directory => EntryKind::Directory,
        FileKind::Symlink => EntryKind::Symlink,
        FileKind::CharacterDevice | FileKind::Fifo | FileKind::Other => EntryKind::Other,
    }
}

/// Whether `error` says the Host is not reachable now.
fn not_running(error: &HostError) -> bool {
    matches!(
        error.kind,
        HostErrorKind::Offline | HostErrorKind::Unavailable
    )
}

/// The revision of the user's sets, which a Host is brought to once per
/// connection: each plugin with the names its Host directory holds.
fn revision_of(sets: &DirectorySets) -> String {
    let mut revision = String::new();
    for (plugin, set) in sets {
        revision.push_str(plugin.as_str());
        for directory in set {
            revision.push(' ');
            revision.push_str(&directory.host_name());
        }
        revision.push('\n');
    }
    revision
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use demi_backend_remote_host::testing::{FixtureOptions, RunnerFixture, answered_requests};
    use tokio::sync::mpsc;

    use super::*;

    // About a tenth of a second: a real runner process looks at the paths.
    #[tokio::test(flavor = "local")]
    async fn a_read_of_several_paths_is_one_request_to_the_host_and_answers_each() {
        let (tap, mut replies) = mpsc::channel(1 << 10);
        let fixture = RunnerFixture::start(FixtureOptions {
            tap: Some(tap),
            ..FixtureOptions::default()
        })
        .await;
        let skills = fixture.home_dir().join("skills");
        std::fs::create_dir_all(skills.join("review")).unwrap();
        std::fs::write(skills.join("review/SKILL.md"), "---\nname: review\n").unwrap();
        std::os::unix::fs::symlink("review", skills.join("linked")).unwrap();
        std::fs::write(skills.join("plain"), "not a directory").unwrap();
        let locked = skills.join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        answered_requests(&mut replies);

        let home = fixture.home();
        let read = |path: &str, limit: u64| HostRead {
            path: format!("{home}/skills/{path}"),
            limit,
        };
        let reads = [
            read("review", 0),
            read("review/SKILL.md", 8),
            read("linked/SKILL.md", 1024),
            read("missing", 0),
            read("plain/SKILL.md", 0),
            read("locked", 0),
        ];
        let found = look(&fixture.host(), &reads).await.unwrap();

        assert_eq!(answered_requests(&mut replies), 1, "one request reads them all");
        assert_eq!(
            found[0],
            HostFile::Directory {
                entries: vec![HostEntry {
                    name: "SKILL.md".into(),
                    kind: EntryKind::File,
                }],
            }
        );
        // A file's first bytes, up to each read's limit, with its size.
        assert_eq!(
            found[1],
            HostFile::File {
                bytes: B64Bytes::from(b"---\nname".to_vec()),
                size: 17,
            }
        );
        assert_eq!(
            found[2],
            HostFile::File {
                bytes: B64Bytes::from(b"---\nname: review\n".to_vec()),
                size: 17,
            }
        );
        assert_eq!(found[3], HostFile::Missing);
        assert_eq!(found[4], HostFile::Missing);
        assert!(
            matches!(&found[5], HostFile::Unreadable { message } if message.contains("ermission")),
            "{:?}",
            found[5]
        );
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        fixture.stop().await;
    }

    // About a fifth of a second: a real runner process lists, removes and
    // writes the directories.
    #[tokio::test(flavor = "local")]
    async fn plugin_directories_are_listed_removed_and_each_written_with_one_request() {
        let (tap, mut replies) = mpsc::channel(1 << 10);
        let fixture = RunnerFixture::start(FixtureOptions {
            tap: Some(tap),
            ..FixtureOptions::default()
        })
        .await;
        std::fs::create_dir(fixture.home_dir().join("store")).unwrap();
        let stores = demi_backend_blobs::blobs::BlobStores::new(
            std::sync::Arc::new(
                demi_backend_blobs::local::LocalObjects::new(&fixture.home_dir().join("store"))
                    .unwrap(),
            ),
            std::sync::Arc::new(demi_shared_types::SystemClock),
        );
        let blobs = stores.for_user(&demi_web_api_protocol::ids::UserId::try_from("u1").unwrap());
        let file = async |path: &str, text: &'static str, executable: bool| demi_plugin_interface::DirectoryFile {
            path: path.into(),
            executable,
            blob: blobs.put(bytes::Bytes::from_static(text.as_bytes())).await.unwrap(),
        };
        let kept = HostDirectory {
            name: "kept".into(),
            files: vec![file("SKILL.md", "kept", false).await],
        };
        let review = HostDirectory {
            name: "review".into(),
            files: vec![
                file("SKILL.md", "---\nname: review\n", false).await,
                file("scripts/check.sh", "#!/bin/sh\necho ok\n", true).await,
            ],
        };
        let plugins = fixture.home_dir().join(".demi/plugins");
        let installed = plugins.join("skills").join(kept.host_name());
        std::fs::create_dir_all(&installed).unwrap();
        // A directory the sets no longer name, read-only as installed.
        let stale = plugins.join("skills/stale-0123456789ab");
        std::fs::create_dir_all(stale.join("deep")).unwrap();
        std::fs::set_permissions(stale.join("deep"), std::fs::Permissions::from_mode(0o555)).unwrap();
        std::fs::set_permissions(&stale, std::fs::Permissions::from_mode(0o555)).unwrap();
        let sets: DirectorySets = vec![
            (PluginId::try_from("skills").unwrap(), vec![kept.clone(), review.clone()]),
            (PluginId::try_from("browser").unwrap(), Vec::new()),
        ];
        answered_requests(&mut replies);

        sync_directories(&fixture.host(), fixture.home(), &sets, &blobs)
            .await
            .unwrap();

        assert_eq!(
            answered_requests(&mut replies),
            3,
            "one listing, one removal and one write"
        );
        assert!(!stale.exists());
        let written = plugins.join("skills").join(review.host_name());
        let mode = |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(std::fs::read_to_string(written.join("scripts/check.sh")).unwrap(), "#!/bin/sh\necho ok\n");
        assert_eq!(std::fs::read_to_string(written.join("SKILL.md")).unwrap(), "---\nname: review\n");
        assert_eq!(
            [
                mode(&written),
                mode(&written.join("scripts")),
                mode(&written.join("SKILL.md")),
                mode(&written.join("scripts/check.sh")),
            ],
            [0o555, 0o555, 0o444, 0o555]
        );
        let mut names: Vec<_> = std::fs::read_dir(plugins.join("skills"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        let mut expected = vec![kept.host_name(), review.host_name()];
        expected.sort();
        assert_eq!(names, expected, "no temporary directory is left");
        // Read-only as it is, the test's home goes with the fixture.
        for directory in [written.join("scripts"), written] {
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        fixture.stop().await;
    }
}
