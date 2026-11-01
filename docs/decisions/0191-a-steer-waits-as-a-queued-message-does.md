---
status: proposed
---

# A steer waits as a queued message does

Supersedes the steer's tint and the row's spacing of
[0186](0186-a-waiting-message-holds-its-controls.md).

## Problem

The user sends "Keep the old name." while the agent works, and it waits as a
steer. Its bubble is tinted with the accent and its clock is in the accent's
color, unlike a queued message's, though both are the same kind of thing: a
message not sent yet. On the tint, the gray fill a pointer gives its Send
reads as dirt rather than as a control, and that fill stands 8 pixels from
the bubble's start but 10 from its bottom, askew in the corner.

## Decision

A steer stands on the same fill as a queued message, half a sent bubble's,
and its clock is the same faint gray as the queue's mark: only the mark and
its words tell the two apart.

A bubble's row of controls, a waiting message's or a long one's Show More,
stands as far from the bubble's bottom as its first control's fill stands
from the bubble's start, 8 pixels, so a hovered control sits square in the
corner.

## Spike

None.

## Prior art

None: this takes back a color 0186 added.

## Alternatives

- A tint on the steer's clock alone: the one colored mark in the transcript
  draws the eye to a message that needs nothing from the user.

## Open questions

None.
