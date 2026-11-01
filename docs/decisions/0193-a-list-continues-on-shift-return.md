---
status: accepted
---

# A list continues on Shift-Return

## Problem

Alex writes three steps in the composer:

```text
1. run the tests
2. fix what fails
3. open a pull request
```

After the first line, Alex presses Shift-Return and types `2. ` by hand,
then again `3. `. Inserting a step between the first two means typing its
number and renumbering every step after it. A quote and a task list need
their marks typed on every line the same way.

The composer highlights a message's Markdown as a code editor does
(0183), but it continues none of it: Shift-Return starts an empty line.

## Decision

**Shift-Return on a list item or a quote starts the next line with the
same marks, as GitHub's comment box, Zed and Obsidian do on their Return.
Return still sends.** Every message editor does it: the composer, an edit
of a sent message, a comment and a message saved for later.

```text
  1. run the tests|        Shift-Return    1. run the tests
                           ───────────▶    2. |

  - [x] write the docs|                    - [x] write the docs
                                           - [ ] |

  > the old helper|                        > the old helper
                                           > |
```

What a line starts with is read as the highlighting reads it (`lineForm`),
quote marks first, then a list marker:

- A bullet item continues with its own bullet, `-`, `*` or `+`, its
  indentation and its spacing; an ordered item with the next number and
  its own delimiter, `.` or `)`; a task item with an unchecked box; a quote
  with its `>` marks, a list in a quote with both.
- With the caret in the item's text, the text after the caret goes to the
  new item. With the caret in the marks, or a selection, Shift-Return
  starts a line as it does today.
- An ordered item inserted before others renumbers the items that follow
  it at its level while their numbers ran on by one, and stops at the
  first that does not, so a list numbered on purpose keeps its numbers.
- Shift-Return on an item with no text ends the list: its marker goes and
  the line stays, empty. A nested item moves out a level instead, taking
  the next marker of the item it was under. An empty quote line loses its
  last `>`.
- Backspace right after a list marker turns the marker into spaces up to
  the text of the item above, so the line continues that item as a second
  line of its text, or removes the marker on a list's first item; Backspace
  again takes the line back to that item's marker column; after that it
  deletes a character as usual. Right after a quote's `>`, it removes that
  `>`.
- Tab on a list item nests it under the item above, its marker under that
  item's text, as 0183 lays a nested item out; Shift-Tab moves it out a
  level. Its own lines and the items nested under it move with it, and the
  numbers on both levels follow. Tab anywhere else moves the focus, as now.
- Each of these is a step of its own for Undo, never joined to the edit
  before it. A heading, a rule, plain text and the code between fences
  continue nothing, and renumbering passes over code. Nothing typed is
  converted, and a send changes nothing: the marks are characters as any
  other.

A phone's return key sends, so a phone writes no list; it is left as is.

### Design

- `web-ui` `message-editor/list-keys.ts`: pure functions of a message's
  lines (`lineTexts`), the caret's line and offset: `listNewline`,
  `listBackspace` and `listIndent` answer the edits to make, a line to
  split, and where the caret lands, or `null` when the key does what it
  did before. They read the marks with one parser of a line's prefix, quote
  marks, indentation, marker and task box, which agrees with `lineForm`.
- `extensions.ts` `MessageKeys`: Shift-Return, Backspace, Tab and
  Shift-Tab apply that answer as one transaction, outside code only.
- Nothing stored changes.

## Prior art

- **CodeMirror**'s Markdown mode (`insertNewlineContinueMarkup`,
  `deleteMarkupBackward` in `@codemirror/lang-markdown`): this design's
  rules for continuing, renumbering only while the numbers ran on, ending
  on an empty item, moving a nested item out a level, and Backspace
  turning a marker into spaces come from it. It reads a syntax tree; the
  message editor reads lines, as its highlighting does, so the rules are
  written again over lines rather than shared.
- **GitHub**'s comment box continues bullet, ordered and task lists on
  Return, which starts a line there
  ([changelog](https://github.blog/changelog/2020-12-15-markdown-list-syntax-now-autocompleted/)),
  and an empty item ends the list.
- **Zed** continues lists on Return and indents them on Tab
  (`extend_list_on_newline`, `indent_list_on_tab`,
  [zed-industries/zed#42800](https://github.com/zed-industries/zed/pull/42800)).
- **Obsidian** continues lists and quotes, ends them on an empty item, and
  indents on Tab.

Demi's Return sends, so the key that starts a line, Shift-Return, is the
one that continues.

## Alternatives

- **Return continues inside a list, as it starts a line between fences,
  and Command-Return sends.** A message that starts with `1. ` would not
  send on Return, and ending a list would take Return twice before the
  send.
- **A second key for a plain line inside a list**, as GitHub's
  Shift-Return. Demi has no key left beside Return and Shift-Return;
  Backspace after the marker gives the plain line.
- **Leave numbers as typed.** Inserting an item would leave two items with
  the same number for the user to fix.

## Open questions

None.
