# Backend scenario acceptance

Scenario tests verify complete backend paths with scripted providers and real
native runners. They complement package tests for schemas, primitive conformance,
provider adapters, and command parsing. No scenario calls a real model.

## System under test

```text
Authenticated API / AgentClient
    -> backend conversation and agent
    -> remote shell environment
    -> native Rust runner with embedded brush
         +-- native command service beside the files
         +-- RPC relay to the invoking node's backend handler

Observe: next provider request, target files, transcript, usage, and wire frames
```

`backend/src/__tests__/scenarios/World` owns temporary backend storage, a scripted
provider, paired runners, trace collection, and cleanup. `Driver` opens a
conversation through the API, connects an `AgentClient`, supplies scripted events,
and records inference requests and tool results. Assertions inspect what the model
receives, not just what the client displays.

A fake managed provisioner starts the same native runner as a local process with
a retained home directory and a pre-issued token. It records wake, hibernate,
checkpoint, reset, and growth calls. It does not start a sandbox, mount disk images,
or implement filesystem isolation. Its reset clears runner state while retaining
home; a successful fake reset does not demonstrate replacement of system packages.

Restart tests reuse the data directory and a fixed backend port. Runner restarts
reuse device identity and persistent directories. Multi-user scenarios must use
separate authenticated sessions; `World`'s default helper uses one session and
must not be mistaken for an isolation test by itself.

## Required scenario coverage

The matrix defines required observations. Existing scenarios cover the named
paths, while the implementation limits below distinguish narrower assertions and
future deployment requirements.

| Path | Required observation |
|---|---|
| File workflow | Create, read, edit, and list agree with bytes on the selected target |
| Output view | Long/binary output and nonzero exits preserve full target files while bounding model-visible previews |
| Long jobs | Status, stdin, abort, and background jobs retain attribution and do not leave orphan processes; observation timeout does not cancel work |
| Command storage | Todo RPC reaches the invoking node; state survives turns and remains separate across nodes and conversations |
| Subagents | Inherited execution target, independent command state, completion events, and cross-host callbacks retain child identity |
| Target exchange | Cloud/device/project changes update each node's context; files stay on their original device; same-device changes do not duplicate bindings |
| Concurrent conversations | cwd and job handles stay scoped while filesystem data is shared on the same machine |
| Attachments | Provider requests receive authorized bytes; persisted references and blob reads enforce ownership |
| Client disconnect | Backend work continues and the client can reattach to persisted results |
| Editing and Fork | History and command-state boundaries are durable and retries are idempotent; external file effects and source children are not replayed or cloned |
| First Cloud use | History-only access does not boot a machine; concurrent execution joins one allocation and wake |
| Multiple Cloud projects | Project directories share one user's managed device and can read each other's files |
| User isolation | Another account cannot resolve, reset, read, or invoke commands on the first account's resources |
| Lifecycle admission | Active trees and admitted file/process operations prevent idle retirement; wake/reset/checkpoint races do not create duplicate writers |
| Archive and project removal | Metadata operations preserve machine identity and files; conversation deletion is not a product operation |
| Reset | Affected jobs end, retained home survives, and the selected system generation becomes authoritative without changing device/project identity |
| Cross-host RPC and pipes | Valid callbacks use the invoking Host; unauthorized or mismatched job/node/device contexts fail; bulk bytes stream separately from bounded control views |
| Host expose | A public URL relays HTTP, streaming and WebSocket to the device service; another user cannot manage it; expiry, removal and Cloud stop destroy it and end its connections ([Host expose](expose.md#acceptance)) |

Use [Commands](commands.md), [Sessions and targets](sessions-and-targets.md),
[Storage](storage.md), and [Managed hosts](managed-hosts.md) as the authorities for
behavior. A fixture must not define an alternative execution contract.

## Failure and recovery

| Failure | Required result |
|---|---|
| Idle backend restart | History and command state restore, devices reconnect, and new work runs |
| Backend restart after dispatch but before a result | The call has an explicit unknown outcome and is never silently replayed |
| Runner death | Existing jobs fail; reconnect permits new work; persistent files remain |
| Cloud hibernate/wake | System and home changes survive; old processes and temporary output do not |
| Checkpoint publication failure | Previous committed generation remains usable; working files remain available for retry |
| Reset failure before publication | Home and the previous generation remain recoverable; new work cannot use an indeterminate system |
| Boot failure after reset publication | The new committed generation stays authoritative; retry boots it without resetting again |
| Broken or disconnected guest | Backend-controlled reset does not depend on guest commands |
| Worker ownership loss | The old writer is fenced before a replacement gains writable disk access |

The last row requires distributed ownership infrastructure. It is an acceptance
requirement for that deployment, not an implemented guarantee of the local suite.

## Existing shared checks and their limits

`World.close()` checks that scripted responses were consumed and compares cold
and live transcript **block IDs in order**. It does not compare every block field.
Tests for content, media, queue, and command-state correctness must assert those
values explicitly; ID equality alone cannot establish full transcript recovery.

For explicitly paired devices, the world totals `job_output` bytes per job,
checks the bound derived from `JOB_VIEW_BYTES`, and reconciles job starts with
exit reports or intentionally lost jobs. It also checks named pipe ends against
`pipe_done` reports, allowing losses only for intentionally stopped runners.
These are count and byte checks, not a complete proof of job ownership or every
wire-frame limit. Managed fake runners are not in the world's paired-device map,
so their equivalent trace coverage needs separate assertions.

The usage check compares ledger totals with answered scripted requests. Those
scripts emit a response carrying usage. This does not establish billing for every
failed/cancelled request or guarantee durable usage delivery; see
[Provider limits](providers-and-vault.md#implementation-limits).

Cleanup belongs to the world and its provisioner, including assertion failures.
Drivers detach, runners stop, and the backend closes. Tests introducing new
streams, processes, or failure injection must also verify their cleanup rather
than relying only on the common job counters.

## Real machine acceptance

The selected real-machine suite runs the actual gVisor/systrap provisioner and
shipped image with scripted providers. It exercises managed-token registration,
shared Cloud identity, file/job user identity, uploads, system/home persistence,
external reset, and toolchain/browser availability through normal Host access.
Network egress is needed for package checks; no test calls a real model.

The full persistence, isolation, failure, and platform matrix is owned by
[Managed hosts — Verification](managed-hosts.md#verification). Linux amd64,
Linux arm64, and local Lima execution are separate acceptance environments.
Record startup phases, memory, and checkpoint I/O separately from functional
assertions. A fake provisioner does not certify filesystem durability, sandbox
isolation, or distributed writer fencing.

`real-gvisor.e2e.test.ts` implements the managed functional suite; its invocation
is documented in [Managed hosts](managed-hosts.md#implementation-status).
The [evaluation report](../gvisor-evaluation.md) records real manager results and
remaining acceptance limits separately from the earlier Docker experiment.
Deployment prerequisites are in [Cloud setup](../managed-hosts-setup.md).
