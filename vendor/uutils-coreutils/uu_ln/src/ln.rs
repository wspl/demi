// This file is part of the uutils coreutils package.
//
// For the full copyright and license information, please view the LICENSE
// file that was distributed with this source code.

// spell-checker:ignore (ToDO) srcpath targetpath EEXIST

use uucore::context::FileKindExt as _;
use uucore::context::PathExt as _;
use clap::{Arg, ArgAction, Command};
use uucore::context::io::{self, Write, stdout};
use uucore::display::Quotable;
use uucore::error::{UError, UIoError, UResult};

use uucore::fs::{make_path_relative_to, paths_refer_to_same_file};
use uucore::translate;
use uucore::{format_usage, prompt_yes, show_error};

use std::borrow::Cow;
use std::collections::HashSet;
use std::ffi::OsString;
use uucore::context::fs;
use thiserror::Error;

use std::path::{Path, PathBuf};
use uucore::backup_control::{self, BackupMode};
use uucore::fs::{MissingHandling, ResolveMode, canonicalize};

/// Public visibility allows other apps to integrate with our
/// `ln` utility by calling `exec` directly with their `Settings`.
pub struct Settings {
    pub overwrite: OverwriteMode,
    pub backup: BackupMode,
    pub suffix: OsString,
    pub symbolic: bool,
    pub relative: bool,
    pub logical: bool,
    pub target_dir: Option<PathBuf>,
    pub no_target_dir: bool,
    pub no_dereference: bool,
    pub verbose: bool,
    /// macOS's `-F`: a directory that is the destination of a symbolic link
    /// is removed, when it is empty, so that the link may be made
    pub replace_directory: bool,
    /// macOS's `-w`: a symbolic link whose source is missing is warned of
    pub warn_missing: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OverwriteMode {
    NoClobber,
    Interactive,
    Force,
}

#[derive(Error, Debug)]
pub enum LnError {
    #[error("{}", translate!("ln-error-target-is-not-directory", "target" => _0.quote()))]
    TargetIsNotADirectory(PathBuf),

    #[error("{0}")]
    Io(#[from] UIoError),

    #[error("{1}: {0}")]
    IoContext(UIoError, String),

    #[error("")]
    SomeLinksFailed,

    #[error("{}", translate!("ln-error-same-file", "file1" => _0.quote(), "file2" => _1.quote()))]
    SameFile(PathBuf, PathBuf),

    #[error("{}", translate!("ln-error-missing-destination", "operand" => _0.quote()))]
    MissingDestination(PathBuf),

    #[error("{}", translate!("ln-error-extra-operand", "operand" => _0.quote(), "program" => _1))]
    ExtraOperand(OsString, String),

    #[error("{}", translate!("ln-failed-to-create-hard-link-dir", "source" => _0.to_string_lossy()))]
    FailedToCreateHardLinkDir(PathBuf),
}

impl UError for LnError {
    fn code(&self) -> i32 {
        1
    }
}
pub type LnResult<T> = Result<T, LnError>;

impl From<io::Error> for LnError {
    fn from(err: io::Error) -> Self {
        Self::Io(UIoError::from(err))
    }
}

mod options {
    pub const FORCE: &str = "force";
    //pub const DIRECTORY: &str = "directory";
    pub const INTERACTIVE: &str = "interactive";
    pub const NO_DEREFERENCE: &str = "no-dereference";
    pub const SYMBOLIC: &str = "symbolic";
    pub const LOGICAL: &str = "logical";
    pub const PHYSICAL: &str = "physical";
    pub const TARGET_DIRECTORY: &str = "target-directory";
    pub const NO_TARGET_DIRECTORY: &str = "no-target-directory";
    pub const RELATIVE: &str = "relative";
    pub const VERBOSE: &str = "verbose";
    #[cfg(target_os = "macos")]
    pub const REPLACE_DIRECTORY: &str = "replace-directory";
    #[cfg(target_os = "macos")]
    pub const WARN_MISSING: &str = "warn-missing";
}

static ARG_FILES: &str = "files";

#[uucore::main]
pub fn uumain(args: impl uucore::Args) -> UResult<()> {
    let matches = uucore::clap_localization::handle_clap_result(uu_app(), args)?;

    /* the list of files */

    let paths: Vec<PathBuf> = matches
        .get_many::<OsString>(ARG_FILES)
        .unwrap()
        .map(PathBuf::from)
        .collect();

    let symbolic = matches.get_flag(options::SYMBOLIC);

    let replace_directory = replace_directory(&matches);
    let overwrite_mode = if matches.get_flag(options::FORCE) || replace_directory {
        OverwriteMode::Force
    } else if matches.get_flag(options::INTERACTIVE) {
        OverwriteMode::Interactive
    } else {
        OverwriteMode::NoClobber
    };

    let backup_mode =
        backup_control::determine_backup_mode(uucore::context::env::var("VERSION_CONTROL").ok(), &matches)?;
    let backup_suffix = backup_control::determine_backup_suffix(&matches);

    // When we have "-L" or "-L -P", false otherwise
    let logical = matches.get_flag(options::LOGICAL);

    let settings = Settings {
        overwrite: overwrite_mode,
        backup: backup_mode,
        suffix: OsString::from(backup_suffix),
        symbolic,
        logical,
        relative: matches.get_flag(options::RELATIVE),
        target_dir: matches
            .get_one::<OsString>(options::TARGET_DIRECTORY)
            .map(PathBuf::from),
        no_target_dir: matches.get_flag(options::NO_TARGET_DIRECTORY),
        no_dereference: matches.get_flag(options::NO_DEREFERENCE),
        verbose: matches.get_flag(options::VERBOSE),
        replace_directory,
        warn_missing: warn_missing(&matches),
    };

    exec(&paths[..], &settings)?;
    Ok(())
}

/// Whether macOS's `-F` was given, which implies `-f`.
#[cfg(target_os = "macos")]
fn replace_directory(matches: &clap::ArgMatches) -> bool {
    matches.get_flag(options::REPLACE_DIRECTORY)
}

#[cfg(not(target_os = "macos"))]
fn replace_directory(_matches: &clap::ArgMatches) -> bool {
    false
}

/// Whether macOS's `-w` was given.
#[cfg(target_os = "macos")]
fn warn_missing(matches: &clap::ArgMatches) -> bool {
    matches.get_flag(options::WARN_MISSING)
}

#[cfg(not(target_os = "macos"))]
fn warn_missing(_matches: &clap::ArgMatches) -> bool {
    false
}

pub fn uu_app() -> Command {
    let after_help = format!(
        "{}\n\n{}",
        translate!("ln-after-help"),
        backup_control::BACKUP_CONTROL_LONG_HELP
    );

    // GNU's `ln` has no `-h`, which clap would take for help and so link
    // nothing; macOS's is `-n`.
    let command = Command::new("ln")
        .disable_help_flag(true)
        .arg(
            Arg::new("help")
                .long("help")
                .help(translate!("ln-help-help"))
                .action(ArgAction::HelpLong),
        )
        .version(uucore::crate_version!())
        .help_template(uucore::localized_help_template("ln"))
        .about(translate!("ln-about"))
        .override_usage(format_usage(&translate!("ln-usage")))
        .infer_long_args(true)
        .after_help(after_help)
        .arg(backup_control::arguments::backup())
        .arg(backup_control::arguments::backup_no_args())
        /*.arg(
            Arg::new(options::DIRECTORY)
                .short('d')
                .long(options::DIRECTORY)
                .help("allow users with appropriate privileges to attempt to make hard links to directories")
        )*/
        .arg(
            Arg::new(options::FORCE)
                .short('f')
                .long(options::FORCE)
                .help(translate!("ln-help-force"))
                .overrides_with(options::INTERACTIVE)
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new(options::INTERACTIVE)
                .short('i')
                .long(options::INTERACTIVE)
                .help(translate!("ln-help-interactive"))
                .overrides_with(options::FORCE)
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new(options::NO_DEREFERENCE)
                .short('n')
                .long(options::NO_DEREFERENCE)
                .help(translate!("ln-help-no-dereference"))
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new(options::LOGICAL)
                .short('L')
                .long(options::LOGICAL)
                .help(translate!("ln-help-logical"))
                .overrides_with(options::PHYSICAL)
                .action(ArgAction::SetTrue),
        )
        .arg(
            // Not implemented yet
            Arg::new(options::PHYSICAL)
                .short('P')
                .long(options::PHYSICAL)
                .help(translate!("ln-help-physical"))
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new(options::SYMBOLIC)
                .short('s')
                .long(options::SYMBOLIC)
                .help(translate!("ln-help-symbolic"))
                // override added for https://github.com/uutils/coreutils/issues/2359
                .overrides_with(options::SYMBOLIC)
                .action(ArgAction::SetTrue),
        )
        .arg(backup_control::arguments::suffix())
        .arg(
            Arg::new(options::TARGET_DIRECTORY)
                .short('t')
                .long(options::TARGET_DIRECTORY)
                .help(translate!("ln-help-target-directory"))
                .value_name("DIRECTORY")
                .value_hint(clap::ValueHint::DirPath)
                .value_parser(clap::value_parser!(OsString))
                .conflicts_with(options::NO_TARGET_DIRECTORY),
        )
        .arg(
            Arg::new(options::NO_TARGET_DIRECTORY)
                .short('T')
                .long(options::NO_TARGET_DIRECTORY)
                .help(translate!("ln-help-no-target-directory"))
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new(options::RELATIVE)
                .short('r')
                .long(options::RELATIVE)
                .help(translate!("ln-help-relative"))
                .requires(options::SYMBOLIC)
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new(options::VERBOSE)
                .short('v')
                .long(options::VERBOSE)
                .help(translate!("ln-help-verbose"))
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new(ARG_FILES)
                .action(ArgAction::Append)
                .value_hint(clap::ValueHint::AnyPath)
                .value_parser(clap::value_parser!(OsString))
                .required(true)
                .num_args(1..),
        );
    #[cfg(target_os = "macos")]
    let command = command
        .mut_arg(options::NO_DEREFERENCE, |arg| arg.short_alias('h'))
        .arg(
            Arg::new(options::REPLACE_DIRECTORY)
                .short('F')
                .help(translate!("ln-help-replace-directory"))
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new(options::WARN_MISSING)
                .short('w')
                .help(translate!("ln-help-warn-missing"))
                .action(ArgAction::SetTrue),
        );
    command
}

fn exec(files: &[PathBuf], settings: &Settings) -> LnResult<()> {
    // Handle cases where we create links in a directory first.
    if let Some(ref target_path) = settings.target_dir {
        // 4th form: a directory is specified by -t.
        return link_files_in_dir(files, target_path, settings);
    }

    // macOS's -F makes the link in place of an empty directory that is the
    // destination, unless the destination is written with a trailing slash.
    if settings.replace_directory
        && settings.symbolic
        && !settings.no_target_dir
        && let [source, destination] = files
        && !destination.as_os_str().as_encoded_bytes().ends_with(b"/")
        && destination.context_is_dir()
        && !destination.context_is_symlink()
    {
        return link(source, destination, settings);
    }

    // if -T is specified we may have a wrong number of operands.
    match files.split_last().expect("clap rejects empty") {
        // the target directory is the current directory
        (_, []) if !settings.no_target_dir => {
            link_files_in_dir(files, &PathBuf::from("."), settings)
        }
        // create links in the last argument
        (last, rest) if !settings.no_target_dir && (rest.len() > 1 || last.context_is_dir()) => {
            link_files_in_dir(rest, last, settings)
        }
        (f0, []) => Err(LnError::MissingDestination(f0.clone())),
        (last, [f]) => link(f, last, settings),
        (extra, [_, _]) | (_, [_, _, extra, ..]) => Err(LnError::ExtraOperand(
            extra.into(),
            uucore::execution_phrase().to_string(),
        )),
    }
}

#[allow(clippy::cognitive_complexity)]
fn link_files_in_dir(files: &[PathBuf], target_dir: &Path, settings: &Settings) -> LnResult<()> {
    if !target_dir.context_is_dir() {
        return Err(LnError::TargetIsNotADirectory(target_dir.to_owned()));
    }
    // remember the linked destinations for further usage
    let mut linked_destinations: HashSet<PathBuf> = HashSet::with_capacity(files.len());

    let mut all_successful = true;
    for srcpath in files {
        let targetpath = if settings.no_dereference && target_dir.context_is_symlink() {
            let remove_target = || {
                // Not sure why but on Windows, the symlink can be
                // considered as a dir
                // See test_ln::test_symlink_no_deref_dir
                #[cfg(windows)]
                if let Err(e) = fs::remove_dir(target_dir) {
                    show_error!(
                        "{}",
                        translate!("ln-error-could-not-update", "target" => target_dir.quote(), "error" => e)
                    );
                }
            };
            match settings.overwrite {
                OverwriteMode::NoClobber => {}
                OverwriteMode::Interactive => {
                    if prompt_yes!(
                        "{}",
                        translate!("ln-prompt-replace", "file" => target_dir.quote())
                    ) {
                        remove_target();
                    }
                }
                OverwriteMode::Force => {
                    remove_target();
                }
            }
            target_dir.to_path_buf()
        } else {
            match srcpath.file_name() {
                Some(basename) => target_dir.join(basename),
                // This can be None only for "." or "..". Trying
                // to create a link with such name will fail with
                // EEXIST, which agrees with the behavior of GNU
                // coreutils.
                None => target_dir.join(srcpath),
            }
        };

        if linked_destinations.contains(&targetpath) {
            // If the target file was already created in this ln call, do not overwrite
            show_error!(
                "{}",
                translate!("ln-error-will-not-overwrite", "target" => targetpath.quote(), "source" => srcpath.quote())
            );
            all_successful = false;
        } else if let Err(e) = link(srcpath, &targetpath, settings) {
            show_error!("{e}");
            all_successful = false;
        }

        linked_destinations.insert(targetpath);
    }
    if all_successful {
        Ok(())
    } else {
        Err(LnError::SomeLinksFailed)
    }
}

fn relative_path<'a>(src: &'a Path, dst: &Path) -> Cow<'a, Path> {
    // `dst.parent()` is None for a destination with no parent (`/`, `""`, or a
    // bare Windows prefix). Fall through to the non-relative `src` rather than
    // unwrapping it; the caller then reports the usual error.
    let Some(dst_parent) = dst.parent() else {
        return src.into();
    };
    let (Ok(src_abs), Ok(dst_abs)) = (
        canonicalize(src, MissingHandling::Missing, ResolveMode::Physical),
        canonicalize(dst_parent, MissingHandling::Missing, ResolveMode::Physical),
    ) else {
        return src.into();
    };

    make_path_relative_to(src_abs, dst_abs).into()
}

/// Decide whether `src` and `dst` are actually the same directory entry.
fn is_same_entry(src: &Path, dst: &Path) -> bool {
    match (
        canonicalize(src, MissingHandling::Missing, ResolveMode::Physical),
        canonicalize(dst, MissingHandling::Missing, ResolveMode::Physical),
    ) {
        (Ok(src), Ok(dst)) => src == dst,
        _ => true,
    }
}

/// Create symlink to src at dst with the given settings
///
/// This is made public to allow other apps to use `ln` as a library.
#[allow(clippy::cognitive_complexity)]
pub fn link(src: &Path, dst: &Path, settings: &Settings) -> LnResult<()> {
    let mut backup_path = None;
    let source: Cow<'_, Path> = if settings.relative {
        relative_path(src, dst)
    } else {
        src.into()
    };

    if settings.replace_directory
        && settings.symbolic
        && dst.context_is_dir()
        && !dst.context_is_symlink()
    {
        let _edit = uucore::context::edit_remove(dst);
        fs::remove_dir(dst).map_err(|e| {
            LnError::IoContext(UIoError::from(e), dst.display().to_string())
        })?;
    }

    // A relative source is looked for where the link is, as the link reads it.
    let source_exists = match dst.parent() {
        Some(directory) if src.is_relative() => directory.join(src).context_exists(),
        _ => src.context_exists(),
    };
    if settings.warn_missing && settings.symbolic && !source_exists {
        show_error!("warning: {}: {}", src.maybe_quote(), translate!("ln-warning-missing-source"));
    }

    if dst.context_is_symlink() || dst.context_exists() {
        backup_path = backup_control::get_backup_path(settings.backup, dst, &settings.suffix);
        if settings.backup == BackupMode::Existing && !settings.symbolic {
            // when ln --backup f f, it should detect that it is the same file
            if paths_refer_to_same_file(src, dst, true) && is_same_entry(src, dst) {
                return Err(LnError::SameFile(src.to_owned(), dst.to_owned()));
            }
        }
        if let Some(ref p) = backup_path {
            fs::rename(dst, p).map_err(|e| {
                LnError::IoContext(
                    UIoError::from(e),
                    translate!("ln-cannot-backup", "file" => dst.quote()),
                )
            })?;
        }
        match settings.overwrite {
            OverwriteMode::NoClobber => {}
            OverwriteMode::Interactive => {
                if !prompt_yes!("{}", translate!("ln-prompt-replace", "file" => dst.quote())) {
                    return Err(LnError::SomeLinksFailed);
                }

                let _edit = uucore::context::edit_remove(dst);
                let _ = fs::remove_file(dst);
                // In case of error, don't do anything
            }
            OverwriteMode::Force => {
                if !dst.context_is_symlink()
                    && paths_refer_to_same_file(src, dst, true)
                    && is_same_entry(src, dst)
                {
                    // Even in force overwrite mode, verify we are not targeting the same entry and return a SameFile error if so
                    return Err(LnError::SameFile(src.to_owned(), dst.to_owned()));
                }
                let _edit = uucore::context::edit_remove(dst);
                let _ = fs::remove_file(dst);
                // In case of error, don't do anything
            }
        }
    }

    let res = if settings.symbolic {
        symlink(&source, dst).map_err(|e| {
            LnError::IoContext(
                UIoError::from(e),
                translate!(
                    "ln-failed-to-create-symbolic-link",
                    "dest" => dst.quote()
                ),
            )
        })
    } else {
        let p = if settings.logical && source.context_is_symlink() {
            fs::canonicalize(&source).map_err(|e| {
                LnError::IoContext(
                    UIoError::from(e),
                    translate!("ln-failed-to-access", "file" => source.quote()),
                )
            })?
        } else {
            source.to_path_buf()
        };
        match fs::hard_link(&p, dst) {
            Ok(()) => Ok(()),
            Err(_) if p.context_is_dir() => Err(LnError::FailedToCreateHardLinkDir(source.to_path_buf())),
            Err(e) => Err(LnError::IoContext(
                UIoError::from(e),
                translate!(
                    "ln-failed-to-create-hard-link",
                    "source" => source.quote(),
                    "dest" => dst.quote()
                ),
            )),
        }
    };

    if let Err(e) = res {
        if let Some(ref p) = backup_path {
            fs::rename(p, dst).map_err(|e| {
                LnError::IoContext(
                    UIoError::from(e),
                    translate!("ln-cannot-backup", "file" => dst.quote()),
                )
            })?;
        }
        return Err(e);
    }

    if settings.verbose {
        let mut out = stdout();
        write!(out, "{} -> {}", dst.quote(), source.quote())?;
        match backup_path {
            Some(path) => writeln!(
                out,
                " ({})",
                translate!("ln-backup", "backup" => path.quote())
            )?,
            None => writeln!(out)?,
        }
    }
    Ok(())
}

#[cfg(windows)]
pub fn symlink<P1: AsRef<Path>, P2: AsRef<Path>>(src: P1, dst: P2) -> io::Result<()> {
    use uucore::context::fs::{symlink_dir, symlink_file};
    if src.as_ref().context_is_dir() {
        symlink_dir(src, dst)
    } else {
        symlink_file(src, dst)
    }
}

#[cfg(any(unix, target_os = "wasi"))]
pub fn symlink<P1: AsRef<Path>, P2: AsRef<Path>>(src: P1, dst: P2) -> io::Result<()> {
    fs::symlink(src.as_ref(), dst.as_ref())
}
