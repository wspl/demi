---
status: proposed
---

# The sidebar lists the projects that hold conversations, and a fold keeps the open one

## Problem

Alex archives the last conversation of the project *scratch*. The sidebar
still lists *scratch* under Projects, a header with nothing under it, and
keeps it there until Alex deletes the project. Ten such projects push the
projects that have work in them down the list.

Alex then opens *Fix the footer links* in the project *website* and folds
*website* to tidy the list. The row of the conversation on screen goes with
the fold, so the sidebar no longer shows where Alex is. When Alex opens a
conversation in a folded project from the search window, the sidebar unfolds
that project instead, undoing the fold Alex chose.

## Decision

A project is listed while it holds a listed conversation. The open
conversation is always listed, an empty new one included, so a project shows
from the moment a conversation opens in it, and one whose last conversation
is archived or deleted leaves the list. Add project on that project's folder
brings it back, with a new conversation in it, as it does today for a folder
that is already a project.

A fold never hides the open conversation. A folded project lists that row
alone under its header, as a folded group already does under its label, and a
project inside a folded Projects group shows its header and that row. The
sidebar no longer unfolds a project when a conversation in it opens: the fold
stays as the user left it.

```
PROJECTS                       PROJECTS
▾ website                      ▸ website        (folded)
    Fix the footer links  ◀       Fix the footer links  ◀ open
    Redesign the pricing page
▸ research-notes               ▸ research-notes
                               (scratch, with no conversation, is not listed)
```

The rule lives in the sidebar component, `web-ui`'s `AppSidebar`:

- `projectGroups` answers only the projects with a listed conversation, and
  the sidebar's projects for dragging, reordering by keyboard and telling
  namesakes apart are those groups' projects.
- `listLines` keeps the open conversation's line under a folded project.
- `useRevealOpen`, which unfolded the project, is removed.

The page, the backend and the stored data do not change: the projects and the
folds are the same; only what the sidebar draws from them does.

## Spike

None.

## Prior art

- Slack: a collapsed sidebar section still shows the channel being viewed,
  and its unread channels, and navigating to a channel in it does not expand
  it.
- Discord: a collapsed channel category keeps the selected channel and the
  unread ones in sight.
- Codex's desktop app lists a project with no threads as *No chats*. This
  departs from it: a header with nothing under it is a row to scroll past.

## Alternatives

- Keep empty projects, marked as empty, as Codex does: every finished project
  stays in the list until deleted, and deleting it is the only way to tidy.
- Keep unfolding a project when a conversation in it opens, as Finder reveals
  a file: the fold the user chose is undone each time, and the open row shows
  without it now.

## Open questions

None.
