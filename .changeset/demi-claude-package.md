---
'@demicodes/web-ui': minor
'@demicodes/web': minor
---

Demi installs its own Claude Code CLI. The `demi.claude` native package
installs a verified executable of the vendor's newest version (or the version
an entry is held at) under the Host user's `.demi/claude`, and a provider's
process runs on the user's Cloud, whatever the conversation's execution target
is. `/api/providers/:id/cli` reads and manages the CLI.
