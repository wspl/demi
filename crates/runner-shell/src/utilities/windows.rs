//! `chmod` and `chown` on Windows, which has no Unix mode or owner: each
//! reads GNU's command line and checks what GNU's checks, its operands, the
//! mode or the owner and group, and each file, failing with GNU's message and
//! status; a valid change changes nothing, so a script's `chmod +x build.sh`
//! succeeds. An owner or group is a number or the name of an account Windows
//! knows.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};
use uucore::{context::fs, display::Quotable, error::io_message};

/// GNU's status for every failure of these utilities.
const FAILED: i32 = 1;

/// The options the two utilities share.
#[derive(Default)]
struct Options {
    recursive: bool,
    silent: bool,
    verbose: bool,
    /// Whether a file operand that is a symbolic link stands for its target.
    dereference: bool,
    reference: Option<OsString>,
    operands: Vec<OsString>,
}

/// What reading a command line came to.
enum Read {
    Run(Options),
    /// It answered `--help` or `--version`, or failed: the status.
    Done(i32),
}

/// A usage error as GNU words it, with the line that points to `--help`.
fn usage_error(name: &str, message: &str) -> i32 {
    uucore::context_eprintln!("{name}: {message}");
    uucore::context_eprintln!("Try '{name} --help' for more information.");
    FAILED
}

/// Reads `args` as GNU's `chmod` (`chown` when `owner`) reads them: options
/// may come together and anywhere before `--`; for `chmod` a word such as
/// `-w` or `-x` is a mode, as in GNU's.
fn read(name: &str, args: Vec<OsString>, owner: bool, usage: &str) -> Read {
    let mut options = Options {
        dereference: true,
        ..Options::default()
    };
    let mut args = args.into_iter().skip(1);
    let mut ended = false;
    while let Some(arg) = args.next() {
        let text = arg.to_string_lossy().into_owned();
        if ended || text == "-" || !text.starts_with('-') {
            options.operands.push(arg);
            continue;
        }
        if let Some(long) = text.strip_prefix("--") {
            let (option, value) = match long.split_once('=') {
                Some((option, value)) => (option, Some(OsString::from(value))),
                None => (long, None),
            };
            match option {
                "" => ended = true,
                "help" => {
                    uucore::context_print!("{usage}");
                    return Read::Done(0);
                }
                "version" => {
                    uucore::context_println!("{name} (Demi)");
                    return Read::Done(0);
                }
                "recursive" => options.recursive = true,
                "silent" | "quiet" => options.silent = true,
                "verbose" | "changes" => options.verbose |= option == "verbose",
                "preserve-root" | "no-preserve-root" => {}
                "dereference" if owner => options.dereference = true,
                "no-dereference" if owner => options.dereference = false,
                "from" if owner => {
                    let Some(from) = value.or_else(|| args.next()) else {
                        return Read::Done(usage_error(name, "option '--from' requires an argument"));
                    };
                    if let Err(status) = check_owner(name, &from.to_string_lossy()) {
                        return Read::Done(status);
                    }
                }
                "reference" => match value.or_else(|| args.next()) {
                    Some(reference) => options.reference = Some(reference),
                    None => {
                        return Read::Done(usage_error(
                            name,
                            "option '--reference' requires an argument",
                        ));
                    }
                },
                _ => return Read::Done(usage_error(name, &format!("unrecognized option '{text}'"))),
            }
            continue;
        }
        let flags = &text[1..];
        let letters = if owner { "cfvhRHLP" } else { "cfvR" };
        match flags.chars().find(|letter| !letters.contains(*letter)) {
            None => {
                for letter in flags.chars() {
                    match letter {
                        'R' => options.recursive = true,
                        'f' => options.silent = true,
                        'v' => options.verbose = true,
                        'h' => options.dereference = false,
                        _ => {}
                    }
                }
            }
            // GNU's `chmod` takes `-w`, `-rx` or `-x,u+w` as its mode.
            Some(_) if !owner && flags.chars().all(|letter| "rwxXstugoa+-=,01234567".contains(letter)) => {
                options.operands.push(arg);
            }
            Some(letter) => {
                return Read::Done(usage_error(name, &format!("invalid option -- '{letter}'")));
            }
        }
    }
    Read::Run(options)
}

/// The files to act on, after the mode or owner unless `--reference`
/// names a file to take it from, which must exist; or the status of a
/// missing operand or reference.
fn files(name: &str, options: &mut Options) -> Result<(Option<String>, Vec<OsString>), i32> {
    if let Some(reference) = &options.reference {
        if let Err(error) = fs::metadata(reference) {
            uucore::context_eprintln!(
                "{name}: failed to get attributes of {}: {}",
                reference.quote(),
                io_message(&error)
            );
            return Err(FAILED);
        }
        if options.operands.is_empty() {
            return Err(usage_error(name, "missing operand"));
        }
        return Ok((None, std::mem::take(&mut options.operands)));
    }
    let mut operands = std::mem::take(&mut options.operands).into_iter();
    let Some(change) = operands.next() else {
        return Err(usage_error(name, "missing operand"));
    };
    let change = change.to_string_lossy().into_owned();
    let files: Vec<OsString> = operands.collect();
    if files.is_empty() {
        return Err(usage_error(name, &format!("missing operand after {}", change.quote())));
    }
    Ok((Some(change), files))
}

/// Visits `file` and, with `recursive`, everything under it, as GNU's walk
/// does: a link met inside is not followed. `visit` hears each path and its
/// metadata. Gives whether everything could be read.
fn walk(
    name: &str,
    options: &Options,
    file: &OsString,
    visit: &mut dyn FnMut(&Path, &fs::Metadata),
) -> bool {
    let read = if options.dereference {
        fs::metadata(file)
    } else {
        fs::symlink_metadata(file)
    };
    let metadata = match read {
        Ok(metadata) => metadata,
        Err(error) => {
            if !options.silent {
                uucore::context_eprintln!(
                    "{name}: cannot access {}: {}",
                    file.quote(),
                    io_message(&error)
                );
            }
            return false;
        }
    };
    let path = PathBuf::from(file);
    visit(&path, &metadata);
    if !(options.recursive && metadata.is_dir()) {
        return true;
    }
    let mut pending = vec![path];
    let mut whole = true;
    while let Some(directory) = pending.pop() {
        // Each entry read checks the job's cancellation.
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) => {
                if !options.silent {
                    uucore::context_eprintln!(
                        "{name}: cannot read directory {}: {}",
                        directory.quote(),
                        io_message(&error)
                    );
                }
                whole = false;
                continue;
            }
        };
        // Each entry's path is the directory as given and its name, joined
        // with `/` as GNU's chmod prints it. An entry or a child that cannot
        // be read is reported and the walk goes on, ending with 1, as GNU's.
        let mut children = Vec::new();
        for entry in entries {
            match entry {
                Ok(entry) => children.push(entry.path()),
                Err(error) => {
                    if !options.silent {
                        uucore::context_eprintln!(
                            "{name}: cannot read directory {}: {}",
                            directory.quote(),
                            io_message(&error)
                        );
                    }
                    whole = false;
                }
            }
        }
        children.sort();
        for child in children.into_iter().rev() {
            let metadata = match fs::symlink_metadata(&child) {
                Ok(metadata) => metadata,
                Err(error) => {
                    if !options.silent {
                        uucore::context_eprintln!(
                            "{name}: cannot access {}: {}",
                            child.quote(),
                            io_message(&error)
                        );
                    }
                    whole = false;
                    continue;
                }
            };
            visit(&child, &metadata);
            if metadata.is_dir() {
                pending.push(child);
            }
        }
    }
    whole
}

const CHMOD_USAGE: &str = "\
Usage: chmod [OPTION]... MODE[,MODE]... FILE...
  or:  chmod [OPTION]... OCTAL-MODE FILE...
  or:  chmod [OPTION]... --reference=RFILE FILE...
Change the mode of each FILE to MODE.
Windows has no Unix mode: a valid change succeeds and changes nothing.

  -c, --changes          like verbose but report only when a change is made
  -f, --silent, --quiet  suppress most error messages
  -v, --verbose          output a diagnostic for every file processed
      --reference=RFILE  use RFILE's mode instead of specifying MODE values
  -R, --recursive        change files and directories recursively
      --help        display this help and exit
      --version     output version information and exit
";

pub fn chmod(args: Vec<OsString>) -> i32 {
    let name = "chmod";
    let mut options = match read(name, args, false, CHMOD_USAGE) {
        Read::Run(options) => options,
        Read::Done(status) => return status,
    };
    let (mode, files) = match files(name, &mut options) {
        Ok(found) => found,
        Err(status) => return status,
    };
    if let Some(mode) = &mode {
        let numeric_out_of_range = mode.bytes().all(|byte| byte.is_ascii_digit())
            && u32::from_str_radix(mode, 8).map_or(true, |mode| mode > 0o7777);
        let parsed = uucore::mode::parse_chmod(0o644, mode, false, uucore::context::umask());
        if numeric_out_of_range || parsed.is_err() {
            return usage_error(name, &format!("invalid mode: {}", mode.quote()));
        }
    }
    let mut status = 0;
    for file in &files {
        let whole = walk(name, &options, file, &mut |path, metadata| {
            if options.verbose && !metadata.is_symlink() {
                let mode = uucore::fs::portable_mode(metadata);
                uucore::context_println!(
                    "mode of {} retained as {:04o} ({})",
                    path.quote(),
                    mode & 0o7777,
                    uucore::fs::display_permissions_unix(mode, false)
                );
            }
        });
        if !whole {
            status = FAILED;
        }
    }
    status
}

const CHOWN_USAGE: &str = "\
Usage: chown [OPTION]... [OWNER][:[GROUP]] FILE...
  or:  chown [OPTION]... --reference=RFILE FILE...
Change the owner and/or group of each FILE to OWNER and/or GROUP.
Windows has no Unix owner: a valid change succeeds and changes nothing.
OWNER and GROUP are numbers or the names of Windows accounts.

  -c, --changes          like verbose but report only when a change is made
  -f, --silent, --quiet  suppress most error messages
  -v, --verbose          output a diagnostic for every file processed
      --dereference      affect the referent of each symbolic link (default)
  -h, --no-dereference   affect symbolic links instead of their referents
      --from=CURRENT_OWNER:CURRENT_GROUP
                         change only files that match
      --reference=RFILE  use RFILE's owner and group
  -R, --recursive        operate on files and directories recursively
      --help        display this help and exit
      --version     output version information and exit
";

pub fn chown(args: Vec<OsString>) -> i32 {
    let name = "chown";
    let mut options = match read(name, args, true, CHOWN_USAGE) {
        Read::Run(options) => options,
        Read::Done(status) => return status,
    };
    let (owner, files) = match files(name, &mut options) {
        Ok(found) => found,
        Err(status) => return status,
    };
    if let Some(owner) = &owner
        && let Err(status) = check_owner(name, owner)
    {
        return status;
    }
    let mut status = 0;
    for file in &files {
        if !walk(name, &options, file, &mut |_, _| {}) {
            status = FAILED;
        }
    }
    status
}

/// Checks `spec`, `OWNER`, `OWNER:`, `OWNER:GROUP` or `:GROUP`, as GNU's
/// `chown` does: each named part is a number or an account Windows knows.
/// GNU still reads `OWNER.GROUP` when no account has the whole name, with a
/// warning.
fn check_owner(name: &str, spec: &str) -> Result<(), i32> {
    let (user, group) = match spec.split_once(':') {
        Some(parts) => parts,
        None => match spec.split_once('.') {
            Some(parts) if !known(spec) => {
                uucore::context_eprintln!("{name}: warning: '.' should be ':'");
                parts
            }
            _ => (spec, ""),
        },
    };
    let valid = |part: &str| part.is_empty() || part.bytes().all(|byte| byte.is_ascii_digit()) || known(part);
    if !valid(user) {
        uucore::context_eprintln!("{name}: invalid user: {}", spec.quote());
        return Err(FAILED);
    }
    if !valid(group) {
        uucore::context_eprintln!("{name}: invalid group: {}", spec.quote());
        return Err(FAILED);
    }
    Ok(())
}

/// Whether Windows knows an account, a user or a group, named `account`.
fn known(account: &str) -> bool {
    use windows_sys::Win32::{Foundation::ERROR_INSUFFICIENT_BUFFER, Security::LookupAccountNameW};
    let wide: Vec<u16> = account.encode_utf16().chain(Some(0)).collect();
    let (mut sid_size, mut domain_size, mut kind) = (0u32, 0u32, 0);
    // SAFETY: `wide` is nul-terminated; with no buffers the call only
    // reports the sizes an account would need, which it fails for once it
    // found one.
    let found = unsafe {
        LookupAccountNameW(
            std::ptr::null(),
            wide.as_ptr(),
            std::ptr::null_mut(),
            &raw mut sid_size,
            std::ptr::null_mut(),
            &raw mut domain_size,
            &raw mut kind,
        )
    };
    found != 0
        || std::io::Error::last_os_error().raw_os_error() == Some(ERROR_INSUFFICIENT_BUFFER as i32)
}
