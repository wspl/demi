---
status: proposed
---

# A Host's path form is its own crate

## Problem

Two crates write a path under a Host's home from `~`, by two rules. The
backend uses `HostPaths::written_from_home` in `host-interface`. The runner,
when it cannot register itself to start at login, says where the file it could
not write is, through a private function in `runner-protocol`:

```text
the file that registers it, ~/.config/systemd/user/demi-runner.service, could not be written
```

The private function reads `\` as a separator on macOS and Linux too, where it
is a character of a name, and compares a Windows path with its case, where
`C:\Users\Alex` and `c:\users\alex` are one directory. A fix to one rule misses
the other.

`runner-protocol` cannot use `HostPaths` where it is. `host-interface` is the
contract between the agent and what executes its work, which runs inside a
user's shard; the runner and the machine manager link none of it.

## Decision

`HostPaths`, `is_inside` and `relative_to` move from `host-interface` to a new
crate, `host-paths`: how a Host writes its paths, as pure functions of text,
with no IO and no runtime. It depends on `serde` and `typed-path` alone.

- `host-interface`, `runner-protocol` and every crate that uses these names
  depend on `host-paths` and import them from there. `host-interface` keeps no
  copy and no re-export.
- `RunnerPlatform::paths()` in `runner-protocol` maps a runner's platform to its
  path form, in place of `host_paths` in `runner-client`.
- `AutostartUnavailable::sentence` takes the runner's platform and writes the
  path with `HostPaths::written_from_home`. The private function is removed.

```text
                 host-paths
                /          \
   host-interface          runner-protocol
   (agent, plugins,        (runner, runner-client,
    backend)                machine manager)
```

Nothing stored or sent changes: `HostPaths` keeps its serde form.

## Spike

None.

## Prior art

Codex keeps its path type in
[`codex-utils-absolute-path`](https://github.com/openai/codex/tree/main/codex-rs/utils/absolute-path),
a crate of its own that both its core and its exec server use. Its paths are
the local machine's, resolved against its filesystem; `host-paths` reads the
text of a path on another machine and does no IO.

## Alternatives

- `runner-protocol` depends on `host-interface`: the runner and the machine
  manager would build the agent's Host contract, and its tokio and command
  system, for one enum.
- `HostPaths` in `shared-types`: that crate holds no Host detail by its own
  boundary.
- Keep the private function: two rules for one purpose.

## Open questions

None.
