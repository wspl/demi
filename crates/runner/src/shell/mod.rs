//! Fresh brush shells with streaming, in-process native utilities.

mod declared;
pub mod job;
pub mod scope;
pub mod utilities;

use brush_core::{
    CommandArg, ExecutionContext, ExecutionResult, Shell, SourceInfo, builtins, openfiles::OpenFile,
};
use std::{
    collections::{BTreeMap, HashMap},
    fs::File,
    io,
    path::PathBuf,
    sync::Arc,
};

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
    let fds = HashMap::from([
        (0, OpenFile::File(options.stdin)),
        (1, OpenFile::File(options.stdout)),
        (2, OpenFile::File(options.stderr)),
    ]);
    let mut registrations = brush_builtins::default_builtins(brush_builtins::BuiltinSet::BashMode);
    for &name in crate::shell::utilities::NAMES {
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
        let name = crate::shell::utilities::NAMES
            .iter()
            .copied()
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

fn native_file(file: OpenFile) -> Result<File, brush_core::Error> {
    #[cfg(unix)]
    {
        Ok(File::from(file.try_borrow_as_fd()?.try_clone_to_owned()?))
    }
    #[cfg(windows)]
    {
        Ok(file.into_file()?)
    }
}
