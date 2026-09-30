//! The integration tests of the runner's processes and their IO, in one
//! process.

mod lines;
mod pipes;
mod process;

// Each test's whole-body timeout is a hang guard of 60 s, not a latency
// check: the tests here run together, and a tighter bound fails them on a
// loaded machine.
