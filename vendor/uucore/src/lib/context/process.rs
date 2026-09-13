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
pub struct Command(process_wrap::std::CommandWrap);
impl Command {
    pub fn current_dir(&mut self, path: impl AsRef<std::path::Path>) -> &mut Self {
        self.0.command_mut().current_dir(super::resolve(path));
        self
    }
    pub fn arg(&mut self, arg: impl AsRef<OsStr>) -> &mut Self {
        self.0.command_mut().arg(arg);
        self
    }
    pub fn args(&mut self, args: impl IntoIterator<Item = impl AsRef<OsStr>>) -> &mut Self {
        self.0.command_mut().args(args);
        self
    }
    pub fn stdin(&mut self, input: impl Into<Stdio>) -> &mut Self {
        self.0.command_mut().stdin(input);
        self
    }
    pub fn stdout(&mut self, output: impl Into<Stdio>) -> &mut Self {
        self.0.command_mut().stdout(output);
        self
    }
    pub fn stderr(&mut self, output: impl Into<Stdio>) -> &mut Self {
        self.0.command_mut().stderr(output);
        self
    }
    pub fn env(&mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> &mut Self {
        self.0.command_mut().env(key, value);
        self
    }
    pub fn envs(
        &mut self,
        values: impl IntoIterator<Item = (impl AsRef<OsStr>, impl AsRef<OsStr>)>,
    ) -> &mut Self {
        self.0.command_mut().envs(values);
        self
    }
    pub fn env_remove(&mut self, key: impl AsRef<OsStr>) -> &mut Self {
        self.0.command_mut().env_remove(key);
        self
    }
    pub fn env_clear(&mut self) -> &mut Self {
        self.0.command_mut().env_clear();
        self
    }
    pub fn spawn(&mut self) -> std::io::Result<Child> {
        super::check_cancelled();
        let mut inner = self.0.spawn()?;
        let stdin = inner.stdin().take().map(child_file);
        let stdout = inner.stdout().take().map(child_file);
        let stderr = inner.stderr().take().map(child_file);
        let guard = super::control().map(|control| control.task_guard());
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
                .envs(&context.env)
                .stdin(
                    context
                        .stdin
                        .try_clone()
                        .expect("duplicate invocation stdin"),
                )
                .stdout(
                    context
                        .stdout
                        .try_clone()
                        .expect("duplicate invocation stdout"),
                )
                .stderr(
                    context
                        .stderr
                        .try_clone()
                        .expect("duplicate invocation stderr"),
                );
            #[cfg(unix)]
            {
                use command_fds::{CommandFdExt, FdMapping};
                use std::os::fd::AsFd;
                let mappings = context
                    .descriptors
                    .iter()
                    .map(|(fd, file)| FdMapping {
                        child_fd: *fd,
                        parent_fd: file
                            .as_fd()
                            .try_clone_to_owned()
                            .expect("duplicate invocation descriptor"),
                    })
                    .collect();
                command
                    .fd_mappings(mappings)
                    .expect("valid invocation descriptors");
            }
        });
        let mut command = process_wrap::std::CommandWrap::from(command);
        #[cfg(unix)]
        command.wrap(process_wrap::std::ProcessGroup::leader());
        #[cfg(windows)]
        command.wrap(process_wrap::std::JobObject);
        Self(command)
    }
}
impl Deref for Command {
    type Target = std::process::Command;
    fn deref(&self) -> &Self::Target {
        self.0.command()
    }
}
impl DerefMut for Command {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.command_mut()
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
