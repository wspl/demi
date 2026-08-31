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

## M1+

Not started.
