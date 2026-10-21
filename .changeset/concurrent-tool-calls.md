---
"@demicodes/agent": minor
"@demicodes/shell": patch
---

Run adjacent calls of concurrent tools from one provider response at the same time. Tools opt in with `concurrent: true`; the standard shell tools do, while `yield` and tools without the flag keep running alone in response order. `BashEnvironment.exec` now picks and claims a shell in one step, so concurrent execs without `shellId` no longer collide on the session's default shell.
