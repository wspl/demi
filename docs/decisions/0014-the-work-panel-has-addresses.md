---
status: accepted
---

# The work panel has addresses

## Context and Problem Statement

The page's address names only the conversation, such as `/chat/c1`. What
the work panel shows, whether it is open, which tab it selects and where
that tab is, such as a file in Files or an edit in Changes, is kept
elsewhere. A reload, a copied link or the browser's Back button cannot
bring the panel back to where it was. Files and Changes keep a back and
forward history of their own inside their tab, apart from the browser's.

## Considered Options

* The address holds the panel's state, and the browser's history holds the
  panel's moves, as a query beside the conversation's path.
* The address holds the panel's state as more path segments.
* Keep the panel out of the address.

## Decision Outcome

Chosen option: the query beside the conversation's path, because the panel
belongs to the conversation it stands beside, as Notion's side peek and
Linear's side panels put theirs in the query.

### The address

```text
/chat/c1                                        the panel closed
/chat/c1?panel=file&path=src/main.ts            Files at a file
/chat/c1?panel=change&edit=3                    Changes at an edit
/chat/c1?panel=t7                               a browser tab
```

* `panel` names the selected tab: a pinned tab by its kind, any other tab
  by its id. The address has no `panel` while the panel is closed.
* Each tab's kind says how its place is written into the address and read
  back. The page reads the rest of the query only through the kind it
  belongs to, and a place the kind cannot read shows the kind's start.
  A plugin's page declares this through the plugin SDK.
* An address with a `panel` shows what it names. An address without one,
  such as one the sidebar opens, shows the panel as the user last left it
  in that conversation, and the address then says so.
* A tab the address names that no longer exists gives way to the tab the
  panel would show without it, and the address is corrected in place.

### The history

* Opening and closing the panel, selecting a tab, and moving to another
  place in a tab, such as another file or folder in Files or another
  edit or commit in Changes, are each a step of the browser's history:
  Back returns to where the panel was.
* A change of how the same place is seen, such as Preview or Source,
  replaces the address without a step.
* The back and forward buttons in the panel's address bar are the
  browser's Back and Forward, available while the step they would take
  stays in this conversation's panel. Files and Changes keep no history of
  their own.
* On a phone, where the panel covers the conversation, the panel's moves
  are steps as on a desktop: Back returns through them, and Back from the
  panel's first step closes it and shows the conversation.

### Consequences

* Good, because a reload, a copied link and Back bring the panel to where
  it was, as in a web browser.
* Good, because one history serves the browser's Back and the panel's own
  buttons.
* Bad, because the page's history holds the panel's moves among the
  conversations, so leaving a conversation by Back can take several steps.
