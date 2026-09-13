# Demi Next: Acceptance Ledger

This ledger records verified behavior of the native implementation. Earlier runner
and Cloud acceptance results are historical evidence in
[the September 8 ledger](../experiments/acceptance-2026-09-08.md); they do not prove
acceptance of a newly built runner.

## Verified locally

- All 66 Rust workspace tests pass on macOS arm64 and a native Linux arm64
  container with an init process to reap orphaned test descendants, including protocol framing,
  resident service concurrency, cancellation, verified artifacts, file commands,
  runner connections, local forwarding, shell pipelines and Host IO.
- TypeScript and all three frontend typechecks pass. The final application suite
  ran 1,179 tests: 1,157 passed, 22 conditionally skipped and zero failed.
  Package builds and the public-registry configuration check pass.
- Provider CLI integration uses a mock upstream. Host conformance and explicit
  device-environment inheritance tests pass. No real model calls are used.
- Runner release tests reject corrupt immutable artifacts, preserve the current
  manifest on failure and clean temporary publication files.
- Linux-container cross-compilation has produced all six runner and command-service
  artifacts. ELF inspection confirms both Linux executables on both architectures
  have no interpreter or shared dependencies. Windows imports are OS libraries,
  with no dynamic MSVC CRT dependency. Compilation is not target runtime acceptance.

## Platform execution gates

`.github/workflows/native.yml` runs the shared Rust workspace suite on macOS,
Linux and Windows, each on arm64 and x64. Linux tests use musl targets and final
binaries are inspected for ELF interpreter/dynamic-library dependencies. Windows
builds use static CRT. A workflow definition is not a passed execution result;
record the workflow run and exact commit when results are available.

[Native workflow run 34733491545](https://github.com/wspl/demi/actions/runs/34733491545)
on commit `3be21941` passed all six native platform jobs and the Rust formatting
and Clippy gate. Each platform executed the Rust workspace suite, installer and
immutable publication fixtures, and built both release executables. Both Linux
jobs also passed the static-linkage inspection. Installer and publication fixtures
each passed three consecutive executions per platform.
[Application workflow run 34734045186](https://github.com/wspl/demi/actions/runs/34734045186)
on commit `7a116340` passed typechecks, application tests, package and frontend
builds, and generated-documentation validation.

Earlier Windows arm64 installer fixture runs intermittently exceeded their test
deadline. The fixture now bounds HTTP requests, records installer progress and
process output, and checks retained artifacts with HEAD without leaving a binary
response body unread. Windows arm64 passed another three installer executions in
[diagnostic run 34734045193](https://github.com/wspl/demi/actions/runs/34734045193)
on commit `7a116340`. The earlier timeout's root cause remains unconfirmed;
successful reruns do not establish that a specific change eliminated it.

The Intel Mac release passed 17 Host, file-command and installer tests under
Rosetta on the local arm64 Mac; this is translated execution.

The native Linux arm64 runner passed the real Firecracker scenario for guest
startup, uid 1000 file/job identity, persistent system/home across shutdown and
wake, and external reset with home retained. The fixture used the repository kernel
and an Ubuntu base image containing the new runner. The toolchain scenario verified
rustup installation followed by `cargo` in the next job, and nvm installation
followed by its Node executable in the next job. Both complete scenarios passed
with the native pipeline fixes. The new native runner also passed the complete
persistence/reset scenario under jailer with its TAP owned by the configured
slot UID: 10 assertions, including three separate VM starts. Direct and jailer
tests used scripted providers without real model calls.

[Synthetic transport measurements](../native-transport-measurements.md) record
release-build latency, throughput, slow-output isolation, cancellation through
process reap and Linux RSS. They do not measure actual file-command performance.

## Product boundaries

The native change leaves backend ownership, scoped journals, device selection,
Cloud lifecycle and shared web components with their existing responsible
packages. The backend on port 3271, product frontend on 18922 and gallery on 18944
were restarted. The backend installer endpoint `/install.sh` and both frontend
root pages returned HTTP 200. The connected macOS runner uses the native
release `4e0cc15a4c68a5269237dd078c862c01d8cdb0ad55d428c0a89016f1172dea03`
and reports online with its existing registration. The local backend uses
an explicit development assembly with verified local artifacts; production
S3/OSS deployment configuration has not been supplied. Implementation contracts are in `native-runtime.md`,
`runner.md`, `commands.md` and `package-boundaries.md`.
