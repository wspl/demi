# Migration to Go

This document is the plan for replacing Demi's Rust programs with Go
programs: who does the work, how the work is divided so that many agents
can work at once without touching each other's files, in what order the
parts are built, and how each part is accepted. It is a delivery document,
not a design contract. The Go design itself lives in the architecture
documents, which the first phase rewrites
([Phase 0](#phase-0-design-and-foundation)).

For example, the runner's shell is migrated like this: an agent receives the
work package `r-shell` and a worktree of its own, in which its sandbox lets
it write only under `internal/runner/shell/`. It ports
`crates/runner-shell`, runs the package checks, and reports. The tech lead reviews the diff, commits it on the package's branch
and merges it into `gomig/main`. When the whole runner is merged, the Rust
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
- The embedded standard utilities (uutils coreutils, findutils, diffutils,
  sed, grep, ripgrep, jaq) are not migrated. The runner's shell runs the
  system's own utilities.
- The migration and its acceptance are done by `gpt-6-astra` agents at the
  `low` reasoning level through the Codex CLI, as many at once as the work
  allows. Spikes are done by astra agents too.
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
| In the `workspace-write` sandbox an agent can write only its working directory and the directories given with `--add-dir`; a write into another worktree fails with `Operation not permitted` | probe runs |
| Starting an agent with `-C <worktree>/<package dir>` confines its writes to that directory: it cannot write `go.mod` at the root | probe run |
| The sandbox blocks network access by default; `-c sandbox_workspace_write.network_access=true` enables it, which `go get` needs | probe runs |
| The sandbox protects every `.git` directory: an agent cannot commit, stash, reset or check out, even with `--add-dir` on the repository's `.git` | probe run |
| The Go build and module caches outside the worktree need `--add-dir ~/Library/Caches/go-build --add-dir ~/go` | probe run |
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
| `machine-manager` | `internal/machines`, subpackages `sandbox`, `storage`, `network` | Linux only |
| `backend`, `backend-*` | `internal/backend`, `internal/backend/{accounts,blobs,cloud,database,expose,hostaccess,http,idlewatch,pagesync,plugins,providers,remotehost,runners,usershard}` | |
| `xtask` | `tools/release`, `tools/contractgen`, `tools/archcheck`, `tools/cgocheck` | |
| `vendor/brush` | `third_party/mvdan-sh` | The patched `mvdan.cc/sh` fork, if [decision 3](#open-decisions-for-the-owner) is (A); the other vendored crates have no successor |
| binaries | `cmd/{demi-backend,demi-runner,demi-file,demi-browser,demi-claude-code,demi-machine-manager,demi-native-fixture}` | |

A crate's `testing` feature becomes a test-support package beside it, named
after it with a `test` suffix, as the standard library's `httptest` is:
`internal/provider/providertest`, `internal/gates/gatestest`. Tests live
beside the code in `_test.go` files; a suite that needs real programs or
real machines carries a build tag.

## Roles

| Role | Who | Does | Does not |
|---|---|---|---|
| Owner | The repository owner | Fixes scope and the decisions above; approves the Go design at gate G0 and the cutover at gate G4 | Review each work package |
| Tech lead | Claude | Writes and keeps this plan and the work package briefs; decides the Go design; reviews every work package for design, taste and conventions; commits, merges and syncs branches; runs the gates | Write production code itself, apart from small fixes found in review |
| Implementer | An astra agent, one per work package | Ports one work package inside its write boundary, writes its tests, runs its checks, reports | Commit, touch files outside its boundary, change another package's exported API, add a dependency |
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
- The tech lead commits a work package on its branch (agents cannot), with a
  Conventional Commit subject and the report's summary in the body, merges it
  into `gomig/main` with `--no-ff`, and pushes `gomig/main`.
- When a work package needs code merged after its branch was cut, the tech
  lead merges `gomig/main` into its branch; the agent then continues.

### One directory or many worktrees

Both layouts can confine an agent's writes to its own package directory,
because the sandbox's writable root is the agent's working directory. They
differ in what an agent sees of the others' work.

| Situation | One shared directory | One worktree per work package (chosen) |
|---|---|---|
| Agent A is halfway through `internal/agent/session` and the package does not compile; agent B, whose package imports it, runs `go test` | B's build fails on A's half-written code; B cannot tell whose fault it is and may waste its run on it | B builds against the last merged `session`, which compiles |
| Two agents' changes must be reviewed and accepted separately | The tech lead separates them by path in one working tree, and a revert has to pick paths out of shared commits | Each work package is one branch and one diff |
| An agent's run goes wrong and has to be thrown away | Its files are mixed into the live tree that others build against | The worktree is removed; nothing else changed |
| An agent needs another package's latest merged work | It sees it at once | The tech lead merges `gomig/main` into its branch first |
| Disk and caches | One checkout | One checkout per work package, about 0.5 GB each without Rust build products; Go's build and module caches are shared, so builds stay incremental |

Worktrees cost a merge step and a little disk; a shared directory costs
builds that fail for reasons outside the agent's package. With up to sixteen
agents at once the second cost dominates, so every work package runs in its
own worktree.

## Isolation rules

A work package has a **write boundary**: one or more package directories,
listed in its brief. The boundary is enforced, not just requested.

- The agent runs with `-C <worktree>/<first directory>` and an `--add-dir`
  for each further directory of its boundary, plus the Go caches. It can
  read the whole repository and `gomig-ref`; it can write nothing else.
- A package directory has exactly one owner at a time. Two running work
  packages never share a directory, so their merges never conflict.
- `go.mod` and `go.sum` belong to the tech lead. Phase 0 pins every
  dependency the spikes chose. An agent that needs another module stops and
  says so in its report; the tech lead adds it to `gomig/main` and syncs the
  branch.
- A package's exported API belongs to the package's owner. A consumer that
  needs a change describes it in its report; the tech lead decides and
  routes it to the owner, never to the consumer.
- Shared registries have one owner: the dependency table of the import
  check and the generator's root list belong to the tech lead; `cmd/<program>`
  belongs to the program's assembly work package.
- Generated Go code is committed in the package that owns its source types,
  so it belongs to that package's owner. The generated TypeScript is not
  committed, as today.
- Documents: each documentation work package owns named files. Two running
  work packages never edit the same file.

## Work package lifecycle

```text
tech lead: brief + worktree + branch
   |
   v
astra: port -> package checks -> REPORT -----> tech lead review
   ^                                                  |
   |   review findings (codex exec resume <session>)  | changes needed
   +--------------------------------------------------+
                                                      | accepted
                                                      v
                       tech lead: commit on gomig/<wp> -> merge into gomig/main -> push
```

1. **Brief.** The tech lead writes the brief from a template: the Rust
   sources and documents to port, the write boundary, the API the package
   must export (from `crates-and-packages.md` and the Rust public items),
   the dependencies it may use, the tests to port or write, and the checks
   that define done.
2. **Run.** `scripts/gomig/agent.sh` creates the worktree and starts the
   agent with the sandbox, the boundary, `CGO_ENABLED=0` and
   `GOFLAGS=-p=2`, recording its events in `gomig-ref/runs/`.
3. **Checks before the report.** The agent runs the package checks:
   `go build` of its packages for every target they ship on, `go vet`,
   `golangci-lint`, the cgo check, the import check, and `go test -race`
   for its packages and those that import them. A report without passing
   checks is not reviewed.
4. **Review.** The tech lead reviews the diff against the brief and the
   [review checklist](#review-checklist). Findings go back to the same agent
   session with `codex exec resume`, so it keeps its context.
5. **Merge.** The tech lead commits, merges, runs the whole-tree checks in
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
- **Fidelity.** Behavior matches the design documents and the Rust tests it
  ports; any deliberate difference is named in the report and approved.

## Roadmap

The work runs in five phases, each closed by a gate that the tech lead runs.
Inside a phase, work packages start as soon as the API checkpoints they
depend on are merged, not when the whole previous level is done.

```text
Phase 0  design + foundation        8 at once     gate G0: owner approves the Go design
Phase 1  contracts + leaf libraries up to 8       gate G1: generated TypeScript and corpora match
Phase 2  API checkpoints, then      up to 16      gate G2: every package implemented,
         implementations                                   unit tests pass
Phase 3  programs + acceptance      up to 12      gate G3: Rust suites pass against Go programs;
                                                           all-Go suites pass
Phase 4  cutover                    up to 10      gate G4: owner approves; Rust removed
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
| `f-skeleton` | the whole worktree; the review holds it to `go.mod`, `go.sum`, `cmd/`, `internal/*/doc.go`, `tools/archcheck/`, `tools/cgocheck/`, `.golangci.yml` and `scripts/gomig/check.sh` | The module with every dependency the spikes chose, every package directory with its package comment, the guard rails, the package check script | the package map |
| `f-contractgen` | `tools/contractgen/`, `internal/contract/` | The generator and the encoding helpers every contract package uses, proven on the shared-types fixtures | sp01, sp02 |
| `f-harness` | `scripts/gomig/accept/` | The cross-language acceptance harness, proven green with the Rust programs in place | none |

The tech lead writes the Go section of `AGENTS.md`, the brief and report
templates and the package map, creates the Lima VMs `gomig-vm-1` to
`gomig-vm-3` for Linux work, and reviews the documents. **Gate G0:** the
owner approves the rewritten architecture documents.

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
| Machine manager | `m-core` (config, server, manager), `m-sandbox`, `m-storage`, `m-network` |
| Providers | `p-anthropic`, `p-openai`, `p-google`, `p-codex`, `p-grok`, `p-claudecode` |
| Agent | `a-store`, `a-transcript`, `a-session`, `a-tools`, `a-server` |
| Plugins | `g-browser`, `g-expose`, `g-skills`, `g-small` (changes, file, file browser, todo) |
| Backend | `b-database`, `b-remotehost`, `b-blobs`, `b-accounts`, `b-pagesync`, `b-expose`, `b-runners`, `b-cloud`, `b-providers`, `b-plugins`, `b-hostaccess`, `b-usershard`, `b-http`, `b-backend` |
| Tools | `t-release` (packaging, Cloud image, development store, Chrome pin) |

The API checkpoints follow the dependency graph, so they form short levels:
each takes one agent run, and the longest chain, from `l-host` to
`b-backend`, is about ten levels. The implementation checkpoints then run
side by side. **Gate G2:** no package panics as not written; every package's
tests pass; the whole tree passes the guard rails on all targets.

### Phase 3: programs and acceptance

| Work package | Accepts | How |
|---|---|---|
| `x-commands` | `demi-file`, `demi-browser`, `demi-claude-code` | The Rust runner and command-sdk suites with the Go programs substituted |
| `x-runner` | `demi-runner` | The Rust backend scenarios, `backend-remote-host` and `backend-plugins` suites with the Go runner and Go command programs substituted |
| `x-machines` | `demi-machine-manager` | The Go manager's tests and the Cloud real-machine suite in the Lima VM |
| `x-webapp` | `demi-backend` | The web app contract suite (`bun run test` with `DEMI_TEST_PROGRAMS` pointing at the Go programs) |
| `s-scenarios-*` | `demi-backend` | The Rust backend scenarios (`crates/backend/tests`) ported to Go, one work package per scenario file group |
| `x-real` | Everything | The real-machine suites, all-Go: Chrome (in a Linux VM as an ordinary user, and by the tech lead on macOS), Claude Code, Cloud |

An acceptance agent never fixes another package. It writes a failure
report naming the owning work package, and the tech lead reopens that
package with the report. **Gate G3:** every suite above passes.

### Phase 4: cutover

| Work package | Output |
|---|---|
| `z-remove-rust` | `crates/`, `vendor/`, `Cargo.*` and `rust-toolchain.toml` removed; `package.json` scripts call Go |
| `z-docs-*` | The behavior documents freed of Rust specifics, one work package per document group |
| `z-release` | The six-target release and the Cloud image built from Go |

The tech lead rewrites `AGENTS.md` for Go only. **Gate G4:** a release
builds for all targets, the real-machine acceptance passes, and the owner
approves the merge of `gomig/main`.

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
to the machine manager, HTTP and WebSockets to the web app. Each Go program
is therefore accepted by the existing tests of the program on the other end
of its wire, before the Go side has tests of its own at that level.

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
`gomig/sp*` and are never merged. The tech lead re-ran the two experiments the
sandbox could not run (FSEvents and Chrome) outside it.

| Spike | Question | Verdict | What the migration does |
|---|---|---|---|
| sp01 contracts | Go types as the only contract definition, with a generator | Works with caveats: on the transcript-block family, 26 of 26 fixture blocks round-trip, 42 of 42 refused mutations are refused, and the generated Zod has no validation difference from the Rust-generated Zod for any of the 31 shared schemas; a removed `switch` case fails the exhaustiveness check | The Go-first design ([Contracts decisions](#tech-lead-decisions)) |
| sp02 wire | Runner MessagePack, machine-manager lines and the manifest digest, byte for byte | Works with caveats: 25 runner fixtures, the kept-record corpus, all 18 machine-manager lines and the manifest and package digests match | `github.com/vmihailenco/msgpack/v5` v5.4.1 with compact integers, declaration order, a sorted encoder for string maps and non-nil empty byte slices; `github.com/gowebpki/jcs` v1.0.2 for RFC 8785; encoders and decoders generated from the same Go types as sp01 |
| sp03 HTTP/2 | The command wire in Go, interoperating with Rust | Works with caveats: streaming, client cancellation and bounded flow control interoperate both ways; a handler cannot choose `RST_STREAM(CANCEL)` for a service-side abort | `net/http` with unencrypted HTTP/2, one owned connection per service (`Transport.NewClientConn`), a 64 KiB stream window, the concurrent-stream limit and early-reset guard configured. The command wire says a service abort resets the stream and stops naming the code |
| sp04 shell | The runner's shell on `mvdan.cc/sh` with the system's utilities | Does not work unmodified: commands and pipelines work, 50 concurrent jobs complete, cancellation by process group works, but v3.14.1 cannot join every task a job started at any depth and cannot cancel an unopened process substitution; 34 of 40 typical agent snippets print what `bash` prints | See sp04b |
| sp04b shell fork | Can a small fork close those gaps and keep edit tracking of redirections? | Works with caveats: a patch of 311 added and 43 removed lines in nine files of `interp` joins every nested task, cancels used and unused process substitutions, and hands every writable redirection (`>`, `>>`, `<>`, `exec 3>f`) in all seven scopes to the open handler, 28 of 28 cases; each new test fails on the unpatched library. On macOS a FIFO's end of file needs a 10 ms recheck | [Decision 3](#open-decisions-for-the-owner), option A |
| sp05 namespaces | The machine manager's Linux primitives without cgo | Works with caveats: mount and network namespaces, recovery into a saved mount namespace, veth, nftables, loop devices, `FIFREEZE`/`FITHAW`, sparse copy and `sd_notify` all ran in the VM | One namespace-job entry point: a goroutine locks its thread, unshares `CLONE_FS`, does the whole job and exits without unlocking. Recovery starts `/proc/self/exe` from that entered thread, which `os.StartProcess` documents. `nftables.WithNetNSFd`; not `netlink.NewHandleAt`, whose restore path ignores an error; the two freeze ioctl numbers defined per architecture |
| sp06 FSEvents | Watching repository trees on macOS without cgo | Works, through purego: outside the sandbox 100 events arrived with a median latency of 11 ms, and 200 start-stop cycles left threads, descriptors and memory flat. Inside the Codex sandbox the stream does not start. A full `lstat` walk of this repository takes 18 to 25 s, too slow to replace a watch | purego for FSEvents if the owner approves ([decision 1](#open-decisions-for-the-owner)); `fsnotify` on Linux and Windows |
| sp07 git | go-git for working-tree changes and skill fetches | Works with caveats: with an adapter, staged and unstaged changes, untracked files, renames and line counts matched `git` on this repository and on Kubernetes; `Worktree.Status()` itself is wrong on a restored-mtime edit and slow. A full status of Kubernetes takes 6.5 to 6.8 s with go-git's status, 1.9 s with the spike's parallel walk, and 0.23 s with the `git` CLI; this repository takes 0.13 to 0.15 s with the walk. A shallow HTTPS fetch of a skill source took 4.4 s | go-git for objects, the index and the transport; Demi's own parallel walk over a watched baseline, as the runner design already requires; line counts with a Myers diff |
| sp08 Chrome | The conversation browser on chromedp | Works on cdproto, not on chromedp's actions: outside the sandbox, an unpacked extension, three tabs driven at once, a 9 MB full-page screenshot, acknowledged screencast frames, network and console events and a download all worked, with no goroutine left. chromedp itself lacks a bound on message size and the child-session guarantees Demi needs. Chrome does not start inside the Codex sandbox | `github.com/chromedp/cdproto` types behind Demi's own `cdp.Executor`: a bounded transport and a session router of Demi's, about 0.9 to 1.6 thousand lines for today's `src/cdp` |
| sp09 SQLite | A cgo-free SQLite for the backend | Works with caveats: the real schemas and statements ran on both `modernc.org/sqlite` and `ncruces/go-sqlite3`, and each driver reopened the other's files | `modernc.org/sqlite` v1.60.1 behind `database/sql`: one `sql.DB` with one connection per writable database, every statement inside its `sql.Tx`, short read-only connections for cold reads, the global limit of 64 open writers kept as an explicit LRU. Appends ran at 3,884 transactions/s with a p99 of 0.65 ms |
| sp10 shard | The user shard's guarantees in Go | Works with caveats: a mutex variant and an actor variant both held admission atomic under 10,000 interleaved races with `-race`, fired the idle watch at exactly 3,600 s of `synctest` time, and caught a forgotten lease | One `sync.Mutex` per shard with short critical sections; gates on `golang.org/x/sync/semaphore` (FIFO); leases released with `defer`, idempotent, and audited at shutdown in tests; `goleak` and `testing/synctest` |
| sp11 matrix | Six targets without cgo, sizes, guard rails | Works: all 18 program and target builds pass with `CGO_ENABLED=0`, statically linked on Linux, in 64 s from clean and 5.5 s with no change. The runner skeleton is 10.5 MiB on macOS arm64 | Guard rails: the cgo check per target, an import-direction table checked over `go list -deps -json`, `go-check-sumtype -default-signifies-exhaustive=false`, `golangci-lint` v2.14.0 |

### What the sandbox cannot do

The Codex `workspace-write` sandbox refuses three things the migration needs:
starting an FSEvents stream, starting Chrome, and listing processes (`ps`,
`pgrep`). Tests that need them on macOS carry the build tag `hostonly`. An
implementer runs every other test in its sandbox, and runs the Chrome tests
in its Linux VM, where Chrome for Testing has a linux-arm64 build. The tech
lead runs the `hostonly` tests outside the sandbox as part of the review,
before the merge. No agent runs
without the sandbox, because the sandbox is what keeps it inside its write
boundary. Linux-only work runs in a Lima VM through `limactl`, which the
sandbox allows; two work packages never share a VM, because their privileged
tests change the same kernel state (mounts, loop devices, firewall tables).

A resumed session needs its first run to have exited: `codex exec resume`
fails while the original process still writes the session.

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
- **go-git's major version is chosen in `r-host`'s API checkpoint:** the
  stable v5 line if it passes the spike's tests, otherwise v6.
- **Tests use the standard library**, `github.com/google/go-cmp`,
  `go.uber.org/goleak` and `testing/synctest`; no assertion library.

## Open decisions for the owner

1. **purego for FSEvents on macOS.** purego builds with `CGO_ENABLED=0` and
   needs no C compiler, but it calls the CoreServices C API at run time
   through `dlopen`. Without it, macOS has no tree watch: every working-tree
   request walks the whole tree (0.13 s on this repository, about 2 s on one
   the size of Kubernetes) instead of reading the watched baseline.
   *Recommendation:* allow purego in the one darwin file of the tree watch,
   and nowhere else.
2. **The race detector on Linux needs cgo.** `go test -race` works with
   `CGO_ENABLED=0` only on macOS. *Recommendation:* run `-race` on macOS;
   Linux tests run without it; nothing that ships is built with cgo.
3. **The shell interpreter.** sp04 showed `mvdan.cc/sh` cannot be used
   unmodified. The choice is between (A) a vendored, patched fork of
   `mvdan.cc/sh` running in the runner, as brush is vendored today, which
   keeps declared commands as builtins, one shell dialect on every platform
   and edit tracking of redirections; and (B) the system's `bash` started per
   job, which gives real bash and lets the operating system cancel a job by
   its process group, but loses edit tracking of redirections, runs bash 3.2
   on macOS and needs Git for Windows' bash on Windows. *Recommendation:* (A).
   sp04b showed the fork is a 354-line patch whose rebase on a compatible
   upstream release takes about a day, and keeping the shell in the runner
   keeps declared commands as builtins with no process per call, one shell
   dialect on every platform, and edit tracking of redirections. The fork lives in
   `third_party/mvdan-sh` behind a `replace` directive, with its patch kept
   as a file so a rebase starts from it; `r-shell` owns both.
4. **What using the system's utilities means for users.** With either
   interpreter: edit tracking stops recording files that `sed -i`, `tee` and
   `sort -o` write; a paired Mac runs BSD utilities with different flags from
   the Cloud's GNU ones; a paired Windows device has no Unix utilities unless
   the user installs them; the Cloud image must ship GNU coreutils,
   findutils, diffutils, sed, grep, ripgrep and jq; and the runner must tell
   the model which shell, platform and utility family a job runs with.
   *Recommendation:* accept these, and update `edit-tracking.md` and
   `runner.md` in Phase 0.
5. **Rust changes during the migration.** Every change to `crates/` on
   `feat/demi-next` must be ported again. *Recommendation:* only bug fixes
   land on `feat/demi-next` until gate G4; each one becomes a delta work
   package at the next gate.
