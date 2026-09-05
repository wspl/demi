# @demicodes/shell

A sandboxable bash engine for Demi. Commands run through a `Host` abstraction
(`fs` / `process` / `store`) rather than touching the machine directly, so the
same agent can target local, container, remote, or in-memory backends.

- `BashEnvironment` — long-running shell sessions with `exec` / `status` / `write`
  / `abort` returning `ShellCommandStatus`, plus a `/@` virtual filesystem of
  command artifacts.
- `Host` contract (see [Implement a Host](../../docs/guides/implement-a-host.md)).
- Built on a forked `just-bash` workspace package.

Subpaths: `@demicodes/shell/storage`, `@demicodes/shell/testing`.
See [docs/shell-yield-control-plan.md](../../docs/shell-yield-control-plan.md) for
the model-facing control surface and yield wakeups, and
[docs/bash-behavior.md](../../docs/bash-behavior.md) for Host-backed behavior
versus GNU bash.

Part of [Demi](../../README.md). Apache-2.0.
