//! Fresh brush shells with streaming, in-process native utilities.

mod declared;
pub mod edit_report;
pub mod job;
pub mod scope;
pub mod utilities;

use brush_core::{
    CommandArg, ExecutionContext, ExecutionResult, Shell, SourceInfo, builtins,
    execution_host::FileControl, openfiles::OpenFile,
};
use std::{
    collections::{BTreeMap, HashMap},
    fs::File,
    io,
    path::PathBuf,
    sync::Arc,
};

/// The shell runtime (`concurrency.md` § Runner): every interpreter unit
/// and utility of a job runs on a thread of its blocking pool, apart from the
/// control thread. A unit blocked on a full pipe waits for a sibling that has
/// a thread of its own, since the pool is far larger than a job can use.
#[derive(Clone)]
pub struct ShellRuntime(tokio::runtime::Handle);

/// Far above the units a job can have: each holds at least one pipe, and a
/// process has at most a few thousand open files.
const UNIT_THREADS: usize = 4096;

impl ShellRuntime {
    /// One worker drives the units' asynchronous parts, such as reaping
    /// children; the blocking pool runs the units themselves.
    pub fn build() -> io::Result<tokio::runtime::Runtime> {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .max_blocking_threads(UNIT_THREADS)
            .thread_name("shell")
            .enable_all()
            .build()
    }

    pub fn new(runtime: &tokio::runtime::Runtime) -> Self {
        Self(runtime.handle().clone())
    }

    /// The runtime the caller runs on, for tests that host jobs themselves.
    pub fn current() -> Self {
        Self(tokio::runtime::Handle::current())
    }

    /// Runs `work` on a thread of the shell pool, inside the shell runtime,
    /// so what it spawns stays there too.
    pub fn spawn_blocking<F, R>(&self, work: F) -> tokio::task::JoinHandle<R>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        self.0.spawn_blocking(work)
    }
}

pub struct ShellOptions {
    pub scope: scope::Scope,
    pub login: bool,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    pub stdin: File,
    pub stdout: File,
    pub stderr: File,
}

pub struct ShellResult {
    pub code: u8,
    pub cwd: PathBuf,
}

pub async fn execute(
    script: &str,
    options: ShellOptions,
) -> Result<ShellResult, brush_core::Error> {
    // Every copy the shell makes of these goes through the job's scope, which
    // waits out a lack of open files (`runner.md` § Load).
    let control: Arc<dyn FileControl> = Arc::new(options.scope.clone());
    let fds = HashMap::from([
        (0, OpenFile::Controlled { file: options.stdin, control: control.clone() }),
        (1, OpenFile::Controlled { file: options.stdout, control: control.clone() }),
        (2, OpenFile::Controlled { file: options.stderr, control }),
    ]);
    let mut registrations = brush_builtins::default_builtins(brush_builtins::BuiltinSet::BashMode);
    for &(name, _) in crate::shell::utilities::UTILITIES {
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
    if let Some(commands) = &options.scope.commands {
        for name in commands.execution.manifest.roots.keys() {
            registrations.insert(
                name.clone(),
                builtins::Registration {
                    execute_func: declared::execute,
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
        .working_dir(options.cwd.clone())
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
        shell.set_working_dir(options.cwd)?;
    }
    let result = shell
        .run_string(script, &SourceInfo::default(), &shell.default_exec_params())
        .await?;
    // The runner job owns asynchronous shell tasks until completion or cancellation.
    // Keep their interpreter and builtin threads alive while retaining the foreground status.
    shell.jobs_mut().wait_all().await?;
    Ok(ShellResult {
        code: result.exit_code.into(),
        cwd: shell.working_dir().to_owned(),
    })
}

/// Login profiles configure user tools; runner-owned context stays authoritative.
fn restore_execution_context(
    shell: &mut Shell,
    env: &BTreeMap<String, String>,
) -> Result<(), brush_core::Error> {
    for (name, value) in env {
        if name.starts_with("DEMI_") || matches!(name.as_str(), "TMPDIR" | "TEMP") {
            let mut variable = brush_core::ShellVariable::new(value.clone());
            variable.export();
            shell.env_mut().set_global(name, variable)?;
        }
    }
    // A live command context always owns the first PATH entry, created by
    // ExecutionContext::environment. Preserve that alias directory after profiles.
    if env.contains_key(crate::commands::command_client::CONTEXT_ENV) {
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
        let name = crate::shell::utilities::UTILITIES
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
        let scope = context
            .shell
            .execution_host()
            .and_then(|host| (host.as_ref() as &dyn std::any::Any).downcast_ref::<scope::Scope>())
            .ok_or_else(|| io::Error::other("missing shell execution owner"))?
            .clone();
        let invocation = crate::shell::utilities::Context {
            name,
            descriptors: context
                .iter_fds()
                .filter(|(fd, _)| *fd > 2)
                .map(|(fd, file)| Ok((fd, Arc::new(native_file(file)?))))
                .collect::<Result<_, brush_core::Error>>()?,
            control: Some(Arc::new(scope.clone())),
            live_input: crate::stdio::is_live(&stdin, &env)?,
            umask: 0o022,
            cwd: context.shell.working_dir().to_owned(),
            env,
            stdin,
            stdout: Arc::new(invocation_file(&context, 1)?),
            stderr: Arc::new(invocation_file(&context, 2)?),
        };
        let args = args.into_iter().map(|arg| arg.to_string().into()).collect();
        let code = scope
            .tasks
            .spawn_blocking(move || crate::shell::utilities::run(invocation, args))
            .await
            .map_err(io::Error::other)?
            .map_err(io::Error::other)?;
        Ok(brush_core::ExecutionExitCode::from(code as u8).into())
    })
}

fn invocation_file(
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
