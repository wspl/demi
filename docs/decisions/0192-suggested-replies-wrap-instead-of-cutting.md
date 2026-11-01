---
status: proposed
---

# Suggested replies wrap instead of cutting

Supersedes the one-line row of
[0182](0182-a-reply-offers-the-users-likely-next-messages.md).

## Problem

After an answer, the row offers "Merge it, then ship a release to the beta
channel for the team" and "Run the whole auth suite again on the staging
database first". They do not fit on one line, so each is cut to "Merge it,
then ship a rel…", and the user has to hover each to read what a click
would send. On a phone the line scrolls sideways instead, hiding the last
pill past the screen's edge.

## Decision

Every pill shows its words whole. A pill that does not fit beside the ones
before it goes to the next line, and a pill wider than the row wraps its
words inside it, breaking a word longer than the pill, such as an
identifier; its corners stay as round as a one-line pill's ends, its mark
stands on its first line, and Edit at the end of its last line. A phone does
the same; the row no longer scrolls sideways. The tooltip that showed a cut
pill's words is gone with the cut.

The row still keeps one line's height while the suggestions are written and
when none came. Suggestions that need more lines make the row taller when
they arrive, and what follows it moves down: the space after the answer, or
a compaction's divider when the conversation was compacted after the answer.

## Spike

None.

## Prior art

- Material Design's chip group lays chips out on as many lines as they need,
  the default of Android's `ChipGroup` (`singleLine` false); a single line
  that scrolls is its opt-in alternative.

## Alternatives

- Keep the cut and widen the row: a long suggestion is still cut.
- One pill per line: three short replies take three lines.

## Open questions

None.
