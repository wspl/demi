# Testing

The normal regression suite uses scripted providers, injected HTTP responses,
local mock servers, synthetic credentials and temporary state directories.
It never requires a provider account. Native runner tests execute locally built
binaries; they do not invoke models.

## Commands

Run from the repository root after `bun install --frozen-lockfile`:

```sh
bun run typecheck
bun run typecheck:web
bun run test
```

`bun run test` prepares the native runner and runs every workspace test directory
plus the repository-tool tests. It sets `DEMI_TEST_MODE=offline` for the test
process. `scripts/test-policy.ts` makes this mode override inherited external
service opt-ins. CI uses this command.

Native preparation requires CMake, a C/C++ compiler and the pinned recursive
`vendor/txiki.js` submodule. `CMAKE` may name an installed CMake executable.
Browser typechecks resolve source through the packages' `development` export
conditions; generated declaration files are unnecessary.

For a focused regression, use an explicit repository-relative path, for example:

```sh
DEMI_TEST_MODE=offline bun test --conditions development ./packages/agent/src/__tests__/tool-contracts.test.ts
```

Use the leading `./` so Bun treats the argument as a path. Native integration
suites additionally require `packages/runner/runtime/prepare-tests.ts` to run on
the current source before the test invocation. Linux-only image and jailer tests
report skips when their required platform tools are unavailable.

## External tests

The Codex and Claude Code `real-*.e2e.test.ts` suites retain their per-scenario
`DEMI_*_E2E=1` opt-ins. The real Firecracker suite has its own opt-in. An explicit
flag enables its selected test only outside offline mode. These tests are not
part of data-contract acceptance and must not be run for repository work that
prohibits real model calls.

The backend `claude-chain.e2e.test.ts` suite is different: it uses an installed
Claude CLI with a loopback mock upstream, a synthetic token and an isolated
configuration directory. It does not contact a model endpoint and skips when
that CLI is unavailable.

## Boundary verification

`platform-entrypoints.test.ts` discovers workspaces from `package.json`.
`scripts/source-audit.ts` parses production TypeScript, JavaScript and Vue script
blocks, including arrow functions. Tests, fixture programs and declarations are
excluded from production dependency checks. Parser errors fail the audit.

The checks cover import ownership, package dependencies, direct JSON assertions
and structural copies of shared helper implementations. They permit ordinary
type narrowing and same-named functions with different behavior. These checks
cannot establish complete runtime validation: each boundary also needs independent
valid and invalid examples, serialized round trips and cleanup/error-path tests.

Provider contract tests use malformed synthetic secrets to check that diagnostic
messages identify fields without revealing values. UI changes require both product
and gallery verification, using their shared `web-ui` implementation.
