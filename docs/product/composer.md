# Writing a message

The composer: writing and sending a message, typing with an input method, and attachments.

## Writing a message


The composer shows a message as it will look once sent. For example, a user
types "Compare ", drops `before.png`, types " with ", pastes `after.png`, and
ends with ". The **modal** `padding` is off." The composer reads

```text
Compare [▣ before.png] with [▣ after.png]. The modal padding is off.
```

with "modal" in bold, "padding" set as code, and each file a capsule where it
was put ([Attachments](#attachments)). The conversation shows the sent message
exactly so, the model receives its text and files in that order, and editing
it opens it the same way ([Message editing](../agent/message-editing.md)).

A message is Markdown in the user dialect below. The composer formats a
construct as it is typed, once its closing delimiter is; Backspace right after
gives back the characters. A star typed right before the construct holds it
back, so `**bold**` does not turn italic at its first closing star. A fence
typed as a line of its own opens a code block when the line ends, and typed
as the block's last line closes it. Pasted text is read the same way.

| Written | Shows as |
| --- | --- |
| `**bold**`, `*italic*`, `~~struck~~`, `` `code` `` | Formatted |
| A fenced code block | A highlighted code block |
| A Markdown link, or a bare `http` or `https` URL | A link, its target resolved as in [Files named in messages](file-previews.md#files-named-in-messages) |
| A Markdown image | The image, its target resolved the same way |
| A line break | A line break; a blank line is an empty line |
| `_` and `__`, a single `~`, HTML, `<…>`, math, tables, and lines that start a list, heading, quote, rule or indented code | The characters as typed |

The literal ones are what a conversation about code types as text:
`snake_case`, `__init__`, `~/.zshrc`, `x < y`, `$PATH`, `- item`. Formatting
stays on its line and on its side of a capsule: `**a` and `b**` on two lines
are the characters as typed. Emphasis beside CJK text follows the
[CJK-friendly amendment](https://github.com/tats-u/markdown-cjk-friendly/blob/main/specification.md)
to CommonMark, so `**注意：**这是` is bold as its writer means it. The same
holds wherever Demi reads Markdown: an agent's answer, a Markdown file's
preview, a notification's text and a conversation's title. Plain CommonMark
would show `**建议：**一组` with its stars, since its closing `**` follows a
full-width colon and comes right before a letter, which Chinese, Japanese
and Korean write without a space.

The model receives the message's Markdown. Where the user typed literally a
character the dialect would read as formatting, it carries a backslash
(`\*args`); nothing else is escaped, so `snake_case` and `x < y` reach the
model as typed.

Copying and pasting never add line breaks of their own: a copy of a
message, whole or a selection of it, holds no blank line before its first
line or after its last, and a paste into a composer drops the blank lines at
the start and end of what it pastes, keeping the lines between them and each
line's indentation. For example, the user copies a sent message and pastes it
back into the empty composer: the composer holds exactly the message, with no
line under it.

A selection that runs past one message, from the user's message into the
answer, copies as a web page does: the text of everything selected, in
order, as it reads on screen, with none of a message's own forms. For
example, the user drags from their question into the answer and copies: the
clipboard holds both, the question as shown rather than its Markdown.

Copying a sent message, by its Copy control or by selecting part of it,
copies its files with its text, and pasting that into a composer brings the
files back as capsules, where they stood, without uploading them again. For
example, the user copies "Compare `before.png` with `after.png`" from a sent
bubble and pastes it into the composer of another conversation of theirs:
the composer holds the same text and two image capsules, ready to send. The
clipboard carries the copy in three forms, each for the place it is pasted:

| Form | What it holds | Who reads it |
| --- | --- | --- |
| Demi's own | The message's content: its text, and each file as the reference the message holds (its stored file, name, kind and size, or its device and path) | A composer of the same Demi; it rebuilds each capsule from its reference, so the file is not sent again |
| HTML | The text formatted, each image as an image | Mail, documents, chat apps |
| Plain text | The Markdown, each file by its name | Terminals, code editors, anything else |

Demi's own form travels inside the HTML, as an attribute of its outermost
element, since every browser keeps HTML on the clipboard and none keeps a
format of its own across pages; a paste reads it there first. A copy that
holds a single image and no text also carries the image itself, so pasting
it anywhere pastes the picture. A file whose reference the paste cannot use,
one of another account or since deleted, comes in from the image the copy
carried when there is one, uploaded as any pasted image is, and otherwise
as its name in the text, with a toast that says which file could not be
added. Copying never puts a file's bytes on the clipboard for the composer's
sake; the reference is the file. A paste lands at once, its text and its
capsules, and each capsule attaches its stored file beside it as an upload
does, showing that it is under way, so the message can be sent at once and
waits for them as for an upload. A picture pasted in place of a reference
keeps the copied file's name.

Enter sends and Shift+Enter breaks the line; in a code block Enter breaks the
line and ⌘/Ctrl+Enter sends. While the agent works, ⌘/Ctrl+Enter outside a
code block sends the other way than Enter
([Steer or queue](turns.md#steer-or-queue)); in a code block it is the send key and
sends the way Enter does elsewhere. A phone's keyboard labels its return
key by this: *Send*, or *return* in a code block (`enterkeyhint`). An input method's Enter never sends, and
its Escape never ends an edit ([Typing with an input method](#typing-with-an-input-method)). A
message whose files are still uploading is sent like any other: it leaves
the composer, shows at once as the user's bubble with each capsule showing
how far its upload has come, and the tail row says Requesting from that
moment; it reaches the agent once every upload has finished, and never
without one of its files. For example, the user drops a 40 MB recording,
types "what goes wrong here?" and presses Enter: the bubble shows the
recording's capsule at 30%, and the turn starts when it reaches 100%. An
upload that fails leaves the message undelivered, its bubble saying so with
Retry, which retries the upload and then delivers the message; no file of it
is dropped. A page closed while such a message waits asks first, as a mail
client does while it sends attachments. Enter on a message that cannot go
yet for another reason does not send in silence: the send button says why it
cannot, in the words it says when it is pointed at — an upload that failed,
a model that cannot send — and the answer goes by itself. A message sent or queued, by Enter or by the send button, leaves the
focus in the composer, so the next one is typed at once. An edit sent with
Save and resend takes no typing while it is saved; once it ends, the
composer's draft takes the focus, unless the user has put it elsewhere
meanwhile. The composer and the conversation render a user message with one
editor, read-only in the conversation, so a sent message cannot look
different from what was written.

The composer stands one line high while the message fits that line. As soon
as the message needs more — a line break, a code block, an image, or text
wider than the line — the composer opens into a box that wraps the text,
grows to six lines and scrolls beyond them; it closes again once the message
fits one line. No part of a message is ever cut off at the line's end.

### Typing with an input method

For example, the user types `nihao` with a Chinese input method in the
filter of a menu and presses Enter to confirm 你好: the filter shows 你好,
and no row is picked. A second Enter picks the highlighted row.

A key that an input method uses to confirm or cancel a candidate, such as
Enter, Escape, Tab or an arrow, belongs to the composition. In every text
field of the app, the composer, a rename, a search, a dialog's field or a
menu's filter, it sends, submits, picks, confirms or closes nothing. This
holds in every browser, Safari included, which delivers the confirming Enter
just after the composition has ended, as Chrome and Firefox do not. The
`web-ui` text fields, the composer's editor and every key handler on them
pass such a key through one shared guard, as VS Code, Lexical and Headless
UI each do in one place.

## Attachments

Short pasted text stays in the composer and sends as message text. A paste of at
least 2,000 characters or 40 lines becomes `pasted-text.txt`, using the shared
composer's paste thresholds. Dropped, selected, and pasted files use the same
staged attachment flow, and editing a sent message adds files the same way.

The composer holds two things and nothing else: text in the user dialect,
and capsules. Whatever enters it, typed, pasted or dropped, from Finder, a web
page, another app or Demi's own transcript, is turned into those as it
enters, never kept as what it was. For example, the user drags a screenshot
from the agent's reply, or an image from a web page, onto the composer: it
lands as an `image.png` capsule where it was dropped, exactly as a pasted
screenshot does, never as a picture set into the text. So:

- A file, or an image whose bytes the page can read (dragged or pasted as a
  file, a `data:` image, an image of Demi's own page), becomes a capsule; an
  attachment of the conversation dragged from the transcript becomes a
  capsule of the same file, uploading nothing.
- An image the page cannot read, such as one another site serves without
  letting the page fetch it, becomes its link as text.
- Pasted or dropped rich text keeps only the dialect's formats; images in it
  follow the two rules above, and every other element, a table, a layout,
  a style, becomes its text.
- A capsule dragged within the composer moves; one dragged from another
  composer, as from a message being edited, is copied as a capsule.

Each attachment is a capsule in the message's text, and that text is the
editor's document. The document is the message: it says which files the
message carries, in what order, and what each one is. Nothing outside it adds
a file to a message or takes one away.

A capsule carries what it shows and what the message sends: the file's name
and kind, its picture when it has one, the device and path of a file that
lives on another device, and the opening lines of a text file. An uploaded
file's media type and opening lines come from the backend, in the answer to
its upload ([Uploads and media](web-api-hosts.md#uploads-and-media)), so the capsule
shows the file as the backend will send it. Bytes on their way to the Host
cannot be in a document, so they wait beside it under the capsule's id: the
file itself, how far its upload has come, whether it failed. That is a
transfer, not a fact of the message, and a capsule with no transfer beside it
is a file that needs none — a restored draft, or a sent message.

What follows from that split:

- A capsule is a character of the message and is edited as one. Backspace
  deletes it, undo brings it back, and it can be dragged to another place in
  the text; it carries no control of its own for any of that.
- A dropped file lands where it is dropped, a picked or pasted one at the
  cursor, and one added from a file browser at the cursor. A message holds any
  files the user adds, two of the same name included, as two screenshots
  pasted one after the other are both `image.png`: a name says nothing about
  a file's contents, and the Host keeps both, under its own suffixes
  ([Sending a file](#attachments)). Only an empty file and one over 25 MB are
  refused. A file joins the
  message as one change to its document, so there is never a file without a
  capsule or a capsule without a file.
- Deleting a capsule takes its file out of the message and stops its upload.
  The transfer waits, so undo brings the file back with the capsule and the
  upload starts over. What is still waiting goes when the message is sent or
  the composer is left.
- While a file uploads its capsule shows how far along it is, and a failed
  upload offers Retry. Pointed at, a capsule shows the picture larger or a
  text file's opening lines. A sent message shows every file as the
  composer did, an image or a video included: a capsule in the line, a
  small gap from the text and the capsules beside it. In the conversation,
  a click on a capsule opens its file in the File view.
- The message's content is read off the document: for the example in
  [Writing a message](#writing-a-message), `Compare `, then `before.png`,
  then ` with `, then `after.png`, then the rest. Providers pass content in
  order, so the model meets each file where the user put it.
- The saved draft keeps the message's text with a mark where each capsule
  stands and, beside it, each file as its upload or its device and path, so
  opening the draft on any page builds the same document again.

Sending a file stores it with the backend and nothing more:

```text
Composer file -> backend upload/blob -> attachment record in the message
                                          (a7, name, type, size)
                                               |
                         demi attachment download a7, when a command needs it
                                               v
                                     a file in the command's directory
```

Files remain staged in the draft until send, each as its upload, so another
device shows the file and can send it. The send gives each file the
conversation's next attachment number, the sequence the agent's own
attachments take theirs from
([Attachment commands](../execution/commands.md#attachment-commands)), and
writes nothing to the Host: most files are only for the model to read, which
it does from the blob, and a Host that is slow to reach, offline or a stopped
Cloud never holds a message. When a command needs the file, the agent fetches
it into the command's directory with `demi attachment download a7`.

The agent message contains one `attachment` record with number, name, media
type, size, and blob hash; a text file's record also carries its opening lines, which
the capsule shows when pointed at. The model receives the record as a tag
that names the file by its number ([Replay](../agent/transcript.md#replay)),
so it can fetch the file for its commands. The complete text
file is not duplicated into the message.

Native media adds the corresponding image, video, audio, or document input beside
the attachment record. An image enters fitted to what every provider accepts,
while the attachment keeps the original
([Images in the transcript](../agent/transcript.md#images-in-the-transcript)). The
attachment remains one capsule. The accepted extensions of each request's model
govern what the request carries: a medium the model does not accept, or one
too large for its requests, travels as a text that names it and says why
([Replay](../agent/transcript.md#replay)), and an adapter must not replace
supported media with a placeholder. Other files reach the
model through `demi attachment download`. Attachment presence and model capability are separate facts.

The upload cap is 25 MiB. Upload IDs and transcript media references travel in
conversation frames; bulk bytes use HTTP. Exact wire forms, ownership checks,
missing-upload behavior, and blob delivery to the web app are defined in
[Backend media handling](../backend/backend.md#media-by-reference) and
[Web API uploads](web-api-hosts.md#uploads-and-media).

A remote-file selection is different from an upload: it names an existing file
on a connected device, and its capsule names the device. Its bytes are read
when the model executes the supplied host command, so it is not a snapshot.
Revocation, disconnect, or file changes can affect that later read.
