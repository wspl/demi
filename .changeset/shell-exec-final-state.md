---
"@demicodes/shell": patch
"@demicodes/agent": patch
"@demicodes/host-local": patch
---

Final-state cleanup of bridge exec plumbing: the command scope id is exposed
under a single env var (`DEMI_SESSION_ID`; the exec-time `DEMI_AGENT_SESSION_ID`
alias and the shim's fallback chain are gone), and ephemeral execs take an
explicit `cwd` (validated as a directory) instead of a rendered `cd … &&`
prefix in the script.
