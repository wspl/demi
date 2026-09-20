---
'@demicodes/backend': minor
'@demicodes/host-remote': minor
'@demicodes/provider-claude-code': minor
---

Demi installs its own Claude Code CLI. The new `demi.claude` native package
installs a verified executable of the vendor's newest version (or the version
an entry is held at) under the Host user's `.demi/claude`, and the backend
starts that executable instead of a `claude` on `PATH`. Account work — the
install after an account is added, **Test connection** — runs on the acting
user's Cloud through a placement that names the machine for each kind of work;
inference installs on the conversation's execution target when first asked.
`/api/providers/:id/cli` reads and manages it.

`RemoteHost.services.call` asks a package operation one question over a service
stream. The Claude Code kit takes `resolveClaudePath` to name the executable per
process, and turns the CLI's own updater off.
