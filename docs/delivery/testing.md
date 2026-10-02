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
  generator produces, constants, accessors, the steps inside a function, or the
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

- **Scenarios first.** The default choice for a behavior is a scenario through
  real programs and their boundaries: the backend over HTTP and WebSocket,
  real runner processes,
  and a scripted model ([Scenarios](scenarios.md)). One scenario covers many
  modules working together and survives refactoring, since it depends on no
  internals.
- **Tables for dense logic.** Code with many cases and no side effects, such as
  parsers, validators, state machines and a vendor stream's conversion, is
  tested directly with a table of its edge cases. Reaching every case through a
  scenario would cost more than the logic is worth.
- **Contracts at their boundary.** A wire format that another program or the
  web app reads is pinned where it is encoded or decoded, with the values the
  other side depends on. A format that nothing outside the package reads is not
  pinned. When the other side's types are generated from the same
  definitions, as the web app's are
  ([Generated TypeScript](../architecture/contracts.md#generated-typescript)),
  generation carries the field names and tags, and a test that pins them
  restates the generator. Pin what generation does not carry: the fixtures both
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
- Timer logic runs inside `testing/synctest.Test`: its bubble owns the
  goroutines and advances virtual time when they are durably blocked.
  `synctest.Wait` waits for that blocked state before an assertion; it is not
  a substitute for joining work. This pins an idle deadline exactly without
  spending an hour of wall time. Real sockets, child processes and machines
  stay outside the bubble: wait for their events with a hang deadline, and
  use an injected clock for expiry where needed
  ([Tests and time](../architecture/concurrency.md#tests-and-time)). In the
  web app's packages, bun's `jest.useFakeTimers()` is that clock: it moves
  `setTimeout`, `setInterval`, `Date.now` and `performance.now`, while
  `setImmediate` stays real, so a test can still let the event loop turn
  once. Its `jest.runAllTimers()` also runs the timers that the timers set,
  with no limit, so where a timer sets the next one, as a socket's silence
  watch does for each new socket, it never returns and fills the memory: run
  the timers due now (`jest.runOnlyPendingTimers()`) or advance the clock by
  the wait instead.
- A failure without a related change is a defect in the product or the test.
  Find its cause; never rerun to get a pass, and never fix a failure with a
  longer timeout, a retry, a weaker assertion or a broader fake.
- A fix for an intermittent failure is proven by repeated clean runs: the test
  alone 20 times, and the whole suite 3 times while another build loads the
  machine.

## Cost

- A test takes about 1 second, and a package test run or TypeScript file about
  10 seconds. A test that needs more states why in a comment: which contract
  it proves that no cheaper test can.
- Soaks, benchmarks and long waits stay out of the regular suite.
- A commit that adds or changes tests states their measured time.

## Resources and placement

- A test binds port 0, owns its temporary directories and removes them, and
  never reaches the internet: it uses local servers and fixtures.
- No automated test calls a real model. Tests use scripted providers and
  fixtures, so a run costs nothing and answers the same way every time. Suites
  that need a real machine manager, Chrome or a vendor's CLI run only when
  environment variables supply those resources; of them the Chrome, Cloud and
  Claude Code suites exist, and release acceptance checks by hand what they
  leave out
  ([Real machine acceptance](scenarios.md#real-machine-acceptance)).
- Go tests live beside their package in `_test.go` files. Prefer the public
  boundary (`package name_test`) where it exposes the behavior. A behavior
  reachable only through private state may indicate separate responsibilities;
  split only where the [package design](../architecture/crates-and-packages.md#module-layout)
  permits it, otherwise use a same-package test.
- Go builds each package's test binary incrementally and caches successful
  results in package-list mode. There is no rule to combine unrelated tests
  into a few binaries or to keep one fixed package selection. Run the affected
  package while editing; use `-run` for one behavior and `-count=1` when an
  actual repeat is required for proof or measurement.
- A new test goes into the existing test file for the code it covers. A test
  for a fixed bug sits beside the tests of the behavior it restores and is
  named after that behavior. Use table tests when several inputs exercise the
  same behavior, with named subtests that identify the failing case.
- Tests use the standard library, `github.com/google/go-cmp` for useful value
  differences, and `go.uber.org/goleak`; no assertion library. Use
  `t.Context()` for work owned by a test. It is canceled before cleanup runs;
  cleanup must then wait for the work to end. Register resource cleanup at
  acquisition with `t.Cleanup` or `defer`, including failure paths.
- Every worker has an owner that cancels and joins it. Use
  `goleak.VerifyTestMain` after package cleanup to detect leaked goroutines;
  do not hide application workers with broad ignore lists. Leases and permits
  need explicit release and a shutdown audit where owned; garbage collection
  and a clean goroutine count do not prove their release.
- Test support belongs beside the package that owns what is faked or observed,
  in a package with a `test` suffix, such as `providertest`. Production code
  never imports it, so it is not linked into product executables.
- Suites that start real programs or machines carry `//go:build acceptance`.
  They find already-built executables in the directory named by
  `DEMI_TEST_PROGRAMS`, just as the TypeScript suites do, and never build them
  inside a test. Resource-dependent suites additionally require the variables
  in [Scenarios](scenarios.md#real-machine-acceptance). The tag alone does not
  supply Chrome, a machine manager, or the vendor CLI.

## Race detection

Run `go test -race` on macOS with `CGO_ENABLED=0`. On Linux, only the race
test binary uses `CGO_ENABLED=1 -tags netgo,osusergo`: the race runtime needs
ThreadSanitizer through libc and a C compiler there, while those tags keep
DNS resolution and user lookup in Go, as in the shipped build. Product builds
and the per-target cgo check always use `CGO_ENABLED=0`. The exact commands
are in [Validation](builds-and-releases.md#validation).

A passing race run checks the paths the tests exercised; it does not prove
atomic admission, correct cancellation, timely lease release, or absence of
deadlock. Test those observable behaviors with controlled interleavings and
cleanup checks as well.

## Coverage

- Line coverage is measured with `go test -coverprofile=coverage.out ./...`
  and inspected with `go tool cover -func=coverage.out` for Go, and with
  `bun test --coverage` for the web app's packages, and reported when a body of
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
