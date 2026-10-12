//! Test support (feature `testing`): a backend for services that ask it
//! nothing. The feature also builds `demi-native-fixture`, the command
//! program with fixture operations that tests install and start.

use demi_command_protocol::ServiceSequence;
use futures_util::future::BoxFuture;

use crate::ServiceBackend;

/// Refuses every request, as a backend does for a conversation that is not
/// the device's, and hears no loss.
pub struct NoBackend;

impl ServiceBackend for NoBackend {
    fn reserve(&self, _: String, _: ServiceSequence, _: u32) -> BoxFuture<'_, Result<u64, String>> {
        Box::pin(async { Err("this test gives out no conversation numbers".to_owned()) })
    }

    fn tell(&self, _: String, _: String, _: serde_json::Value) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async { Err("this test takes no plugin messages".to_owned()) })
    }

    fn lost(&self, _: String) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}
