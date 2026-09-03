# Demi Next: Scenario Suite

| | |
|---|---|
| Date | 2026-09-03 |
| Status | Delivered (M10) |
| Scope | The integration suite over the headless system: what it covers, the world fixture, the driver, the invariants, the scenarios |

## What the suite is for

Every layer of the headless system has a suite of its own: the Host
conformance cases, the tinybash corpus, the tinyjs primitives, the runner
protocol codecs, the pairing state machine, the provider assembly. Each
proves its contract once. What none of them proves is the **composition**:
that a tool call the model makes in a conversation created over the Web API
reaches the target, runs, and comes back as the text the model reads on its
next turn, while the transcript, the databases, the ledger and the runner
sockets all show what they should. The scenario suite is that proof, and it
is where every later milestone adds its end-to-end cases instead of
building a fixture of its own.

```
 Web API ─▶ backend ─▶ agent loop ─▶ shell tool ─┬─▶ hostless engine (tinybash + virtual fs)
                                                 └─▶ runner (packed tinyjs, real socket) ─▶ bash and commands on the target
    ◀── the text the model receives ◀── the view ◀──┘
    ◀── transcript (live) / conversations/<id>.sqlite (cold) / ledger / blobs
```

### Covered

The path in the diagram, driven from the outside: conversations over the
HTTP and WebSocket surface, the model as a script, the hostless engine and
a real runner as the two targets, the commands under `demi` on both,
subagents, target switching as a step inside a longer script, attachments
and media by reference, detach and reattach, and the restarts of both
processes.

### Not covered

The provider wire (the model is scripted at the provider-event level, as
in every other suite; the real-CLI chain in `claude-chain.e2e.test.ts`
keeps its own gate), the tinybash grammar and builtins, the tinyjs
primitives, the Host conformance cases, the login surface — setup, the
cookie session and its sliding expiry over a test clock, the gate, the
lockout, the own password (`auth.test.ts`), the admin surface — accounts
by rank, resets down the ranks, the read-only mode (`admin.test.ts`),
the instance mode — who configures providers, the connection scope on
listings, catalog, selection and usage, the mode fixed once providers
exist (`mode.test.ts`) — the pairing state machine
(`runner.test.ts`), vault and provider assembly (`llm.test.ts`), the M6
switch acceptance (`switch.test.ts`), the home-image store and the image
tools (`home-image.test.ts`; the e2fsprogs round trip runs where the tools
are — the Linux fixture — and skips elsewhere), the guest init plan
(`packages/runner`'s `init.test.ts`), the Firecracker provisioner's pure
parts — slots, the kernel command line, the configuration from the
environment (`firecracker.test.ts`) — the browser. The Firecracker smoke
(`real-firecracker.e2e.test.ts`, gated by `DEMI_FIRECRACKER_E2E=1`, on a
Linux machine with `/dev/kvm` after the install script, the Lima `fc`
instance in development) is a world over the real provisioner: a hostless
conversation upgraded to a guest, `sudo` into the upper, hibernate with
the shrunk image in the store, wake over the same home with the upper
gone, growth past the reserve, destroy on archive; it prints the
cold-provision and wake latency.

## The world

One fixture per test file; one conversation per scenario.

```
 world = createWorld({ runners: 1 | 2, port?: fixed })
 ┌──────────────────────────────────────────────────────────────┐
 │ backend (temp dataDir, the `stub` provider type, wire trace)   │
 │   ├─ device A ◀── tinyjs runner A (stateDir A, home A, workspace A) │
 │   └─ device B ◀── tinyjs runner B                              │
 │ world.conversation(target) ─▶ driver                          │
 │   target: 'hostless' | 'runner:A' | 'runner:B'                │
 └──────────────────────────────────────────────────────────────┘
```

- **backend**: `createBackend` over a fresh data directory, the port
  ephemeral except in the restart file, where it is fixed so a runner can
  find the restarted process. The registry's `trace` hook records every
  runner frame with its device and direction.
- **runners**: `startTinyjsRunner` from `@demicodes/runner/testing`, the
  packed binary built once per test process. The world claims each pairing
  code over `POST /api/devices/claim` and creates a workspace at the
  runner's home over `POST /api/workspaces`.
- **model**: the `stub` provider type is registered with a runtime factory
  that looks up the conversation's script queue. The queue is filled by the
  driver one turn at a time, so a scenario reads linearly: script the turn,
  send, observe. A turn's script is either a list of provider events or a
  function of the inference request, which is how a scenario asserts what
  the model was shown.
- **conversation(target)**: `POST /api/conversations`, then `PATCH` to
  the workspace for a runner target, then the WebSocket stream opened
  through `AgentClient`.

## The driver

```
 const turn = await driver.turn({ model: [events.toolCall('t1', 'shell_exec', {script}), events.text('done'), events.response()] })
 turn.received      // the tool results as the model received them: the items of the next inference request
 turn.shell         // the shell_output frames of this turn, in order
 turn.blocks        // the blocks this turn appended to the live transcript
 await driver.files()   // the target's files under the conversation's cwd: host store for hostless, disk for a runner
 world.wire(deviceId)   // the trace since the last call: frame counts by type, rpc_output bytes
```

`received` is the primary observation. The suite's central question is
"what did the model see", so most assertions are on that text, not on
frames or files; frames and files confirm the mechanism behind it.

## Two targets, one script

Every scenario that does not itself switch targets runs twice, over
`describe.each(['hostless', 'runner'])`, with the same script. The
differences a scenario may special-case are the following and no others;
any other difference between the two runs is a bug.

| Difference | Hostless | Runner |
|---|---|---|
| `demi host current` | `host: virtual` | the device name |
| `outputDir` in a status view, `path` on a stream | absent | present (the tee's output files) |
| binary final stream note | "not kept beyond this view" | the output file path |
| the executable set | tinybash builtins and the manifest roots | whatever the target has |
| a refused script | "runs nothing and says so" | bash's own error |
| shell state between turns | the default shell keeps its variables and cwd | the shell keeps its cwd and nothing else |
| a command killed by `shell_abort` | `status: aborted` | `status: aborted`, with the process's own last words on stderr |

## Invariants at teardown

`world.close()` checks these for every conversation the file opened, so a
scenario never repeats them:

- the cold transcript (`GET /api/conversations/:id/transcript`) has the
  same block ids in the same order as the live one;
- no runner socket carried output beyond the view: every `rpc_output` and
  job output frame stays within the view's byte budget, and no frame type
  outside the protocol's control set appears (the audit `host-shell`
  introduced, generalized);
- every job started on a runner reported its exit, except those running
  when the world killed that runner; every transfer completed;
- the usage ledger has exactly one row per provider request the model
  answered (a request cut off by an abort carries no usage).

A failed invariant names the conversation and the scenario that opened it.

## Scenarios

Each scenario is one test, one conversation, a linear script of turns.

| # | Scenario | Turns | Asserts |
|---|---|---|---|
| S1 | File workflow | create through a heredoc, read, edit, list, across four turns | the text the model receives equals the file's content; the file is where the target keeps it |
| S2 | Output view | a command printing far past the view budget; a binary final stream; a non-zero exit; a command hitting its timeout | head and tail within budget with the elision note; the binary placeholder; exit and timeout reported in the result; on a runner the wire bytes equal the view |
| S3 | Long commands and steering | a command outliving its window (`demi agent spawn` with a slow child: the long-running command both targets share) polled with `shell_status` to its end; a second command aborted with `shell_abort`; stdin fed with `shell_write` to an unredirected `head -n 1`; on a runner only, a background job outliving its command | the status machine as the model sees it; nothing left running after the abort |
| S4 | Todo | `demi todo` written in one turn, read in a later one; a second conversation sees an empty list | the rpc leaf crosses the relay on a runner and runs in-process hostless; storage scoped to the session |
| S5 | Subagents | `demi agent spawn` with the `explore` profile, then `default`; the child runs commands on the same target; the parent reads the result | parent and child share the target; a profile is a prompt, not a restriction; the parent's transcript carries the subagent frames |
| S6 | Continuing across a switch | S1's first turn hostless, switch to the runner over `PATCH`, S1's remaining turns there, switch back, one more read | the script keeps working across both switches; the context block appears at each; files are where each target keeps them |
| S7 | Concurrent sessions on one runner | two conversations on the same device interleaving commands that change `cwd` and shell state | each session sees only its own state; the frames carry the right session attribution |
| S8 | Attachments | an image attached to a user message | the model receives the bytes inline; the transcript stores a `ref`; `GET /api/blobs/:sha256` serves it; the cold transcript carries the same ref |
| S9 | Detach mid-turn | the client closes its socket while a command runs; a new client attaches | the turn completes server-side; the reattached client's transcript has the result; cold equals live |
| S11 | Session upgrade (M11) | a hostless conversation working in a subdirectory with a variable set and a file under `/tmp`; a script outside tinybash's subset; further scripts; a world whose provisioner refuses; then the split-equivalence run: a seven-step sequence run whole on a machine and split at every point | the outside script runs on the machine from the directory and with the variable tinybash held; the files are in the home with `/tmp` under `.tmp`; no context block, no preamble, no pending switch; later scripts run there; a provisioning failure is that call's tool error and the conversation stays hostless with its files; every split's tool results and final files equal the whole run's byte for byte |
| S10 | Managed-host lifecycle (M11) | a machine provisioned and bound over the fake provisioner, worked on, left idle, woken by the next turn, pinned by a job, capped, killed twice, a socket standing in for its runner asking for a bigger home, archived | the device row and its owner; the `sync` before the hibernate after the idle window, its `untouched` report handed to the provisioner, and the checkpoint before it; wake with a fresh token over the same home; jobs pin past the idle window but not past the hard cap; the crash-loop guard reaches the model as a tool error; the per-user cap; another conversation refused; a token-less managed hello refused; `home_grow` reaches the provisioner and `home_grown` answers with the size; archive destroys the guest |
| S12 | Cloud workspace (M11) | `POST /api/workspaces` with `cloud: true`; two conversations switched under it, one writing a file the other reads; both idle; the delete while in use, then after they leave; a second Cloud workspace past the per-user cap; the choice on a backend without a provisioner | the host is a managed device owned by the workspace, hidden from the device list, the workspace at its home; both conversations execute on it and see each other's files; the guest hibernates once no conversation under the workspace has a turn or job, and the next turn of either wakes it; delete is refused while conversations point at it and destroys the guest afterwards; a refused provision leaves neither a workspace nor a device row; a backend that provisions no machines answers 409 |

S9 moves in from `backend.test.ts`, where it was the M2 acceptance.

## Restart scenarios

A separate file with a fixed port. Its world is closed and reopened over
the same data directory to restart the backend; a runner is stopped and
started over the same state directory to restart it.

| # | Scenario | Asserts |
|---|---|---|
| R1 | Backend restart, idle | the conversation on the runner comes back from its database; the runner reconnects with its device token on its own; the next turn executes on the runner; the ledger totals carry over |
| R2 | Backend restart mid-turn | the backend is closed while a command is running: the job is killed on the runner, the tool call settles as an error, the turn closes with an abort block; after the restart the next turn executes and the model sees the aborted call's result first |
| R3 | Runner death and return | the runner is killed while a command runs: the command ends with exit 127 and `runner disconnected`; the runner restarted over its state directory reconnects; the next turn executes; files written before the death are still there |
| R4 | Hostless persistence | files, `demi todo` entries and the ledger survive a backend restart of a hostless conversation |

R1 moves in from `backend.test.ts`, where it was the M3 acceptance, and
gains the runner.

## Layout

```
packages/backend/src/__tests__/scenarios/
  world.ts          the world fixture: backend, runners, claim, workspaces, trace, teardown invariants
  model.ts          the per-conversation script queue behind the stub provider type
  driver.ts         conversation(target) and turn()
  s1-files.test.ts … s12-cloud-workspace.test.ts
  restart.test.ts   R1–R4
  fake-provisioner.ts  the provisioner seam over a local packed runner: the "VM" is a process over the owner's home directory
```

A world takes `managedHosts` (a provisioner and the lifecycle sizes),
`pingIntervalMs` (the liveness ping is what carries `pong.jobs`, so the
idle rule needs it on) and `providerRequestsPerMinute` (a scenario with
many short turns raises the limit). A hostless conversation's files are
rows: `driver.readFile` and `world.hostlessFile` read them through the
`files` table and the blob store; `driver.filePath` is a runner's path.

The fixture is internal to the backend package: nothing here is exported
from a `testing` entry, since no other package drives a backend.
`bun test packages/backend` runs the suite; the runners need the tinyjs
crate built or `TINYJS_BIN` set, as every runner test already does.
