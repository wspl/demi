---
status: proposed
---

# The sidebar collapses on a computer

## Problem

On a computer the sidebar always takes its width. A user reading a long
conversation, or working beside the work panel on a small screen, cannot
put it away; the sidebar's shortcut, which the keyboard settings list,
does nothing there.

## Decision

**A button at the right end of the sidebar's top row, beside `demi`,
collapses the sidebar; the same button at the left of the page's title
brings it back.** The sidebar slides out to the left while the main pane
widens with it, and back in the same way.

```text
Open                                    Collapsed
┌──────────────────┬─────────────────┐  ┌──────────────────────────────┐
│ demi          [◧]│ Title       ⋯ ◨ │  │ [◧] Title                ⋯ ◨ │
│ New              │                 │  │                              │
│ Search           │  conversation   │  │        conversation          │
│ …                │                 │  │                              │
└──────────────────┴─────────────────┘  └──────────────────────────────┘
```

* The button is the Panel Left icon the phone's drawer button uses, with
  the tooltip "Hide Sidebar" or "Show Sidebar" and ⌘\, which does the same.
* The shortcut's default is ⌘\, as Notion
  toggles its sidebar. ⌘B stays the composer's bold and ⇧⌘S its
  strikethrough: the focused control takes its keys first, so a sidebar
  shortcut on either would never run while the composer, where the user
  usually is, has the focus.
* The conversation's top bar shows the button left of its title while the
  sidebar is collapsed.
* The motion is 200 ms, ease out; with reduced motion the sidebar shows or
  hides at once.
* The browser keeps whether it is collapsed, as it keeps the sidebar's
  width, so a reload opens it as it was left.
* A phone is unchanged: its sidebar is the drawer.

The design to build from:

* `SidebarLayout` gains `collapsed` (v-model, a computer only) beside its
  phone-only `open`; the host keeps it in the browser's local settings
  beside `sidebarWidth`.
* The conversation's top bar takes the button through the one provided entry the
  phone's drawer button uses (`useSidebarDrawer`), so the button has one
  component and one place that decides where it shows.
* The gallery's frame specimen shows both states and the motion.

## Spike

None.

## Prior art

* Notion toggles its sidebar with ⌘\
  ([Notion Help](https://www.notion.com/help/navigate-with-the-sidebar))
  and keeps ⌘⇧S for strikethrough in its editor
  ([Notion Help](https://www.notion.com/help/keyboard-shortcuts)).
* Finder hides and shows a window's sidebar with View › Hide Sidebar and
  View › Show Sidebar
  ([Apple Support](https://support.apple.com/en-us/127254)); the window's
  content takes the width.

## Alternatives

* **A narrow rail of icons when collapsed.** It keeps part of the width
  the user asked to give back.

## Open questions

None.
