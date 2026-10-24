---
status: proposed
---

# A medium a tool showed the model is an attachment

## Problem

An agent checks a change on the running product. It saves a screenshot,
runs `demi file view shot.png` to look at it, and then wants to show the
screenshot to Alex in its reply. The tool result says only
`[image 1: image/png, 1200 × 1500 px, 133808 bytes]`, so the agent runs
`demi attachment upload shot.png`, which reads the file from the Host and
sends its bytes to the backend a second time, and only then has an id to
write as `<attachment id="a9" />`.

The backend already had those bytes. When `demi file view` ends, the runner
announces each medium by its SHA-256, and the backend reads its bytes from
the Host, or from the conversation owner's blobs when they hold it, to build
the tool result. Every upload of a file the agent has just viewed is a
second transfer of a file the backend has, and one more command each time.

## Decision

Each medium a tool result reports, an image, a video or a PDF document, is
an attachment of the conversation from the moment the command ends, and the
result names its attachment id. The agent shows it to the user by writing
that id, with no upload.

```
[image 1: shown to you, not to the user; attachment_id: a12]
[image 2: shown to you as image/jpeg of 1568 × 1960 px, fitted from image/png
of 2400 × 3000 px, not to the user; attachment_id: a13]
[image 3: not shown: the model does not read image/webp; attachment_id: a14]
```

* **Every medium gets an id**, shown to the model or not, so the agent can
  show any of them. An id the reply never writes shows nowhere: a reply shows
  only the attachments it names.
* **The attachment is the original**: the bytes `demi file view` handed over,
  never the copy fitted for the model.
* **The same bytes keep one id**: a medium whose blob an attachment of the
  conversation already has takes that attachment's id, so viewing a file
  twice gives one id.
* **A medium the backend could not read** has no id; its line says why, as
  today. A medium it could not store has none either, and its line says why:
  `[image 1: shown to you, not to the user; not an attachment: …]`.
* **A stopped command's media are no attachments**: its result shows none of
  them.
* **A command lost with its Host's connection** keeps each medium whose blob
  the conversation owner's namespace holds: the result shows it and names its
  id, as a command that ended does.
* **The id outlives a restart of the backend**: a command's record keeps what
  its result tells of each medium, without its bytes, so a result that a
  backend started again reports names the same ids:
  `[image 1: not shown: not kept past the command's end; attachment_id: a12]`.
* `demi attachment upload` stays for files no tool result showed. An upload,
  and the picture a reply shows from a file, of bytes an attachment of the
  conversation has takes that attachment's id, so one file is one attachment.

The design to build from:

* `MediumFacts.name` carries the file's name for every medium `demi file
  view` reads from a file, not only for a document. A medium read from stdin
  is named by its kind and number: `image-1.png`, `video-2.mp4`,
  `document-3.pdf`.
* `CommandKeeper` (`backend-remote-host`) gains

  ```rust
  /// Stores each medium of `command` as an attachment, in order: its id,
  /// or why it could not store it; none for one whose bytes the backend
  /// does not have.
  fn attach<'a>(
      &'a self,
      command: &'a CommandId,
      media: &'a [CommandMedium],
  ) -> LocalBoxFuture<'a, Vec<Option<Result<String, String>>>>;
  ```

  The shell environment calls it once the media are read and the command's
  end is known, before the command reads as ended, for a command that exited
  or was lost, and `CommandMedium` carries what it answers. The
  `Keeper` of `backend-host-access` puts each medium's bytes as a blob,
  which the blob store keeps once by its SHA-256, and in one transaction
  takes the number of an attachment with that blob, or writes a row with the
  next number of the conversation's `attachment` sequence, the agent's node
  and no source.
* `agent-tools` writes the line of every medium a result reports, with its
  `attachment_id` or why it has none, where it now writes one only for an
  image it fitted and for a medium it did not show.
* The conversation's `command_outputs` row gains `media`, a JSON array of
  each medium's facts, its attachment or why it has none, and why the backend
  lacked its bytes. The result a backend started again reports reads it. The
  schema has shipped in no release, so only development databases migrate,
  by adding the column.
* The harness guide says, in its line about attachments, that a medium a
  tool result showed has its `attachment_id` already.

Nothing new is stored but the attachment's row, the original's blob when
the model saw a fitted copy, and the command row's `media`. No protocol
changes: the page learns of
the attachment as it learns of an upload. A conversation of an earlier
release keeps its attachments; its earlier results name no id, and the agent
uploads those files as before.

## Spike

None: the bytes already reach the backend at the command's end
(`backend-remote-host/src/shell_environment.rs`, `media`), and a row and
its number are what `demi attachment upload` already writes.

## Prior art

* [LibreChat](https://github.com/danny-avila/LibreChat) stores each file a
  code execution makes as a file record with its own `file_id` and attaches
  it to the tool call's message, where it shows at once
  (`api/server/services/Files/Code/process.js`).
* OpenAI's Assistants API gives each image its code interpreter makes a
  `file_id`, which the assistant's message names as an `image_file`.

Demi gives every medium an id as both do, but shows it only where the reply
names it: most media an agent views are steps of its work, such as a
screenshot it checks before it fixes a layout, not something to hand over.

## Alternatives

* **Give an id only on request**, with a command that turns a result's
  medium into an attachment: one more command each time, which is the cost
  this record removes.
* **Show every medium in the reply**: the reply would carry every
  screenshot the agent took to check its work.
* **Attach the fitted copy**: the user would get less than the file, unlike
  an upload of it.

## Open questions

None.
