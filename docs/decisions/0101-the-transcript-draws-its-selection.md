---
status: proposed
---

# The transcript draws its own selection

## Problem

A user drags across two lines of a reply, "the same 30-second timeout as
every other API call", and picks Comment. While they drag, the browser
paints the selection: a tall block as high as each line, square, that runs
to the end of the first line. The comment field opens under the line and
takes the focus, which collapses the page's selection, so the page paints
the same words again as a stand-in, a `::highlight()` mark: as high as the
letters only, a different blue, with no picture in it. Each time the focus
moves between the field and the reply, the selection jumps between the two
looks, and a picture in it loses its tint altogether.

The browser gives a page no way to keep its own selection painted while a
field has the focus, and no way to make its selection match a stand-in:
`::selection` sets a color, not a height, a corner or a picture's look.

## Decision

**The conversation's list paints no browser selection; the page draws it,
in one look, whether the reply or a field has the focus.** Each line of the
selection is a rounded strip as tall as the line, with a small gap between
lines; a picture in the selection gets a blue outline instead of a tint.

```text
   Uploads fail on slow connections ╭────────────────────────────╮
                                    │because the client gives up │
                                    ╰────────────────────────────╯
   ╭──────────────────────────────────────────────────────────╮
   │request runs under the same 30-second timeout as every    │
   ╰──────────────────────────────────────────────────────────╯
   ╔══════════════╗
   ║   picture    ║   a picture: a 2px outline, 2px off its edge
   ╚══════════════╝
   ┌─────────────────────────────────┐
   │ Comment on this…    ( Comment ) │   the field: the strips stay
   └─────────────────────────────────┘
```

* **The look.** One strip per line of text, rounded 5px, as tall as the line
  less 4px, so two lines' strips stand 4px apart, and reaching 3px past the
  line's first and last selected letter. Inline code, a link or bold words
  in the line do not break its strip: the pieces of one line merge into one.
  Paragraphs, list items and blocks are separate strips, with the space
  between them unpainted, as the browser leaves it. The fill is blue,
  `rgb(59 130 246 / 22%)` multiplied over the light theme and `rgb(59 130
  246 / 42%)` screened over the dark, so the words under it keep their
  color. A picture, a video, a chart or a file's card gets a 2px outline,
  `rgb(59 130 246)` in the light theme and `rgb(96 165 250)` in the dark,
  2px off its edge and following its corners.
* **Where.** Everything in the conversation's list: replies, the user's
  messages, thinking and tool output. A text field, the composer and a code
  editor keep the browser's own selection, since their caret and selection
  are theirs. On a phone the list keeps the browser's selection: a phone's
  selection comes with its own handles, and a phone opens no comment
  field.
* **When.** The strips follow the selection as it is dragged, extended with
  the keyboard or set by a double click, and redraw when the list scrolls,
  resizes or streams more words. When the comment or Ask field opens, the
  strips stay on the words it holds until the field closes, in the same
  look. A click elsewhere clears them, as it clears a selection.
* **What stays the browser's.** Only the painting changes: the selection
  itself is still the browser's, so copying, the context menu, dragging the
  words out, Select All and the keyboard work as before.

The design to build from:

* `packages/web-ui/src/ui/drawn-selection.ts` owns the drawing:
  `selectionShapes(range, root)` measures the range's text boxes
  (`Range.getClientRects()`), merges the boxes of one line of one block,
  leaves out text the browser does not select (`user-select: none`), clips
  a line to the scrolled box it stands in, such as a wide code block, and
  returns strips and outlines relative to the root. One function draws the
  live selection and the held one, so the two cannot differ.
* `heldSelection`, in the same module, is the range the page keeps drawn
  while the browser's selection is gone: the comment field sets it to the
  words it has the focus for, or to the picture a menu opened it on.
* `DrawnSelection.vue` is the layer, mounted over the rows of
  `AgentMessageList.vue` with `pointer-events: none`. It draws the
  browser's selection while there is one in the list, and the held one
  otherwise, again on each selection change, scroll inside a row, resize
  and change of the rows. It marks the list `data-drawn-selection`, in
  which `base.css` makes `::selection` transparent, except in a text
  field.
* The `comment-selection` mark goes: its highlight kind, its CSS, and its
  picture outline, which the drawn outline replaces.

## Spike

* **Can the browser's selection stay painted while a field has the focus?**
  Moving the field into an `iframe` keeps the parent page's selection, but
  Chrome and WebKit then paint it grey, as an inactive selection. A grey
  selection still differs from the active one, and an `iframe` per field
  costs its own document, styles and focus handling. Ruled out.
* **Do measured boxes give whole lines?** `getClientRects()` returns one
  box per text node per line: a line with inline code returns three boxes
  that overlap where the code's padding meets the words, and two
  translucent fills darken the overlap. Merging the boxes of one line into
  one strip, by their vertical overlap, removes it.

## Prior art

* [CodeMirror 6](https://github.com/codemirror/dev) draws its selection
  with `drawSelection()`: a layer of absolutely placed boxes behind the
  text, the browser's `::selection` made transparent, so the selection
  looks the same when the editor loses the focus. Demi's editor already
  uses it (`cmTheme.ts`). This record does the same for the transcript.
* [VS Code](https://github.com/microsoft/vscode) draws the editor's
  selection as a layer of rounded boxes, one per line
  (`src/vs/editor/browser/viewParts/selections`), and keeps it drawn,
  dimmed, when the focus moves to the find field.
* Google Docs draws its own selection on its own canvas.
* Where this departs: CodeMirror and VS Code join the lines into one
  shape; here each line is its own strip, which reads better in prose,
  where lines differ in length a lot.

## Alternatives

* **The browser's selection and a `::highlight()` stand-in in the same
  color.** The height, the corners and the pictures still differ, so the
  jump stays, only smaller.
* **`::highlight()` for both.** A highlight paints the letters' height
  only, with no corners, and paints no picture.
* **The field in an `iframe`.** See the spike.
* **Not focusing the field until the user clicks it.** The user could not
  type right after picking Comment, as every comment field in mainstream
  software lets them.

## Open questions

None. The drawn selection does not dim while the field has the focus, as
VS Code's does: the user is writing about these words, and they are still
the selection the field acts on.
