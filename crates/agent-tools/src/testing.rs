//! A device type for agents that run no shell tools, with its shell
//! environment factory (feature `testing`).

use std::rc::Rc;

use demi_device_interface::{
    Device, DeviceError, DeviceFs, DeviceIdentity, DeviceKey, DeviceProcess, ShellEnvironment,
};
use futures_util::future::LocalBoxFuture;

use crate::{EnvironmentScope, ShellEnvironmentFactory};

/// The device type of a test product whose agents run no shell tools: it has
/// no values, so no environment is ever made on one.
#[derive(Debug)]
pub enum NoDevice {}

impl Device for NoDevice {
    fn key(&self) -> DeviceKey {
        match *self {}
    }

    fn default_cwd(&self) -> &str {
        match *self {}
    }

    fn identity(&self) -> DeviceIdentity {
        match *self {}
    }

    fn fs(&self) -> &dyn DeviceFs {
        match *self {}
    }

    fn process(&self) -> &dyn DeviceProcess {
        match *self {}
    }
}

/// The shell environment factory of a product whose device is [`NoDevice`].
#[derive(Debug, Default)]
pub struct NoShells;

impl ShellEnvironmentFactory<NoDevice> for NoShells {
    fn create<'a>(
        &'a self,
        _: EnvironmentScope<'a>,
        device: Rc<NoDevice>,
    ) -> LocalBoxFuture<'a, Result<Rc<dyn ShellEnvironment>, DeviceError>> {
        match *device {}
    }
}
