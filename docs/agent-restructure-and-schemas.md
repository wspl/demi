# Agent Package Restructure & Boundary Schemas

| | |
|---|---|
| Date | 2026-09-01 |
| Status | Implemented (progress log has the commit-by-commit record) |
| Scope | `@demicodes/agent` module layout; schema-ization of the product's trust boundaries (`agent`, `runner-protocol`, `backend`); the shared-guard dedup sweep |

Two problems, one root cause. `@demicodes/agent` is 23 flat files
(7,086 lines) with three of them over 1,000 lines, and the codebase probes
untyped data with hand-rolled per-file helpers (three copies of
`errorCode`, a 40-line `isRecord` chain validating shell-output shapes).
Both come from organizing by impression instead of by design: files were
never mapped to the package's modules, and validation was never assigned a
home. This record fixes both with the already-ratified rules — Module
Layout Conventions (`docs/package-boundaries.md`) and the Data Validation
tiers (`AGENTS.md`).

## Part 1 — `@demicodes/agent` layout

### Current state

```
1289 session.ts     1200 subagent.ts    1132 server.ts      717 transcript.ts
 526 tools.ts        476 client.ts       370 provider-turn-loop.ts
 323 types.ts        234 session-store.ts                    …16 more flat files
```

The package has clear design modules — the session runtime, the
transcript, persistence, the wire protocol, the server, the subagent
supervisor, the client, the standard tools — but the directory shows none
of them. `server.ts` and `subagent.ts` additionally violate the
split-by-responsibility rule internally (each carries several).

### Target layout

Directories mirror the modules; the root keeps only entrypoints, the
public contract file, and single-file modules:

```
packages/agent/src/
  index.ts                 # root export (unchanged public surface)
  client-entry.ts          # ./client build entry
  types.ts                 # public contracts (harness, session, store) — one
                           #   responsibility: the package's type surface
  tools.ts                 # standard agent tools — a single-file module
  session/                 # the AgentSession state machine and its collaborators
    session.ts             #   AgentSession
    turn-loop.ts           #   provider-turn-loop.ts
    steer-queue.ts         #   pending-steer-queue.ts
    yield-scheduler.ts
    recovery.ts
    retry-policy.ts
    compaction.ts          #   compaction-controller.ts + compaction-support.ts
                           #   (two halves of one mechanism, merged)
    provider-stream-error.ts
  transcript/
    transcript.ts          #   TranscriptLog
    patch.ts               #   patch application (client-shared)
  store/
    session-store.ts       #   AgentSessionStore realization over HostStore
    media.ts               #   BlobStore contract + externalize/rehydrate
                           #   (split from session-store.ts: persistence
                           #   plumbing vs media representation are two
                           #   responsibilities)
  protocol/                # the wire: frame types, schemas, transports
    frames.ts
    schemas.ts             #   NEW — zod schemas for inbound frames (Part 2)
    transport.ts
    websocket-transport.ts
    stdio-transport.ts     #   ./stdio build entry (exports map updated)
  server/
    server.ts              #   AgentServer facade + options types
    binding.ts             #   AgentTransportBindingImpl — frame dispatch and
                           #   attach/detach lifecycle only
    open-session.ts        #   the session-assembly pipeline extracted from
                           #   binding.open(): store selection, restore
                           #   decision, supervisor/command-tree/tools/runtime/
                           #   LiveSession construction (closure captures
                           #   become explicit parameters)
    live-session.ts        #   LiveSession (shell environments, sink swap)
    ownership.ts           #   SessionOwnershipRegistry
    summaries.ts           #   conversation summaries + progress→frame views
  subagent/
    supervisor.ts          #   ChildSupervisor lifecycle (spawn/steer/abort/
                           #   restore/archive)
    commands.ts            #   the `demi agent` command-tree definition
  client/
    client.ts              #   AgentClient
```

Non-goals, stated deliberately:

- `session/session.ts` and `transcript/transcript.ts` are **not** split
  further: each is one cohesive responsibility (a state machine; a
  mutation journal). Long-but-single-responsibility files are legal.
- `types.ts` stays whole. Splitting the public contract file per module
  would churn every import in the workspace for zero structural gain.
- No `internal/` or `shared/` directories — anything tempted to live
  there either belongs to a module or to `@demicodes/utils`.

Mechanics to watch (verified against the repo):

- `package.json` exports `.` / `./client` / `./stdio`; tsdown entries and
  the root `tsconfig.json` `paths` mapping for `@demicodes/agent/stdio`
  must follow `stdio-transport.ts` into `protocol/`.
- `packages/core/src/__tests__/platform-entrypoints.test.ts` asserts only
  AgentServer imports AgentSession as a runtime value — path updates only,
  the assertion itself is unaffected.
- All moves are behavior-preserving; `index.ts` keeps the public surface
  byte-identical, so no other package changes except import paths inside
  `agent` itself.

## Part 2 — Boundary schemas (schema-ization)

Per the AGENTS.md Data Validation guidance: structured data arriving from
outside the process gets a zod schema next to the boundary's types (TS
types via `z.infer`); field probes of thrown values use the
`@demicodes/utils` guards; where both sides are our code, the contract
itself carries the type.

### Boundary inventory and placement

| # | Boundary | Today | Target |
|---|---|---|---|
| 1 | Browser → backend HTTP bodies | blind `c.req.json<{…}>()` casts | zod schema per route module (`backend/src/http/*.ts`), parse before use, 400 with `{code, message}` on failure |
| 2 | Browser → backend WS frames (`ClientFrame`) | `JSON.parse` + cast; frame types hand-written in `frames.ts` | `agent/src/protocol/schemas.ts` declares the client frames as zod schemas and becomes the **single source of truth**: `frames.ts` derives `ClientFrame` via `z.infer` (no parallel hand-written declaration to keep in sync). Validated once at AgentServer transport ingress (the binding); rejected frames answered with the existing `rejected`/`error` frames |
| 3 | Runner ⇄ backend protocol messages | cast + scattered field checks; message types hand-written | same single-source treatment: `runner-protocol/src/schemas.ts` declares the messages, `messages.ts` derives the types via `z.infer`; both ends validate their inbound direction (`HostRpcServer` validates backend→runner, `RemoteHost`/backend validates runner→backend) |
| 4 | Tool progress → shell-output frames | 40-line hand-rolled `isRecord` chain (`progressToShellOutput` / `isShellStreamView`) | zod schema in `agent/src/server/summaries.ts` — the tool-progress channel is legitimately `unknown` (tools are arbitrary), so this is a real boundary, just currently validated by hand |
| 5 | Server → browser frames | none | **none, by design** — this process constructed them |
| 6 | Persisted rows read back (`control.sqlite`, conversation DBs, host_store) | typed cast | **cast stays, by design** — single-writer own data; corruption fails loudly, never normalized |
| 7 | Provider HTTP responses | provider-kit wire mapping | out of scope here — provider kits own their wire; revisit per-provider if their hand mapping grows validation chains |

Notes:

- The browser-side `AgentClient` does **not** validate server frames: the
  server is the same product's authoritative peer; a malformed frame is a
  server bug that should surface as one.
- Portable-codec payloads (`Uint8Array` in fs results) validate with
  `z.instanceof(Uint8Array)` after codec decode — schema order is
  decode-then-validate, never validate the raw JSON envelope.
- Zod stays out of `@demicodes/utils` (the shared guards must remain
  dependency-free); schemas live beside their boundary's frame types.
- Schema-ization must never produce two declarations of one shape: where a
  hand-written type exists today, the schema replaces it as the source of
  truth and the type becomes `z.infer`. `ServerFrame` (outbound, unvalidated
  by design) keeps its hand-written type — no schema, so no duplication.

### Guard dedup sweep (tier 1)

- `agent/src/session.ts` `providerErrorCode` and `agent/src/server.ts`
  `errorCode` → `@demicodes/utils` `errorCode` (null/undefined difference
  absorbed at call sites).
- Repo-wide grep for further private `(error: unknown)` probes; genuinely
  generic ones move to `utils/errors.ts`, domain-specific ones
  (`isUnauthorized` in provider-codex, `isContextLengthExceeded`) stay in
  their owning module — they encode domain knowledge, not structure
  probing.

## Execution plan

Ordered so every commit is green and mechanically reviewable:

1. **Dedup sweep** — replace the guard clones, one commit.
2. **Agent moves** — create the directories, move files, fix imports and
   the three build-entry registrations. Pure `git mv` + import rewrites;
   no logic edits. One commit.
3. **`server/` internal split** — carve `server.ts` into the six files
   above; `open-session.ts` converts closure captures to parameters but
   preserves construction order exactly. One commit.
4. **`subagent/` split** — supervisor vs command tree. One commit.
5. **Schemas** — boundaries #4 (with the server split fresh), then #2,
   then #1, then #3; one commit per boundary, each adding
   rejection-path tests (malformed body → 400; malformed frame →
   `rejected`; malformed protocol message → connection-level error).
6. **Registry update** — `docs/package-boundaries.md` gains the agent
   Layout section mirroring Part 1; progress log records completion.

Verification: repo-wide typecheck plus the existing suites
(agent 248, backend 10, runner/runner-protocol/coding-agent sweeps) stay
green after every commit; step 5 adds the new negative-path tests.
