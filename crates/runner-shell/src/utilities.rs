//! In-process uutils adapters; algorithms remain in the contextual upstream crates.

#[cfg(windows)]
mod windows;

/// One utility's entry point: its arguments, the utility's own name first.
type Entry = fn(Vec<std::ffi::OsString>) -> i32;

/// Every utility a job runs in process, by the name a script calls it.
pub const UTILITIES: &[(&str, Entry)] = &[
    ("cat", |args| uu_cat::uumain(args.into_iter())),
    ("head", |args| uu_head::uumain(args.into_iter())),
    ("tail", |args| uu_tail::uumain(args.into_iter())),
    ("wc", |args| uu_wc::uumain(args.into_iter())),
    ("ls", |args| uu_ls::uumain(args.into_iter())),
    ("cp", |args| uu_cp::uumain(args.into_iter())),
    ("mv", |args| uu_mv::uumain(args.into_iter())),
    ("rm", |args| uu_rm::uumain(args.into_iter())),
    ("mkdir", |args| uu_mkdir::uumain(args.into_iter())),
    ("rmdir", |args| uu_rmdir::uumain(args.into_iter())),
    ("touch", |args| uu_touch::uumain(args.into_iter())),
    ("tee", |args| uu_tee::uumain(args.into_iter())),
    ("sort", |args| uu_sort::uumain(args.into_iter())),
    ("uniq", |args| uu_uniq::uumain(args.into_iter())),
    ("cut", |args| uu_cut::uumain(args.into_iter())),
    ("tr", |args| uu_tr::uumain(args.into_iter())),
    ("paste", |args| uu_paste::uumain(args.into_iter())),
    ("nl", |args| uu_nl::uumain(args.into_iter())),
    ("tac", |args| uu_tac::uumain(args.into_iter())),
    ("basename", |args| uu_basename::uumain(args.into_iter())),
    ("dirname", |args| uu_dirname::uumain(args.into_iter())),
    ("realpath", |args| uu_realpath::uumain(args.into_iter())),
    ("env", |args| uu_env::uumain(args.into_iter())),
    ("seq", |args| uu_seq::uumain(args.into_iter())),
    ("date", |args| uu_date::uumain(args.into_iter())),
    ("sleep", |args| uu_sleep::uumain(args.into_iter())),
    ("mktemp", |args| uu_mktemp::uumain(args.into_iter())),
    ("stat", stat),
    ("du", |args| uu_du::uumain(args.into_iter())),
    ("df", |args| uu_df::uumain(args.into_iter())),
    ("od", |args| uu_od::uumain(args.into_iter())),
    ("chmod", chmod),
    ("chown", chown),
    ("grep", |args| uu_grep::uumain(args.into_iter())),
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
    ("diff", |args| {
        usage("diff", &args, "Compare files or directories. -u unified, -c context, -y side by side, -q brief, -s identical, -e ed script, -r recursive, -N absent files as empty.")
            .unwrap_or_else(|| diffutilslib::diff::main(args.into_iter().peekable()))
    }),
    ("cmp", |args| {
        usage(
            "cmp",
            &args,
            "Compare files byte by byte. -s silent, -l list differences, -n LIMIT, -i SKIP.",
        )
        .unwrap_or_else(|| diffutilslib::cmp::main(args.into_iter().peekable()))
    }),
    ("jq", jaq::uumain),
    ("rg", ripgrep::uumain),
];

#[cfg(unix)]
fn stat(args: Vec<std::ffi::OsString>) -> i32 {
    uu_stat::uumain(args.into_iter())
}

#[cfg(windows)]
fn stat(args: Vec<std::ffi::OsString>) -> i32 {
    windows::stat(args)
}

#[cfg(unix)]
fn chmod(args: Vec<std::ffi::OsString>) -> i32 {
    uu_chmod::uumain(args.into_iter())
}

#[cfg(unix)]
fn chown(args: Vec<std::ffi::OsString>) -> i32 {
    uu_chown::uumain(args.into_iter())
}

/// Windows has no Unix mode or owner to change: the utilities succeed
/// without changing either.
#[cfg(windows)]
fn chmod(args: Vec<std::ffi::OsString>) -> i32 {
    windows_permissions("chmod", &args)
}

#[cfg(windows)]
fn chown(args: Vec<std::ffi::OsString>) -> i32 {
    windows_permissions("chown", &args)
}

#[cfg(windows)]
fn windows_permissions(name: &str, args: &[std::ffi::OsString]) -> i32 {
    if args.iter().any(|arg| arg == "--help") {
        uucore::context_println!(
            "Usage: {name} [OPTION]... FILE...\nWindows permission compatibility: succeeds without changing ownership or mode."
        );
    }
    0
}

/// The usage `diff` and `cmp` print for `--help`, which their library lacks.
fn usage(name: &str, args: &[std::ffi::OsString], summary: &str) -> Option<i32> {
    if !args.iter().any(|arg| arg == "--help") {
        return None;
    }
    uucore::context_println!("Usage: {name} [OPTION]... FILE1 FILE2");
    uucore::context_println!("{summary}");
    Some(0)
}

fn strings(args: &[std::ffi::OsString]) -> Vec<String> {
    args.iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

/// The utility a child program's name names: a utility that starts a
/// program by one of these bare names, as `find -exec`, `xargs` and `env`
/// do, starts that utility, as the job's shell would run the name
/// (`runner.md` § Standard utilities); a path names a program.
pub(crate) fn named(program: &std::ffi::OsStr) -> Option<&'static str> {
    let program = program.to_str()?;
    let names = UTILITIES.iter().map(|(name, _)| *name);
    #[cfg(unix)]
    let names = names.chain([crate::timeout::NAME]);
    names.into_iter().find(|name| *name == program)
}

/// Starts the utility `context.name`, which a utility started as its child
/// program, on a thread of the shell's pool; killing the child cancels
/// `cancellation`, which the context's control ends the run on.
pub(crate) fn start(
    tasks: &tokio_util::task::TaskTracker,
    cancellation: tokio_util::sync::CancellationToken,
    context: uucore::context::Context,
    args: Vec<std::ffi::OsString>,
) -> std::io::Result<Box<dyn uucore::context::UtilityRun>> {
    let stderr = context.stderr.clone();
    let name = context.name;
    start_with(tasks, cancellation, name, stderr, move || run(context, args))
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
