---
'@demicodes/agent': minor
---

Subagents form an unbounded session tree: every session carries the identical `demi agent` command tree, so spawn nests to any depth, persisted recursively and restored as a subtree. A new connection-wide agent directory backs cross-tree communication: `demi agent send <id|parent>` leaves a mailbox message delivered at the target's next turn boundary (a pending message defers a subagent's close by one turn), `demi agent steer <id|parent>` chimes into a running turn, `demi agent show` snapshots any live agent, and `demi agent list` renders the whole tree with a self marker. Lifecycle verbs (spawn / abort / resume) stay with the spawning session. `demi agent send-parent` and `DEMI_SUBAGENT_DEPTH` are removed; the archive is never pruned; the live-children ceiling is configurable via `AgentServerOptions.subagents.maxLiveSubagents`; a child can be barred from delegating with `--no-subagents` or profile `canSpawnSubagents: false`.
