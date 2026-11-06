//! What Demi's releases build in: the provider families and the plugins
//! that the server and a program's process start with alike.

mod families;
mod plugins;

pub use families::{RelayFamily, families};
pub use plugins::plugins;
