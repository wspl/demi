//! Child programs inherit the invocation's streams, cwd and environment.

pub use std::process::{ExitCode, ExitStatus, Output, Termination, abort, id};
use std::{
    ffi::{OsStr, OsString},
    ops::{Deref, DerefMut},
    sync::Arc,
};

pub type ChildStdin = super::fs::File;
pub type ChildStdout = super::fs::File;
pub type ChildStderr = super::fs::File;

pub fn exit(code: i32) -> ! {
    super::exit(code)
}

/// A child's standard stream, as std's `Stdio` is, kept as what it is so
/// that a child the embedding owner runs as one of its own utilities gets
/// the same stream.
#[derive(Debug)]
pub struct Stdio(Stream);

#[derive(Debug)]
enum Stream {
    Inherit,
    Null,
    Piped,
    File(std::fs::File),
}

impl Stdio {
    pub fn inherit() -> Self {
        Self(Stream::Inherit)
    }
    pub fn null() -> Self {
        Self(Stream::Null)
    }
    pub fn piped() -> Self {
        Self(Stream::Piped)
    }
}
impl From<std::fs::File> for Stdio {
    fn from(file: std::fs::File) -> Self {
        Self(Stream::File(file))
    }
}
impl From<super::fs::File> for Stdio {
    fn from(file: super::fs::File) -> Self {
        Self(Stream::File(file.into()))
    }
}
#[cfg(unix)]
impl From<std::os::fd::OwnedFd> for Stdio {
    fn from(descriptor: std::os::fd::OwnedFd) -> Self {
        Self(Stream::File(descriptor.into()))
    }
}
#[cfg(windows)]
impl From<std::os::windows::io::OwnedHandle> for Stdio {
    fn from(handle: std::os::windows::io::OwnedHandle) -> Self {
        Self(Stream::File(handle.into()))
    }
}
impl From<Stdio> for std::process::Stdio {
    fn from(stdio: Stdio) -> Self {
        match stdio.0 {
            Stream::Inherit => Self::inherit(),
            Stream::Null => Self::null(),
            Stream::Piped => Self::piped(),
            Stream::File(file) => file.into(),
        }
    }
}

#[derive(Debug)]
pub struct Command {
    inner: process_wrap::std::CommandWrap,
    /// The standard streams the caller chose; the child gets the
    /// invocation's own for the others when it starts.
    stdin: Option<Stdio>,
    stdout: Option<Stdio>,
    stderr: Option<Stdio>,
}

impl Command {
    pub fn current_dir(&mut self, path: impl AsRef<std::path::Path>) -> &mut Self {
        self.inner.command_mut().current_dir(super::resolve(path));
        self
    }
    pub fn arg(&mut self, arg: impl AsRef<OsStr>) -> &mut Self {
        self.inner.command_mut().arg(arg);
        self
    }
    pub fn args(&mut self, args: impl IntoIterator<Item = impl AsRef<OsStr>>) -> &mut Self {
        self.inner.command_mut().args(args);
        self
    }
    pub fn stdin(&mut self, input: impl Into<Stdio>) -> &mut Self {
        self.stdin = Some(input.into());
        self
    }
    pub fn stdout(&mut self, output: impl Into<Stdio>) -> &mut Self {
        self.stdout = Some(output.into());
        self
    }
    pub fn stderr(&mut self, output: impl Into<Stdio>) -> &mut Self {
        self.stderr = Some(output.into());
        self
    }
    pub fn env(&mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> &mut Self {
        self.inner.command_mut().env(key, value);
        self
    }
    pub fn envs(
        &mut self,
        values: impl IntoIterator<Item = (impl AsRef<OsStr>, impl AsRef<OsStr>)>,
    ) -> &mut Self {
        self.inner.command_mut().envs(values);
        self
    }
    pub fn env_remove(&mut self, key: impl AsRef<OsStr>) -> &mut Self {
        self.inner.command_mut().env_remove(key);
        self
    }
    pub fn env_clear(&mut self) -> &mut Self {
        self.inner.command_mut().env_clear();
        self
    }
    pub fn spawn(&mut self) -> std::io::Result<Child> {
        super::check_cancelled();
        let control = super::control();
        if let Some(control) = &control
            && let Some(name) = control.utility(self.inner.command().get_program())
        {
            return self.run_utility(control.as_ref(), name);
        }
        self.inherit()?;
        let mut inner = match &control {
            Some(control) => control.spawn(&mut self.inner)?,
            None => self.inner.spawn()?,
        };
        let stdin = inner.stdin().take().map(child_file);
        let stdout = inner.stdout().take().map(child_file);
        let stderr = inner.stderr().take().map(child_file);
        let guard = control.map(|control| control.task_guard());
        Ok(Child {
            inner: Running::Process(inner),
            stdin,
            stdout,
            stderr,
            _guard: guard,
        })
    }

    /// Runs the embedding owner's utility `name`, which the program names,
    /// in the owner's process with this command's arguments, directory,
    /// environment and streams, as the owner's shell would run it.
    fn run_utility(
        &mut self,
        control: &dyn super::Control,
        name: &'static str,
    ) -> std::io::Result<Child> {
        let invocation = super::snapshot();
        let own_input = matches!(self.stdin, None | Some(Stdio(Stream::Inherit)));
        let (stdin, parent_stdin) = utility_stream(self.stdin.take(), &invocation.stdin, true)?;
        let (stdout, parent_stdout) = utility_stream(self.stdout.take(), &invocation.stdout, false)?;
        let (stderr, parent_stderr) = utility_stream(self.stderr.take(), &invocation.stderr, false)?;
        let command = self.inner.command();
        // `new` cleared the environment and set the invocation's, so what
        // the command sets is all the child has.
        let env = command
            .get_envs()
            .filter_map(|(key, value)| {
                Some((
                    key.to_string_lossy().into_owned(),
                    value?.to_string_lossy().into_owned(),
                ))
            })
            .collect();
        let context = super::Context {
            name,
            control: invocation.control.clone(),
            live_input: invocation.live_input && own_input,
            umask: invocation.umask,
            cwd: command
                .get_current_dir()
                .map_or_else(|| invocation.cwd.clone(), ToOwned::to_owned),
            env,
            descriptors: invocation.descriptors.clone(),
            stdin,
            stdout,
            stderr,
        };
        let args = std::iter::once(OsString::from(name))
            .chain(command.get_args().map(ToOwned::to_owned))
            .collect();
        let run = control.run_utility(context, args)?;
        Ok(Child {
            inner: Running::Utility(run),
            stdin: parent_stdin,
            stdout: parent_stdout,
            stderr: parent_stderr,
            _guard: Some(control.task_guard()),
        })
    }
    pub fn status(&mut self) -> std::io::Result<ExitStatus> {
        self.spawn()?.wait()
    }
    pub fn output(&mut self) -> std::io::Result<Output> {
        self.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?
            .wait_with_output()
    }
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        let mut command = std::process::Command::new(program);
        super::CURRENT.with(|current| {
            let current = current.borrow();
            let context = current.as_ref().expect("utility context");
            command
                .current_dir(&context.cwd)
                .env_clear()
                .envs(&context.env);
        });
        let mut command = process_wrap::std::CommandWrap::from(command);
        #[cfg(unix)]
        command.wrap(process_wrap::std::ProcessGroup::leader());
        #[cfg(windows)]
        command.wrap(process_wrap::std::JobObject);
        Self {
            inner: command,
            stdin: None,
            stdout: None,
            stderr: None,
        }
    }

    /// Gives the child the invocation's standard streams the caller did not
    /// choose, and its other descriptors, each duplicated through the
    /// embedding owner, which may wait out a lack of open files.
    fn inherit(&mut self) -> std::io::Result<()> {
        let context = super::snapshot();
        let stdin = process_stream(self.stdin.take(), &context.stdin)?;
        let stdout = process_stream(self.stdout.take(), &context.stdout)?;
        let stderr = process_stream(self.stderr.take(), &context.stderr)?;
        let command = self.inner.command_mut();
        command.stdin(stdin).stdout(stdout).stderr(stderr);
        #[cfg(unix)]
        {
            use command_fds::{CommandFdExt, FdMapping};
            let mappings = context
                .descriptors
                .iter()
                .map(|(fd, file)| {
                    Ok(FdMapping {
                        child_fd: *fd,
                        parent_fd: super::duplicate(file)?.into(),
                    })
                })
                .collect::<std::io::Result<_>>()?;
            // The invocation's descriptor numbers are distinct.
            command
                .fd_mappings(mappings)
                .expect("valid invocation descriptors");
        }
        Ok(())
    }
}
impl Deref for Command {
    type Target = std::process::Command;
    fn deref(&self) -> &Self::Target {
        self.inner.command()
    }
}
impl DerefMut for Command {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.command_mut()
    }
}

/// The stream a child program gets: the one the caller chose, or a copy of
/// the invocation's own, made through the embedding owner, which may wait
/// out a lack of open files.
fn process_stream(
    chosen: Option<Stdio>,
    own: &std::fs::File,
) -> std::io::Result<std::process::Stdio> {
    match chosen {
        Some(stdio) => Ok(stdio.into()),
        None => Ok(super::duplicate(own)?.into()),
    }
}

/// The stream a utility run in process gets, and the caller's end of it
/// when the caller chose a pipe. Inheriting is the invocation's own stream:
/// the utility shares the invocation's process.
fn utility_stream(
    chosen: Option<Stdio>,
    own: &Arc<std::fs::File>,
    input: bool,
) -> std::io::Result<(Arc<std::fs::File>, Option<super::fs::File>)> {
    Ok(match chosen.map(|stdio| stdio.0) {
        None | Some(Stream::Inherit) => (own.clone(), None),
        Some(Stream::File(file)) => (Arc::new(file), None),
        Some(Stream::Null) => {
            #[cfg(unix)]
            let device = "/dev/null";
            #[cfg(windows)]
            let device = "NUL";
            let file = std::fs::OpenOptions::new()
                .read(input)
                .write(!input)
                .open(device)?;
            (Arc::new(file), None)
        }
        Some(Stream::Piped) => {
            let (reader, writer) = std::io::pipe()?;
            let (reader, writer) = (pipe_file(reader), pipe_file(writer));
            if input {
                (Arc::new(reader), Some(writer.into()))
            } else {
                (Arc::new(writer), Some(reader.into()))
            }
        }
    })
}

#[cfg(unix)]
fn pipe_file(end: impl Into<std::os::fd::OwnedFd>) -> std::fs::File {
    end.into().into()
}
#[cfg(windows)]
fn pipe_file(end: impl Into<std::os::windows::io::OwnedHandle>) -> std::fs::File {
    end.into().into()
}

/// What a child is: a program, or one of the embedding owner's utilities
/// running in its process.
enum Running {
    Process(Box<dyn process_wrap::std::ChildWrapper>),
    Utility(Box<dyn super::UtilityRun>),
}

/// Every native utility child is killed and reaped when its invocation unwinds.
pub struct Child {
    inner: Running,
    pub stdin: Option<super::fs::File>,
    pub stdout: Option<super::fs::File>,
    pub stderr: Option<super::fs::File>,
    _guard: Option<Box<dyn Send + Sync>>,
}

impl Child {
    /// The program's process id; none for a utility that runs in the
    /// embedding owner's process.
    pub fn id(&self) -> Option<u32> {
        match &self.inner {
            Running::Process(child) => Some(child.id()),
            Running::Utility(_) => None,
        }
    }
    pub fn kill(&mut self) -> std::io::Result<()> {
        match &mut self.inner {
            Running::Process(child) => match child.kill() {
                #[cfg(unix)]
                Err(error) if error.raw_os_error() == Some(libc::ESRCH) => Ok(()),
                result => result,
            },
            Running::Utility(run) => run.kill(),
        }
    }
    pub fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        match &mut self.inner {
            Running::Process(child) => child.try_wait(),
            Running::Utility(run) => run.try_wait(),
        }
    }
    pub fn wait(&mut self) -> std::io::Result<ExitStatus> {
        self.stdin.take();
        loop {
            // A program ends in its own time once the invocation's work is
            // to stop, unless it is to be killed; a utility ends with it.
            match self.inner {
                Running::Process(_) => super::check_processes(),
                Running::Utility(_) => super::check_cancelled(),
            }
            if let Some(status) = self.try_wait()? {
                return Ok(status);
            }
            super::thread::sleep(std::time::Duration::from_millis(25));
        }
    }
    pub fn wait_with_output(mut self) -> std::io::Result<Output> {
        use std::io::Read;
        self.stdin.take();
        let stderr = self.stderr.take();
        let reader = super::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut file) = stderr {
                file.read_to_end(&mut bytes)?;
            }
            Ok::<_, std::io::Error>(bytes)
        });
        let mut stdout = Vec::new();
        let read = match self.stdout.take() {
            Some(mut file) => file.read_to_end(&mut stdout).map(|_| ()),
            None => Ok(()),
        };
        if read.is_err() {
            self.kill()?;
        }
        let status = self.wait();
        let stderr = reader
            .join()
            .map_err(|_| std::io::Error::other("child stderr reader panicked"))??;
        read?;
        Ok(Output {
            status: status?,
            stdout,
            stderr,
        })
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        if let Err(error) = self.kill() {
            eprintln!("utility child cleanup failed: {error}");
        }
        let reaped = match &mut self.inner {
            Running::Process(child) => child.wait(),
            Running::Utility(run) => run.wait(),
        };
        if let Err(error) = reaped {
            eprintln!("utility child reap failed: {error}");
        }
    }
}

#[cfg(unix)]
fn child_file(handle: impl Into<std::os::fd::OwnedFd>) -> super::fs::File {
    std::fs::File::from(handle.into()).into()
}
#[cfg(windows)]
fn child_file(handle: impl Into<std::os::windows::io::OwnedHandle>) -> super::fs::File {
    std::fs::File::from(handle.into()).into()
}
