//! Builds and embeds the iOS helper on Apple silicon
//! (`crates/command-package-mobile/src/ios/mod.rs`), and prepares the Android
//! helper on every target.
//!
//! # The iOS helper
//!
//! The build produces two things:
//!
//! - `demi-ios-sim`, the Swift package in `ios-sim/` linking idb's v1.7.6
//!   libraries.
//! - idb's guest bridge `SimulatorFrameworkBridge-iOS`, built from the same
//!   release's source with Xcode and XcodeGen.
//!
//! Both downloads are pinned by their SHA-256. The helper is signed ad hoc; the
//! bridge keeps the signature its build gives it.
//!
//! Both go, with the stub frameworks the helper loads from beside itself and
//! idb's MIT notice (`ios-sim/LICENSE-idb`, which a release's third-party
//! notices carry too), into a tar archive compressed with zstd, which the
//! program carries.
//!
//! idb's libraries are built for macOS on arm64 only, so only an
//! `aarch64-apple-darwin` build carries the helper. Every build of it,
//! development or release, builds the helper here, keeping the downloads and
//! the Swift and Xcode builds under the target directory's `ios-sim/`.
//!
//! A Mac without Xcode or its iOS SDK builds no helper and says why. Then `demi
//! mobile list` says iOS needs Xcode. A release build sets
//! `DEMI_MOBILE_IOS_HELPER=required`, which makes that an error.
//!
//! # The Android helper
//!
//! On every target the build also:
//!
//! - compiles the part of the Android Emulator's gRPC interface the Android
//!   driver uses (`android/`), with protox, so no `protoc` is needed;
//! - fetches the Android helper, openatx's `u2.jar` (MIT), pinned by its
//!   SHA-256, which the program carries as it is
//!   (`crates/command-package-mobile/src/android/mod.rs`).
//!
//! A build that cannot fetch it carries none and says why;
//! `DEMI_MOBILE_ANDROID_HELPER=required` makes that an error.

use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

use sha2::{Digest, Sha256};

/// The one target whose build carries the helper.
const TARGET: &str = "aarch64-apple-darwin";
/// What a release build sets so a build without the helper fails.
const REQUIRED: &str = "DEMI_MOBILE_IOS_HELPER";

/// idb's v1.7.6 source, whose guest bridge the build compiles.
const IDB_SOURCE: Pinned = Pinned {
    url: "https://github.com/facebook/idb/archive/refs/tags/v1.7.6.tar.gz",
    file: "idb-v1.7.6.tar.gz",
    sha256: "f8fe969e067c55cc5a5fa6a43a231c45697fec1df343c689e1263968a041386f",
};
/// The directory the source archive unpacks into.
const IDB_DIRECTORY: &str = "idb-1.7.6";
/// XcodeGen, which writes the bridge's Xcode project from its project.yml.
const XCODEGEN: Pinned = Pinned {
    url: "https://github.com/yonaskolb/XcodeGen/releases/download/2.46.0/xcodegen.zip",
    file: "xcodegen-2.46.0.zip",
    sha256: "4d9e34b62172d645eed6457cac13fc222569974098ef4ee9c3368bedf0196806",
};
/// The guest bridge, in the archive's `Resources/`, where idb looks for it
/// beside the helper.
const BRIDGE: &str = "SimulatorFrameworkBridge-iOS";

/// What a release build sets so a build without the Android helper fails.
const ANDROID_REQUIRED: &str = "DEMI_MOBILE_ANDROID_HELPER";
/// openatx's UI automation server, run inside an emulator with
/// `app_process`.
const U2_JAR: Pinned = Pinned {
    url: "https://github.com/openatx/android-uiautomator-server-jar/releases/download/0.4.0/u2.jar",
    file: "u2-0.4.0.jar",
    sha256: "0b74e83c55f443539a9f76f5ce023a51466b764b1100e4097a897053fdfc0eb6",
};

/// A download the build trusts by its SHA-256.
struct Pinned {
    url: &'static str,
    file: &'static str,
    sha256: &'static str,
}

fn main() {
    println!("cargo::rustc-check-cfg=cfg(demi_ios_helper)");
    println!("cargo::rustc-check-cfg=cfg(demi_android_helper)");
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=ios-sim");
    println!("cargo::rerun-if-changed=android");
    println!("cargo::rerun-if-env-changed={REQUIRED}");
    println!("cargo::rerun-if-env-changed={ANDROID_REQUIRED}");
    android();
    if env::var("TARGET").ok().as_deref() != Some(TARGET) {
        return;
    }
    let required = env::var(REQUIRED).is_ok_and(|value| value == "required");
    if let Some(missing) = missing_xcode() {
        if required {
            panic!("the iOS helper needs Xcode with its iOS SDK: {missing}");
        }
        println!("cargo::warning=demi-mobile carries no iOS helper: {missing}");
        return;
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let cache = target_directory(&out).join("ios-sim");
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let archive = build(&manifest.join("ios-sim"), &cache)
        .unwrap_or_else(|error| panic!("the iOS helper did not build: {error}"));
    let digest: String = Sha256::digest(&archive).iter().map(|byte| format!("{byte:02x}")).collect();
    fs::write(out.join("demi-ios-sim.tar.zst"), &archive).expect("the archive is written");
    println!("cargo::rustc-env=DEMI_IOS_SIM_SHA256={digest}");
    println!("cargo::rustc-cfg=demi_ios_helper");
}

/// The emulator's gRPC client and server, and the Android helper.
fn android() {
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let proto = manifest.join("android");
    let descriptors = protox::compile([proto.join("emulator_controller.proto")], [&proto])
        .unwrap_or_else(|error| panic!("the emulator's interface does not compile: {error}"));
    tonic_prost_build::configure()
        .build_client(true)
        // Pictures arrive as `Bytes`, which the encoder and the
        // screenshot take without a copy.
        .bytes(".")
        // The tests' stub emulator serves it.
        .build_server(true)
        .server_mod_attribute("android.emulation.control", "#[cfg(feature = \"testing\")]")
        .compile_fds(descriptors)
        .unwrap_or_else(|error| panic!("the emulator's interface was not generated: {error}"));
    let cache = target_directory(&out).join("android-helper");
    let required = env::var(ANDROID_REQUIRED).is_ok_and(|value| value == "required");
    let fetched = fs::create_dir_all(&cache)
        .map_err(Failure::from)
        .and_then(|()| fetch(&cache, &U2_JAR));
    match fetched {
        Ok(jar) => {
            fs::copy(&jar, out.join("u2.jar")).expect("the Android helper is copied");
            println!("cargo::rustc-env=DEMI_U2_JAR_SHA256={}", U2_JAR.sha256);
            println!("cargo::rustc-cfg=demi_android_helper");
        }
        Err(error) if required => panic!("the Android helper was not fetched: {error}"),
        Err(error) => println!("cargo::warning=demi-mobile carries no Android helper: {error}"),
    }
}

/// Why this Mac cannot build the helper: no Xcode, or no iOS SDK in it.
fn missing_xcode() -> Option<String> {
    let found = |command: &mut Command| command.output().is_ok_and(|output| output.status.success());
    if !found(Command::new("xcodebuild").arg("-version")) {
        return Some("Xcode is not installed, or xcode-select points at the Command Line Tools".into());
    }
    if !found(Command::new("xcrun").args(["--sdk", "iphonesimulator", "--show-sdk-path"])) {
        return Some("Xcode has no iOS Simulator SDK; install the iOS platform in Xcode's settings".into());
    }
    None
}

/// The target directory `out` is in: the one with Cargo's cache tag.
fn target_directory(out: &Path) -> PathBuf {
    out.ancestors()
        .find(|directory| directory.join("CACHEDIR.TAG").exists())
        .unwrap_or(out)
        .to_path_buf()
}

type Failure = Box<dyn std::error::Error>;

/// Builds the helper and the bridge below `cache` and answers the archive
/// that carries them.
fn build(package: &Path, cache: &Path) -> Result<Vec<u8>, Failure> {
    fs::create_dir_all(cache)?;
    let source = cache.join(IDB_DIRECTORY);
    if !source.join("LICENSE").exists() {
        let archive = fetch(cache, &IDB_SOURCE)?;
        let unpacking = cache.join(format!("{IDB_DIRECTORY}.unpacking"));
        if unpacking.exists() {
            fs::remove_dir_all(&unpacking)?;
        }
        fs::create_dir_all(&unpacking)?;
        tar::Archive::new(flate2::read::GzDecoder::new(fs::File::open(&archive)?)).unpack(&unpacking)?;
        // A source without its license is a partial one, such as a CI
        // cache that kept some of its files: it is replaced whole.
        if source.exists() {
            fs::remove_dir_all(&source)?;
        }
        fs::rename(unpacking.join(IDB_DIRECTORY), &source)?;
        fs::remove_dir_all(&unpacking)?;
    }
    let xcodegen = cache.join("xcodegen/bin/xcodegen");
    if !xcodegen.exists() {
        let archive = fetch(cache, &XCODEGEN)?;
        run(Command::new("/usr/bin/ditto").args(["-x", "-k"]).arg(&archive).arg(cache))?;
    }
    // The bridge: its project from project.yml, then Xcode's build of it.
    let bridge_source = source.join("SimulatorFrameworkBridge");
    run(Command::new(&xcodegen)
        .args(["generate", "--quiet", "--spec", "project.yml"])
        .current_dir(&bridge_source))?;
    let derived = cache.join("bridge");
    run(xcode(Command::new("xcodebuild"))
        .arg("-project")
        .arg(bridge_source.join("SimulatorFrameworkBridge.xcodeproj"))
        .args(["-scheme", BRIDGE, "-sdk", "iphonesimulator", "-configuration", "Release"])
        .arg("-derivedDataPath")
        .arg(&derived)
        .args(["ARCHS=arm64", "ONLY_ACTIVE_ARCH=NO", "-quiet", "build"]))?;
    let bridge = derived.join("Build/Products/Release-iphonesimulator").join(BRIDGE);
    // The helper.
    let scratch = cache.join("swift");
    let swift = |extra: &[&str]| {
        let mut command = xcode(Command::new("swift"));
        command
            .args(["build", "-c", "release", "--arch", "arm64"])
            .arg("--package-path")
            .arg(package)
            .arg("--scratch-path")
            .arg(&scratch)
            // The stub frameworks are beside the helper. Swift 6.2's
            // packages looked there by default and Swift 6.4's do not, so
            // the build says it.
            .args(["-Xlinker", "-rpath", "-Xlinker", "@executable_path"])
            .args(extra);
        command
    };
    run(&mut swift(&[]))?;
    let shown = swift(&["--show-bin-path"]).output()?;
    let products = PathBuf::from(String::from_utf8(shown.stdout)?.trim());
    let helper = products.join("demi-ios-sim");
    // The archive's contents, signed and packed.
    let staging = cache.join("staging");
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir_all(staging.join("Resources"))?;
    fs::copy(&helper, staging.join("demi-ios-sim"))?;
    run(Command::new("codesign").args(["--force", "--sign", "-"]).arg(staging.join("demi-ios-sim")))?;
    // idb's stub frameworks, which the helper loads from beside itself
    // (`@loader_path`), their symlinks kept.
    for entry in fs::read_dir(&products)? {
        let path = entry?.path();
        if path.extension().is_some_and(|extension| extension == "framework") {
            let name = path.file_name().expect("a framework has a name");
            run(Command::new("/usr/bin/ditto").arg(&path).arg(staging.join(name)))?;
        }
    }
    run(Command::new("/usr/bin/ditto").arg(&bridge).arg(staging.join("Resources").join(BRIDGE)))?;
    fs::copy(package.join("LICENSE-idb"), staging.join("LICENSE-idb"))?;
    pack(&staging)
}

/// A command of Xcode's tools as a Mac's own build runs it: without the
/// variables Cargo sets for the Rust build, which name its SDK and target.
fn xcode(mut command: Command) -> Command {
    for variable in ["SDKROOT", "MACOSX_DEPLOYMENT_TARGET", "TARGET", "HOST", "CC", "CFLAGS", "LDFLAGS"] {
        command.env_remove(variable);
    }
    command
}

/// The pinned download in `cache`, fetched when it is not there or is not
/// the pinned file, with curl, which macOS, Windows 10 and later and every
/// Linux build machine have.
fn fetch(cache: &Path, pinned: &Pinned) -> Result<PathBuf, Failure> {
    let path = cache.join(pinned.file);
    if fs::read(&path).is_ok_and(|bytes| sha256(&bytes) == pinned.sha256) {
        return Ok(path);
    }
    let partial = cache.join(format!("{}.part", pinned.file));
    let curl = if cfg!(target_os = "macos") { "/usr/bin/curl" } else { "curl" };
    run(Command::new(curl)
        .args(["--fail", "--location", "--silent", "--show-error", "--retry", "5", "--retry-all-errors"])
        .arg("--output")
        .arg(&partial)
        .arg(pinned.url))?;
    let digest = sha256(&fs::read(&partial)?);
    if digest != pinned.sha256 {
        fs::remove_file(&partial)?;
        return Err(format!("{} has SHA-256 {digest}, not the pinned {}", pinned.url, pinned.sha256).into());
    }
    fs::rename(&partial, &path)?;
    Ok(path)
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The files below `directory` as a tar archive compressed with zstd, each
/// with its mode, so the helper stays executable.
fn pack(directory: &Path) -> Result<Vec<u8>, Failure> {
    let mut builder = tar::Builder::new(Vec::new());
    builder.follow_symlinks(false);
    builder.append_dir_all(".", directory)?;
    let tar = builder.into_inner()?;
    let mut compressed = Vec::new();
    let mut encoder = zstd::Encoder::new(&mut compressed, 19)?;
    encoder.write_all(&tar)?;
    encoder.finish()?;
    Ok(compressed)
}

fn run(command: &mut Command) -> Result<(), Failure> {
    let status = command.status()?;
    if !status.success() {
        return Err(format!("{command:?} ended with {status}").into());
    }
    Ok(())
}
