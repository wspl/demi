//! `xtask sdk-release`: assembles the npm packages of the TypeScript SDK
//! (`packages/sdk`) from the executables `xtask native build` wrote and the
//! SDK's own build, one directory for each package, as `npm pack` takes it:
//!
//! ```text
//! <output>/
//!   sdk/                     @demicodes/sdk
//!     package.json           the SDK's, at the build version, with each
//!                            platform's package an optional dependency
//!     dist/                  the SDK's build (`bun run build`)
//!     release/commands/      every command package's descriptor
//!   sdk-darwin-arm64/        @demicodes/sdk-darwin-arm64, one for each target
//!     package.json           with its `os` and `cpu`
//!     bin/demi-core          the target's process and runner
//!     bin/demi-runner
//!     release/files/         the target's command programs, compressed
//!   ...
//! ```
//!
//! A package manager installs only the platform package whose `os` and `cpu`
//! are the machine's, as esbuild's are installed. The SDK finds its
//! executables and release in this layout (`packages/sdk/src/binaries.ts`).
//!
//! The SDK's dependencies in the workspace are written at the build version,
//! which they are published at too, and its `development` export, which
//! names its sources, is left out: the package holds none.

use std::path::{Path, PathBuf};

use demi_runner_protocol::release::{FILES, compressed_file};
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

use crate::native::{self, Caches, Executable, Spec, Versioning};

/// The SDK's main package's name, and the prefix of each platform package's.
const SDK: &str = "@demicodes/sdk";

#[derive(clap::Args)]
pub struct Options {
    /// The directory to assemble the packages in, which does not exist yet.
    #[arg(long, value_name = "DIRECTORY")]
    output: PathBuf,
    /// A target of the platform packages; repeat for several [default: all
    /// six].
    #[arg(long = "target", value_name = "TRIPLE", value_parser = native::target)]
    targets: Vec<&'static str>,
    /// The SDK's build [default: packages/sdk/dist].
    #[arg(long, value_name = "DIRECTORY")]
    dist: Option<PathBuf>,
    /// Names the command packages with the build version itself; only the
    /// build workflow passes it.
    #[arg(long)]
    publish: bool,
    /// The Cargo target directory the build wrote [default:
    /// .cache/native-target in the repository].
    #[arg(long, value_name = "DIRECTORY")]
    artifacts: Option<PathBuf>,
    #[command(flatten)]
    caches: Caches,
}

/// Why assembling stopped.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{} exists: the SDK's packages are assembled once", .0.display())]
    Exists(PathBuf),
    #[error("{} has no parent directory to assemble the packages beside", .0.display())]
    NoParent(PathBuf),
    #[error("{} holds no index.mjs: `bun run build` in packages/sdk builds it", .0.display())]
    NotBuilt(PathBuf),
    #[error("packages/sdk/package.json: {0}")]
    Manifest(String),
    #[error(transparent)]
    Stage(#[from] crate::server_release::Error),
    #[error(transparent)]
    Native(#[from] native::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub fn run(options: Options) -> Result<(), Error> {
    let assembled = crate::interruptible(|cancel| async move { assemble(&options, &cancel).await })??;
    println!("SDK packages: {}", assembled.display());
    Ok(())
}

/// Assembles the packages `options` names and returns their directory.
async fn assemble(options: &Options, cancel: &CancellationToken) -> Result<PathBuf, Error> {
    let output = std::path::absolute(&options.output)?;
    if tokio::fs::try_exists(&output).await? {
        return Err(Error::Exists(output));
    }
    let repository = crate::repository();
    let dist = match &options.dist {
        Some(dist) => std::path::absolute(dist)?,
        None => repository.join("packages/sdk/dist"),
    };
    if !tokio::fs::try_exists(dist.join("index.mjs")).await? {
        return Err(Error::NotBuilt(dist));
    }
    let parent = output
        .parent()
        .ok_or_else(|| Error::NoParent(output.clone()))?
        .to_owned();
    tokio::fs::create_dir_all(&parent).await?;
    let name = output
        .file_name()
        .ok_or_else(|| Error::NoParent(output.clone()))?
        .to_string_lossy()
        .into_owned();
    // The stages are removed however assembling ends, unless the packages'
    // becomes the output.
    let stage = crate::server_release::staged(&parent, format!(".{name}-stage-")).await?;
    let packaged = crate::server_release::staged(&parent, format!(".{name}-packages-")).await?;
    let artifacts = native::artifacts(options.artifacts.as_deref())?;
    let versioning = if options.publish {
        Versioning::Published
    } else {
        Versioning::Development
    };
    let version = demi_shared_artifacts::BUILD_VERSION;
    let targets = native::targets(&[Executable::Runner, Executable::Core], &options.targets)?;

    // The command packages: their descriptors in the main package, their
    // programs gathered here, then handed to each target's package.
    let main = stage.path().join("sdk");
    let release = main.join("release");
    let gathered = packaged.path().join(FILES);
    for executable in Executable::COMMANDS {
        let targets = native::targets(&[executable], &targets)?;
        let output = packaged.path().join(executable.name());
        let spec = Spec {
            executable,
            targets: &targets,
            artifacts: &artifacts,
            output: &output,
            caches: &options.caches,
            versioning,
            version,
        };
        println!("{}", native::package(&spec, cancel).await?);
        native::split(&output, executable, &release, &gathered, cancel).await?;
    }

    let mut platforms = Map::new();
    for &target in &targets {
        let (os, cpu) = platform(target);
        let package = format!("{SDK}-{os}-{cpu}");
        let directory = stage.path().join(format!("sdk-{os}-{cpu}"));
        let bin = directory.join("bin");
        tokio::fs::create_dir_all(&bin).await?;
        for executable in [Executable::Core, Executable::Runner] {
            let built = native::built(&artifacts, executable, target);
            if !tokio::fs::try_exists(&built).await? {
                return Err(native::Error::NotBuilt {
                    executable,
                    target,
                    path: built,
                }
                .into());
            }
            // `copy` keeps the executable's permissions.
            tokio::fs::copy(&built, bin.join(executable.file_name(target))).await?;
        }
        let files = directory.join("release").join(FILES);
        tokio::fs::create_dir_all(&files).await?;
        for executable in Executable::COMMANDS {
            let file = compressed_file(executable.name(), target);
            if tokio::fs::try_exists(gathered.join(&file)).await? {
                tokio::fs::rename(gathered.join(&file), files.join(&file)).await?;
            }
        }
        let manifest = json!({
            "name": package,
            "version": version,
            "description": format!("The executables and command programs of {SDK} for {}.", described(target)),
            "license": "Apache-2.0",
            "os": [os],
            "cpu": [cpu],
            "files": ["bin", "release"],
            // Yarn's Plug'n'Play runs no executable from inside a zip.
            "preferUnplugged": true,
            "repository": {
                "type": "git",
                "url": "git+https://github.com/demi-sh/demi.git",
            },
        });
        write_manifest(&directory, &manifest).await?;
        legal(&directory).await?;
        platforms.insert(package, Value::String(version.to_owned()));
    }

    crate::server_release::copy_directory(dist, main.join("dist")).await?;
    let source = tokio::fs::read(repository.join("packages/sdk/package.json")).await?;
    let manifest = main_manifest(&source, version, platforms)?;
    write_manifest(&main, &manifest).await?;
    legal(&main).await?;

    if cancel.is_cancelled() {
        return Err(native::Error::Artifact(demi_shared_artifacts::Error::Cancelled).into());
    }
    let staged = stage.keep();
    if let Err(error) = tokio::fs::rename(&staged, &output).await {
        // Nothing else holds the stage: it goes rather than lingering.
        let _ = tokio::fs::remove_dir_all(&staged).await;
        return Err(error.into());
    }
    Ok(output)
}

/// The main package's manifest: the SDK's own at `version`, its
/// dependencies in the workspace at `version` too, without its
/// `development` export, and with each of `platforms` an optional
/// dependency.
fn main_manifest(source: &[u8], version: &str, platforms: Map<String, Value>) -> Result<Value, Error> {
    let mut manifest: Value =
        serde_json::from_slice(source).map_err(|error| Error::Manifest(error.to_string()))?;
    let object = manifest
        .as_object_mut()
        .ok_or_else(|| Error::Manifest("not an object".to_owned()))?;
    object.insert("version".to_owned(), Value::String(version.to_owned()));
    if let Some(Value::Object(dependencies)) = object.get_mut("dependencies") {
        for value in dependencies.values_mut() {
            if value.as_str().is_some_and(|range| range.starts_with("workspace:")) {
                *value = Value::String(version.to_owned());
            }
        }
    }
    if let Some(Value::Object(exports)) = object.get_mut("exports") {
        for entry in exports.values_mut() {
            if let Value::Object(conditions) = entry {
                conditions.shift_remove("development");
            }
        }
    }
    object.shift_remove("scripts");
    object.insert("optionalDependencies".to_owned(), Value::Object(platforms));
    Ok(manifest)
}

/// npm's `os` and `cpu` of `target`, as Node.js names its platform and
/// architecture.
fn platform(target: &str) -> (&'static str, &'static str) {
    let os = if target.contains("apple") {
        "darwin"
    } else if target.contains("windows") {
        "win32"
    } else {
        "linux"
    };
    let cpu = if target.starts_with("aarch64") { "arm64" } else { "x64" };
    (os, cpu)
}

/// `target` in words, as a platform package's description says it.
fn described(target: &str) -> &'static str {
    match platform(target) {
        ("darwin", "arm64") => "macOS on ARM64",
        ("darwin", _) => "macOS on x86-64",
        ("win32", "arm64") => "Windows on ARM64",
        ("win32", _) => "Windows on x86-64",
        (_, "arm64") => "Linux on ARM64",
        _ => "Linux on x86-64",
    }
}

async fn write_manifest(directory: &Path, manifest: &Value) -> Result<(), Error> {
    let mut text = serde_json::to_string_pretty(manifest).map_err(|error| Error::Manifest(error.to_string()))?;
    text.push('\n');
    tokio::fs::write(directory.join("package.json"), text).await?;
    Ok(())
}

/// The project's license and notice, which each package carries.
async fn legal(directory: &Path) -> Result<(), Error> {
    for (name, text) in crate::server_release::LEGAL {
        tokio::fs::write(directory.join(name), text).await?;
    }
    Ok(())
}
