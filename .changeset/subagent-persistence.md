---
'@demicodes/agent': patch
---

Subagents now persist exactly like their parent: each live child keeps a checkpoint and job record under the parent's session directory, closing the parent connection detaches children (flush, no `closed` frame) instead of aborting them, and reopening the parent restores every persisted child and resumes its interrupted turn. `SubagentJob` frames carry the spawn round's `metadata`.
