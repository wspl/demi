---
status: proposed
---

# A widget is shared, or copied as an image

Supersedes: 0153 (A reply shows a widget), its button that opens a widget
in a tab of the work panel, or of the web browser on a share.

## Problem

Alex's reply holds a chart of weekly orders, and he wants it in a team
chat. The card's one button opens the chart in a tab of the work panel,
where it shows the same chart larger beside the conversation. To send it,
he takes a screenshot by hand and crops it, or shares the whole answer, and
his team reads the text around the chart too.

## Decision

The card's corner holds two buttons, Copy as Image and Share, always shown
as the open button was. The work panel's widget tab is gone.

```text
┌──────────────────────────────────────────────── [▣▣] [⇪] ┐
│ ORDERS · Q3                                                │
│ 12,480                                                     │
│ ╭──────────────────────────────────────────────╮           │
│ ╰──────────────────────────────────────────────╯           │
└────────────────────────────────────────────────────────────┘
   ▣▣ Copy as Image (lucide Images)   ⇪ Share (lucide Share)
```

* **Copy as Image** puts on the clipboard a PNG of what the frame shows
  now: its size, the reply's appearance, and the state the user left it
  in, a point he hovered or a tab he picked, at twice its CSS pixels, on
  the reply's surface. The icon turns to a check for 1.5 seconds, as a
  copy button's does. A failure shows the toast every copy shows, Could
  Not Copy, with why.
* **Share** opens the share dialog an answer's share opens, titled
  Share "Weekly orders", the widget's page title as the page writes it, or
  Share "Widget 3" for a page without one, as an app names an untitled
  document: "Anyone with the link can see it." A widget never changes, so
  the dialog has no Update Link; Stop Sharing ends the link. The plugin
  keeps the page's title in place of the file's name it kept, which lost
  the title's punctuation; a migration turns each name back into a title.
* **The shared page** is the widget alone, as wide as the window and as
  high as in a reply, at most the window's height, centered in it when
  lower, with a line at its foot, Shared from Demi; the web browser's tab
  bears the widget's title. The reader's page has Copy as Image in the
  same corner and no Share. The page follows the reader's light or dark,
  as a shared conversation's does.
* A shared answer's widgets have Copy as Image for its reader, in the
  corner, and no Share.

### Design

* **A share holds a reply block.** `ShareTarget` gains a third target,
  which any plugin's reply block can be:

  ```rust
  pub enum ShareTarget {
      Conversation,
      Answer { block_id: BlockId },
      ReplyBlock { tag: String, id: String, title: String },
  }
  ```

  `title` is what the block's component names it, the widget's page title
  for a widget, stored as the share's `title`, which a conversation's share
  holds its title in. The table `shares` gains `kind = 'reply_block'` with
  `reply_block_tag` and `reply_block_id`, by migration. One block has at
  most one share in a conversation, as an answer has. When the share is
  made, the backend refuses a block that no text of the conversation names,
  so the share goes with the conversation the block is in, and one whose
  plugin answers no facts of it (`ReplyBlocks`), which the user does not
  have. Its snapshot holds no entries.
* **Its public routes.** `GET /api/shares/:token` answers
  `kind: "reply_block"` with the block's tag, id and facts, read when it is
  sent as a page's are. The share's block route serves the block the share
  holds as it serves a block a shared text names. A conversation's
  deletion removes its shares, this kind with the others; the widget
  plugin deletes the conversation's widgets, so a link to one answers
  `share_not_found`.
* **A reply block's component gets `share` in place of `open`.**

  ```ts
  /** Where the page can share: opens the share dialog for the block. */
  share?: (title: string) => void
  /** Drawn alone, filling its box, as a share of the block shows it. */
  whole?: boolean
  ```

  The conversation's page gives `share` where it shares an answer; a
  share gives none. The intent `block` and the widget's panel kind are
  removed. The migration takes the widget tabs out of every saved work
  panel, their ids retired as a closed tab's are: the panel shows a tab of
  a kind no page has as such, and never drops one itself.
* **The frame makes the image.** The bootstrap script answers a message
  `{ widget: "picture" }` by loading html-to-image 1.11.13 from jsDelivr,
  where Tailwind loads from, on the first copy only. It draws the
  document at the frame's size, twice its pixels, on the surface's color,
  and posts the PNG back as a `Blob`. The block writes it to the clipboard
  with `navigator.clipboard.write([new ClipboardItem({ "image/png": promise })])`
  inside the click, which Safari requires. The page around the frame cannot
  draw it: the frame is an opaque origin it cannot read.

## Spike

* **Which library draws inside an opaque origin.** modern-screenshot 4,
  which LobeChat uses, failed in all three of the gallery's widgets: it
  makes a frame of its own to read default styles, and an opaque origin
  cannot read that frame. html-to-image 1.11.13 drew all three.
* **What it draws.** The line chart (a Chart.js canvas), the heatmap (CSS
  grid, colors mixed by `color-mix`) and the flow (SVG), light and dark,
  each beside a screenshot of the frame at 2x: the same pixels to the eye,
  in 47 to 319 ms after a first load of 349 ms.
* **A frame off screen.** Chrome pauses `requestAnimationFrame` in a
  cross-origin frame out of view, and html-to-image waits on it to clone a
  canvas, so a copy of an unseen frame never ends. A frame clicked is in
  view, but one scrolled away while its picture is drawn is not: the frame
  gives its page 10 seconds, then the copy fails, saying to scroll the
  widget into view and try again.
* **What it leaves out.** An image or a font the widget loads from a site
  that refuses cross-origin reads is missing from the copy; the spike's
  widgets, Tailwind's classes, Chart.js and inline SVG, load nothing of
  that kind.

## Prior art

* Claude's artifacts offer Copy, Download and Publish; a published
  artifact is a public page that the artifact fills.
* [LobeChat](https://github.com/lobehub/lobe-chat) shares a conversation as
  an image drawn from its DOM with modern-screenshot (`useScreenshot.ts`),
  and copies it or downloads it. This departs to html-to-image, from which
  modern-screenshot forked, since the fork does not draw in an opaque
  origin.

## Alternatives

* **The Host's browser draws the image**, as `demi widget create` draws its
  screenshots: the most exact pixels, but a copy waits seconds, needs the
  Host online, loses the state the user left the widget in, and a share's
  reader has no Host.
* **The browser's screen capture** (`getDisplayMedia`) asks the user to
  pick a screen or a tab each time.
* **The share page as a conversation's**, a title above the widget's card:
  the reader came for the widget, which a full page gives its room, as a
  published artifact does.
* **Keeping Open in Work Panel beside the two:** the panel showed the same
  widget wider and nothing else, and three buttons crowd the corner.

## Open questions

None.
