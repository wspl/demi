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
| Contracts | Each wire and stored format has one Rust definition that both ends use; the web app's schemas are generated from those definitions; every boundary decodes and validates what it receives | [Contracts](../architecture/contracts.md) |
| Crate and package boundaries | Product storage and execution policy stay outside the reusable agent, shell, and provider crates; the crate graph and the TypeScript package graph hold | [Crates and packages](../architecture/crates-and-packages.md) |
| Native execution | Validated wire contracts, shell/job conformance, cancellation, independent installations, and resident service lifecycle work on the offered platforms | [Native runtime](../execution/native-runtime.md), [Runner](../execution/runner.md) |
| Commands | Native operations run beside their files; RPC invokes the correct node's backend handler and scoped storage | [Commands](../execution/commands.md) |
| Plugins | Every agent capability beyond the runtime's tools and groups and `demi host` is a plugin; each plugin passes its tests through the JSON loopback transport; the agent runtime names no plugin | [Plugins](../architecture/plugins.md) |
| Skills | Sources fetch and pin; a repository's own skills are found without waking a Host; the catalog of the skills that are available reaches every node as context; user skills' directories install on a Host before a job needs them | [Skills](../agent/skills.md#acceptance) |
| Backend and storage | Scripted turns persist and recover; each user's work runs in that user's shard; metadata, journal, blobs, and command state respect ownership boundaries | [Backend](../backend/backend.md), [Storage](../backend/storage.md), [Concurrency](../architecture/concurrency.md) |
| Providers | Configured entries, accounts, model catalogs, and reported usage work through scripted endpoints without credential disclosure | [Providers](../providers/providers.md), [Models](../providers/models.md), [Usage and quota](../providers/usage-and-quota.md) |
| Devices and targets | Claim/reconnect/revoke and target exchange preserve attribution, context, and execution-tree admission | [Sessions and targets](../execution/sessions-and-targets.md) |
| Resource lifecycle | One conversation idle rule stops idle Cloud machines and releases idle conversations on paired devices and running Clouds; no cleanup wake or orphan state | [Conversation idle and Host resource release](../execution/resource-lifecycle.md#acceptance) |
| Personal Cloud | One user has one machine; disk generations survive ordinary wake; external reset retains home and serializes recovery | [Managed hosts](../cloud/managed-hosts.md) |
| Accounts and product | Authentication, roles, provider mode, and resource isolation hold across every exposed operation | [Product](../product/product.md), [Web API](../product/web-api.md) |
| Web app | Shared UI has real product adapters and gallery examples; backend persistence and authorization are verified through the product | [Web architecture](../product/web-application.md) |
| Browser automation | Conversation-owned browser, shell commands, observations, screenshots, and lifecycle satisfy their contracts on paired devices and Cloud | [Conversation browser](../browser/browser.md#acceptance) |
| Live browser view | The user watches and operates the conversation's tabs in the work panel on paired devices and Cloud | [Live browser view](../browser/live-view.md#acceptance) |
| Host expose | A device service gets a one-hour public URL; HTTP, streaming, and WebSocket relay byte-faithfully on paired devices and Cloud; expiry, removal, Cloud stop, and revocation destroy it | [Host expose](../execution/expose.md#acceptance) |
| Packaging | The released runner, command programs, backend, and machine manager install and start on their targets; shipped images run under gVisor/systrap on supported Linux hosts | [Builds and releases](builds-and-releases.md), [Cloud setup](../cloud/setup.md) |
| Distributed deployment | Ownership loss fences stale writers before reassignment; metadata and disk generations recover consistently | [Backend](../backend/backend.md#deployment-and-user-ownership), [Storage](../backend/storage.md#multi-worker-storage-placement) |

### Plugins

The plugin design ([Plugins](../architecture/plugins.md#built-in-plugins)) is
delivered in this order, each a checkpoint of its own. The dependency graphs
in [Crates and packages](../architecture/crates-and-packages.md#dependency-graphs),
which the boundary checks hold the code to, list the edges the code has: each
step adds the lines of the crates and packages it builds, such as
`plugin-expose`, `plugin-skills` and the `@demicodes/plugin-*` packages, and
removes the edges it retires, such as `web-api-protocol`'s and `backend-http`'s
on the browser protocol and `backend-host-access`'s on `backend-expose`.

1. **The contract and the command plugins.** `plugin-interface` with its
   loopback transport, `backend-plugins` with the rule that leaves out a
   group whose package the catalog does not serve, the runtime without a
   harness (the product's dependencies, context sources that name their
   source and see only their blocks since the last compaction, profiles as
   data, no harness name in a checkpoint), and the commands of `plugin-todo`,
   `plugin-file` and `plugin-browser`. `agent-coding-harness` is removed.
   Done.
2. **The page-facing plugins.** `plugin-browser`'s `browser` user stream and
   its tab methods over package calls; and `plugin-expose` with `demi expose`, its numbers, the conversation hosts and
   exposes operations and a page state that follows the user's exposes. The
   plugin call routes replace the browser tab routes and `/api/exposes`, and
   `web` calls them, with the plugins' types generated into their page
   packages' `src/generated`.
3. **A user's plugins.** Each user's plugin choices with the plugin switch
   route and the plugin list in the product state; the agent server's trees
   taking their commands and profiles when they open, with their revision;
   context sources, page calls, page states and user streams following the
   choices at once; `pluginsChanged` and the reload route; and the settings
   page's plugin list with its switches and the reload offer, in `web-ui`,
   `web` and `web-gallery`.
4. **Skills on the backend.** `plugin-skills` with its sources, its values and
   blobs, the Host directories with their installation before a job, the Host
   file reads that never wake a Host, project skills, the catalog, and the
   page call route with the `plugin` sync message.
5. **The plugins' pages.** `PluginClient`, `usePlugin()` and the slots in
   `web-ui`; `@demicodes/plugin-browser` (the `browser` work panel kind),
   `@demicodes/plugin-expose` (the conversation header tool) and
   `@demicodes/plugin-skills` (the settings section); their registration in
   `web` and their specimens in `web-gallery`.

## Evidence required at a checkpoint

Every checkpoint passes the Rust checks and tests and, when TypeScript
changed, the frontend's typechecks and tests
([Validation](builds-and-releases.md#validation)).
Use crate tests for schemas, state machines, parsers, and adapters. Use
[Scenarios](scenarios.md) for complete backend paths with scripted providers
and real native runners, and its web app contract suite for what the web app
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

Web app changes are made once in `web-ui`, with product and gallery adapters in
the same checkpoint. Verify the affected real product flow and the shared
gallery examples. Component styles and layouts are maintained in the gallery,
not in separate specifications.

Documentation-only checkpoints check consistency and links, and, where code
already implements the described behavior, that the two agree. They do not
imply runtime acceptance. Implementation checkpoints also restart locally
running services on the finished code before handoff, then commit and push.

## Decisions before expanding deployment

Each decision below is open, and must be settled before the deployment it
affects is offered. The first four concern the multi-worker deployment, where a
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
- **Changes across workers.** A change can concern users whom another worker
  serves: a shared instance's provider entries serve every user, and each
  worker keeps its own quota snapshots. A worker marks such a change only on
  the synchronization channels it holds
  ([Page synchronization](../backend/backend.md#page-synchronization));
  how it reaches the pages of users on other workers is undecided.
- **Retention.** Define how long disks and blobs are kept after an explicit
  account or data deletion; [Account deletion](../backend/storage.md#account-deletion)
  names the objects it must remove. When an unreferenced blob goes is decided
  ([Retention](../backend/storage.md#retention)). Removing project metadata
  must never implicitly delete the user's Cloud machine or files.
- **Resource profiles.** Choose resource profiles and unattended lifetime from
  measured memory, startup, and storage costs. Add transport prioritization or
  batching only for demonstrated contention, without changing job attribution
  or cancellation guarantees.
