---
'@demicodes/agent': patch
---

Complete tool calls left `executing` in a checkpoint when restoring via `AgentSession.fromCheckpoint`. A checkpoint can only hold an executing call if the process died mid-tool; replaying it without a result made providers reject every subsequent request (`No tool output found for function call ...`), deadlocking the session. Restore now completes each dangling call with an interrupted error result.
