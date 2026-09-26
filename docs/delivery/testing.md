# Testing

A test is worth its cost when it would fail for a change a user or another
component would notice, and for no other change. For example, the pairing
scenario starts a real backend and a real runner, pairs the runner with a code
and runs a command on it: it fails if the device token is malformed, if the
runner cannot connect, or if the backend loses the device. A unit test that
asserts the token is 64 hexadecimal characters adds nothing to that scenario:
it cannot fail unless the scenario fails too, and it would break if the token
format changed for a good reason. This document says what a test protects,
where it runs, how it proves itself, and what it may cost. The commands that
run the tests are in [Validation](builds-and-releases.md#validation), and the
backend's scenario suites in [Scenarios](scenarios.md).

## What a test protects

- Each test protects one behavior that a user or another component relies on,
  or one bug that was fixed. It checks that behavior at the boundary where it
  is observable: an API scenario, a wire format, a state machine, a program's
  output and exit status.
- A behavior is tested once, at one level, not again in every layer beneath
  it.
- Do not write tests that restate the implementation: serialization that a
  derive produces, constants, accessors, the steps inside a function, or the
  shape of a value no other component reads.
- Do not write checks that cannot fail, such as asserting that output contains
  no panic.
- A test never asserts a defect as correct behavior to prove something else.
  For example, to show that a cache hit does not read the cached file, do not
  plant corrupt bytes and expect them to be accepted; a performance property
  is shown by measurement, not by a test.
- A test of a helper is not proof that the product works: the helper's test
  passes even when no entry point calls the helper. Prove the behavior through
  the entry point that users reach.

## Levels

- **Scenarios first.** The default test runs the real programs through their
  real boundaries: the backend over HTTP and WebSocket, real runner processes,
  and a scripted model ([Scenarios](scenarios.md)). One scenario covers many
  modules working together and survives refactoring, since it depends on no
  internals.
- **Tables for dense logic.** Code with many cases and no side effects, such as
  parsers, validators, state machines and a vendor stream's conversion, is
  tested directly with a table of its edge cases. Reaching every case through a
  scenario would cost more than the logic is worth.
- **Contracts at their boundary.** A wire format that another program or the
  browser reads is pinned where it is encoded or decoded, with the values the
  other side depends on. A format that nothing outside the crate reads is not
  pinned. When the other side's types are generated from the same
  definitions, as the browser's are
  ([Generated TypeScript](../architecture/contracts.md#generated-typescript)),
  generation carries the field names and tags, and a test that pins them
  restates the derive. Pin what generation does not carry: the fixtures both
  sides decode, the rules the generator translates (strict or tolerant,
  optional or nullable, bounds), checks that only one side makes, and stored
  formats.

## Proof

- A test for a fixed bug fails on the code before the fix. Check it by running
  the test at the parent commit, or by reverting the fix, before you commit.
- A test for a new behavior fails when that behavior is broken: plant the
  defect it guards against, see the test fail, then remove the defect.
- A test that has never failed for a real reason is suspect: it may test
  nothing that can break.

## Time and stability

- A test never waits for time to pass. It waits for the event itself, or polls
  for a condition with a deadline that only guards against a hang. A fixed
  window that the event must fall into fails under load and wastes its length
  when the event comes early.
- A test that shows that something does not happen before a window ends,
  such as a Cloud that must not stop before its idle window has passed, waits
  for the event that ends the window (the stop), with a deadline that guards
  against a hang, and asserts that it came no earlier than the window allows.
  It does not sleep across the window and then look.
- Timer logic runs on a paused or injected clock where the code allows it
  ([Tests and time](../architecture/concurrency.md#tests-and-time)).
- A failure without a related change is a defect in the product or the test.
  Find its cause; never rerun to get a pass, and never fix a failure with a
  longer timeout, a retry, a weaker assertion or a broader fake.
- A fix for an intermittent failure is proven by repeated clean runs: the test
  alone 20 times, and the whole suite 3 times while another build loads the
  machine.

## Cost

- A test takes about 1 second, and a test binary or file about 10 seconds. A
  test that needs more states why in a comment: which contract it proves that
  no cheaper test can.
- Soaks, benchmarks and long waits stay out of the regular suite.
- A commit that adds or changes tests states their measured time.

## Resources and placement

- A test binds port 0, owns its temporary directories and removes them, and
  never reaches the internet: it uses local servers and fixtures.
- No automated test calls a real model. Tests use scripted providers and
  fixtures, so a run costs nothing and answers the same way every time. Suites
  that need a real machine manager, Chrome or a vendor's CLI run only when
  environment variables supply those resources
  ([Real machine acceptance](scenarios.md#real-machine-acceptance)).
- Each crate has one integration test binary
  ([Module layout](../architecture/crates-and-packages.md#module-layout)); a
  test gets a binary of its own only when it changes or exhausts process-wide
  state, such as the open-file limit.
- A new test goes into the existing test file for the code it covers. A test
  for a fixed bug sits beside the tests of the behavior it restores and is
  named after that behavior.
- Test support belongs to the crate that owns what is faked or observed, behind
  that crate's `testing` feature, never in its production build.

## Coverage

- Line coverage is measured with `cargo llvm-cov` for the Rust workspace and
  `bun test --coverage` for the browser packages, and reported when a body of
  tests is added or reviewed.
- A new test either covers code that no test reached or adds an edge case that
  no other test holds; otherwise it is not added.
- Coverage guides which tests to keep; it is not a target. Code that a test
  runs without asserting its result is not proven.

## Review checklist

Before a test is committed, and when a test is reviewed:

1. Which behavior or fixed bug does it protect, and who relies on it?
2. At which boundary does it observe the behavior, and does a scenario already
   cover it?
3. Did it fail before the fix, or with the planted defect?
4. How long does it take, and what does it wait on?
5. What does it leave behind: ports, processes, files?
