# Demi Next: Acceptance Ledger

Implementation checkpoint: 2026-09-08. This ledger records the current source
and completed checks. Historical changes and earlier designs are in Git.

## Implemented architecture

| Contract | Responsible code | Evidence |
|---|---|---|
| One TypeScript runner on txiki.js for Cloud and connected devices | `runner`, `host-remote`, `command-loader` | real-runner command, filesystem, process, relay and shell-control suites |
| One managed device per user; projects are directories | backend control schema and `managed/lifecycle.ts` | concurrent first use, shared project/device identity, target ownership and switching tests |
| Persistent system and home, pinned base | `@demicodes/machines` (`firecracker/`, `machine-image-store.ts`) | paired publication, orphan recovery, failed publication, bounded generation retention; real shutdown/wake |
| External system reset retaining home | lifecycle, `http/cloud.ts`, per-conversation reset context | concurrent operation-ID reuse, failed reset/retry, preserved files and model announcement; real broken-bash reset |
| Device-wide activity and capacity admission | lifecycle and registry-injected `RemoteHost` admission | file calls, spawns and jobs retain leases; cached Hosts refuse new work during transitions; idle/wake scenario |
| Shared Cloud settings | `web-ui/cloud/CloudSettings.vue` | product/gallery usage, frontend typechecks and browser verification of confirmation, progress and completion |

Cloud guest init mounts both writable disks and drops the runner to uid/gid
1000. Filesystem RPCs, raw processes and shell jobs therefore use the same
account. Connected runners use their local account. Both paths share command
manifests, JS command loading, Zod contracts, relay, output handling and process
control. VM provisioning, boot mounts, credentials and disk lifecycle remain
managed-host responsibilities.

The workspace contains no virtual-host package or shell interpreter. The Host
process contract requires `spawn`. Test fixtures use real Node filesystem/process
facets or the actual packed runner. Runtime credentials are dropped once after
guest init; there is no separate per-job guest identity path.

## Native client checkpoint

`packages/command-client` now provides a separate C + static libuv `demi`.
The runner owns loader dispatch and isolated runtime workers. A live
context pins each job to its runner, backend registration and manifest;
random owner-only Unix endpoints and installation locks isolate concurrent
runners. Matched release generation, backend downloads and the Unix
installer support independent reuse and draining upgrades.

Native tests cover binary/fragmented frames, bounded input/output,
disconnects, cancellation under backpressure and CPU-loop interruption.
Integration tests cover simultaneous backend registrations, cwd/env,
stale contexts and real-binary installation upgrades. Windows release,
local-only pipe ACLs and runtime acceptance remain open; the PowerShell
endpoint explicitly reports unavailable.

## Completed checks

- `bun run typecheck`: passed.
- `bun run typecheck:web`: passed for all three frontend packages.
- `bun run test`: 900 passed, 11 conditionally skipped, 0 failed across 154 files.
- `bun run build`: passed.
- `bun run check:registry`: passed.
- Fork checks: exclusive file leases, CPU-bound worker termination, inherited descriptors, partial file writes, detached processes,
  privilege validation, queued stream writes, slow HTTP readers, early upload
  cancellation and the existing fetch request/abort regressions passed.
- An embedded fixture passed Linux PID 1 orphan reaping (32 descendants with
  registered exit statuses preserved), permanent privilege drop with empty
  supplementary groups, empty-chroot startup without `/proc`, and a full
  3 MiB awaited stdout write.
- macOS ARM64 and Intel application startup and strict code-signature checks
  passed. Linux ARM64 and x64 static applications cross-compiled.
- `packages/tinyjs` has been removed. The runner uses the pinned
  `vendor/txiki.js` fork; build/test helpers compile and cache the native
  single-file application before the ordinary tests run.
- Browser inspection of the gallery Cloud settings: confirmation layout,
  progress, completion, and the preserved-home explanation verified.

All executed provider turns used scripted providers. No real-model tests ran.

## Linux/KVM verification

The env-gated `real-firecracker.e2e.test.ts` ran inside the local ARM64 Lima KVM
host using Firecracker 1.16.1, the repository-built Linux 6.1.155 kernel and
the Ubuntu 26.04 rootfs. Both direct and jailer launch modes passed on the
24.04 rootfs; the 26.04 rootfs ran in direct mode, with the machine manager
as its own process reached over the Unix socket, and from a macOS backend
through the `demi-machines` Lima instance created from the checked-in
template. The login-shell scenario in the same file (rustup, then `cargo` in
the next command; nvm, then `node` from `~/.nvm`) passed on 26.04.

The scenario verified upload ownership and shell uid 1000, shared machine
identity for another project, persistence of a system installation and home
file across shutdown/wake, and stable device identity after reset. Both final
runs additionally removed bash execute permission and wrote a newer home file
immediately before reset; external reset restored bash and preserved that newer
file. These are functional tests, not capacity or boot-latency benchmarks.

## Product integration boundary

The backend implements authenticated `GET /api/cloud` and
`POST /api/cloud/reset`. The shared settings component receives Cloud state and
emits a reset operation ID; retries reuse that ID.

`web/settings/DevicesPanel.vue` now supplies backend Cloud state and reset handlers.
The gallery retains simulated handlers for the same shared component. Product
state and operation ownership are described in `../web-integration.md`.

The tests above establish the named scenarios. They do not establish
production-scale capacity, exhaustive crash injection at every filesystem
operation, or a production deployment of the web prototype.


## Backend web-integration checkpoint (2026-09-09)

The backend now provides email/password identity, verified email changes through
an injected mail sender, nickname updates, per-field appearance/shortcut storage,
manual API-provider model configuration, subscription account management,
provider status/quota and forced model-catalog refresh. Public provider snapshots
exclude credential material. Shared provider changes remain admin-only.

Conversation APIs persist pins, ordering and read acknowledgements, expose live
and persisted activity summaries, refuse archived writes and running archives,
and report independent PATCH/batch outcomes. Device browsing includes home and
file metadata; remote references preserve device identity and use existing host
commands. Conditional `/api/state` snapshots support page reconciliation, and an
optional built-web directory supports deep navigation alongside API/WS routes.
Responsible modules and request contracts are in [backend.md](backend.md).

Validation uses scripted providers, fake email delivery, mocked vendor responses
and local runner fixtures; no real-model tests were run. Backend suite: 128 passed,
12 environment-dependent tests skipped. Agent server/transcript tests: 36 passed.
Package-boundary checks: 26 passed. TypeScript checking and backend build passed.
Frontend API wiring, Vite proxy configuration and deployment-specific mail delivery
remain integration work. Initial/admin-provisioned accounts are trusted provisioning;
public registration and password recovery are outside this checkpoint.


## Web product integration checkpoint (2026-09-10)

`packages/web` now consumes the backend account, state, provider, conversation,
project, device, file and Cloud APIs. Chat uses the agent WebSocket protocol,
including admission confirmation, queue/steer controls, recovery, subagent
history and terminal output. Leaving a conversation detaches the browser while
the backend task continues. The prototype data and reply timers are removed.

Shared interaction lives in `packages/web-ui`: `ChatSession`, `SidebarLayout`,
directory and remote-file pickers, attachment retry, media rendering, settings
validation and keyboard bindings. Gallery examples use these components with
fixtures. Per-user drafts include file bytes and scroll state; IndexedDB writes
are issued before page teardown and object URLs are released on removal or exit.
The current contracts and deliberate exclusions are in
[web-integration.md](../web-integration.md).

Validation used scripted providers and a disposable local backend/runner, never
real models. Backend suite: 128 passed, 12 environment-dependent skips; the final
remote-reference change also passed its targeted regression test. Agent transport,
admission, recovery and steer checks: 58 passed. Shared UI: 132 passed. Product
state reconciliation: 5 passed. Package boundaries: 26 passed. Root and all web
type checks, agent/backend builds and product/gallery production builds passed.
The web builds retain the existing large-chunk advisory.

Browser checks covered login and expiry, live chat and refreshed history, device
pairing, directory listing and creation, target switching, image/workspace uploads,
failed-upload retry and removal, draft restoration after immediate reload, theme
persistence, nickname/email/password changes, manual-model validation and saving,
and conversation archive/restore. Gallery checks covered the shared session and
child-agent panel. Production mail delivery and deployment remain environment
configuration; the installation instructions and other listed deferred features
remain outside this integration.

## Local conversation lifecycle (2026-09-10)

New opens a local UUID immediately and reuses an empty draft for the same project.
The first Send retains the conversation even if the network fails. Server creation
accepts that UUID idempotently, and unconfirmed messages retain their own IDs,
content and retry action. Workspace files wait for first Send before uploading.
Draft changes enter IndexedDB immediately; empty unsent drafts are discarded.
The shared message list renders unconfirmed sends with the same virtualization
and scrolling as accepted messages. Gallery fixtures cover staged files and
submission failure.

Validation used scripted providers only. The backend suite passed 130 tests with
12 environment-dependent skips. Shared UI and product tests passed, including
local creation, failed first-send retry, staged files and empty-draft disposal.
Browser checks verified immediate-reload persistence, a successful first send,
an offline first-send failure, restored failure state, and retry in the original
conversation. Chrome's file-upload permission prevented the automated picker
check; local staging tests and backend workspace-file tests passed.

## Loading and operation feedback checkpoint (2026-09-10)

`web/state/product.ts` shares vendor reads, caches loaded catalogs and reports
initial loading separately from empty results. Provider draft IDs and subscription
order remain stable across settings opens. Account-scoped provider and device
stores keep submitted writes running after panel closure. Shared loading controls
and regions cover forms, model selection, settings lists, project selection, host
changes, conversation metadata and directory creation. Failed configuration and
model edits retain their inputs for retry. Email/password submissions continue
when their dialogs close and clear credentials on success.

Validation uses mocked HTTP and the isolated scripted-provider browser fixture.
The browser checks confirmed stable selection after reopening, equal button widths
before and during save, retained failed model input, and a successful retry whose
request completed after settings closed. Gallery Control layout includes the shared
loading examples. No validation in this checkpoint calls a real model.
