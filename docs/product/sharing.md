# Sharing

A user shares a conversation, or one answer of it, as a read-only page that
anyone with its link can open, without an account. For example, the agent
finds why a deployment failed; the user opens the answer's **Share**, which
copies `https://demi.example.com/s/Qm9i…` at once, and pastes it to a
colleague, who reads the question and the answer and nothing else.

## What a share holds

A share is a **snapshot**: what it shows is copied when the link is made,
and nothing the conversation does later reaches it, as ChatGPT's and
Claude's shared links behave. A command that runs after the user shared the
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
answer's footer holds **Share** to the right of Fork. Either creates the
share at once and opens the share dialog with its link copied, so no step
asks before the link exists; a share that exists already opens with its
link. The dialog says who can read it, *Anyone with the link can read this
conversation, including commands and their output. Later messages are not
included.*, or for an answer *Anyone with the link can read this answer and
the message it replies to.*, and holds:

- **Copy Link**.
- **Update Link**, for a conversation's share: copies the conversation as
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
- **The share reaches nothing live.** The snapshot is a copy kept with the
  conversation, and the public routes read only it: no conversation, Host,
  file or account route answers a share's token, and a blob is served only
  when the snapshot shows it.
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
The snapshot is kept in the conversation's database: its blocks, each
subagent's included, copied from the transcript as they were, with the
command output each call's view holds and no more, since a later edit or compaction rewrites the
transcript ([Message editing](../agent/message-editing.md)). A conversation's
snapshot copies every block except `context` blocks, the hidden inputs that
carry the execution context, instructions and skills, and `response` blocks,
the usage of a request; the shared page hides the rest as the conversation
does, with the same rule, so the backend keeps no second list of what a page
shows. The page's compaction dividers read their summary sizes from the
snapshot's own blocks. Deleting the
conversation removes both in its first step
([Deleting a conversation](../backend/storage.md#deleting-a-conversation)), and
the blobs a snapshot shows stay referenced while it does
([Retention](../backend/storage.md#retention)).

The shared page reads its snapshot a page at a time through the routes of
[Sharing](web-api.md#sharing), with the same pages, light form and
whole-block reads as a conversation's history, so a long conversation opens
as fast as in the product.
