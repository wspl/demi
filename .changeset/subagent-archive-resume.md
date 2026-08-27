---
"@demicodes/agent": patch
---

Finished subagents move to an archive instead of being deleted: their transcript checkpoints stay on store (capped, pruned oldest-first), `demi agent list` shows an archived section, and the new `demi agent resume <id> <message>` revives an archived child on top of its preserved transcript. Parent restores still skip archived children.
