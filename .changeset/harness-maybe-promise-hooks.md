---
"@demicodes/agent": minor
---

Let `AgentHarness.systemPrompt`, `preamble`, and `commands` return a promise. Harness authors can now build prompts and command sets from I/O (reading workspace files, querying a store) instead of pre-computing them and closing over the result. Existing synchronous implementations keep working — the return types are widened, not replaced. `initialState` stays synchronous because `AgentSession` calls it from its constructor; async setup belongs in `host()` or a lifecycle hook.
