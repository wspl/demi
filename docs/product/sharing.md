# Sharing

A user shares a conversation, or one answer of it, as a read-only page that
anyone with its link can open, without an account. For example, the agent
finds why a deployment failed; the user opens the answer's **Share**, which
copies `https://demi.example.com/s/Qm9i…` at once, and pastes it to a
colleague, who reads the question and the answer and nothing else.

## What a share holds

A share shows the conversation as it stood when the link was made, up to
that moment, and nothing the conversation does later reaches
it, as ChatGPT's and Claude's shared links behave. A command that runs after the user shared the
conversation, and its output, never become public without the user sharing
again.

| | An answer | The conversation |
| --- | --- | --- |
| Holds | The user's message and the text that ends the agent's reply to it, the text that carries Copy and Fork ([Following the agent's work](product.md#following-the-agents-work)), with the attachments and media they show | Every block the transcript shows, as the conversation shows it: messages, thinking, tool calls with their scripts and outputs, permission receipts, compaction dividers, and each subagent's transcript, opened from its link |
| Title | The conversation's title when shared | The same |

A share never holds what the user's own conversation does not show either,
such as the context blocks, the instructions' text or the system prompt, and
nothing of a Host: the working tree, the files, the browser tabs or a
command still running. A link in the shared text that opens something of a
Host, a file path or a `commit:` link, is plain text there; an `agent:` link
opens that subagent's transcript in a conversation's share and is plain text
in an answer's. The page names no sharer.

## Making, updating and removing a share

The conversation header's **⋯** menu holds **Share Conversation…**; an
answer's footer holds **Share** to the right of Fork. Either opens the
share dialog at once and creates the share from it, so no step asks before
the link exists: while the link is made, the dialog shows *Creating
link…* with a spinner where the link will stand, and Copy Link waits
disabled; once it is made, the link stands there, already copied, as
ChatGPT's share dialog fills in its link. A failure says so in the dialog,
with Try Again. Update Link shows the same progress in place of the link.
A share that exists already opens with its link. Making a link copies
nothing but the few blocks still changing ([Storage and routes](#storage-and-routes)),
so even a long conversation's link takes a moment. The dialog says who can read it, *Anyone with the link can read this
conversation, including commands and their output. Later messages are not
included.*, or for an answer *Anyone with the link can read this answer and
the message it replies to.*, and holds:

- **Copy Link**.
- **Update Link**, for a conversation's share: shares the conversation as
  it is now under the same link.
- **Stop Sharing**: removes the share; its link answers *This shared link
  does not exist* from then on. Sharing again makes a new link.

A conversation has at most one share of its own, and an answer at most one.
Deleting the conversation removes its shares; archiving keeps them; a Fork
copies none ([Conversation Fork](../agent/conversation-fork.md)). There is no
list of a user's shares yet: a share is found from what it shares.

## Safety

- **The link is the only key.** Its token is 128 random bits, never listed or
  derived from anything, so it cannot be guessed; nothing else reaches a
  share, and removing it ends access at once.
- **The share reaches nothing live.** The public routes read only what the
  share's list holds: no conversation, Host,
  file or account route answers a share's token, and a blob is served only
  when the share shows it.
- **What it holds is a list.** Each share route reads through the share's
  list and nothing else: a block, a page, an agent, a blob or an
  attachment is served only when the list holds it, so what is not on the
  list, a hidden block, a block written after the link was made, another
  version of an edited message, a subagent started later or a command's
  later output, is out of reach by construction, not by a rule each route
  repeats. Anything not on the list, asked by index, block id, page
  anchor, agent number or hash, answers the same 404 as a token that names
  no share, so a reader cannot even tell that it exists. Tests ask the
  routes for each of these and expect that 404.
- **The page carries no power.** The shared page is its own small entry of
  the web app, the transcript's read-only components and nothing that calls
  the user's routes, under a Content Security Policy that lets it load its
  own scripts and connect only to its share's routes. The product's session
  cookie, which the browser still sends on the same site, is ignored by the
  share routes.
- **The token stays on the page.** `Referrer-Policy: no-referrer`, and
  external links open with `rel="noopener noreferrer nofollow"`, so a site
  the reader follows never learns the link.
- **Not indexed.** The page and its routes answer with
  `X-Robots-Tag: noindex, nofollow`.

Secrets a command printed are the user's to judge, which the dialog's words
say; `demi credentials` keeps the secrets it handles out of the transcript
([Credentials](web-api.md#credentials)).

## Storage and routes

The control database holds each share's record: its token, its user, its
conversation, its kind, `conversation` or `answer` with the answer's block,
and when it was taken ([Control records](../backend/storage.md#control-records)).
What it shows stays in the conversation's database, where the transcript
already holds it, so a share copies almost nothing:

- **A list of what it holds.** A share is a list, made when the link is
  made: each block it shows, in the order it shows them, numbered from 0,
  by the transcript row it reads, its agent's and its own position, and
  each attachment those blocks show, found through the conversation's index
  of the attachments each block names; a blob is the share's when a block on
  its list names it, found through the conversation's index of the blobs
  each block names ([Storage](../backend/storage.md)), never by reading the
  blocks. The list holds positions, not
  content: for a conversation of 86,000 blocks and 1.3 GB, a list of about
  2 MB, made from the transcript's index without reading a block's text.
  A block it lists does not change afterwards: the transcript only
  appends, and a compaction keeps every block ([Compaction](../agent/compaction.md)).
- **What still changes is copied.** A block that is still changing when the
  link is made, a call not yet returned or a command whose output still
  grows, is copied into the share as it stands, with the output its view
  holds then, and its entry in the list points at the copy, so a command's
  later output never becomes public. That is a handful of blocks at most.
- **What is rewritten is copied first.** Any write that replaces or
  removes a row a share lists, an edit or a retry that replaces a turn, a
  command's view that a later call rewrites, first copies the row as the
  share shows it into the share, in the same transaction, and points the
  entry at the copy, so a share never shows a row's later content and never
  loses one.
- **Update Link** makes the list anew and drops the old one with its
  copies.

The list is the whole of what a share can show: every share route reads
through it and nothing else ([Safety](#safety)).

A conversation's share shows every block except `context` blocks, the hidden inputs that
carry the execution context, instructions and skills, and `response` blocks,
the usage of a request; the shared page hides the rest as the conversation
does, with the same rule, so the backend keeps no second list of what a page
shows. The page's compaction dividers read their summary sizes from the
blocks the share holds. Deleting the
conversation removes both in its first step
([Deleting a conversation](../backend/storage.md#deleting-a-conversation)), and
the blobs a share shows stay referenced while it does
([Retention](../backend/storage.md#retention)).

The shared page reads what its share holds a page at a time through the routes of
[Sharing](web-api.md#sharing), with the same pages, light form and
whole-block reads as a conversation's history, so a long conversation opens
as fast as in the product.
