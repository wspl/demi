---
status: accepted
---

# A message is shown as it was typed, its Markdown highlighted

## Problem

Alex types `the grid is 2*3*4 cells, keep ~~old~~ as is` in the composer.
As the line is typed, the asterisks and tildes disappear: the composer shows
"the grid is 234 cells", with the 3 in italics, and "old" struck through.
Typing `` `npm test` `` shows `npm test` in a code box without its
backticks, and `[docs](https://example.com)` shows only "docs", the address
hidden. To keep the characters, Alex has to undo each formatting or escape
the characters, and to see the address again, open the link.

The composer formats a dialect of Markdown as it is typed: bold, italics,
strikethrough, inline code, code blocks and links, each from the characters
that open and close it. What is sent is then the composer's rewrite of what
is on screen, not the typed text, and nothing on screen shows the
characters the model reads.

## Decision

**A user message is plain text with its Markdown highlighted, as a code
editor highlights a language, the same in the composer and in the bubble:
every character stays as typed, the marks are dimmed, and the text takes
the look its Markdown asks for, a heading bold, code in monospace, bold in
bold, all at the text's size.** The only thing drawn as something else is a file's capsule. What is
typed is the message's Markdown, character for character, and what the
bubble shows is what was typed.

```text
  ## Steps                                  ← bold, "##" dimmed
  use `npm test` and **not** yarn, see [docs](https://example.com)
      ╰ monospace╯      ╰bold╯        ╰link╯╰ dimmed address ╯
  - run the tests                           ← "-" dimmed, not indented
  > keep the helper as is                   ← ">" dimmed, text muted
  ```ts                                     ← dimmed, monospace
  const answer = 42                         ← monospace, code's colors
  ```
```

- Bold, italics, strikethrough, inline code and links are highlighted as
  they are typed, their marks dimmed. Inline code is in monospace and code's
  color, a link's text in the link color with its address dimmed.
- A heading is bold at the text's size, with room above it and a little
  below. A quote's text is muted. A line's marker, `#`, `-`, `1.` or `>`
  with the spaces before it, is dimmed and in monospace, so a nested list
  item's marker stands under its parent's text, as in a code editor. A
  list item is one at any indentation. A fence's lines are dimmed, and the code between
  fences is in monospace and its language's colors.
- Every line is a line of text: nothing is put in a box, on a well or
  indented by the page, so the caret moves through a message as through
  plain text. A list is indented only by the spaces typed.
- Nothing typed is converted or escaped, when typed or when sent. A send
  trims the blank space at the message's ends, and drops text of only
  spaces between two files, as now, since a model takes no empty text.
  Selecting, undoing and an input method work on the characters.
- A capsule stands where each file is, as now. Markdown's image syntax,
  `![alt](address)`, is text like the rest, highlighted as a link: a picture
  is a file.
- A paste takes text as text, line by line: a web page's or a document's
  formatting is left out, and its pictures and files come in as capsules, as
  now. A copy of a Demi message pastes its text and its capsules. Code
  between fences takes a paste as it is, blank lines and all.
- A copy out of a message, sent or being written, is what was selected, as
  a text field copies it.
- The keys do not change: Return sends and Shift-Return starts a line,
  except on a fence or between fences, where Return starts a line and
  Command-Return sends.
- The bubble shows a sent message exactly as its composer showed it. The
  same holds for every editor of a message, Edit on a sent message and a
  message saved for later, and for the comment fields once 0182 makes them
  message editors.

A message written before keeps its stored Markdown, and shows it: where the
composer escaped a character, such as `\*`, its backslash shows.

### Design

- `web-ui` `message-editor/highlights.ts`: one function reads a message's
  document and gives each line its attributes and each range of it the
  element and class it is drawn with. It reads each line with the dialect's
  lexer (`markdown/user-markdown.ts` § `markdownSpans`), a line's form by
  CommonMark's line markers on that line alone (`lineForm`), and the fences
  as CommonMark does (`lineRoles`).
- The message's schema (`extensions.ts`): a paragraph for each line, text
  and `AttachmentNode`. The editors hold it and draw the
  highlighting as ProseMirror decorations; `SentMessage` draws it from the
  same document as elements, without an editor. Bold, italics,
  strikethrough, code, code blocks, links, line breaks and pictures leave
  the schema, and with them `MessageFormat`, which formatted a construct as
  it was typed, the dialect's reading into marks and its writing back with
  escapes. A pasted line break, which the schema has no node for, reaches it
  as a line separator and becomes a line.
- `markdown/user-markdown.ts` reads Markdown into the document and writes it
  back character for character.
- A copy from a message, sent or in an editor, is its text as typed, in
  Demi's own form with its files (`message-input/copied-message.ts`).
- Nothing stored changes.

## Spike

Whether tiptap can style Markdown without changing the text was not known.
A test page made a tiptap editor whose document is lines of plain text,
with a ProseMirror decoration plugin that wraps bold, italics,
strikethrough, inline code and links in styled spans and dims their marks,
and a node decoration on the lines between fences. Typing
``then run `make check`, it must be **green** before ~~friday~~ *today*``
styled each construct as its closing mark was typed, with every character
kept, and the text the editor held was the typed line.

Whether drawing every bubble from the highlighting costs more than drawing
it formatted was not known either. A test page mounted 300 sent messages,
a third of them with a heading, a list, a quote and a fenced block, twelve
times in Chrome: the median was 28 ms with the highlighting and 23 ms with
the formatted drawing before it, about 0.02 ms more for each message, which
the transcript draws only for the messages in view.

## Prior art

- **ChatGPT** and **Gemini** take the prompt as plain text, files as tiles.
- **Cursor** takes plain text with a pill for each file or symbol it names.
- **Bear**, **iA Writer** and **Obsidian**'s source mode keep the Markdown
  marks visible, dimmed, and style the text between them. **Typora** hides
  the marks until the caret is in them.
- **Discord**'s message box keeps the marks and styles the text between
  them, in a chat app.
- **GitHub** writes a comment's Markdown as text and renders it once it is
  posted.
- **Claude**'s desktop app formats Markdown as it is typed; its users asked
  for a plain text input instead
  ([anthropics/claude-code#57268](https://github.com/anthropics/claude-code/issues/57268)).

## Alternatives

- **Keep formatting as it is typed and fix each surprise.** Every rule that
  turns characters into formatting can catch characters meant as text; the
  fixes are more rules, and the characters stay hidden.
- **Plain text without highlighting.** The characters would be right, but
  nothing would show what the bubble will make of them.
- **Draw code and lists as blocks**, code on a well and a list indented by
  the page, as a reply does. A box the caret moves around, or an indent that
  is not in the text, makes a message stop behaving as plain text.
- **Hide the marks until the caret is in them**, as Typora does. The text
  would move as the caret passes, and the characters the model reads would
  stay hidden.
- **Keep the bubble drawing Markdown formatted.** The composer and the
  bubble would show the same message two ways, from two renderers to keep
  alike.
- **A formatting toolbar**, as Slack has. A message to an agent is mostly
  instructions, paths and code, which formatting does not help.

## Open questions

None.
