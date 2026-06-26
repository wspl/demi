# @demi/shell

A sandboxable bash engine for Demi. Commands run through a `Host` abstraction
(`fs` / `process` / `store`) rather than touching the machine directly, so the
same agent can target local, container, remote, or in-memory backends.

- `BashEnvironment` — long-running shell sessions with `exec` / `status` / `write`
  / `abort` and a `/@` virtual filesystem of command artifacts.
- `Host` contract (see [Implement a Host](../../docs/guides/implement-a-host.md)).
- Built on a vendored fork of `just-bash`.

Subpaths: `@demi/shell/storage`, `@demi/shell/host-fs`.

Part of [Demi](../../README.md). Apache-2.0.
