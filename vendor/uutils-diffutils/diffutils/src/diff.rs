// This file is part of the uutils diffutils package.
//
// For the full copyright and license information, please view the LICENSE-*
// files that was distributed with this source code.

use crate::params::{parse_params, Format, Params};
use crate::utils::report_file_error;
use crate::{context_diff, ed_diff, normal_diff, side_diff, unified_diff};
type ArgsOs = std::vec::IntoIter<std::ffi::OsString>;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::Metadata;
use std::iter::Peekable;
use std::path::{Path, PathBuf};
use uucore::context::fs;
use uucore::context::io::{self, ErrorKind, Read, Write};
use uucore::context::process::exit;

// Exit codes are documented at
// https://www.gnu.org/software/diffutils/manual/html_node/Invoking-diff.html.
//     An exit status of 0 means no differences were found,
//     1 means some differences were found,
//     and 2 means trouble.
pub fn main(opts: Peekable<ArgsOs>) -> i32 {
    let params = parse_params(opts).unwrap_or_else(|error| {
        uucore::context_eprintln!("{error}");
        exit(2);
    });
    let mut from = PathBuf::from(&params.from);
    let mut to = PathBuf::from(&params.to);
    // diff DIRECTORY FILE => diff DIRECTORY/FILE FILE
    // diff FILE DIRECTORY => diff FILE DIRECTORY/FILE
    // Whether each operand is a directory, or None when it names nothing.
    let is_dir = |path: &Path| match path == Path::new("-") {
        true => Some(false),
        false => fs::metadata(path).ok().map(|metadata| metadata.is_dir()),
    };
    let (dir, file) = match (is_dir(&from), is_dir(&to)) {
        (Some(true), Some(false)) => (&mut from, &to),
        (Some(false), Some(true)) => (&mut to, &from),
        _ => return compare(&params, &from, &to, 0),
    };
    if file == Path::new("-") {
        uucore::context_eprintln!(
            "{}: cannot compare '-' to a directory",
            params.executable.to_string_lossy()
        );
        return 2;
    }
    dir.push(file.file_name().unwrap_or(file.as_os_str()));
    compare(&params, &from, &to, 0)
}

/// What one side of a comparison is: standard input, a file or directory
/// that exists, or, with `-N`, one that does not.
enum Side {
    Stdin,
    Existing(Metadata),
    Absent,
}

impl Side {
    fn of(path: &Path) -> io::Result<Self> {
        if path == Path::new("-") {
            return Ok(Self::Stdin);
        }
        fs::metadata(path).map(Self::Existing)
    }

    fn is_dir(&self) -> bool {
        matches!(self, Self::Existing(metadata) if metadata.is_dir())
    }
}

/// Compares two operands, or two entries of the directories being compared
/// at `depth` > 0, as GNU's `compare_files` does, and returns the exit
/// status.
fn compare(params: &Params, from: &Path, to: &Path, depth: usize) -> i32 {
    // With `-N`, a file that does not exist is absent when the other one
    // exists; when neither does, both are reported.
    let absent = |side: &io::Result<Side>| {
        params.new_file && matches!(side, Err(error) if error.kind() == ErrorKind::NotFound)
    };
    let (from_side, to_side) = match (Side::of(from), Side::of(to)) {
        (Ok(from_side), Ok(to_side)) => (from_side, to_side),
        (Ok(from_side), to_side) if absent(&to_side) => (from_side, Side::Absent),
        (from_side, Ok(to_side)) if absent(&from_side) => (Side::Absent, to_side),
        (from_side, to_side) => {
            for (path, side) in [(from, from_side), (to, to_side)] {
                if let Err(error) = side {
                    report_file_error(&params.executable, &path.into(), &error);
                }
            }
            return 2;
        }
    };
    let from_dir = from_side.is_dir() || matches!(from_side, Side::Absent) && to_side.is_dir();
    let to_dir = to_side.is_dir() || matches!(to_side, Side::Absent) && from_side.is_dir();
    match (from_dir, to_dir) {
        (true, true) => {
            if is_same_file(from, to) {
                0
            } else if depth > 0 && !params.recursive {
                uucore::context_println!(
                    "Common subdirectories: {} and {}",
                    from.display(),
                    to.display()
                );
                0
            } else {
                compare_directories(params, from, to, depth)
            }
        }
        (false, false) => compare_files(params, from, to, depth, &from_side, &to_side),
        _ => {
            uucore::context_println!(
                "File {} is a {} while file {} is a {}",
                from.display(),
                kind(&from_side),
                to.display(),
                kind(&to_side)
            );
            1
        }
    }
}

/// The kind of file GNU names in "File a is a directory while file b is a
/// regular file".
fn kind(side: &Side) -> &'static str {
    let Side::Existing(metadata) = side else {
        // Standard input, the only other side that gets here, is a pipe.
        return "fifo";
    };
    let file_type = metadata.file_type();
    if file_type.is_dir() {
        return "directory";
    }
    if file_type.is_file() {
        return if metadata.len() == 0 { "regular empty file" } else { "regular file" };
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        if file_type.is_block_device() {
            return "block special file";
        }
        if file_type.is_char_device() {
            return "character special file";
        }
        if file_type.is_fifo() {
            return "fifo";
        }
        if file_type.is_socket() {
            return "socket";
        }
    }
    "weird file"
}

/// Whether both paths, resolved against the job's working directory, are
/// the same file.
fn is_same_file(from: &Path, to: &Path) -> bool {
    same_file::is_same_file(uucore::context::resolve(from), uucore::context::resolve(to))
        .unwrap_or(false)
}

/// Compares the entries of two directories in the byte order of their
/// names, as GNU's `diff_dirs` does in the C locale: an entry on one side
/// only is reported as `Only in DIR: NAME`, unless `-N` compares it with an
/// absent one, the entries on both sides are compared one level deeper, and
/// a name `-x` or `-X` matches is left out.
fn compare_directories(params: &Params, from: &Path, to: &Path, depth: usize) -> i32 {
    // Each name with whether `from` and `to` have it.
    let mut entries: BTreeMap<OsString, (bool, bool)> = BTreeMap::new();
    let mut status = 0;
    for (dir, in_from) in [(from, true), (to, false)] {
        let names = match fs::read_dir(dir) {
            Ok(names) => names,
            // An absent side, which only `-N` makes, has no entries.
            Err(error) if params.new_file && error.kind() == ErrorKind::NotFound => continue,
            Err(error) => {
                report_file_error(&params.executable, &dir.into(), &error);
                status = 2;
                continue;
            }
        };
        for name in names {
            match name {
                Ok(entry) => {
                    let name = entry.file_name();
                    let name_text = name.to_string_lossy();
                    if params.excludes.iter().any(|pattern| pattern.matches(&name_text)) {
                        continue;
                    }
                    let sides = entries.entry(name).or_default();
                    if in_from {
                        sides.0 = true;
                    } else {
                        sides.1 = true;
                    }
                }
                Err(error) => {
                    report_file_error(&params.executable, &dir.into(), &error);
                    status = 2;
                }
            }
        }
    }
    for (name, (in_from, in_to)) in entries {
        let entry_status = if in_from && in_to || params.new_file {
            compare(params, &from.join(&name), &to.join(&name), depth + 1)
        } else {
            let dir = if in_from { from } else { to };
            uucore::context_println!("Only in {}: {}", dir.display(), Path::new(&name).display());
            1
        };
        status = status.max(entry_status);
    }
    status
}

/// Compares two files that are not directories. Within a directory
/// comparison, a pair that differs is introduced by a line repeating the
/// command, as GNU's `diff -r a/x b/x`.
fn compare_files(
    params: &Params,
    from: &Path,
    to: &Path,
    depth: usize,
    from_side: &Side,
    to_side: &Side,
) -> i32 {
    let params = Params {
        from: from.into(),
        to: to.into(),
        ..params.clone()
    };
    // if from and to are the same file, no need to perform any comparison
    let maybe_report_identical_files = || {
        if params.report_identical_files {
            uucore::context_println!(
                "Files {} and {} are identical",
                params.from.to_string_lossy(),
                params.to.to_string_lossy(),
            );
        }
    };
    let both_exist = !matches!(from_side, Side::Absent) && !matches!(to_side, Side::Absent);
    if matches!((from_side, to_side), (Side::Stdin, Side::Stdin)) || both_exist && is_same_file(from, to) {
        maybe_report_identical_files();
        return 0;
    }

    // read files
    fn read_file_contents(side: &Side, filepath: &OsString) -> io::Result<Vec<u8>> {
        match side {
            Side::Stdin => {
                let mut content = Vec::new();
                io::stdin().read_to_end(&mut content).and(Ok(content))
            }
            Side::Existing(_) => fs::read(filepath),
            Side::Absent => Ok(Vec::new()),
        }
    }
    let mut io_error = false;
    let mut from_content = match read_file_contents(from_side, &params.from) {
        Ok(from_content) => from_content,
        Err(e) => {
            report_file_error(&params.executable, &params.from, &e);
            io_error = true;
            vec![]
        }
    };
    let mut to_content = match read_file_contents(to_side, &params.to) {
        Ok(to_content) => to_content,
        Err(e) => {
            report_file_error(&params.executable, &params.to, &e);
            io_error = true;
            vec![]
        }
    };
    if io_error {
        return 2;
    }
    if from_content == to_content {
        maybe_report_identical_files();
        return 0;
    }
    let buffer = [from_side, to_side].into_iter().filter_map(block_size).max().unwrap_or(4096);
    let is_binary = |content: &[u8]| content[..content.len().min(buffer)].contains(&0);
    if !params.text && (is_binary(&from_content) || is_binary(&to_content)) {
        uucore::context_println!(
            "{} {} and {} differ",
            if params.brief { "Files" } else { "Binary files" },
            params.from.to_string_lossy(),
            params.to.to_string_lossy()
        );
        return 1;
    }
    let ignores_space = params.ignore_space_change || params.ignore_all_space;
    if ignores_space || params.ignore_case || params.ignore_blank_lines {
        // -b and -w ignore a missing newline at the end, as GNU's do.
        for content in [&mut from_content, &mut to_content] {
            if ignores_space && content.last().is_some_and(|&byte| byte != b'\n') {
                content.push(b'\n');
            }
        }
        // The files differ only in what the options ignore.
        let brief = Params { brief: true, ..params.clone() };
        if normal_diff::diff(&from_content, &to_content, &brief).is_empty() {
            maybe_report_identical_files();
            return 0;
        }
    }
    if params.brief {
        uucore::context_println!(
            "Files {} and {} differ",
            params.from.to_string_lossy(),
            params.to.to_string_lossy()
        );
        return 1;
    }
    if depth > 0 {
        let mut header = OsString::from("diff");
        for option in &params.options {
            header.push(" ");
            header.push(option);
        }
        for path in [&params.from, &params.to] {
            header.push(" ");
            header.push(path);
        }
        uucore::context_println!("{}", header.to_string_lossy());
    }

    // run diff
    let result: Vec<u8> = match params.format {
        Format::Normal => normal_diff::diff(&from_content, &to_content, &params),
        Format::Unified => unified_diff::diff(&from_content, &to_content, &params),
        Format::Context => context_diff::diff(&from_content, &to_content, &params),
        Format::Ed => ed_diff::diff(&from_content, &to_content, &params).unwrap_or_else(|error| {
            uucore::context_eprintln!("{error}");
            exit(2);
        }),
        Format::SideBySide => {
            let mut output = Vec::new();
            side_diff::diff(&from_content, &to_content, &mut output, &params);
            output
        }
    };
    // A closed pipe has ended the run already, in the write.
    if let Err(error) = io::stdout().write_all(&result) {
        report_file_error(&params.executable, &"standard output".into(), &error);
        return 2;
    }
    1
}

/// The block size GNU diff sizes its first read by, for its binary test.
fn block_size(side: &Side) -> Option<usize> {
    #[cfg(unix)]
    if let Side::Existing(metadata) = side {
        use std::os::unix::fs::MetadataExt;
        return Some(metadata.blksize() as usize);
    }
    let _ = side;
    None
}
