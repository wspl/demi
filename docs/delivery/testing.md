# Testing

A test is worth its cost when it would fail for a change a user or another
component would notice, and for no other change.

For example, the pairing scenario starts a real backend and a real runner,
pairs the runner with a code and runs a command on it. It fails in these
cases:

- The device token is malformed.
- The runner cannot connect.
- The backend loses the device.

A unit test that asserts the token is 64 hexadecimal characters adds nothing to
that scenario. It cannot fail unless the scenario fails too, and it would break
if the token format changed for a good reason.

This document says what a test protects, where it runs, how it proves itself,
and what it can cost. The commands that run the tests are in
`crates/xtask/src/main.rs`, and the backend's scenario suites in
`crates/backend/tests/backend/support.rs`.

## What a test protects

Each test follows these rules:

- Each test protects one behavior that a user or another component relies on,
  or one bug that was fixed. It checks that behavior at the boundary where it
  is observable: an API scenario, a wire format, a state machine, a program's
  output and exit status.
- A test covers a behavior once, at one level, and not again in every layer
  beneath it.
- A test never restates the implementation: serialization that a derive
  produces, constants, accessors, the steps inside a function, or the shape of
  a value no other component reads.
- A test never checks what cannot fail, such as asserting that output contains
  no panic.
- A test never asserts a defect as correct behavior to prove something else.
  For example, to show that a cache hit does not read the cached file, do not
  plant corrupt bytes and expect them to be accepted. Show a performance
  property by measurement, not by a test.

  A fixed bug of complexity, work that grew as the product of two sizes, is the
  exception: a test can count the work deterministically, such as the reads of
  a reactive list, and bound it by the sizes, never by time.
- A test of a helper is not proof that the product works, because the helper's
  test passes even when no entry point calls the helper. Prove the behavior
  through the entry point that users reach.

## Levels

Tests run at the following levels:

- **Scenarios first.** The default test runs the real programs through their
  real boundaries: the backend over HTTP and WebSocket, real runner processes,
  and a scripted model (`crates/backend/tests/backend/support.rs`). One
  scenario covers many modules working together and survives refactoring,
  because it depends on no internals.
- **Tables for dense logic.** Test code with many cases and no side effects,
  such as parsers, validators, state machines and a vendor stream's
  conversion, directly with a table of its edge cases. Reaching every case
  through a scenario would cost more than the logic is worth.
- **Contracts at their boundary.** A test pins a contract as follows:
  - Pin a wire format that another program or the web app reads where the code
    encodes or decodes it, with the values the other side depends on.
  - Do not pin a format that nothing outside the crate reads.
  - When the other side's types are generated from the same definitions, as the
    web app's are
    ([Generated TypeScript](../../crates/xtask/src/contracts/mod.rs)),
    generation carries the field names and tags, and a test that pins them
    restates the derive. Pin what generation does not carry:
    - The fixtures both sides decode.
    - The rules the generator translates (strict or tolerant, optional or
      nullable, bounds).
    - Checks that only one side makes.
    - Stored formats.

## Proof

A test proves itself in these ways:

- A test for a fixed bug fails on the code before the fix. Check it by running
  the test at the parent commit, or by reverting the fix, before you commit.
- A test for a new behavior fails when that behavior is broken: plant the
  defect it guards against, see the test fail, then remove the defect.
- A test that has never failed for a real reason is suspect, because it might
  test nothing that can break.

## Time and stability

Tests handle time and stability by these rules:

- A test never waits for time to pass. It waits for the event itself, or polls
  for a condition with a deadline that only guards against a hang. A fixed
  window that the event must fall into fails under load and wastes its length
  when the event comes early.
- A test that shows that something does not happen before a window ends,
  such as a Cloud that must not stop before its idle window has passed, waits
  for the event that ends the window (the stop), with a deadline that guards
  against a hang, and asserts that it came no earlier than the window permits.
  It does not sleep across the window and then look.
- Timer logic runs on a paused or injected clock where the code permits it
  ([Tests and time](../../ARCHITECTURE.md#tests-and-time)). In the web app's
  packages, bun's
  `jest.useFakeTimers()` is that clock:
  - It moves `setTimeout`, `setInterval`, `Date.now` and `performance.now`,
    while `setImmediate` stays real, so a test can still let the event loop
    turn once.
  - Its `jest.runAllTimers()` also runs the timers that the timers set, with no
    limit. Where a timer sets the next one, as a socket's silence watch does
    for each new socket, it never returns and fills the memory. Run the timers
    due now (`jest.runOnlyPendingTimers()`) or advance the clock by the wait
    instead.
- A failure without a related change is a defect in the product or the test.
  Find its cause. Never rerun to get a pass, and never fix a failure with a
  longer timeout, a retry, a weaker assertion or a broader fake.
- An intermittent failure is a race against time, so its fix removes the race
  rather than outlasting it. The test drives the clock the code reads, or waits
  for the event, and its outcome then no longer depends on how busy the machine
  is. The fix's reasoning and a planted defect that fails the test prove a fix
  of that kind.

## Cost

A test has the following budget:

- A test takes about 1 second, and a test binary or file about 10 seconds. A
  test that needs more states why in a comment: which contract it proves that
  no cheaper test can.
- Soaks, benchmarks and long waits stay out of the regular suite.

## Resources and placement

Tests use resources and live in places by these rules:

- A test binds port 0, owns its temporary directories and removes them, and
  never reaches the internet: it uses local servers and fixtures.
- No automated test calls a real model. Tests use scripted providers and
  fixtures, so a run costs nothing and answers the same way every time.
- Suites that need a real machine manager, Chrome or a vendor's CLI run only
  when environment variables supply those resources. Of them the Chrome, Cloud
  and Claude Code suites exist, and release acceptance checks by hand what they
  leave out (`crates/backend/tests/backend/real_cloud.rs`).
- Each crate has one test binary, which tests its public boundary
  ([Test binaries](../../ARCHITECTURE.md#test-binaries)). A test gets a binary
  of its own only when it changes or exhausts process-wide state, such as the
  open-file limit.
- A behavior that a test can reach only through a crate's private items might
  mean that the crate holds two parts. Split the crate only where
  [the reasons to split a crate](../../ARCHITECTURE.md#when-to-split) permit;
  otherwise the crate keeps its tests as unit tests.
- Cargo starts every test binary through the runner in `.cargo/config.toml`,
  which raises its soft open-file limit to the hard limit (on macOS to
  `kern.maxfilesperproc`). A shell's low default, such as macOS's 256, needs no
  `ulimit`. A test that needs a lower limit sets it itself.
- A new test goes into the existing test file for the code it covers. A test
  for a fixed bug sits beside the tests of the behavior it restores and is
  named after that behavior.
- Test support belongs to the crate that owns what is faked or observed, behind
  that crate's `testing` feature, never in its production build.

## Coverage

Coverage follows these rules:

- Measure line coverage with `cargo llvm-cov` for the Rust workspace and
  `bun test --coverage` for the web app's packages, and report it when you add
  or review a body of tests.
- Add a new test only when it either covers code that no test reached or adds
  an edge case that no other test holds.
- Coverage guides which tests to keep. It is not a target. Code that a test
  runs without asserting its result is not proven.

## Review checklist

Ask these questions before you commit a test, and when you review one:

1. Which behavior or fixed bug does it protect, and who relies on it?
2. At which boundary does it observe the behavior, and does a scenario already
   cover it?
3. Did it fail before the fix, or with the planted defect?
4. How long does it take, and what does it wait on?
5. What does it leave behind: ports, processes, files?
