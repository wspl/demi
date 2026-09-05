# @demicodes/shell

A sandboxable bash engine for Demi. Commands run through a `Host` abstraction
(`fs` / `process` / `store`) rather than touching the machine directly, so the
same agent can target local, container, remote, or in-memory backends.

- `BashEnvironment` — long-running shell sessions with `exec` / `status` / `write`
  / `abort` returning `ShellCommandStatus`, plus a `/@` virtual filesystem of
  command artifacts.
- `Host` contract (see the package registry in [docs/package-boundaries.md](../../docs/package-boundaries.md)).
- Built on a forked `just-bash` workspace package.

Subpaths: `@demicodes/shell/storage`, `@demicodes/shell/testing`.

Part of [Demi](../../README.md). Apache-2.0.
