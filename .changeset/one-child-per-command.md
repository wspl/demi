---
"@demicodes/agent": patch
"@demicodes/shell": patch
---

Let one shell command host at most one subagent: a second `demi agent` spawn or resume inside the same command fails, so a looping script can no longer keep starting children after they are aborted. Registered commands receive the hosting `commandId` on `CommandRunContext`.
