//! `cargo xtask browser-release <version>` (`builds-and-releases.md`
//! § Chrome for Testing): pins a Chrome for Testing version. It reads the
//! version's official download metadata, downloads the archive of each
//! platform Demi supports through `artifact`, measures it, checks that it
//! holds the executable the record names, and writes the release record with
//! builtin-protocol's type, which `demi-commands` installs from.

use demi_artifact::{Mode, Permissions, Publication};
use demi_builtin_protocol::release::{BrowserRelease, ReleasePlatform};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

/// Where Chrome for Testing publishes each version's downloads.
const METADATA: &str = "https://googlechromelabs.github.io/chrome-for-testing";
/// Where every official archive is.
const ARCHIVES: &str = "https://storage.googleapis.com/";
/// The pinned record, which `BrowserRelease::pinned` reads.
const RECORD: &str = "crates/builtin-protocol/src/release/chrome.json";
/// The most bytes a version's metadata may have.
const METADATA_BYTES: u64 = 1024 * 1024;
/// The most bytes an archive may have.
const ARCHIVE_BYTES: u64 = 1024 * 1024 * 1024;

/// A platform Demi pins Chrome for: Chrome for Testing's name for it, its
/// target, and the executable's path in its archive.
struct Platform {
    name: &'static str,
    target: &'static str,
    executable: &'static str,
}

/// Chrome for Testing publishes no Windows arm64 build.
const PLATFORMS: [Platform; 5] = [
    Platform {
        name: "mac-arm64",
        target: "aarch64-apple-darwin",
        executable: "chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
    },
    Platform {
        name: "mac-x64",
        target: "x86_64-apple-darwin",
        executable: "chrome-mac-x64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
    },
    Platform {
        name: "linux-arm64",
        target: "aarch64-unknown-linux-musl",
        executable: "chrome-linux-arm64/chrome",
    },
    Platform {
        name: "linux64",
        target: "x86_64-unknown-linux-musl",
        executable: "chrome-linux64/chrome",
    },
    Platform {
        name: "win64",
        target: "x86_64-pc-windows-msvc",
        executable: "chrome-win64/chrome.exe",
    },
];

#[derive(clap::Args)]
pub struct Options {
    /// Chrome's four-part version, such as 153.0.8010.36.
    #[arg(value_parser = version)]
    version: String,
}

/// Parses the version argument by the rule of the record it goes into.
fn version(value: &str) -> Result<String, String> {
    let probe = BrowserRelease {
        version: value.to_owned(),
        platforms: Vec::new(),
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
    NoArchive { version: String, platform: &'static str },
    #[error("the {platform} archive is not an official download: {url}")]
    Unofficial { platform: &'static str, url: String },
    #[error("the {platform} archive holds no {executable}")]
    NoExecutable { platform: &'static str, executable: &'static str },
    #[error("the release record is invalid: {0}")]
    Record(String),
    #[error(transparent)]
    Artifact(#[from] demi_artifact::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub fn run(options: Options) -> Result<(), Error> {
    let record = crate::repository().join(RECORD);
    let sources = Sources {
        metadata: METADATA,
        archives: ARCHIVES,
    };
    crate::interruptible(|cancel| async move {
        let client = demi_artifact::client()?;
        let bytes = prepare(&client, &sources, &options.version, &cancel).await?;
        let publication = Publication {
            mode: Mode::Replace,
            permissions: Permissions::Default,
            durable: true,
        };
        demi_artifact::publish_bytes(&record, &bytes, publication).await?;
        println!("Chrome for Testing {}: {}", options.version, record.display());
        Ok(())
    })?
}

/// Where the metadata and the archives come from.
struct Sources<'a> {
    /// The directory of the versions' metadata.
    metadata: &'a str,
    /// The prefix of every archive's URL.
    archives: &'a str,
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

/// The release record of `version`, as the file's bytes.
async fn prepare(
    client: &demi_artifact::Client,
    sources: &Sources<'_>,
    version: &str,
    cancel: &CancellationToken,
) -> Result<Vec<u8>, Error> {
    let invalid = |reason: String| Error::Metadata {
        version: version.to_owned(),
        reason,
    };
    let url = format!("{}/{version}.json", sources.metadata);
    let mut bytes = Vec::new();
    demi_artifact::download_measured(client, &url, METADATA_BYTES, &mut bytes, cancel).await?;
    let metadata: Metadata = serde_json::from_slice(&bytes).map_err(|error| invalid(error.to_string()))?;
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
        let digest = demi_artifact::download_measured(client, &download.url, ARCHIVE_BYTES, &mut file, cancel).await?;
        drop(file);
        if !demi_artifact::zip_holds(&path, platform.executable).await? {
            return Err(Error::NoExecutable {
                platform: platform.name,
                executable: platform.executable,
            });
        }
        tokio::fs::remove_file(&path).await?;
        println!("Chrome for Testing {version}: {} archive, {} bytes", platform.name, digest.size);
        platforms.push(ReleasePlatform {
            target: platform.target.to_owned(),
            url: download.url.clone(),
            size: digest.size,
            sha256: digest.sha256,
            executable: platform.executable.to_owned(),
        });
    }
    let release = BrowserRelease {
        version: version.to_owned(),
        platforms,
    };
    let record = crate::record(&release).map_err(|error| Error::Record(error.to_string()))?;
    // The record is checked the way its reader decodes it.
    let text = std::str::from_utf8(&record).map_err(|error| Error::Record(error.to_string()))?;
    BrowserRelease::parse(text).map_err(|error| Error::Record(error.to_string()))?;
    Ok(record)
}

#[cfg(test)]
mod tests {
    use demi_artifact::testing::{Answer, Server, zip};
    use serde_json::json;

    use super::*;

    /// Chrome for Testing as a fixture: each platform's archive on one
    /// server, and metadata that describes `described` and names them on
    /// another, at `version`'s path. The archive of `lacking` holds no
    /// executable.
    struct Fixture {
        metadata: Server,
        archives: Server,
        /// Each platform's archive, in the order of `PLATFORMS`.
        files: Vec<Vec<u8>>,
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
            files.push(zip(&[(executable, platform.name.as_bytes()), ("LICENSE", b"license")]));
        }
        let answers = paths.iter().zip(&files).map(|(path, file)| (path.clone(), Answer::ok(file.clone())));
        let archives = Server::start(answers).await;
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
        let metadata = Server::start([(format!("/{version}.json"), Answer::ok(described.to_string()))]).await;
        Fixture {
            metadata,
            archives,
            files,
        }
    }

    #[tokio::test]
    async fn a_pinned_version_records_each_official_archive_with_its_executable() {
        let client = demi_artifact::client_allowing_http().unwrap();
        let cancel = CancellationToken::new();
        let version = "153.0.8010.36";
        // The argument is Chrome's four-part version, or nothing is fetched.
        assert_eq!(super::version(version).unwrap(), version);
        assert!(super::version("153.0.8010").is_err());
        let fixture = chrome_for_testing(version, version, None).await;
        let metadata = fixture.metadata.url("");
        let official = fixture.archives.url("/");
        let sources = Sources {
            metadata: &metadata,
            archives: &official,
        };
        let record = prepare(&client, &sources, version, &cancel).await.unwrap();
        let release = BrowserRelease::parse(std::str::from_utf8(&record).unwrap()).unwrap();
        assert_eq!(release.version, version);
        assert_eq!(release.platforms.len(), PLATFORMS.len());
        let files = tempfile::tempdir().unwrap();
        for ((recorded, platform), file) in release.platforms.iter().zip(&PLATFORMS).zip(&fixture.files) {
            let path = files.path().join(platform.name);
            std::fs::write(&path, file).unwrap();
            let digest = demi_artifact::digest(&path, u64::MAX, &cancel).await.unwrap();
            let expected = ReleasePlatform {
                target: platform.target.to_owned(),
                url: fixture.archives.url(&format!("/{version}/{0}/chrome-{0}.zip", platform.name)),
                size: digest.size,
                sha256: digest.sha256,
                executable: platform.executable.to_owned(),
            };
            assert_eq!(*recorded, expected);
        }
        assert_eq!(fixture.archives.requests(), PLATFORMS.len());
        // An archive anywhere but the official downloads is refused.
        let elsewhere = Sources {
            metadata: &metadata,
            archives: "https://storage.googleapis.com/",
        };
        let refused = prepare(&client, &elsewhere, version, &cancel).await;
        assert!(matches!(refused, Err(Error::Unofficial { platform: "mac-arm64", .. })), "{refused:?}");
        // So is an archive without the executable the record would name, and
        // metadata of another version.
        let lacking = chrome_for_testing(version, version, Some("linux64")).await;
        let metadata = lacking.metadata.url("");
        let official = lacking.archives.url("/");
        let sources = Sources {
            metadata: &metadata,
            archives: &official,
        };
        let refused = prepare(&client, &sources, version, &cancel).await;
        assert!(matches!(refused, Err(Error::NoExecutable { platform: "linux64", .. })), "{refused:?}");
        let other = chrome_for_testing(version, "153.0.8010.37", None).await;
        let metadata = other.metadata.url("");
        let official = other.archives.url("/");
        let sources = Sources {
            metadata: &metadata,
            archives: &official,
        };
        let refused = prepare(&client, &sources, version, &cancel).await;
        assert!(matches!(refused, Err(Error::Metadata { .. })), "{refused:?}");
        assert_eq!(other.archives.requests(), 0);
    }
}
