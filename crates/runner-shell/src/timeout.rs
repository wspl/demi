//! `timeout DURATION COMMAND…`: runs COMMAND as a part of the job of its
//! own and stops it as `kill` stops a task once DURATION has passed, with
//! GNU's options, durations, messages and statuses (`runner.md` § Background
//! tasks and timeouts). The job's shell runs COMMAND as `command` would; a
//! utility that starts `timeout`, such as `xargs`, starts COMMAND as it
//! starts any program.

use std::{ffi::OsString, future::Future, io::Write, sync::Arc, time::Duration};

use brush_core::traps::TrapSignal;

use crate::scope::{Scope, UtilityControl};

/// The name `timeout` goes by, as a builtin and as a utility's child
/// program.
pub(crate) const NAME: &str = "timeout";

/// GNU's status when the time ran out.
const TIMED_OUT: u8 = 124;
/// GNU's status when `timeout` itself fails.
const FAILED: u8 = 125;
const TRY_HELP: &str = "Try 'timeout --help' for more information.";
const HELP: &str = "\
Usage: timeout [OPTION] DURATION COMMAND [ARG]...
  or:  timeout [OPTION]
Start COMMAND, and kill it if still running after DURATION.

  --preserve-status
                 exit with the same status as COMMAND, even when the
                   command times out
  --foreground
                 when not running timeout directly from a shell prompt,
                   allow COMMAND to read from the TTY and get TTY signals;
                   in this mode, children of COMMAND will not be timed out
  -k, --kill-after=DURATION
                 also send a KILL signal if COMMAND is still running
                   this long after the initial signal was sent
  -s, --signal=SIGNAL
                 specify the signal to be sent on timeout;
                   SIGNAL may be a name like 'HUP' or a number;
                   see 'kill -l' for a list of signals
  -v, --verbose  diagnose to stderr any signal sent upon timeout
      --help        display this help and exit
      --version     output version information and exit

DURATION is a floating point number with an optional suffix:
's' for seconds (the default), 'm' for minutes, 'h' for hours or 'd' for days.
A duration of 0 disables the associated timeout.

Upon timeout, send the TERM signal to COMMAND, if no other SIGNAL specified.
The TERM signal kills any process that does not block or catch that signal.
It may be necessary to use the KILL signal, since this signal cannot be caught.

Exit status:
  124  if COMMAND times out, and --preserve-status is not specified
  125  if the timeout command itself fails
  126  if COMMAND is found but cannot be invoked
  127  if COMMAND cannot be found
  137  if COMMAND (or timeout itself) is sent the KILL (9) signal (128+9)
  -    the exit status of COMMAND otherwise
";

/// What a `timeout` command line asks for.
pub(crate) struct Options {
    signal: i32,
    kill_after: Option<Duration>,
    preserve_status: bool,
    foreground: bool,
    verbose: bool,
    duration: Option<Duration>,
    /// COMMAND and its arguments, never empty.
    pub(crate) command: Vec<String>,
}

/// A `timeout` command line, read as GNU's reads it.
pub(crate) enum Parsed {
    Run(Options),
    Help,
    Version,
    /// A usage error: its message, if any, before the line that points to
    /// `--help`.
    Usage(Option<String>),
}

/// The long options and whether each takes an argument.
const LONG: &[(&str, bool)] = &[
    ("foreground", false),
    ("kill-after", true),
    ("preserve-status", false),
    ("signal", true),
    ("verbose", false),
    ("help", false),
    ("version", false),
];

/// Reads `args`, the arguments after `timeout`, as GNU's `getopt_long`
/// reads them: options end at the first operand, short options may come
/// together, and a long option may be any unambiguous prefix of its name.
pub(crate) fn parse(args: &[String]) -> Parsed {
    let mut signal = libc::SIGTERM;
    let mut kill_after = None;
    let mut preserve_status = false;
    let mut foreground = false;
    let mut verbose = false;
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        index += 1;
        if arg == "--" {
            break;
        }
        if let Some(long) = arg.strip_prefix("--") {
            let (name, value) = match long.split_once('=') {
                Some((name, value)) => (name, Some(value.to_owned())),
                None => (long, None),
            };
            let matches: Vec<_> = LONG
                .iter()
                .filter(|(option, _)| option.starts_with(name))
                .collect();
            let exact = LONG.iter().find(|(option, _)| *option == name);
            let &(option, takes) = match (exact, matches.as_slice()) {
                (Some(exact), _) => exact,
                (None, [only]) => only,
                (None, []) => {
                    return Parsed::Usage(Some(format!("unrecognized option '--{name}'")));
                }
                (None, several) => {
                    let possibilities: Vec<_> = several
                        .iter()
                        .map(|(option, _)| format!("'--{option}'"))
                        .collect();
                    return Parsed::Usage(Some(format!(
                        "option '--{name}' is ambiguous; possibilities: {}",
                        possibilities.join(" ")
                    )));
                }
            };
            let value = match (takes, value) {
                (false, Some(_)) => {
                    return Parsed::Usage(Some(format!(
                        "option '--{option}' doesn't allow an argument"
                    )));
                }
                (false, None) => None,
                (true, Some(value)) => Some(value),
                (true, None) => match args.get(index) {
                    Some(value) => {
                        index += 1;
                        Some(value.clone())
                    }
                    None => {
                        return Parsed::Usage(Some(format!(
                            "option '--{option}' requires an argument"
                        )));
                    }
                },
            };
            match (option, value) {
                ("foreground", _) => foreground = true,
                ("preserve-status", _) => preserve_status = true,
                ("verbose", _) => verbose = true,
                ("help", _) => return Parsed::Help,
                ("version", _) => return Parsed::Version,
                ("kill-after", Some(value)) => match duration(&value) {
                    Ok(after) => kill_after = after,
                    Err(usage) => return usage,
                },
                ("signal", Some(value)) => match signal_number(&value) {
                    Ok(number) => signal = number,
                    Err(usage) => return usage,
                },
                _ => unreachable!("every long option is read above"),
            }
            continue;
        }
        let Some(shorts) = arg.strip_prefix('-').filter(|shorts| !shorts.is_empty()) else {
            // The first operand ends the options.
            index -= 1;
            break;
        };
        for (at, option) in shorts.char_indices() {
            match option {
                'v' => verbose = true,
                'k' | 's' => {
                    let rest = &shorts[at + option.len_utf8()..];
                    let value = if rest.is_empty() {
                        let Some(value) = args.get(index) else {
                            return Parsed::Usage(Some(format!(
                                "option requires an argument -- '{option}'"
                            )));
                        };
                        index += 1;
                        value.clone()
                    } else {
                        rest.to_owned()
                    };
                    if option == 'k' {
                        match duration(&value) {
                            Ok(after) => kill_after = after,
                            Err(usage) => return usage,
                        }
                    } else {
                        match signal_number(&value) {
                            Ok(number) => signal = number,
                            Err(usage) => return usage,
                        }
                    }
                    break;
                }
                option => {
                    return Parsed::Usage(Some(format!("invalid option -- '{option}'")));
                }
            }
        }
    }
    let operands = &args[index.min(args.len())..];
    let [duration_text, command @ ..] = operands else {
        return Parsed::Usage(None);
    };
    if command.is_empty() {
        return Parsed::Usage(None);
    }
    match duration(duration_text) {
        Ok(duration) => Parsed::Run(Options {
            signal,
            kill_after,
            preserve_status,
            foreground,
            verbose,
            duration,
            command: command.to_vec(),
        }),
        Err(usage) => usage,
    }
}

/// A DURATION as GNU reads it: a floating point number of seconds with an
/// optional suffix, `s`, `m`, `h` or `d`; 0 is none.
fn duration(text: &str) -> Result<Option<Duration>, Parsed> {
    let invalid = || Parsed::Usage(Some(format!("invalid time interval ‘{text}’")));
    let (number, unit) = match text.char_indices().last() {
        Some((at, 's')) => (&text[..at], 1.0),
        Some((at, 'm')) => (&text[..at], 60.0),
        Some((at, 'h')) => (&text[..at], 3600.0),
        Some((at, 'd')) => (&text[..at], 86400.0),
        _ => (text, 1.0),
    };
    let seconds = number.parse::<f64>().map_err(|_| invalid())? * unit;
    if seconds.is_nan() || seconds < 0.0 {
        return Err(invalid());
    }
    if seconds == 0.0 {
        return Ok(None);
    }
    // A duration beyond what a timer counts never runs out.
    Ok(Duration::try_from_secs_f64(seconds).ok())
}

/// A SIGNAL as GNU reads it: a number, or a name in any case with or
/// without `SIG`.
fn signal_number(text: &str) -> Result<i32, Parsed> {
    let parsed = if text.bytes().all(|byte| byte.is_ascii_digit()) {
        text.parse::<i32>()
            .ok()
            .and_then(|number| TrapSignal::try_from(number).ok())
    } else {
        TrapSignal::try_from(text)
            .ok()
            .filter(|signal| matches!(signal, TrapSignal::Signal(_)))
    };
    parsed
        .and_then(|signal| i32::try_from(signal).ok())
        .ok_or_else(|| Parsed::Usage(Some(format!("‘{text}’: invalid signal"))))
}

/// Writes what a `timeout` command line that runs nothing prints, and gives
/// its status.
pub(crate) fn answer(parsed: &Parsed, stdout: &mut impl Write, stderr: &mut impl Write) -> u8 {
    // The status is the command line's whether or not the stream takes what
    // it prints, as a shell's builtin's is.
    let _unwritten = match parsed {
        Parsed::Run(_) => Ok(()),
        Parsed::Help => stdout.write_all(HELP.as_bytes()),
        Parsed::Version => writeln!(stdout, "timeout (Demi)"),
        Parsed::Usage(message) => message
            .iter()
            .try_for_each(|message| writeln!(stderr, "timeout: {message}"))
            .and_then(|()| writeln!(stderr, "{TRY_HELP}")),
    };
    match parsed {
        Parsed::Usage(_) => FAILED,
        _ => 0,
    }
}

/// Why COMMAND could not run, as GNU words it, and the status for it.
pub(crate) fn not_run(command: &str, reason: &str, not_found: bool, stderr: &mut impl Write) -> u8 {
    // The status says it failed whether or not standard error takes why.
    let _unwritten = writeln!(stderr, "timeout: failed to run command ‘{command}’: {reason}");
    if not_found { 127 } else { 126 }
}

/// Runs `command`, the run of COMMAND in `scope`, a part of the job of its
/// own, and stops that part as the options say once the time runs out.
/// Gives `command`'s own end, or the status a timeout gives: `124`, or `137`
/// once `KILL` was sent, or with `--preserve-status` COMMAND's own, 128 plus
/// the signal that stopped it when it gave none.
pub(crate) async fn supervise<E>(
    options: &Options,
    scope: &Scope,
    command: impl Future<Output = Result<u8, E>>,
    stderr: &mut impl Write,
) -> Result<u8, E> {
    tokio::pin!(command);
    let kill = libc::SIGKILL;
    let mut send = |signal: i32| {
        if options.verbose {
            let name = TrapSignal::try_from(signal).map_or_else(
                |_| signal.to_string(),
                |signal| signal.as_str().trim_start_matches("SIG").to_owned(),
            );
            // The signal goes whether or not standard error takes the line.
            let _unwritten = writeln!(
                stderr,
                "timeout: sending signal {name} to command ‘{}’",
                options.command[0]
            );
        }
        if let Err(error) = scope.signal(signal, options.foreground) {
            tracing::warn!("timeout could not signal its command: {error}");
        }
    };
    let Some(duration) = options.duration else {
        return command.await;
    };
    tokio::select! {
        end = &mut command => return end,
        () = tokio::time::sleep(duration) => {}
    }
    send(options.signal);
    let mut killed = options.signal == kill;
    let end = match options.kill_after.filter(|_| !killed) {
        Some(after) => tokio::select! {
            end = &mut command => end,
            () = tokio::time::sleep(after) => {
                send(kill);
                killed = true;
                command.await
            }
        },
        None => command.await,
    };
    if killed {
        return Ok(128 + kill as u8);
    }
    if !options.preserve_status {
        return Ok(TIMED_OUT);
    }
    match end {
        Ok(status) => Ok(status),
        // COMMAND's shell work ended at the signal and gave no status.
        Err(_) => Ok(128 + options.signal as u8),
    }
}

/// Starts `timeout` as a child program a utility started, such as `xargs
/// timeout 5 sed …` (`runner.md` § Standard utilities): it starts COMMAND as
/// the utility would, a standard utility by its name or a program, in a part
/// of the job of its own, which killing the child kills.
pub(crate) fn start(
    control: &UtilityControl,
    context: uucore::context::Context,
    args: Vec<OsString>,
) -> std::io::Result<Box<dyn uucore::context::UtilityRun>> {
    // Killing the child cancels this, which kills what COMMAND runs.
    let cancellation = control.scope.cancellation.child_token();
    let killed = cancellation.clone();
    let part = UtilityControl {
        scope: control.scope.part(),
        attributes: control.attributes.clone(),
    };
    let stderr = context.stderr.clone();
    let tasks = control.scope.tasks.clone();
    let work = move || {
        let words: Vec<String> = args
            .iter()
            .skip(1)
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let options = match parse(&words) {
            Parsed::Run(options) => options,
            parsed => return Ok(i32::from(answer(&parsed, &mut &*context.stdout, &mut &*context.stderr))),
        };
        let runtime = tokio::runtime::Handle::current();
        let scope = part.scope.clone();
        let mut stderr = context.stderr.clone();
        let command = uucore::context::Context {
            control: Some(Arc::new(part)),
            ..context
        };
        let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            uucore::context::with(command, || {
                let program = &options.command[0];
                let mut child = match uucore::context::process::Command::new(program)
                    .args(&options.command[1..])
                    .spawn()
                {
                    Ok(child) => child,
                    Err(error) => {
                        let reason = uucore::error::strip_errno(&error);
                        let not_found = error.kind() == std::io::ErrorKind::NotFound;
                        return Ok(not_run(program, &reason, not_found, &mut &*stderr));
                    }
                };
                let ended = async {
                    loop {
                        if let Some(status) = child.try_wait()? {
                            return Ok(crate::scope::shell_status(status));
                        }
                        if killed.is_cancelled() {
                            scope.stop.abort();
                        }
                        // Dropping the child kills what it runs.
                        scope.check_processes()?;
                        tokio::time::sleep(Duration::from_millis(25)).await;
                    }
                };
                runtime.block_on(supervise(&options, &scope, ended, &mut stderr))
            })
        }));
        match ran {
            Ok(Ok(status)) => Ok(i32::from(status)),
            Ok(Err(error)) => Err::<i32, String>(uucore::error::strip_errno(&error)),
            Err(_) => Err("shell job cancelled".to_owned()),
        }
    };
    crate::utilities::start_with(&tasks, cancellation, NAME, stderr, work)
}
