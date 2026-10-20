---
status: accepted
---

# A conversation's pictures and videos open in the work panel

## Context and Problem Statement

A picture or a video in a conversation opens in a dialog over the page. The
dialog hides the conversation, so a user cannot read the reply beside the
picture it explains, and it shows one item at a time: going to the next
picture means closing it and finding the next one. Files, commits and
attachments already open in the work panel beside the conversation.

This record covers where a conversation's pictures and videos open and how
the user moves between them. It leaves out a strip of thumbnails and how
pictures look in a message.

## Considered Options

* One Media tab in the work panel shows the conversation's pictures and
  videos one at a time, with Previous and Next, and keeps the dialog's zoom
  and pan.
* Keep the dialog and add Previous and Next to it.
* Open each picture as a tab of its own in the work panel.

## Decision Outcome

Chosen option: one Media tab in the work panel, because the conversation
stays in view, and one tab holds the whole series instead of a tab per item.

* Opening any picture or video of the conversation opens the work panel's
  Media tab on it. The dialog goes away.
* The series is every picture and video of the conversation, in the order
  they appear: the user's and the agent's attachments, and the pictures and
  videos of the Host a reply shows.
* The tab's top shows the item's name, its place in the series ("3 / 12"),
  Previous, Next and Download. The Left and Right Arrow keys move through the
  series.
* A picture zooms and pans as in the dialog: the wheel or a pinch zooms,
  dragging pans, and a click switches between fitting the panel and the
  picture's own size, about the point clicked. A video plays with the video's
  own controls.
* An attachment shows its file while its Host is online and the file is
  there, and its copy otherwise (`0003-attachment-opens-its-file.md`).
* On a phone the work panel covers the screen, as the dialog did.

```text
 conversation                       │ Changes  Files  [▣ Media]
                                    │ ‹  screenshot.png   3 / 12  ›   ⤓
 …see [▣ screenshot.png] ── open ──▶│ ┌──────────────────────────────┐
                                    │ │                              │
                                    │ │   picture: zoom, drag, pan   │
                                    │ │                              │
                                    │ └──────────────────────────────┘
```

### Consequences

* Good, because the user reads the conversation beside its pictures and goes
  through them without closing and reopening anything.
* Bad, because a picture has the panel's width rather than the window's.
* Modules touched: the media dialog and the work panel in `web-ui`, and the
  conversation's list of pictures and videos in `web` and the gallery. No
  data to migrate.
