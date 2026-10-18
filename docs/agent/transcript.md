# Transcript

What an agent's transcript holds: its block types, the context blocks, how it replays into a provider request, media and images, views, and its patches and versions. Sessions and turns are in [Agent runtime](runtime.md), and how the page draws a transcript in [Rendering](rendering.md).

## Transcript

Words used for session data:

| Word | Meaning |
| --- | --- |
| transcript | A session's history: its ordered blocks |
| status | A point-in-time answer an operation returns, such as a command's status; never stored as history |
| checkpoint | The durable, restorable state of one session ([Tree store](runtime.md#tree-store)) |
| whole output | A command's output as kept: on its Host while it runs, in the backend once it ends ([The whole output](shell-tool.md#the-whole-output)) |
| view | Bounded data a block carries for the user; never replayed to the model |
| blob | Content-addressed bytes, such as media, edit copies and commands' outputs, in the conversation owner's blob namespace |

### Block types

| Block | Written by | The model receives | The user sees it |
| --- | --- | --- | --- |
| `user` | A send or an edit: the submitted content, the comments sent with it (`comments`, each the quoted text and the comment, [Comments and side questions](../product/comments-and-ask.md)) and, for a subagent, its identity (`preamble`, [Child context](subagents.md#child-context)) | A user message: the preamble, then each comment as its quoted text in a `>` quote followed by the comment, then the content | Yes; the only editable block ([Message editing](message-editing.md)) |
| `context` | The session before a provider request, with the text one context source answered ([Context](#context)), the source's name (`source`), and for the instructions source the list of what the text holds (`instructions`, [What the card lists](instructions.md#what-the-card-lists)) | A user message with its text | No |
| `wakeup` | Command reports that arrived together, with the placement `new_turn` or `steer`: each report's command, its call's title and what it reports | The reports' text, as a user message or as a steer | As a notice row for each end it reports, none for progress ([Command reports](shell-tool.md#command-reports)) |
| `steer` | A human steer, at a continuation boundary | A steer in the current turn | Yes |
| `agent_message` | Another agent of the tree ([Communication](subagents.md#communication)) | A steer holding the message's source envelope | As a receipt row |
| `resume` | A turn continuing after a cut: `resume`, compaction inside a turn, or a model switch that landed inside a turn and compacted | A user message: "Continue from where you left off." | No |
| `abort` | Stop | Nothing | Yes, until the turn is continued and `isResumed` is set |
| `thinking` | The provider: reasoning text and its signature | The text; a signed block whole | Yes |
| `redacted_thinking` | The provider: opaque reasoning data | The data, whole | No |
| `text` | The provider: assistant text, marked `forkable` once complete ([Eligibility](conversation-fork.md#eligibility)) | The text | Yes |
| `tool_call` | The provider's call, completed by the session with the result | The call and, once completed, its result | Yes |
| `response` | The provider: the usage of one completed request | Nothing; its usage anchors the context estimate | No |
| `error` | A failed request or an interrupted turn ([The failure record](failures-and-recovery.md#the-failure-record)); `outsideTurn` when the failure ended no turn ([The unfinished turn](failures-and-recovery.md#the-unfinished-turn)) | Nothing | Yes |
| `account_switch` | [Account rotation](../providers/providers.md#account-rotation): the account that reached its limit, the window and its reset, and the account the agent continues with | Nothing | Yes, as a neutral line, its time in the viewer's zone |
| `compaction_boundary` | Compaction: the summary, inserted where the kept history begins | A user message: "Previous conversation summary:" and the summary | Through its marker; at its own place only when an edit removed the marker |
| `compaction_marker` | Compaction: the estimated size of what was summarized, appended at the end | Nothing | Yes, as the compaction's divider, with its boundary's summary size |

Each block carries the time it began by the backend's clock: a `text` block
the arrival of its first text, a tool block the call's start. The page shows a
reply's time from it, and never shows a past time as a future one.

A compaction shows where it was triggered, not where its summary sits. For
example, the user compacts after the eleventh message: the boundary goes in
before the tenth answer, and the marker after the eleventh message, so the
divider reads below the eleventh message, where the user acted. While the
phase is `compacting`, the page shows the divider in progress at the end of
the transcript, before pending steers and queued messages, which is where the
marker will be appended; the finished divider then takes its place. A pass
that fails ends with its `error` block at the same place
([Retries](failures-and-recovery.md#retries)). A compaction is not the end of
a turn: behind its divider, the turn before keeps its Resume or Continue
([The unfinished turn](failures-and-recovery.md#the-unfinished-turn)).

A `user`, `context` or `wakeup` block with the placement `new_turn` opens an
input turn: recovery treats it as the start of its turn
([Recovery](failures-and-recovery.md#recovery-is-one-mechanism)). For retry,
the first `agent_message` of a continuation opens its turn as well. The other
blocks belong to the turn they appear in.

A `tool_call` block holds the provider's `toolUseId` and `toolName`, the call's
`input` as the JSON text the provider supplied, its `status` (`executing`,
`completed` or `error`), its `output`, and its `view`.

### Context

What the model must learn between its requests, and what differs by user or
conversation, reaches it as `context` blocks, never through the system prompt.
For example, the user switches the conversation from the Cloud to their
laptop. Before the node's next request, the execution context source answers
the new target, and the model receives it as a user message.

Before each provider request of a node, the session asks every context source
the product supplies, in the product's order:

- Each source is given the text of its own `context` blocks that the model
  receives, those from the last `compaction_boundary` on, oldest first, with
  the node's working directory and the id of its current input turn, and
  answers new text or nothing. After a compaction, a source therefore tells
  the model again what the summary may have left out.
- The product's order is the execution context, then the instructions, then
  the side questions, then each plugin that is a context source. The side
  questions source answers only at the first request of a new input turn of
  the conversation's root, and only with side questions asked since its last
  block ([What the agent learns](../product/comments-and-ask.md#what-the-agent-learns)).
- Each answer becomes one `context` block that names its source: `execution`
  for the conversation's execution context
  ([Switch the primary target](../execution/sessions-and-targets.md#switch-the-primary-target)),
  `instructions` for the user's personal instructions and the project's
  instruction files ([Instructions](instructions.md)), which also lists what
  its text holds, `side_questions` for what the user asked aside, or the id
  of the plugin that answered
  ([Prompt text and context](../architecture/plugins.md#prompt-text-and-context)).
- The blocks are appended before the request and saved at once, so a request
  never carries a context its transcript does not hold.
- A source that fails adds no block, and the failure is logged. Nothing is
  lost by it: the source is asked again before the next request.

### Replay

The model receives the blocks from the last `compaction_boundary` onward, the
replayed blocks, each as the table says. A long text is cut in the middle by
the replay bound ([Text bounds](compaction.md#text-bounds)). Signed thinking
and redacted data are replayed whole, because the vendor verifies them as they
were sent. Each medium is replayed with the bytes the session holds for it,
and a tool's medium that is gone as its text ([Media](#media)). A message's
reference is replayed as its text, and an attachment record
([Attachments](../product/composer.md#attachments)) as a tag that names the
file, its media type, its size and its path, never its content. A tool
call's input is replayed as the JSON value the provider supplied, or as text
when it is not valid JSON. A call of a tool the request no longer declares,
such as `yield` or `shell_status` in a transcript from before they were
removed, is replayed with its result as the model's text, `[called yield {"durationMs":600000}: yield
scheduled]`, since a vendor may refuse a call of a tool it was not given.

A tool call keeps the id its provider gave it, stored once and never
rewritten, such as Codex's `call_7Qm…|fc_68e…`, its call id and item id
joined by `|`. A provider whose vendor restricts ids sends each id with
every character its vendor does not allow replaced by `_`, the call and its
result alike: Anthropic allows letters, digits, `_` and `-`, so that id goes
to Claude as `call_7Qm…_fc_68e…`. The form follows from the stored id alone,
so every request sends the same bytes and the cached prefix holds, and a
switch back to Codex sends its own id again, unchanged.

An image, video or document is replayed as a text that names it and says why
it was not sent when the request's model does not accept its type
([Accepted attachment types](../providers/models.md#accepted-attachment-types)),
or when its base64 is longer than half of the model's request body limit
([Request limits](../providers/models.md#request-limits)), which leaves no room
for the history around it:

| The medium | Its text |
| --- | --- |
| Of a type the model does not accept | `[<kind>:<name>, not sent: the model does not accept it]` |
| Over half of the model's body limit | `[<kind>:<name>, not sent: too large for the model's requests]` |

The kind is `image`, `video` or `document`, and the name is the media type of
an image or a video and the file name of a document. The text depends only on
the medium and the request's model, so every request to that model carries the
same text. For example, after a switch to a model without video, a tool
result's video reaches that model as
`[video:video/mp4, not sent: the model does not accept it]` in every request,
and a switch back sends the video again.
The token estimates for a model weigh such a medium as that text
([Block estimates](compaction.md#block-estimates)).

#### Request size

A request's size is what its content weighs as the vendor receives it: the
base64 length of each image, video and document it replays, plus the UTF-8
length of its system prompt and texts. Its images are the image blocks it
replays, in user messages, steers and tool results alike. Both are read off the
request's items ([Replay](#replay)), after the media the model does
not accept, or that are too large for its requests, became text, so they
count what the request sends. The bytes the vendor's
format adds, such as JSON keys and escapes, are not counted. Nothing compacts
or leaves media out on a request's size
([When compaction runs](compaction.md#when-compaction-runs)): a request over
its vendor's limits is refused and fails its turn with `request_too_large`
([Retries](failures-and-recovery.md#retries)), and images are kept light where
they enter ([Images in the transcript](#images-in-the-transcript)).

Reasoning between the last `compaction_boundary` and its marker, which
compaction kept after the summary, is marked as kept past a summary: the
history it followed is gone, and a provider whose vendor checks reasoning
against that history leaves it out
([Per vendor](../providers/providers.md#per-vendor)). When an edit removed the
marker, all reasoning after the boundary is marked, since leaving reasoning
out is safe and replaying reasoning whose history changed is not.

Replay reads only what the blocks hold for the model, and which media the
request's model accepts, never an ID or a time, and a block keeps its place.
The same history therefore gives the same items for the same model in any
session. Each request's items begin with the previous request's: new blocks
are appended, a tool call's result completes a call no request has carried
yet, and a view is never replayed. Only compaction, a model switch and an edit
change what an earlier request carried; a retry and a resume cut back to an
earlier request. This is how a session keeps its vendor's cache
([Prompt cache](../providers/providers.md#prompt-cache)).

A request also says how many of its leading items the session's latest
answered request carried. That request is the one whose `response` block
comes last after the last compaction, that is, after the last
`compaction_boundary` and its marker when the history has them; its
answer begins at the first of the thinking, text and tool-call blocks directly
before that `response` block, and its content is what the blocks before its
answer replay. Compaction summarizes that content
([One pass](compaction.md#one-pass)), and a vendor reads its cache entry at its
end. Before any request is answered after the last compaction, the request
says none. With Claude Code, a run that ends in a batch of tool calls
([Tool-call batches](../providers/claude-code.md#tool-call-batches)) writes no
`response` block, because the CLI reports usage only on the line that ends its
turn
([Requests over stream-json](../providers/claude-code.md#requests-over-stream-json)),
so its latest answered request is the last request that ended a turn. A title request says instead that it is no session's
([Conversation titles](../product/conversations.md#conversation-titles)).

### Media

A message's images, videos and documents and a tool result's images,
videos and documents are stored once, as blobs in the conversation owner's blob namespace,
and blocks hold them by reference: `{ type: "ref", ref, mediaType }`, with an
image's `width` and `height` in pixels, which its fitting reads, a video's
when the backend reads them from its container's header, and a
document's `fileName`. The session keeps a medium's bytes only while a
provider request can still send them.

For example, the agent runs a command that prints a 400 KB screenshot:

```text
the tool returns the PNG's bytes
  -> the session's store puts them under their SHA-256:  blobs/<owner>/<sha256>
  -> the tool_call block's result holds the reference, and the
     session holds the bytes beside the transcript
saves, patches, resets, syncs, other pages, edits, Forks:  the reference
each provider request while the block is replayed:         the held bytes
```

A medium has one form in each place, and each form is a type of its own, so
a block cannot hold bytes and a provider cannot receive a reference:

| Where | The medium |
| --- | --- |
| What a tool returns | Its bytes and media type |
| Blocks, queued messages, pending steers, checkpoint rows and frames | A reference; a message's image or video may name a URL the vendor fetches instead |
| A provider request's items | Its bytes and media type, or that URL |

- **Storing.** A medium's bytes are stored when the medium enters, and never
  again; an image is fitted before that
  ([Images in the transcript](#images-in-the-transcript)). An upload is
  stored when the page uploads it, and the medium a message receives from it
  references the upload's own blob, or the fitted image's, which the backend
  stores as it resolves the message's content
  ([Media by reference](../backend/backend.md#media-by-reference)). The
  session has its store put a tool's medium after the tool returns and
  before the result enters the transcript. The put computes the bytes'
  SHA-256, which names the blob, and sends no bytes for a blob the namespace
  holds already ([The object store](../backend/storage.md#the-object-store)).
  A tool's medium whose put fails is gone from its result (below), so the
  turn goes on and no block names a blob that was not stored.
- **A medium that is gone.** A tool result's medium whose bytes
  could not be stored gives way, in its place, to a part of its own that says
  what it was and why it is gone:
  `{ type: "gone", kind, mediaType, cause }`, where `kind` is `image`,
  `video` or `document`, and `cause` is `{ type: "not_stored", error }`, with the store's
  error. The model reads the part as one line of text, which the agent
  renders in one place: `[<kind> not stored: <reason>]`. Nothing that was
  stored is ever removed ([Retention](../backend/storage.md#retention)). The
  page shows the part where the medium was
  ([Media a tool returned](../product/file-previews.md#media-a-tool-returned)).
  A message's media are never gone: an upload is stored before a message can
  name it.
- **References.** The transcript, the queued messages, the pending steers,
  every checkpoint row and every frame hold media only by reference, or by
  the URL a message's image or video names: their types have no place for
  bytes. Nothing converts media when a block is saved or sent, and an edit or
  a Fork copies references.
- **Held bytes.** The session holds something only for the media that its
  replayed blocks or its waiting input (queued messages and pending steers)
  reference: the medium's bytes, or the fact that its blob is missing. A
  medium arrives with its bytes: a tool's medium with its own, an upload's
  with those the backend read to write the file to the Host or with the
  fitted image's. When a new
  `compaction_boundary`, a history rewrite or a withdrawn input leaves a
  medium unreferenced, the session lets go of it before its next request. A
  live tree therefore holds, per live session, at most the media its next
  request sends and those of its waiting input.
- **The model's view.** Replay, the token estimates and compaction read the
  replayed blocks together with what the session holds for each medium they
  reference: its bytes, or the fact that its blob is missing
  ([Replay](#replay), [Token estimates](compaction.md#token-estimates)).
  Before the session builds that view, it reads the blob of each replayed
  medium it holds nothing for, which after a restore is every one, a few at a
  time concurrently, since on S3 each read is a round trip; it builds the
  view only once it holds something for every replayed medium. Opening a
  conversation therefore reads no blob, a request never reads a blob the
  session holds, and no blob from before the last `compaction_boundary` is
  read.
- **Requests.** Replay puts each medium's held bytes into the request's
  items, and a medium whose blob is missing becomes the text
  `[missing <kind>]` there, so the turn goes on; the transcript
  keeps the reference. A provider request is the only place a medium carries
  its bytes, and it carries no reference, so a provider never has one to
  refuse.
- **Stable requests.** Within a live tree, a medium reaches the model in one
  form for as long as it is replayed: its held bytes, or the missing text for
  a blob found missing. Bytes the session let go of and reads again are the
  same bytes, since a blob's name is their hash and no blob is ever deleted
  ([Retention](../backend/storage.md#retention)). Media therefore never
  change the start a request shares with the previous one.
- **Failures.** A read that fails, rather than finding the blob missing,
  fails the action before its request, as a failed save does; the next
  action reads again.
- **Where the bytes go.** The agent defines these rules, and the backend
  decides where the bytes go. A session reaches the owner's blob namespace
  only through its tree store ([Tree store](runtime.md#tree-store)); the agent's server
  has no blob store of its own.

### Images in the transcript

An image enters the transcript once: when a tool result attaches it
([Media the model views](shell-tool.md#media-the-model-views)), or when a message's upload
becomes its native medium
([Attachments](../product/composer.md#attachments)). It is fitted then to what
every provider accepts of one image, at most 2,000 px on each side and
5,000,000 bytes as base64, which is 3,750,000 bytes of image, and kept light,
since every request replays it. It never changes afterwards, so every request
sends the same bytes.

| The image | What enters the transcript |
| --- | --- |
| PNG, JPEG or WebP, at most 2,000 px on each side and 500,000 bytes | The image, unchanged |
| Larger than 2,000 px on a side | The image turned upright by its EXIF orientation and scaled down to fit 2,000 × 2,000 px, keeping its aspect ratio: a JPEG as JPEG at quality 90, a PNG or WebP as PNG |
| A GIF | Its first frame as PNG, fitted the same way |
| Over 500,000 bytes after that | The fitted image as JPEG at quality 85, its transparency flattened onto white, when that is smaller |
| Still over 3,750,000 bytes, not decodable, or needing more than 256 MiB to decode | No image: a tool result says why, and an upload stays an attachment the model reads by path |

For example, a screenshot of 3,000 × 1,500 px enters as 2,000 × 1,000 px, and
an illustration of 1,672 × 941 px saved as a PNG of 2.2 MB enters as a JPEG of
about 440 KB. Half a megabyte keeps a screenshot of text, seldom heavier as PNG,
lossless, while a photograph or an illustration enters at a fifth of its size. The
limits follow the vendors: the Anthropic API refuses a request with more than
20 images when a side of any of them exceeds 2,000 px, and OpenAI and Gemini
scale a larger image down themselves; the vendors' strictest limit for one
image is that of the Anthropic models on Amazon Bedrock and Google Cloud, 5 MB
as base64. A GIF becomes a PNG because Gemini does not read GIF and the other
vendors read only its first frame. A fitted image carries no EXIF data, so its
orientation is applied before it is scaled.

The original stays where it came from: an image the model viewed in the
file it viewed, and an upload in its attachment file
on the Host and in its upload blob, whose attachment record keeps the
original's size and hash. Fitting decodes and encodes on the
blocking pool ([Blocking work](../architecture/concurrency.md#blocking-work)).
Videos and documents are not fitted; they count toward the request's size
([Request size](#request-size)).

### Views

A `tool_call` block's `view` carries bounded data for the user. It is never
replayed to the model, it never embeds unbounded payloads such as full output,
file bodies or raw bytes, and its type is fixed per tool by `kind`:

| `kind` | Fields |
| --- | --- |
| `shell` | `status` (`running`, `exited` or `aborted`); `commandId`; `exitCode`, once exited; `runningMs`; `idleMs`; `chunks`, the last 32,768 characters of the output the result covers, stdout and stderr merged, each chunk tagged with its stream, a line that stands for bytes the output does not hold tagged as stderr; `viewTruncated`, true when that window or the output itself was cut; `files` and `filesTruncated`, once the command has exited and changed files |
| `repeated_shell` | `script`, `count` |

The shell view's characters are Unicode scalar values, counted from the end so
the newest output stays. `files` lists one entry per changed path with its line
counts and, for each edit segment, the blobs of its two sides when they were
stored ([Edit copies](../execution/edit-tracking.md#edit-copies)).
The model's result and the view come from the same command status; the view
shows nothing the model could not read from the command, except `files`, which
exists only for the user.

### Patches and versions

Every change a session makes to its transcript is recorded as patches, each
naming the index of the block it touches:

| `op` | Fields | Meaning |
| --- | --- | --- |
| `add` | `index`, `value` | Insert a block at the index |
| `replace_block` | `index`, `value` | Replace the block at the index |
| `append_text` | `index`, `delta` | Append text to the block at the index |
| `truncate` | `length` | Remove every block from the index `length` on |

Indices in patches are positions on the current path
([Versions](message-editing.md#versions)). Consecutive appends to one block
merge into one `append_text`. A rewrite of the path, such as a retry, a
resume's unwind, an accepted edit, Regenerate or a switch of versions, is
one batch; the rows it leaves stay stored on their branch:
a `truncate` at the first block it changes, then an `add` for each block
from there. It never sends the blocks before the cut, which may be most of
a long transcript, and which a page may not hold
([Transcript windows](../product/web-application.md#transcript-windows)).
Each batch of patches advances the transcript's revision by one.

A transcript version is `{ epoch, revision }`. The epoch is new each time the
session's transcript is built, at creation and at every restore, so a version
taken before a backend restart never matches after it. Message editing uses
the version to refuse an edit made against a transcript that has changed since.
