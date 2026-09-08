# Demi Next: Acceptance Ledger

Status: documentation checkpoint, 2026-09-08. The records in this directory are
target contracts. Their presence does not establish implementation completion.
Historical implementation logs, review discussions and measurements are available
in Git; this ledger records acceptance against the current contract.

## Architecture acceptance

| Contract | State | Required evidence |
|---|---|---|
| TypeScript runner and JS commands on tinyjs | retained architecture; existing source inspected | scoped runner, loader and shared-schema checks when implementation changes |
| All production machine operations through remote Host and real bash | pending implementation acceptance | end-to-end scripted turns and filesystem operations on a runner |
| One managed device per user | pending implementation acceptance | database uniqueness, concurrent first use, multiple projects and user isolation |
| Persistent system and home | pending implementation acceptance | paired disk checkpoint, shutdown/wake and failure recovery on Linux/KVM |
| System reset with retained home | pending implementation acceptance | broken-guest reset, failure at each commit boundary and stable device/project identity |
| User-wide activity and capacity admission | pending implementation acceptance | active child, file upload, cross-host job, simultaneous wake/reset/shutdown races |
| Shared Cloud UI | pending implementation acceptance | product and gallery lifecycle/reset states, backend integration and error recovery |

## Documentation verification

The documentation checkpoint must check terminology, target ownership, storage
schema, command/runtime boundaries, internal references and whitespace. It does
not execute runtime tests or call models. New implementation checkpoints add
concrete commands, environments and results here after those checks actually run.
Benchmarks must identify their workload and platform; hypervisor microbenchmarks
are not evidence of product command-ready latency or capacity.

### 2026-09-08 documentation checkpoint — verified

- Audited all 19 records in this directory plus `docs/package-boundaries.md` for
  obsolete execution terminology and type/table references: no matches.
- Checked local document references, referenced section headings and balanced
  fenced blocks across those 20 documents: passed.
- Checked the target dependency graph against its package registry: identical
  package set, all dependencies named, no cycles.
- `git diff --check`: passed.

No implementation files changed and no runtime or model tests ran. The target
package graph intentionally specifies the desired package set; current workspace
manifest/source conformance remains implementation acceptance work.
