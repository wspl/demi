# Demi Next: Implementation Log

Live status per roadmap milestone of `docs/demi-next.md`. Updated as the work
happens — status, pitfalls, and conclusions land here so the effort can be
resumed and reviewed at any point.

Working branch: `feat/demi-next` (all implementation happens on this single
branch, renamed from the design branch `feat/chat-gui-gateway-design`).

## M0 — Groundwork

Status: **done** (2026-08-31). All three items landed and verified with
scoped `bun test` runs; repo-wide `bun run typecheck` green.

### Pre-flight (done)

- Verified the design record's cited code facts against the repo before
  starting; all hold (claude-code hard-coded `child_process.spawn` +
  `process.env`, `createLogicalHostCwd`, host-routing tests, `authStore?`
  injection points, no existing remote-Host code anywhere).
- ~~Committed the dangling `just-bash` submodule pointer (`d9d2c82`)~~ —
  **wrong, reverted** (`d75be59`): the dirty state was the working tree
  *lagging* at an older commit, not a new change; shell needs `7ed8adb`
  (hostCwd dirfd + spawn-error classification). Lesson: compare commit
  dates/ancestry before accepting a dirty submodule pointer.

### Pitfalls hit while landing M0 (all fixed on this branch)

- **Baseline typecheck was red** (pre-existing): grok-build narrowing +
  `HeadersInit`; fixed in `da997c1`. `bun run typecheck` is green again —
  keep it green.
- **just-bash needs a local build**: the workspace links
  `packages/just-bash/packages/just-bash` whose `dist/` is gitignored; after
  a submodule update run `bun install && bun run build && bun run build:worker`
  inside `packages/just-bash` or shell/agent tests fail en masse.
- **macOS cwd dirfd anchor was broken** (pre-existing, hit all real-spawn
  shell tests on macOS): `/dev/fd/N` cannot be opened or traversed for
  directory fds (ENOTDIR/ENOENT, Bun 1.3.11); the anchor is now Linux-only,
  darwin uses logical path semantics, and a failed spawn's stdio no longer
  throws `ERR_STREAM_PREMATURE_CLOSE` through `Host` streams (`714b440`).
  Consequence for the runner protocol: dirfd-stable cwd is a Linux-only
  nicety; the proxy Host's `createLogicalHostCwd` plan is unaffected.

### Item 1 — claude-code injectable spawn + env overlay

Status: **done** (`a43a744`). Tests: `transport.test.ts` (spawn injection,
env cleanliness, stream-json round trip, local wrapper smoke),
`cli.test.ts` (overlay precedence), utils `utf8Lines`.

Decisions:

- `@demicodes/provider-claude-code` must not import `@demicodes/shell`
  (package boundary), so the injectable spawn is typed **structurally** in
  `transport.ts` as a subset of `Host.process.spawn`
  (`ClaudeSpawnParams`/`ClaudeSpawnHandle`) — a real `host.process.spawn` is
  directly assignable.
- One transport implementation over the spawn-handle shape; the local
  default wraps `child_process.spawn` into the same shape (no second code
  path).
- `buildClaudeEnv` gains a public `overlay` applied last (backend sets
  `ANTHROPIC_BASE_URL` + `CLAUDE_CODE_OAUTH_TOKEN`; overlay wins over the
  authStore-resolved token).
- With an injected spawn the env base is **empty** (never leak the backend's
  `process.env` across the wire); the spawn implementation owns merging with
  the target machine's environment. The local default keeps `process.env` as
  base, unchanged.
- Local-only `resolveSpawnCwd` (statSync fallback for virtual cwds) stays in
  the local wrapper; injected spawns receive the request cwd as-is.
- Per-session spawn routing (which runner a given session's CLI lands on) is
  deliberately **not** solved here — the option is provider-level; M3's
  backend owns routing.

### Item 2 — provider execution-requirement capability flag

Status: **done** (`a8269d0`). `Provider.requiresProcessCapableHost?: boolean`
— a flat optional flag, no grouping structure (nothing else to group yet).
claude-code declares it; enforcement (virtual target refusal with upgrade
guidance) lands with `@demicodes/host-virtual` in M1.

### Item 3 — host-switch integration test (migration primitive)

Status: **done** (`734ae4f`,
`packages/agent/src/__tests__/host-switch-migration.test.ts`).

Conclusions for M4's product implementation:

- The context-block injection point is the **harness `preamble` hook**
  (`AgentHarness.preamble(ctx)` with `ctx.metadata`) — it lands on the user
  block (`Block.preamble`) and is replayed to the model prepended to the
  user turn. No new agent mechanism is needed for switch announcements.
- Per-Host `BashEnvironment` reuse verified across an excursion: A → B → A
  returns to A's shell with cwd and files intact; B starts fresh.
- The harness `host()` hook is also called in metadata-less contexts
  (session open); a product harness must fall back to the session's current
  target there rather than requiring action metadata.

### Other conclusions from landing M0

- The root-entry boundary test (`platform-entrypoints.test.ts`) rejects
  root exports from claude-code's `./transport`; the public injectable
  spawn shape types therefore live in their own `./spawn` module
  (`2059dab`) — transport stays internal.

## M1 — Backend skeleton + virtual default (A) / Runner protocol core (B)

Status: **in progress** (started 2026-08-31). No parallel work — tracks run
sequentially, B first per the risk-first ordering principle (the only
greenfield contract).

### Track B — runner protocol core

Status: **done** (`58bbfaf`, `02154de`).

What landed:

- `@demicodes/runner-protocol` (platform-neutral, certified by the
  entrypoints boundary test): wire messages, portable frame codec,
  backend-side `RemoteHost`, runner-side `HostRpcServer`.
- `@demicodes/runner`: `RunnerClient` (outbound WS, reconnect/backoff,
  claim handshake client, ping/pong), `RunnerState` (`runner.json` +
  `runner-token` 0600), `demi-runner` CLI entry.
- M1-B acceptance test passes end-to-end over a real WebSocket: real
  cat/tee/spawn on the runner, mid-command runner death → ordinary tool
  error + session continues, auto-reconnect → next command succeeds.

Wire decisions beyond the design table (the table said the op set is fixed
when the package lands):

- `hello` carries a `protocol` version integer — the backend must be able
  to tell an incompatible runner from a broken one.
- `spawn_stdin_end` added (wire form of `closeStdin`).
- fs errors carry `{message, code}` and the proxy rethrows with `code`
  intact — errno codes (ENOENT…) drive shell behavior and must survive the
  wire.
- Trailing `undefined` fs args are trimmed so the wire carries real arity.

Pitfalls:

- **`Buffer.toJSON` broke the portable codec** (pre-existing): `toJSON`
  runs before the `JSON.stringify` replacer, so Buffers (and Dates)
  arrived as plain objects. Fixed in utils by recovering originals from
  the replacer's holder (`this[key]`). Same trick was needed for the new
  Date support.
- The platform-entrypoints test scans source *text* — even a comment
  containing the word "Buffer" fails a platform-neutral package.
- Long-running silent commands emit no `shell_output` frames while
  running, so tests can't use events to detect "command started";
  `RemoteHost.activeSpawnCount` was added as the honest diagnostic.

### Track A — backend skeleton + virtual default

Status: in progress.
