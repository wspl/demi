//! `nohup COMMAND…`: runs COMMAND with `SIGHUP` ignored, with GNU's options,
//! messages and statuses. It is the job's own on every Host, Windows included,
//! so that a script does not depend on a Host having one.
//!
//! A job's output is never a terminal, so there is no `nohup.out` to write and
//! no input to take from the null device, and where the system has no signals,
//! COMMAND only runs.
//!
//! The job's shell runs COMMAND as `exec` would, as a builtin, a standard
//! utility, a script or a program, with the shell's attributes and `SIGHUP`
//! ignored. A utility that starts `nohup`, such as `xargs`, starts COMMAND as
//! it starts any program. What COMMAND leaves running runs on, as it does
//! after any program (`work.rs`).

use std::{ffi::OsString, io::Write, sync::Arc};

use crate::scope::UtilityControl;

/// The name `nohup` goes by, as a builtin and as a utility's child program.
pub(crate) const NAME: &str = "nohup";

/// GNU's status when `nohup` itself fails.
const FAILED: u8 = 125;
const TRY_HELP: &str = "Try 'nohup --help' for more information.";
const HELP: &str = "\
Usage: nohup COMMAND [ARG]...
  or:  nohup OPTION
Run COMMAND, ignoring hangup signals.

      --help        display this help and exit
      --version     output version information and exit

If standard input is a terminal, redirect it from an unreadable file.
If standard output is a terminal, append output to 'nohup.out' if possible,
'$HOME/nohup.out' otherwise.
If standard error is a terminal, redirect it to standard output.
To save output to FILE, use 'nohup COMMAND > FILE'.

Exit status:
  125  if the nohup command itself fails, and COMMAND is not invoked
  126  if COMMAND is found but cannot be invoked
  127  if COMMAND cannot be found
  -    the exit status of COMMAND otherwise
";

/// What a `nohup` command line asks for.
pub(crate) enum Parsed {
    /// COMMAND and its arguments, never empty.
    Run(Vec<String>),
    Help,
    Version,
    /// A usage error: its message, before the line that points to `--help`.
    Usage(String),
}

/// Reads `args`, the arguments after `nohup`, as GNU's `getopt_long` reads
/// them: options end at the first operand, or at `--`.
pub(crate) fn parse(args: &[String]) -> Parsed {
    let operands = match args.first().map(String::as_str) {
        Some("--help") => return Parsed::Help,
        Some("--version") => return Parsed::Version,
        Some("--") => &args[1..],
        Some(option) if option.len() > 1 && option.starts_with('-') => {
            return Parsed::Usage(format!("invalid option -- '{}'", option.trim_start_matches('-')));
        }
        _ => args,
    };
    if operands.is_empty() {
        return Parsed::Usage("missing operand".into());
    }
    Parsed::Run(operands.to_vec())
}

/// Writes what a `nohup` command line that runs nothing prints, and gives its
/// status.
pub(crate) fn answer(parsed: &Parsed, stdout: &mut impl Write, stderr: &mut impl Write) -> u8 {
    // The status is the command line's whether or not the stream takes what
    // it prints, as a shell's builtin's is.
    let _unwritten = match parsed {
        Parsed::Run(_) => Ok(()),
        Parsed::Help => stdout.write_all(HELP.as_bytes()),
        Parsed::Version => writeln!(stdout, "nohup (Demi)"),
        Parsed::Usage(message) => {
            writeln!(stderr, "nohup: {message}").and_then(|()| writeln!(stderr, "{TRY_HELP}"))
        }
    };
    match parsed {
        Parsed::Usage(_) => FAILED,
        _ => 0,
    }
}

/// Why COMMAND could not run, as GNU words it, and the status for it.
pub(crate) fn not_run(command: &str, reason: &str, not_found: bool, stderr: &mut impl Write) -> u8 {
    // The status says it failed whether or not standard error takes why.
    let _unwritten = writeln!(stderr, "nohup: failed to run command ‘{command}’: {reason}");
    if not_found { 127 } else { 126 }
}

/// Starts `nohup` as a child program a utility started, such as `xargs nohup
/// sed …` (`crates/runner-shell/src/utilities.rs`): COMMAND starts as the
/// utility would start it, with `SIGHUP` ignored.
pub(crate) fn start(
    control: &UtilityControl,
    context: uucore::context::Context,
    args: Vec<OsString>,
) -> std::io::Result<Box<dyn uucore::context::UtilityRun>> {
    let mut attributes = control.attributes.clone();
    attributes.hangup_ignored = true;
    let part = UtilityControl {
        scope: control.scope.clone(),
        attributes,
    };
    let stderr = context.stderr.clone();
    let cancellation = control.scope.cancellation.child_token();
    let tasks = control.scope.tasks.clone();
    let work = move || {
        let words: Vec<String> = args
            .iter()
            .skip(1)
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let command = match parse(&words) {
            Parsed::Run(command) => command,
            parsed => {
                return Ok(i32::from(answer(&parsed, &mut &*context.stdout, &mut &*context.stderr)));
            }
        };
        let stderr = context.stderr.clone();
        let context = uucore::context::Context {
            control: Some(Arc::new(part)),
            ..context
        };
        uucore::context::with(context, || {
            let program = &command[0];
            let status = uucore::context::process::Command::new(program)
                .args(&command[1..])
                .status();
            Ok(match status {
                Ok(status) => i32::from(crate::scope::shell_status(status)),
                Err(error) => i32::from(not_run(
                    program,
                    &uucore::error::strip_errno(&error),
                    error.kind() == std::io::ErrorKind::NotFound,
                    &mut &*stderr,
                )),
            })
        })
    };
    crate::utilities::start_with(&tasks, cancellation, NAME, stderr, work)
}
