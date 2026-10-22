//! A Host type for agents that run no shell tools, with its shell
//! environment factory (feature `testing`).

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
