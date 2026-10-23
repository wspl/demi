//! `which [-as] NAME…`: where a command runs from, as Debian's prints it, and
//! on Windows `where [/Q] PATTERN…`, which prints every match as Windows'
//! own does.
//!
//! A name is searched for in the directories of the job's `PATH`, as an
//! executable file; on Windows with each extension of `PATHEXT` as well. That
//! holds for a name the runner runs itself (`crates/runner-shell/src/utilities.rs`)
//! too: it prints the path of the Host's program of that name, as the system's
//! `which` would, and the bare name where the Host has none. Naming the path
//! runs the runner's own (`utilities::named`), so `"$(which sed)" -i …` runs
//! the runner's `sed`, and `[ -x "$(which sed)" ]` holds where the Host has one.
//!
//! The status is 0 when every name was found, 1 when one was not, and 2 for
//! an option it lacks.

use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

use uucore::{context_eprintln, context_println};

/// Runs `which` for the arguments `args`, its own name first.
pub fn uumain(args: Vec<OsString>) -> i32 {
    let mut all = false;
    let mut silent = false;
    let mut names = Vec::new();
    let mut options = true;
    for arg in args.iter().skip(1) {
        let text = arg.to_string_lossy();
        if options && text == "--" {
            options = false;
        } else if options && text.len() > 1 && text.starts_with('-') {
            for flag in text.chars().skip(1) {
                match flag {
                    'a' => all = true,
                    's' => silent = true,
                    _ => {
                        context_eprintln!("Illegal option -{flag}");
                        context_eprintln!("Usage: which [-as] program ...");
                        return 2;
                    }
                }
            }
        } else {
            names.push(arg.clone());
        }
    }
    if names.is_empty() {
        return 1;
    }
    let mut status = 0;
    for name in &names {
        let found = locate(name, all);
        if found.is_empty() {
            status = 1;
        } else if !silent {
            for line in found {
                context_println!("{}", line.to_string_lossy());
            }
        }
    }
    status
}

/// What a command `name` runs from: the executable files of the `PATH`, all
/// of them or the first, or the file a path names; for a name the runner runs
/// itself, its own name where there is no file.
fn locate(name: &OsStr, all: bool) -> Vec<OsString> {
    let mut found = Vec::new();
    let text = name.to_string_lossy();
    if text.contains('/') || (cfg!(windows) && text.contains('\\')) {
        if let Some(path) = executable(Path::new(name)) {
            found.push(path.into_os_string());
        }
        return found;
    }
    let path = uucore::context::env::var_os("PATH").unwrap_or_default();
    let extensions = extensions();
    for directory in std::env::split_paths(&path) {
        // An empty entry is the current directory.
        let directory = if directory.as_os_str().is_empty() {
            PathBuf::from(".")
        } else {
            directory
        };
        for extension in &extensions {
            let mut file = name.to_owned();
            file.push(extension);
            if let Some(path) = executable(&directory.join(file)) {
                found.push(shown(path).into_os_string());
                if !all {
                    return found;
                }
                break;
            }
        }
    }
    if found.is_empty() && crate::utilities::named(name).is_some() {
        found.push(name.to_owned());
    }
    found
}

/// `path` as the file system spells it: on Windows a name is found whatever
/// its case, and `where` prints the file's own (`ls.exe`, not `ls.EXE`).
#[cfg(windows)]
fn shown(path: PathBuf) -> PathBuf {
    let (Some(directory), Some(name)) = (path.parent(), path.file_name()) else {
        return path;
    };
    let wanted = name.to_string_lossy().to_ascii_lowercase();
    uucore::context::fs::read_dir(directory)
        .ok()
        .and_then(|entries| {
            entries
                .flatten()
                .find(|entry| entry.file_name().to_string_lossy().to_ascii_lowercase() == wanted)
        })
        .map_or(path.clone(), |entry| directory.join(entry.file_name()))
}

#[cfg(not(windows))]
fn shown(path: PathBuf) -> PathBuf {
    path
}

/// The extensions a program's file may carry: none on Unix, and on Windows
/// none and each of `PATHEXT`.
fn extensions() -> Vec<OsString> {
    if !cfg!(windows) {
        return vec![OsString::new()];
    }
    let pathext = uucore::context::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned());
    std::iter::once(String::new())
        .chain(
            pathext
                .split(';')
                .filter(|extension| !extension.is_empty())
                .map(str::to_owned),
        )
        .map(OsString::from)
        .collect()
}

/// `path` when it is an executable file, resolved against the job's
/// directory.
fn executable(path: &Path) -> Option<PathBuf> {
    let metadata = uucore::context::fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o111 == 0 {
            return None;
        }
    }
    Some(path.to_owned())
}

/// Runs Windows' `where [/Q] PATTERN…` for the arguments `args`, its own name
/// first: every file of the job's directory and then of each directory of
/// `PATH` that a pattern names, with each extension of `PATHEXT` as well, the
/// pattern's `*` and `?` standing for any run of characters and any one.
/// A name the runner runs itself prints bare where nothing matched. The status is 0 when every
/// pattern matched, 1 when one did not, and 2 for an option it lacks.
#[cfg(windows)]
pub fn where_main(args: Vec<OsString>) -> i32 {
    let mut quiet = false;
    let mut patterns = Vec::new();
    for arg in args.iter().skip(1) {
        let text = arg.to_string_lossy();
        match text.to_ascii_lowercase().as_str() {
            "/q" => quiet = true,
            "/?" => {
                context_println!("WHERE [/Q] pattern...");
                context_println!("Prints the files of the current directory and of the PATH that match each pattern.");
                return 0;
            }
            option if option.starts_with('/') => {
                context_eprintln!("ERROR: Invalid argument or option - '{text}'.");
                context_eprintln!("Type \"WHERE /?\" for usage.");
                return 2;
            }
            _ => patterns.push(arg.clone()),
        }
    }
    if patterns.is_empty() {
        context_eprintln!("ERROR: Missing pattern.");
        return 2;
    }
    let mut status = 0;
    for pattern in &patterns {
        let found = matches(pattern);
        if found.is_empty() {
            context_eprintln!("INFO: Could not find files for the given pattern(s).");
            status = 1;
        } else if !quiet {
            for line in found {
                context_println!("{}", line.to_string_lossy());
            }
        }
    }
    status
}

/// The files `pattern` names, for [`where_main`].
#[cfg(windows)]
fn matches(pattern: &OsStr) -> Vec<OsString> {
    let mut found = Vec::new();
    let text = pattern.to_string_lossy().to_ascii_lowercase();
    let path = uucore::context::env::var_os("PATH").unwrap_or_default();
    let extensions = extensions();
    let directories = std::iter::once(PathBuf::from(".")).chain(std::env::split_paths(&path));
    for directory in directories {
        let directory = if directory.as_os_str().is_empty() {
            PathBuf::from(".")
        } else {
            directory
        };
        let Ok(entries) = uucore::context::fs::read_dir(&directory) else {
            continue;
        };
        let mut names: Vec<OsString> = entries
            .flatten()
            .map(|entry| entry.file_name())
            .collect();
        names.sort();
        for name in names {
            let lower = name.to_string_lossy().to_ascii_lowercase();
            let hit = extensions.iter().any(|extension| {
                let wanted = format!("{text}{}", extension.to_string_lossy().to_ascii_lowercase());
                wildcard(&wanted, &lower)
            });
            if hit && executable(&directory.join(&name)).is_some() {
                let shown = if directory == Path::new(".") {
                    uucore::context::resolve(&name)
                } else {
                    directory.join(&name)
                };
                found.push(shown.into_os_string());
            }
        }
    }
    // A name the runner runs itself is its own where the Host has no file.
    if found.is_empty() && crate::utilities::named(pattern).is_some() {
        found.push(pattern.to_owned());
    }
    found
}

/// Whether `name` matches `pattern`, whose `*` stands for any run of
/// characters and `?` for any one, both lowercase.
#[cfg(windows)]
fn wildcard(pattern: &str, name: &str) -> bool {
    let (pattern, name): (Vec<char>, Vec<char>) = (pattern.chars().collect(), name.chars().collect());
    let (mut p, mut n, mut star, mut resume) = (0, 0, None, 0);
    while n < name.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == name[n]) {
            p += 1;
            n += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            resume = n;
            p += 1;
        } else if let Some(star) = star {
            p = star + 1;
            resume += 1;
            n = resume;
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|c| *c == '*')
}
