//! The builtins that would act on the runner's own process, which every job
//! shares: in a job each acts for the job's shell instead, or refuses
//! (`runner.md` § Builtins that act on a process).

use std::{collections::HashMap, io::Write};

use brush_core::{
    CommandArg, ExecutionContext, ExecutionControlFlow, ExecutionExitCode, ExecutionResult,
    ShellExtensions,
    builtins::{self, BoxFuture, Registration},
    commands::{ShellForCommand, SimpleCommand},
    extensions::DefaultShellExtensions,
};

/// Puts the runner's builtins in place of brush's of the same name. Help
/// text and a builtin's special status stay brush's.
pub(crate) fn register(registrations: &mut HashMap<String, Registration<DefaultShellExtensions>>) {
    registrations.insert("exec".into(), builtins::builtin::<Exec, _>().special());
    replace(registrations, "fg", fg);
    #[cfg(unix)]
    {
        if let Some(brush_kill) = registrations.get("kill").map(|kill| kill.execute_func) {
            // A static holds brush's own kill, which a registration's plain
            // function pointer cannot capture. Setting it again fails, which
            // is fine: brush's kill is the same function every time.
            let _already_set = BRUSH_KILL.set(brush_kill);
            replace(registrations, "kill", kill);
        }
        replace(registrations, "suspend", suspend);
        replace(registrations, "ulimit", unix::ulimit);
        replace(registrations, "umask", unix::umask);
    }
}

/// Runs `name` with `execute`, keeping the rest of brush's registration.
fn replace(
    registrations: &mut HashMap<String, Registration<DefaultShellExtensions>>,
    name: &str,
    execute: builtins::CommandExecuteFunc<DefaultShellExtensions>,
) {
    if let Some(registration) = registrations.get_mut(name) {
        registration.execute_func = execute;
    }
}

/// A builtin that refuses: why on standard error, and a general failure.
fn refused(context: &ExecutionContext<'_>, why: &str) -> Result<ExecutionResult, brush_core::Error> {
    writeln!(context.stderr(), "{}: {why}", context.command_name)?;
    Ok(ExecutionResult::general_error())
}

/// `exec`: with a command, the command runs as the shell's last, and the
/// shell (a subshell's, or the job's) then ends with its status; bash would
/// replace the process, which here is the runner. With only redirections,
/// they stay with the shell, as in bash.
#[derive(clap::Parser)]
struct Exec {
    /// Pass NAME to the command as its zeroth argument.
    #[arg(short = 'a', value_name = "NAME")]
    name: Option<String>,
    /// Run the command with an empty environment.
    #[arg(short = 'c')]
    empty_environment: bool,
    /// Run the command as a login shell, with a dash before its zeroth argument.
    #[arg(short = 'l')]
    login: bool,
    /// The command and its arguments.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    command: Vec<String>,
}

impl builtins::Command for Exec {
    type Error = brush_core::Error;

    async fn execute<SE: ShellExtensions>(
        &self,
        context: ExecutionContext<'_, SE>,
    ) -> Result<ExecutionResult, Self::Error> {
        let Some(program) = self.command.first() else {
            let files: Vec<_> = context.iter_fds().collect();
            context.shell.replace_open_files(files.into_iter());
            return Ok(ExecutionResult::success());
        };
        let mut stderr = context.stderr();
        let shell = if self.empty_environment {
            let mut target = context.shell.clone();
            let exported: Vec<String> = target
                .env()
                .iter_exported()
                .map(|(name, _)| name.clone())
                .collect();
            for name in exported {
                if let Some((_, variable)) = target.env_mut().get_mut(&name) {
                    variable.unexport();
                }
            }
            ShellForCommand::OwnedShell {
                target: Box::new(target),
                parent: context.shell,
            }
        } else {
            ShellForCommand::ParentShell(context.shell)
        };
        let args = self
            .command
            .iter()
            .map(|arg| CommandArg::String(arg.clone()))
            .collect();
        // As `command` runs it: a standard utility or builtin in the runner,
        // a program through the job's process start.
        let mut command = SimpleCommand::new(shell, context.params, program.clone(), args);
        command.use_functions = false;
        command.argv0 = self
            .name
            .clone()
            .or_else(|| self.login.then(|| format!("-{program}")));
        let mut result = match command.execute().await {
            Ok(started) => started.wait().await?.into(),
            Err(error) => {
                match error.kind() {
                    brush_core::ErrorKind::CommandNotFound(_) => {
                        writeln!(stderr, "exec: {program}: not found")?;
                    }
                    _ => writeln!(stderr, "exec: {program}: {error}")?,
                }
                ExecutionResult::from(ExecutionExitCode::from(error.kind()))
            }
        };
        result.next_control_flow = ExecutionControlFlow::ExitShell;
        Ok(result)
    }
}

/// `fg` would give the runner's terminal to a job; a job's shell has no job
/// control, as a bash script has none.
fn fg(
    context: ExecutionContext<'_>,
    _args: Vec<CommandArg>,
) -> BoxFuture<'_, Result<ExecutionResult, brush_core::Error>> {
    Box::pin(async move { refused(&context, "no job control") })
}

/// Brush's `kill`, which the runner's runs for every other target.
#[cfg(unix)]
static BRUSH_KILL: std::sync::OnceLock<builtins::CommandExecuteFunc<DefaultShellExtensions>> =
    std::sync::OnceLock::new();

/// `kill`, except to the runner itself: `$$` is the runner's process, and 0
/// its process group.
#[cfg(unix)]
fn kill(
    context: ExecutionContext<'_>,
    args: Vec<CommandArg>,
) -> BoxFuture<'_, Result<ExecutionResult, brush_core::Error>> {
    let runner = kill_target(&args).is_some_and(|target| {
        brush_core::int_utils::parse::<i32>(&target, 10)
            .is_ok_and(|pid| pid == 0 || u32::try_from(pid).is_ok_and(|pid| pid == std::process::id()))
    });
    if runner {
        return Box::pin(async move { refused(&context, "a job cannot signal the runner it runs in") });
    }
    let brush_kill = BRUSH_KILL
        .get()
        .expect("the runner's kill is registered only over brush's");
    brush_kill(context, args)
}

/// The process or job `kill` signals, read as brush's `kill` reads its
/// arguments: the first that is not an option, unless it lists signals.
#[cfg(unix)]
fn kill_target(args: &[CommandArg]) -> Option<String> {
    let mut args = args.iter().skip(1).map(ToString::to_string);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-s" | "-n" => {
                args.next();
            }
            "-l" | "-L" => return None,
            _ if arg.starts_with('-') => {}
            _ => return Some(arg),
        }
    }
    None
}

/// `suspend` would stop the runner.
#[cfg(unix)]
fn suspend(
    context: ExecutionContext<'_>,
    _args: Vec<CommandArg>,
) -> BoxFuture<'_, Result<ExecutionResult, brush_core::Error>> {
    Box::pin(async move { refused(&context, "a job cannot suspend the runner it runs in") })
}

#[cfg(unix)]
mod unix {
    use std::io::Write;

    use brush_core::{
        CommandArg, ExecutionContext, ExecutionExitCode, ExecutionResult, builtins::BoxFuture,
        execution_host::ChildAttributes,
    };
    use rlimit::Resource;

    /// How `ulimit` counts a resource's limit, and what bash calls that.
    #[derive(Clone, Copy)]
    enum Unit {
        Blocks,
        Bytes,
        HalfKbytes,
        Kbytes,
        Microseconds,
        Number,
        Seconds,
    }

    impl Unit {
        const fn scale(self) -> u64 {
            match self {
                Self::Blocks | Self::HalfKbytes => 512,
                Self::Kbytes => 1024,
                _ => 1,
            }
        }

        const fn label(self) -> &'static str {
            match self {
                Self::Blocks => "blocks, ",
                Self::Bytes => "bytes, ",
                Self::HalfKbytes => "512 bytes, ",
                Self::Kbytes => "kbytes, ",
                Self::Microseconds => "microseconds, ",
                Self::Number => "",
                Self::Seconds => "seconds, ",
            }
        }
    }

    /// The resources `ulimit` knows, by option, as bash names and counts
    /// them and in the order `-a` shows them. The pipe size is no resource
    /// limit: `-p` only shows it.
    const RESOURCES: &[(char, Option<Resource>, &str, Unit)] = &[
        ('b', Some(Resource::SBSIZE), "socket buffer size", Unit::Bytes),
        ('c', Some(Resource::CORE), "core file size", Unit::Blocks),
        ('d', Some(Resource::DATA), "data seg size", Unit::Kbytes),
        ('e', Some(Resource::NICE), "scheduling priority", Unit::Number),
        ('f', Some(Resource::FSIZE), "file size", Unit::Blocks),
        ('i', Some(Resource::SIGPENDING), "pending signals", Unit::Number),
        ('k', Some(Resource::KQUEUES), "max kqueues", Unit::Number),
        ('l', Some(Resource::MEMLOCK), "max locked memory", Unit::Kbytes),
        ('m', Some(Resource::RSS), "max memory size", Unit::Kbytes),
        ('n', Some(Resource::NOFILE), "open files", Unit::Number),
        ('p', None, "pipe size", Unit::HalfKbytes),
        ('q', Some(Resource::MSGQUEUE), "POSIX message queues", Unit::Bytes),
        ('r', Some(Resource::RTPRIO), "real-time priority", Unit::Number),
        ('R', Some(Resource::RTTIME), "real-time non-blocking time", Unit::Microseconds),
        ('s', Some(Resource::STACK), "stack size", Unit::Kbytes),
        ('t', Some(Resource::CPU), "cpu time", Unit::Seconds),
        ('u', Some(Resource::NPROC), "max user processes", Unit::Number),
        ('v', Some(Resource::AS), "virtual memory", Unit::Kbytes),
        ('x', Some(Resource::LOCKS), "file locks", Unit::Number),
        ('P', Some(Resource::NPTS), "number of pseudoterminals", Unit::Number),
        ('T', Some(Resource::THREADS), "number of threads", Unit::Number),
    ];

    /// What a `ulimit` command line asks for.
    struct Asked {
        soft: bool,
        hard: bool,
        all: bool,
        /// Each resource's option, with the value to set it to, if any.
        resources: Vec<(char, Option<String>)>,
    }

    /// Reads a `ulimit` command line as bash does: options may come
    /// together, a value follows the option it sets, and a value alone sets
    /// the file size. An error is its message.
    fn parse(args: &[CommandArg]) -> Result<Asked, String> {
        let mut asked = Asked {
            soft: false,
            hard: false,
            all: false,
            resources: Vec::new(),
        };
        for arg in args.iter().skip(1).map(ToString::to_string) {
            let Some(options) = arg.strip_prefix('-').filter(|options| !options.is_empty()) else {
                match asked.resources.last_mut() {
                    Some((_, value @ None)) => *value = Some(arg),
                    Some(_) => return Err(format!("{arg}: too many arguments")),
                    None => asked.resources.push(('f', Some(arg))),
                }
                continue;
            };
            for option in options.chars() {
                match option {
                    'S' => asked.soft = true,
                    'H' => asked.hard = true,
                    'a' => asked.all = true,
                    option if RESOURCES.iter().any(|&(known, ..)| known == option) => {
                        asked.resources.push((option, None));
                    }
                    option => return Err(format!("-{option}: invalid option")),
                }
            }
        }
        if asked.all {
            asked.resources = RESOURCES.iter().map(|&(option, ..)| (option, None)).collect();
        } else if asked.resources.is_empty() {
            asked.resources.push(('f', None));
        }
        Ok(asked)
    }

    /// The limits of `resource` that a process started with `attributes`
    /// gets.
    fn limits(attributes: &ChildAttributes, resource: Resource) -> std::io::Result<(u64, u64)> {
        match attributes.limits.iter().find(|&&(set, ..)| set == resource) {
            Some(&(_, soft, hard)) => Ok((soft, hard)),
            None => crate::process::child_limit(resource),
        }
    }

    /// A limit as `ulimit` shows it, counted in `unit`.
    fn shown(limit: u64, unit: Unit) -> String {
        if limit == rlimit::INFINITY {
            "unlimited".to_owned()
        } else {
            (limit / unit.scale()).to_string()
        }
    }

    /// `ulimit`, for the processes the job's shell starts from then on
    /// (`runner.md` § Builtins that act on a process). The runner's own
    /// limits, which its builtins and standard utilities share, never change.
    pub(super) fn ulimit(
        context: ExecutionContext<'_>,
        args: Vec<CommandArg>,
    ) -> BoxFuture<'_, Result<ExecutionResult, brush_core::Error>> {
        Box::pin(async move {
            let mut stderr = context.stderr();
            let asked = match parse(&args) {
                Ok(asked) => asked,
                Err(message) => {
                    writeln!(stderr, "ulimit: {message}")?;
                    return Ok(ExecutionExitCode::InvalidUsage.into());
                }
            };
            let mut attributes = context.shell.child_attributes().clone();
            // Each limit to show: its description, unit, option and value.
            let mut showing = Vec::new();
            for (option, value) in asked.resources {
                let &(_, resource, description, unit) = RESOURCES
                    .iter()
                    .find(|&&(known, ..)| known == option)
                    .expect("the parse keeps only known options");
                let Some(resource) = resource else {
                    if value.is_some() {
                        writeln!(stderr, "ulimit: {description}: cannot modify limit: Invalid argument")?;
                        return Ok(ExecutionResult::general_error());
                    }
                    let pipe_size = u64::try_from(libc::PIPE_BUF).expect("a pipe buffer's size fits");
                    showing.push((description, unit, option, shown(pipe_size, unit)));
                    continue;
                };
                if !resource.is_supported() {
                    if asked.all {
                        continue;
                    }
                    writeln!(stderr, "ulimit: -{option}: not supported here")?;
                    return Ok(ExecutionResult::general_error());
                }
                let (soft, hard) = limits(&attributes, resource)?;
                let Some(value) = value else {
                    let limit = if asked.hard { hard } else { soft };
                    showing.push((description, unit, option, shown(limit, unit)));
                    continue;
                };
                let limit = match value.as_str() {
                    "unlimited" => rlimit::INFINITY,
                    "hard" => hard,
                    "soft" => soft,
                    number => match number.parse::<u64>().ok().and_then(|n| n.checked_mul(unit.scale())) {
                        Some(limit) => limit,
                        None => {
                            writeln!(stderr, "ulimit: {number}: invalid number")?;
                            return Ok(ExecutionResult::general_error());
                        }
                    },
                };
                // Without -S or -H, both limits change, as in bash.
                let both = asked.soft == asked.hard;
                let soft = if asked.soft || both { limit } else { soft };
                let hard = if asked.hard || both { limit } else { hard };
                attributes.limits.retain(|&(set, ..)| set != resource);
                attributes.limits.push((resource, soft, hard));
                if let Err(error) = allowed(&context, &attributes).await {
                    writeln!(stderr, "ulimit: {description}: cannot modify limit: {error}")?;
                    return Ok(ExecutionResult::general_error());
                }
            }
            *context.shell.child_attributes_mut() = attributes;
            let mut stdout = context.stdout();
            if let [(_, _, _, limit)] = showing.as_slice() {
                writeln!(stdout, "{limit}")?;
            } else {
                for (description, unit, option, limit) in showing {
                    let label = format!("({}-{option})", unit.label());
                    writeln!(stdout, "{description:<27} {label:>18} {limit}")?;
                }
            }
            Ok(ExecutionResult::success())
        })
    }

    /// Whether the system lets a process have `attributes`: raising a hard
    /// limit takes privilege, and a system caps some limits, as macOS caps
    /// open files. A job's `ulimit` asks it the way its commands will meet
    /// it: it starts `/bin/sh -c :` with them, which exits at once, and the
    /// system refuses the start if it refuses a limit.
    async fn allowed(context: &ExecutionContext<'_>, attributes: &ChildAttributes) -> std::io::Result<()> {
        let scope = super::super::scope(&*context.shell)?;
        let mut command = tokio::process::Command::new("/bin/sh");
        command
            .args(["-c", ":"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let mut command = crate::process::wrap(command, false, attributes);
        let mut child = crate::process::start(|| {
            scope.check()?;
            command.spawn()
        })
        .await?;
        child.wait().await?;
        Ok(())
    }

    /// `umask`, for the processes the job's shell starts from then on and
    /// the files its redirections and standard utilities create
    /// (`runner.md` § Builtins that act on a process). The runner's own
    /// umask never changes.
    pub(super) fn umask(
        context: ExecutionContext<'_>,
        args: Vec<CommandArg>,
    ) -> BoxFuture<'_, Result<ExecutionResult, brush_core::Error>> {
        Box::pin(async move {
            let mut stderr = context.stderr();
            let mut reusable = false;
            let mut symbolic = false;
            let mut mode = None;
            for arg in args.iter().skip(1).map(ToString::to_string) {
                match arg.strip_prefix('-').filter(|options| !options.is_empty() && mode.is_none()) {
                    Some(options) if options.chars().all(|option| matches!(option, 'p' | 'S')) => {
                        reusable |= options.contains('p');
                        symbolic |= options.contains('S');
                    }
                    _ if mode.is_none() => mode = Some(arg),
                    _ => {
                        writeln!(stderr, "umask: {arg}: too many arguments")?;
                        return Ok(ExecutionExitCode::InvalidUsage.into());
                    }
                }
            }
            let current = context
                .shell
                .child_attributes()
                .umask
                .unwrap_or_else(crate::process::umask);
            if let Some(mode) = mode {
                let umask = if mode.starts_with(|digit: char| digit.is_ascii_digit()) {
                    match u32::from_str_radix(&mode, 8) {
                        Ok(umask) if umask <= 0o777 => umask,
                        _ => {
                            writeln!(stderr, "umask: {mode}: octal number out of range")?;
                            return Ok(ExecutionResult::general_error());
                        }
                    }
                } else {
                    // A symbolic mode says which permissions new files may
                    // have, so it changes the complement of the mask.
                    match uucore::mode::parse_chmod(!current & 0o777, &mode, false, 0) {
                        Ok(allowed) => !allowed & 0o777,
                        Err(error) => {
                            writeln!(stderr, "umask: {mode}: {error}")?;
                            return Ok(ExecutionResult::general_error());
                        }
                    }
                };
                context.shell.child_attributes_mut().umask = Some(umask);
                if !symbolic {
                    return Ok(ExecutionResult::success());
                }
            }
            let umask = context.shell.child_attributes().umask.unwrap_or(current);
            let shown = if symbolic {
                let who = |shift: u32| {
                    let allowed = (!umask >> shift) & 0o7;
                    [(0o4, 'r'), (0o2, 'w'), (0o1, 'x')]
                        .iter()
                        .filter(|&&(bit, _)| allowed & bit != 0)
                        .map(|&(_, letter)| letter)
                        .collect::<String>()
                };
                format!("u={},g={},o={}", who(6), who(3), who(0))
            } else {
                format!("{umask:04o}")
            };
            let prefix = match (reusable, symbolic) {
                (true, true) => "umask -S ",
                (true, false) => "umask ",
                _ => "",
            };
            writeln!(context.stdout(), "{prefix}{shown}")?;
            Ok(ExecutionResult::success())
        })
    }
}
