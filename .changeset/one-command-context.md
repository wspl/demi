---
"@demicodes/command-protocol": minor
"@demicodes/runner-protocol": minor
"@demicodes/shell": minor
"@demicodes/command-loader": minor
"@demicodes/host-remote": minor
"@demicodes/backend": minor
"@demicodes/web": minor
---

Declared commands receive one command context — the conversation, who started the work (an agent node, or the user) and the user's locale — instead of reading identities from environment variables. The backend builds it when a job starts, from the conversation's user's preferences, and keeps it in the job's record: `job_start` carries it, a native invocation carries it in place of the `conversation` and `caller` strings, and an application callback names only its job, so `rpc_call` drops `agentSessionId` and `shellId` and handlers receive `context`. A job's environment keeps only the local endpoint, the context handle, `DEMI_HOME` and the aliases on `PATH`; `DEMI_CONVERSATION_ID`, `DEMI_AGENT_NODE_ID`, `DEMI_SESSION_ID` and `DEMI_SHELL_ID` are gone. The local command client sends its own `LocalInvocation` metadata. User preferences store the `locale` the product reports — the browser's time zone and languages — and commands get `UTC` and `en-US` until it does. The runner wire version is 17.
