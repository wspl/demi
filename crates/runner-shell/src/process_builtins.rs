//! The builtins that would act on the runner's own process, which every job
//! shares: in a job each acts for the job's shell instead, or refuses
//! (`runner.md` § Builtins that act on a process).

use std::{
    collections::{BTreeMap, HashMap},
    io::Write,
};

use futures_util::{FutureExt as _, StreamExt as _, stream::FuturesUnordered};

use brush_core::{
    CommandArg, ExecutionContext, ExecutionControlFlow, ExecutionExitCode, ExecutionResult,
    Shell, ShellExtensions,
    builtins::{self, BoxFuture, Registration},
    commands::{ShellForCommand, SimpleCommand},
    extensions::DefaultShellExtensions,
};

/// Puts the runner's builtins in place of brush's of the same name. Help
/// text and a builtin's special status stay brush's.
pub(crate) fn register(registrations: &mut HashMap<String, Registration<DefaultShellExtensions>>) {
    registrations.insert("exec".into(), builtins::builtin::<Exec, _>().special());
    registrations.insert(
        "disown".into(),
        Registration {
            execute_func: disown,
            content_func: |name, _, _| {
                Ok(format!("{name}: keeps each task in its job, which runs on while the task does"))
            },
            disabled: false,
            special_builtin: false,
            declaration_builtin: false,
        },
    );
    replace(registrations, "fg", fg);
    // Statics hold brush's own builtins, which the runner's run for what they
    // leave to them and which a registration's plain function pointer cannot
    // capture. Setting one again fails, which is fine: brush's is the same
    // function every time.
    if let Some(brush_wait) = registrations.get("wait").map(|wait| wait.execute_func) {
        let _already_set = BRUSH_WAIT.set(brush_wait);
        replace(registrations, "wait", wait);
    }
    #[cfg(unix)]
    {
        if let Some(brush_kill) = registrations.get("kill").map(|kill| kill.execute_func) {
            let _already_set = BRUSH_KILL.set(brush_kill);
            replace(registrations, "kill", unix::kill);
        }
        registrations.insert(
            crate::timeout::NAME.into(),
            Registration {
                execute_func: unix::timeout,
                content_func: |name, _, _| Ok(format!("{name}: use {name} --help for its options")),
                disabled: false,
                special_builtin: false,
                declaration_builtin: false,
            },
        );
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
fn refused(
    context: &ExecutionContext<'_>,
    why: &str,
) -> Result<ExecutionResult, brush_core::Error> {
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

/// `disown` would let a task outlive its job; here it keeps the task in its
/// job, which runs on while the task does and whose stop stops it, so it
/// succeeds and changes nothing else (`runner.md` § Background tasks and
/// timeouts).
fn disown(
    _context: ExecutionContext<'_>,
    _args: Vec<CommandArg>,
) -> BoxFuture<'_, Result<ExecutionResult, brush_core::Error>> {
    Box::pin(async { Ok(ExecutionResult::success()) })
}

/// Brush's `kill`, which lists signals for the runner's.
#[cfg(unix)]
static BRUSH_KILL: std::sync::OnceLock<builtins::CommandExecuteFunc<DefaultShellExtensions>> =
    std::sync::OnceLock::new();

/// Brush's `wait`, which takes the options of the runner's.
static BRUSH_WAIT: std::sync::OnceLock<builtins::CommandExecuteFunc<DefaultShellExtensions>> =
    std::sync::OnceLock::new();

/// `wait`, as bash's (`runner.md` § Background tasks and timeouts): `wait
/// <id>` waits for a background task or a process the job started and gives
/// its status, the task's last or 128 plus the signal that ended it; `wait`
/// alone waits for every task and gives 0. Its options are brush's.
fn wait(
    context: ExecutionContext<'_>,
    args: Vec<CommandArg>,
) -> BoxFuture<'_, Result<ExecutionResult, brush_core::Error>> {
    let mut words: Vec<String> = args.iter().skip(1).map(ToString::to_string).collect();
    let option = words
        .first()
        .is_some_and(|word| word.starts_with('-') && word != "--" && word.parse::<i32>().is_err());
    if option {
        let brush_wait = BRUSH_WAIT
            .get()
            .expect("the runner's wait is registered only over brush's");
        return brush_wait(context, args);
    }
    if words.first().is_some_and(|word| word == "--") {
        words.remove(0);
    }
    Box::pin(async move {
        if words.is_empty() {
            wait_tasks(context.shell, |_| {}).await?;
            return Ok(ExecutionResult::success());
        }
        let scope = crate::interpreter::scope(context.shell)?;
        let mut status = 0;
        for word in &words {
            let job = if word.starts_with('%') {
                let job = context
                    .shell
                    .jobs_mut()
                    .resolve_job_spec(word)
                    .map(|job| job.id);
                if job.is_none() {
                    writeln!(context.stderr(), "wait: {word}: no such job")?;
                    status = 127;
                    continue;
                }
                job
            } else if let Ok(id) = word.parse::<i32>() {
                let job = context
                    .shell
                    .jobs()
                    .jobs
                    .iter()
                    .find(|job| job.task_id() == Some(id))
                    .map(|job| job.id);
                if job.is_none() {
                    status = match scope.work.process(id) {
                        Some(crate::work::Process::Ended(ended)) => ended,
                        Some(crate::work::Process::Running(mut ended)) => {
                            match ended.wait_for(Option::is_some).await {
                                Ok(ended) => ended.unwrap_or(127),
                                // The process went without a status.
                                Err(_) => 127,
                            }
                        }
                        None => {
                            writeln!(context.stderr(), "wait: pid {id} is not a child of this shell")?;
                            127
                        }
                    };
                    continue;
                }
                job
            } else {
                writeln!(context.stderr(), "wait: `{word}': not a pid or valid job spec")?;
                status = 1;
                continue;
            };
            let jobs = &mut context.shell.jobs_mut().jobs;
            let at = jobs
                .iter()
                .position(|listed| Some(listed.id) == job)
                .expect("the job was just found");
            let job = jobs.remove(at);
            status = task_status(&scope, job).await?;
        }
        Ok(ExecutionExitCode::from(status).into())
    })
}

/// Waits for the background task of `job` to end and gives its status: 128
/// plus the signal that stopped it, as a subshell such a signal ends gives,
/// or else its last command's.
async fn task_status(
    scope: &crate::scope::Scope,
    mut job: brush_core::jobs::Job,
) -> Result<u8, brush_core::Error> {
    let ended = job.wait().await;
    let signal = job
        .task_id()
        .and_then(|id| scope.work.task(id))
        .and_then(|task| task.signal());
    match (signal, ended) {
        (Some(signal), _) => Ok((128 + signal) as u8),
        (None, ended) => Ok(ended?.exit_code.into()),
    }
}

/// Waits for every background task of `shell` to end, as `wait` alone and
/// the end of a job do. A task a signal stopped ended as asked. `running`
/// hears the command lines of the tasks that still run: once those that had
/// ended are waited for, and again after each end.
pub(crate) async fn wait_tasks(
    shell: &mut Shell,
    mut running: impl FnMut(Vec<String>),
) -> Result<(), brush_core::Error> {
    let scope = crate::interpreter::scope(shell)?;
    let jobs = std::mem::take(&mut shell.jobs_mut().jobs);
    let mut lines: BTreeMap<usize, String> = jobs
        .iter()
        .map(|job| (job.id, job.command_line.clone()))
        .collect();
    let mut waits: FuturesUnordered<_> = jobs
        .into_iter()
        .map(|job| {
            let scope = &scope;
            async move { (job.id, task_status(scope, job).await) }
        })
        .collect();
    loop {
        while let Some(Some((id, status))) = waits.next().now_or_never() {
            status?;
            lines.remove(&id);
        }
        if waits.is_empty() {
            return Ok(());
        }
        running(lines.values().cloned().collect());
        if let Some((id, status)) = waits.next().await {
            status?;
            lines.remove(&id);
        }
    }
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
    use std::{io::Write, sync::Arc};

    use brush_core::{
        CommandArg, ErrorKind, ExecutionContext, ExecutionExitCode, ExecutionResult,
        builtins::BoxFuture,
        commands::{ShellForCommand, SimpleCommand},
        execution_host::ChildAttributes,
        traps::TrapSignal,
    };
    use rlimit::Resource;

    use crate::timeout;

    /// Bash's usage of `kill`.
    const KILL_USAGE: &str = "kill: usage: kill [-s sigspec | -n signum | -sigspec] pid | jobspec ... or kill -l [sigspec]";

    /// A signal's number by the name or number a command line gives it, as
    /// bash reads it: any case, with or without `SIG`; 0 asks only whether
    /// the target exists.
    fn signal_number(spec: &str) -> Option<i32> {
        spec.parse::<TrapSignal>()
            .ok()
            .and_then(|signal| i32::try_from(signal).ok())
    }

    /// `kill`, as bash's (`runner.md` § Background tasks and timeouts): the
    /// default signal is `TERM`; a background task's id, or a job spec,
    /// signals the whole task; a process ID or the negative of a process
    /// group ID signals that process or group; the runner itself is refused
    /// (§ Builtins that act on a process). As in a bash script, the first
    /// target that fails ends the command with status 1. Listing signals is
    /// brush's.
    pub(super) fn kill(
        context: ExecutionContext<'_>,
        args: Vec<CommandArg>,
    ) -> BoxFuture<'_, Result<ExecutionResult, brush_core::Error>> {
        let words: Vec<String> = args.iter().skip(1).map(ToString::to_string).collect();
        if words.iter().take_while(|word| *word != "--").any(|word| word == "-l" || word == "-L") {
            let brush_kill = super::BRUSH_KILL
                .get()
                .expect("the runner's kill is registered only over brush's");
            return brush_kill(context, args);
        }
        Box::pin(async move {
            let mut stderr = context.stderr();
            let mut spec = "TERM".to_owned();
            let mut signal = Some(libc::SIGTERM);
            let mut saw_signal = false;
            let mut index = 0;
            while let Some(word) = words.get(index) {
                let given = match word.as_str() {
                    "-s" | "-n" => {
                        let Some(value) = words.get(index + 1) else {
                            writeln!(stderr, "kill: {word}: option requires an argument")?;
                            return Ok(ExecutionResult::general_error());
                        };
                        index += 1;
                        value.clone()
                    }
                    "--" => {
                        index += 1;
                        break;
                    }
                    "-?" => {
                        writeln!(stderr, "{KILL_USAGE}")?;
                        return Ok(ExecutionExitCode::InvalidUsage.into());
                    }
                    word if word.len() > 2
                        && ((word.starts_with("-s") && word.as_bytes()[2].is_ascii_alphabetic())
                            || (word.starts_with("-n") && word.as_bytes()[2].is_ascii_digit())) =>
                    {
                        word[2..].to_owned()
                    }
                    word if word.len() > 1 && word.starts_with('-') && !saw_signal => {
                        word[1..].to_owned()
                    }
                    _ => break,
                };
                signal = signal_number(&given);
                spec = given;
                saw_signal = true;
                index += 1;
            }
            let Some(signal) = signal else {
                writeln!(stderr, "kill: {spec}: invalid signal specification")?;
                return Ok(ExecutionResult::general_error());
            };
            let targets = &words[index.min(words.len())..];
            if targets.is_empty() {
                writeln!(stderr, "{KILL_USAGE}")?;
                return Ok(ExecutionExitCode::InvalidUsage.into());
            }
            let scope = crate::interpreter::scope(context.shell)?;
            for target in targets {
                let task = if target.starts_with('%') {
                    let task = context
                        .shell
                        .jobs_mut()
                        .resolve_job_spec(target)
                        .and_then(|job| job.task_id());
                    let Some(id) = task else {
                        writeln!(stderr, "kill: {target}: no such job")?;
                        return Ok(ExecutionResult::general_error());
                    };
                    Some(id)
                } else {
                    let Ok(pid) = target.parse::<i32>() else {
                        writeln!(stderr, "kill: `{target}': not a pid or valid job spec")?;
                        return Ok(ExecutionResult::general_error());
                    };
                    if runner(pid) {
                        return super::refused(&context, "a job cannot signal the runner it runs in");
                    }
                    // A task's id stands for the task with a minus sign too,
                    // as a process group's does.
                    if scope.work.task(pid.saturating_abs()).is_some() {
                        Some(pid.saturating_abs())
                    } else {
                        let sent = demi_runner_process::process::signal_target(pid, signal);
                        if let Err(error) = sent {
                            if error.raw_os_error() == Some(libc::EINVAL) {
                                writeln!(stderr, "kill: {spec}: invalid signal specification")?;
                            } else {
                                let reason = uucore::error::strip_errno(&error);
                                writeln!(stderr, "kill: ({pid}) - {reason}")?;
                            }
                            return Ok(ExecutionResult::general_error());
                        }
                        None
                    }
                };
                let Some(id) = task else {
                    continue;
                };
                let task = scope
                    .work
                    .task(id)
                    .expect("a job's task stays in its table");
                if task.done.is_cancelled() {
                    writeln!(stderr, "kill: ({id}) - No such process")?;
                    return Ok(ExecutionResult::general_error());
                }
                if let Err(error) = scope.work.signal(&task, signal, false) {
                    let reason = uucore::error::strip_errno(&error);
                    writeln!(stderr, "kill: ({id}) - {reason}")?;
                    return Ok(ExecutionResult::general_error());
                }
            }
            Ok(ExecutionResult::success())
        })
    }

    /// Whether `kill`'s `pid` reaches the runner: `$$` is its process, 0 and
    /// -1 its process group and every process, and the negative of its
    /// group ID its group.
    fn runner(pid: i32) -> bool {
        let process = rustix::process::getpid().as_raw_nonzero().get();
        let group = rustix::process::getpgrp().as_raw_nonzero().get();
        pid == 0 || pid == -1 || pid == process || pid == -group
    }

    /// `timeout` in the job's shell: COMMAND runs as `command` would run it,
    /// a builtin, a standard utility or a program, in a part of the job of
    /// its own (`timeout::supervise`).
    pub(super) fn timeout(
        context: ExecutionContext<'_>,
        args: Vec<CommandArg>,
    ) -> BoxFuture<'_, Result<ExecutionResult, brush_core::Error>> {
        Box::pin(async move {
            let words: Vec<String> = args.iter().skip(1).map(ToString::to_string).collect();
            let mut stderr = context.stderr();
            let options = match timeout::parse(&words) {
                timeout::Parsed::Run(options) => options,
                parsed => {
                    let status = timeout::answer(&parsed, &mut context.stdout(), &mut stderr);
                    return Ok(ExecutionExitCode::from(status).into());
                }
            };
            let parent = crate::interpreter::scope(context.shell)?;
            let part = parent.part();
            let mut target = context.shell.clone();
            let mut params = context.params;
            target.set_execution_host(Arc::new(part.clone()), &mut params);
            let program = options.command[0].clone();
            let mut command = SimpleCommand::new(
                ShellForCommand::OwnedShell {
                    target: Box::new(target),
                    parent: context.shell,
                },
                params,
                program.clone(),
                options
                    .command
                    .iter()
                    .map(|arg| CommandArg::String(arg.clone()))
                    .collect(),
            );
            command.use_functions = false;
            let run = async move {
                let result: ExecutionResult = command.execute().await?.wait().await?.into();
                Ok::<u8, brush_core::Error>(result.exit_code.into())
            };
            let status = match timeout::supervise(&options, &part, run, &mut stderr).await {
                Ok(status) => status,
                // The job or the task COMMAND belongs to is stopping.
                Err(error) if parent.check().is_err() => return Err(error),
                Err(error) => match error.kind() {
                    ErrorKind::CommandNotFound(_) => {
                        timeout::not_run(&program, "No such file or directory", true, &mut stderr)
                    }
                    ErrorKind::FailedToExecuteCommand(_, reason) => timeout::not_run(
                        &program,
                        &uucore::error::strip_errno(reason),
                        reason.kind() == std::io::ErrorKind::NotFound,
                        &mut stderr,
                    ),
                    _ => return Err(error),
                },
            };
            Ok(ExecutionExitCode::from(status).into())
        })
    }

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
        (
            'b',
            Some(Resource::SBSIZE),
            "socket buffer size",
            Unit::Bytes,
        ),
        ('c', Some(Resource::CORE), "core file size", Unit::Blocks),
        ('d', Some(Resource::DATA), "data seg size", Unit::Kbytes),
        (
            'e',
            Some(Resource::NICE),
            "scheduling priority",
            Unit::Number,
        ),
        ('f', Some(Resource::FSIZE), "file size", Unit::Blocks),
        (
            'i',
            Some(Resource::SIGPENDING),
            "pending signals",
            Unit::Number,
        ),
        ('k', Some(Resource::KQUEUES), "max kqueues", Unit::Number),
        (
            'l',
            Some(Resource::MEMLOCK),
            "max locked memory",
            Unit::Kbytes,
        ),
        ('m', Some(Resource::RSS), "max memory size", Unit::Kbytes),
        ('n', Some(Resource::NOFILE), "open files", Unit::Number),
        ('p', None, "pipe size", Unit::HalfKbytes),
        (
            'q',
            Some(Resource::MSGQUEUE),
            "POSIX message queues",
            Unit::Bytes,
        ),
        (
            'r',
            Some(Resource::RTPRIO),
            "real-time priority",
            Unit::Number,
        ),
        (
            'R',
            Some(Resource::RTTIME),
            "real-time non-blocking time",
            Unit::Microseconds,
        ),
        ('s', Some(Resource::STACK), "stack size", Unit::Kbytes),
        ('t', Some(Resource::CPU), "cpu time", Unit::Seconds),
        (
            'u',
            Some(Resource::NPROC),
            "max user processes",
            Unit::Number,
        ),
        ('v', Some(Resource::AS), "virtual memory", Unit::Kbytes),
        ('x', Some(Resource::LOCKS), "file locks", Unit::Number),
        (
            'P',
            Some(Resource::NPTS),
            "number of pseudoterminals",
            Unit::Number,
        ),
        (
            'T',
            Some(Resource::THREADS),
            "number of threads",
            Unit::Number,
        ),
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
            asked.resources = RESOURCES
                .iter()
                .map(|&(option, ..)| (option, None))
                .collect();
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
            None => demi_runner_process::process::child_limit(resource),
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
                        writeln!(
                            stderr,
                            "ulimit: {description}: cannot modify limit: Invalid argument"
                        )?;
                        return Ok(ExecutionResult::general_error());
                    }
                    let pipe_size =
                        u64::try_from(libc::PIPE_BUF).expect("a pipe buffer's size fits");
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
                    number => match number
                        .parse::<u64>()
                        .ok()
                        .and_then(|n| n.checked_mul(unit.scale()))
                    {
                        Some(limit) => limit,
                        None => {
                            writeln!(stderr, "ulimit: {number}: invalid number")?;
                            return Ok(ExecutionResult::general_error());
                        }
                    },
                };
                // Without -S or -H, both limits change, as in bash.
                let both = asked.soft == asked.hard;
                let raised = (asked.hard || both) && limit > hard;
                // Raising a hard limit takes privilege, measured against the
                // job's own limit: the probe below starts from the runner's,
                // which may be higher, as macOS's unlimited open files are.
                if raised && !rustix::process::geteuid().is_root() {
                    writeln!(
                        stderr,
                        "ulimit: {description}: cannot modify limit: Operation not permitted"
                    )?;
                    return Ok(ExecutionResult::general_error());
                }
                let soft = if asked.soft || both { limit } else { soft };
                let hard = if asked.hard || both { limit } else { hard };
                attributes.limits.retain(|&(set, ..)| set != resource);
                attributes.limits.push((resource, soft, hard));
                if let Err(error) = allowed(&context, &attributes).await {
                    writeln!(
                        stderr,
                        "ulimit: {description}: cannot modify limit: {error}"
                    )?;
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
    async fn allowed(
        context: &ExecutionContext<'_>,
        attributes: &ChildAttributes,
    ) -> std::io::Result<()> {
        let scope = crate::interpreter::scope(&*context.shell)?;
        let mut command = tokio::process::Command::new("/bin/sh");
        command
            .args(["-c", ":"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let mut command = demi_runner_process::process::wrap(
            command,
            false,
            &crate::scope::child_attributes(attributes),
        );
        let mut child = demi_runner_process::process::start(|| {
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
                match arg
                    .strip_prefix('-')
                    .filter(|options| !options.is_empty() && mode.is_none())
                {
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
                .unwrap_or_else(demi_runner_process::process::umask);
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
