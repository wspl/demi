//! The integration tests of the artifact cache and the service registry, in
//! one process. They start `demi-native-fixture`, this crate's fixture
//! program.

mod artifact_cache;
mod load;
mod services;

// Each test's whole-body timeout is a hang guard of 60 s, not a latency
// check: the tests here run together, and a tighter bound fails them on a
// loaded machine.
