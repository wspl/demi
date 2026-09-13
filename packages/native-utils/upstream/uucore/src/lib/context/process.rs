//! Child programs inherit the invocation's streams, cwd and environment.

pub use std::process::{Child, ChildStdin, ChildStdout, ChildStderr, ExitCode, ExitStatus, Output, Stdio, Termination, abort, id};
use std::{ffi::OsStr, ops::{Deref, DerefMut}};

pub fn exit(code: i32) -> ! { super::exit(code) }

#[derive(Debug)]
pub struct Command(std::process::Command);
impl Command {
    pub fn current_dir(&mut self, path: impl AsRef<std::path::Path>) -> &mut Self {
        self.0.current_dir(super::resolve(path));
        self
    }
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        let mut command = std::process::Command::new(program);
        super::CURRENT.with(|current| {
            let current = current.borrow();
            let context = current.as_ref().expect("utility context");
            command.current_dir(&context.cwd).env_clear().envs(&context.env)
                .stdin(context.stdin.try_clone().expect("duplicate invocation stdin"))
                .stdout(context.stdout.try_clone().expect("duplicate invocation stdout"))
                .stderr(context.stderr.try_clone().expect("duplicate invocation stderr"));
        });
        Self(command)
    }
}
impl Deref for Command {
    type Target = std::process::Command;
    fn deref(&self) -> &Self::Target { &self.0 }
}
impl DerefMut for Command {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.0 }
}
