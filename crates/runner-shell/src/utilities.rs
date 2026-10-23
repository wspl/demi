//! The utilities a job runs in the runner, from uutils and the
//! GNU-compatible tools vendored beside it ([`UTILITIES`]), and the `sh` and
//! `bash` that start them (`nested.rs`).
//!
//! Two rules decide which utility runs:
//!
//! 1. Demi's own, on every system: the job's shell, `sh` and `bash`, and every
//!    utility that can write files or start programs, so that what it writes,
//!    and what the programs it starts write, are the command's edits, which
//!    only the runner's own utilities record. [`UTILITIES`] lists them.
//! 2. Every other utility is the system's own: GNU's on Linux, macOS's on a
//!    Mac, Git for Windows' on Windows (`crates/runner/src/git.rs`). One the
//!    Host lacks is missing, as it is on the machine itself.
//!
//! A utility that runs in the runner takes macOS's forms on a Mac where GNU's
//! refuses them or reads them otherwise (`sed -i ''`, `cp -c`), so that a model
//! that writes macOS's gets what it meant; elsewhere options, output, messages
//! and exit statuses are GNU's, in English. Every path is resolved against the
//! job's directory, never the runner's own.
//!
//! A utility that starts a program (`find -exec`, `xargs`, `env`, `timeout`):
//!
//! - runs a utility of these named by its file name in the runner ([`named`]),
//!   as the shell would, a path such as `/usr/bin/sed` or `/bin/bash`
//!   included;
//! - runs any other program as the system does.
//!
//! On Windows `/dev/null` and `/dev/zero` are devices for operands too, and
//! `/tmp` is the temporary folder (`%TMP%`, else `%TEMP%`) and `/c/…` is
//! `C:\…`, as in Git Bash. Windows tells nothing of its null device's kind,
//! so a utility that reads the operand's kind first sees an empty regular
//! file, and one that moves it is refused, as in Git Bash.
//!
//! Only a utility's open of a file and start of a program wait out a lack of
//! open files. Making every utility wait would change most of them, and the
//! raised limit makes running out rare.

#[cfg(windows)]
mod windows;

/// One utility's entry point: its arguments, the utility's own name first.
type Entry = fn(Vec<std::ffi::OsString>) -> i32;

/// Every utility a job runs in the runner, by the name a script calls it.
pub const UTILITIES: &[(&str, Entry)] = &[
    ("cp", |args| uu_cp::uumain(args.into_iter())),
    ("mv", |args| uu_mv::uumain(args.into_iter())),
    ("rm", |args| uu_rm::uumain(args.into_iter())),
    ("mkdir", |args| uu_mkdir::uumain(args.into_iter())),
    ("rmdir", |args| uu_rmdir::uumain(args.into_iter())),
    ("touch", |args| uu_touch::uumain(args.into_iter())),
    ("tee", |args| uu_tee::uumain(args.into_iter())),
    ("sort", |args| uu_sort::uumain(args.into_iter())),
    ("uniq", |args| uu_uniq::uumain(args.into_iter())),
    ("env", |args| uu_env::uumain(args.into_iter())),
    ("mktemp", |args| uu_mktemp::uumain(args.into_iter())),
    ("chmod", chmod),
    ("chown", chown),
    ("chgrp", chgrp),
    ("ln", |args| uu_ln::uumain(args.into_iter())),
    ("link", |args| uu_link::uumain(args.into_iter())),
    ("unlink", |args| uu_unlink::uumain(args.into_iter())),
    ("truncate", |args| uu_truncate::uumain(args.into_iter())),
    ("split", |args| uu_split::uumain(args.into_iter())),
    ("csplit", |args| uu_csplit::uumain(args.into_iter())),
    ("shuf", |args| uu_shuf::uumain(args.into_iter())),
    ("shred", |args| uu_shred::uumain(args.into_iter())),
    ("dd", |args| uu_dd::uumain(args.into_iter())),
    ("install", |args| uu_install::uumain(args.into_iter())),
    // Upstream builds these for Unix only: Windows has no such files.
    #[cfg(unix)]
    ("mkfifo", |args| uu_mkfifo::uumain(args.into_iter())),
    #[cfg(unix)]
    ("mknod", |args| uu_mknod::uumain(args.into_iter())),
    ("sed", |args| sed::sed::uumain(args.into_iter())),
    ("find", |args| {
        findutils::find::find_main(
            &strings(&args)
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            &findutils::find::StandardDependencies::new(),
        )
    }),
    ("xargs", |args| {
        findutils::xargs::xargs_main(
            &strings(&args)
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
        )
    }),
    // Finding a command: a name the runner runs prints bare.
    ("which", crate::which::uumain),
    #[cfg(windows)]
    ("where", crate::which::where_main),
];

#[cfg(unix)]
fn chmod(args: Vec<std::ffi::OsString>) -> i32 {
    uu_chmod::uumain(args.into_iter())
}

#[cfg(unix)]
fn chown(args: Vec<std::ffi::OsString>) -> i32 {
    uu_chown::uumain(args.into_iter())
}

#[cfg(unix)]
fn chgrp(args: Vec<std::ffi::OsString>) -> i32 {
    uu_chgrp::uumain(args.into_iter())
}

#[cfg(windows)]
fn chgrp(args: Vec<std::ffi::OsString>) -> i32 {
    windows::chgrp(args)
}

#[cfg(windows)]
fn chmod(args: Vec<std::ffi::OsString>) -> i32 {
    windows::chmod(args)
}

#[cfg(windows)]
fn chown(args: Vec<std::ffi::OsString>) -> i32 {
    windows::chown(args)
}

fn strings(args: &[std::ffi::OsString]) -> Vec<String> {
    args.iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

/// The utility a program's name names, by the file name of `program`,
/// whatever directory a path puts it in, as for `sh` and `bash`: the runner's
/// `sed` is what `sed`, `/usr/bin/sed` and `"$(which sed)"` run, and so a
/// script that names the system's path for a utility still has its writes
/// recorded. On Windows the case of the name and an `.exe` are not part of it.
pub(crate) fn named(program: &std::ffi::OsStr) -> Option<&'static str> {
    if let Some(shell) = crate::nested::shell_named(program) {
        return Some(shell);
    }
    let text = program.to_str()?;
    let name = if cfg!(windows) {
        text.rsplit(['/', '\\']).next()?
    } else {
        text.rsplit('/').next()?
    };
    let name = if cfg!(windows) && name.len() > 4 && name[name.len() - 4..].eq_ignore_ascii_case(".exe") {
        &name[..name.len() - 4]
    } else {
        name
    };
    let mut names = UTILITIES
        .iter()
        .map(|(name, _)| *name)
        .chain([crate::timeout::NAME, crate::nohup::NAME]);
    names.find(|known| {
        if cfg!(windows) {
            known.eq_ignore_ascii_case(name)
        } else {
            *known == name
        }
    })
}

/// Starts the utility `context.name`, which a utility started as its child
/// program, on a thread of the shell's pool; killing the child cancels
/// `cancellation`, which the context's control ends the run on. `running`
/// counts it among the job's utilities until it ends.
pub(crate) fn start(
    tasks: &tokio_util::task::TaskTracker,
    cancellation: tokio_util::sync::CancellationToken,
    running: crate::work::RunningUtility,
    context: uucore::context::Context,
    args: Vec<std::ffi::OsString>,
) -> std::io::Result<Box<dyn uucore::context::UtilityRun>> {
    let stderr = context.stderr.clone();
    let name = context.name;
    start_with(tasks, cancellation, name, stderr, move || {
        let _running = running;
        run(context, args)
    })
}

/// Starts `work`, the run of `name` as a utility's child program, as
/// `start` starts a utility: on a thread of the shell's pool, ended when
/// `cancellation` is; why it failed goes to `stderr`.
pub(crate) fn start_with(
    tasks: &tokio_util::task::TaskTracker,
    cancellation: tokio_util::sync::CancellationToken,
    name: &'static str,
    stderr: std::sync::Arc<std::fs::File>,
    work: impl FnOnce() -> Result<i32, String> + Send + 'static,
) -> std::io::Result<Box<dyn uucore::context::UtilityRun>> {
    let runtime = tokio::runtime::Handle::try_current().map_err(std::io::Error::other)?;
    let (sender, exit) = std::sync::mpsc::channel();
    let ended = cancellation.clone();
    tasks.spawn_blocking_on(
        move || {
            let status = match work() {
                Ok(code) => exit_status(code),
                Err(_) if ended.is_cancelled() => killed(),
                Err(error) => {
                    use std::io::Write as _;
                    // The status says it failed whether or not its standard
                    // error takes the reason.
                    let _unwritten = writeln!(&*stderr, "{name}: {error}");
                    exit_status(1)
                }
            };
            // The utility that started this one may have stopped waiting.
            let _unreceived = sender.send(status);
        },
        &runtime,
    );
    Ok(Box::new(UtilityChild {
        cancellation,
        exit,
        status: None,
    }))
}

/// A utility running as another utility's child program.
struct UtilityChild {
    cancellation: tokio_util::sync::CancellationToken,
    exit: std::sync::mpsc::Receiver<std::process::ExitStatus>,
    status: Option<std::process::ExitStatus>,
}

impl uucore::context::UtilityRun for UtilityChild {
    fn kill(&mut self) -> std::io::Result<()> {
        self.cancellation.cancel();
        Ok(())
    }

    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        if self.status.is_none() {
            match self.exit.try_recv() {
                Ok(status) => self.status = Some(status),
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    return Err(std::io::Error::other("utility thread ended without a status"));
                }
            }
        }
        Ok(self.status)
    }

    fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        if let Some(status) = self.status {
            return Ok(status);
        }
        let status = self.exit.recv().map_err(std::io::Error::other)?;
        self.status = Some(status);
        Ok(status)
    }
}

#[cfg(unix)]
fn exit_status(code: i32) -> std::process::ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    std::process::ExitStatus::from_raw((code & 0xff) << 8)
}

#[cfg(windows)]
fn exit_status(code: i32) -> std::process::ExitStatus {
    use std::os::windows::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(code as u32)
}

/// The status of a utility its starter killed: as a program killed by
/// SIGKILL on Unix, and exit status 1 on Windows, as `TerminateProcess`
/// gives.
#[cfg(unix)]
fn killed() -> std::process::ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    std::process::ExitStatus::from_raw(libc::SIGKILL)
}

#[cfg(windows)]
fn killed() -> std::process::ExitStatus {
    exit_status(1)
}

pub fn run(
    context: uucore::context::Context,
    args: Vec<std::ffi::OsString>,
) -> Result<i32, String> {
    let name = context.name;
    let entry = UTILITIES
        .iter()
        .find(|(utility, _)| *utility == name)
        .map(|(_, entry)| *entry)
        .ok_or_else(|| format!("unknown native utility: {name}"))?;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        uucore::context::with(context, || {
            uucore::locale::setup_localization(name).map_err(|error| error.to_string())?;
            Ok(entry(args))
        })
    }));
    match result {
        Ok(result) => result,
        Err(error) if error.is::<uucore::context::Cancelled>() => Err("shell job cancelled".into()),
        Err(error) => match error.downcast_ref::<uucore::context::ExitRequest>() {
            Some(request) => Ok(request.0),
            None => Err(format!("{name}: utility thread panicked")),
        },
    }
}
