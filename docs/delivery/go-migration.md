# Migration to Go

**Status: complete (2026-10-04).** Gate G4 passed on `gomig/main` at
`f38fb7b3b`: no Rust remains, a release builds for all six targets, and the
real-machine acceptance passed (Chrome on macOS, Claude Code, and the arm64
Cloud suite, 6/6). The Linux x86_64 Cloud suite has not run here and is
needed before a release. `gomig/main` is ready to merge. The rest of this
document is the plan as it was carried out, kept as history.

This document is the plan for replacing Demi's Rust programs with Go
programs: who does the work, how the work is divided so that many agents
can work at once without touching each other's files, in what order the
parts are built, and how each part is accepted. It is a delivery document,
not a design contract. The Go design itself lives in the architecture
documents, which the first phase rewrites
([Phase 0](#phase-0-design-and-foundation)).

For example, the runner's shell is migrated like this: an agent receives the
work package `r-shell` and a worktree of its own, whose write boundary is
`internal/runner/shell/` and `third_party/mvdan-sh/`. It ports
`crates/runner-shell` and its tests, runs the package checks, commits on its
branch and reports. A script checks that every changed path lies inside the
boundary; the tech lead reviews the diff and merges it into `gomig/main`. When the whole runner is merged, the Rust
backend's own scenario tests run against the Go `demi-runner` binary; when
they pass, the runner is accepted.

## Scope and fixed decisions

These decisions are the owner's and are not reopened by this plan:

- Every Rust program moves to Go: the backend, the runner, the command
  programs (`demi-file`, `demi-browser`, `demi-claude-code`), the machine
  manager and `xtask`. The web app stays Vue and TypeScript; only the
  generator of its contract schemas changes.
- The Go code follows Go's own idioms, not a transliteration of the Rust.
- **No cgo, anywhere.** Every program builds with `CGO_ENABLED=0` for every
  target it ships on.
- **No cgo, with one exception for tests:** `go test -race` on Linux builds
  its test binary with `CGO_ENABLED=1`, because Go's race detector links
  ThreadSanitizer through libc there (macOS needs no cgo for it). Product
  builds and the cgo check are unaffected.
- The embedded standard utilities (uutils coreutils, findutils, diffutils,
  sed, grep, ripgrep, jaq) are not migrated. The runner's shell runs the
  system's own utilities, interpreted by a patched fork of `mvdan.cc/sh`.
- On macOS, the tree watch calls FSEvents through `github.com/ebitengine/purego`,
  in that one file only.
- Every test moves to Go: each work package ports its crate's Rust tests.
  Running Rust test binaries against Go programs is a transitional check
  that ends with the Rust code.
- Agents run Codex **without any sandbox**. Write boundaries are kept by the
  rules and checks in [Isolation rules](#isolation-rules).
- The migration and its acceptance are done by `gpt-6-astra` agents at the
  `low` reasoning level through the Codex CLI, as many at once as the work
  allows. Spikes are done by astra agents too.
- Until gate G4, only bug fixes land on `feat/demi-next`; each becomes a
  delta work package at the next gate.
- The tech lead (Claude) owns the design, the work division, the reviews of
  design, taste and conventions, the commits and the merges.
- The migration happens on a new branch, `gomig/main`, cut from
  `feat/demi-next`. The existing `port/*`, `go/*` and `feat/demi-next-go`
  branches play no part.

## Verified environment

Checked on 2026-10-02 on the development Mac (Apple M5 Max, 18 cores,
48 GiB, 664 GiB free).

| Fact | How it was checked |
|---|---|
| Codex CLI 0.160.0, the latest release, is installed through npm | `npm i -g @openai/codex@latest`; `codex --version` |
| `gpt-6-astra` offers the `low` effort | the Codex model catalog |
| 16 astra sessions run at once without rate limiting; 11 long spike sessions ran concurrently for over an hour without a limit error | parallel `codex exec`; their logs |
| Go 1.27.1 is installed | `brew install go`; `go version` |
| Rust's `vendor/` at the module root puts Go in vendor mode, so `go build` fails; `GOFLAGS=-mod=readonly` builds normally, until `z-remove-rust` removes `vendor/` | a module with one dependency |
| Codex's `workspace-write` sandbox would refuse what the migration needs: commits (it protects every `.git`), FSEvents streams, Chrome, `ps` and, by default, network access; agents therefore run without it | probe runs and spikes |
| A Rust test binary copied into a directory with a `.cargo-lock` file starts the programs it finds beside that file: 25 runner starts of the `backend-remote-host` suite went to a substitute `demi-runner` | relocation experiment, 56 of 57 tests passed; the one failure came from the substitute being a shell script that changed `argv[0]`, which the runner's command-alias mode dispatches on |
| `feat/demi-next` at `eb6158b3` does not build on macOS; `73198853` does, and `gomig/main` starts there | `cargo test --workspace --features demi-runner/test-fixtures --no-run` |
| A dedicated Linux VM, `gomig-spike` (Lima, Ubuntu 26.04 arm64, kernel 7.0, passwordless `sudo`), runs Linux-only work; the other VMs are not used | `limactl` |

## Package map

One Go module, `github.com/wspl/demi`, replaces the Cargo workspace. Each
crate becomes one package, or a package with subpackages where the crate's
modules can be understood and changed separately. Package names are short
and say what the package provides; `d-packages` makes this table the
authoritative package contract.

| Rust crate | Go package | Notes |
|---|---|---|
| `shared-types` | `internal/core` | The data the web app, the agent and the backend share |
| (none) | `internal/programtest` | Test support: builds the repository's programs once per test binary, or takes them from `DEMI_TEST_PROGRAMS` |
| (none) | `internal/contract` | The generic runtime the generated contract code calls: strict object reading, presence and duplicate checks, field-path errors |
| `conversation-socket-protocol` | `internal/framewire` | |
| `command-protocol` | `internal/commandwire` | |
| `command-declarations` | `internal/declare` | Argv parsing, help, schema checks |
| `runner-protocol` | `internal/runnerwire` | |
| `machine-manager-protocol` | `internal/machinewire` | |
| `web-api-protocol` | `internal/webapi` | |
| `command-package-file-protocol` | `internal/cmdpkg/file/fileop` | |
| `command-package-browser-protocol` | `internal/cmdpkg/browser/browserop` | |
| `command-package-claude-code-protocol` | `internal/cmdpkg/claudecode/claudecodeop` | |
| `command-sdk` | `internal/cmdsdk` | |
| `command-package-file` | `internal/cmdpkg/file` | |
| `command-package-browser`, `command-package-browser-chrome` | `internal/cmdpkg/browser`, `internal/cmdpkg/browser/chrome/{cdp,tabs,page,live}` | Split along the Rust modules |
| `command-package-claude-code` | `internal/cmdpkg/claudecode` | |
| `shared-gates` | `internal/gates` | |
| `shared-artifacts` | `internal/artifacts` | |
| `shared-cli` | `internal/cli` | |
| `host-interface` | `internal/host` | |
| `plugin-interface` | `internal/plugin` | |
| `plugin-*` | `internal/plugins/{browser,changes,expose,file,filebrowser,skills,todo}` | |
| `provider-common` | `internal/provider` | |
| vendor providers | `internal/providers/{anthropicapi,openaiapi,google,codex,grokbuild,claudecode}` | |
| `agent-*` | `internal/agent/{store,transcript,session,tools,server}` | |
| `runner`, `runner-*` | `internal/runner`, `internal/runner/{process,host,jobs,shell,cmdpkgs}` | |
| `machine-manager` | `internal/machines`, subpackages `sandbox`, `storage`, `network`, `system` | Linux only |
| `backend`, `backend-*` | `internal/backend`, `internal/backend/{accounts,blobs,cloud,database,edge,expose,hostaccess,idlewatch,pagesync,plugins,providers,remotehost,runners,usershard}` | |
| `xtask` | `tools/release`, `tools/contractgen`, `tools/archcheck`, `tools/cgocheck` | |
| `vendor/brush` | `third_party/mvdan-sh` | The patched `mvdan.cc/sh` fork ([decision 3](#owner-decisions)); the other vendored crates have no successor |
| binaries | `cmd/{demi-backend,demi-runner,demi-file,demi-browser,demi-claude-code,demi-machine-manager,demi-native-fixture}` | |

A crate's `testing` feature becomes a test-support package beside it, named
after it with a `test` suffix, as the standard library's `httptest` is:
`internal/provider/providertest`, `internal/gates/gatestest`. Tests live
beside the code in `_test.go` files; a suite that needs real programs or
real machines carries a build tag.

## Roles

| Role | Who | Does | Does not |
|---|---|---|---|
| Owner | The repository owner | Fixes scope and the decisions above | Review each work package or approve gates |
| Tech lead | Claude | Writes and keeps this plan and the work package briefs; decides the Go design; reviews every work package for design, taste and conventions; accepts a behavior change that is reasonable and refuses one that is not; merges and syncs branches; runs every gate | Write or fix code: review findings go back to the work package's agent; the tech lead touches only what a merge needs, such as `go.mod` |
| Implementer | An astra agent, one per work package | Ports one work package and its tests inside its write boundary, runs its checks, commits on its own branch, reports | Push, merge, touch files outside its boundary or any other branch, worktree or stash, change another package's exported API, add a module its brief does not list |
| Acceptance agent | An astra agent | Runs an acceptance suite against the built programs, triages each failure to the owning work package, writes the failure report | Fix the failure itself in another package's files |
| Spike agent | An astra agent | Answers one technical question with running code and measurements | Produce code that is merged |

## Branches and worktrees

```text
/Users/zan/Projects/demi                    feat/demi-next   the owner's checkout; never touched
/Users/zan/Projects/demi-worktrees/
  gomig-lead          gomig/main            integration: design documents, merges, gates (tech lead only)
  gomig-<wp>          gomig/<wp>            one per running work package, removed after its merge
  gomig-ref/                                outside git: reference programs, agent logs, briefs, reports
    bin/                                    Rust reference programs built from gomig/main's base
    generated/                              the TypeScript the Rust generator emits, for comparison
    runs/                                   each agent's prompt, JSONL events and last message
```

- `gomig/main` is the integration branch. Only the tech lead writes to it,
  in `gomig-lead`, and pushes it after each merge.
- Each work package gets a branch `gomig/<wp>` from the current
  `gomig/main` and a worktree `gomig-<wp>`. Work package branches stay local
  and are deleted after their merge.
- The agent commits on its own branch with Conventional Commit subjects. The
  tech lead merges an accepted branch into `gomig/main` with `--no-ff`, puts
  the report's summary in the merge commit, and pushes `gomig/main`.
- When a work package needs code merged after its branch was cut, the tech
  lead merges `gomig/main` into its branch; the agent then continues.

### One directory or many worktrees

The layouts differ in what an agent sees of the others' work, and, with no
sandbox, in how far a mistake reaches.

| Situation | One shared directory | One worktree per work package (chosen) |
|---|---|---|
| Agent A is halfway through `internal/agent/session` and the package does not compile; agent B, whose package imports it, runs `go test` | B's build fails on A's half-written code; B cannot tell whose fault it is and may waste its run on it | B builds against the last merged `session`, which compiles |
| Two agents' changes must be reviewed and accepted separately | The tech lead separates them by path in one working tree, and a revert has to pick paths out of shared commits | Each work package is one branch and one diff |
| An agent writes outside its boundary | The write lands in the tree everyone builds | The write stays in its worktree, and the boundary check refuses the branch |
| An agent's run goes wrong and has to be thrown away | Its files are mixed into the live tree that others build against | The worktree is removed; nothing else changed |
| An agent needs another package's latest merged work | It sees it at once | The tech lead merges `gomig/main` into its branch first |
| Disk and caches | One checkout | One checkout per work package, about 0.5 GB each without Rust build products; Go's build and module caches are shared, so builds stay incremental |

Worktrees cost a merge step and a little disk; a shared directory costs
builds that fail for reasons outside the agent's package. With up to sixteen
agents at once the second cost dominates, so every work package runs in its
own worktree.

## Isolation rules

A work package has a **write boundary**: the directories and files listed
in its brief. Agents run without a sandbox, so the boundary is kept by rules
the agent follows and by checks it cannot skip:

- **The boundary check.** Before a merge, `scripts/gomig/boundary.sh <wp>`
  lists every path the branch changed against `gomig/main`, committed or
  not, and refuses the branch if one lies outside the boundary.
- **The snapshot.** `scripts/gomig/agent.sh` records, before and after each
  run, the state the agent must not touch: the owner's checkout, every other
  worktree's status, every branch and tag but the agent's own, and the stash
  list. Any difference is a violation the tech lead resolves before anything
  else. The owner keeps working in the owner's checkout, so a difference
  there is compared by hand with what the work package touched; the probe
  run on 2026-10-02 showed exactly that case.
- **Git rules.** An agent commits only on its own branch in its own
  worktree. It never pushes, merges, rebases, deletes a branch, uses
  `git stash` (the stash is shared by every worktree) or runs
  `git worktree`.
- **Processes.** An agent stops every process it started (Chrome, servers,
  VM jobs) before it reports; the report names any it could not stop.

A package directory has exactly one owner at a time. Two running work
packages never share a directory, so their merges never conflict.

- **Modules.** `go.mod` and `go.sum` belong to the tech lead. A brief lists
  the modules its agent may add; the boundary check reports every change to
  `go.mod` for the review. When two branches add modules, the tech lead
  resolves the merge with `go mod tidy`.
- **APIs.** A package's exported API belongs to the package's owner. A
  consumer that needs a change describes it in its report; the tech lead
  decides and routes it to the owner, never to the consumer.
- **Registries.** The Go dependency graph in `crates-and-packages.md`, which
  the import check reads, belongs to the tech lead; `cmd/<program>` belongs
  to the program's assembly work package. The generator needs no root list:
  a root is a marker on its type.
- **Generated code.** Generated Go is committed in the package that owns its
  source types, so it belongs to that package's owner. The generated
  TypeScript is not committed, as today.
- **Documents.** A documentation work package owns named files. Two running
  work packages never edit the same file.
- **Linux VMs.** Two work packages never share a VM, because their
  privileged tests change the same kernel state (mounts, loop devices,
  firewall tables).

## Work package lifecycle

```text
tech lead: brief + worktree + branch
   |
   v
astra: port -> checks -> commit -> REPORT ---> boundary check -> tech lead review
   ^                                                  |
   |   review findings (codex exec resume <session>)  | changes needed
   +--------------------------------------------------+
                                                      | accepted
                                                      v
                                   tech lead: merge into gomig/main -> push
```

1. **Brief.** The tech lead writes the brief from a template: the Rust
   sources and documents to port, the write boundary, the API the package
   must export (from `crates-and-packages.md` and the Rust public items),
   the dependencies it may use, the tests to port or write, and the checks
   that define done.
2. **Run.** `scripts/gomig/agent.sh` creates the worktree, takes the
   snapshot and starts the agent with `CGO_ENABLED=0` and `GOFLAGS=-p=2`,
   recording its events in `gomig-ref/runs/`; when the agent exits it takes
   the snapshot again and reports any difference.
3. **Checks before the report.** The agent runs `scripts/gomig/check.sh` on
   its packages: `go build` for every target they ship on, `go vet`,
   `golangci-lint`, the cgo check, the import check, the exhaustiveness
   check, and `go test -race` for its packages and those that import them.
   A report without passing checks is not reviewed.
4. **Review.** The tech lead runs the boundary check, then reviews the diff
   against the brief and the [review checklist](#review-checklist). Findings
   go back to the same agent session with `scripts/gomig/resume.sh`, so it
   keeps its context.
5. **Merge.** The tech lead merges, runs the whole-tree checks in
   `gomig-lead`, pushes, and removes the worktree.

A package with dependents is delivered in **two checkpoints**: first its
**API checkpoint** (exported types, functions and interfaces with doc
comments, the test fakes its dependents need, bodies that are not written
yet), then its **implementation checkpoint**. Dependents start as soon as the
API checkpoint is merged. A body not written yet panics with a message that
names the work package; the cutover gate fails while any such panic remains.

### Review checklist

The tech lead checks every work package for:

- **Boundary.** The package owns what its entry in the packages document
  says and nothing more; no import against the dependency direction; no
  plugin or product knowledge below where the design puts it.
- **Go idiom.** Small interfaces declared where they are used; constructors
  return concrete types; `context.Context` first on everything that waits;
  errors returned and wrapped with `%w`, compared with `errors.Is` and
  `errors.As`, never by text; no getters named `Get`; package names short,
  no stutter (`gates.Activity`, not `gates.ActivityGate`); no panics across
  package boundaries; standard library first.
- **Ownership and cleanup.** Every goroutine has an owner that cancels and
  waits for it; every lease, gate permit, timer, listener and process is
  released on success, failure and cancellation, with `defer` at the point
  of acquisition; no lock held across a blocking call.
- **Contracts.** Values from outside the process are decoded and validated
  at entry by the contract package's generated decoder; no hand-written
  second declaration of a contract shape.
- **Tests.** The rules of `docs/delivery/testing.md`: behavior at its
  boundary, once; no sleeps (`testing/synctest` or events); the planted
  defect fails it; no assertion library beyond `go-cmp`.
- **Fidelity.** The Go code does what the Rust code does: nothing left out,
  nothing added. A difference is allowed only as a bug fix or a reasonable
  normalization, named in the report, and accepted by the tech lead; anything
  else goes back. The report's three fidelity tables (each Rust test to its Go
  test, each public Rust item to its Go identifier, and every Go behavior with
  no Rust counterpart) are checked line by line against the Rust source, and
  the diff is read for logic that none of them names.

## Roadmap

The work runs in five phases, each closed by a gate that the tech lead runs.
Inside a phase, work packages start as soon as the API checkpoints they
depend on are merged, not when the whole previous level is done.

```text
Phase 0  design + foundation        8 at once     gate G0: the Go design accepted
Phase 1  contracts + leaf libraries up to 8       gate G1: generated TypeScript and corpora match
Phase 2  API checkpoints, then      up to 16      gate G2: every package implemented,
         implementations                                   unit tests pass
Phase 3  programs + acceptance      up to 12      gate G3: Rust suites pass against Go programs;
                                                           all-Go suites pass
Phase 4  cutover                    up to 10      gate G4: Rust removed, release accepted
```

### Phase 0: design and foundation

The architecture documents describe Rust. Before any Go is written, they
are rewritten for Go, because each work package implements against them.

| Work package | Write boundary | Output | Depends on |
|---|---|---|---|
| `d-contracts` | `docs/architecture/contracts.md` | The Go contract design: Go types with markers as the only definition, the generator, encodings, validation at entry | sp01, sp02 |
| `d-concurrency` | `docs/architecture/concurrency.md` | The Go concurrency design: the user shard, gates, ownership, cancellation, blocking work, tests and time | sp10 |
| `d-packages` | `docs/architecture/crates-and-packages.md` | The package contract: the package map below, each package's ownership, the dependency graph, module layout, boundary checks | the package map |
| `d-delivery` | `docs/delivery/testing.md`, `docs/delivery/builds-and-releases.md` | Go test levels, commands and costs; Go builds for the six targets and releases | sp11 |
| `d-execution` | `docs/execution/runner.md`, `docs/execution/edit-tracking.md`, `docs/execution/commands.md`, `docs/execution/native-runtime.md` | The shell on the chosen interpreter with the system's utilities, what edit tracking records now, the platform report to the model, the stream reset on a service abort | decisions 3 and 4 |
| `f-skeleton` | `cmd/`, `internal/`, `tools/archcheck/`, `tools/cgocheck/`, `.golangci.yml`, `scripts/gomig/check.sh` | Every package directory with its package comment, the guard rails, the package check script | the package map |
| `f-contractgen` | `tools/contractgen/`, `internal/contract/` (outside `f-skeleton`'s boundary) | The generator and the encoding helpers every contract package uses, proven on the shared-types fixtures | sp01, sp02 |
| `f-harness` | `scripts/gomig/accept/` | The transitional harness that runs Rust test binaries against substituted programs, proven green with the Rust programs in place | none |

The tech lead writes the Go section of `AGENTS.md`, the brief and report
templates and the package map, creates `go.mod`, and reviews the
documents. The Lima VMs for Linux work (`gomig-vm-1` to `gomig-vm-3`) are
created before Phase 2. **Gate G0:** the tech
lead accepts the rewritten architecture documents and every behavior change
they make.

### Phase 1: contracts and leaf libraries

Every contract package is complete after this phase; the leaf libraries
have no other Demi dependency, so they are complete too.

| Level | Work packages |
|---|---|
| a | `c-core` (shared-types), `c-commandwire` (command-protocol), `l-declare` (command-declarations), `l-gates`, `l-artifacts`, `l-cli`, `c-claudecodeop` |
| b | `c-framewire`, `c-fileop`, `c-browserop`, `c-runnerwire`, `l-cmdsdk`, `l-host`, `l-provider`, `l-idlewatch` |
| c | `c-webapi`, `c-machinewire`, `l-plugin` |

**Gate G1:** the Go generator's TypeScript for every root equals what the
Rust generator emits, apart from differences the tech lead approved; the
web app type-checks against it; every golden corpus round-trips byte for
byte; `go test -race ./internal/...` passes.

### Phase 2: API checkpoints, then implementations

| Group | Work packages |
|---|---|
| Runner | `r-process`, `r-host`, `r-shell`, `r-cmdpkgs`, `r-jobs`, `r-runner` |
| Command programs | `k-file`, `k-claudecode`, `k-chrome-cdp`, `k-chrome-tabs`, `k-chrome-page`, `k-chrome-live`, `k-browser` |
| Machine manager | `m-core` (config, server, manager), `m-sandbox`, `m-storage`, `m-network`, `m-system` (the Linux interfaces and tool runs they share) |
| Providers | `p-anthropic`, `p-openai`, `p-google`, `p-codex`, `p-grok`, `p-claudecode` |
| Agent | `a-store`, `a-transcript`, `a-session`, `a-tools`, `a-server` |
| Plugins | `g-browser`, `g-expose`, `g-skills`, `g-small` (changes, file, file browser, todo) |
| Backend | `b-database`, `b-remotehost`, `b-blobs`, `b-accounts`, `b-pagesync`, `b-expose`, `b-runners`, `b-cloud`, `b-providers`, `b-plugins`, `b-hostaccess`, `b-usershard`, `b-edge`, `b-backend` |
| Tools | `t-release` (packaging, Cloud image, development store, Chrome pin), `t-pages` (the plugin page registry that `bun run contracts` writes for `web` and `web-gallery`, in `tools/contractgen`, after the plugins' API checkpoints) |
| Documents | `d-hostaccess` (`docs/execution/sessions-and-targets.md` § Host operations for the Go shard and leases, before `b-hostaccess`) |

The API checkpoints follow the dependency graph, so they form short levels:
each takes one agent run, and the longest chain, from `l-host` to
`b-backend`, is about ten levels. The implementation checkpoints then run
side by side. **Gate G2:** no package panics as not written; every package's
tests pass; the whole tree passes the guard rails on all targets.

### Phase 3: programs and acceptance

| Work package | Accepts | How |
|---|---|---|
| `x-commands` | `demi-file`, `demi-browser`, `demi-claude-code` | Their ported Go tests; early on, the Rust runner and command-sdk suites with the Go programs substituted |
| `x-runner` | `demi-runner` | Its ported Go tests; early on, the Rust backend scenarios, `backend-remote-host` and `backend-plugins` suites with the Go runner and Go command programs substituted |
| `x-machines` | `demi-machine-manager` | The Go manager's tests and the Cloud real-machine suite in the Lima VM |
| `x-webapp` | `demi-backend` | The web app contract suite (`bun run test` with `DEMI_TEST_PROGRAMS` pointing at the Go programs) |
| `s-scenarios-*` | `demi-backend` | The Rust backend scenarios (`crates/backend/tests`) ported to Go, one work package per scenario file group |
| `x-real` | Everything | The real-machine suites, all-Go: Chrome (in a Linux VM as an ordinary user, and by the tech lead on macOS), Claude Code, Cloud |

An acceptance agent never fixes another package. It writes a failure
report naming the owning work package, and the tech lead reopens that
package with the report. **Gate G3:** every Go suite passes, the web app
contract suite passes against the Go programs, and no Rust test is left
without its Go port.

### Phase 4: cutover

| Work package | Output |
|---|---|
| `z-remove-rust` | `crates/`, `vendor/`, `Cargo.toml`, `Cargo.lock`, `.cargo/`, `rust-toolchain.toml` and `scripts/native/Dockerfile` (the Rust cross-build container) removed; `package.json` scripts and `tsconfig.json` call and include Go-era paths only; `cloud-guest-image` builds from the Go programs; the transitional harness, the Rust reference build and `GOFLAGS=-mod=readonly` removed |
| `z-docs-*` | The behavior documents freed of Rust specifics, one work package per document group |
| `z-release` | The six-target release and the Cloud image built from Go |

Not everything under `crates/` is Rust. These move with the work package
that owns their Go successor, before `z-remove-rust` deletes the directory:
the golden corpora and fixtures (`crates/*/tests/**/fixtures`, into the
owner's `testdata/`), the browser page scripts and the capture extension
(JavaScript in `crates/command-package-browser-chrome/src/page` and
`src/driver/capture`), the Chrome for Testing release record
(`crates/command-package-browser-protocol/src/release`), and the machine
manager's scripts, runtime files and Lima configuration
(`crates/machine-manager/{scripts,runtime,lima}`). The TypeScript tests that
read fixtures from `crates/`, such as
`packages/conversation-client/src/__tests__/patches.test.ts`, are pointed at
the new location by the same work package.

The tech lead rewrites `AGENTS.md` for Go only. **Gate G4:** no Rust
remains: `git ls-files` lists no `*.rs` file, no Cargo or rustup file, and no
script, document or test that runs or names `cargo`, `rustc` or `crates/`
except as history; a release builds for all targets; the real-machine
acceptance passes. `gomig/main` is then ready to merge.

### Concurrency and pace

How many agents run at once is limited by what is ready, not by the tools:
sixteen sessions ran at once without a rate limit, and the machine builds
with eighteen cores (each agent builds with `GOFLAGS=-p=2`). Linux work
packages are limited by VMs: one VM per running work package, at most three
at once (each 4 CPUs and 6 GiB).

```text
agents at once
16 |                    ############
12 |                  ##############  ######
 8 |        ######  ################  ########
 4 | #######################################################
   +-------------------------------------------------------------
     Phase 0   Phase 1   Phase 2 API -> impl   Phase 3   Phase 4
```

*Estimate, to be calibrated in Phase 1:* the Rust to port is 152 thousand
lines of production code and 83 thousand lines of tests. If an agent ports
about a thousand lines of Go with its tests per hour, the work is about 250
agent-hours; at twelve agents at once that is about 21 hours of agent time,
plus review rounds, the ten short levels of API checkpoints and the
acceptance loops of Phase 3. That suggests one to two weeks from gate G0 to
gate G3, set mostly by how fast the tech lead reviews: about a hundred
checkpoints pass through review. Phase 1 measures the real lines per
agent-hour and review rounds per checkpoint, and this section is updated.

## Acceptance strategy

The wires between Demi's programs do not depend on the language: MessagePack
over a WebSocket to the runner, HTTP/2 to the command programs, JSON lines
to the machine manager, HTTP and WebSockets to the web app. Every Rust test is
ported to Go, and the Go tests are what accept a program. Until a program's
Go tests at that level exist, the existing tests of the program on the
other end of its wire give an early signal; this ends with the Rust code.

```text
Rust test binary (unchanged)                 Go program under test
  backend scenarios, remote-host suite  --->  demi-runner (Go)
  runner and command-sdk suites         --->  demi-file, demi-browser, demi-claude-code (Go)
  Cloud real-machine suite (Rust backend) ->  demi-machine-manager (Go), in the Lima VM
web app contract suite (TypeScript)     --->  demi-backend (Go) + demi-runner (Go)
```

The harness copies each Rust test binary into a directory that holds a
`.cargo-lock` file and the programs under test, because a Rust test starts
the programs it finds beside the profile directory's `.cargo-lock`. The
reference build lives in `gomig-lead/target` and is rebuilt only when
`gomig/main` merges `feat/demi-next`.

## Keeping up with feat/demi-next

`feat/demi-next` keeps moving while the migration runs: two commits landed
on the day this plan was written. At each gate, the tech lead merges
`feat/demi-next` into `gomig/main`, lists the changes under `crates/` and
`docs/`, and opens a delta work package for each package they touch.

## Spike results

Eleven spikes ran at once on 2026-10-02, and a twelfth (sp04b) followed one of them, each an astra agent in its own
worktree; their code and full reports stay on the local branches
`gomig/sp*` and are never merged. The spikes ran in Codex's sandbox; the tech lead
re-ran outside it the two experiments the sandbox refused (FSEvents and
Chrome).

| Spike | Question | Verdict | What the migration does |
|---|---|---|---|
| sp01 contracts | Go types as the only contract definition, with a generator | Works with caveats: on the transcript-block family, 26 of 26 fixture blocks round-trip, 42 of 42 refused mutations are refused, and the generated Zod has no validation difference from the Rust-generated Zod for any of the 31 shared schemas; a removed `switch` case fails the exhaustiveness check | The Go-first design ([Contracts decisions](#tech-lead-decisions)) |
| sp02 wire | Runner MessagePack, machine-manager lines and the manifest digest, byte for byte | Works with caveats: 25 runner fixtures, the kept-record corpus, all 18 machine-manager lines and the manifest and package digests match | `github.com/vmihailenco/msgpack/v5` v5.4.1 with compact integers, declaration order, a sorted encoder for string maps and non-nil empty byte slices; `github.com/gowebpki/jcs` v1.0.2 for RFC 8785; encoders and decoders generated from the same Go types as sp01 |
| sp03 HTTP/2 | The command wire in Go, interoperating with Rust | Works with caveats: streaming, client cancellation and bounded flow control interoperate both ways; a handler cannot choose `RST_STREAM(CANCEL)` for a service-side abort | `net/http` with unencrypted HTTP/2, one owned connection per service (`Transport.NewClientConn`), a 64 KiB stream window, the concurrent-stream limit and early-reset guard configured. The command wire says a service abort resets the stream and stops naming the code |
| sp04 shell | The runner's shell on `mvdan.cc/sh` with the system's utilities | Does not work unmodified: commands and pipelines work, 50 concurrent jobs complete, cancellation by process group works, but v3.14.1 cannot join every task a job started at any depth and cannot cancel an unopened process substitution; 34 of 40 typical agent snippets print what `bash` prints | See sp04b |
| sp04b shell fork | Can a small fork close those gaps and keep edit tracking of redirections? | Works with caveats: a patch of 311 added and 43 removed lines in nine files of `interp` joins every nested task, cancels used and unused process substitutions, and hands every writable redirection (`>`, `>>`, `<>`, `exec 3>f`) in all seven scopes to the open handler, 28 of 28 cases; each new test fails on the unpatched library. On macOS a FIFO's end of file needs a 10 ms recheck | [Decision 3](#owner-decisions) |
| sp05 namespaces | The machine manager's Linux primitives without cgo | Works with caveats: mount and network namespaces, recovery into a saved mount namespace, veth, nftables, loop devices, `FIFREEZE`/`FITHAW`, sparse copy and `sd_notify` all ran in the VM | One namespace-job entry point: a goroutine locks its thread, unshares `CLONE_FS`, does the whole job and exits without unlocking. Recovery starts `/proc/self/exe` from that entered thread, which `os.StartProcess` documents. `nftables.WithNetNSFd`; not `netlink.NewHandleAt`, whose restore path ignores an error; the two freeze ioctl numbers defined per architecture |
| sp06 FSEvents | Watching repository trees on macOS without cgo | Works, through purego: outside the sandbox 100 events arrived with a median latency of 11 ms, and 200 start-stop cycles left threads, descriptors and memory flat. Inside the Codex sandbox the stream does not start. A full `lstat` walk of this repository takes 18 to 25 s, too slow to replace a watch | purego for FSEvents ([decision 1](#owner-decisions)); `fsnotify` on Linux and Windows |
| sp07 git | go-git for working-tree changes and skill fetches | Works with caveats: with an adapter, staged and unstaged changes, untracked files, renames and line counts matched `git` on this repository and on Kubernetes; `Worktree.Status()` itself is wrong on a restored-mtime edit and slow. A full status of Kubernetes takes 6.5 to 6.8 s with go-git's status, 1.9 s with the spike's parallel walk, and 0.23 s with the `git` CLI; this repository takes 0.13 to 0.15 s with the walk. A shallow HTTPS fetch of a skill source took 4.4 s | go-git for objects, the index and the transport; Demi's own parallel walk over a watched baseline, as the runner design already requires; line counts with a Myers diff |
| sp08 Chrome | The conversation browser on chromedp | Works on cdproto, not on chromedp's actions: outside the sandbox, an unpacked extension, three tabs driven at once, a 9 MB full-page screenshot, acknowledged screencast frames, network and console events and a download all worked, with no goroutine left. chromedp itself lacks a bound on message size and the child-session guarantees Demi needs. Chrome does not start inside the Codex sandbox | `github.com/chromedp/cdproto` types behind Demi's own `cdp.Executor`: a bounded transport and a session router of Demi's, about 0.9 to 1.6 thousand lines for today's `src/cdp` |
| sp09 SQLite | A cgo-free SQLite for the backend | Works with caveats: the real schemas and statements ran on both `modernc.org/sqlite` and `ncruces/go-sqlite3`, and each driver reopened the other's files | `modernc.org/sqlite` v1.60.1 behind `database/sql`: one `sql.DB` with one connection per writable database, every statement inside its `sql.Tx`, short read-only connections for cold reads, the global limit of 64 open writers kept as an explicit LRU. Appends ran at 3,884 transactions/s with a p99 of 0.65 ms |
| sp10 shard | The user shard's guarantees in Go | Works with caveats: a mutex variant and an actor variant both held admission atomic under 10,000 interleaved races with `-race`, fired the idle watch at exactly 3,600 s of `synctest` time, and caught a forgotten lease | One `sync.Mutex` per shard with short critical sections; gates on `golang.org/x/sync/semaphore` (FIFO); leases released with `defer`, idempotent, and audited at shutdown in tests; `goleak` and `testing/synctest` |
| sp11 matrix | Six targets without cgo, sizes, guard rails | Works: all 18 program and target builds pass with `CGO_ENABLED=0`, statically linked on Linux, in 64 s from clean and 5.5 s with no change. The runner skeleton is 10.5 MiB on macOS arm64 | Guard rails: the cgo check per target, an import-direction table checked over `go list -deps -json`, `go-check-sumtype -default-signifies-exhaustive=false`, `golangci-lint` v2.14.0 |

### Codex sessions

A resumed session needs its first run to have exited: `codex exec resume`
fails while the original process still writes the session. `codex queue` does not reach a running `codex exec`: the run ends its
turn and exits without reading the queued message, so new instructions reach
a running agent only through the next `resume.sh` round.

## Tech lead decisions

These follow from the spikes and are the input to the Phase 0 documents.

- **Contracts are Go types.** Plain structs with `json` tags; a union is a
  sealed interface whose variants are pointer types; bounds, nullability,
  strictness and custom rules are `+demi:` marker comments. `tools/contractgen`
  reads them with `golang.org/x/tools/go/packages` and writes, into each
  contract package, the decoders, encoders, `Validate` methods and the Zod.
  Generated Go is committed. `go-check-sumtype` keeps every `switch` over a
  union exhaustive.
- **Stable `encoding/json` only.** `encoding/json/v2` is still behind
  `GOEXPERIMENT=jsonv2` in Go 1.27, so the generated decoders build on the
  stable package, refuse duplicate keys and check UTF-8 at the boundary, as
  serde does. Moving to v2 when it is stable is a regeneration.
- **A timestamp's spelling is the documented one.** UTC, three fractional
  digits; the Rust parser accepted more spellings than the contract documents,
  and every stored time was written in the documented one.
- **Validity of an identifier is a boundary guarantee.** Go cannot stop
  `core.BlockID("")`; decoders validate, and code that makes an identifier
  calls its generated `Parse` constructor. The contracts document says so.
- **A service abort resets the stream.** The command wire stops naming
  `RST_STREAM(CANCEL)` for a service-side abort, because Go's HTTP/2 server
  does not let a handler choose the code.
- **Line counts come from a Myers diff.** They are display figures on file
  pills; a histogram diff is not available in pure Go.
- **go-git is the stable v5 line.** In `r-host`'s API checkpoint v5.19.2
  passed the spike's status comparisons and a shallow HTTPS fetch; it fails
  only a shallow clone from a local repository without a git executable, which
  no product path makes: skill sources are GitHub or `https` repositories
  ([Skills](../agent/skills.md)). v6 is still an alpha.
- **One library per job, chosen once:** `github.com/coder/websocket` for
  every WebSocket (Codex, Chrome's CDP, the runner link, the web app);
  `github.com/sergi/go-diff` with interned lines and no timeout for line
  counts, in `internal/runner/process`; the standard library with
  `golang.org/x/image` (WebP decoding, scaling) for images.
- **YAML is `github.com/goccy/go-yaml`**, in strict mode where Rust's
  `serde_saphyr` deserializes into a typed struct: skill front matter is its
  only use.
- **Phase 2 does not wait for gate G1.** A Phase 2 work package starts when
  every package its graph line names is merged; G1 still closes Phase 1, and
  a G1 finding goes back to the contract package that owns it.
- **Tests use the standard library**, `github.com/google/go-cmp`,
  `go.uber.org/goleak` and `testing/synctest`; no assertion library.

## Owner decisions

Decided on 2026-10-02:

1. **purego for FSEvents** on macOS, in the tree watch's darwin file only.
2. **`-race` on Linux** builds its test binary with `CGO_ENABLED=1` and
   `-tags netgo,osusergo`, so the standard library keeps its pure-Go
   resolver and user lookup as in the shipped build; macOS runs `-race` with
   cgo off. Products are never built with cgo.
3. **The shell interpreter is a patched fork of `mvdan.cc/sh`** in
   `third_party/mvdan-sh`, behind a `replace` directive, with its patch kept
   as a file so a rebase starts from it; `r-shell` owns both. sp04b showed
   the patch is 354 lines and keeps declared commands as builtins, one shell
   dialect on every platform, and edit tracking of redirections.
4. **What the system's utilities cost users is pending, and must be done
   later**, not in this migration: edit tracking of files that `sed -i`,
   `tee` and `sort -o` write; the BSD utilities of a paired Mac; a Windows
   device without Unix utilities; GNU coreutils, findutils, diffutils, sed,
   grep, ripgrep and jq in the Cloud image; and telling the model which
   shell, platform and utilities a job has. The design documents name each
   as an open item.
5. **Only bug fixes land on `feat/demi-next`** until gate G4.
