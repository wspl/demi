# Backend scenario acceptance

Scenario tests verify complete backend paths with scripted providers and real
native runners. They complement crate tests, which cover schemas, state
machines, primitive conformance, provider adapters, and command parsing. No
test calls a real model: every suite scripts the model, so a run costs nothing
and answers the same way every time. A check against a real vendor is done by
hand, never as part of a suite.

Three suites drive the whole backend:

| Suite | Where it lives | How it reaches the backend | Model | Hosts |
| --- | --- | --- | --- | --- |
| Backend scenarios | Rust integration tests of the backend crate (`crates/backend/tests`) | HTTP and the conversation WebSocket, with the agent protocol's typed frames | A scripted provider family | Real runner processes; a scripted machine manager for the Cloud |
| Browser-contract suite | Tests of `packages/web` | The web application's API client and `AgentClient`, against the backend executable | A scripted Anthropic-compatible endpoint | A real runner |
| Real-machine suites | Rust tests that run only when environment variables supply their resources | As the backend scenarios | Scripted | A real machine manager, gVisor sandbox, and shipped image; real Chrome; the real Claude Code CLI |

## System under test

```text
Driver: HTTP with a session cookie; conversation WebSocket, typed agent frames
    -> backend in the test process: the edge and the user's shard
    -> the conversation's host access and remote shell environment
    -> runner process with embedded brush
         +-- native command service beside the files
         +-- rpc relay to the invoking node's backend handler

Observe: the next provider request, target files, the cold transcript,
the usage ledger, and the runner wire frames
```

The scenario harness, the *world*, starts the backend inside the test process,
with its production shard threads and a temporary data directory. In place of
external services it registers a scripted model as a provider family, serves
a scripted machine manager on a Unix socket, and serves local fixtures in
place of models.dev and the Claude Code release distribution. It pairs real
runner processes, records every runner wire frame in both directions, and owns
cleanup. The runners are the real runner executable, which `cargo xtask test`
builds before the tests start
([Validation](builds-and-releases.md#validation)). The fakes come from the
test-support features of the crates that own what they fake
([Crates and packages](../architecture/crates-and-packages.md)); the scripted
machine manager is the backend's own test code, since the manager's crate runs
only on Linux.

The scripted model keeps a queue of turn scripts per session, answers title
requests from a queue of their own, and records every request it receives. A
*driver* opens a conversation through the HTTP API, connects to its WebSocket,
supplies the scripted events for each turn, and records inference requests and
tool results. Assertions inspect what the model receives, not just what a
client displays.

A driver reads a transcript as the backend serves it cold, through the
transcript route, and follows events and phases in the frames. It does not
rebuild the live transcript from patches: the browser's patch applier is the
only one, so the [browser-contract suite](#browser-contract-suite) compares
live with cold.

The scripted machine manager speaks `machines-protocol` as the real one does:
operations of one device run in arrival order, requests on a connection are
answered as they finish, and a death reaches every connection. It starts the
same runner as a local process with the boot record's backend URL and token,
over a home that survives stop, wake and reset, and it records wake,
hibernate, checkpoint, reset, and growth calls. A test can hold or fail a
reset, keep a wake's runner from connecting, fail a save, or kill a runner as
a crash would. It does not start a sandbox, mount disk images, or implement
filesystem isolation. Its reset clears runner state while retaining home; a
successful scripted reset does not demonstrate replacement of system
packages.

Restart tests reuse the data directory and a fixed backend port. Runner
restarts reuse device identity and persistent directories. Multi-user scenarios
must use separate authenticated sessions; the world's default helper uses one
session and must not be mistaken for an isolation test by itself.

Scenarios run real runner processes, so they use real time with short
configured windows: a short idle window instead of the product's hour, for
example. A paused clock would advance whenever the runtime waits on a
runner and fire every timer early
([Tests and time](../architecture/concurrency.md#tests-and-time)). In-process
tests on a paused clock pin the exact boundaries of timer rules. A scenario
that needs wall time to pass, such as an expose's expiry, sets an injected
clock instead of waiting.

## Required scenario coverage

The matrix defines the observations each path requires.
[Shared checks and their limits](#shared-checks-and-their-limits) says what the
common checks do not establish.

| Path | Required observation |
|---|---|
| File workflow | Create, read, edit, and list agree with bytes on the selected target |
| Output view | Long/binary output and nonzero exits preserve full target files while bounding model-visible previews |
| Long jobs | Status, stdin, abort, and background jobs retain attribution and do not leave orphan processes; observation timeout does not cancel work |
| Command storage | Todo RPC reaches the invoking node; state survives turns and remains separate across nodes and conversations |
| Subagents | Inherited execution target, independent command state, completion events, and cross-host callbacks retain child identity |
| Target exchange | Cloud/device/project changes update each node's context; files stay on their original device; same-device changes do not duplicate bindings |
| Transitions | While a reset, target change, or archive holds a conversation, opening it, a message, a metadata change, and a Host operation wait and then proceed; a conversation that only has the Cloud attached keeps running through a Cloud reset ([How a conversation uses a device](../execution/sessions-and-targets.md#how-a-conversation-uses-a-device)) |
| Concurrent conversations | cwd and job handles stay scoped while filesystem data is shared on the same machine |
| Attachments | Provider requests receive authorized bytes; the upload answer carries the detected media type and, for a text file, its snippet; persisted references and blob reads enforce ownership |
| Client disconnect | Backend work continues and the client can reattach to persisted results |
| Editing and Fork | History and command-state boundaries are durable and retries are idempotent; external file effects and source children are not replayed or cloned |
| First Cloud use | History-only access does not boot a machine; concurrent execution joins one allocation and wake |
| Multiple Cloud projects | Project directories share one user's managed device and can read each other's files |
| User isolation | Another account cannot resolve, reset, read, or invoke commands on the first account's resources |
| Lifecycle admission | Active trees and admitted file/process operations prevent idle retirement; wake/reset/checkpoint races do not create duplicate writers |
| Archive and project removal | Metadata operations preserve machine identity and files; conversation deletion is not a product operation |
| Reset | Affected jobs end, retained home survives, and the selected system generation becomes authoritative without changing device/project identity |
| Cross-host RPC and pipes | Valid callbacks use the invoking Host; unauthorized or mismatched job/node/device contexts fail; bulk bytes stream separately from bounded control views |
| Host expose | A public URL relays HTTP, streaming, and WebSocket to the device service; another user cannot manage it; expiry, removal, and Cloud stop destroy it and end its connections ([Host expose](../execution/expose.md#acceptance)) |

Use [Commands](../execution/commands.md),
[Sessions and targets](../execution/sessions-and-targets.md),
[Storage](../backend/storage.md), and
[Managed hosts](../cloud/managed-hosts.md) as the authorities for behavior. A
fixture must not define an alternative execution contract.

## Failure and recovery

| Failure | Required result |
|---|---|
| Idle backend restart | History and command state restore, devices reconnect, and new work runs |
| Backend restart after dispatch but before a result | The call has an explicit unknown outcome and is never silently replayed |
| Backend shutdown | Every shutdown step runs even when one fails; open transfers and user streams end before the user's Cloud hibernates, so the Cloud still saves ([Startup and shutdown](../backend/backend.md#startup-and-shutdown)) |
| Runner death | Its running jobs fail; reconnect permits new work; persistent files remain |
| Cloud hibernate/wake | System and home changes survive; processes and temporary output do not |
| Checkpoint publication failure | Previous committed generation remains usable; working files remain available for retry |
| Reset failure before publication | Home and the previous generation remain recoverable; new work cannot use an indeterminate system |
| Boot failure after reset publication | The new committed generation stays authoritative; retry boots it without resetting again |
| Broken or disconnected guest | Backend-controlled reset does not depend on guest commands |
| Worker ownership loss | The stale worker is fenced before the new owner gains writable disk access |

The last row belongs to the multi-worker deployment
([Deployment and user ownership](../backend/backend.md#deployment-and-user-ownership));
a suite that runs one backend cannot demonstrate it.

## Shared checks and their limits

When a scenario closes, the world checks that every scripted response was
consumed. It does not compare the live transcript with the cold one; tests for
content, media, queue, and command-state correctness assert those values
explicitly on the cold transcript.

For explicitly paired devices, the world totals `job_output` bytes per job
from the wire frames, checks them against the bound the runner wire derives
from `JOB_VIEW_BYTES`, and reconciles job starts with exit reports or
intentionally lost jobs. It also checks named pipe ends against `pipe_done`
reports, allowing losses only for intentionally stopped runners. These are
count and byte checks, not a complete proof of job ownership or every
wire-frame limit. Runners that the fake machine manager starts are not paired
devices, so their equivalent coverage needs separate assertions.

The usage check compares ledger rows with answered scripted requests; each
answered script ends with a response that carries usage. It does not establish
a row for a request that failed or was cancelled before a response
([Usage ledger](../providers/usage-and-quota.md#usage-ledger)).

Cleanup belongs to the world and its fake machine manager, including after
assertion failures: drivers detach, runners stop, and the backend closes. Tests
introducing new streams, processes, or failure injection must also verify their
cleanup rather than relying only on the common job counters.

## Browser-contract suite

The browser-contract suite checks the backend the way the page uses it, and it
is the regression suite for that contract: a change on either side that breaks
the other fails here. It is part of the tests of `packages/web`. It starts the
backend executable with a temporary data directory, calls it through the web
application's API client, and drives conversations with `AgentClient` from
`@demicodes/agent-client`, the client the page runs. The generated schemas
validate every response and frame on arrival, as they do in the page
([Generated TypeScript](../architecture/contracts.md#generated-typescript)).
The model is an Anthropic-compatible HTTP endpoint that the suite scripts and
the backend's Anthropic API provider calls; tools run on a real runner.

| Path | Required observation |
|---|---|
| Sign-in | A user signs in through the API client, and the session admits the account snapshot and the conversation WebSocket |
| Create and chat | A conversation created through the API runs a turn, and the client receives the scripted reply and the end of the turn |
| Tools | A scripted tool call runs on the real runner, and its result appears in the transcript |
| Reload | After a reload, the transcript the client assembled from live patches equals the cold transcript the backend serves |

The patch protocol has one more check. The Rust agent tests write patch
sequences together with the snapshot each must produce, and the
`@demicodes/agent-client` tests apply them with the browser's patch applier
([Frame protocol](../agent/runtime.md#frame-protocol)). With the reload check,
this verifies the patches without a second applier.

## Real machine acceptance

The Cloud's real-machine suite runs the backend against a real machine
manager, its gVisor/systrap sandbox, and a shipped image, with scripted
providers. It reaches the Cloud through normal Host access, as a conversation
does, and its package checks need network egress; no test calls a real model.
[Managed hosts — Verification](../cloud/managed-hosts.md#verification) owns the
full persistence, isolation, failure, and platform matrix that a run must
show. Linux amd64, Linux arm64, and local Lima execution are separate
acceptance environments. Record startup phases, memory, and checkpoint I/O
separately from functional assertions. A fake machine manager does not certify
filesystem durability, sandbox isolation, or distributed writer fencing.

Each real-machine suite runs only when environment variables supply what it
needs, and is skipped otherwise, so an ordinary test run needs no Linux host,
Chrome, or Claude Code CLI:

| Suite | What is real | What its environment supplies |
|---|---|---|
| Cloud | The machine manager, its gVisor sandbox, and the shipped image | The manager's socket, a backend URL the manager allows, and a local copy of the image manifest |
| Browser | Chrome for Testing on a paired device or on the Cloud | The command program that drives Chrome, and on the Cloud what the Cloud suite needs |
| Claude Code | The vendor's CLI on a runner, calling a local mock of the vendor's endpoint | The CLI executable |

Deployment prerequisites are in [Cloud setup](../cloud/setup.md).
