---
"@demicodes/shell": patch
"@demicodes/agent": patch
---

Harden command-bridge execution semantics: `AgentServer.runCommandLine` now
runs every bridge invocation in an ephemeral shell (disposed after the call),
so the caller's `cd`/env can never leak into the model's persistent session
shell; and newline-terminated bridge stdin arrives byte-identical instead of
gaining a duplicated trailing newline from heredoc rendering.
`ShellExecInput` gains an `ephemeral` option backing this.
