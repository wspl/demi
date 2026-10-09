//! A fresh brush shell for one job: its options, its builtins, and the run
//! of its script.

use brush_core::{
    CommandArg, ExecutionContext, ExecutionResult, Shell, SourceInfo, builtins,
    execution_host::FileControl, openfiles::OpenFile,
};
use demi_runner_process::{command_client::CONTEXT_ENV, stdio::is_live};
use std::{
    collections::{BTreeMap, HashMap},
    fs::File,
    io::{self, Write as _},
    path::PathBuf,
    sync::Arc,
};

use crate::scope::Scope;

pub struct ShellOptions {
    pub scope: Scope,
    pub login: bool,
    /// Where the job starts.
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    pub stdin: File,
    pub stdout: File,
    pub stderr: File,
}

pub struct ShellResult {
    pub code: u8,
}

pub async fn execute(
    script: &str,
    options: ShellOptions,
) -> Result<ShellResult, brush_core::Error> {
    // A directory that does not exist fails the job before its script runs
    // (`runner.md` § Shell jobs). Standard output stays the script's alone,
    // so a job whose stdout is piped elsewhere carries no line of the
    // runner's.
    if !options.cwd.try_exists()? {
        let line = format!(
            "demi: cannot start in {}: No such file or directory\n",
            options.cwd.display()
        );
        (&options.stderr).write_all(line.as_bytes())?;
        return Ok(ShellResult { code: 1 });
    }
    let cwd = options.cwd;
    // Every copy the shell makes of these goes through the job's scope, which
    // waits out a lack of open files (`runner.md` § Load).
    let control: Arc<dyn FileControl> = Arc::new(options.scope.clone());
    let fds = HashMap::from([
        (
            0,
            OpenFile::Controlled {
                file: options.stdin,
                control: control.clone(),
            },
        ),
        (
            1,
            OpenFile::Controlled {
                file: options.stdout,
                control: control.clone(),
            },
        ),
        (
            2,
            OpenFile::Controlled {
                file: options.stderr,
                control,
            },
        ),
    ]);
    let mut registrations = registrations();
    if let Some(commands) = &options.scope.commands {
        for name in &commands.roots {
            registrations.insert(
                name.clone(),
                builtins::Registration {
                    execute_func: crate::declared::execute,
                    content_func: |name, _, _| {
                        Ok(format!("{name}: use {name} --help for command options"))
                    },
                    disabled: false,
                    special_builtin: false,
                    declaration_builtin: false,
                },
            );
        }
    }
    let mut builder = Shell::builder()
        .execution_host(
            Arc::new(options.scope.clone()) as Arc<dyn brush_core::execution_host::ExecutionHost>
        )
        .builtins(registrations)
        .working_dir(cwd.clone())
        .login(options.login)
        .do_not_inherit_env(true)
        .fds(fds);
    for (name, value) in &options.env {
        let mut variable = brush_core::ShellVariable::new(value.clone());
        variable.export();
        builder = builder.var(name, variable);
    }
    let mut shell = builder.build().await?;
    if options.login {
        restore_execution_context(&mut shell, &options.env)?;
        shell.set_working_dir(cwd)?;
    }
    let result = shell
        .run_string(script, &SourceInfo::default(), &shell.default_exec_params())
        .await?;
    // The runner job owns asynchronous shell tasks until completion or cancellation.
    // Keep their interpreter and builtin threads alive while retaining the foreground status.
    crate::process_builtins::wait_tasks(&mut shell).await?;
    Ok(ShellResult {
        code: result.exit_code.into(),
    })
}

/// The builtins every job's shell has: brush's, the runner's own in place of
/// those that would act on the runner's process (`process_builtins`), and
/// the standard utilities.
pub(crate) fn registrations()
-> HashMap<String, builtins::Registration<brush_core::extensions::DefaultShellExtensions>> {
    let mut registrations = brush_builtins::default_builtins(brush_builtins::BuiltinSet::BashMode);
    crate::process_builtins::register(&mut registrations);
    for &(name, _) in crate::utilities::UTILITIES {
        registrations.insert(
            name.to_owned(),
            builtins::Registration {
                execute_func: execute_utility,
                content_func: |name, _, _| {
                    Ok(format!("{name}: use {name} --help for utility options"))
                },
                disabled: false,
                special_builtin: false,
                declaration_builtin: false,
            },
        );
    }
    registrations
}

/// The job that owns `shell`: its execution host is the job's scope.
pub(crate) fn scope(shell: &Shell) -> io::Result<Scope> {
    // Brush hands its host back only as the trait object it was given.
    shell
        .execution_host()
        .and_then(|host| (host.as_ref() as &dyn std::any::Any).downcast_ref::<Scope>())
        .cloned()
        .ok_or_else(|| io::Error::other("missing shell execution owner"))
}

/// Login profiles configure user tools; the runner's own variables, its
/// `DEMI_*` names, stay authoritative. `TMPDIR` is the device's, which a
/// profile may set as for any other program (`runner.md` § Shell jobs).
fn restore_execution_context(
    shell: &mut Shell,
    env: &BTreeMap<String, String>,
) -> Result<(), brush_core::Error> {
    for (name, value) in env {
        if name.starts_with("DEMI_") {
            let mut variable = brush_core::ShellVariable::new(value.clone());
            variable.export();
            shell.env_mut().set_global(name, variable)?;
        }
    }
    // A live command context always owns the first PATH entry, created by
    // ExecutionContext::environment. Preserve that alias directory after profiles.
    if env.contains_key(CONTEXT_ENV) {
        let aliases = env
            .get("PATH")
            .and_then(|path| std::env::split_paths(path).next())
            .ok_or_else(|| io::Error::other("command context has no alias directory"))?;
        let configured = shell
            .env()
            .get("PATH")
            .map(|(_, variable)| variable.value().to_cow_str(shell).into_owned())
            .unwrap_or_default();
        let paths = std::iter::once(aliases.clone())
            .chain(std::env::split_paths(&configured).filter(|path| *path != aliases));
        let path = std::env::join_paths(paths).map_err(io::Error::other)?;
        let path = path
            .into_string()
            .map_err(|_| io::Error::other("profile PATH is not UTF-8"))?;
        let mut variable = brush_core::ShellVariable::new(path);
        variable.export();
        shell.env_mut().set_global("PATH", variable)?;
    }
    Ok(())
}

fn execute_utility(
    context: ExecutionContext<'_>,
    args: Vec<CommandArg>,
) -> builtins::BoxFuture<'_, Result<ExecutionResult, brush_core::Error>> {
    Box::pin(async move {
        let name = crate::utilities::UTILITIES
            .iter()
            .map(|(name, _)| *name)
            .find(|name| *name == context.command_name)
            .ok_or_else(|| io::Error::other("unregistered native utility"))?;
        let env = context
            .shell
            .env()
            .iter_exported()
            .filter(|(_, variable)| variable.value().is_set())
            .map(|(name, variable)| {
                (
                    name.clone(),
                    variable.value().to_cow_str(context.shell).into_owned(),
                )
            })
            .collect();
        let stdin = Arc::new(invocation_file(&context, 0)?);
        let scope = scope(context.shell)?;
        // The utility's files and the programs it starts get the umask and
        // limits of the shell that ran it (`runner.md` § Builtins that act on
        // a process).
        let attributes = crate::scope::child_attributes(context.shell.child_attributes());
        #[cfg(unix)]
        let umask = attributes
            .umask
            .unwrap_or_else(demi_runner_process::process::umask);
        #[cfg(windows)]
        let umask = 0o022;
        let invocation = uucore::context::Context {
            name,
            descriptors: context
                .iter_fds()
                .filter(|(fd, _)| *fd > 2)
                .map(|(fd, file)| Ok((fd, Arc::new(native_file(file)?))))
                .collect::<Result<_, brush_core::Error>>()?,
            control: Some(Arc::new(crate::scope::UtilityControl {
                scope: scope.clone(),
                attributes,
            })),
            live_input: is_live(&stdin, &env)?,
            umask,
            cwd: context.shell.working_dir().to_owned(),
            env,
            stdin,
            stdout: Arc::new(invocation_file(&context, 1)?),
            stderr: Arc::new(invocation_file(&context, 2)?),
        };
        let args = args.into_iter().map(|arg| arg.to_string().into()).collect();
        let code = scope
            .tasks
            .spawn_blocking(move || crate::utilities::run(invocation, args))
            .await
            .map_err(io::Error::other)?
            .map_err(io::Error::other)?;
        Ok(brush_core::ExecutionExitCode::from(code as u8).into())
    })
}

pub(crate) fn invocation_file(
    context: &ExecutionContext<'_>,
    descriptor: brush_core::ShellFd,
) -> Result<File, brush_core::Error> {
    let file = context
        .try_fd(descriptor)
        .ok_or_else(|| io::Error::other(format!("closed descriptor {descriptor}")))?;
    native_file(file)
}

/// The native file of a descriptor the shell has already copied for the
/// utility, so no second copy is made.
fn native_file(file: OpenFile) -> Result<File, brush_core::Error> {
    Ok(file.into_file()?)
}
