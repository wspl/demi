# Go port plan

Internal working plan for moving Demi from TypeScript and Rust to Go. Read it at
the start of every session and after every context compaction, before any other
port work. The orchestrator is the only writer of this directory.

## Goal and fixed decisions

- Everything except the frontend becomes Go. The frontend is `packages/web`,
  `packages/web-ui`, `packages/web-gallery` and the browser-side libraries they
  import: `@demicodes/core`, `@demicodes/utils`, `@demicodes/agent/client` (with
  the protocol schemas it uses) and `@demicodes/browser-protocol`.
- Base: `6e043eb1`, the last product that was used and known to work. The Go
  line lives on `go/main`. The Rust migration line (`feat/demi-next`,
  `port/*`) is ignored: not read, not reused, not compared during the port.
- The 41 shell utilities run in the runner's process. No utility runs as a
  child process. Forking permissively licensed code (MIT, BSD, Apache) is
  allowed; GNU tools are black-box references only.
- Linux x86_64 only for now. Other targets come after the port.
- Acceptance runs on Linux in this environment.
- Parallel work uses plain subagents (the Agent tool). No Workflow tool.
- This directory is ignored by git. It is backed up to the orphan branch
  `go/internal` at every checkpoint, because the container is reclaimed when
  idle and everything local is lost.

## Principle: every checkpoint is a working product

The port replaces one process at a time behind an existing protocol, so the
product keeps working and the old tests keep judging it.

```text
frontend ──HTTP/WS (S4)──> backend ──WS runner wire (S1)──> runner ──HTTP/2 stdio (S2)──> native packages
                              │                                                           (demi-commands,
                              └──unix socket, machines wire (S3)──> machine manager          demi-claude)

6e043eb1   TS backend + Rust runner + Rust native packages + TS machines
Gate A     Go runner passes the old suites (partial utilities; old runner stays the default)
Gate B     all 41 utilities in Go; the Go runner becomes the default
Gate D     Go backend, agent and providers; the Go backend becomes the default
Gate C     Go native packages and Go machine manager; no Rust left
X1         internal contract sources move to Go; server-side TS and all Rust deleted
```

Old tests select programs through `DEMI_RUNNER_TEST_BINARY` and
`DEMI_NATIVE_TEST_BINARY`, so a Go program is dropped in without changing a
test.

## Contracts

- Frontend-facing contracts (web API bodies, conversation frames, live view
  protocol, core types) keep their Zod schemas in the frontend libraries as the
  single source. Go types and validators are generated from them
  (`xtask contracts`: Zod → JSON Schema → Go). The frontend does not change.
- Internal contracts (runner wire, machines wire, command-service wire,
  manifests) keep Zod as the source while a TypeScript end still speaks them.
  In X1, when both ends are Go, the source moves to Go and the TypeScript
  packages are deleted.

## Go layout

One module at the repository root (`go.mod`, module `github.com/wspl/demi`).

```text
cmd/<program>/            demi-runner, demi-backend, demi-commands, demi-claude, demi-machines, xtask
internal/contract/<name>/ generated: runnerwire, machineswire, cmdservice, webapi, agentproto, core, browserlive
internal/toolctx/         the utility execution context and registry (orchestrator-owned interface)
internal/tools/<name>/    one package per utility family
internal/shell/           the job model on the forked interpreter
internal/runner/...       connection, host operations, services, native packages, git, tree
internal/commandservice/  HTTP/2-over-stdio SDK, client and server
internal/commands/...     demi-commands: files, browser, live
internal/claude/          demi-claude
internal/machines/        machine manager
internal/provider/...     provider core and one package per vendor
internal/agent/, internal/codingagent/
internal/hostremote/, internal/shellenv/
internal/backend/...      storage, http, auth, conversation, llm, vault, managed, expose
internal/testing/gnucorpus/  differential corpus harness; corpora under testdata/gnu/<tool>/
third_party/<fork>/       forks, each with PATCHES.md (maintainer, every change and its reason)
```

## Work packages

One work package (WP) is one checkpoint: implemented, verified, squash-merged
into `go/main` as one Conventional Commit, pushed. Sizes are relative (1 ≈ one
wave). References are paths in the baseline worktree `/home/user/demi-base`.

### F: Foundation (wave 0)

| ID | Scope | Who |
|---|---|---|
| F1 | `go/main`, `go/internal`, slots s1–s4, slot data and env files, prebuilt old binaries | orchestrator |
| F2 | AGENTS.md Go section, `.golangci.yml` (errcheck, govet, staticcheck, exhaustive, forbidigo), check script | orchestrator |
| F3 | Design delta in `docs/`: Go layout and boundaries, runner shell and utility context, contracts, concurrency and ownership, Linux builds | orchestrator |
| F4a | Cross-lane interfaces: `internal/toolctx`, `internal/commandservice` API | orchestrator |
| F4b | `xtask contracts`: Zod → JSON Schema → Go for all contract packages | agent in s2 |
| F5 | Gate 0 baseline and the ledger: every old test case mapped to a behavior and a WP | orchestrator with Explore agents |

### Lanes

| ID | Scope | Reference | Owned Go paths | Depends on | Size |
|---|---|---|---|---|---|
| S1 | Shell core: fork mvdan/sh; job scope, completion, cancellation, exit status; open handler records edits; exec handler routes declared roots, utilities, external programs; login profiles and HOME; background jobs, here-docs, process substitution | `crates/runner/src/shell/`, `docs/demi-next/runner.md`, `edit-tracking.md`; tests `shell.rs`, `edit_tracking.rs`, `pipes.rs`, `tasks.rs`, `load.rs`, `open_files.rs` | `internal/shell`, `third_party/mvdan-sh` | F | 1.5 |
| S2 | Runner connection and host operations: registration, pairing, claim, wire codec against the old TS backend, filesystem and process operations, pipes, file contents, Host log, management, command-alias mode, state | `crates/runner/src/{connection,host,host_log,management,mode,state,stdio,process,pipes,fs,files,paths,net,volumes}`; tests `connection.rs`, `host.rs`, `mode.rs`, `process.rs`, `fs.rs`, `local.rs` | `internal/runner/{connection,host,hostlog,management,state,alias}`, `cmd/demi-runner` | F | 1 |
| S3 | Native package execution (manifests, artifact cache, command-service client), resident services, git status and diff (go-git), working tree and watch | `crates/runner/src/{commands,git.rs,tree_watch.rs,file_diff.rs}`; tests `dispatch.rs`, `command_client.rs`, `artifact_cache.rs`, `git.rs`, `conversations.rs` | `internal/runner/{native,services,artifacts,git,tree}` | S2, N1 | 1 |
| T0 | Differential corpus harness and corpora for all 41 utilities: core and extended option sets, cases, expected output from the GNU tools here | GNU coreutils 9.4, grep 3.11, sed 4.9, findutils 4.9, diffutils 3.10, rg, jq | `internal/testing/gnucorpus`, `testdata/gnu` | F4a | 1 (light, s4) |
| T1a | Text: cat head tail wc tee sort uniq cut tr | `crates/runner/src/shell/utilities.rs`, `vendor/uu_*` | `internal/tools/text` | F4a, T0 | 1 |
| T1b | Search: grep rg find xargs | as above, `vendor/{uu_grep,ripgrep,findutils}` | `internal/tools/search` | F4a, T0 | 1 |
| T1c | Transform: sed jq (fork gojq) | `vendor/{sed,jaq}` | `internal/tools/transform`, `third_party/gojq` | F4a, T0 | 1 |
| T2a | Files: ls cp mv rm mkdir rmdir touch stat du df chmod chown realpath mktemp | `vendor/uu_*` | `internal/tools/files` | T0 | 1 |
| T2b | Misc: basename dirname env seq date sleep paste nl tac od | `vendor/uu_*` | `internal/tools/misc` | T0 | 1 |
| T2c | diff cmp | `vendor/diffutils` | `internal/tools/diff` | T0 | 0.5 |
| N1 | command-service SDK: HTTP/2 over stdio, record framing, invocation exchange, bounded IO, cancellation, edit journal with file lock | `crates/command-service` | `internal/commandservice` | F4a, F4b | 1 |
| N2 | demi-commands file commands (read, create, edit, patch) and operation routing | `crates/demi-commands` (files) | `internal/commands/files`, `cmd/demi-commands` | N1 | 1 |
| N3 | demi-commands browser: chromedp driver, tab registry, observations, operations, cleanup | `crates/demi-commands` (browser), `vendor/chromiumoxide` patches as behaviors | `internal/commands/browser` | N1 | 1.5 |
| N4 | Live view: capture extension, page observers, streaming | `crates/demi-commands` (live) | `internal/commands/live` | N3 | 1 |
| N5 | demi-claude | `crates/demi-claude` | `internal/claude`, `cmd/demi-claude` | N1 | 0.5 |
| M0 | Feasibility here: runsc with systrap, loop devices, nftables, namespaces | — | — | F1 | 0.3 |
| M1 | Machine manager: machines wire server, device workers, gVisor lifecycle, images, volumes, networking | `packages/machines` | `internal/machines`, `cmd/demi-machines` | M0, F4b | 1 |
| P1 | Provider core: types, HTTP and SSE streaming, credential pools and refresh, quota, models.dev catalog, model selection, response wires, scripted provider | `packages/provider` | `internal/provider` | F4b | 1 |
| P2 | Anthropic API, OpenAI API, Google | `packages/provider-{anthropic-api,openai-api,google}` | `internal/provider/{anthropicapi,openaiapi,google}` | P1 | 1 |
| P3a | Codex | `packages/provider-codex` | `internal/provider/codex` | P1 | 1 |
| P3b | Grok Build | `packages/provider-grok-build` | `internal/provider/grokbuild` | P1 | 1 |
| P4 | Claude Code (CLI through the Host) | `packages/provider-claude-code` | `internal/provider/claudecode` | P1, H2 | 1 |
| A1 | Agent core: types, protocol frames, transcript, store, node, tools | `packages/agent/src/{types.ts,protocol,transcript,store,node,tools.ts}` | `internal/agent` (core) | P1, F4b | 1 |
| A2 | Agent session | `packages/agent/src/session` | `internal/agent/session` | A1 | 1 |
| A3 | Subagents and agent server | `packages/agent/src/{subagent,server}` | `internal/agent/{subagent,server}` | A2 | 1 |
| A4 | Coding agent: harness, demi commands and their routing, todo | `packages/coding-agent` | `internal/codingagent` | A3, H2 | 0.5 |
| H1 | Host access, backend side: runner registry, remote host, pipes | `packages/backend/src/runner`, `packages/host-remote/src/{remote-host.ts,pipes.ts}` | `internal/hostremote`, `internal/backend/runnerreg` | E1 | 1 |
| H2 | Shell environment and commands: command records, command ABI, reserved names, file host store, remote shell environment, command loader and manifests | `packages/{shell,command-loader,command-protocol}`, `packages/host-remote/src/{remote-shell-environment.ts,shell-environment-factory.ts}` | `internal/shellenv`, `internal/commandloader` | H1 | 1 |
| E1 | Backend skeleton and storage: config, lifecycle, SQLite, migrations, stores | `packages/backend/src/{storage,backend.ts,lifecycle,dev.ts,testing*}`, `docs/demi-next/storage.md` | `internal/backend/{storage,config,lifecycle}`, `cmd/demi-backend` | F4b | 1 |
| E2 | HTTP API and auth: routes, sessions and accounts, product state, installer routes | `packages/backend/src/{http,auth}` | `internal/backend/{http,auth}` | E1, H1 | 1 |
| E3 | Conversations and LLM services | `packages/backend/src/{conversation,llm}` | `internal/backend/{conversation,llm}` | E2, A3 | 1 |
| E4 | Vault, managed hosts (Cloud through the machines client), expose | `packages/backend/src/{vault,managed,expose}` | `internal/backend/{vault,managed,expose}` | E2 | 1 |
| X1 | Switch and cleanup: internal contract sources to Go; delete server-side TS, Rust crates and vendor; docs final check; ledger complete | — | — | Gates B, C, D | 1 |

## Schedule

Three implementer slots, so about 11 waves after wave 0. A wave is the time of
one size-1 WP including verification. The table is indicative; the scheduling
rule below decides.

| Wave | s1 | s2 | s3 | s4 (light) |
|---|---|---|---|---|
| 0 | F1–F3, F4a (orchestrator) | F4b | F5 inventory (Explore) | baseline reruns |
| 1 | S1 | N1 | P1 | T0 |
| 2 | S2 | T1a | A1 | verify |
| 3 | S3 | T1b | A2 | verify |
| 4 | E1 | T1c → **Gate A** | A3 | verify |
| 5 | H1 | T2a | P2 | verify |
| 6 | E2 | T2b | H2 | verify |
| 7 | E3 | T2c → **Gate B** | A4 | verify |
| 8 | E4 | P3a | P4 | verify |
| 9 | N2 + N5 | P3b → **Gate D** | M0, M1 | verify |
| 10 | N3 | — | M1 | verify |
| 11 | N4 → **Gate C** | X1 | — | verify |

Scheduling rule: when a slot frees, it takes the ready WP with the highest
priority. Priority: (1) the WPs of the next gate in the order A, B, D, C; (2) the
backend chain P1 → A1 → A2 → A3 → E3; (3) the rest. Among equal priorities, the
WP whose lane agent is idle goes first (context reuse).

## Gates

All criteria are measured on this machine against the baseline product. "100%"
means every test that passes in the baseline.

**Gate 0, baseline** (at `6e043eb1`, `bun run test`, 13 min 18 s including the
Rust build): 1560 tests in 273 files; 1499 pass, 19 skip, 42 fail in 17 files.
Rerun one file at a time with a 20 s timeout, 37 failures in 16 files
reproduce; only `provider-accounts.test.ts` passes, so load explains almost
none of them. Clusters:

- Native `demi` commands through the runner hang for about 10 s
  (`demi-command.test.ts` 11, `coding-marathon.test.ts` 3, scenarios `s1-files`,
  `s6-switch`, `edit-tracking`, `claude-chain.e2e`). Example: `demi file create
  note.txt <<'EOF' …` is still `running` after 10 s.
- Standard input and stream plumbing: `s3-long-commands` (stdin through
  `shell_write`), `host-shell` (`--host` pipes), `net.test.ts` (1 MiB echo),
  `service-streams.test.ts` (cancel on page loss), `working-tree.test.ts`
  (uploads, transfers on archive).
- Others: the runner's Host conformance suite, `expose` (WebSocket echo, 65th
  connection), `request-bodies` (body caps), `agent` `tools.test.ts` and
  `subagent.test.ts`.

The product was used on macOS; on Linux here these behaviors fail. F5 triages
each one: a cause outside the product (missing credential, network, tool) is
excluded and recorded; a product failure on Linux becomes a required behavior
of the WP that owns it, so the Go line must pass it. "100%" in the gates means
the baseline pass set plus these required behaviors.

**Gate A, the Go runner replaces the old runner** (old TS backend, old Rust
native packages):

- A1: every old TS test that starts a runner passes with the Go runner.
- A2: the old runner's Rust integration tests, ported by behavior, pass.
- A3: shell differential corpus (old test scripts, the GNU bash observations
  from `docs/bash-behavior-comparison`, agent-typical scripts) against bash 5.2
  here: every case the old runner matches, the Go runner matches.
- A4: isolation: 50 concurrent jobs × 20 rounds with different cwd, env and
  umask, pipelines, redirections and background jobs: no cross-talk. forbidigo
  bans process-global state in `internal/shell` and `internal/tools`
  (`os.Chdir`, `os.Getwd`, `os.Setenv`, `os.Unsetenv`, `os.Exit`, `os.Stdin`,
  `os.Stdout`, `os.Stderr`, `syscall.Umask`, `signal.Notify`).
- A5: cancellation: `sleep`, `tail -f`, `cat` on idle stdin, `yes | head`
  loops, `find /`, large `sort`, `grep -r`, `jq` and background jobs release
  every goroutine and descriptor within 200 ms; goleak after every test.
- A6: edit tracking: every recorded row of the old `edit-tracking.md` has a
  passing test; forbidigo bans direct file writes in `internal/tools`.
- A7: performance against the old runner: job start p50 ≤ 1.5×; 1000
  iterations of `cat f | wc -l` ≤ 1.5×; peak RSS with 50 concurrent jobs ≤ 2×.
- Time box: Gate A must pass by the end of wave 6. Otherwise stop and report
  with data. Criteria are never lowered in code.

**Gate B, all 41 utilities:** per utility, stdout byte-identical, exit code
identical, stderr present exactly when the reference writes it; 100% of the core
option set (options the old tests use plus options agents commonly use,
proposed by the corpus agent, approved by the orchestrator); ≥ 95% of the
extended set with every gap recorded. Every utility supports `--help`, resolves
paths against the job's directory, honors cancellation and writes only through
`toolctx`. The Go runner becomes the product default.

**Gate D, backend:** the old backend, agent, provider, host-remote, shell and
coding-agent tests, ported by behavior, pass (ledger complete for them); the
unchanged frontend works against the Go backend; product acceptance on Linux
here, including a real message when a provider credential is available.

**Gate C, native packages and machines:** old command-service, demi-commands,
demi-claude and machines tests, ported by behavior, pass; Cloud acceptance here
if M0 finds runsc usable, otherwise the scripted machine manager and a recorded
gap.

**Checkpoint checks (every WP):** `gofmt -l` empty; `go vet ./...`;
`golangci-lint run`; `go test -race` for changed packages; build all programs;
the old TS suites the WP affects, with the new program dropped in.

## Multi-agent collaboration

### Roles

- **Orchestrator** (the main session): the only writer of `docs/internal/`,
  design documents, cross-lane interfaces, `go.mod` and `go/main`. Assigns WPs,
  merges, runs checkpoint checks, pushes, backs up, recycles agents. Does not
  read large code itself; lookups go to Explore agents to keep its context
  small.
- **Lane agents** (implementers): one per lane, working in one slot on one WP
  branch at a time.
- **Verifiers**: one per lane, never the implementer. Read the diff against the
  design, AGENTS.md and the gate criteria, run the WP's tests, report findings
  as high, medium or low.
- **Corpus agent**: builds differential corpora from the GNU tools, separate
  from the utility implementers.

### Worktree slots

A slot is a long-lived worktree with warm state that successive WPs reuse. A
fresh worktree per agent is never created (the Agent tool's worktree isolation
is not used), because it starts cold and is deleted afterwards.

| Slot | Worktree | Data | Ports | Use |
|---|---|---|---|---|
| s1 | `/home/user/demi-slots/s1` | `/home/user/demi-slot-data/s1` | 41000–41999 | implementer |
| s2 | `/home/user/demi-slots/s2` | `/home/user/demi-slot-data/s2` | 42000–42999 | implementer |
| s3 | `/home/user/demi-slots/s3` | `/home/user/demi-slot-data/s3` | 43000–43999 | implementer |
| s4 | `/home/user/demi-slots/s4` | `/home/user/demi-slot-data/s4` | 44000–44999 | verification, corpus, light work |

- The integration worktree is `/home/user/demi` on `go/main`; the baseline
  worktree is `/home/user/demi-base` (detached at `6e043eb1`).
- Each slot has `env.sh` in its data directory: `TMPDIR`, `DEMI_BACKEND_DATA`,
  the port base, `GOFLAGS=-p=2`, and the test program variables.
- Warm state kept across WPs: `node_modules`, the shared Go build and module
  caches. The old Rust programs are built once in the baseline worktree and
  shared through `DEMI_RUNNER_TEST_BINARY` and `DEMI_NATIVE_TEST_BINARY`.
- Life of a slot: free → `git switch -C go/wp/<ID> go/main` → implementing →
  verified (verification runs in s4 on the WP branch) → squash-merged → WP
  files cleaned, warm state kept → free. `slots.md` records slot, WP, branch,
  agent and state.

### Context reuse

- The next WP of a lane goes to that lane's existing agent through SendMessage,
  not to a new agent, so the agent keeps the lane's design and code in context.
  Review fixes go back to the implementer the same way.
- A verifier is reused within its lane for the same reason, and never crosses
  lanes.

### Recycling

- **Context:** an agent is retired at a WP boundary when its reported context
  use passes about 60% of the window, when it has been compacted, after three
  WPs, or when it repeats a corrected mistake or contradicts an earlier
  decision. It first writes a handoff (`handoffs/<lane>-<n>.md`: state, decisions
  and reasons, open issues, file map, commands, pitfalls); the orchestrator
  reviews it; a fresh agent starts in the same warm slot with the brief and the
  handoff. The retired agent is never resumed.
- **Disk:** at every wave end the orchestrator checks disk use. Above 80% of
  the allowance it removes, in order: WP leftovers in slot data, Rust build
  directories no longer needed after their gate, the Go test cache.
- **Orchestrator:** the plan, `slots.md`, the ledger and handoffs are complete
  at every wave end, so work resumes from this directory alone after a
  compaction or in a new session.

### Life of a WP

1. The orchestrator writes the brief (`briefs/<ID>.md`) and assigns a slot.
2. The implementer works only in its slot and owned paths, commits on the WP
   branch, runs the WP checks and reports.
3. The verifier checks the WP branch in s4. High findings go back to the
   implementer; the WP is re-verified.
4. The orchestrator squash-merges into `go/main`, runs the checkpoint checks,
   pushes, backs up this directory to `go/internal`, and updates the status,
   metrics and ledger.
5. Merges happen one at a time. A change to a cross-lane interface lands on
   `go/main` first; the affected lanes merge it into their WP branch.

### Rules for agents (quoted in every brief)

1. Work only in your slot and your owned paths. Never touch another slot,
   `go/main`, or `docs/internal/` (read-only for you).
2. If the design does not answer a question, stop that part and report the
   exact situation. Do not decide it in code.
3. Follow AGENTS.md and the Go standards. No child process for a utility. No
   process-global state in the shell or utilities.
4. Fork only MIT, BSD or Apache code and record it in the fork's PATCHES.md.
   Use GNU tools only as black-box references; do not read GPL sources.
5. Test behavior at boundaries; never call a real model.
6. Commit on your WP branch; never push.
7. Report: what changed, files, tests run and results, deviations, open
   questions, design gaps.

## Status

| WP | Slot | Agent | State | Notes |
|---|---|---|---|---|
| F1 | — | orchestrator | in progress | `go/main` pushed at `6e043eb1` |
| F5 | — | orchestrator | in progress | baseline and serial reruns done (37 reproducible failures); triage next |

## Working method notes

- 2026-09-26: the baseline suite runs in parallel by default and takes 13 min
  18 s here, most of it the Rust build; reuse the prebuilt old programs through
  the test program variables instead of rebuilding.
