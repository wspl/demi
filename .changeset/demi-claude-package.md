---
'@demicodes/backend': minor
'@demicodes/host-remote': minor
'@demicodes/provider-claude-code': minor
'@demicodes/shell': minor
'@demicodes/web-ui': minor
'@demicodes/web': minor
---

Demi installs its own Claude Code CLI. The new `demi.claude` native package
installs a verified executable of the vendor's newest version (or the version
an entry is held at) under the Host user's `.demi/claude`, and the backend
starts that executable instead of a `claude` on `PATH`. A provider's process
runs on the user's Cloud, whatever the conversation's execution target is: a
placement names the machine for each kind of work, the conversation uses that
Cloud in the `provider` role (its turn keeps the Cloud awake, a reset holds it
and lets it go on), and the process is retained between turns without counting
as activity. `/api/providers/:id/cli` reads and manages the CLI.

`RemoteHost.services.call` asks a package operation one question over a service
stream, and `Host.process` spawns take `retained`. The Claude Code kit takes
`resolveProcess` to name each process's executable and directories, and turns
the CLI's own updater off.
