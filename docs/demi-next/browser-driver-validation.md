# Browser driver validation

Date: 2026-09-15. This is selection evidence, not the browser design or product
acceptance. [Conversation browser](browser.md) remains authoritative.

## Conclusion

Keep chromiumoxide as a CDP foundation candidate. Do not approve unmodified
chromiumoxide 0.9.1 as the complete browser driver yet.

For example, its element click returned success when an overlay received the
click instead of the requested button. It also returned success for a disabled
button that received no click. Demi must implement its specified targeting and
actionability rules before exposing these operations.

The more significant dependency issue is unbounded event buffering. A bounded
queue after a library subscription does not bound the queue inside the library.
Selection requires a verified way to bound or replace that ingress, including
an explicit overflow policy. A fast consumer alone is insufficient. Any library
change must have an identified maintenance owner; this evaluation does not
authorize a production fork or select another transport.

No production dependency, browser version pin, runner behavior, or deployed
runtime changed. The probe is an independent local Rust program. It does not
exercise a conversation Host, paired runner, Cloud guest, or workpanel.

## Reproduced environment

| Item | Value |
| --- | --- |
| Driver | `chromiumoxide = 0.9.1`, dependencies fixed in the probe's Cargo.lock |
| Browser | Full Chrome for Testing `153.0.8010.36`, headless mode |
| Platform | macOS, Apple Silicon (`mac-arm64`) |
| Rust | `rustc 1.98.1 (48a229cea 2026-09-01)` |
| Browser source | [Official versioned mac-arm64 archive](https://storage.googleapis.com/chrome-for-testing-public/153.0.8010.36/mac-arm64/chrome-mac-arm64.zip) |
| Downloaded archive SHA-256 | `1f701ef60757c63c6ccf98afaf28291dd0c8d1457d3d738e81fd62201c230ad0` |

The digest records the downloaded artifact; it is not an independently published
Google checksum. This evaluation version does not establish the product release
pin. The executable is supplied explicitly, and each run uses a fresh temporary
profile and an ephemeral loopback HTTP server.

## Observed results

| Probe | Observation | Meaning for selection |
| --- | --- | --- |
| Navigate, enter text, click | Input contained `probe@example.test`; normal button counter became 1 | Basic page operations work |
| Accessibility tree | Returned Chinese label `电子邮件`, accessible button name `提交订单`, and open Shadow DOM button name `影子按钮` | Typed CDP exposes useful semantic data; this is not a complete locator engine |
| Missing CSS target | Failed immediately, within a few milliseconds | Does not implement Demi's wait-until-deadline rule |
| Duplicate CSS target | Single-element lookup succeeded; multi-element lookup returned two | Demi must reject ambiguous semantic matches |
| Covered button | Click returned success; overlay counter became 1, button counter stayed 0 | Built-in click does not satisfy obstruction checks |
| Disabled button | Click returned success; counter stayed 0 | Built-in click does not satisfy enabled-state checks |
| Read-only DOM reads | Title and input value succeeded | Useful reads work with side-effect checking enabled |
| Read-only mutation attempts | DOM attribute write, localStorage write, fetch, sendBeacon, mutating getter, timer creation, and global increment all returned exception details | The tested expressions were rejected by Chrome, not by prompt instructions |
| Side-effect verification | Attribute absent, storage value null, counter 0, loopback effect endpoint received 0 requests | No effects from those test expressions were observed |
| PNG screenshot | Nonempty image with PNG signature | Screenshot API works; no visual-fidelity acceptance was performed |
| Screencast | Received and acknowledged three frames with 800 × 600 metadata; stop command succeeded | Typed frame events and acknowledgements work |
| Unread event burst | All 1,000 console events remained available after the consumer paused | Events accumulate rather than replacing older entries or enforcing a capacity |
| Canceled Rust wait | 50 ms timeout ended the wait; a previously dispatched 500 ms page timer still changed its counter to 99 | Dropping a future does not cancel work already dispatched to Chrome |
| Other tab after cancellation | Evaluating `1 + 1` returned 2 | This canceled wait did not close or block the other tab |
| Normal cleanup | Handler finished normally; browser process was reaped | Graceful shutdown works in this fixture |
| Injected probe failure | Error remained visible; handler finished and browser was reaped before exit | Harness cleanup runs on the failure path |
| Forced termination | Browser was reaped; handler reported WebSocket reset without closing handshake | Forced shutdown needs to classify connection loss as expected during retirement |

After the runs, no process using the probe's Chrome installation remained.
`cargo fmt --check` and `cargo clippy --locked -- -D warnings` are the source checks.

## Library boundaries confirmed from source

The installed 0.9.1 source agrees with the observed results:

- `src/element.rs`: `click` scrolls into view, calculates a point from content
  quads, and dispatches input. It does not check obstruction or enabled state.
- `src/handler/page.rs`: click consists of separate mouse-move, press, and release
  requests. Cancellation of the caller does not provide a held-input cleanup
  contract. Demi needs its own input ownership and release path.
- `src/listeners.rs`: event receivers use `UnboundedReceiver`, and listeners also
  contain a `VecDeque`. There is no subscription capacity parameter.
- `src/handler/mod.rs`: pending commands are removed on response or timeout.
  Canceling a Rust wait is not a protocol-level revocation of a dispatched action.
- `src/page.rs`: `close` invokes `Page.close`, which runs beforeunload hooks.
  Demi's unconditional tab retirement must use the appropriate typed target
  operation and verify target destruction instead of assuming this helper has
  the required behavior. That alternative was not exercised by this probe.

These are library capabilities and gaps, not reasons to duplicate functionality
the library already provides. Browser algorithms remain in `demi-commands` under
the [implementation ownership](browser.md#implementation-ownership) contract.

`Runtime.evaluate` accepts `throwOnSideEffect`; the probe sets it explicitly,
disables promise awaiting, and inspects exception details from the typed CDP
response. The successful negative tests are evidence for this mechanism on this
Chrome version, not proof covering every JavaScript expression. Production eval
still needs scoped execution contexts, strict result handling, execution limits,
and broader regression fixtures. An isolated JavaScript world alone would not
establish read-only behavior.

Sources: [chromiumoxide API](https://docs.rs/chromiumoxide/0.9.1/chromiumoxide/),
[published source](https://docs.rs/crate/chromiumoxide/0.9.1/source/src/),
[typed Evaluate parameters](https://docs.rs/chromiumoxide_cdp/0.9.1/chromiumoxide_cdp/cdp/js_protocol/runtime/struct.EvaluateParams.html).
The project offers MIT or Apache-2.0 licensing; the complete selected dependency
set still needs the normal release review.

## Reproduce

The diagnostic lives in [scripts/native/browser-probe](../../scripts/native/browser-probe/Cargo.toml).
Its standalone Cargo workspace keeps candidate dependencies out of the product.
Use the official full Chrome for Testing artifact for the machine being tested.
The program only accepts an executable path and one of three fixed scenarios.

From the repository root:

```bash
export DEMI_PROBE_CHROME='/absolute/path/to/Chrome for Testing executable'
cargo run --locked --manifest-path scripts/native/browser-probe/Cargo.toml -- "$DEMI_PROBE_CHROME" normal
cargo run --locked --manifest-path scripts/native/browser-probe/Cargo.toml -- "$DEMI_PROBE_CHROME" failure
cargo run --locked --manifest-path scripts/native/browser-probe/Cargo.toml -- "$DEMI_PROBE_CHROME" kill
cargo fmt --manifest-path scripts/native/browser-probe/Cargo.toml --check
cargo clippy --locked --manifest-path scripts/native/browser-probe/Cargo.toml -- -D warnings
```

Run these separately: `failure` deliberately exits nonzero after cleanup.
The program prints observations; exit zero means the diagnostic completed, not
that the candidate satisfies the browser contract. Compare the counters and
results with the table above. The main probe has a 45-second deadline. Cleanup
attempts graceful browser closure, falls back to process termination, joins the
event task, stops the local server, and removes the temporary profile. On probe
failure, closing the browser also stops any active capture and page timers.

## Remaining selection gates

1. Establish bounded event ingress inside the chosen driver, not only after it.
   Verify log floods and slow consumers without blocking control operations.
2. Validate a single semantic targeting and actionability implementation,
   including ambiguity, disabled/covered elements, delayed readiness, frames,
   navigation, and stale references. Raw AX data does not complete this work.
3. Verify cancellation between input press and release, interruption during
   navigation/dialogs, and unconditional tab cleanup. Do not promise rollback
   of already dispatched effects or terminate unrelated page work blindly.
4. Extend read-only tests to frame contexts, additional side-effect mechanisms,
   exceptions, unsupported results, and execution limits.
5. Complete browser/driver compatibility checks on supported platforms and the
   paired-device and Cloud paths before product acceptance. The local probe
   supplies no evidence for those paths, user-input arbitration, or bounded
   workpanel transport.

Until those gates are resolved, the driver decision in the browser design stays
open. The current evidence supports further work on this candidate, not an
unqualified adoption or a claim of Playwright-equivalent behavior.
