//! Child programs inherit the invocation's streams, cwd and environment.

pub use std::process::{ExitCode, ExitStatus, Output, Stdio, Termination, abort, id};
use std::{
    ffi::OsStr,
    ops::{Deref, DerefMut},
};

pub type ChildStdin = super::fs::File;
pub type ChildStdout = super::fs::File;
pub type ChildStderr = super::fs::File;

pub fn exit(code: i32) -> ! {
    super::exit(code)
}

#[derive(Debug)]
pub struct Command {
    inner: process_wrap::std::CommandWrap,
    /// The standard streams the caller chose; the child gets the
    /// invocation's own for the others when it starts.
    chosen: Chosen,
}

/// Which standard streams a caller set.
#[derive(Debug, Default)]
struct Chosen {
    stdin: bool,
    stdout: bool,
    stderr: bool,
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
        self.inner.command_mut().stdin(input);
        self.chosen.stdin = true;
        self
    }
    pub fn stdout(&mut self, output: impl Into<Stdio>) -> &mut Self {
        self.inner.command_mut().stdout(output);
        self.chosen.stdout = true;
        self
    }
    pub fn stderr(&mut self, output: impl Into<Stdio>) -> &mut Self {
        self.inner.command_mut().stderr(output);
        self.chosen.stderr = true;
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
        self.inherit()?;
        let control = super::control();
        let mut inner = match &control {
            Some(control) => control.spawn(&mut self.inner)?,
            None => self.inner.spawn()?,
        };
        let stdin = inner.stdin().take().map(child_file);
        let stdout = inner.stdout().take().map(child_file);
        let stderr = inner.stderr().take().map(child_file);
        let guard = control.map(|control| control.task_guard());
        Ok(Child {
            inner,
            stdin,
            stdout,
            stderr,
            _guard: guard,
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
            chosen: Chosen::default(),
        }
    }

    /// Gives the child the invocation's standard streams the caller did not
    /// choose, and its other descriptors, each duplicated through the
    /// embedding owner, which may wait out a lack of open files.
    fn inherit(&mut self) -> std::io::Result<()> {
        let context = super::snapshot();
        let command = self.inner.command_mut();
        if !self.chosen.stdin {
            command.stdin(super::duplicate(&context.stdin)?);
        }
        if !self.chosen.stdout {
            command.stdout(super::duplicate(&context.stdout)?);
        }
        if !self.chosen.stderr {
            command.stderr(super::duplicate(&context.stderr)?);
        }
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

/// Every native utility child is killed and reaped when its invocation unwinds.
pub struct Child {
    inner: Box<dyn process_wrap::std::ChildWrapper>,
    pub stdin: Option<super::fs::File>,
    pub stdout: Option<super::fs::File>,
    pub stderr: Option<super::fs::File>,
    _guard: Option<Box<dyn Send + Sync>>,
}

impl Child {
    pub fn id(&self) -> u32 {
        self.inner.id()
    }
    pub fn kill(&mut self) -> std::io::Result<()> {
        match self.inner.kill() {
            #[cfg(unix)]
            Err(error) if error.raw_os_error() == Some(libc::ESRCH) => Ok(()),
            result => result,
        }
    }
    pub fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.inner.try_wait()
    }
    pub fn wait(&mut self) -> std::io::Result<ExitStatus> {
        self.stdin.take();
        loop {
            super::check_cancelled();
            if let Some(status) = self.inner.try_wait()? {
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
        if let Err(error) = self.inner.wait() {
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
