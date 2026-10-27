# Testing

Demi has no automated tests by default. A change is accepted by whoever made
it, on the running product, and a problem is diagnosed from the log.

A test is added only for a problem that came back after it was fixed, and
only when the user agrees to it. It gives the logic input at its interface
and checks its output, never its internal steps, which change too often for
such a test to hold, and it is seen to fail with the defect, planted, before
it is committed.

The commands that run the tests are in [Running](#running).

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
  ([Tests and time](../../ARCHITECTURE.md#tests-and-time)).
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

## Running

Every Rust build and test uses one Cargo selection, because each selection
keeps its own copy of the dependencies and switching between them has cost
140 seconds a time:

```sh
cargo check --workspace --all-targets
cargo test --workspace
```

Add `--test agent-session` to run the compaction tests alone, or
`--test live-view` the live view's rate control's; never
`-p <crate>`.

## Resources

A test binds port 0, owns its temporary directories and removes them, and
never reaches the internet. No automated test calls a real model: the
compaction tests use scripted providers, so a run costs nothing and answers
the same way every time.

Test support belongs to the crate that owns what is faked or observed, behind
that crate's `testing` feature, never in its production build.
