---
status: accepted, superseded in part by 0049
---

# A work panel pane changes whole

## Context and Problem Statement

A pane of the work panel shows a Host's file, its changes or its browser
tab. It reads several things before it is complete, and shows each as it
lands: a file's text and then the bar with its kind, size and lines; a
folder's ancestors one listing after another; the change list, then the
selected file's two sides. Switching to a file not read yet clears the pane
to a spinner at once. The browser pane reads its tab list from the Host
through the backend each time, then opens its stream, which sends the same
list.

## Considered Options

* Show each part as it arrives, with room reserved for it.
* A pane switches to the next view at once, shows nothing of it until all
  its parts are in, and the reads a view needs are made together.

## Decision Outcome

Chosen option: a pane changes whole, because what the user sees is the
thing they opened, complete, never a mix of parts.

* A click switches the pane at once: its tab, address and content area are
  the new view's. The content area stays empty while the view's reads are in
  flight and shows the view when all its parts are in. If they take longer
  than 300 ms, it says what it waits for, as the media viewer does (0018).
* The reads a view needs, when their paths are known at once, are made
  together: a file's text, its description and its folder's listing; the
  listings of a deep file's ancestors, in one request; the change list and
  the changes it selects first.
* The backend keeps the browser's tabs of each conversation, told by the
  Host when they change, as it keeps other Host facts (`kept.rs`); the pane
  takes them with the panel, and the Host is not asked for them as a pane
  shows. The live view's stream keeps telling its tabs and their viewports
  as they change.

```text
  Switching from a.ts to b.ts (not read yet)

  before:  [ a.ts ] -> [ spinner ] -> [ b.ts text ] -> [ bar + b.ts ]
  after:   [ a.ts ] -> [ b.ts, empty ] ---------------> [ bar + b.ts ]
                       (after 300 ms: "Reading b.ts")
```

### Performance

* Measured: a deep file's eight ancestor listings, read one after another,
  take 21 ms locally and 2.4 s with the Host 300 ms away; one request saves
  about 2.1 s there.
* Removed: one Host read per browser pane shown, now held by the backend.

### Consequences

* Good, because a pane never shows half a view or blanks between two.
* Bad, because a pane is empty for the moment its reads take, up to 300 ms
  before it says it is reading.
