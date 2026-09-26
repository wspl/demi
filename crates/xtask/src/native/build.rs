//! `cargo xtask native build` (`builds-and-releases.md` § Cross builds):
//! one release build per target with the machine's own cross tools,
//! cargo-zigbuild for the Apple and Linux targets and cargo-xwin for
//! Windows, or inside the build container's image.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use super::{Error, Executable, apple, windows};

/// The Apple SDK the Apple targets are built against.
pub(super) const APPLE_SDK_VERSION: &str = "15.4";
/// The Windows SDK and C runtime that cargo-xwin downloads.
const WINDOWS_SDK_VERSION: &str = "10.0.26100";
const WINDOWS_CRT_VERSION: &str = "14.44.17.14";
/// The oldest macOS the Apple builds run on.
const MACOS_MINIMUM: &str = "13.0";
/// The build container's paths: the checkout, the Cargo target directory
/// (not `/output`, which clang-cl reads as an output flag) and the SDK.
const CONTAINER_CHECKOUT: &str = "/work";
const CONTAINER_ARTIFACTS: &str = "/build";
const CONTAINER_SDK: &str = "/sdk";

#[derive(clap::Args)]
pub struct Options {
    /// An executable to build; repeat for several [default: the runner and
    /// the command programs].
    #[arg(long = "package", value_name = "CRATE")]
    packages: Vec<Executable>,
    /// A target to build for; repeat for several [default: every target of
    /// each executable].
    #[arg(long = "target", value_name = "TRIPLE", value_parser = super::target)]
    targets: Vec<&'static str>,
    /// The Cargo target directory the builds write [default:
    /// .cache/native-target in the repository].
    #[arg(long, value_name = "DIRECTORY")]
    artifacts: Option<PathBuf>,
    /// The Apple SDK, which the Apple targets need.
    #[arg(long, env = "SDKROOT", value_name = "DIRECTORY")]
    sdk: Option<PathBuf>,
    /// Builds inside this image of scripts/native/Dockerfile, for a machine
    /// without the cross tools.
    #[arg(long, value_name = "IMAGE")]
    container: Option<String>,
}

pub fn run(options: Options) -> Result<(), Error> {
    let repository = crate::repository();
    let named = if options.packages.is_empty() {
        Executable::DEFAULT.to_vec()
    } else {
        options.packages
    };
    let mut executables: Vec<Executable> = Vec::new();
    for executable in named {
        if !executables.contains(&executable) {
            executables.push(executable);
        }
    }
    let targets = super::targets(&executables, &options.targets)?;
    let sdk = if targets.iter().any(|target| apple(target)) {
        Some(apple_sdk(options.sdk.as_deref())?)
    } else {
        None
    };
    let artifacts = super::artifacts(options.artifacts.as_deref())?;
    std::fs::create_dir_all(&artifacts)?;
    let build = Build {
        repository: &repository,
        artifacts: &artifacts,
        sdk: sdk.as_deref(),
    };
    for target in targets {
        let built: Vec<Executable> = executables
            .iter()
            .copied()
            .filter(|executable| executable.targets().contains(&target))
            .collect();
        println!("Native build: {target}");
        let mut command = match &options.container {
            Some(image) => build.in_container(image, target, &built)?,
            None => build.here(target, &built),
        };
        let status = command.status()?;
        if !status.success() {
            return Err(Error::Build { target, status });
        }
    }
    println!("Native artifacts: {}", artifacts.display());
    Ok(())
}

/// The Apple SDK at `named`, checked against the pin.
fn apple_sdk(named: Option<&Path>) -> Result<PathBuf, Error> {
    /// What the check reads of the SDK's settings.
    #[derive(Deserialize)]
    struct Settings {
        #[serde(rename = "Version")]
        version: String,
    }
    let sdk = std::path::absolute(named.ok_or(Error::NoSdk)?)?;
    let path = sdk.join("SDKSettings.json");
    let unreadable = |reason: String| Error::SdkSettings {
        path: path.clone(),
        reason,
    };
    let bytes = std::fs::read(&path).map_err(|error| unreadable(error.to_string()))?;
    let settings: Settings = serde_json::from_slice(&bytes).map_err(|error| unreadable(error.to_string()))?;
    if settings.version != APPLE_SDK_VERSION {
        return Err(Error::SdkVersion {
            path: sdk,
            found: settings.version,
        });
    }
    Ok(sdk)
}

/// What every target's build shares.
struct Build<'a> {
    repository: &'a Path,
    artifacts: &'a Path,
    sdk: Option<&'a Path>,
}

impl Build<'_> {
    /// Cargo's arguments for `target`'s release build of `executables`.
    fn arguments(target: &str, executables: &[Executable]) -> Vec<OsString> {
        let mut arguments: Vec<OsString> = if windows(target) {
            vec!["xwin".into(), "build".into()]
        } else {
            vec!["zigbuild".into()]
        };
        for argument in ["--release", "--locked", "--target", target] {
            arguments.push(argument.into());
        }
        for executable in executables {
            arguments.push("-p".into());
            arguments.push(executable.name().into());
        }
        arguments
    }

    /// The settings of the pinned inputs, the same here and in the
    /// container.
    fn pins(target: &str) -> Vec<(&'static str, &'static str)> {
        let mut pins = vec![
            ("XWIN_SDK_VERSION", WINDOWS_SDK_VERSION),
            ("XWIN_CRT_VERSION", WINDOWS_CRT_VERSION),
            ("MACOSX_DEPLOYMENT_TARGET", MACOS_MINIMUM),
        ];
        if windows(target) {
            // ring selects clang for Windows arm64: keep cargo-xwin's MSVC
            // driver dialect, with its SDK include flags, and optimization.
            pins.push(("CFLAGS", "--driver-mode=cl /O2"));
        }
        pins
    }

    /// Where cargo-xwin keeps the pinned Windows SDK and C runtime, below
    /// `checkout`.
    fn xwin_cache(checkout: &Path) -> PathBuf {
        checkout
            .join(".cache")
            .join(format!("native-xwin-{WINDOWS_CRT_VERSION}-{WINDOWS_SDK_VERSION}"))
    }

    /// The linker flags of `target`'s build: Windows links its C runtime
    /// statically, and the Apple targets find the SDK's frameworks.
    fn rustflags(target: &str, sdk: Option<&Path>) -> Option<OsString> {
        if windows(target) {
            return Some("-C target-feature=+crt-static".into());
        }
        let sdk = sdk.filter(|_| apple(target))?;
        let mut flags = OsString::from("-L framework=");
        flags.push(sdk.join("System/Library/Frameworks"));
        Some(flags)
    }

    /// The build with this machine's cross tools.
    fn here(&self, target: &str, executables: &[Executable]) -> Command {
        // `cargo xtask` names the toolchain's cargo; a direct run takes the
        // one on PATH.
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let mut command = Command::new(cargo);
        command
            .args(Self::arguments(target, executables))
            .current_dir(self.repository)
            .envs(Self::pins(target))
            .env("CARGO_TARGET_DIR", self.artifacts)
            .env("XWIN_CACHE_DIR", Self::xwin_cache(self.repository));
        match Self::rustflags(target, self.sdk) {
            Some(flags) => command.env("RUSTFLAGS", flags),
            None => command.env_remove("RUSTFLAGS"),
        };
        match self.sdk.filter(|_| apple(target)) {
            Some(sdk) => command.env("SDKROOT", sdk),
            None => command.env_remove("SDKROOT"),
        };
        command
    }

    /// The same build inside `image`, with the checkout, the target
    /// directory, Cargo's download caches and the SDK mounted.
    fn in_container(&self, image: &str, target: &str, executables: &[Executable]) -> Result<Command, Error> {
        let cache = self.repository.join(".cache");
        let registry = cache.join("native-registry");
        let git = cache.join("native-git");
        std::fs::create_dir_all(&registry)?;
        std::fs::create_dir_all(&git)?;
        let checkout = Path::new(CONTAINER_CHECKOUT);
        let mut environment: Vec<(&str, OsString)> = Self::pins(target)
            .into_iter()
            .map(|(name, value)| (name, value.into()))
            .collect();
        environment.push(("CARGO_TARGET_DIR", CONTAINER_ARTIFACTS.into()));
        environment.push(("XWIN_CACHE_DIR", Self::xwin_cache(checkout).into()));
        environment.push(("CARGO_BUILD_JOBS", "4".into()));
        environment.push(("ZIG_GLOBAL_CACHE_DIR", checkout.join(".cache/native-zig").into()));
        let sdk = self.sdk.filter(|_| apple(target)).map(|_| Path::new(CONTAINER_SDK));
        if let Some(flags) = Self::rustflags(target, sdk) {
            environment.push(("RUSTFLAGS", flags));
        }
        if let Some(sdk) = sdk {
            environment.push(("SDKROOT", sdk.into()));
        }
        let mount = |source: &Path, destination: &str| {
            let mut volume = source.as_os_str().to_owned();
            volume.push(":");
            volume.push(destination);
            volume
        };
        let mut command = Command::new("docker");
        command
            .args(["run", "--rm", "-v"])
            .arg(mount(self.repository, CONTAINER_CHECKOUT))
            .arg("-v")
            .arg(mount(self.artifacts, CONTAINER_ARTIFACTS))
            .arg("-v")
            .arg(mount(&registry, "/usr/local/cargo/registry"))
            .arg("-v")
            .arg(mount(&git, "/usr/local/cargo/git"));
        if let Some(sdk) = self.sdk.filter(|_| apple(target)) {
            command.arg("-v").arg(mount(sdk, &format!("{CONTAINER_SDK}:ro")));
        }
        for (name, value) in environment {
            let mut setting = OsString::from(name);
            setting.push("=");
            setting.push(value);
            command.arg("-e").arg(setting);
        }
        command.arg(image).arg("cargo").args(Self::arguments(target, executables));
        Ok(command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_apple_sdk_is_checked_against_the_pin_before_anything_builds() {
        let root = tempfile::tempdir().unwrap();
        let sdk = |name: &str, settings: Option<&str>| {
            let directory = root.path().join(name);
            std::fs::create_dir(&directory).unwrap();
            if let Some(settings) = settings {
                std::fs::write(directory.join("SDKSettings.json"), settings).unwrap();
            }
            directory
        };
        let pinned = sdk("pinned", Some(r#"{"CanonicalName":"macosx15.4","Version":"15.4"}"#));
        assert_eq!(apple_sdk(Some(&pinned)).unwrap(), pinned);
        let older = sdk("older", Some(r#"{"CanonicalName":"macosx15.2","Version":"15.2"}"#));
        let refused = apple_sdk(Some(&older));
        assert!(matches!(&refused, Err(Error::SdkVersion { found, .. }) if found == "15.2"), "{refused:?}");
        let empty = sdk("empty", None);
        assert!(matches!(apple_sdk(Some(&empty)), Err(Error::SdkSettings { .. })));
        assert!(matches!(apple_sdk(None), Err(Error::NoSdk)));
    }
}
