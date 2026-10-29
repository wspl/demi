---
status: proposed
---

# An archived conversation's panel still opens tabs

Supersedes: 0153 (A reply shows a widget), an archived conversation's widget
opening in a new tab of the web browser.

## Problem

Alex archives a conversation whose reply attached `report.md`, then opens it
from Archived to read it again. He clicks the attachment's card: an error
says "Could Not Change the Work Panel. This conversation is archived. Restore
it first.", and no preview opens. Open in Work Panel on a link fails the same
way, and so does closing a tab the panel already holds. Everything the
conversation offers to open in its panel leads to the same error.

The backend refuses every change of an archived conversation's panel, as it
refuses a message or a draft. Its tabs, though, are not what the conversation
says: they are how Alex looks at it.

## Decision

Archiving freezes what a conversation says, not how it is viewed. The panel
of an archived conversation opens, selects, moves and closes tabs as any
other's does:

- An attachment previews from its stored copy, and a widget opens in its tab.
- A view that reads the conversation's Host, such as Files, Changes or the
  browser, says in its tab that the conversation is archived, as it does
  today: archiving releases the Host, and restoring it brings the Host back.

`POST /api/conversations/:id/panel/changes` no longer answers
`conversation_archived`; the store applies an archived conversation's
changes as any other's, and `PanelOutcome::Archived` and
`PanelError::Archived` go.

A reply block whose conversation is archived opens in the panel again, not in
a new tab of the web browser.

## Spike

None.

## Prior art

ChatGPT shows an archived chat read only, and its files and pictures still
open. Slack's archived channels are read only, and their files open and
preview.

## Alternatives

* **A read-only panel in an archived conversation:** every control that opens
  a tab would have to hide or change there, an attachment would only
  download, and Alex could see nothing of what the conversation holds without
  restoring it.

## Open questions

None.
