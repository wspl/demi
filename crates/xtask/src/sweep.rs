//! `xtask sweep` (`builds-and-releases.md`): removes the build products no
//! current build uses from the checkout's Cargo build directory.
//!
//! Cargo never removes a unit it no longer builds. Every change of
//! `Cargo.lock`, of a feature or of a flag gives the affected units new
//! hashes, and the old ones stay: `debug/build/<package>/<hash>/` holds one
//! unit each (its `fingerprint/` and its `out/`), and
//! `debug/incremental/<crate>-<id>/` holds one compiled unit's incremental
//! cache. The sweep asks Cargo for the products of the builds this repository
//! runs, keeps the unit directories they are in and the incremental caches
//! those units wrote, and removes every other unit directory and incremental
//! cache of the `debug` profile. Every other Cargo target directory, such as
//! the cross builds' under `.cache`, is only reported with its size.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs::{File, Metadata};
use std::io::{BufRead, BufReader};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::{Duration, SystemTime};

use serde::Deserialize;

/// The builds whose products the sweep keeps, as `cargo` arguments: the one
/// selection's type check and test build (`builds-and-releases.md`
/// § Validation), and the build `bun xtask` makes before every command,
/// `bun xtask dev` and `bun run test` among them.
const BUILDS: [&[&str]; 3] = [
    &[
        "check",
        "--workspace",
        "--all-targets",
        "--features",
        "demi-runner/test-fixtures",
    ],
    &[
        "test",
        "--workspace",
        "--features",
        "demi-runner/test-fixtures",
        "--no-run",
    ],
    &[
        "build",
        "--workspace",
        "--all-targets",
        "--features",
        "demi-runner/test-fixtures",
    ],
];

/// The profile directory the builds write, under the build directory.
const PROFILE: &str = "debug";

/// How far an incremental session's start may lie outside the time its unit
/// was compiled in and still count as that compilation's: the coarsest
/// modification time a file system keeps is a second.
const CLOCK_SLACK: Duration = Duration::from_secs(1);

/// How many of the largest packages a dry run names.
const LARGEST: usize = 20;

#[derive(clap::Args)]
pub struct Options {
    /// Reports what the sweep would remove and the space it would free, and
    /// removes nothing.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("cargo {command} failed ({status}); nothing was removed")]
    Cargo { command: String, status: ExitStatus },
    #[error("cargo's output: {0}")]
    Output(#[from] serde_json::Error),
    #[error("no unit directory holds a file like {}; nothing was removed", .0.display())]
    UnknownUnit(PathBuf),
}

/// What the sweep reads of `cargo metadata`.
#[derive(Deserialize)]
struct Directories {
    target_directory: PathBuf,
    build_directory: PathBuf,
}

/// The messages of `cargo --message-format=json` the sweep reads: each names
/// files of one unit of the build.
#[derive(Deserialize)]
#[serde(tag = "reason", rename_all = "kebab-case")]
enum Message {
    /// A compiled unit, fresh or compiled now; `filenames` are its products,
    /// in its unit directory or, for an executable or a dynamic library of
    /// the workspace, the copy Cargo puts in the profile directory.
    CompilerArtifact {
        target: Target,
        filenames: Vec<PathBuf>,
    },
    /// A build script's run, whose `out_dir` is in its unit directory.
    BuildScriptExecuted { out_dir: PathBuf },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct Target {
    name: String,
}

/// One unit of the current build as Cargo reports it.
struct Product {
    /// The crate a compiled unit compiles, whose incremental caches are named
    /// after it; `None` for a build script's run, which compiles nothing.
    crate_name: Option<String>,
    files: Vec<PathBuf>,
}

/// What the sweep keeps and removes in one profile directory.
struct Plan {
    /// The unit directories the current build uses.
    kept: BTreeSet<PathBuf>,
    /// The unit directories, whole package directories and incremental
    /// caches no current build uses.
    stale: Vec<PathBuf>,
    /// The products the current build reports in the profile directory
    /// whose unit could not be told apart from an older one of the same
    /// name; every unit directory that holds a file of that name is kept.
    ambiguous: Vec<PathBuf>,
}

pub fn run(options: Options) -> Result<(), Error> {
    let repository = crate::repository();
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let metadata = metadata(&cargo, &repository)?;
    let profile = metadata.build_directory.join(PROFILE);

    let mut products = Vec::new();
    for arguments in BUILDS {
        products.extend(products_of(&cargo, &repository, arguments)?);
    }

    // Cargo takes this lock for every build in the build directory; holding
    // it, no build starts a unit the plan does not know of, and none is
    // writing one the sweep removes.
    let _lock = lock(&profile)?;
    let plan = plan(&profile, &products)?;
    for file in &plan.ambiguous {
        println!(
            "Kept every unit with a product named like {}: none matches it exactly.",
            file.display()
        );
    }
    let sizes = sizes(&plan.stale)?;
    let freed: u64 = sizes.values().sum();
    let (units, caches) = plan
        .stale
        .iter()
        .partition::<Vec<_>, _>(|path| path.starts_with(profile.join("build")));
    let (removed, kept) = if options.dry_run {
        print_largest(&profile, &sizes);
        ("Would remove", "keep")
    } else {
        for path in &plan.stale {
            std::fs::remove_dir_all(path)?;
        }
        ("Removed", "kept")
    };
    println!(
        "{removed} {} unit directories and {} incremental caches from {}, {}, and {kept} {} units.",
        units.len(),
        caches.len(),
        relative(&repository, &profile),
        gigabytes(freed),
        plan.kept.len(),
    );
    print_others(&repository, &metadata)
}

/// The workspace's build and target directories, from `cargo metadata`.
fn metadata(cargo: &std::ffi::OsStr, repository: &Path) -> Result<Directories, Error> {
    let output = std::process::Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(repository)
        .stderr(Stdio::inherit())
        .output()?;
    if !output.status.success() {
        return Err(Error::Cargo {
            command: "metadata".to_owned(),
            status: output.status,
        });
    }
    Ok(serde_json::from_slice(&output.stdout)?)
}

/// Runs one build and returns the units it reports. A build that is up to
/// date compiles nothing and still reports every unit; Cargo's progress and
/// its diagnostics' summary go to the terminal.
fn products_of(
    cargo: &std::ffi::OsStr,
    repository: &Path,
    arguments: &[&str],
) -> Result<Vec<Product>, Error> {
    let command = arguments.join(" ");
    eprintln!("cargo {command}");
    let mut child = std::process::Command::new(cargo)
        .args(arguments)
        .arg("--message-format=json")
        .current_dir(repository)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let products = read_products(child.stdout.take().expect("stdout is piped"));
    // Waited for on every path, so the build never outlives the sweep; a
    // build that failed leaves the plan incomplete, so its error comes first.
    let status = child.wait()?;
    if !status.success() {
        return Err(Error::Cargo { command, status });
    }
    products
}

/// The units in the messages of `cargo --message-format=json`.
fn read_products(messages: impl std::io::Read) -> Result<Vec<Product>, Error> {
    let mut products = Vec::new();
    for line in BufReader::new(messages).lines() {
        match serde_json::from_str(&line?)? {
            Message::CompilerArtifact { target, filenames } => products.push(Product {
                crate_name: Some(target.name.replace('-', "_")),
                files: filenames,
            }),
            Message::BuildScriptExecuted { out_dir } => products.push(Product {
                crate_name: None,
                files: vec![out_dir],
            }),
            Message::Other => {}
        }
    }
    Ok(products)
}

/// The build directory's lock, held exclusively; waits for a running build
/// as Cargo does.
fn lock(profile: &Path) -> std::io::Result<File> {
    let file = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(profile.join(".cargo-build-lock"))?;
    match file.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => {
            eprintln!("Blocking waiting for file lock on build directory");
            file.lock()?;
        }
        Err(std::fs::TryLockError::Error(error)) => return Err(error),
    }
    Ok(file)
}

/// Which directories of `profile` the current build, reported as
/// `products`, uses, and which no current build uses.
fn plan(profile: &Path, products: &[Product]) -> Result<Plan, Error> {
    let build = profile.join("build");
    let units = unit_directories(&build)?;

    // Each kept unit directory with the crate it compiles, if any.
    let mut kept: BTreeMap<PathBuf, Option<String>> = BTreeMap::new();
    let mut ambiguous = Vec::new();
    let mut keep = |unit: PathBuf, crate_name: &Option<String>| {
        let entry = kept.entry(unit).or_default();
        if entry.is_none() {
            entry.clone_from(crate_name);
        }
    };
    for product in products {
        let in_units: Vec<PathBuf> = product
            .files
            .iter()
            .filter_map(|file| unit_of(&build, file))
            .collect();
        if !in_units.is_empty() {
            // A library of the workspace reports its copy in the profile
            // directory beside its metadata in the unit directory.
            for unit in in_units {
                keep(unit, &product.crate_name);
            }
            continue;
        }
        for file in &product.files {
            let (exact, by_name) = copied_from(&units, file)?;
            if by_name.is_empty() {
                return Err(Error::UnknownUnit(file.clone()));
            }
            if exact.is_empty() {
                ambiguous.push(file.clone());
            }
            let units = if exact.is_empty() { by_name } else { exact };
            for unit in units {
                keep(unit, &product.crate_name);
            }
        }
    }
    ambiguous.sort();
    ambiguous.dedup();

    let mut stale = Vec::new();
    for (package, hashes) in &units {
        let stale_hashes: Vec<&PathBuf> =
            hashes.iter().filter(|unit| !kept.contains_key(*unit)).collect();
        if stale_hashes.len() == hashes.len() {
            stale.push(package.clone());
        } else {
            stale.extend(stale_hashes.into_iter().cloned());
        }
    }
    stale.extend(stale_caches(&profile.join("incremental"), &kept)?);

    Ok(Plan {
        kept: kept.into_keys().collect(),
        stale,
        ambiguous,
    })
}

/// The unit directories under `build`, by package directory.
fn unit_directories(build: &Path) -> std::io::Result<BTreeMap<PathBuf, Vec<PathBuf>>> {
    let mut units = BTreeMap::new();
    for package in directories(build)? {
        let hashes = directories(&package)?;
        units.insert(package, hashes);
    }
    Ok(units)
}

/// The unit directory `build/<package>/<hash>` a product inside it is in.
fn unit_of(build: &Path, file: &Path) -> Option<PathBuf> {
    let mut components = file.strip_prefix(build).ok()?.components();
    let package = components.next()?;
    let hash = components.next()?;
    Some(build.join(package).join(hash))
}

/// The unit directories whose `out/` holds the file Cargo copied to `file`
/// in the profile directory: on macOS a clone, on Linux a hard link, either
/// way with the same length and modification time. The copy is named after
/// the target (`demi-backend`), the original after its crate
/// (`demi_backend`). The second list is every unit directory with a file of
/// that name, for when none matches exactly.
fn copied_from(
    units: &BTreeMap<PathBuf, Vec<PathBuf>>,
    file: &Path,
) -> std::io::Result<(Vec<PathBuf>, Vec<PathBuf>)> {
    let copy = std::fs::metadata(file)?;
    let name = file
        .file_name()
        .expect("a product is a file")
        .to_string_lossy();
    let mut names = vec![name.to_string(), name.replace('-', "_")];
    names.dedup();
    let mut exact = Vec::new();
    let mut by_name = Vec::new();
    for unit in units.values().flatten() {
        for name in &names {
            let Some(original) = metadata_if_present(&unit.join("out").join(name))? else {
                continue;
            };
            by_name.push(unit.clone());
            if original.len() == copy.len() && original.modified()? == copy.modified()? {
                exact.push(unit.clone());
            }
        }
    }
    Ok((exact, by_name))
}

/// The incremental caches under `incremental` that no kept unit wrote.
///
/// rustc names a crate's cache `<crate>-<id>`, where nothing Cargo reports
/// gives the id, and keeps in it the newest session, `s-<start>-…`, whose
/// start is the microseconds since the epoch in base 36. Cargo touches a
/// unit's `fingerprint/invoked.timestamp` just before it starts rustc and
/// writes the fingerprint when rustc succeeded, so the cache a kept unit
/// wrote is the one of its crate whose newest session started in between.
/// Units of the same crate compiled at the same time match each other's
/// caches too, which keeps more, never less.
fn stale_caches(
    incremental: &Path,
    kept: &BTreeMap<PathBuf, Option<String>>,
) -> std::io::Result<Vec<PathBuf>> {
    // When each kept unit's crate was last compiled; a unit without a
    // fingerprint keeps every cache of its crate.
    let mut compiled: BTreeMap<&str, Vec<(SystemTime, SystemTime)>> = BTreeMap::new();
    let mut every_cache: HashSet<&str> = HashSet::new();
    for (unit, crate_name) in kept {
        let Some(crate_name) = crate_name else {
            continue;
        };
        match compiled_between(&unit.join("fingerprint"))? {
            Some(window) => compiled.entry(crate_name).or_default().push(window),
            None => {
                every_cache.insert(crate_name);
            }
        }
    }

    let mut stale = Vec::new();
    for cache in directories(incremental)? {
        let name = cache
            .file_name()
            .expect("a directory entry has a name")
            .to_string_lossy();
        let crate_name = cache_crate(&name);
        if every_cache.contains(crate_name) {
            continue;
        }
        let used = newest_session(&cache)?.is_some_and(|started| {
            compiled.get(crate_name).is_some_and(|windows| {
                windows.iter().any(|(from, to)| {
                    started + CLOCK_SLACK >= *from && started <= *to + CLOCK_SLACK
                })
            })
        });
        if !used {
            stale.push(cache);
        }
    }
    Ok(stale)
}

/// The crate an incremental cache `<crate>-<id>` belongs to.
fn cache_crate(name: &str) -> &str {
    name.rsplit_once('-').map_or(name, |(crate_name, _)| crate_name)
}

/// When Cargo last started rustc for a unit and when it last wrote the
/// unit's fingerprint; `None` without a fingerprint.
fn compiled_between(fingerprint: &Path) -> std::io::Result<Option<(SystemTime, SystemTime)>> {
    let Some(invoked) = metadata_if_present(&fingerprint.join("invoked.timestamp"))? else {
        return Ok(None);
    };
    let mut written = invoked.modified()?;
    for entry in std::fs::read_dir(fingerprint)? {
        written = written.max(entry?.metadata()?.modified()?);
    }
    Ok(Some((invoked.modified()?, written)))
}

/// When the newest session of an incremental cache started.
fn newest_session(cache: &Path) -> std::io::Result<Option<SystemTime>> {
    let mut newest = None;
    for entry in std::fs::read_dir(cache)? {
        let name = entry?.file_name();
        let started = name
            .to_str()
            .and_then(|name| name.strip_prefix("s-"))
            .and_then(|rest| rest.split(['-', '.']).next())
            .and_then(|start| u64::from_str_radix(start, 36).ok())
            .map(|micros| SystemTime::UNIX_EPOCH + Duration::from_micros(micros));
        newest = newest.max(started);
    }
    Ok(newest)
}

/// The subdirectories of `directory`; none if it does not exist.
fn directories(directory: &Path) -> std::io::Result<Vec<PathBuf>> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut directories = Vec::new();
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            directories.push(entry.path());
        }
    }
    directories.sort();
    Ok(directories)
}

fn metadata_if_present(path: &Path) -> std::io::Result<Option<Metadata>> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// The disk space each tree in `paths` takes, as `du` counts it: allocated
/// blocks, and a file with several links once. `fs_extra::dir::get_size`
/// adds up lengths instead, which counts a sparse file in full and a hard
/// link twice.
fn sizes(paths: &[PathBuf]) -> std::io::Result<BTreeMap<PathBuf, u64>> {
    let mut seen = HashSet::new();
    let mut sizes = BTreeMap::new();
    for path in paths {
        let mut size = 0;
        let mut pending = vec![path.clone()];
        while let Some(path) = pending.pop() {
            let metadata = std::fs::symlink_metadata(&path)?;
            if metadata.nlink() > 1 && !seen.insert((metadata.dev(), metadata.ino())) {
                continue;
            }
            size += metadata.blocks() * 512;
            if metadata.is_dir() {
                for entry in std::fs::read_dir(&path)? {
                    pending.push(entry?.path());
                }
            }
        }
        sizes.insert(path.clone(), size);
    }
    Ok(sizes)
}

/// The packages and crates whose stale units take the most space.
fn print_largest(profile: &Path, sizes: &BTreeMap<PathBuf, u64>) {
    let build = profile.join("build");
    let incremental = profile.join("incremental");
    let mut groups: BTreeMap<String, u64> = BTreeMap::new();
    for (path, size) in sizes {
        let group = if let Ok(unit) = path.strip_prefix(&build) {
            let package = unit.components().next().expect("a unit is in a package");
            format!("build/{}", package.as_os_str().to_string_lossy())
        } else {
            let cache = path
                .strip_prefix(&incremental)
                .unwrap_or(path)
                .to_string_lossy();
            format!("incremental/{}", cache_crate(&cache))
        };
        *groups.entry(group).or_default() += size;
    }
    let mut groups: Vec<_> = groups.into_iter().collect();
    groups.sort_by(|left, right| right.1.cmp(&left.1));
    for (group, size) in groups.iter().take(LARGEST) {
        println!("{:>10}  {group}", gigabytes(*size));
    }
    if groups.len() > LARGEST {
        println!("            and {} more", groups.len() - LARGEST);
    }
}

/// Reports the Cargo target directories the sweep leaves alone: everything
/// in the target and build directories beside the swept profile, and every
/// directory of `.cache` that Cargo marked as its own with a `CACHEDIR.TAG`.
fn print_others(repository: &Path, metadata: &Directories) -> Result<(), Error> {
    let mut others = BTreeSet::new();
    for root in [&metadata.target_directory, &metadata.build_directory] {
        others.extend(
            directories(root)?
                .into_iter()
                .filter(|directory| !directory.ends_with(PROFILE)),
        );
    }
    for directory in directories(&repository.join(".cache"))? {
        if directory.join("CACHEDIR.TAG").is_file() {
            others.insert(directory);
        }
    }
    if others.is_empty() {
        return Ok(());
    }
    let others: Vec<PathBuf> = others.into_iter().collect();
    println!("Left alone:");
    for (directory, size) in sizes(&others)? {
        println!("{:>10}  {}", gigabytes(size), relative(repository, &directory));
    }
    Ok(())
}

fn relative(repository: &Path, path: &Path) -> String {
    path.strip_prefix(repository)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn gigabytes(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file with `contents`, modified at `seconds` after the epoch.
    fn file(path: &Path, contents: &str, seconds: u64) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds))
            .unwrap();
    }

    /// A unit directory whose rustc ran from `invoked` to `written`.
    fn unit(build: &Path, package: &str, hash: &str, invoked: u64, written: u64) -> PathBuf {
        let unit = build.join(package).join(hash);
        file(&unit.join("fingerprint/invoked.timestamp"), "", invoked);
        file(&unit.join("fingerprint/lib-x"), "0123", written);
        unit
    }

    /// An incremental cache whose newest session started at `seconds`, named
    /// as rustc names it.
    fn cache(incremental: &Path, name: &str, seconds: u64) -> PathBuf {
        let mut micros = seconds * 1_000_000;
        let mut start = Vec::new();
        while micros > 0 {
            start.push(char::from_digit((micros % 36) as u32, 36).unwrap());
            micros /= 36;
        }
        let start: String = start.into_iter().rev().collect();
        let cache = incremental.join(name);
        std::fs::create_dir_all(cache.join(format!("s-{start}-1abc-0svh"))).unwrap();
        file(&cache.join(format!("s-{start}-1abc.lock")), "", seconds);
        cache
    }

    // Cost: a few dozen files in a temporary directory, milliseconds.
    #[test]
    fn the_sweep_keeps_what_the_current_build_reports_and_removes_the_rest() {
        let directory = tempfile::tempdir().unwrap();
        let profile = directory.path().join("debug");
        let build = profile.join("build");
        let incremental = profile.join("incremental");

        // The library's current unit, compiled from 1000 s to 1010 s, with
        // the copy of its library in the profile directory, which only the
        // metadata beside it in the unit directory identifies; and the unit
        // an older Cargo.lock gave it.
        let library = unit(&build, "library", "aaaa", 1000, 1010);
        file(&library.join("out/liblibrary-aaaa.rmeta"), "rmeta", 1010);
        file(&library.join("out/liblibrary-aaaa.rlib"), "rlib", 1010);
        file(&profile.join("liblibrary.rlib"), "rlib", 1010);
        let old_library = unit(&build, "library", "bbbb", 500, 510);
        // A package the build no longer has.
        unit(&build, "gone", "cccc", 400, 410);
        // The executable's current unit and an older one of the same length,
        // with the copy Cargo put in the profile directory.
        let tool = unit(&build, "demi-tool", "dddd", 2000, 2010);
        file(&tool.join("out/demi_tool"), "new", 2009);
        let old_tool = unit(&build, "demi-tool", "eeee", 1500, 1510);
        file(&old_tool.join("out/demi_tool"), "old", 1509);
        file(&profile.join("demi-tool"), "new", 2009);
        // A build script's run, which compiles nothing.
        let script = build.join("library/ffff");
        std::fs::create_dir_all(script.join("out")).unwrap();

        // The caches the current units wrote, and older ones.
        cache(&incremental, "library-1aaa", 1005);
        let old_library_cache = cache(&incremental, "library-2bbb", 505);
        cache(&incremental, "demi_tool-3ddd", 2001);
        let old_tool_cache = cache(&incremental, "demi_tool-4eee", 1501);
        let gone_cache = cache(&incremental, "gone-5ccc", 405);

        let products = [
            Product {
                crate_name: Some("library".to_owned()),
                files: vec![
                    profile.join("liblibrary.rlib"),
                    library.join("out/liblibrary-aaaa.rmeta"),
                ],
            },
            Product {
                crate_name: Some("demi_tool".to_owned()),
                files: vec![profile.join("demi-tool")],
            },
            Product {
                crate_name: None,
                files: vec![script.join("out")],
            },
        ];
        let plan = plan(&profile, &products).unwrap();

        assert_eq!(
            plan.kept,
            BTreeSet::from([library, tool, script]),
            "the current units are kept"
        );
        assert_eq!(
            plan.stale.into_iter().collect::<BTreeSet<_>>(),
            BTreeSet::from([
                old_tool,
                build.join("gone"),
                old_library,
                old_library_cache,
                old_tool_cache,
                gone_cache,
            ]),
            "every other unit and cache is removed"
        );
        assert!(plan.ambiguous.is_empty(), "every product's unit is known");
    }
}
