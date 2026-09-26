//! In-process uutils adapters; algorithms remain in the contextual upstream crates.

pub use uucore::context::Context;

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
    ("find", |args| findutils::find::find_main(&strings(&args).iter().map(String::as_str).collect::<Vec<_>>(), &findutils::find::StandardDependencies::new())),
    ("xargs", |args| findutils::xargs::xargs_main(&strings(&args).iter().map(String::as_str).collect::<Vec<_>>())),
    ("diff", |args| {
        usage("diff", &args, "Compare files. -u unified, -c context, -y side by side, -q brief, -s identical, -e ed script.")
            .unwrap_or_else(|| diffutilslib::diff::main(args.into_iter().peekable()))
    }),
    ("cmp", |args| {
        usage("cmp", &args, "Compare files byte by byte. -s silent, -l list differences, -n LIMIT, -i SKIP.")
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

pub fn run(context: Context, args: Vec<std::ffi::OsString>) -> Result<i32, String> {
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
