---
status: proposed
---

# The comment field leaves the page's selection to the browser

Supersedes:

* 0054 (A selection opens a comment field), the field that takes the page's
  selection with the focus, and the page that draws the words it held.
* 0092 (A remark stands under its line), the press on the page that decides
  nothing until it is released, and the click on the selected words that
  leaves the field open.
* 0101 (The transcript draws its own selection), the strip reaching 3px past
  the line's first and last selected letter, and its 5px corners.

## Problem

A user selects "designed for a small" in a reply. The comment field opens
under the line and takes the focus. A document has one selection, so the
focus in the field ends the page's: from then on the page only draws the
words as selected, and decides by its own rules what each key and press
does to them. Those rules differ from the browser's, as measured in Chrome
against the same steps on a plain page:

| The user                         | The browser                    | Demi today                          |
| -------------------------------- | ------------------------------ | ----------------------------------- |
| Presses on other words           | Clears the selection at once   | Clears it on release                |
| Clicks the selected words        | Clears the selection on release | Keeps it, the field takes the focus |
| Shift-clicks other words         | Extends the selection there    | Nothing                             |
| Clicks a link or a button        | Keeps the selection            | Clears it, the field closes         |
| Presses Shift-Right Arrow        | Extends the selection a letter | Moves the field's caret             |
| Presses Command-A                | Its Select All on the page     | Selects the field's empty text      |
| Presses Escape                   | Keeps the selection            | Clears it, the field closes         |
| Right-clicks other words         | Its menu, for what it selects  | The message's menu, the old words still drawn |

The strip is also wider than the selection: it reaches 3px past the first
and last selected letter, and a space between two words of 15px text is
4px wide, so it tints the spaces around "designed for a small", or part of
a letter where the selection starts or ends inside a word.

## Decision

**The field stands in its own document, so the focus in it leaves the page's
selection where it is: every press, click and key on the page does what it
does without the field, and the field follows the selection.** The field
keeps what it is for: it opens under the selected lines, and a word typed
right after the selection, an input method's included, goes into it.

* **The page.** The selection stays the browser's while the field is open
  and has the focus. Pressing, dragging, clicking, Shift-clicking, double-
  and triple-clicking, right-clicking and dragging the words out work on it
  as they do with no field. The page draws it as before, only from the
  browser's selection now.
* **The field follows the selection.** A new selection opens a new field
  once the button that makes it comes up, as today. A selection extended or
  shrunk from the same start or end keeps the field and what it holds. A
  selection that is gone closes the field and drops what was typed.
* **The keys.** The field takes the focus as it opens, since an input
  method works only in a focused field. While the field is empty, the keys
  the page answers are the page's:
  * Shift with an arrow extends the selection, by a letter, a word with
    Option, or to the line's or the page's edge with Command.
  * Command-A does what the browser's Select All does on the page;
    Command-C copies the selected words.
  * Space, Shift-Space, Page Up, Page Down, Home, End and the up and down
    arrows scroll the conversation.
  * Tab and Shift-Tab move the focus from the selection, as they do with no
    field.
  * The app's shortcuts, such as Command-K, work.

  Once the field holds a word, the keys are the field's, as in any text
  field. Escape closes the field and leaves the selection: it does nothing
  to a selection with no field.
* **The strip.** It starts and ends at the selected letters' edges. At the
  end of a line that wraps inside the selection, it covers the space the
  line ends with, as the browser does. Its corners are rounded 3px instead
  of 5px, which stays inside the space beside a letter. It turns the
  system's inactive selection color while the window is not the active
  one, as the browser's does. Its height, the 4px gap between lines and its
  colors stay as 0101 has them.

What does not change: where the field stands and how it looks, its Comment,
Ask and Send, a picture's field, and the phone, which opens no field.

The design to build from:

* `FramedTextArea.vue` puts the field's text area in a same-origin `iframe`
  with no source; the rest of the field, its buttons and their tooltips,
  stays in the page. The frame mirrors the page's style sheets and the
  `<html>` attributes the theme reads. It is as wide as the window and shows
  only the text area's part, so the page's rules for a window's width, as a
  phone's, hold in it too.
* The host reads the page's selection at every `selectionchange`, during a
  press as after it: the press is kept only to open a new field once the
  button comes up. `restoreSelection()`, the field's refocus after a click,
  and `heldSelection` for words go; `heldSelection` stays for a picture,
  which the browser does not select.
* A `keydown` listener in the frame acts for the page while the field is
  empty and no composition runs: `Selection.modify()` for Shift with an
  arrow, `selectAllChildren()` for Command-A, the conversation's scroller
  for the scrolling keys, the next focusable element after the selection
  for Tab, and the browser's `selectAll` command for Command-A. Command-C
  runs the page's `copy` while the field selects nothing of its own.
* `drawn-selection.ts`: no padding past the letters, and the wrapped line's
  last space kept. `base.css` rounds `.drawn-selection-strip` 3px and gives
  it the inactive color under a window that is not focused.

## Spike

* **Does a focused field in a frame leave the page's selection the
  browser's?** A plain page with a selection and a focused `textarea` in a
  same-origin `iframe` with no source. Each step gave what the same step
  gives with no field:
  * The selection stayed, and the page's `getSelection()` still returned
    it.
  * A press on the selected words kept it until release, and the release
    cleared it. A press on other words cleared it at once.
  * A Shift-click extended it, and a double click selected a word.
  * A click on a button kept it.
  * Typing went into the field.
  * With the field empty, `Selection.modify()` from the frame's `keydown`
    extended it as Shift-Right Arrow and Shift-Option-Right Arrow do, and
    Command-A selected the page.
  * Command-C copied the selected words from the frame's `copy`.
* **What the automated browser cannot show** is checked by hand on the
  product: an input method's first key in the field, dragging the selected
  words out, and the inactive color.

## Prior art

* [VS Code](https://github.com/microsoft/vscode) takes the keys and an input
  method's composition in a hidden `textarea` (`TextAreaHandler`) and draws
  the selection on its own, so the element with the focus is not the one
  that shows the selection. Here, too, the focus and the selection belong
  to two documents.
* [CodeMirror 6](https://github.com/codemirror/dev) draws its own selection,
  and its `MouseSelection` (`@codemirror/view`) copies the browser's rules
  for a press: a press outside the selection selects at once, and a single
  press inside it waits, since it may start a drag, then collapses it on
  release. Here the browser applies its own rules, so none are copied.

## Alternatives

* **Put the selection back on the page at each press.** Every press is the
  browser's then, but the keys are still the field's: Shift with an arrow
  and Command-A act in the field, not on the page.
* **No focus in the field until a key is typed, then move it there.** The
  first key of an input method reaches the page before the field has the
  focus and arrives as a plain letter, so "nihao" typed in Pinyin becomes
  "n" and a composition of "ihao".
* **Copy the browser's rules into the page.** Each rule is a guess at the
  browser's, as 0092's were; this record removes the need for any.

## Open questions

None.
