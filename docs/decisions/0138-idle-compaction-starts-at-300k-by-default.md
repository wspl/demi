---
status: proposed
---

# Idle compaction starts at 300K tokens by default

Supersedes: 0124 (The context an idle conversation compacts from is a
setting), its default of 200,000 tokens.

## Problem

Alex works with a model whose window is 1,000,000 tokens and never changed
Settings › Agent › Context › Minimum Context Size. A conversation at
220,000 tokens is summarized each time Alex leaves it for an hour, so it
loses its detail long before its window is in reach.

## Decision

Minimum Context Size is 300K until the user changes it:

```text
│ Minimum context size                                    [ 300    K ] │
```

- `IDLE_COMPACTION_CONTEXT`, which the backend applies for an absent
  `idle_compaction_context` and the page shows in the field, becomes
  300,000. The floor stays 100,000.
- A user who stored a value keeps it, 200K included; only a user who never
  set one moves to 300K.

What older pieces meet: an older page shows 200K for a user who set
nothing, while the backend compacts from 300K, until the page is reloaded
from the new release.

## Spike

None.

## Prior art

Claude Code compacts an idle conversation from 200,000 tokens, and its
`CLAUDE_CODE_IDLE_COMPACT_MIN_TOKENS` changes that minimum
([changelog](https://code.claude.com/docs/en/changelog)).
Demi departs from it for windows of 1,000,000 tokens, where 200,000 is a
fifth of the window.

## Alternatives

- **A default that scales with the window,** such as a share of it. The
  row would show a different number for each model, while the setting is
  one number for every conversation.

## Open questions

None.
