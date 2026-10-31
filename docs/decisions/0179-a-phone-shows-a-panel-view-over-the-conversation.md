---
status: proposed
---

# On a phone, a view the conversation opens shows as an overlay over it

Supersedes: 0004 (a conversation's pictures and videos open in the work panel),
its last clause, that on a phone the work panel covers the screen as the
dialog did. Its other clauses, and 0014 (the work panel has addresses), still
hold.

## Problem

Alex reads a conversation on a phone and taps a screenshot in a reply. The
whole screen slides away to the right and the work panel takes its place: a
bar with a Back chevron and a strip of tabs, and under it the Media view with
the picture. To see one picture, Alex has left the conversation for another
screen, fenced by a tab strip. Going back, glancing at the reply and tapping
the next picture repeats the whole screen change each time.

A computer shows the panel beside the conversation, and its strip holds the
views Alex works with there. A phone has no room beside the conversation. The
strip still matters there: the conversation's Tabs button opens the panel, and
the strip is how Alex reaches Changes, Files, a browser tab or a device. What
it costs is a picture or a file Alex only wants to look at, which iOS Photos,
Telegram and Mail show over the thread and close with a tap or a flick.

## Decision

A phone shows the work panel in one of two ways, and the way it opened says
which:

- **The Tabs button** opens the full panel as today: full screen, sliding in
  from the right, under the bar with Back to Conversation and the strip.
- **A view the conversation opens** while the panel is closed, such as a
  picture tapped, a file a message names, a change, a web page, a device or a
  widget, fades in over the conversation as an overlay: the view alone, with
  no strip and no bar of the panel's. A view opened inside an open panel,
  such as a file a Change names, shows the way the panel shows.

The overlay's top is the view's own bar, which ends with Close (✕), so Close
and the view's controls read as one bar:

```text
 phone, reading a conversation          taps a picture
 ┌───────────────────────┐             ┌───────────────────────┐
 │ ≡  Fix the login page │             │ ▣ shot.png ⤓ ‹3/12› ✕ │ ← the view's own bar,
 │                       │             ├───────────────────────┤   ended by Close
 │  …see [▣ shot.png] …  │  ── tap ──▶ │                       │
 │                       │   overlay   │    ◀ swipe ▶          │ ← steps the pictures
 │                       │   fades in  │                       │   and videos
 │ [ message…        ↑ ] │             │    pull down ▼        │ ← closes the overlay
 └───────────────────────┘             └───────────────────────┘
```

- **Close** closes the overlay to the conversation, as the full panel's Back
  to Conversation does, however many views it has shown.
- **In Media,** across a picture or a video:
  - A swipe left or right steps the conversation's pictures and videos
    (0004).
  - A finger pulling the medium down moves it down and fades it; lifted past
    72 px it closes the overlay, and short of that, or when the system cancels
    the touch, it springs back. A picture pulls down only at its fitted size,
    as a zoomed one pans.
  - A tap beside the medium, on the space it leaves empty, closes the overlay
    too, as the phone's full-screen viewer closes.
  - A tap on a picture still switches it between fitted and its own size, and
    a tap on a video, or anything on its player's controls along the bottom,
    such as dragging the playhead, is the player's: only a finger that moves on
    the video's picture becomes a swipe.
- **A swipe from the screen's left edge** drags the overlay aside, the
  conversation showing beneath, and lets go of it as it lets go of the full
  panel: past 30% of the width or with a flick it slides away to the
  conversation, otherwise it springs back.
- **The browser's Back** steps back one view, as the panel's history does
  (0014): to the view opened before, or to the conversation from the first.
- **Motion:** the overlay fades in over 200 ms, growing from 98% of its size,
  and fades out the same way; one swiped away slides out from where the finger
  left it. With reduced motion it shows and goes at once.
- **Where a view has no bar,** the overlay gives Close one: over a Host that
  is offline, a tab the panel cannot show, and a widget, which shows its title
  before it.
- **A device's toolbar** keeps Close's room at its end, its actions folding
  into More before it, and names the device, which the strip names in the full
  panel.

The conversation stays mounted under the overlay, so closing it shows the
conversation with its scroll and its draft as they were. A computer is
unchanged.

### What is built

- **Which way an entry shows** is a flag in the history entry's state,
  `workPanelSheet`, beside the steps 0014 keeps there, and not in the address:
  a copied link opens the full panel as before, and Back and a reload show
  each entry the way it opened. The work store (`work.ts`) writes it with
  each move: an open from the conversation (`openIn`, a tab the agent shows)
  sets it while the panel is closed; the Tabs button clears it; every other
  move keeps it. The store reads it with the move, before the router makes
  it, and gives it as `sheet`, false off a phone.
- **`WorkPanel`** takes `sheet`. As an overlay it draws none of its own bar,
  holds off the notch itself, and provides the views what closes it
  (`useSheetDismiss`), which it gives only on a phone. Closing it is closing
  the panel, which on a phone goes back to the entry before the panel opened.
- **`SheetCloseButton`** is Close: it shows only where an overlay provides
  what closes it, so a view's bar places it unconditionally. Media, File,
  Change, an attachment's file bar, the browser's address bar and a widget's
  bar end with it, as does the device toolbar, in a new `end` slot whose room
  its actions fit around. The tree frame and the address bar gain an `end`
  slot for it.
- **`swipe-layer.ts`** is the one layer a swiped medium slides on, with its
  decisions in `swipe-step.ts`: `ImagePreview` and the new `SwipeVideo`, which
  wraps the video player for `MediumStage`, both report their swipes to it.
  It closes only on a release that `pointer-drag.ts` or the video's own press
  says was the finger's lift, never on a cancel, a lost capture or the
  medium's unmounting.
- **`SidebarLayout`** takes `asideSheet` and fades the overlay in, keeping the
  way the panel came while it leaves.
- An older page knows no flag and slides the full panel in, as before.

## Spike

None. The overlay reuses the panel's history, its views and their bars.

## Prior art

- **iOS Photos, Telegram, Signal, WhatsApp:** a picture or a video in a thread
  opens over the thread with a close control at the top, a swipe steps the
  series, and a pull down closes it. None shows a tab strip over it.
- **iOS sheets:** a view presented over another closes with a control in its
  own bar, back to where it was presented from, not step by step.
- **Mobile Safari:** its tab button opens the tabs, and a link opens a page,
  not the tab overview; Demi's Tabs button and an open from the conversation
  part the same way.

This departs from them in that the series a swipe steps is the conversation's
whole run of pictures and videos (0004), not an album.

## Alternatives

- **Drop the strip from the phone's panel altogether.** The Tabs button would
  open a panel with no way to reach Changes, Files, a browser tab or a device.
  Rejected.
- **A media-only lightbox beside the panel's Media view.** A second viewer for
  pictures would split one purpose in two, and leave files, changes and pages
  as they are. Rejected.
- **A Back chevron at the start of the view's bar, stepping back one view.**
  What Alex opened from the conversation is a look, and leaving it means
  returning to the conversation; the browser's Back still steps through the
  views. Rejected.
- **Rise from the bottom, as a sheet.** It claims the vertical axis that a
  pull down on a medium uses to close. A fade claims none. Rejected.
- **The picture grows from its thumbnail.** It needs the thumbnail on screen to
  grow from and shrink to, which a swipe through the series or a scroll takes
  away, and it holds only for pictures. Rejected for its cost.
- **A tap on the medium closes it.** A tap on a picture zooms it and one on a
  video is the player's; a tap beside the medium closes it instead. Rejected.

## Open questions

None.
