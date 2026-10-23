//! `sh` and `bash`: a new shell of the job's own, on every Host.
//!
//! A job that runs `sh -c …` or `bash x.sh`, or a script whose first line
//! names one, would otherwise start the system's program: dash, macOS's
//! bash 3.2, Git's MSYS2 shell or the WSL launcher, whose utilities are the
//! system's and whose writes no edit records. Here the command runs a new
//! shell inside the runner, built as the job's own is:
//!
//! ```text
//! sh / bash / /bin/bash / #!/bin/sh ──┬─ a builtin of the job's shell
//!                                     └─ a child program a utility starts
//!                                        (find -exec, xargs, env, …)
//!              │
//!              ▼
//!   exported variables, directory, descriptors, builtins and attributes
//!   of the caller ─▶ a new brush shell under a part of the job
//!              │
//!              ▼
//!       -c string │ script file │ standard input
//! ```
//!
//! - The shell takes bash's options, and `sh` runs in POSIX mode.
//! - It owns a part of the job (`Scope::shell`) numbered as a background task
//!   is, which its `$$` gives: `kill $$` ends the shell and what it runs, and
//!   two shells never share an id, as two processes would not.
//! - Its `SHLVL` is one above its caller's.
//! - What a shell started in the background runs on when it ends, as a
//!   process's children do; the end of the job's script waits for those that
//!   hold the job's output open and reports them as it does its own
//!   (`Work::leave`), so `sh -c 'server &'` ends as `server &` does.
//! - Standard input is read to its end before the shell runs it: a script a
//!   `cat` inside reads the rest of does not get what follows.
//! - A program another program starts by its own search, such as Python's
//!   `subprocess` or `npm run`, finds the system's `sh`, as it finds its
//!   `sed`.

use std::{
    collections::{BTreeMap, HashMap},
    ffi::{OsStr, OsString},
    io::{self, Read as _, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use brush_core::{
    CommandArg, ExecutionContext, ExecutionExitCode, ExecutionResult, Shell, SourceInfo,
    builtins::{BoxFuture, Registration},
    execution_host::ChildAttributes,
    extensions::DefaultShellExtensions,
    namedoptions::{ShellOptionKind, options as named_options},
    openfiles::OpenFile,
};

use crate::scope::{Scope, UtilityControl};

/// The names `sh` and `bash` go by, as builtins and as a utility's child
/// programs.
pub(crate) const SH: &str = "sh";
pub(crate) const BASH: &str = "bash";

/// GNU bash's version, which brush implements.
const VERSION: &str = "5.2.37(1)-release";

const USAGE: &str = "\
Usage:\tbash [GNU long option] [option] ...
\tbash [GNU long option] [option] script-file ...
GNU long options:
\t--help
\t--init-file
\t--login
\t--noediting
\t--noprofile
\t--norc
\t--posix
\t--rcfile
\t--restricted
\t--verbose
\t--version
Shell options:
\t-ilrsD or -c command or -O shopt_option\t\t(invocation only)
\t-abefhkmnptuvxBCEHPT or -o option
";

/// The shell program a name stands for: `sh` or `bash`, by the file name of
/// `program`, whatever directory the path names. On Windows `.exe` is not
/// part of the name.
pub(crate) fn shell_named(program: &OsStr) -> Option<&'static str> {
    let name = Path::new(program).file_name()?.to_str()?;
    let name = if cfg!(windows) && name.len() > 4 && name[name.len() - 4..].eq_ignore_ascii_case(".exe") {
        &name[..name.len() - 4]
    } else {
        name
    };
    match name {
        SH => Some(SH),
        BASH => Some(BASH),
        _ => None,
    }
}

/// What a command line of `sh` or `bash` asks for.
pub(crate) enum Parsed {
    Run(Options),
    Help,
    Version,
    /// A usage error: its message, which the usage follows.
    Usage(String),
}

#[derive(Default)]
pub(crate) struct Options {
    /// The command string of `-c`.
    command: Option<String>,
    /// Reads commands from standard input, as `-s` asks, with the operands
    /// as positional parameters.
    stdin: bool,
    login: bool,
    posix: bool,
    noprofile: bool,
    norc: bool,
    /// The short options set, in order, each with the value it sets.
    flags: Vec<(char, bool)>,
    /// The `-o` and `+o` names.
    named: Vec<(String, bool)>,
    /// The `-O` and `+O` names.
    shopts: Vec<(String, bool)>,
    /// The script file and its arguments, or the arguments after the command
    /// string.
    operands: Vec<String>,
}

/// The `set` option name each short option stands for.
fn flag_name(flag: char) -> Option<&'static str> {
    Some(match flag {
        'a' => "allexport",
        'b' => "notify",
        'e' => "errexit",
        'f' => "noglob",
        'h' => "hashall",
        'k' => "keyword",
        'm' => "monitor",
        'n' => "noexec",
        'p' => "privileged",
        't' => "onecmd",
        'u' => "nounset",
        'v' => "verbose",
        'x' => "xtrace",
        'B' => "braceexpand",
        'C' => "noclobber",
        'E' => "errtrace",
        'H' => "histexpand",
        'P' => "physical",
        'T' => "functrace",
        _ => return None,
    })
}

/// Reads `args`, the arguments after the shell's own name, as bash reads its
/// command line: options up to the first operand, `--` or a lone `-`.
pub(crate) fn parse(args: &[String]) -> Parsed {
    let mut options = Options::default();
    let mut command = false;
    let mut index = 0;
    while let Some(word) = args.get(index) {
        index += 1;
        if word == "--" {
            break;
        }
        // A lone `-` ends the options as `--` does; it is not `-s`.
        if word == "-" {
            break;
        }
        if let Some(long) = word.strip_prefix("--") {
            match long {
                "help" => return Parsed::Help,
                "version" => return Parsed::Version,
                "login" => options.login = true,
                "posix" => options.posix = true,
                "noprofile" => options.noprofile = true,
                "norc" => options.norc = true,
                "verbose" => options.flags.push(('v', true)),
                "noediting" | "restricted" | "debugger" | "pretty-print" => {}
                "rcfile" | "init-file" => {
                    if args.get(index).is_none() {
                        return Parsed::Usage(format!("{word}: option requires an argument"));
                    }
                    index += 1;
                }
                _ => return Parsed::Usage(format!("{word}: invalid option")),
            }
            continue;
        }
        let on = match word.chars().next() {
            Some('-') if word.len() > 1 => true,
            Some('+') if word.len() > 1 => false,
            _ => {
                index -= 1;
                break;
            }
        };
        for flag in word.chars().skip(1) {
            match flag {
                'c' if on => command = true,
                's' if on => options.stdin = true,
                'l' => options.login = on,
                'i' | 'r' | 'D' => {}
                'o' | 'O' => {
                    let Some(name) = args.get(index) else {
                        return Parsed::Usage(format!("-{flag}: option requires an argument"));
                    };
                    index += 1;
                    if flag == 'o' {
                        options.named.push((name.clone(), on));
                    } else {
                        options.shopts.push((name.clone(), on));
                    }
                }
                flag if flag_name(flag).is_some() => options.flags.push((flag, on)),
                flag => {
                    return Parsed::Usage(format!("{}{flag}: invalid option", if on { '-' } else { '+' }));
                }
            }
        }
    }
    options.operands = args[index.min(args.len())..].to_vec();
    if command {
        if options.operands.is_empty() {
            return Parsed::Usage("-c: option requires an argument".into());
        }
        options.command = Some(options.operands.remove(0));
    }
    Parsed::Run(options)
}

/// Writes what a command line that runs nothing prints, and gives its
/// status: 2 for a usage error, as bash's.
pub(crate) fn answer(
    name: &str,
    parsed: &Parsed,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> u8 {
    let machine = format!("{}-unknown-{}", std::env::consts::ARCH, std::env::consts::OS);
    // The status is the command line's whether or not the stream takes what
    // it prints.
    let _unwritten = match parsed {
        Parsed::Run(_) => Ok(()),
        Parsed::Help => write!(stdout, "GNU bash, version {VERSION} ({machine})\n{USAGE}"),
        Parsed::Version => writeln!(stdout, "GNU bash, version {VERSION} ({machine})"),
        Parsed::Usage(message) => write!(stderr, "{name}: {message}\n{USAGE}"),
    };
    match parsed {
        Parsed::Usage(_) => 2,
        _ => 0,
    }
}

/// What a new shell starts from: its caller's state.
pub(crate) struct Invocation {
    /// `sh` or `bash`.
    pub name: &'static str,
    /// The caller's scope, whose part the new shell is.
    pub scope: Scope,
    pub env: BTreeMap<String, String>,
    pub cwd: PathBuf,
    pub fds: HashMap<brush_core::ShellFd, OpenFile>,
    pub builtins: HashMap<String, Registration<DefaultShellExtensions>>,
    pub attributes: ChildAttributes,
}

/// Runs the shell `options` ask for under `invocation`, and gives its exit
/// status.
pub(crate) async fn run(invocation: Invocation, options: Options) -> Result<u8, brush_core::Error> {
    let Invocation {
        name,
        scope,
        mut env,
        cwd,
        fds,
        builtins,
        attributes,
    } = invocation;
    let mut stderr = fds
        .get(&2)
        .map(OpenFile::try_clone)
        .transpose()?
        .ok_or_else(|| io::Error::other("closed descriptor 2"))?;
    let invalid = options
        .named
        .iter()
        .map(|(option, _)| (option, ShellOptionKind::SetO, "invalid option name"))
        .chain(
            options
                .shopts
                .iter()
                .map(|(option, _)| (option, ShellOptionKind::Shopt, "invalid shell option name")),
        )
        .find(|(option, kind, _)| named_options(*kind).get(option).is_none());
    if let Some((option, _, reason)) = invalid {
        let _unwritten = writeln!(stderr, "{name}: {option}: {reason}");
        return Ok(2);
    }
    let level = env
        .get("SHLVL")
        .and_then(|level| level.parse::<u32>().ok())
        .unwrap_or(0);
    env.insert("SHLVL".into(), (level + 1).to_string());

    // The shell's work is a part of the job of its own, numbered as a task is.
    let (part, _numbered) = scope.shell();
    let (script, positional, shell_name) = match (&options.command, options.stdin) {
        (Some(_), _) => {
            let mut operands = options.operands.clone();
            let shell_name = if operands.is_empty() {
                name.to_owned()
            } else {
                operands.remove(0)
            };
            (None, operands, shell_name)
        }
        (None, false) if !options.operands.is_empty() => {
            let mut operands = options.operands.clone();
            let script = operands.remove(0);
            (Some(script.clone()), operands, script)
        }
        (None, _) => (None, options.operands.clone(), name.to_owned()),
    };
    let posix = options.posix || name == SH;
    let flag = |flag: char| {
        options
            .flags
            .iter()
            .rev()
            .find(|(set, _)| *set == flag)
            .is_some_and(|(_, on)| *on)
    };
    let mut builder = Shell::builder()
        .execution_host(Arc::new(part.clone()) as Arc<dyn brush_core::execution_host::ExecutionHost>)
        .builtins(builtins)
        .working_dir(cwd.clone())
        .login(options.login)
        .posix(posix)
        .do_not_inherit_env(true)
        .fds(fds)
        .shell_name(shell_name)
        .shell_args(positional.clone())
        .profile(if options.noprofile {
            brush_core::ProfileLoadBehavior::Skip
        } else {
            brush_core::ProfileLoadBehavior::LoadDefault
        })
        .rc(if options.norc {
            brush_core::RcLoadBehavior::Skip
        } else {
            brush_core::RcLoadBehavior::LoadDefault
        })
        .exit_on_nonzero_command_exit(flag('e'))
        .treat_unset_variables_as_error(flag('u'))
        .print_commands_and_arguments(flag('x'))
        .verbose(flag('v'))
        .do_not_execute_commands(flag('n'))
        .disable_pathname_expansion(flag('f'));
    for (name, value) in env.iter().filter(|(name, _)| function_name(name).is_none()) {
        let mut variable = brush_core::ShellVariable::new(value.clone());
        variable.export();
        builder = builder.var(name, variable);
    }
    let mut shell = builder.build().await?;
    // `$BASH` is the shell's own name, which runs this shell again, where
    // brush gives the name of the script (`$0`).
    shell
        .env_mut()
        .set_global("BASH", brush_core::ShellVariable::new(name))?;
    *shell.child_attributes_mut() = attributes;
    // The functions the caller exported, which are the shell's own and go on
    // to what it starts.
    for (variable, value) in &env {
        if let Some(name) = function_name(variable)
            && shell.define_func_from_str(name, value).is_ok()
            && let Some(function) = shell.func_mut(name)
        {
            function.export();
        }
    }
    // The descriptors the caller gave are the new shell's part's from here
    // on: reading one ends when `kill $$` does.
    let mut params = shell.default_exec_params();
    shell.set_execution_host(Arc::new(part.clone()), &mut params);
    let kind_options = named_options(ShellOptionKind::SetO);
    let flags = options.flags.iter().filter_map(|&(flag, on)| {
        if "euxvnf".contains(flag) {
            return None;
        }
        flag_name(flag).map(|name| (name.to_owned(), on))
    });
    for (name, on) in flags.chain(options.named.iter().cloned()) {
        if let Some(option) = kind_options.get(&name) {
            option.set(shell.options_mut(), on);
        }
    }
    let shopt_options = named_options(ShellOptionKind::Shopt);
    for (name, on) in &options.shopts {
        if let Some(option) = shopt_options.get(name) {
            option.set(shell.options_mut(), *on);
        }
    }
    if options.login {
        crate::interpreter::restore_execution_context(&mut shell, &env)?;
        shell.set_working_dir(cwd.clone())?;
    }

    let result = if let Some(command) = options.command.clone() {
        shell.run_dash_c_incrementally(&command).await
    } else if let Some(script) = script {
        let path = shell.absolute_path(&script);
        let reason = match std::fs::metadata(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Some(("No such file or directory", 127)),
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => Some(("Permission denied", 126)),
            Ok(metadata) if metadata.is_dir() => Some(("Is a directory", 126)),
            _ => None,
        };
        if let Some((reason, status)) = reason {
            let _unwritten = writeln!(stderr, "{name}: {script}: {reason}");
            return Ok(status);
        }
        // The script goes by its name as given, which `$0` says.
        shell.run_script_incrementally(script, positional.into_iter()).await
    } else {
        let mut text = Vec::new();
        let mut input = shell
            .open_files()
            .try_stdin()
            .map(OpenFile::try_clone)
            .transpose()?
            .ok_or_else(|| io::Error::other("closed descriptor 0"))?;
        input.read_to_end(&mut text)?;
        let result = shell
            .run_string_incrementally(
                &String::from_utf8_lossy(&text),
                &SourceInfo::default(),
                &shell.default_exec_params(),
            )
            .await;
        let _ended = shell.on_exit().await;
        result
    };
    // What the shell started in the background runs on when it ends, as a
    // process's children do; the end of the job's script waits for those that
    // hold the job's output open, as for its own.
    scope.work.leave(std::mem::take(&mut shell.jobs_mut().jobs));
    // The caller's own end, a job's cancellation or a signal to the task it
    // runs in, is the caller's to report; a signal to this shell's `$$` ends
    // it as a signal ends a process.
    if let Err(error) = scope.check() {
        return match scope.stop.signal() {
            Some(signal) => Ok(128 + signal as u8),
            None => Err(io::Error::other(error).into()),
        };
    }
    if part.check().is_err() {
        return Ok(128 + part.stop.signal().unwrap_or(9) as u8);
    }
    result.map(|result| result.exit_code.into())
}

/// The exported variables of `shell`, as a program it starts gets them, and
/// its exported functions as bash passes them, `BASH_FUNC_name%%` holding `()`
/// and the body.
pub(crate) fn exported(shell: &Shell) -> BTreeMap<String, String> {
    let functions = shell
        .funcs()
        .iter()
        .filter(|(_, function)| function.is_exported())
        .map(|(name, function)| {
            (
                format!("{FUNCTION_PREFIX}{name}{FUNCTION_SUFFIX}"),
                format!("() {}", function.definition().body),
            )
        });
    shell
        .env()
        .iter_exported()
        .filter(|(_, variable)| variable.value().is_set())
        .map(|(name, variable)| (name.clone(), variable.value().to_cow_str(shell).into_owned()))
        .chain(functions)
        .collect()
}

/// The name of the variable an exported function travels in is
/// `BASH_FUNC_<name>%%`.
const FUNCTION_PREFIX: &str = "BASH_FUNC_";
const FUNCTION_SUFFIX: &str = "%%";

/// The function `name` stands for when `variable` is the one it travels in.
fn function_name(variable: &str) -> Option<&str> {
    variable
        .strip_prefix(FUNCTION_PREFIX)?
        .strip_suffix(FUNCTION_SUFFIX)
}

/// `sh` and `bash` as builtins of the job's shell.
pub(crate) fn register(registrations: &mut HashMap<String, Registration<DefaultShellExtensions>>) {
    for name in [SH, BASH] {
        registrations.insert(
            name.into(),
            Registration {
                execute_func: shell,
                content_func: |name, _, _| Ok(format!("{name}: a new shell of the job's own, as bash's")),
                disabled: false,
                special_builtin: false,
                declaration_builtin: false,
            },
        );
    }
}

fn shell(
    context: ExecutionContext<'_>,
    args: Vec<CommandArg>,
) -> BoxFuture<'_, Result<ExecutionResult, brush_core::Error>> {
    Box::pin(async move {
        let name = shell_named(OsStr::new(&context.command_name)).unwrap_or(BASH);
        let words: Vec<String> = args.iter().skip(1).map(ToString::to_string).collect();
        let options = match parse(&words) {
            Parsed::Run(options) => options,
            parsed => {
                let status = answer(name, &parsed, &mut context.stdout(), &mut context.stderr());
                return Ok(ExecutionExitCode::from(status).into());
            }
        };
        let scope = crate::interpreter::scope(context.shell)?;
        let invocation = Invocation {
            name,
            scope: scope.clone(),
            env: exported(context.shell),
            cwd: context.shell.working_dir().to_owned(),
            fds: context.iter_fds().collect(),
            builtins: context.shell.builtins().clone(),
            attributes: context.shell.child_attributes().clone(),
        };
        // On a thread of its own, as a child program's shell is: a shell run
        // inline would add its frames to this one's stack, and a script that
        // starts itself would overflow the runner's stack and end the runner,
        // where it ends at the system's limit on threads.
        let runtime = tokio::runtime::Handle::current();
        let status = tokio::task::spawn_blocking(move || runtime.block_on(run(invocation, options)))
            .await
            .map_err(io::Error::other)??;
        Ok(ExecutionExitCode::from(status).into())
    })
}

/// Starts `sh` or `bash` as a child program a utility started, such as
/// `find -exec bash -c …` (`crates/runner-shell/src/utilities.rs`): a new
/// shell as the builtin starts one, with the utility's own directory,
/// environment and descriptors.
pub(crate) fn start(
    control: &UtilityControl,
    context: uucore::context::Context,
    args: Vec<OsString>,
) -> io::Result<Box<dyn uucore::context::UtilityRun>> {
    let cancellation = control.scope.cancellation.child_token();
    let scope = control.scope.with_cancellation(cancellation.clone());
    let attributes = control.attributes.clone();
    let tasks = control.scope.tasks.clone();
    let stderr = context.stderr.clone();
    let program = args.first().cloned().unwrap_or_default();
    let name = shell_named(&program).unwrap_or(BASH);
    let words: Vec<String> = args
        .iter()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let work = move || {
        let options = match parse(&words) {
            Parsed::Run(options) => options,
            parsed => {
                return Ok(i32::from(answer(
                    name,
                    &parsed,
                    &mut &*context.stdout,
                    &mut &*context.stderr,
                )));
            }
        };
        let controlled = |file: &Arc<std::fs::File>| -> Result<OpenFile, String> {
            Ok(OpenFile::Controlled {
                file: scope.duplicate(file).map_err(|error| error.to_string())?,
                control: Arc::new(scope.clone()),
            })
        };
        let mut fds = HashMap::from([
            (0, controlled(&context.stdin)?),
            (1, controlled(&context.stdout)?),
            (2, controlled(&context.stderr)?),
        ]);
        for (&fd, file) in &context.descriptors {
            fds.insert(fd, controlled(file)?);
        }
        let invocation = Invocation {
            name,
            builtins: crate::interpreter::job_builtins(&scope),
            scope: scope.clone(),
            env: context.env.clone(),
            cwd: context.cwd.clone(),
            fds,
            attributes: brush_core::execution_host::ChildAttributes {
                umask: attributes.umask,
                #[cfg(unix)]
                limits: attributes.limits.clone(),
                hangup_ignored: attributes.hangup_ignored,
            },
        };
        let runtime = tokio::runtime::Handle::current();
        runtime
            .block_on(run(invocation, options))
            .map(i32::from)
            .map_err(|error| error.to_string())
    };
    crate::utilities::start_with(&tasks, cancellation, name, stderr, work)
}
