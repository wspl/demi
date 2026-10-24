---
status: proposed
---

# An edit of a sent message shows and changes its comments

## Problem

A user selects "the same 30-second timeout" in a reply and comments "Should
uploads get their own, longer timeout?", selects a second passage and
comments on it too, and sends the two comments with no text of their own.
The message shows both quotes and their comments. The user then picks Edit
on it to reword the first comment.

The editor opens empty. An edit holds the message's content, and the
comments are not content: a `user` block keeps them beside it, in
`comments`. Nothing in the editor shows them, so the user cannot tell what
the message holds, cannot change a comment or drop one, and cannot send
the edit until they type something, since an edit needs content. When they
do, the new version silently keeps both comments as they were sent.

## Decision

**The editor shows the edited message's comments as the composer showed
them before it was sent.** The *2 Comments* pill stands by the paperclip
of the edit's composer, as it stands in the message field for the comments
waiting in a draft (0092). A click on it lists the comments, each its
quoted words and its comment, with Edit and Delete; its × removes them
all.

```text
  ╭──────────────────────────────────────────────────────────────────╮
  │ Editing message                                                  │
  │ Ask anything…                                                    │
  │ ( 📎 ) [ 💬 2 Comments  × ]                     Cancel  ( ↑ )   │
  ╰──────────────────────────────────────────────────────────────────╯
```

* **What the edit sends.** Its text and files, and the comments the pill
  lists when it is sent: a comment edited there goes with its new words, and
  one deleted there is not in the new version. The new version holds those
  comments, never the first version's.
* **Without text.** An edit that keeps a comment may have no text, as a
  message sent with comments alone may.
* **Nothing left.** An edit with no text, no file and no comment cannot be
  sent, as an empty message cannot.
* **While it is sent.** Once the edit is sent, and while its outcome is
  uncertain, the pill still lists the comments but offers no Edit, Delete
  or ×, as the editor takes no typing then. The editor a lost Regenerate
  opens shows the message's comments too.
* **The reply.** The comments the message sent stay under their lines,
  marked Sent, as they were sent, while the edit is open. Once the edit is
  sent, they are the new version's.
* **The draft.** The comments waiting in the draft stay the draft's. The
  edit's composer shows the edit's own; the draft's come back when the edit
  ends.
* **Regenerate.** It keeps the message as it is, its comments included.

The design to build from:

* `EditRequest` carries the comments, as `SendMessage` does. Its content may
  be empty when it carries a comment. Without `comments`, the new version
  keeps the edited message's, as every edit did before:

  ```rust
  pub struct EditRequest {
      pub operation_id: OperationId,
      pub target_block_id: BlockId,
      pub version: Version,
      /// The complete replacement content; empty only beside a comment.
      pub content: Vec<ClientContent>,
      /// The new version's comments, which replace the edited message's;
      /// none keeps the edited message's.
      #[serde(default, skip_serializing_if = "Option::is_none")]
      pub comments: Option<Vec<MessageComment>>,
  }
  ```

  `EditSubmission` takes `comments` next to `content`, and the session gives
  the new `user` block those comments, or the target's when it has none.
  The blank line the editor opens a message without text with stays in the
  request, so a retried edit's digest is the one first sent; the session
  drops blank text from the new version.
* The page's `SentMessageEdit` request carries `comments`, read from the
  block when the editor opens, and the edit's content counts them as
  something to send. An edit the page kept from before this change, and an
  older page, send none, and their new version keeps the comments as before.
* `SessionComposer` shows its `carried` slot during an edit too.
  `ConversationComposer` fills it with the shown edit's comments, through
  the same `WaitingComments`, whose Edit, Delete and × change the edit;
  otherwise with the draft's.
* A message saved for later holds no comments, so the edit of one shows
  none.

## Spike

None: the frames, the session's edit and the composer's slot exist; this
fills them.

## Prior art

* ChatGPT keeps a passage quoted from a reply by the message field as a
  removable chip until the message is sent.
* GitHub's review comments are edited in place with their quote; an edited
  comment replaces the first.

Demi shows a message's comments the same way before it is sent and while
it is edited, so the two never differ.

## Alternatives

* Write the comments into the editor's text as `>` quotes, as Copy writes
  them: the new version would hold them as text, no longer as comments, and
  a comment on a picture the reply drew from a URL could not be kept, since
  an edit keeps only stored media.
* Keep the comments unseen and unchanged, as today: the user edits a
  message without seeing what it says.
