//! `xtask browser-release <version> --runtime <release>`
//! (`builds-and-releases.md` § Chrome for Testing): pins a Chrome for
//! Testing version and the Chrome runtime release Linux Hosts start it with.
//! It reads the version's official download metadata, downloads the archive
//! of each platform Demi supports through `artifact`, measures it, checks
//! that it holds the executable the record names, does the same for the
//! runtime release's archives, and writes the release record with
//! command-package-browser-protocol's type, which `demi-browser` installs from.

use std::collections::BTreeMap;

use demi_command_package_browser_protocol::release::{
    BrowserRelease, ChromeRuntime, RUNTIME_FONTS_ENTRY, RUNTIME_LIBRARIES_ENTRY, ReleasePlatform,
    RuntimeArchive,
};
use demi_shared_artifacts::{Mode, Permissions, Publication};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

/// Where Chrome for Testing publishes each version's downloads.
const METADATA: &str = "https://googlechromelabs.github.io/chrome-for-testing";
/// Where every official archive is.
const ARCHIVES: &str = "https://storage.googleapis.com/";
/// Where the Chrome runtime's releases are, each in a directory named by its
/// number (`builds-and-releases.md` § Chrome runtime).
const RUNTIME: &str = "https://github.com/wspl/demi-chrome-runtime/releases/download";
/// The runtime's fonts' archive, which every Linux target shares.
const RUNTIME_FONTS: &str = "fonts.tar.zst";
/// The pinned record, which `BrowserRelease::pinned` reads.
const RECORD: &str = "crates/command-package-browser-protocol/src/release/chrome.json";
/// The most bytes a version's metadata may have.
const METADATA_BYTES: u64 = 1024 * 1024;
/// The most bytes an archive may have.
const ARCHIVE_BYTES: u64 = 1024 * 1024 * 1024;

/// A platform Demi pins Chrome for: Chrome for Testing's name for it, its
/// target, the executable's path in its archive, and on Linux the runtime's
/// archive of its libraries.
struct Platform {
    name: &'static str,
    target: &'static str,
    executable: &'static str,
    libraries: Option<&'static str>,
}

/// Chrome for Testing publishes no Windows arm64 build.
const PLATFORMS: [Platform; 5] = [
    Platform {
        name: "mac-arm64",
        target: "aarch64-apple-darwin",
        executable: "chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
        libraries: None,
    },
    Platform {
        name: "mac-x64",
        target: "x86_64-apple-darwin",
        executable: "chrome-mac-x64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
        libraries: None,
    },
    Platform {
        name: "linux-arm64",
        target: "aarch64-unknown-linux-musl",
        executable: "chrome-linux-arm64/chrome",
        libraries: Some("libs-aarch64.tar.zst"),
    },
    Platform {
        name: "linux64",
        target: "x86_64-unknown-linux-musl",
        executable: "chrome-linux64/chrome",
        libraries: Some("libs-x86_64.tar.zst"),
    },
    Platform {
        name: "win64",
        target: "x86_64-pc-windows-msvc",
        executable: "chrome-win64/chrome.exe",
        libraries: None,
    },
];

#[derive(clap::Args)]
pub struct Options {
    /// Chrome's four-part version, such as 153.0.8010.36.
    #[arg(value_parser = version)]
    version: String,
    /// The Chrome runtime's release number, such as 1.
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    runtime: u32,
}

/// Parses the version argument by the rule of the record it goes into.
fn version(value: &str) -> Result<String, String> {
    let probe = BrowserRelease {
        version: value.to_owned(),
        platforms: Vec::new(),
        runtime: ChromeRuntime {
            release: 1,
            libraries: BTreeMap::new(),
            fonts: RuntimeArchive {
                url: format!("{RUNTIME}/1/{RUNTIME_FONTS}"),
                size: 1,
                sha256: "0".repeat(64),
            },
        },
    };
    let json = serde_json::to_string(&probe).map_err(|error| error.to_string())?;
    BrowserRelease::parse(&json).map_err(|error| error.to_string())?;
    Ok(value.to_owned())
}

/// Why a version could not be pinned.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the metadata of Chrome for Testing {version} is invalid: {reason}")]
    Metadata { version: String, reason: String },
    #[error("Chrome for Testing {version} has no {platform} archive")]
    NoArchive {
        version: String,
        platform: &'static str,
    },
    #[error("the {platform} archive is not an official download: {url}")]
    Unofficial { platform: &'static str, url: String },
    #[error("the {platform} archive holds no {executable}")]
    NoExecutable {
        platform: &'static str,
        executable: &'static str,
    },
    #[error("the Chrome runtime's {archive} holds no {entry}")]
    NoRuntimeEntry {
        archive: &'static str,
        entry: &'static str,
    },
    #[error("the release record is invalid: {0}")]
    Record(String),
    #[error(transparent)]
    Artifact(#[from] demi_shared_artifacts::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub fn run(options: Options) -> Result<(), Error> {
    let record = crate::repository().join(RECORD);
    let sources = Sources {
        metadata: METADATA,
        archives: ARCHIVES,
        runtime: RUNTIME,
    };
    crate::interruptible(|cancel| async move {
        // GitHub serves the runtime's assets through a redirect to
        // short-lived storage; what arrives is measured either way.
        let client = demi_shared_artifacts::client_following_redirects()?;
        let bytes = prepare(
            &client,
            &sources,
            &options.version,
            options.runtime,
            &cancel,
        )
        .await?;
        let publication = Publication {
            mode: Mode::Replace,
            permissions: Permissions::Default,
            durable: true,
        };
        demi_shared_artifacts::publish_bytes(&record, &bytes, publication).await?;
        println!(
            "Chrome for Testing {} with the Chrome runtime {}: {}",
            options.version,
            options.runtime,
            record.display()
        );
        Ok(())
    })?
}

/// Where the metadata and the archives come from.
struct Sources<'a> {
    /// The directory of the versions' metadata.
    metadata: &'a str,
    /// The prefix of every archive's URL.
    archives: &'a str,
    /// The directory of the runtime's releases.
    runtime: &'a str,
}

/// What the command reads of a version's metadata; the rest is Chrome for
/// Testing's.
#[derive(Deserialize)]
struct Metadata {
    version: String,
    downloads: Downloads,
}

#[derive(Deserialize)]
struct Downloads {
    chrome: Vec<Download>,
}

#[derive(Deserialize)]
struct Download {
    platform: String,
    url: String,
}

/// The release record of `version` with the runtime release `runtime`, as
/// the file's bytes.
async fn prepare(
    client: &demi_shared_artifacts::Client,
    sources: &Sources<'_>,
    version: &str,
    runtime: u32,
    cancel: &CancellationToken,
) -> Result<Vec<u8>, Error> {
    let invalid = |reason: String| Error::Metadata {
        version: version.to_owned(),
        reason,
    };
    let url = format!("{}/{version}.json", sources.metadata);
    let mut bytes = Vec::new();
    demi_shared_artifacts::download_measured(client, &url, METADATA_BYTES, &mut bytes, cancel)
        .await?;
    let metadata: Metadata =
        serde_json::from_slice(&bytes).map_err(|error| invalid(error.to_string()))?;
    if metadata.version != version {
        return Err(invalid(format!("it describes {}", metadata.version)));
    }
    // One archive at a time, so the disk holds one at most.
    let downloads = tempfile::tempdir()?;
    let mut platforms = Vec::new();
    for platform in &PLATFORMS {
        let download = metadata
            .downloads
            .chrome
            .iter()
            .find(|download| download.platform == platform.name)
            .ok_or_else(|| Error::NoArchive {
                version: version.to_owned(),
                platform: platform.name,
            })?;
        if !download.url.starts_with(sources.archives) {
            return Err(Error::Unofficial {
                platform: platform.name,
                url: download.url.clone(),
            });
        }
        let path = downloads.path().join(format!("{}.zip", platform.name));
        let mut file = tokio::fs::File::create(&path).await?;
        let digest = demi_shared_artifacts::download_measured(
            client,
            &download.url,
            ARCHIVE_BYTES,
            &mut file,
            cancel,
        )
        .await?;
        drop(file);
        if !demi_shared_artifacts::holds(&path, platform.executable).await? {
            return Err(Error::NoExecutable {
                platform: platform.name,
                executable: platform.executable,
            });
        }
        tokio::fs::remove_file(&path).await?;
        println!(
            "Chrome for Testing {version}: {} archive, {} bytes",
            platform.name, digest.size
        );
        platforms.push(ReleasePlatform {
            target: platform.target.to_owned(),
            url: download.url.clone(),
            size: digest.size,
            sha256: digest.sha256,
            executable: platform.executable.to_owned(),
        });
    }
    let mut libraries = BTreeMap::new();
    for platform in &PLATFORMS {
        let Some(archive) = platform.libraries else {
            continue;
        };
        let measured = runtime_archive(
            client,
            sources,
            runtime,
            archive,
            RUNTIME_LIBRARIES_ENTRY,
            downloads.path(),
            cancel,
        )
        .await?;
        libraries.insert(platform.target.to_owned(), measured);
    }
    let fonts = runtime_archive(
        client,
        sources,
        runtime,
        RUNTIME_FONTS,
        RUNTIME_FONTS_ENTRY,
        downloads.path(),
        cancel,
    )
    .await?;
    let release = BrowserRelease {
        version: version.to_owned(),
        platforms,
        runtime: ChromeRuntime {
            release: runtime,
            libraries,
            fonts,
        },
    };
    let record = crate::record(&release).map_err(|error| Error::Record(error.to_string()))?;
    // The record is checked the way its reader decodes it.
    let text = std::str::from_utf8(&record).map_err(|error| Error::Record(error.to_string()))?;
    BrowserRelease::parse(text).map_err(|error| Error::Record(error.to_string()))?;
    Ok(record)
}

/// The runtime release `runtime`'s `archive`, downloaded into `downloads`
/// and measured, once it holds `entry`; the download is not kept.
async fn runtime_archive(
    client: &demi_shared_artifacts::Client,
    sources: &Sources<'_>,
    runtime: u32,
    archive: &'static str,
    entry: &'static str,
    downloads: &std::path::Path,
    cancel: &CancellationToken,
) -> Result<RuntimeArchive, Error> {
    let url = format!("{}/{runtime}/{archive}", sources.runtime);
    let path = downloads.join(archive);
    let mut file = tokio::fs::File::create(&path).await?;
    let digest =
        demi_shared_artifacts::download_measured(client, &url, ARCHIVE_BYTES, &mut file, cancel)
            .await?;
    drop(file);
    if !demi_shared_artifacts::holds(&path, entry).await? {
        return Err(Error::NoRuntimeEntry { archive, entry });
    }
    tokio::fs::remove_file(&path).await?;
    println!(
        "Chrome runtime {runtime}: {archive}, {} bytes",
        digest.size
    );
    Ok(RuntimeArchive {
        url,
        size: digest.size,
        sha256: digest.sha256,
    })
}

#[cfg(test)]
mod tests {
    use demi_shared_artifacts::testing::{Answer, Server, tar_zst, zip};
    use serde_json::json;

    use super::*;

    /// The runtime release the fixture serves.
    const RELEASE: u32 = 1;

    /// Chrome for Testing as a fixture: each platform's archive and the
    /// runtime release's archives on one server, and metadata that describes
    /// `described` and names the platforms' archives on another, at
    /// `version`'s path. The archive named `lacking`, a platform or a
    /// runtime archive, holds no executable or entry.
    struct Fixture {
        metadata: Server,
        archives: Server,
        /// Each platform's archive, in the order of `PLATFORMS`.
        files: Vec<Vec<u8>>,
        /// Each runtime archive, by its name.
        runtime: BTreeMap<&'static str, Vec<u8>>,
    }

    async fn chrome_for_testing(version: &str, described: &str, lacking: Option<&str>) -> Fixture {
        let mut paths = Vec::new();
        let mut files = Vec::new();
        for platform in &PLATFORMS {
            let executable = if lacking == Some(platform.name) {
                "chrome/elsewhere"
            } else {
                platform.executable
            };
            paths.push(format!("/{version}/{0}/chrome-{0}.zip", platform.name));
            files.push(zip(&[
                (executable, platform.name.as_bytes()),
                ("LICENSE", b"license"),
            ]));
        }
        let mut runtime = BTreeMap::new();
        let libraries = PLATFORMS.iter().filter_map(|platform| platform.libraries);
        for archive in libraries {
            let entry = if lacking == Some(archive) {
                "lib/elsewhere.so"
            } else {
                RUNTIME_LIBRARIES_ENTRY
            };
            runtime.insert(archive, tar_zst(&[("lib/gio/modules/", b""), (entry, archive.as_bytes())]));
        }
        let fonts = if lacking == Some(RUNTIME_FONTS) {
            "fontconfig/elsewhere.conf"
        } else {
            RUNTIME_FONTS_ENTRY
        };
        runtime.insert(RUNTIME_FONTS, tar_zst(&[(fonts, b"<fontconfig/>")]));
        let chrome_answers = paths
            .iter()
            .zip(&files)
            .map(|(path, file)| (path.clone(), Answer::ok(file.clone())));
        let runtime_answers = runtime.iter().map(|(archive, file)| {
            (
                format!("/runtime/{RELEASE}/{archive}"),
                Answer::ok(file.clone()),
            )
        });
        let archives = Server::start(chrome_answers.chain(runtime_answers)).await;
        let chrome: Vec<serde_json::Value> = PLATFORMS
            .iter()
            .zip(&paths)
            .map(|(platform, path)| json!({"platform": platform.name, "url": archives.url(path)}))
            .collect();
        let described = json!({
            "version": described,
            "revision": "1500000",
            "downloads": {"chrome": chrome, "chromedriver": []},
        });
        let metadata = Server::start([(
            format!("/{version}.json"),
            Answer::ok(described.to_string()),
        )])
        .await;
        Fixture {
            metadata,
            archives,
            files,
            runtime,
        }
    }

    impl Fixture {
        /// Where the fixture's metadata, archives and runtime are.
        fn sources(&self) -> (String, String, String) {
            (
                self.metadata.url(""),
                self.archives.url("/"),
                self.archives.url("/runtime"),
            )
        }
    }

    async fn measured(bytes: &[u8], cancel: &CancellationToken) -> demi_shared_artifacts::Digest {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), bytes).unwrap();
        demi_shared_artifacts::digest(file.path(), u64::MAX, cancel)
            .await
            .unwrap()
    }

    /// About 0.1 s: two fixture servers on this machine.
    #[tokio::test]
    async fn a_pinned_version_records_each_official_archive_with_its_executable() {
        let client = demi_shared_artifacts::client_allowing_http().unwrap();
        let cancel = CancellationToken::new();
        let version = "153.0.8010.36";
        // The argument is Chrome's four-part version, or nothing is fetched.
        assert_eq!(super::version(version).unwrap(), version);
        assert!(super::version("153.0.8010").is_err());
        let fixture = chrome_for_testing(version, version, None).await;
        let (metadata, official, runtime) = fixture.sources();
        let sources = Sources {
            metadata: &metadata,
            archives: &official,
            runtime: &runtime,
        };
        let record = prepare(&client, &sources, version, RELEASE, &cancel)
            .await
            .unwrap();
        let release = BrowserRelease::parse(std::str::from_utf8(&record).unwrap()).unwrap();
        assert_eq!(release.version, version);
        assert_eq!(release.platforms.len(), PLATFORMS.len());
        for ((recorded, platform), file) in
            release.platforms.iter().zip(&PLATFORMS).zip(&fixture.files)
        {
            let digest = measured(file, &cancel).await;
            let expected = ReleasePlatform {
                target: platform.target.to_owned(),
                url: fixture
                    .archives
                    .url(&format!("/{version}/{0}/chrome-{0}.zip", platform.name)),
                size: digest.size,
                sha256: digest.sha256,
                executable: platform.executable.to_owned(),
            };
            assert_eq!(*recorded, expected);
        }
        // The runtime's libraries for each Linux platform, and its fonts.
        assert_eq!(release.runtime.release, RELEASE);
        let mut expected = BTreeMap::new();
        for platform in &PLATFORMS {
            let Some(archive) = platform.libraries else {
                continue;
            };
            let digest = measured(&fixture.runtime[archive], &cancel).await;
            expected.insert(
                platform.target.to_owned(),
                RuntimeArchive {
                    url: format!("{runtime}/{RELEASE}/{archive}"),
                    size: digest.size,
                    sha256: digest.sha256,
                },
            );
        }
        assert_eq!(release.runtime.libraries, expected);
        let digest = measured(&fixture.runtime[RUNTIME_FONTS], &cancel).await;
        assert_eq!(
            release.runtime.fonts,
            RuntimeArchive {
                url: format!("{runtime}/{RELEASE}/{RUNTIME_FONTS}"),
                size: digest.size,
                sha256: digest.sha256,
            }
        );
        assert_eq!(
            fixture.archives.requests(),
            PLATFORMS.len() + fixture.runtime.len()
        );
        // An archive anywhere but the official downloads is refused.
        let elsewhere = Sources {
            metadata: &metadata,
            archives: "https://storage.googleapis.com/",
            runtime: &runtime,
        };
        let refused = prepare(&client, &elsewhere, version, RELEASE, &cancel).await;
        assert!(
            matches!(
                refused,
                Err(Error::Unofficial {
                    platform: "mac-arm64",
                    ..
                })
            ),
            "{refused:?}"
        );
        // So is an archive without the executable or the entry the record
        // would name, and metadata of another version.
        for (lacking, archive) in [
            ("linux64", None),
            ("libs-aarch64.tar.zst", Some("libs-aarch64.tar.zst")),
            (RUNTIME_FONTS, Some(RUNTIME_FONTS)),
        ] {
            let fixture = chrome_for_testing(version, version, Some(lacking)).await;
            let (metadata, official, runtime) = fixture.sources();
            let sources = Sources {
                metadata: &metadata,
                archives: &official,
                runtime: &runtime,
            };
            let refused = prepare(&client, &sources, version, RELEASE, &cancel).await;
            match archive {
                None => assert!(
                    matches!(
                        refused,
                        Err(Error::NoExecutable {
                            platform: "linux64",
                            ..
                        })
                    ),
                    "{refused:?}"
                ),
                Some(archive) => assert!(
                    matches!(refused, Err(Error::NoRuntimeEntry { archive: refused, .. }) if refused == archive),
                    "{refused:?}"
                ),
            }
        }
        let other = chrome_for_testing(version, "153.0.8010.37", None).await;
        let (metadata, official, runtime) = other.sources();
        let sources = Sources {
            metadata: &metadata,
            archives: &official,
            runtime: &runtime,
        };
        let refused = prepare(&client, &sources, version, RELEASE, &cancel).await;
        assert!(
            matches!(refused, Err(Error::Metadata { .. })),
            "{refused:?}"
        );
        assert_eq!(other.archives.requests(), 0);
    }
}
