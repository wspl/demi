# Demi Next: Acceptance Ledger

Implementation checkpoint: 2026-09-08. This ledger records the current source
and completed checks. Historical changes and earlier designs are in Git.

## Implemented architecture

| Contract | Responsible code | Evidence |
|---|---|---|
| One TypeScript runner on txiki.js for Cloud and connected devices | `runner`, `host-remote`, `command-loader` | real-runner command, filesystem, process, relay and shell-control suites |
| One managed device per user; projects are directories | backend control schema and `managed/lifecycle.ts` | concurrent first use, shared project/device identity, target ownership and switching tests |
| Persistent system and home, pinned base | `managed/firecracker`, `storage/machine-image-store.ts` | paired publication, orphan recovery, failed publication, bounded generation retention; real shutdown/wake |
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

## Completed checks

- `bun run typecheck`: passed.
- `bun run typecheck:web`: passed for all three frontend packages.
- `bun run test`: 893 passed, 11 conditionally skipped, 0 failed across 152 files.
- `bun run build`: passed.
- `bun run check:registry`: passed.
- Fork checks: inherited descriptors, partial file writes, detached processes,
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
Ubuntu 24.04 rootfs. Both direct and jailer launch modes passed.

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

As with the rest of the current web prototype, `web/prototype/cloud.ts` and the
gallery provide local state and simulated handlers. They do not call these REST
endpoints. Connecting the web prototype to backend authentication and live state
is product-wide integration work, described in `web-prototype.md`.

The tests above establish the named scenarios. They do not establish
production-scale capacity, exhaustive crash injection at every filesystem
operation, or a production deployment of the web prototype.
