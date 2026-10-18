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
and two buttons, as GitHub's review offers *Add single comment* beside
*Start a review*: **Comment**, the default, which adds the comment to the
waiting ones, and **Send Now**, which sends a message holding this comment
alone at once. Return comments, Command-Return (Control-Return elsewhere)
sends now, Shift-Return makes a new line, Escape cancels. Send Now leaves
the other waiting comments and the typed draft as they are, and while the
agent works it steers or waits as the composer's send would
([Steer or queue](turns.md#steer-or-queue)). A saved comment marks its text with a yellow tint and a solid yellow
underline and puts a comment mark in the reply's right margin at that line.
Pointing at the text or the mark shows a card with the comment as text and
ghost **Edit** and **Delete** buttons at its right end; a click on the text
edits nothing, and Edit turns the card into the field with **Save** and
**Cancel**. On a phone, a tap on the text or the mark opens the card as a
sheet.

### Images

A picture in the agent's reply, an image it showed from the Host, an
attachment or a picture its Markdown drew, takes comments and side questions
as its words do. Its menu, on a right-click or a phone's long press, holds
*Copy Image* and *Save Image*, then **Comment**, **Ask** and **Send to
Conversation** ([Context menus](web-application.md#context-menus)). Comment
opens the same box under the picture; a saved comment outlines the picture in
yellow and puts its mark in the margin at the picture's top. The comment
points at the picture by its block and its place among the block's pictures,
and keeps the picture itself as what it quotes, its alt text, file name or
the end of its address as the quote's words: an attachment's picture by the
attachment it already is, a Host image's bytes as they were when commented
on, since the file may change, and a picture from the web as its bytes when
its site lets the page read them, otherwise by its address, so the bubble shows a small
copy of it as its quote, and the model reads the picture, as an image the
message carries, and then the comment. Ask on a picture starts a side
question whose first message carries the picture as its quote. Send to
Conversation adds the picture to the other conversation's draft as an
attachment. A video or a sound takes none of these yet.

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

## Side questions

The user asks about a reply's words without interrupting the conversation,
as Claude Code's `/btw` and Codex's `/side` ask beside a session. For
example, the reply says "All API calls share one client, created in
`client.ts` with a 30-second timeout". The user selects those words, clicks
**Ask** and writes "Is that really so?". The words turn purple with a purple
underline, and a card opens under the block holding them: its header reads
*Ask* until a small model titles it, *Shared API client timeout*, then the
question in bold, one status line while the answer is made, *Reading
client.ts…*, and then the short answer. The conversation's own agent goes on
with its turn if it has one, and learns of the question only with the
user's next message, as something the user wondered, never as a request
([What the agent learns](#what-the-agent-learns)).

**Where it starts.** The selection's toolbar offers **Ask** after
**Comment**, on the same text and disabled in the same cases
([Where comments come from](#where-comments-come-from)). It opens a small box
under the selection, *Ask about this*, with a send button: Return sends,
Shift-Return makes a new line, Escape cancels.

**What the side conversation knows.** It starts from the conversation's
history up to the end of the reply asked about, cut as a
[Fork](../agent/conversation-fork.md#the-fork-seed) cuts it, the reply's
response included, so the request reads as the conversation's next one
would have at that point. When the conversation was compacted after that
reply, the reply lives only in the summary, and the side conversation starts
from the conversation's context as it stands, the summary and everything
after it, as its next request would send it; the quoted words say what is
asked about. It runs on the
conversation's model, system prompt and tools, so its first request reads
that history from the provider's prompt cache instead of paying for it again
([Prompt cache](../providers/providers.md#prompt-cache)): it also sends what
the provider keys its cache and its routing by as the conversation does, its
session, prompt cache key and conversation headers, so a provider that
routes by them sends the side question to the same cache. Its first user
message, after that history, holds the quoted words, then what makes it a side
question: the user asks this aside from the conversation; answer briefly, in a
few sentences; read and check what is needed but change nothing, no file
written and no command that changes the Host. This narrows it by instruction,
as a profile narrows a child
([Child context](../agent/subagents.md#child-context)); the Host does not
enforce it. Then comes the question. A follow-up is the side conversation's
next turn, after its own history, and keeps its model when the conversation's
changes. It runs on the conversation's Host, through the conversation's host
access, beside the conversation's turn and never waiting for it; its commands
are its own, it starts no subagent and sends no message, `demi agent`
answering that a side question cannot, and it appears in no agent list and
takes no agent number.

**What the agent learns.** A question the user had to ask aside says what
they were unsure of, which the agent should know without taking it as a
task. So when the user next sends a message, or steers, the conversation's
agent receives, before it, the side questions asked since it last heard of
them, as a `context` block of the side-questions source
([Context](../agent/transcript.md#context)):

```text
The user asked these aside and was answered there; they show what the user was unsure of, for you to know, with no need to answer them again.

On "All API calls share one client":
Q: Is that really so?
A: Yes. Every call goes through the one client in client.ts, with its 30-second timeout.
Q: And for uploads?
A: Uploads use it too, which is why a slow upload is cut off at 30 seconds.
```

One sentence says what these are; then each side question under its quoted
words, each question the user asked in it, the first and every follow-up, in
the order asked, with its final answer, cut by the replay bound when long
([Text bounds](../agent/compaction.md#text-bounds)). A side question the
agent heard of comes again only with a new follow-up, and then only the new
questions and answers. Nothing reaches the agent before
the user's next message, so a turn under way is never turned by it.

**What the card shows.** Each question and its final answer, never the steps
between them; while a turn works, the one status line names what it does.
The header holds a purple question mark, the title in secondary text, and two
ghost icon buttons at its right end: ↗, *Open Full Conversation*, and ×,
*Collapse*. Below the last answer a bordered **Reply** opens a field with a
send button for a follow-up; while the field is open the card's other
controls still do what they say, and an empty field closes when a click lands
outside the card. ↗ opens the side conversation, its steps included, in
the window a subagent's opens in; it starts with the user's first question,
drawn as the bubble of a message with its quote, since the history before
it is the conversation's own and the side question's instruction is the
product's, not the user's,
([Product rendering](../agent/subagents.md#product-rendering)). × folds the
card into a purple question mark in the reply's right margin at the asked
line, where a comment's mark stands; a click on the mark or on the purple
words opens the card again, scrolled into view. Hovering the purple words
deepens them, as hovering a comment's does. A failed answer shows the
failure's words and ID in the card, as a turn's failure does, with **Retry**.

**Where it lives.** A side question belongs to its conversation: the backend
keeps it, with its reply's block, offsets and quoted words as a comment keeps
them ([Sending them](#sending-them)) and whether its card is folded; every page
that shows the conversation shows it, and a page opened later reads it. It
belongs to the reply's block: an edit or a regenerated reply that leaves the
block on another branch hides its side questions with it, and switching back
to that branch shows them again; they go only when the block itself is
deleted. A Fork copies
none, and shared pages show none. The usage ledger counts its turns under the
conversation, as a subagent's are counted.

**On a phone** the toolbar holds Comment, Ask and Send to Conversation; the
card spans the reply's width, and ↗ opens the side conversation full screen.
