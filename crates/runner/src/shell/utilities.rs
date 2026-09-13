//! In-process uutils adapters; algorithms remain in the contextual upstream crates.

pub use uucore::context::Context;

#[cfg(windows)]
mod windows;

pub const NAMES: &[&str] = &[
    "cat", "head", "tail", "wc", "ls", "cp", "mv", "rm", "mkdir", "rmdir", "touch", "tee", "sort",
    "uniq", "cut", "tr", "paste", "nl", "tac", "basename", "dirname", "realpath", "env", "seq",
    "date", "sleep", "mktemp", "stat", "du", "df", "od", "chmod", "chown", "grep", "sed", "find",
    "xargs", "diff", "cmp", "jq", "rg",
];

pub fn run(context: Context, args: Vec<std::ffi::OsString>) -> Result<i32, String> {
    let name = context.name;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        uucore::context::with(context, || {
            uucore::locale::setup_localization(name).map_err(|error| error.to_string())?;
            if matches!(name, "diff" | "cmp") && args.iter().any(|arg| arg == "--help") {
                uucore::context_println!("Usage: {name} [OPTION]... FILE1 FILE2");
                if name == "diff" {
                    uucore::context_println!(
                        "Compare files. -u unified, -c context, -y side by side, -q brief, -s identical, -e ed script."
                    );
                } else {
                    uucore::context_println!(
                        "Compare files byte by byte. -s silent, -l list differences, -n LIMIT, -i SKIP."
                    );
                }
                return Ok(0);
            }
            let code = match name {
                "cat" => uu_cat::uumain(args.into_iter()),
                "head" => uu_head::uumain(args.into_iter()),
                "tail" => uu_tail::uumain(args.into_iter()),
                "wc" => uu_wc::uumain(args.into_iter()),
                "ls" => uu_ls::uumain(args.into_iter()),
                "cp" => uu_cp::uumain(args.into_iter()),
                "mv" => uu_mv::uumain(args.into_iter()),
                "rm" => uu_rm::uumain(args.into_iter()),
                "mkdir" => uu_mkdir::uumain(args.into_iter()),
                "rmdir" => uu_rmdir::uumain(args.into_iter()),
                "touch" => uu_touch::uumain(args.into_iter()),
                "tee" => uu_tee::uumain(args.into_iter()),
                "sort" => uu_sort::uumain(args.into_iter()),
                "uniq" => uu_uniq::uumain(args.into_iter()),
                "cut" => uu_cut::uumain(args.into_iter()),
                "tr" => uu_tr::uumain(args.into_iter()),
                "paste" => uu_paste::uumain(args.into_iter()),
                "nl" => uu_nl::uumain(args.into_iter()),
                "tac" => uu_tac::uumain(args.into_iter()),
                "basename" => uu_basename::uumain(args.into_iter()),
                "dirname" => uu_dirname::uumain(args.into_iter()),
                "realpath" => uu_realpath::uumain(args.into_iter()),
                "env" => uu_env::uumain(args.into_iter()),
                "seq" => uu_seq::uumain(args.into_iter()),
                "date" => uu_date::uumain(args.into_iter()),
                "sleep" => uu_sleep::uumain(args.into_iter()),
                "mktemp" => uu_mktemp::uumain(args.into_iter()),
                #[cfg(unix)]
                "stat" => uu_stat::uumain(args.into_iter()),
                "du" => uu_du::uumain(args.into_iter()),
                "df" => uu_df::uumain(args.into_iter()),
                "od" => uu_od::uumain(args.into_iter()),
                #[cfg(unix)]
                "chmod" => uu_chmod::uumain(args.into_iter()),
                #[cfg(unix)]
                "chown" => uu_chown::uumain(args.into_iter()),
                #[cfg(windows)]
                "stat" => windows::stat(args),
                #[cfg(windows)]
                "chmod" | "chown" => {
                    if args.iter().any(|arg| arg == "--help") {
                        uucore::context_println!(
                            "Usage: {name} [OPTION]... FILE...\nWindows permission compatibility: succeeds without changing ownership or mode."
                        );
                    }
                    0
                }
                "jq" => jaq::uumain(args),
                "rg" => ripgrep::uumain(args),
                "grep" => uu_grep::uumain(args.into_iter()),
                "sed" => sed::sed::uumain(args.into_iter()),
                "diff" => diffutilslib::diff::main(args.into_iter().peekable()),
                "cmp" => diffutilslib::cmp::main(args.into_iter().peekable()),
                "find" | "xargs" => {
                    let args: Vec<_> = args.iter().map(|arg| arg.to_string_lossy()).collect();
                    let args: Vec<_> = args.iter().map(|arg| arg.as_ref()).collect();
                    if name == "find" {
                        findutils::find::find_main(
                            &args,
                            &findutils::find::StandardDependencies::new(),
                        )
                    } else {
                        findutils::xargs::xargs_main(&args)
                    }
                }
                _ => return Err(format!("unknown native utility: {name}")),
            };
            Ok(code)
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
