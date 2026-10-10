//! `xtask native build` (`builds-and-releases.md` § Executables and
//! targets, § Cross builds): one release build per target. A target of the
//! machine's own platform builds with that platform's toolchain, Apple's on
//! a Mac and MSVC on Windows, checked against the pins; Linux keeps
//! cargo-zigbuild for its static musl build; a target of another platform
//! builds with the cross tools, cargo-zigbuild for the Apple and Linux
//! targets and cargo-xwin for Windows, here or inside the build container's
//! image.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use super::{Error, Executable, apple, linux, windows};

/// The Apple SDK the Apple targets are built against: the one the Command
/// Line Tools and Xcode 26.6 install, on a developer's Mac and on the
/// release workflow's `macos-26` runner alike.
pub(super) const APPLE_SDK_VERSION: &str = "26.5";
/// The Windows SDK and C runtime: what cargo-xwin downloads from another
/// platform, and what a Windows build's Visual Studio environment must
/// select.
const WINDOWS_SDK_VERSION: &str = "10.0.26100";
const WINDOWS_CRT_VERSION: &str = "14.44.17.14";
/// The MSVC toolset of that C runtime, as `VCToolsVersion` begins.
const MSVC_TOOLSET: &str = "14.44.";
/// The oldest macOS the Apple builds run on.
const MACOS_MINIMUM: &str = "13.0";
/// The build container's paths: the checkout, the Cargo target directory
/// (not `/output`, which clang-cl reads as an output flag) and the SDK.
const CONTAINER_CHECKOUT: &str = "/work";
const CONTAINER_ARTIFACTS: &str = "/build";
const CONTAINER_SDK: &str = "/sdk";

/// The variable that names the workspace version a build carries in place
/// of `Cargo.toml`'s, which `demi_shared_artifacts::WORKSPACE_VERSION` reads.
const WORKSPACE_VERSION: &str = "DEMI_WORKSPACE_VERSION";
/// The release profile of a Linux executable's build: unstripped and with
/// its line tables in the executable, since stripping leaves a packed debug
/// file unusable there; [`split`] moves them into a debug file instead
/// (`builds-and-releases.md` § Build profiles).
const LINUX_PROFILE: [(&str, &str); 2] = [
    ("CARGO_PROFILE_RELEASE_STRIP", "false"),
    ("CARGO_PROFILE_RELEASE_SPLIT_DEBUGINFO", "off"),
];

#[derive(clap::Args)]
pub struct Options {
    /// An executable to build; repeat for several [default: the runner and
    /// the command programs].
    #[arg(long = "package", value_name = "CRATE")]
    pub(crate) packages: Vec<Executable>,
    /// A target to build for; repeat for several [default: every target of
    /// each executable].
    #[arg(long = "target", value_name = "TRIPLE", value_parser = super::target)]
    pub(crate) targets: Vec<&'static str>,
    /// The Cargo target directory the builds write [default:
    /// .cache/native-target in the repository].
    #[arg(long, value_name = "DIRECTORY")]
    pub(crate) artifacts: Option<PathBuf>,
    /// The Apple SDK, which the Apple targets need [default: the one xcrun
    /// --show-sdk-path names on a Mac].
    #[arg(long, env = "SDKROOT", value_name = "DIRECTORY")]
    pub(crate) sdk: Option<PathBuf>,
    /// Builds inside this image of scripts/native/Dockerfile, for a machine
    /// without the cross tools.
    #[arg(long, value_name = "IMAGE")]
    pub(crate) container: Option<String>,
    /// The workspace version the executables carry, which `xtask deploy`
    /// names for a development build [default: the workspace version].
    #[arg(skip)]
    pub(crate) version: Option<String>,
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
        Some(apple_sdk(options.sdk.as_deref(), xcrun_sdk)?)
    } else {
        None
    };
    // The container is a Linux machine, whatever this one is.
    let host = match &options.container {
        Some(_) => Platform::Linux,
        None => Platform::HOST,
    };
    if host == Platform::Windows && targets.iter().any(|target| windows(target)) {
        check_msvc(|name| std::env::var(name).ok())?;
    }
    let artifacts = super::artifacts(options.artifacts.as_deref())?;
    std::fs::create_dir_all(&artifacts)?;
    let build = Build {
        repository: &repository,
        artifacts: &artifacts,
        sdk: sdk.as_deref(),
        host,
        version: options.version.as_deref(),
    };
    for target in targets {
        let built: Vec<Executable> = executables
            .iter()
            .copied()
            .filter(|executable| executable.targets().contains(&target))
            .collect();
        println!("Native build: {target}");
        let packages = Packages::Executables(&built);
        let mut command = match &options.container {
            Some(image) => build.in_container(image, target, packages)?,
            None => build.here(target, packages),
        };
        let status = command.status()?;
        if !status.success() {
            return Err(Error::Build { target, status });
        }
        if linux(target) {
            for executable in &built {
                split(&super::built(&artifacts, *executable, target))?;
            }
        }
    }
    println!("Native artifacts: {}", artifacts.display());
    Ok(())
}

/// Moves the line tables and symbols of the Linux executable at `executable`
/// into `<executable>.debug` beside it, which the executable then names, so that
/// a crash's addresses can be read without shipping them. Cargo copies the
/// unstripped executable there again at each build, so the split starts
/// from it every time; the stripped one replaces it by a rename, which
/// leaves Cargo's own copy, the file it links there, unstripped.
fn split(executable: &Path) -> Result<(), Error> {
    let tool = objcopy()?;
    let suffixed = |suffix: &str| {
        let mut path = executable.as_os_str().to_owned();
        path.push(suffix);
        PathBuf::from(path)
    };
    let debug = suffixed(".debug");
    let stripped = suffixed(".stripped");
    let mut link = OsString::from("--add-gnu-debuglink=");
    link.push(&debug);
    let steps: [&[&std::ffi::OsStr]; 2] = [
        &["--only-keep-debug".as_ref(), executable.as_ref(), debug.as_ref()],
        // All symbols, not only the debug sections: the symbol table and its
        // names, which the debug file keeps, made the x86_64 runner 54.6 MB
        // rather than 35.5 MB (12.3 MB rather than 10.5 MB compressed).
        &["--strip-all".as_ref(), &link, executable.as_ref(), stripped.as_ref()],
    ];
    for arguments in steps {
        let status = Command::new(&tool)
            .args(arguments)
            .status()
            .map_err(|source| Error::Objcopy {
                tool: tool.clone(),
                source,
            })?;
        if !status.success() {
            return Err(Error::Split {
                path: executable.to_owned(),
                status,
            });
        }
    }
    std::fs::rename(&stripped, executable)?;
    Ok(())
}

/// The pinned toolchain's `llvm-objcopy`, from its `llvm-tools` component
/// (`rust-toolchain.toml`), beside the host's target libraries, so every
/// machine splits with the same tool whatever its own toolchains are
/// (`builds-and-releases.md` § Release profile).
fn objcopy() -> Result<PathBuf, Error> {
    let tool = PathBuf::from("llvm-objcopy");
    let output = Command::new("rustc")
        .args(["--print", "target-libdir"])
        .output()
        .map_err(|source| Error::Objcopy {
            tool: tool.clone(),
            source,
        })?;
    let libdir = String::from_utf8_lossy(&output.stdout);
    Ok(Path::new(libdir.trim()).with_file_name("bin").join(tool))
}

/// The Apple SDK at `named` (`--sdk` or `SDKROOT`), or else the one
/// `found` names, as Apple's own tools find it; checked against the pin
/// either way.
fn apple_sdk(
    named: Option<&Path>,
    found: impl FnOnce() -> Option<PathBuf>,
) -> Result<PathBuf, Error> {
    /// What the check reads of the SDK's settings.
    #[derive(Deserialize)]
    struct Settings {
        #[serde(rename = "Version")]
        version: String,
    }
    let sdk = match named {
        Some(named) => named.to_path_buf(),
        None => found().ok_or(Error::NoSdk)?,
    };
    let sdk = std::path::absolute(sdk)?;
    let path = sdk.join("SDKSettings.json");
    let unreadable = |reason: String| Error::SdkSettings {
        path: path.clone(),
        reason,
    };
    let bytes = std::fs::read(&path).map_err(|error| unreadable(error.to_string()))?;
    let settings: Settings =
        serde_json::from_slice(&bytes).map_err(|error| unreadable(error.to_string()))?;
    if settings.version != APPLE_SDK_VERSION {
        return Err(Error::SdkVersion {
            path: sdk,
            found: settings.version,
        });
    }
    Ok(sdk)
}

/// The SDK `xcrun --show-sdk-path` names on a Mac; none elsewhere, or when
/// xcrun cannot name one. Its failure is not an error of its own: the build
/// then stops with [`Error::NoSdk`], which says how to name the SDK.
fn xcrun_sdk() -> Option<PathBuf> {
    if Platform::HOST != Platform::Mac {
        return None;
    }
    let output = Command::new("xcrun")
        .arg("--show-sdk-path")
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    let path = String::from_utf8(output.stdout).ok()?;
    let path = path.trim_end();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

/// Checks that the Visual Studio environment a Windows build runs in,
/// whose variables `variable` reads, selects the pinned MSVC toolset and
/// Windows SDK.
fn check_msvc(variable: impl Fn(&str) -> Option<String>) -> Result<(), Error> {
    let toolset = variable("VCToolsVersion").ok_or(Error::NoMsvc)?;
    let sdk = variable("WindowsSDKVersion").ok_or(Error::NoMsvc)?;
    if !toolset.starts_with(MSVC_TOOLSET) || !sdk.starts_with(WINDOWS_SDK_VERSION) {
        return Err(Error::MsvcVersion {
            toolset,
            sdk,
            pinned: format!("{MSVC_TOOLSET}x and {WINDOWS_SDK_VERSION}"),
        });
    }
    Ok(())
}

/// The platform a build runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Platform {
    Mac,
    Linux,
    Windows,
}

impl Platform {
    /// This machine's platform.
    const HOST: Self = if cfg!(target_os = "macos") {
        Self::Mac
    } else if cfg!(windows) {
        Self::Windows
    } else {
        Self::Linux
    };
}

/// What compiles and links a target's build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    /// Cargo with the platform's own toolchain: Apple's on a Mac, MSVC on
    /// Windows.
    Cargo,
    /// cargo-zigbuild: the Linux targets everywhere, since Zig carries the
    /// musl that `aws-lc-sys` builds against, and the Apple targets from
    /// another platform.
    Zig,
    /// cargo-xwin: the Windows targets from another platform.
    Xwin,
}

impl Tool {
    /// The tool that builds `target` on `host`.
    fn of(host: Platform, target: &str) -> Self {
        match (host, apple(target), windows(target)) {
            (Platform::Mac, true, _) | (Platform::Windows, _, true) => Self::Cargo,
            (_, _, true) => Self::Xwin,
            _ => Self::Zig,
        }
    }
}

/// Builds `xtask` for the Linux `target` with this machine's cross tools,
/// for a Linux builder of the Cloud image, without the commands only a
/// developer runs, as the release workflow builds it
/// (`builds-and-releases.md` § Executables and targets); with the workspace
/// version `version` when named. Returns the executable's path.
pub fn xtask(target: &'static str, version: Option<&str>) -> Result<PathBuf, Error> {
    let repository = crate::repository();
    let artifacts = super::artifacts(None)?;
    std::fs::create_dir_all(&artifacts)?;
    let build = Build {
        repository: &repository,
        artifacts: &artifacts,
        sdk: None,
        host: Platform::HOST,
        version,
    };
    println!("Native build: xtask for {target}");
    let status = build.here(target, Packages::Xtask).status()?;
    if !status.success() {
        return Err(Error::Build { target, status });
    }
    Ok(artifacts.join(target).join("release").join("xtask"))
}

/// What a build compiles.
#[derive(Debug, Clone, Copy)]
enum Packages<'a> {
    Executables(&'a [Executable]),
    /// `xtask` without its default features, the developer's commands.
    Xtask,
}

/// What every target's build shares.
struct Build<'a> {
    repository: &'a Path,
    artifacts: &'a Path,
    sdk: Option<&'a Path>,
    host: Platform,
    /// The workspace version the build carries, unless `Cargo.toml`'s.
    version: Option<&'a str>,
}

impl Build<'_> {
    /// Cargo's arguments for `target`'s release build of `packages`.
    fn arguments(&self, target: &str, packages: Packages<'_>) -> Vec<OsString> {
        let mut arguments: Vec<OsString> = match Tool::of(self.host, target) {
            Tool::Cargo => vec!["build".into()],
            Tool::Zig => vec!["zigbuild".into()],
            Tool::Xwin => vec!["xwin".into(), "build".into()],
        };
        for argument in ["--release", "--locked", "--target", target] {
            arguments.push(argument.into());
        }
        match packages {
            Packages::Executables(executables) => {
                for executable in executables {
                    arguments.push("-p".into());
                    arguments.push(executable.name().into());
                }
            }
            Packages::Xtask => {
                for argument in ["-p", "xtask", "--no-default-features"] {
                    arguments.push(argument.into());
                }
            }
        }
        arguments
    }

    /// The settings of the pinned inputs, the same here and in the
    /// container.
    fn pins(&self, target: &str) -> Vec<(&'static str, &'static str)> {
        let mut pins = vec![
            ("XWIN_SDK_VERSION", WINDOWS_SDK_VERSION),
            ("XWIN_CRT_VERSION", WINDOWS_CRT_VERSION),
            ("MACOSX_DEPLOYMENT_TARGET", MACOS_MINIMUM),
        ];
        if windows(target) {
            match Tool::of(self.host, target) {
                // aws-lc-sys, rustls's provider, compiles C with
                // cargo-xwin's clang: keep its MSVC driver dialect, with its
                // SDK include flags, and optimization.
                Tool::Xwin => pins.push(("CFLAGS", "--driver-mode=cl /O2")),
                // On Windows arm64 the crate requires clang-cl, which the
                // Visual Studio installation carries.
                Tool::Cargo if target.starts_with("aarch64") => {
                    pins.push(("CC_aarch64_pc_windows_msvc", "clang-cl"));
                    pins.push(("CXX_aarch64_pc_windows_msvc", "clang-cl"));
                }
                Tool::Cargo | Tool::Zig => {}
            }
            // Its x86-64 assembly needs NASM, which neither the cross tools
            // nor Visual Studio carry; the crate's prebuilt NASM objects
            // take its place.
            pins.push(("AWS_LC_SYS_PREBUILT_NASM", "1"));
        }
        pins
    }

    /// Where cargo-xwin keeps the pinned Windows SDK and C runtime, below
    /// `checkout`.
    fn xwin_cache(checkout: &Path) -> PathBuf {
        checkout.join(".cache").join(format!(
            "native-xwin-{WINDOWS_CRT_VERSION}-{WINDOWS_SDK_VERSION}"
        ))
    }

    /// The linker flags of `target`'s build: Windows links its C runtime
    /// statically, and the Apple targets find the SDK's frameworks when Zig
    /// links them; Apple's linker finds them in `SDKROOT`.
    fn rustflags(&self, target: &str, sdk: Option<&Path>) -> Option<OsString> {
        if windows(target) {
            return Some("-C target-feature=+crt-static".into());
        }
        if Tool::of(self.host, target) != Tool::Zig {
            return None;
        }
        let sdk = sdk.filter(|_| apple(target))?;
        let mut flags = OsString::from("-L framework=");
        flags.push(sdk.join("System/Library/Frameworks"));
        Some(flags)
    }

    /// The settings of `target`'s build of `packages` that change Cargo's
    /// release profile: [`LINUX_PROFILE`] for a Linux executable's.
    fn profile(target: &str, packages: Packages<'_>) -> &'static [(&'static str, &'static str)] {
        match packages {
            Packages::Executables(_) if linux(target) => &LINUX_PROFILE,
            Packages::Executables(_) | Packages::Xtask => &[],
        }
    }

    /// The build with this machine's cross tools.
    fn here(&self, target: &str, packages: Packages<'_>) -> Command {
        // `cargo run` names the toolchain's cargo; `bun xtask` takes the one
        // on PATH.
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let mut command = Command::new(cargo);
        command
            .args(self.arguments(target, packages))
            .current_dir(self.repository)
            .envs(self.pins(target))
            .envs(Self::profile(target, packages).iter().copied())
            .env("CARGO_TARGET_DIR", self.artifacts)
            .env("XWIN_CACHE_DIR", Self::xwin_cache(self.repository));
        match self.rustflags(target, self.sdk) {
            Some(flags) => command.env("RUSTFLAGS", flags),
            None => command.env_remove("RUSTFLAGS"),
        };
        match self.sdk.filter(|_| apple(target)) {
            Some(sdk) => command.env("SDKROOT", sdk),
            None => command.env_remove("SDKROOT"),
        };
        match self.version {
            Some(version) => command.env(WORKSPACE_VERSION, version),
            None => command.env_remove(WORKSPACE_VERSION),
        };
        command
    }

    /// The same build inside `image`, with the checkout, the target
    /// directory, Cargo's download caches and the SDK mounted.
    fn in_container(
        &self,
        image: &str,
        target: &str,
        packages: Packages<'_>,
    ) -> Result<Command, Error> {
        let cache = self.repository.join(".cache");
        let registry = cache.join("native-registry");
        let git = cache.join("native-git");
        std::fs::create_dir_all(&registry)?;
        std::fs::create_dir_all(&git)?;
        let checkout = Path::new(CONTAINER_CHECKOUT);
        let mut environment: Vec<(&str, OsString)> = self
            .pins(target)
            .into_iter()
            .chain(Self::profile(target, packages).iter().copied())
            .map(|(name, value)| (name, value.into()))
            .collect();
        environment.push(("CARGO_TARGET_DIR", CONTAINER_ARTIFACTS.into()));
        environment.push(("XWIN_CACHE_DIR", Self::xwin_cache(checkout).into()));
        environment.push(("CARGO_BUILD_JOBS", "4".into()));
        environment.push((
            "ZIG_GLOBAL_CACHE_DIR",
            checkout.join(".cache/native-zig").into(),
        ));
        let sdk = self
            .sdk
            .filter(|_| apple(target))
            .map(|_| Path::new(CONTAINER_SDK));
        if let Some(flags) = self.rustflags(target, sdk) {
            environment.push(("RUSTFLAGS", flags));
        }
        if let Some(sdk) = sdk {
            environment.push(("SDKROOT", sdk.into()));
        }
        if let Some(version) = self.version {
            environment.push((WORKSPACE_VERSION, version.into()));
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
            command
                .arg("-v")
                .arg(mount(sdk, &format!("{CONTAINER_SDK}:ro")));
        }
        for (name, value) in environment {
            let mut setting = OsString::from(name);
            setting.push("=");
            setting.push(value);
            command.arg("-e").arg(setting);
        }
        command
            .arg(image)
            .arg("cargo")
            .args(self.arguments(target, packages));
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
        let pinned = sdk(
            "pinned",
            Some(r#"{"CanonicalName":"macosx26.5","Version":"26.5"}"#),
        );
        let none = || None;
        assert_eq!(apple_sdk(Some(&pinned), none).unwrap(), pinned);
        // Without --sdk or SDKROOT, the SDK xcrun names, checked the same
        // way.
        assert_eq!(
            apple_sdk(None, || Some(pinned.clone())).unwrap(),
            pinned
        );
        let older = sdk(
            "older",
            Some(r#"{"CanonicalName":"macosx15.2","Version":"15.2"}"#),
        );
        let refused = apple_sdk(Some(&older), none);
        assert!(
            matches!(&refused, Err(Error::SdkVersion { found, .. }) if found == "15.2"),
            "{refused:?}"
        );
        let empty = sdk("empty", None);
        assert!(matches!(
            apple_sdk(Some(&empty), none),
            Err(Error::SdkSettings { .. })
        ));
        let xcrun_older = apple_sdk(None, || Some(older.clone()));
        assert!(
            matches!(&xcrun_older, Err(Error::SdkVersion { found, .. }) if found == "15.2"),
            "{xcrun_older:?}"
        );
        assert!(matches!(apple_sdk(None, none), Err(Error::NoSdk)));
    }

    #[test]
    fn a_target_builds_with_its_own_platforms_toolchain_and_cross_tools_elsewhere() {
        let mac = "aarch64-apple-darwin";
        let linux = "x86_64-unknown-linux-musl";
        let windows = "aarch64-pc-windows-msvc";
        assert_eq!(Tool::of(Platform::Mac, mac), Tool::Cargo);
        assert_eq!(Tool::of(Platform::Mac, linux), Tool::Zig);
        assert_eq!(Tool::of(Platform::Mac, windows), Tool::Xwin);
        assert_eq!(Tool::of(Platform::Linux, mac), Tool::Zig);
        assert_eq!(Tool::of(Platform::Linux, linux), Tool::Zig);
        assert_eq!(Tool::of(Platform::Windows, windows), Tool::Cargo);
        assert_eq!(Tool::of(Platform::Windows, linux), Tool::Zig);
    }

    #[test]
    fn a_windows_build_runs_only_in_the_pinned_visual_studio_environment() {
        fn environment(
            toolset: Option<&'static str>,
            sdk: Option<&'static str>,
        ) -> impl Fn(&str) -> Option<String> {
            move |name| match name {
                "VCToolsVersion" => toolset.map(str::to_owned),
                "WindowsSDKVersion" => sdk.map(str::to_owned),
                _ => None,
            }
        }
        check_msvc(environment(Some("14.44.35207"), Some("10.0.26100.0\\"))).unwrap();
        assert!(matches!(
            check_msvc(environment(None, Some("10.0.26100.0\\"))),
            Err(Error::NoMsvc)
        ));
        assert!(matches!(
            check_msvc(environment(Some("14.43.34808"), Some("10.0.26100.0\\"))),
            Err(Error::MsvcVersion { .. })
        ));
        assert!(matches!(
            check_msvc(environment(Some("14.44.35207"), Some("10.0.22621.0\\"))),
            Err(Error::MsvcVersion { .. })
        ));
    }
}
