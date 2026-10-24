---
status: accepted
---

# A comment and a side question stand under the line they are on

Supersedes:

* 0054 (a selection opens a comment field), where the field stands above
  the selection with its words over Send, Ask and Comment, and a phone
  opens it in a bottom sheet.
* 0056 (a side question can be stopped), where × folds a card into a mark
  in the reply's margin.

## Problem

A user reads a reply, selects "the same 30-second timeout" and comments
"Should uploads get their own, longer timeout?". Today:

* The comment's words are hidden. The reply shows only the yellow underline
  and a yellow mark in its right margin; the words show in a floating card
  after a click on either, and the card closes on the next click elsewhere.
  Reading three comments takes three clicks.
* A side question's card stands after the whole paragraph that holds its
  words, which in a long paragraph is several lines away from them. Folded,
  it leaves only a purple mark in the margin.
* While the draft carries a comment, the message field opens into its tall
  shape for the "1 Comment" pill alone, even with no text typed.
* The field a selection opens is two rows for a remark of a few words, its
  words over three buttons, and stands above the words it is about, over the
  lines the reader has just read.

## Decision

### Under the line

A comment and a side question's card stand right under the line where their
words end, inside the reply. The rest of the paragraph goes on under them:

```text
  Uploads fail on slow connections because the client gives up long
  before the server does. The upload request runs under ‾the same
  30-second timeout‾ as every other API call, so a 40 MB video on a 3G
  ╭───────────────────────────────────────────────────────────────╮
  │ (…) Should uploads get their own, longer timeout?       🗑  ✎ │
  ╰───────────────────────────────────────────────────────────────╯
  connection is cut off part of the way through, and the retry
  starts again from the first byte.
```

* **The line.** The line that holds the end of the words, or of the
  picture, as the reply lays out now. Words that end on their block's last
  line put it after the block. In bold words or a link that goes on to the
  next line, it stands between the two lines all the same, the words bold or
  linked on both sides of it. In a code block, a table, a display formula or
  a row of pictures, which cannot be cut, it stands after the whole of it.
* **Not the text.** The boxes and cards stand in the reply but are none of
  its text: a mark on words drawn across them skips them, and Copy of a
  selection across them copies the reply's words alone.
* **Several.** On one line, the comments stand first, then the side
  questions' cards, each in the order it was made.
* **Width.** When the reply's width changes, the lines wrap again and each
  moves under its new line.
* **The comment.** A box with a thin border and round corners, the side
  question card's, holding a yellow `MessageCircleMore`, the words, and
  Delete and Edit. Edit turns the box into the editor in place. A sent
  comment stays, its words and mark dimmer, with ✓ Sent in place of the
  buttons. The yellow underline stays on the words, dimmer once sent.
* **Gone.** The margin's yellow and purple marks, the comment card a click
  opens, the deeper yellow of an open card, the deeper purple and pointer
  over a side question's words, and the phone's sheet of a line's comments.

### A side question folds to its header

The card's × becomes a chevron. Folded, the card keeps only its header line
in place: the purple mark, the title, ↗ and the chevron. A click on the
header, or on the chevron, opens or folds it. The header stands where it
stood in the open card, so it does not move.

```text
  ╭───────────────────────────────────────────────────────────────╮
  │ (?) Why uploads time out                                ↗   ⌄ │
  ╰───────────────────────────────────────────────────────────────╯
```

A question still on its first answer, which × deleted, is deleted from the
card's right-click menu, Delete.

### The selection's field is one line, under the selection

The field a selection opens is a capsule of one line: the words, a gray
round Ask, and a blue round Comment. It stands under the selection, centered
on it, and above it only where there is no room under it.

```text
  …connection is cut off part of the way through, and ▒the retry starts
  again from the first byte▒.
            ╭──────────────────────────────────────────────────╮
            │ Is 30 s too short for 3G?|                (?)  (…) │
            ╰──────────────────────────────────────────────────╯
```

* **The buttons.** Ask is the side question's mark,
  `MessageCircleQuestionMark`, on a gray disc, as the message field's
  paperclip. Comment is the comment's mark, `MessageCircleMore`, on a blue
  disc, as the message field's send button. Return presses Comment.
* **Send.** The field has no Send button. While ⌘ is held on a Mac, or
  Control elsewhere, Comment turns into the send button, an up arrow, and
  ⌘Return (Control-Return) sends the quote and the words to the main agent
  now. Comment's tooltip says so, as the message field's send button says
  its other way: `Comment · ⌘⏎ to send`, and `Send` while the key is held.
* **Ask from the menu.** The right-click menu's Ask opens the field with
  the two swapped: the blue disc is Ask, so Return asks, and the gray one is
  Comment. ⌘ turns the blue disc into the send button as well.
* **More than a line.** Once the words outgrow the line, the field grows a
  row: the words above, the buttons under them at the right, as the message
  field does.
* **Closing.** Escape, a click elsewhere or a new selection; the field has
  no ✕. With no words, Return and the buttons shake it once, as before.

### A phone takes no remark on a selection

A selection on a phone opens nothing: no field, no sheet. The menu a long
press on a picture opens has no Comment or Ask. The phone shows the comments
and side questions made elsewhere, under their lines, and its message field
still lists and sends the waiting comments. The bottom sheet of 0054, and
its quote line, are removed.

### The message field stays on one line

The comments' pill stands by the paperclip, and the field keeps its
one-line shape until the text needs more:

```text
  ( 📎 ) [ 💬 2 Comments  × ]  Ask anything…                    ( ↑ )
```

Once the text outgrows the line, the field opens as it does for any long
text, and the pill moves with the paperclip to the row under the text. An
edit of a sent message keeps its caption row over the text.

### Build

All in `web-ui`; nothing stored or sent changes.

* `ReplyComments` owns one place per comment and per side question: an
  element marked `data-no-anchor`, so anchors and copies skip it, set at the
  start of the next line of the block that holds the end of the words. The
  place is found by walking the block's text from that end until a
  character lies on a lower line, and the text node is split there. It is
  placed again when the marks, the reply's text or the list's width change,
  and its own changes are dropped from the mutation observer.
* `CommentItem` renders the comment's box; `ReplyComments` teleports one
  into each place, as it does the side questions' cards.
* `AskCard` takes `folded`; the header becomes the fold's button.
* `SelectionField` lays out its words and buttons in one grid that moves
  the buttons under the words once they outgrow the line, measured at the
  one-line width in both shapes; it follows the held modifier while it has
  the focus. `SelectionFieldHost` places it under the selection's last line
  and flips it above the first. On a phone it opens nothing, the pictures'
  menus offer no Comment or Ask, and `SelectionFieldQuote` is removed.
* `ComposerShell` takes a `carried` slot, the column after the paperclip in
  both shapes; `SessionComposer` puts its `context` slot there and opens
  the shell only for the text or an edit.

## Spike

Splitting a paragraph at the start of the next line keeps the lines above
as they were: a block set before the first character of a line breaks the
line where it already broke. Tried on the gallery's reply at 1400 px: the
lines above stayed, and the paragraph went on under the box. Equal margins
above and below the box leave 11 px over it and 10 px under it to the text.

## Prior art

* GitHub and GitLab show a review comment under the line of code it is on,
  with the thread in a bordered box; a submitted comment stays there.
* Google Docs and Notion show comments in the right margin, which needs a
  page wide enough for a column of cards.
* ChatGPT shows a chosen tool as a pill by the + button on the one-line
  message field.
* Notion's Ask AI field opens under the selected text, one line wide.
* macOS changes a menu item while Option is held, such as Finder's Close
  Window becoming Close All; Demi's message field already names its other
  way in its send button's tooltip.

Demi departs in cutting a paragraph of prose at a line, which code review
never has to do.

## Alternatives

* Keep the box after the paragraph, as side questions did: the paragraph
  stays whole, but in a long one the box is far from its words.
* A column of comments in the margin: the conversation column is too
  narrow for one, and a phone has none.
* Keep the words in a card on click: three comments are three clicks.
* The selection's field with text buttons, Send, Ask and Comment: the
  widest, 460 px.
* A switch of Comment, Ask and Send at the field's start with one round
  button: one more choice before every remark.
* One Comment button with a menu of Ask and Send: Ask, a common act, hides
  in a menu.
* Words on both buttons, Ask and Comment: wider, and unlike the message
  field's round buttons.

## Open questions

None.
