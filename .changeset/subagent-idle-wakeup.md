---
'@demicodes/agent': patch
---

The idle-parent wakeup and send-parent delivery now carry the metadata of the round that spawned the child, so harness hooks (host, systemPrompt, preamble) see the same action metadata as the spawn round. New `subagents.notifyParentOnIdle` server option lets a host app disable the automatic wakeup and orchestrate the parent itself from the `subagent closed` frame.
