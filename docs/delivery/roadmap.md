# Delivery and acceptance

A checkpoint is complete when its implementation matches the responsible design
and the relevant checks pass. Design approval, successful compilation, fixture
success, and execution on a real target are different evidence. Record check
results with the checkpoint; keep measured results and decision history separate
from the design contracts.

## Dependency order

Deliver complete paths through the system before broadening their deployment.
Each stage below builds on the stages before it.

```text
Contracts and crate boundaries
    -> native runner and command transport
    -> backend conversation, storage, and provider integration
    -> devices, target exchange, and user Cloud
    -> product integration and packaged deployment
    -> distributed ownership and recovery
```

| Area | Completion condition | Contract |
|---|---|---|
| Contracts | Each wire and stored format has one Rust definition that both ends use; the browser's schemas are generated from those definitions; every boundary decodes and validates what it receives | [Contracts](../architecture/contracts.md) |
| Crate and package boundaries | Product storage and execution policy stay outside the reusable agent, shell, and provider crates; the crate graph and the TypeScript package graph hold | [Crates and packages](../architecture/crates-and-packages.md) |
| Native execution | Validated wire contracts, shell/job conformance, cancellation, independent installations, and resident service lifecycle work on the offered platforms | [Native runtime](../execution/native-runtime.md), [Runner](../execution/runner.md) |
| Commands | Native operations run beside their files; RPC invokes the correct node's backend handler and scoped storage | [Commands](../execution/commands.md) |
| Backend and storage | Scripted turns persist and recover; each user's work runs in that user's shard; metadata, journal, blobs, and command state respect ownership boundaries | [Backend](../backend/backend.md), [Storage](../backend/storage.md), [Concurrency](../architecture/concurrency.md) |
| Providers | Configured entries, accounts, model catalogs, and reported usage work through scripted endpoints without credential disclosure | [Providers](../providers/providers.md), [Models](../providers/models.md), [Usage and quota](../providers/usage-and-quota.md) |
| Devices and targets | Claim/reconnect/revoke and target exchange preserve attribution, context, and execution-tree admission | [Sessions and targets](../execution/sessions-and-targets.md) |
| Resource lifecycle | One conversation idle rule stops idle Cloud machines and releases idle conversations on paired devices; no cleanup wake or orphan state | [Conversation idle and Host resource release](../execution/resource-lifecycle.md#acceptance) |
| Personal Cloud | One user has one machine; disk generations survive ordinary wake; external reset retains home and serializes recovery | [Managed hosts](../cloud/managed-hosts.md) |
| Accounts and product | Authentication, roles, provider mode, and resource isolation hold across every exposed operation | [Product](../product/product.md), [Web API](../product/web-api.md) |
| Browser | Shared UI has real product adapters and gallery examples; backend persistence and authorization are verified through the product | [Web architecture](../product/web-application.md) |
| Browser automation | Conversation-owned browser, shell commands, observations, screenshots, and lifecycle satisfy their contracts on paired devices and Cloud | [Conversation browser](../browser/browser.md#acceptance) |
| Live browser view | The user watches and operates the conversation's tabs in the work panel on paired devices and Cloud | [Live browser view](../browser/live-view.md#acceptance) |
| Host expose | A device service gets a one-hour public URL; HTTP, streaming, and WebSocket relay byte-faithfully on paired devices and Cloud; expiry, removal, Cloud stop, and revocation destroy it | [Host expose](../execution/expose.md#acceptance) |
| Packaging | The released runner, command programs, backend, and machine manager install and start on their targets; shipped images run under gVisor/systrap on supported Linux and Lima hosts | [Builds and releases](builds-and-releases.md), [Cloud setup](../cloud/setup.md) |
| Distributed deployment | Ownership loss fences stale writers before reassignment; metadata and disk generations recover consistently | [Backend](../backend/backend.md#deployment-and-user-ownership), [Storage](../backend/storage.md#multi-worker-storage-placement) |

## Evidence required at a checkpoint

Every checkpoint passes `cargo xtask check` and, when TypeScript changed, the
frontend's typechecks and tests ([Validation](builds-and-releases.md#validation)).
Use crate tests for schemas, state machines, parsers, and adapters. Use
[Scenarios](scenarios.md) for complete backend paths with scripted providers
and real native runners, and its browser-contract suite for what the browser
relies on. Tests never call real models.

Native release acceptance includes cross builds and execution on each offered
target. Compilation alone does not establish platform support. Exercise
streaming, cancellation, owner-restricted local endpoints, concurrent invocation
contexts, registration isolation, artifact verification, and retirement of
superseded services, and run the tests that start Chrome. A change to the
runner or a native command program is accepted on a paired device and on a
Cloud running a refreshed image
([Cloud image refresh](builds-and-releases.md#cloud-image-refresh)). Report
performance measurements separately from conformance results.

Cloud acceptance requires both scenarios on the fake machine manager and
real-machine checks. A fixture can prove lifecycle admission and operation
ordering; it cannot prove ext4 durability, snapshot publication, sandbox
isolation, or gVisor startup. Real-machine checks must establish system and home
preservation through wake and home preservation through reset, including a guest
that cannot connect. A local restart or a scenario on the fake machine manager
is not acceptance of worker failover.

Browser changes are made once in `web-ui`, with product and gallery adapters in
the same checkpoint. Verify the affected real product flow and the shared
gallery examples. Component styles and layouts are maintained in the gallery,
not in separate specifications.

Documentation-only checkpoints check consistency and links, and, where code
already implements the described behavior, that the two agree. They do not
imply runtime acceptance. Implementation checkpoints also restart locally
running services on the finished code before handoff, then commit and push.

## Decisions before expanding deployment

Each decision below is open, and must be settled before the deployment it
affects is offered. The first three concern the multi-worker deployment, where a
reverse proxy pins each user to one backend worker
([Deployment and user ownership](../backend/backend.md#deployment-and-user-ownership)).

- **Fencing and reassignment.** A route-map entry decides where a user's new
  requests go. It does not stop a stale worker that still holds the user's
  conversation databases and machine disks from writing to them, or from
  running jobs over the runner connections it still holds. Define worker
  leases, fencing, and reassignment before enabling multiple writers or
  restoring a user's machine on another worker: the stale worker's storage and
  execution authority must end before the new owner gains writable access, and
  moving a user must wait until the replicated state the destination restores
  is ready
  ([Multi-worker storage placement](../backend/storage.md#multi-worker-storage-placement)).
- **Claim routing.** A new runner connects before anyone owns it, so the proxy
  places its connection on any worker, where it waits with its pairing code.
  The user's claim arrives with the user's session and goes to the user's
  worker. When the two workers differ, the claim cannot reach the waiting
  connection. The routing must keep a pairing code and its connection on one
  worker; how it does so is undecided.
- **Expose routing.** An expose hostname carries the expose id and nothing
  else, so the proxy cannot tell from it which worker owns the user. Either the
  route map gains an id-to-user lookup, or the hostname carries the user's
  routing key ([Host expose](../execution/expose.md#deployment)).
- **Retention.** Define how long disks and blobs are kept after an explicit
  account or data deletion, and when an unreferenced blob is removed. Removing
  project metadata must never implicitly delete the user's Cloud machine or
  files.
- **Resource profiles.** Choose resource profiles and unattended lifetime from
  measured memory, startup, and storage costs. Add transport prioritization or
  batching only for demonstrated contention, without changing job attribution
  or cancellation guarantees.
