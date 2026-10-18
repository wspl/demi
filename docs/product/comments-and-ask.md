# Comments and side questions

The user answers a reply by pointing at its words, as a reviewer comments on a
document. For example, the agent explains why uploads fail on slow
connections. The user selects "Upload in chunks of 4 MB" in the reply, clicks
**Comment** and writes "4 MB feels large on 2G. Can we start smaller?", then
selects "the same 30-second timeout" and writes "Should uploads get their
own?". Each selection turns yellow with a yellow underline, and the composer
shows **2 Comments** inside its box. The user types "Otherwise go ahead" and
sends: the agent receives one message holding each quoted text with its
comment, then the typed words, and the reply keeps its marks, dimmer.

## Where comments come from

Text selected in the agent's reply, its text blocks only, offers **Comment**
in the toolbar that stands by a selection, above it on a desktop and under
it on a phone ([Context menus](web-application.md#context-menus)). A reply
still being written offers it disabled, saying *Wait for this reply to
finish.*, as a mark set on text that is still changing would move. The
user's own messages, tool calls and the reply's code blocks' controls offer
none; text in a code block can be commented on.

**Comment** opens a small box under the selection, a field, *Add a comment*,
and a send button: Return saves, Shift-Return makes a new line, Escape
cancels. A saved comment marks its text with a yellow tint and a solid yellow
underline and puts a comment mark in the reply's right margin at that line.
Pointing at the text or the mark shows a card with the comment as text and
ghost **Edit** and **Delete** buttons at its right end; a click on the text
edits nothing, and Edit turns the card into the field with **Save** and
**Cancel**. On a phone, a tap on the text or the mark opens the card as a
sheet.

## Waiting comments

Saved comments wait in the composer until they are sent. While any wait, the
composer takes its multi-line layout and shows a pill inside its box, at the
top, its left edge on the text's: a comment mark, **2 Comments** and a ghost
×. The pill is round-ended and as tall as the composer's own buttons. A
click on it lists the waiting comments, each its quoted text and its
comment, with Edit and Delete; × removes them all.

Waiting comments belong to the conversation's draft, as its text and files
do, so another tab and the user's phone show the same marks and the same
pill moments after a change, and any of them can send them
([Conversation drafts](web-api-conversations.md#conversation-drafts)).

## Sending them

The composer's send button sends the typed text and the waiting comments as
one message, or the comments alone, the comments in reading order, the
transcript's order of their blocks and then their place in the block, as the
pill's list shows them. Save for Later saves the typed text and leaves the
comments waiting, since they point at this conversation's replies; a saved
message never holds comments. Recalling a sent or waiting message puts its
comments back with its text into an empty composer; when the composer holds
another draft, Recall leaves it as it is and the recalled message, comments
included, is what Undo sends again. Copy Message and Send to Conversation on a
sent message take its comments as the model reads them, quotes and comments
before the text. A message sent while the agent works carries its
comments as any message does, whether it steers the turn or waits for it
([Steer or queue](turns.md#steer-or-queue)): the model reads the quotes and
comments before the text either way. The message keeps the comments apart from
its text, so its bubble shows each quoted text as a quote block followed by
its comment, then the typed text, and the reply they point at shows their
marks dimmer from then on, read from the sent messages, not kept on the reply.
The model reads the same order as Markdown: each quoted text as a `>` quote,
its comment after it, then the text. Editing the message edits its text and
keeps its comments; Fork copies them with the message.

A comment points at its reply by the reply's text block and the character
offsets of the selection in the block's text as the page renders it, so a
mark survives the reply being drawn again, a theme change re-coloring its
code included. The comment also keeps the text it quotes, which is what the
model reads and what the bubble shows, so a block that no longer holds that
text, after an edit replaced the reply, shows no mark and the message still
reads whole.

A shared conversation shows the sent messages with their quotes and
comments, and no marks on the replies.
