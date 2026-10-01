//! Test support (feature `testing`): a Host type for agents that run no
//! shell tools, and readers of a shell tool's result.

use std::rc::Rc;

use demi_host_interface::{
    Host, HostError, HostFs, HostIdentity, HostKey, HostProcess, ShellEnvironment,
};
use futures_util::future::LocalBoxFuture;

use crate::{EnvironmentScope, ShellEnvironmentFactory};

/// The Host type of a test product whose agents run no shell tools: it has
/// no values, so no environment is ever made on one.
#[derive(Debug)]
pub enum NoHost {}

impl Host for NoHost {
    fn key(&self) -> HostKey {
        match *self {}
    }

    fn default_cwd(&self) -> &str {
        match *self {}
    }

    fn identity(&self) -> HostIdentity {
        match *self {}
    }

    fn fs(&self) -> &dyn HostFs {
        match *self {}
    }

    fn process(&self) -> &dyn HostProcess {
        match *self {}
    }
}

/// The shell environment factory of a product whose Host is [`NoHost`].
#[derive(Debug, Default)]
pub struct NoShells;

impl ShellEnvironmentFactory<NoHost> for NoShells {
    fn create<'a>(
        &'a self,
        _: EnvironmentScope<'a>,
        host: Rc<NoHost>,
    ) -> LocalBoxFuture<'a, Result<Rc<dyn ShellEnvironment>, HostError>> {
        match *host {}
    }
}

/// The value of a shell tool result's `name: value` line, such as its
/// `commandId` (`runtime.md` § Results and previews).
pub fn field<'a>(result: &'a str, name: &str) -> &'a str {
    result
        .lines()
        .find_map(|line| line.strip_prefix(name)?.strip_prefix(": "))
        .unwrap_or_else(|| panic!("the result has no {name}:\n{result}"))
}

/// The lines a shell tool result shows, each with a newline; empty when it
/// shows none. An unfinished line may repeat on successive looks: these
/// are displayed lines, not a reconstruction of the command's raw bytes.
/// A running command's lines after it, its newest output's line and its
/// next step, are not part of it.
pub fn shown_output(result: &str) -> String {
    let Some((_, output)) = result.split_once("\noutput:\n") else {
        return String::new();
    };
    let output = output.split("\nnext: ").next().unwrap_or(output);
    let output = match output.find(" bytes not shown so far; the newest: ") {
        Some(at) => output[..at]
            .rsplit_once("\n[... ")
            .map_or("", |(shown, _)| shown),
        None => output,
    };
    format!("{output}\n")
}
