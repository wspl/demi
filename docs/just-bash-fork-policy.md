# just-bash Fork Policy

`packages/just-bash` is a git submodule pointing at the Demi fork of just-bash
(`github.com/wspl/just-bash`). It provides the bash parser/interpreter/portable
commands behind `@demicodes/shell`. This document is the governance contract
for the fork: what it may change, how it tracks upstream, and how changes land.

## What the fork is allowed to change

Fork commits stay in four narrow categories, chosen to keep upstream merges
cheap:

1. **Expose internals Demi integrates with** — parser hooks, the command
   registry, the encoding module, interpreter session /
   `hostSpawn` / `hostResolveCommand` / `hostCwd` / `spawnError` /
   registered command hooks, subpath export conditions.
2. **Dispatch order required by the Host model** — registered commands resolve
   before `PATH` when a `hostSpawn` hook is present; `preferHostSpawn` falls
   back only on `spawnError.kind === executable_not_found`.
3. **Packaging** — the `@demicodes/just-bash` name, trimmed build artifacts.
4. **GNU bash builtin alignment required for Host-backed observation** —
   `cd` (`HOME` unset, `-P` missing, `cd ..` after unlink), `pwd -P` getcwd
   failure, `test` identity/mode class when `FsStat` carries uid/gid/ino.
   The in-memory VM gets those same builtin answers. Portable `ls` /
   `whoami` stay virtual.

The fork must NOT invent Demi-specific Unix. Semantic alignment with GNU
bash (parser, expansion, builtins) belongs upstream; carrying a Demi-only
meaning turns every upstream sync into a conflict engine. If a semantic
bug blocks Demi, submit it upstream and cherry-pick the upstream commit
when it lands.

The Host-backed shell cannot wait on that for observation-critical
builtins that already have a Host hook: `cd` / `pwd -P` / `test -O/-G/-x/-c/-p/-ef`
consume `hostCwd`, `Host.identity`, and `HostFileStat` identity fields.
Those land in this fork so Host-backed matches bash. Without the hooks,
the in-memory VM stays virtual (`user` / uid `1000`, path-string cwd).
Portable command output that disagrees with GNU coreutils (`ls -l` vs
`stat` on `st_mode`, virtual `whoami`) is still a just-bash portable bug:
fix it upstream, do not paper over it by changing `ls.ts` in the fork
when Host-backed already runs GNU `ls`.

Host-backed dispatch (which names `hostSpawn`), spawn `cwd` as a directory
fd, spawn-error kinds, and registering portable Unix names onto a real Host
are `@demicodes/shell` / Host work, inventoried in `docs/bash-behavior.md` —
not fork-only commits. Do not patch just-bash in the fork to hide a shell
wiring bug.

## Current fork state

- Fork branch: `main` (pinned by the root submodule pointer; see
  `git submodule status`). Package version: `3.1.0-demi.4`.
- Last merged upstream: `vercel-labs/just-bash` `just-bash@3.1.0`
  (`2586623`, “seed cd dash from OLDPWD”).
- vercel-labs `main` is `just-bash@3.4.2`. Syncing it does not fix portable
  `ls -l` / `whoami` / `pwd -P`; it does include `test -ef` identity
  (`FsStat.dev`/`ino`). Inventory: `docs/bash-behavior.md` (Upstream
  just-bash).
- Fork-only surface on top of that base: `hostSpawn` / registered-command
  dispatch / parser hooks / packaging / Bun workspace, plus
  `preferHostSpawn` (`e02953a`, `80c7569`) and `ExecResult.spawnError` /
  `hostCwd` / cd·pwd·file-test alignment (`feat/host-spawn-error-cwd`).

## Version scheme

The fork publishes as `<upstream-version>-demi.<N>` (current:
`3.1.0-demi.4`), keeping the upstream semver visible while marking the
artifact as fork-modified:

- `<upstream-version>` is the upstream version the fork base is rebased onto
  (see "Fork base" above). It only changes when the fork rebases onto a
  newer upstream release.
- `<N>` starts at `1` when the fork base changes, and increments by one for
  each package-affecting fork change in the allowed categories above.
- Bump `packages/just-bash/package.json`'s `version` field in the same
  commit that adds the fork-only change (or the rebase commit that updates
  the base), so the version and "Fork-only commits" list above stay in
  sync.
- The `-demi.N` suffix makes npm/semver treat every published version as a
  prerelease. `npm publish` therefore requires an explicit `--tag latest`
  (npm refuses to default a prerelease to `latest`) — always pass it when
  publishing this package, otherwise `npm install @demicodes/just-bash`
  with no version resolves to nothing.

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
