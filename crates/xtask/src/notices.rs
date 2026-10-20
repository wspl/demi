//! `xtask notices`: writes the third-party notices of what a release
//! ships into the built web app, which shows them at Settings › About ›
//! Acknowledgements and which a server release carries, its notices
//! copied to the root as `THIRD-PARTY-NOTICES`.
//!
//! The Rust crates are those each shipped program links on its own
//! targets, the vendored ones included, read with cargo-about, which finds
//! each crate's license files in its sources and checks its license against
//! [`ACCEPTED`]. The npm packages are those the web app's bundle holds, as
//! Vite's `build.license` lists them from the bundle's module graph, each
//! with the text of its license file. A dependency whose license is not one
//! of [`ACCEPTED`], or whose license cannot be read, stops the command, so a
//! new dependency never ships without its notice. The libraries a program
//! carries that are neither, [`LIBRARIES`], have their notice in the
//! repository.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use clap::ValueEnum as _;
use serde::Deserialize;

use crate::native::Executable;

/// The notices in a built web app. The name has an extension: the backend
/// answers a path without one with the page.
pub const WEB_NOTICES: &str = "third-party-notices.txt";

/// What Vite's `build.license` writes into a built web app: the bundled
/// packages with their licenses (`packages/web/vite.config.ts`).
const BUNDLED: &str = ".vite/license.json";

/// The licenses a shipped dependency may be under: the ones the licensing
/// audit found in the release's dependencies. A dependency offered under
/// several is shipped under the first of these it allows.
const ACCEPTED: [&str; 18] = [
    "MIT",
    "Apache-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "Zlib",
    "Unicode-3.0",
    // finl_unicode's data tables, which the backend links, are under the
    // earlier Unicode license.
    "Unicode-DFS-2016",
    "CDLA-Permissive-2.0",
    "MPL-2.0",
    "BSL-1.0",
    "0BSD",
    "MIT-0",
    "Unlicense",
    "CC0-1.0",
    "WTFPL",
    "bzip2-1.0.6",
    "OpenSSL",
];

/// A library a shipped program carries that is neither a crate nor an npm
/// package, with its notice in the repository.
struct Library {
    name: &'static str,
    /// Empty for one its source does not version.
    version: &'static str,
    license: &'static str,
    /// Its notice, from the repository's root.
    notice: &'static str,
}

/// idb's libraries and guest bridge, in demi-mobile's iOS helper
/// (`crates/command-package-mobile/build.rs`); openatx's UI automation server, demi-mobile's
/// Android helper, and Cisco's OpenH264 binary, which demi-mobile downloads
/// for the Android view and whose binary license asks the product to credit
/// it (`crates/command-package-mobile/build.rs`); the vendors' marks the web app shows,
/// models.dev's and Simple Icons' (`crates/backend-providers/src/llm/vendors.rs`);
/// and the Android robot, an Android emulator's logo
/// (`packages/plugin-mobile/src/index.ts`), whose license asks for its
/// attribution.
const LIBRARIES: [Library; 6] = [
    Library {
        name: "idb",
        version: "1.7.6",
        license: "MIT",
        notice: "crates/command-package-mobile/ios-sim/LICENSE-idb",
    },
    Library {
        name: "android-uiautomator-server-jar",
        version: "0.4.0",
        license: "MIT",
        notice: "crates/command-package-mobile/android/LICENSE-u2",
    },
    Library {
        name: "OpenH264",
        version: "2.6.0",
        license: "BSD-2-Clause",
        notice: "crates/command-package-mobile/android/LICENSE-openh264",
    },
    Library {
        name: "models.dev logos",
        version: "",
        license: "MIT",
        notice: "packages/web-ui/src/settings/logos/LICENSE-models.dev",
    },
    Library {
        name: "Simple Icons",
        version: "",
        license: "CC0-1.0",
        notice: "packages/web-ui/src/settings/logos/LICENSE-simple-icons",
    },
    Library {
        name: "Android robot",
        version: "",
        license: "CC-BY-3.0",
        notice: "packages/web-ui/src/devices/LICENSE-android-robot",
    },
];

#[derive(clap::Args)]
pub struct Options {
    /// The built web app, which the notices are written into.
    #[arg(long, value_name = "DIRECTORY")]
    web: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{} holds no {BUNDLED}, which `vite build` writes: it is not a built web app", .0.display())]
    NotBuilt(PathBuf),
    #[error("{BUNDLED}: {0}")]
    Bundled(serde_json::Error),
    #[error("{package} has no license that can be read")]
    Unreadable { package: String },
    #[error("{package} is under {license}, which is not a license a release accepts")]
    NotAccepted { package: String, license: String },
    #[error(
        "cargo about failed for {0}: a crate it links is under a license a release does not accept, \
         or its license cannot be read (above); cargo-about installs with \
         `cargo install --locked cargo-about --features cli`"
    )]
    Crates(&'static str),
    #[error("cargo about's report for {0}: {1}")]
    Report(&'static str, serde_json::Error),
    #[error("no workspace package is named {0}")]
    NoPackage(&'static str),
    #[error("cargo metadata: {0}")]
    Metadata(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub fn run(options: Options) -> Result<(), Error> {
    let written = write(&crate::repository(), &options.web)?;
    println!("Third-party notices: {}", written.display());
    Ok(())
}

/// Writes the notices into the built web app `web` and answers their path.
pub fn write(repository: &Path, web: &Path) -> Result<PathBuf, Error> {
    let packages = npm(web)?;
    let crates = crates(repository)?;
    let libraries = libraries(repository)?;
    let path = web.join(WEB_NOTICES);
    std::fs::write(&path, render(&crates, &packages, &libraries))?;
    Ok(path)
}

/// The notices of [`LIBRARIES`].
fn libraries(repository: &Path) -> Result<Vec<Notice>, Error> {
    let mut notices = Notices::default();
    for library in LIBRARIES {
        let text = std::fs::read_to_string(repository.join(library.notice))?;
        notices.add(
            library.license,
            &text,
            Shipped {
                name: library.name.to_owned(),
                version: library.version.to_owned(),
                source: None,
            },
        );
    }
    Ok(notices.into_vec())
}

/// One license text and the packages under it.
#[derive(Debug, PartialEq, Eq)]
struct Notice {
    /// The license's name, as its SPDX expression.
    license: String,
    text: String,
    packages: BTreeSet<Shipped>,
}

/// A package a release ships.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Shipped {
    name: String,
    version: String,
    /// Where its source is, named for a package under the MPL-2.0, which
    /// asks for it.
    source: Option<String>,
}

/// The notices grouped by their license and text, in that order.
#[derive(Default)]
struct Notices(BTreeMap<(String, String), BTreeSet<Shipped>>);

impl Notices {
    fn add(&mut self, license: &str, text: &str, package: Shipped) {
        self.0
            // Blank lines around the text go; a first line's indent, as
            // the Apache License's centred title has, stays.
            .entry((
                license.to_owned(),
                text.trim_start_matches(['\n', '\r']).trim_end().to_owned(),
            ))
            .or_default()
            .insert(package);
    }

    fn into_vec(self) -> Vec<Notice> {
        self.0
            .into_iter()
            .map(|((license, text), packages)| Notice {
                license,
                text,
                packages,
            })
            .collect()
    }
}

/// The licenses a release accepts, as `spdx` checks a requirement against
/// them.
fn accepted() -> Vec<spdx::Licensee> {
    ACCEPTED
        .iter()
        .map(|id| spdx::Licensee::parse(id).expect("an accepted license is an SPDX identifier"))
        .collect()
}

/// Whether `expression` names the MPL-2.0, whose packages name their source.
fn mpl(expression: &spdx::Expression) -> bool {
    expression
        .requirements()
        .any(|requirement| requirement.req.license.id() == spdx::license_id("MPL-2.0"))
}

/// One package of Vite's list.
#[derive(Deserialize)]
struct Bundled {
    name: String,
    version: String,
    /// Its `package.json`'s `license`.
    identifier: Option<String>,
    /// Its license file's text, which a package may lack.
    text: Option<String>,
}

/// The notices of the npm packages the built web app `web` bundles. A
/// package without a license file has its license's standard text, when
/// it names a single license.
fn npm(web: &Path) -> Result<Vec<Notice>, Error> {
    let list = match std::fs::read(web.join(BUNDLED)) {
        Ok(list) => list,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::NotBuilt(web.to_owned()));
        }
        Err(error) => return Err(error.into()),
    };
    let bundled: Vec<Bundled> = serde_json::from_slice(&list).map_err(Error::Bundled)?;
    let accepted = accepted();
    let mut notices = Notices::default();
    for package in bundled {
        let named = format!("{}@{}", package.name, package.version);
        let unreadable = || Error::Unreadable {
            package: named.clone(),
        };
        let identifier = package.identifier.as_deref().ok_or_else(unreadable)?;
        let expression = spdx::Expression::parse_mode(identifier, spdx::ParseMode::LAX)
            .map_err(|_| unreadable())?;
        if !expression.evaluate(|requirement| {
            accepted
                .iter()
                .any(|licensee| licensee.satisfies(requirement))
        }) {
            return Err(Error::NotAccepted {
                package: named,
                license: identifier.to_owned(),
            });
        }
        let text = match package.text {
            Some(text) => text,
            None => {
                let mut requirements = expression.requirements();
                match (requirements.next(), requirements.next()) {
                    (Some(only), None) => only.req.license.id().ok_or_else(unreadable)?.text().to_owned(),
                    _ => return Err(unreadable()),
                }
            }
        };
        let source = mpl(&expression).then(|| {
            format!(
                "https://www.npmjs.com/package/{}/v/{}",
                package.name, package.version
            )
        });
        notices.add(
            &expression.to_string(),
            &text,
            Shipped {
                name: package.name,
                version: package.version,
                source,
            },
        );
    }
    Ok(notices.into_vec())
}

/// What the command reads of cargo-about's report.
#[derive(Deserialize)]
struct Report {
    licenses: Vec<ReportLicense>,
}

#[derive(Deserialize)]
struct ReportLicense {
    id: String,
    text: String,
    used_by: Vec<UsedBy>,
}

#[derive(Deserialize)]
struct UsedBy {
    #[serde(rename = "crate")]
    krate: ReportCrate,
}

#[derive(Deserialize)]
struct ReportCrate {
    name: String,
    version: String,
    repository: Option<String>,
}

/// cargo-about's settings: the accepted licenses; no build or development
/// dependency, which no program links; and no crate of the workspace,
/// which LICENSE and LICENSE-DEMI cover. The vendored crates are published ones, and so
/// are listed. Its workarounds read ring's and rustls's license files,
/// which hold several licenses each.
fn about_config() -> String {
    let accepted: Vec<String> = ACCEPTED.iter().map(|id| format!("\"{id}\"")).collect();
    format!(
        "accepted = [{}]\n\
         ignore-build-dependencies = true\n\
         ignore-dev-dependencies = true\n\
         workarounds = [\"ring\", \"rustls\"]\n\
         private = {{ ignore = true }}\n",
        accepted.join(", ")
    )
}

/// The notices of the crates the shipped programs link, each program's on
/// its own targets. cargo-about reads one package's dependencies at a
/// time, so it runs once for each program and the reports are merged.
fn crates(repository: &Path) -> Result<Vec<Notice>, Error> {
    let workspace = crate::packages(repository).map_err(Error::Metadata)?;
    let config = tempfile::NamedTempFile::new()?;
    std::fs::write(config.path(), about_config())?;
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut notices = Notices::default();
    for executable in Executable::value_variants() {
        let name = executable.name();
        let package = workspace
            .iter()
            .find(|package| package.name == name)
            .ok_or(Error::NoPackage(name))?;
        println!("Third-party notices: the crates {name} links");
        let mut command = Command::new(&cargo);
        command
            .args(["about", "generate", "--format", "json", "--locked", "--fail"])
            .arg("--config")
            .arg(config.path())
            .arg("--manifest-path")
            .arg(&package.manifest_path)
            .current_dir(repository)
            .stderr(Stdio::inherit());
        for target in executable.targets() {
            command.args(["--target", target]);
        }
        let output = command.output()?;
        if !output.status.success() {
            return Err(Error::Crates(name));
        }
        let report: Report =
            serde_json::from_slice(&output.stdout).map_err(|error| Error::Report(name, error))?;
        for license in report.licenses {
            for used in license.used_by {
                let krate = used.krate;
                let source = (license.id == "MPL-2.0").then(|| {
                    krate.repository.clone().unwrap_or_else(|| {
                        format!("https://crates.io/crates/{}/{}", krate.name, krate.version)
                    })
                });
                notices.add(
                    &license.id,
                    &license.text,
                    Shipped {
                        name: krate.name,
                        version: krate.version,
                        source,
                    },
                );
            }
        }
    }
    Ok(notices.into_vec())
}

/// The notices as the file holds them.
fn render(crates: &[Notice], packages: &[Notice], libraries: &[Notice]) -> String {
    let mut out = "Third-party notices\n\n\
         Demi includes the third-party software below. Each notice names the\n\
         packages under one license text, then gives the text.\n"
        .to_owned();
    for (title, notices) in [("Rust crates", crates), ("npm packages", packages), ("Other libraries", libraries)] {
        let _ = write!(out, "\n\n{title}\n{}\n", "=".repeat(title.len()));
        for notice in notices {
            let _ = write!(out, "\n{}\n{}\n\n", notice.license, "-".repeat(notice.license.len()));
            for package in &notice.packages {
                let _ = write!(out, "{}", package.name);
                if !package.version.is_empty() {
                    let _ = write!(out, " {}", package.version);
                }
                let _ = match &package.source {
                    Some(source) => writeln!(out, ", source: {source}"),
                    None => writeln!(out),
                };
            }
            let _ = write!(out, "\n{}\n", notice.text);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A built web app whose bundle holds `packages`, as Vite lists them.
    fn web(packages: serde_json::Value) -> tempfile::TempDir {
        let web = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(web.path().join(".vite")).unwrap();
        std::fs::write(web.path().join(BUNDLED), packages.to_string()).unwrap();
        web
    }

    /// The notices credit Cisco's OpenH264 as its binary license asks,
    /// with that license's link, though the binary is downloaded at run
    /// time rather than shipped; and the sources of the vendors' marks,
    /// which have no version. Planted defect this catches: OpenH264 or a
    /// marks' source left out of the notices. Milliseconds.
    #[test]
    fn the_notices_credit_ciscos_openh264_and_the_marks_sources() {
        let notices = render(&[], &[], &libraries(&crate::repository()).unwrap());
        assert!(notices.contains("OpenH264 2.6.0"), "{notices}");
        assert!(notices.contains("OpenH264 Video Codec provided by Cisco Systems, Inc."));
        assert!(notices.contains("https://www.openh264.org/BINARY_LICENSE.txt"));
        assert!(notices.contains("\nmodels.dev logos\n"));
        assert!(notices.contains("Copyright (c) 2025 models.dev"));
        assert!(notices.contains("\nSimple Icons\n"));
        assert!(notices.contains("CC0 1.0 Universal"));
    }

    #[test]
    fn a_bundled_package_under_a_license_a_release_does_not_accept_stops_the_notices() {
        let accepted = serde_json::json!([
            { "name": "dompurify", "version": "3.2.0", "identifier": "(MPL-2.0 OR Apache-2.0)", "text": "DOMPurify\nCopyright 2025 Dr.-Ing. Mario Heiderich" },
            { "name": "overlayscrollbars", "version": "2.16.0", "identifier": "MIT" },
            { "name": "vue", "version": "3.5.21", "identifier": "MIT", "text": "The MIT License (MIT)\n\nCopyright (c) 2018-present, Yuxi (Evan) You" },
        ]);
        let notices = npm(web(accepted).path()).unwrap();
        // Each license text once, with the packages under it; the MPL-2.0
        // one names where its source is.
        assert_eq!(
            notices
                .iter()
                .map(|notice| (notice.license.as_str(), notice.packages.len()))
                .collect::<Vec<_>>(),
            [("(MPL-2.0 OR Apache-2.0)", 1), ("MIT", 1), ("MIT", 1)]
        );
        let dompurify = notices[0].packages.first().unwrap();
        assert_eq!(
            dompurify.source.as_deref(),
            Some("https://www.npmjs.com/package/dompurify/v/3.2.0")
        );
        // A package without a license file has its license's standard text.
        assert!(notices.iter().any(|notice| notice.text.starts_with("MIT License")
            && notice.packages.first().unwrap().name == "overlayscrollbars"));

        let copyleft = serde_json::json!([
            { "name": "vue", "version": "3.5.21", "identifier": "MIT", "text": "MIT" },
            { "name": "copyleft", "version": "1.0.0", "identifier": "GPL-3.0-only", "text": "GNU GENERAL PUBLIC LICENSE" },
        ]);
        let refused = npm(web(copyleft).path());
        assert!(
            matches!(&refused, Err(Error::NotAccepted { package, license })
                if package == "copyleft@1.0.0" && license == "GPL-3.0-only"),
            "{refused:?}"
        );
        // A package that names no license cannot be noticed either.
        let unnamed = serde_json::json!([{ "name": "unnamed", "version": "0.1.0", "text": "All rights reserved" }]);
        let unread = npm(web(unnamed).path());
        assert!(
            matches!(&unread, Err(Error::Unreadable { package }) if package == "unnamed@0.1.0"),
            "{unread:?}"
        );
    }
}
