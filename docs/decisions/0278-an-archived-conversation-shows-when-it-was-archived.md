---
status: accepted
---

# An archived conversation shows when it was archived

Supersedes: 0277 (The archived list shows the last archived first), that no
protocol changes.

## Problem

Alex opens Settings > Archived, whose rows run from the last archived to the
first, and each row's end shows when the conversation was last active. The
times jump up and down, so the list looks unsorted.

## Decision

**A row of Settings > Archived shows when its conversation was archived.** The
time keeps its format, and a conversation archived before 0277 shows the time
that record gave it.

The design to build from:

- `ConversationSummary` gains `archivedAt`, nullable. It keeps `archived`, which
  a page from before this reads until it reloads.
- `ConversationRecord` holds `archived_at`, and `archived()` answers from it.
- A page that archives a conversation shows the time by its own clock until the
  backend's answer names it. A draft, which only the page holds, keeps that
  time, in what the page stores of it.

## Spike

None.

## Prior art

None: this adds no capability of its own to learn from.

## Alternatives

- **Both times in a row.** The row has room for one, and the list is sorted by
  this one.

## Open questions

None.
