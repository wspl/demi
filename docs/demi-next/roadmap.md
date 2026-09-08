# Demi Next: Delivery and Acceptance

Status: target delivery plan. A milestone is complete only with evidence in
`progress.md`; this document specifies dependencies and acceptance, not historical
completion. The architecture contracts and their Git diff define implementation
scope. No compatibility paths or legacy-data conversion are part of the design.

## Delivery order

| Milestone | Responsibility | Acceptance |
|---|---|---|
| M0 — Contracts | package boundaries, Host and agent interfaces | lower-level packages remain product-independent; storage is independent of execution devices |
| M1 — Runner wire | shared TypeScript schemas, MessagePack, authenticated filesystem/spawn/job messages | codec and schema tests at both ends; invalid messages rejected |
| M2 — Backend | Web API, conversation module, remote execution | one scripted turn reaches a real runner and persists its result |
| M3 — Storage | control database, per-conversation agent journals, scoped blobs and command storage | cold/live transcript equality, user isolation and restart recovery |
| M4 — Devices | pairing, tokens, registry and remote Host | claim, reconnect, revoke, unknown token and offline behavior |
| M5 — Providers | provider assembly, credential vault, model catalog and usage | mock provider/auth endpoints; process-backed inference obtains its device; no credential disclosure |
| M6 — Targets | Cloud/device/workspace selection, attached devices, per-node context | idle-tree switch admission, device ownership, files stay on original device |
| M7 — tinyjs | QuickJS runtime, native I/O, packed binaries | primitive conformance, platform builds, startup and stream-throughput measurements |
| M8 — Commands | one command tree, manifest, loader, runtime and RPC leaves | JS modules execute on tinyjs; RPC reaches the node's backend handler and storage; independent root embedding |
| M9 — Runner | TypeScript on tinyjs, real bash jobs, tee, relay and transfers | streaming stdin/stdout, cancellation, process cleanup, bounded wire views and authenticated job attribution |
| M10 — Scenarios | backend, scripted provider and real packed runners | `scenarios.md` acceptance matrix, restarts, cross-host pipelines and teardown invariants |
| M11 — Personal Cloud | unique managed device per user; system/home persistence; external reset | multiple projects share one machine; both disks survive shutdown; broken guest resets with home retained; serialized recovery |
| M12 — Multi-user | authentication, roles, provider mode and tenant isolation | cross-user access refused for every named resource, including Cloud status/reset; account controls |
| M13 — Web | shared UI prototype, feature design and backend integration | the reviewed product flows and matching gallery specimens, including Cloud lifecycle and reset |
| M14 — Packaging | backend/web assets, tinyjs runner, kernel/base images and helper | image build and Linux/KVM smoke in both launch modes without real models |
| M15 — Scaled deployment | control service, S3 adapters, replication and user-worker placement | worker fencing, single VM writer, consistent disk generations and user reassignment |

## Cloud acceptance checkpoints

1. **Ownership and execution.** The control database enforces one managed device
   per user. Conversations default to Cloud; projects are directories on it.
   Concurrent first use and wake join one operation. Every production shell
   executes on a runner, with command and file attribution preserved.
2. **Persistent machine.** The image generation pins the base, system and home.
   Shutdown, checkpoint, crash recovery and volume growth preserve the contract.
   System installs survive ordinary wake; `/run` and `/tmp` remain temporary.
3. **Reset and product flows.** Backend-controlled reset works without guest
   cooperation, ends all affected jobs, preserves home and retains device/project
   identity. The settings dialog and progress states live in `web-ui`, with both
   product integration and gallery fixtures in the same checkpoint.

Each checkpoint updates the responsible package, its contract and scoped tests;
commit and push only after the relevant checks pass. Tests use scripted providers
and never call real models. Document-only checkpoints validate consistency and
links; they do not claim runtime acceptance.

## Web delivery

M13 proceeds through a fixture-driven application prototype, accepted feature
semantics, then complete backend integration. `web-ui` owns reusable interactions;
`web` supplies application state/handlers and `web-gallery` supplies specimens.
A browser walkthrough must cover empty, loading, success, error and recovery
states. Prototype fixtures are not production authorization or persistence.
See `product.md`, `web-prototype.md` and `web-gallery-sync.md`.

## Deferred decisions

- Machine-disk retention after explicit account/data deletion and unreferenced
  generation collection; no project action implicitly deletes Cloud data.
- Deployment resource profiles and unattended-service lifetime, based on
  measured VM memory, command-ready latency and storage I/O.
- Scaled worker fencing and placement (M15); user routing alone is insufficient.
- Runner control-priority channels and filesystem batching only when measured
  traffic warrants them (`runner.md`).
