# just-bash Fork Policy

`packages/just-bash` is a git submodule pointing at the Demi fork of just-bash
(`github.com/wspl/just-bash`). It provides the bash parser/interpreter/portable
commands behind `@demicodes/shell`. This document is the governance contract
for the fork: what it may change, how it tracks upstream, and how changes land.

## What the fork is allowed to change

Fork commits stay in three narrow categories, chosen to keep upstream merges
cheap:

1. **Expose internals Demi integrates with** — parser hooks, the command
   registry, the encoding module, interpreter session/`hostSpawn`/registered
   command hooks, subpath export conditions.
2. **Dispatch order required by the Host model** — registered commands resolve
   before `PATH` when a `hostSpawn` hook is present.
3. **Packaging** — the `@demicodes/just-bash` name, trimmed build artifacts.

The fork must NOT change bash semantics (parsing, expansion, builtin
behavior). Semantic fixes belong upstream; carrying them here turns every
upstream sync into a conflict engine. If a semantic bug blocks Demi, submit it
upstream and cherry-pick the upstream commit.

## Current fork state

- Fork branch: `codex/just-bash-mvp-cleanup` (pinned by the root submodule
  pointer; see `git submodule status`).
- Fork base: upstream `main` at `9481331` ("fix(head/tail): read file
  arguments byte-clean").
- Fork-only commits (14, all within the allowed categories):
  `f3645ee` parser hooks · `5e925b7` hostSpawn hook · `4e2ab29` registered
  command dispatch order · `c7f1be5` test typing · `cabfc0f` session hooks ·
  `cc10d4f` trim artifacts · `9576ef2` reject trailing operators (parser
  protection used by Demi) · `f38734a` expose command registry · `8122ac3`
  package rename · `ea06eb1` export encoding module · `496c3d7` dual-condition
  subpath exports · `e798f00` adopt the version suffix scheme below · `54975fb`
  drop leading `./` from `bin` paths (npm silently strips it, which broke the
  published CLI bins) · `395774b` bump to `3.0.1-demi.2` for that fix.

## Version scheme

The fork publishes as `<upstream-version>-demi.<N>` (current:
`3.0.1-demi.1`), keeping the upstream semver visible while marking the
artifact as fork-modified:

- `<upstream-version>` is the upstream version the fork base is rebased onto
  (see "Fork base" above). It only changes when the fork rebases onto a
  newer upstream release.
- `<N>` starts at `1` when the fork base changes, and increments by one each
  time a fork-only commit in the allowed categories above lands on top of
  it.
- Bump `packages/just-bash/package.json`'s `version` field in the same
  commit that adds the fork-only change (or the rebase commit that updates
  the base), so the version and "Fork-only commits" list above stay in
  sync.

## Upstream sync

- Cadence: on demand when Demi needs an upstream fix, plus a quarterly review
  of upstream changes to the parser/interpreter/commands we consume.
- Mechanics: rebase the fork branch onto the new upstream `main` (the fork's
  commit list above IS the rebase todo — keeping it short and categorized is
  the point of this policy). After rebase: run the fork's own suite, then
  Demi's `bun run test` and `test:just-bash-core` before moving the root
  pointer.
- The root repository records sync landings as two commits per the repo rules:
  the fork branch commit inside the submodule (pushed first), then the root
  submodule pointer bump.

## Surface area control

`@demicodes/shell` enables ~55 of the fork's 91 command implementations
(`DEMI_PORTABLE_COMMANDS` in `packages/shell/src/portable-commands.ts`).
Command modules load lazily through `createLazyCommands`, so unused commands
cost repository size but not runtime footprint; bundlers tree-shake them out
of published artifacts. Deleting unused command sources in the fork is
intentionally avoided — it would add permanent rebase friction for a
size-only win. Revisit if the fork ever becomes a published package of its
own.

## Invariants checked by Demi

- `packages/core/src/__tests__/platform-entrypoints.test.ts` verifies runtime
  source imports the forked package (no vendored upstream snapshots).
- `bun run test:just-bash-core` runs the parser-protection suites the fork's
  hooks depend on.
