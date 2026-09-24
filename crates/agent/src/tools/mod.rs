//! The standard tools (`runtime.md` § Tools) and the shell environments
//! they run in: each node keeps one environment per Host it has used, which
//! the product makes.

use std::rc::Rc;

use demi_core::NodeId;
use demi_shell::{CommandSet, HostError, ShellEnvironment};
use futures_util::future::LocalBoxFuture;

/// The node an environment is made for.
#[derive(Clone, Copy)]
pub struct EnvironmentScope<'a> {
    /// The conversation's root node.
    pub root: &'a NodeId,
    pub node: &'a NodeId,
    /// The commands the environment's shells offer: the node's.
    pub commands: &'a Rc<CommandSet>,
}

/// Where a node's shell environments come from. The product makes the
/// environment of one node on one of its Hosts, such as host-remote's over
/// the Host's runner; the agent never knows which shell engine runs.
pub trait ShellEnvironmentFactory<H> {
    fn create<'a>(
        &'a self,
        scope: EnvironmentScope<'a>,
        host: Rc<H>,
    ) -> LocalBoxFuture<'a, Result<Rc<dyn ShellEnvironment>, HostError>>;
}
