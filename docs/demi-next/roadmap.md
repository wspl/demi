# Delivery and acceptance

A checkpoint is complete when its implementation matches the responsible design
and the relevant checks pass. Design approval, successful compilation, fixture
success, and execution on a real target are different evidence. Record check
results with the checkpoint; keep measured results and decision history separate
from the design contracts.

## Dependency order

Deliver complete paths through the system before broadening their deployment.
The sequence below describes dependencies, not a claim that every stage is done.

```text
Contracts and package boundaries
    -> native runner and command transport
    -> backend conversation, storage, and provider integration
    -> devices, target exchange, and user Cloud
    -> product integration and packaged deployment
    -> distributed ownership and recovery
```

| Area | Completion condition | Contract |
|---|---|---|
| Framework boundaries | Product storage and execution policy stay outside reusable agent, shell, and provider packages | [Package boundaries](../package-boundaries.md) |
| Native execution | Validated wire contracts, shell/job conformance, cancellation, independent installations, and resident service lifecycle work on the offered platforms | [Native runtime](native-runtime.md), [Runner](runner.md) |
| Commands | Native operations run beside their files; RPC invokes the correct node's backend handler and scoped storage | [Commands](commands.md) |
| Backend and storage | Scripted turns persist and recover; metadata, journal, blobs, and command state respect ownership boundaries | [Backend](backend.md), [Storage](storage.md) |
| Providers | Configured entries, accounts, model catalogs, and reported usage work through scripted endpoints without credential disclosure | [Providers](providers-and-vault.md) |
| Devices and targets | Claim/reconnect/revoke and target exchange preserve attribution, context, and execution-tree admission | [Sessions and targets](sessions-and-targets.md) |
| Resource lifecycle | One conversation idle rule stops idle Cloud machines and releases idle conversations on paired devices; no cleanup wake or orphan state | [Conversation idle and Host resource release](resource-lifecycle.md#acceptance) |
| Personal Cloud | One user has one machine; disk generations survive ordinary wake; external reset retains home and serializes recovery | [Managed hosts](managed-hosts.md) |
| Accounts and product | Authentication, roles, provider mode, and resource isolation hold across every exposed operation | [Product](product.md), [Web API](web-api.md) |
| Browser | Shared UI has real product adapters and gallery examples; backend persistence and authorization are verified through the product | [Web architecture](web-application.md) |
| Browser automation | Conversation-owned browser, shell commands, observations, screenshots and lifecycle satisfy their contracts on paired devices and Cloud; workpanel display is deferred | [Conversation browser](browser.md#acceptance) |
| Host expose | A device service gets a one-hour public URL; HTTP, streaming and WebSocket relay byte-faithfully on paired devices and Cloud; expiry, removal, Cloud stop and revocation destroy it | [Host expose](expose.md#acceptance) |
| Packaging | Published artifacts install and start; shipped images boot in direct and jailer Linux/KVM modes | [Native builds](../native-builds.md), [Cloud setup](../managed-hosts-setup.md) |
| Distributed deployment | Ownership loss fences old writers before reassignment; metadata and disk generations recover consistently | [Backend](backend.md), [Storage](storage.md) |

## Evidence required at a checkpoint

Use package-level tests for schemas, state machines, parsers, and adapters. Use
[Scenarios](scenarios.md) for complete backend paths with scripted providers and
real native runners. Tests never call real models.

Native release acceptance includes cross-builds and execution on each offered
target. Compilation alone does not establish platform support. Exercise streaming,
cancellation, owner-restricted local endpoints, concurrent invocation contexts,
registration isolation, artifact verification, and retirement of old services.
Report performance measurements separately from conformance results.

Cloud acceptance requires both fake-provisioner scenarios and real machine checks.
A fixture can prove lifecycle admission and operation ordering; it cannot prove
ext4 durability, snapshot publication, guest isolation, or Firecracker startup.
Real machine checks must establish system/home preservation through wake and
home preservation through reset, including a guest that cannot connect.

Browser changes are made once in `web-ui`, with product and gallery adapters in
the same checkpoint. Verify the affected real product flow and the shared gallery
examples. Component styles and layouts are maintained in the gallery, not in
separate specifications.

Documentation-only checkpoints check consistency, source correspondence where
behavior is described as implemented, and links. They do not imply runtime
acceptance. Completed implementation checkpoints also restart locally running
services on the finished code before handoff, then commit and push.

## Current implementation boundary

The repository has a single-backend composition with local control and
conversation stores, native execution, provider integration, browser adapters,
and user Cloud lifecycle code. Their existence does not certify every failure
case or target platform; the responsible documents identify remaining limits.

Distributed control transport, remote storage/replication, user-worker placement,
and writer fencing remain target architecture. Do not describe a local restart
or fake-provisioner test as acceptance of worker failover. In particular, one
user-to-worker routing entry does not prevent a stale worker from writing disks.

## Decisions before expanding deployment

- Define worker leases, fencing, and reassignment before enabling multiple writers
  or restoring a user's machine on another worker.
- Define credential-pool distribution and recovery alongside provider ownership
  before moving inference across workers.
- Define disk/blob retention for explicit account/data deletion. Removing project
  metadata must never implicitly delete the user's Cloud machine or files.
- Choose resource profiles and unattended lifetime from measured memory, startup,
  and storage costs. Add transport prioritization or batching only for demonstrated
  contention, without changing job attribution or cancellation guarantees.
