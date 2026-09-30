//! Test support (feature `testing`): a number source for services that draw
//! no conversation numbers. The feature also builds `demi-native-fixture`,
//! the command program with fixture operations that tests install and start.

use demi_command_service::protocol::ServiceSequence;
use futures_util::future::BoxFuture;

use crate::NumberSource;

/// Refuses every request, as a backend does for a conversation that is not
/// the device's.
pub struct NoNumbers;

impl NumberSource for NoNumbers {
    fn reserve(&self, _: String, _: ServiceSequence, _: u32) -> BoxFuture<'_, Result<u64, String>> {
        Box::pin(async { Err("this test gives out no conversation numbers".to_owned()) })
    }
}
