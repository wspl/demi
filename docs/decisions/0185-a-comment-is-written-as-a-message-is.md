---
status: proposed
---

# A comment is written as a message is

0183 (A message is shown as it was typed) names this record 0182, the
number it was drafted under.

## Problem

Alex selects "the retry waits 30 seconds" in a reply, and in the comment
field under it writes "this is what I see", then presses Command-V to paste
the screenshot of the log. Nothing happens: the field is a plain text area.
The same paste in the composer adds the screenshot as an image where the
caret stands. To show the screenshot, Alex has to leave the comment and
write a message instead, which loses the link between the words and the
remark.

Each place a remark is written today has its own field, and each knows less
than the composer:

| Where                        | Field today            | Composer can, it cannot                         |
| ---------------------------- | ---------------------- | ----------------------------------------------- |
| Comment, Ask, Send under a selection | `FramedTextArea` | Paste or drop files and images, Add Files, Markdown highlighting, file references, a copied message |
| Edit of a waiting comment    | `TextArea`             | The same                                        |
| A side question's follow-up  | `SendField`            | The same                                        |

A comment also keeps words only: `MessageComment.text` is a string, and the
bubble and the reply draw it as plain text.

## Decision

**Every field that writes to the agent is the composer's editor, and what
it writes is a message's content.** A comment, an Ask and a follow-up take
what the composer takes, the same way, and keep it as a message keeps it.

```text
  …the retry waits 30 seconds after the first failure…
              ┌──────────────────────────────────────────────┐
              │ this is what I see [▣ log.png]          (?) (↑) │
              └──────────────────────────────────────────────┘
```

- **The field.** The selection field holds the composer's editor. Typing,
  with the text kept as typed and its Markdown highlighted (0183), pasting
  and dropping files, pictures, long text and a copied message, and the
  capsules where files stand, all behave as in the composer. It has no Add
  Files button: the field is one light line under the selection, and a file
  comes in as it is pasted or dropped. Its buttons, its keys and where it
  stands do not change: Return presses the blue button, Shift-Return starts
  a line, Escape closes it, and while it holds nothing the keys the page answers still act on the page's
  selection (0115). A file counts as something written: a comment of a
  screenshot alone can be added.
- **Edit of a waiting comment** opens the same editor with the comment's
  content, with Save and Cancel as now.
- **A side question's follow-up** is written in the same editor, and the
  question the selection field asks carries what it holds.
- **What a comment shows.** In the waiting list, in its box under the
  reply's line and in the sent message's bubble, its content is drawn as a
  sent message's is (0183): its text as typed and highlighted, and a
  capsule where each file stands.
- **What the model reads.** The quote, then the comment's content in its
  order, its files as the message's own files are given: an image or a
  document the model reads natively, and an attachment by its tag.
- A file in a comment uploads as one in the composer does, while the
  comment waits in the draft; a comment whose file still uploads sends once
  it is there, as the message does.

### Design

The draft is one message and its comments, and a comment is a place on a
reply plus a message. At every stage a comment's content has the type the
main message has at that stage, so each step that handles the message
handles a comment's the same way:

| Stage                      | The message                    | A comment's content |
| -------------------------- | ------------------------------ | ------------------- |
| The page's draft           | `{ text, fileIds }`            | The same            |
| A draft save               | `{ text, files: ClientContent }` | The same          |
| A draft read               | `{ text, files: DraftFile }`   | The same            |
| A frame (send, steer, Ask) | `ClientContent[]`              | The same            |
| The transcript and queue   | `UserContentBlock[]`           | The same            |

- `web-ui`: `FramedTextArea.vue` becomes `FramedField.vue`, which hands its
  slot the frame's own document, and `MessageField.vue` makes a
  `MessageEditor` in that document. ProseMirror listens to the document it
  is made in, so it is made there, never moved there. The frame's key
  forwarding to the page, its sizing and its style mirroring stay.
  `SelectionField`, `CommentBox` and the follow-up's `MessageSendField` use
  `MessageField`; `SendField` and the text areas are no longer used for
  writing to the agent.
- Files. A comment's files are among the conversation's, as the composer's
  are: one pool, each file in one message. They upload, show their transfer
  and paste back through the same steps. An open comment field holds its
  files while it is open, and a file no message holds any longer goes.
- Send waits for the files of the message and of every comment, and refuses
  while one of them failed or is gone, as it does for the message's own.
  A comment sent now goes alone through the same delivery.
- A message taken back, by Recall or Edit of one not delivered, gives each
  comment back to the draft through the same paste that gives back the
  message, its files without their bytes.
- An edit of a sent message lists the comments the message carried; they
  can be deleted there, not edited.
- Types: `Comment<Stage>` in `shared-types` holds the anchor (`block_id`,
  `start`, `end`, `at`, `quote`, `picture`) and `content: Stage::Content`,
  one type for every stage above. `AskRequest.question` and
  `SideFollowUp.content` are `ClientContent[]`, and a side question's record
  keeps its question as `UserContentBlock[]`.
- The backend resolves a comment's uploads and remote files with the
  message's, in the same call. `comments_content` puts each comment's
  content after its quote.
- Stored data, by migration:
  - Transcript records format 19: a `user` or `steer` block's comments hold
    `content`. Format 18 and earlier are read with `[Text { text }]` for
    their `text`.
  - Control schema: each draft's comments hold `content: { text, files }`,
    with no files.
  - Conversation schema: checkpoint state format 11, whose queued messages'
    comments hold `content`; `side_questions.question` holds the question's
    content as JSON.
  - The page's draft database version 7: each comment waiting in a draft,
    carried by a message on its way, or kept by an edit in progress holds
    its words as a message with no files.
- An older page cannot send the new shapes and is refused as for any unknown
  field; pages update with the backend.
- The gallery's comment, selection field, waiting list, sent comments and
  side question specimens hold files, and the selection field has a state
  with a file.

## Spike

Whether the composer's editor keeps the page's selection the browser's
inside a frame, as the text area does, was not known. A test page made a
`tiptap` editor with the composer's extensions inside a same-origin frame's
document and drove it with real input:

- Words of the page were selected by a drag, the editor was clicked and
  "looks good" typed: the page's selection stayed "designed for a small"
  throughout, and the editor took every letter.
- An image on the clipboard pasted with Command-V reached the composer's
  intake as `image.png`, with the page's selection still in place.
- An editor made in the page and then moved into the frame is not enough:
  ProseMirror keeps listening for selection changes on the document it was
  made in, so the design makes it in the frame.

## Prior art

- **Slack** writes a thread reply in the channel's composer: formatting,
  files, pasted images and the attach button are the same.
- **GitHub** writes a review comment on a line in the same Markdown editor as
  an issue comment: pasted and dropped images and files upload into it.
- **Linear** and **Notion** write a comment in the editor of the document it
  is on, with files and images.
- **Google Docs** keeps a comment to plain text; this is the case Alex hit.
- **ChatGPT** and **Claude** quote a selection into the composer itself, so
  what is written about it is a full message.

## Alternatives

- **Add paste to the text area**, with a row of files under it. A second
  editor that takes files would repeat the composer's intake, uploads and
  capsules, and would differ from it in every detail it misses.
- **Quote the selection into the composer**, as ChatGPT does. The remark
  would lose its mark and its box under the line, which 0092 and 0115 keep.
- **Leave side questions out.** The selection field would then take a file
  that its Ask cannot carry, so Ask would refuse what Comment accepts.

## Open questions

None.
