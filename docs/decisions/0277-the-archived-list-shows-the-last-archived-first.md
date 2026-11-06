---
status: accepted
---

# The archived list shows the last archived first

## Problem

Alex archives a conversation, opens Settings > Archived to restore it, and
finds it somewhere in the middle of the list, since the list keeps the order
the conversations had in the sidebar. The conversation just put away is the one
most likely to be looked for, and nothing records when it was put away.

## Decision

**Settings > Archived lists the last archived conversation first.** A pinned
conversation has no place of its own there. A restored conversation goes back
to its place in the sidebar, and a later archive gives it a new time.

The design to build from:

- `conversations.archived_at` holds the time in milliseconds and replaces
  `archived`, which it would only repeat: none means not archived.
- `archive_in` writes the time, and leaves the time of a conversation that is
  archived already. The conversations a deleted project archives share one.
- The product state's order, which pages already take from the
  `conversation_order` event an archive sends, lists the archived ones by that
  time, so no protocol changes.
- A conversation archived before this has no recorded time, so the migration
  gives it `updated_at`, its last activity.

## Spike

None.

## Prior art

None: this adds no capability of its own to learn from.

## Alternatives

- **A column beside `archived`.** Two columns would say one fact.
- **Sorting in the page.** It needs the time in the protocol, and every other
  reader of the order would still see the old one.
- **Sorting by last activity.** A conversation idle for a month and archived
  just now would sink to the bottom of the list.

## Open questions

None.
