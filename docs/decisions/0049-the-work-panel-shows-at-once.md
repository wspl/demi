---
status: accepted
---

# The work panel shows at once

Supersedes, in part: 0029 (a conversation opens whole), where the panel waits
for the opening, and 0038 (a pane changes whole), where a pane's first view
is blank for its first 300 ms.

## Context and Problem Statement

The user reloads the page. For most of a second the work panel is a blank
frame: no tab bar, no text. Then its tabs appear over a white content area,
which stays white for another third of a second before the files show.

The panel waits twice. It is not drawn until the conversation has opened,
from the disk's copy or the socket; after a deploy the copy is of another
build, so the socket's opening, two round trips, is always waited for.
Then the pane's first view stays empty while it reads, and says what it
reads only after 300 ms. Before 0029 and 0038 the same moment showed the tab
bar and a placeholder, and then "Reading…".

## Considered Options

* Keep the waits, and draw a placeholder frame meanwhile.
* Know the panel's tabs before the conversation opens, and say what a first
  view reads from its start.

## Decision Outcome

Chosen option: the panel shows at once, because a panel whose tabs are known
has nothing to wait for before it is drawn, and a blank area tells the user
nothing.

* The conversation's entry in the list carries its panel's tabs, under the
  revision that counts their changes, so the panel draws its frame and its
  tabs as soon as the conversation is in the list, from the disk's copy of
  the list or from the channel's first state. Which tab is selected is the
  page's own, from its address and the selection it kept, both of which it
  has before any list arrives, so the right tab is selected in the first
  frame. A change of the panel is announced with the entry, to every page.
  The conversation's opening no longer carries the tabs; it still carries
  the states the plugins hold for them.
* A pane showing the first view the panel shows, as the page loads, the
  panel opens or its kinds are bound anew, says what it reads from the
  start, then shows the view whole. A pane that has to wait for the
  conversation's working directory, as Files and Changes do before the
  opening brings it, says it reads as well; it never shows an empty list in
  place of the files. A switch from one view to another keeps 0038's rule.

```text
  Reloading a conversation whose panel shows Files

  before:  [          ] -> [ Changes  Files ]  -> [ Changes  Files ]
           (blank frame)   [                ]     [ src/  README   ]
                           (white)
  after:   [ Changes  Files ]  ----------------->  [ Changes  Files ]
           [  Reading Files  ]                     [ src/  README   ]
```

### Performance

* The list's entries grow by their panels' tabs, a few hundred bytes for a
  conversation with a few tabs, and the offline copy of the list with them.
  A change of a tab's data, such as a browser tab's address, now sends the
  entry to every page, not the metadata to the pages that show the
  conversation.
* The panel's tabs show when the list arrives, about one round trip after
  the page asks, where they showed after the opening's two more.
* The files themselves show no sooner: their reads still need the working
  directory the opening brings.

### Consequences

* Good, because a reload never shows an empty panel.
* Bad, because a fast first read shows "Reading" for a moment before its
  view.
