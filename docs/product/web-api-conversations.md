# Web API: conversations

The routes of conversations: creation and Fork, history, the work panel, drafts, messages saved for later, permissions, credentials, a directory's workflows, the sidebar's changes and read state, sharing and search. Conventions every route follows are in [Web API](web-api.md).

## Conversation creation and Fork

`POST /conversations` requires a client-generated UUID. A new conversation uses
Cloud as its target without waking a machine. The body may also carry what a
new conversation starts with, so creating it is one request: the fields
`PATCH /conversations/:id` takes except `archived` (`title`, `pinned`,
`target` and the model settings). They apply as part of
the creation: one that fails refuses the creation with its status and
`{ code, message }`, and nothing is created. Retrying the same ID for the same
owner returns the existing record (200 instead of 201) and applies nothing.
Both answer `{ conversation }`, the conversation as the list shows it. An ID owned by
another user or reserved for a Fork returns 409 `id_unavailable`.

`POST /conversations/:id/fork` takes a destination UUID and an assistant block ID.
It copies a history boundary into a new conversation and returns
`{ conversation }`, with 201 on creation and 200 for a completed retry; the
destination's model settings are the source record's at the time of the Fork.
It does not mutate the source. The operation reserves the destination and
records its metadata so publication can recover after interruption. A source the caller does not own
answers 404 `conversation_not_found`; the destination UUID of another creation
attempt, with another source or block, 409 `fork_conflict`; a UUID another
conversation holds, 409 `id_unavailable`; and a block that is not a completed
assistant text of the source's history, 400 `invalid_fork_target`. Which
texts can end a Fork, and what the destination keeps, belong to
[Conversation Fork](../agent/conversation-fork.md).

## Conversation history

A conversation's history reaches a page a page at a time, never whole. For
example, the user opens a conversation of 3,000 blocks, most of them tool
calls whose output runs to megabytes. The page reads the latest page of its
transcript, the last few requests in about 64 KiB, and shows it; scrolling
up reads the page before it, and a search result opens on the page around
its message. A folded tool call arrives as the row the user sees, with its
title, its outcome and the files it changed; its script and output are read
when the user opens it. How the page holds and joins what it read belongs to
[Transcript windows](web-application.md#transcript-windows).

Every route here reads the conversation's database alone, as the archived
list does: it wakes no Host and needs no live session, and an archived
conversation answers it. A conversation the caller does not own answers 404
`conversation_not_found`, and a `node` that is not one of its agents 404
`not_found`.

### Pages

`GET /api/conversations/:id/transcript` answers one page of one agent's
transcript: the conversation's own without `node`, a subagent's with
`node=<subagentId>`. A block's place is its index in its agent's
transcript, as the database stores it ([Conversation state and
transactions](../backend/storage.md#conversation-state-and-transactions)).

| Query | The page |
| --- | --- |
| None | The latest page |
| `before=<index>&edge=<blockId>` | The page that ends just before the block at `index`, which the page holds as `edge` |
| `after=<index>&edge=<blockId>` | The page that starts just after the block at `index`, which the page holds as `edge` |
| `around=<blockId>` | The page that holds the block, with requests on both sides |

A page holds whole requests. A request is a `user` block and the blocks after
it up to the agent's next `user` block, and the blocks before the first
`user` block belong to the first request. The latest page holds the last
request, and before it as many whole requests as keep the page within
64 KiB of its JSON; `before` and `after` take requests from their edge the
same way, and `around` takes the block's request and then adds requests
after and before it in turn. A page holds at least one request whatever its
size, so a request is never cut: what the page derives from a request, such
as its work groups ([Work groups](../agent/rendering.md#work-groups)) and the
files it changed ([Edit tracking](../execution/edit-tracking.md#what-the-conversation-shows)),
is whole wherever it is shown.

The answer is `{ start, length, blocks, failures, instructions, summaries }`:

| Field | Meaning |
| --- | --- |
| `start` | The index of the page's first block; the page reaches the transcript's start when it is 0 |
| `length` | How many blocks the transcript holds; the page reaches its end when `start` plus its blocks is `length` |
| `blocks` | The page's blocks in their [light form](#light-form) |
| `failures` | The [failure facts](../backend/backend.md#failure-facts) of its error blocks, by block id |
| `instructions` | The entries of the agent's newest instructions block, which the context card lists ([What the card lists](../agent/instructions.md#what-the-card-lists)), empty before the first |
| `summaries` | Each compaction marker of the page (`marker`) with its boundary's summary size (`summaryTokens`), which its divider tells; the boundary may be on another page |

`edge` is how a page that extends what it holds knows the two still join: a
retry, an accepted edit or a compaction can rewrite the transcript between
two reads, and indices then name other blocks. A block at `index` that is not
`edge` answers 409 `transcript_changed`, and the page reads around a block it
shows ([Rewrites](web-application.md#rewrites)). `around` a block the
transcript does not hold answers 404 `block_not_found`. An index outside the
transcript, or two of `before`, `after` and `around`, answer 400
`invalid_query`.

### Light form

A page's blocks leave out what only an open row shows, which is nearly all
of a long conversation's bytes; the rows a folded transcript shows need
nothing more. Every block of a page is in its light form: a field left out
is empty, and every other field means what it means in the block whole
([Block types](../agent/transcript.md#block-types)):

| Block | Left out | Carried instead |
| --- | --- | --- |
| `tool_call` | An `input` past 512 bytes, but its `description`; the text of `output`; the output text of `view` (its chunks) | Such an `input` as JSON holding only the `description`, a shorter one whole, as a script that only looks at a command must be to name the look on its row ([Work groups](../agent/rendering.md#work-groups)); the media of `output`; the rest of `view`: the command, its status, exit code and times, the files it changed and their renames |
| `thinking` | The text and its signature | `textLength`, the length in characters of its text without the white space around it: the row tells how long the agent thought, and a thinking whose length is 0, such as the signed one Gemini writes before each call or an OpenAI reasoning item with no summary, has no row, as it has none live |
| `redacted_thinking` | The data | Nothing |
| `agent_message` | The message | Its sender and what it reports |
| `wakeup` | Each report's output | Each report's command, title, event and media |
| `context` | The text and the instructions it lists | Its source |
| `compaction_boundary` | The summary | Its size |

`user`, `steer`, `text`, `error`, `abort`, `resume`, `response` and
`compaction_marker` blocks come whole. Live frames send blocks whole
([Frame protocol](../agent/rendering.md#frame-protocol)), so the page holds a
block in light form only when it read it from a page, and it marks such a
block as light until the block arrives whole. A running command's tab in the
Running window needs its call's script: when the page holds that call light,
with its input cut, or does not hold it because it lies before the pages
read, the tab reads the call whole by its id, as an opened row does.

`GET /api/conversations/:id/transcript/blocks/:blockId?node=` answers
`{ block, failures }`, the block whole. The page reads it when the user opens
a row whose block it holds in light form. A block the transcript does not
hold answers 404 `block_not_found`.

### Subagents and commands

`GET /api/conversations/:id/subagents` answers `{ subagents }`, the job of
each subagent the conversation has had, in the order they started, as the
`subagent` frame gives it ([Protocol](../agent/subagents.md#protocol)). A
page reads a subagent's transcript, by its pages, when it shows it.

`GET /api/conversations/:id/commands/:commandId` answers where the `shell`
call that started a command of the tree lies, which a report's title brings
into view ([Command reports](../agent/shell-tool.md#command-reports)): `{
commandId, subagentId, blockId }`, `subagentId` null for the root's. The
page then reads the page around `blockId` in that agent's transcript. A
command the conversation does not hold answers 404 `not_found`.

## Work panel state

The backend keeps the [work panel](web-application.md#work-panel)'s tabs of
each conversation: `{ revision, tabs: [{ id, kind, data }] }`, in order.
`revision` counts the changes, 0 before the first. The backend does not
interpret `kind` and `data`: they mean something to the page, which validates
each tab's `data` against its kind's schema when it reads the panel, and to
the plugin that owns the kind ([Panel kinds](../architecture/plugins.md#panel-kinds)).
The selection is not part of it: each page keeps its own.

For example, the user clicks a file a message names, and the page opens it
in a file tab:

```text
POST  .../panel/changes     { changes: [{ op: "create", id: "a1", kind: "file",
                                          data: { path: "/work/src/app.ts" } }] }  -> { revision: 4 }
GET   .../panel             -> { revision: 4, tabs: [{ id: "a1", kind: "file",
                                                       data: { path: "/work/src/app.ts" } }] }
```

`GET /api/conversations/:id/panel` returns the panel, `{ revision: 0, tabs:
[] }` for a conversation never changed. `POST
/api/conversations/:id/panel/changes { changes }` applies a list of changes in
order, as one change of the panel: Close Others, which removes eight tabs, is
one request and one new revision, and other pages see the eight tabs go at
once.

| Change | Fields | Does |
| --- | --- | --- |
| `create` | `id, kind, data, index?` | Creates the tab at `index`, after the others without one |
| `update` | `id, data`, an object | Sets each of its fields in the tab's `data`, removing those that are null, and leaves the other fields |
| `delete` | `id` | Removes the tab |
| `move` | `id, index` | Moves the tab to `index` among the others |

The request answers `{ revision, changed }`: the panel's revision once the
changes are in it, and whether the request changed the panel. A change that
has nothing to do changes nothing, and a request of only such changes answers
the current revision with `changed` false. A page that made a change reads the
panel only when the answer shows another change it does not hold: `changed`
with a revision past its own plus one, or `changed` false with a revision past
its own: an update, move or removal of a tab the panel no longer has,
and a create of an id the panel has, or had. **A tab's id is used once per
conversation.** The page makes a new one for each tab it creates, so sending a
create again after a lost answer creates nothing more, and a create that
arrives after the tab's removal brings nothing back. A plugin derives the ids
of its tabs from what they show, such as `browser-t7` for the browser's tab
`t7`, so it can add a tab for each such thing and none again once its user
removed it. An id is 1 to 64 characters.

Every change marks the conversation's summary, whose `panelRevision` is the
panel's revision, so every page that shows the conversation reads the panel
when the revision is higher than its own
([Page synchronization](web-application.md#page-synchronization)). The backend
applies the changes of a conversation one at a time, the page's and the
plugin's alike, so `revision` orders them all.

A request's changes apply all or none: a change that is refused refuses the
request with its status, and the panel stays as it was. A panel holds at most
64 tabs and 64 KiB of `data` in all: a create past either answers 409
`panel_full`, and an update past the size 413 `too_large`. A create of a `kind` no plugin the user has on declares answers
400 `unknown_panel_kind`. Archived conversations allow the read and refuse
every change with 409 `conversation_archived`.

## Conversation drafts

The backend keeps one draft per conversation: the message its composer holds
before it is sent. For example, a user types "Fix the login" into the
composer in one tab and drops `trace.txt` into it. Every other page that shows
the conversation, in another tab or on the user's phone, shows the same text
and the file's capsule moments after each save, and any of them can send the
message. [Drafts](web-application.md#drafts) says when a page saves and what
it shows.

A draft is `{ revision, text, files, comments, replaced }`:

| Field | Meaning |
| --- | --- |
| `revision` | How many times the draft changed: 0 before its first save, one more with every save, restore and dismissal |
| `text` | The message's Markdown, with the attachment mark U+FFFC where each file's capsule stands |
| `files` | The files in the order of their marks: an upload as `{ type: "upload", ref, fileName, mediaType, sha256, snippet? }`, the last three from its record, or a file on a paired device as `{ type: "remote_file", deviceId, path }` |
| `comments` | The waiting comments, in the order they were saved, which the page shows and sends in reading order, by block and then `at`, where the comment stands in its block's text: a comment on words has its `start` there, a comment on a picture the length of the block's text before the picture, and a picture comes before words at the same place; a comment on a picture also carries `picture`, a `MediaSource`, and its `start` is the picture's place among its block's pictures, counting a picture still loading, `end` one more: `{ id, blockId, start, end, quote, text }`, the reply's text block, the selection's character offsets in its rendered text, the text it quotes and the comment ([Comments and side questions](comments-and-ask.md)) |
| `replaced` | The version a save replaced without having been built on it, `{ revision, text, files, comments }` with the revision it had as the draft, or null |

`GET /api/conversations/:id/draft` answers `{ draft }`; a conversation that
never saved one has revision 0, empty text, no files and no replaced version.

`PUT` takes `{ base, text, files, comments }` and answers `{ draft }` as saved. `base` is
the revision the page's text was built on. The request names an upload as a
frame does, `{ type: "upload", ref, fileName }`, and a remote file as the
frame does; the backend adds the upload's media type, content hash and
snippet from its record, and an upload the caller does not have answers 404
`upload_not_found`. A text without exactly one mark per file answers 400
`invalid_body`. The draft as stored, text and files, is at most 256 KiB; a
larger one answers 413 `too_large`.

A save always takes effect: the last save wins. When `base` is older than
the revision at which the draft's text was written, the page never showed
that text, and the save keeps it as `replaced`, unless it is empty or the
same as the saved one. A dismissal, or a save that repeats the text, changes
the revision and not the text, so it makes no page's next save replace
anything. Only one replaced version is kept, so an earlier one goes. For
example, two tabs show revision 4, "Fix the login", and their users type at
the same time:

1. Tab A saves "Fix the login bug" with base 4, which becomes revision 5.
2. Tab B saves "Fix the login test" with base 4, older than revision 5, at
   which the draft's text was written, so the backend keeps revision 5 as
   `replaced` and saves B's text as revision 6.
3. Every page shows "Fix the login test" and offers "Fix the login bug" for
   restore.

`POST /api/conversations/:id/draft/replaced { action, revision }` acts on the
replaced version whose revision it names. `restore` exchanges it with the
draft's text and files, so the version it displaces becomes the replaced one
and nothing is lost: in the example, the draft becomes "Fix the login bug" as
revision 7, and revision 6 is offered in its place. `dismiss` drops it. Each
answers `{ draft }`; when the replaced version is no longer the one named,
because another save or action changed it, the answer is 409 `draft_changed`.
Otherwise the replaced version stays until a later save replaces a version:
a send, an archive or a reload keeps it.

Other pages learn of a change from the conversation's summary, which their
synchronization channel brings once the change commits and which carries the
draft's revision as `draftRevision`
([Page synchronization](web-api-sync.md#page-synchronization)); they read the draft only
then. A page takes a draft only when its revision is higher than the one it
holds, since a save's answer and a read can reach it in either order. An
archived conversation reads its draft and refuses the other operations with
409 `conversation_archived`; after a restore it has the draft it had. A draft
lasts as long as its conversation, and a Fork starts with an empty one
([Conversation Fork](../agent/conversation-fork.md)).

## Messages saved for later

A conversation keeps the messages its user saved for later
([Saved for later](conversations.md#saved-for-later)), newest first. The saved
messages and the draft are one piece of state to the page
([Responding to the user](web-application.md#responding-to-the-user)): every
answer of a route below and of the draft's `PUT` carries `revisions`,
`{ draft, saved }`, the two revisions after it. A saved
message is `{ id, text, files, savedAt }`, its `text` and `files` as a
draft's are ([Conversation drafts](#conversation-drafts)).

- `GET /api/conversations/:id/saved` answers `{ revision, messages }`.
- `POST /api/conversations/:id/saved/from-draft` takes `{ id, base, text,
  files }`, `id` the page's UUID for the new message, and `base`, `text` and
  `files` as a draft's `PUT` takes them, the composer's message: it saves them
  as a message for later and empties the conversation's draft in the same
  transaction, so the message is never in both places, and answers
  `{ message, draft }` with 201.
- `POST /api/conversations/:id/saved` takes `{ id, text, files }` and adds them
  as a message for later, leaving the draft as it is, as Undo after Delete
  does; it answers `{ message }` with 201.
- Either save with an id the conversation already has answers as the first
  save did when its content is the same, and 409 `id_taken` otherwise, so a
  save sent again never makes two. A text without exactly
  one mark per file answers 400 `invalid_body`, a message over 256 KiB 413
  `too_large`, and a conversation with 200 saved messages 409
  `saved_full`.
- `POST /api/conversations/:id/saved/:messageId/edit` takes `{ op, id }`,
  `op` the page's UUID for this edit and `id` its UUID for the message the
  draft becomes, and moves the message into the conversation's draft, and
  the draft's text and files, when it holds any, into a message saved for
  later under `id`, in one transaction, and answers `{ draft, revisions }`.
  The backend keeps the answer by `op` for a day, so the same edit sent
  again answers as it first did, whatever happened since.
- `DELETE /api/conversations/:id/saved/:messageId` removes one and answers
  `{ revisions }`, and a message already gone answers the same; Undo saves
  it again with `POST` under a new id, its uploads kept as the composer's
  are kept until saved ([Blob references](../backend/storage.md)).
- Sending one is the `send_saved` frame on the conversation socket, `{ id,
  turnId, delivery }` with `turnId` the page's UUID for this send, the id of
  the message it becomes, never the saved message's, and `delivery` the
  composer's choice: the session sends it as
  the composer would, and the message leaves the list when the session has
  accepted it, in the same step, so it is never both sent and kept, nor sent
  twice, and a send the session refuses leaves it on the list
  ([Client frames](../agent/rendering.md#client-frames)).
- Undo after Delete saves the message again with its own id, at the top of the
  list. Saving in a conversation that has no record yet creates the record,
  as its first send does.
- An id the conversation has no saved message of answers 404
  `saved_not_found`.

The conversation's summary carries `saved`, the count, and `savedRevision`,
which every change raises; a page that shows the conversation reads the list
again when the revision it holds is behind.

## Side questions

A conversation's [side questions](comments-and-ask.md#side-questions) reach a
page by revision, as its permission requests do. The conversation's summary
carries `sideQuestionsRevision`, which rises when one is asked, titled or
folded; a page that shows the conversation reads the list again when it
rises:

```text
GET /api/conversations/c_81/side-questions

200 { "revision": 4,
      "sideQuestions": [ { "id": "sq_7f2", "blockId": "b_19", "start": 112, "end": 151,
                           "quote": "All API calls share one client",
                           "question": "Is that really so?",
                           "title": "Shared API client timeout",
                           "folded": false, "askedAt": "2026-10-18T08:12:40Z" } ] }
```

The list is in the order asked; `title` is null until the small model names
it. A side question about a picture carries `picture`, a `MediaSource`,
null for words; its POST carries it too. `POST /api/conversations/:id/side-questions` with `{ id, blockId, start,
end, quote, question }` asks one, the page choosing the `id`, and answers 201
`{ sideQuestion }`; the same request again answers 200 with the same one, so
a page that lost the answer sends it again safely. `PATCH
…/side-questions/:side` with `{ folded }` answers 200 `{ sideQuestion }`.
`POST …/:side/messages` with `{ text }` asks a follow-up and `POST
…/:side/retry` retries a failed turn, each answering 204.

A side question's answers are its transcript, read by its pages with `GET
…/transcript?node=<side>` and followed live in the frames a subagent's
transcript comes in, `subagent_transcript_reset` and `_patch`, `phase`,
`pending_calls`, `waiting` and `context_usage`, keyed by the side question's
id; it has no `subagent` frame and no entry in `GET …/subagents`
([Subagents and commands](#subagents-and-commands)).

| Status | Code | When |
| --- | --- | --- |
| 404 | `conversation_not_found` | The conversation does not exist or is not the user's |
| 409 | `conversation_archived` | The conversation is archived |
| 404 | `side_question_not_found` | The conversation holds no side question with that id |
| 409 | `side_question_conflict` | A side question with that id was asked with other values |
| 400 | `invalid_side_question_target` | The block is not a finished text block of the agent's reply, or the offsets fall outside it |
| 409 | `side_question_refused` | The side question cannot start now, as when its Host cannot be reached, with the cause |

## Conversation permissions

A conversation's [permission requests](../agent/permissions.md) reach a page
by revision, as a draft does. For example, an agent of `c_81`
runs `demi skills add` without the grant. Every page of the user receives the
conversation's summary with `permissionRequests: 1` and a higher
`permissionsRevision`, and a page that shows `c_81` reads:

```text
GET /api/conversations/c_81/permissions

200 { "revision": 12,
      "requests": [ { "id": "pr_3k9",
                      "categories": [ { "id": "skills.manage", "action": "manage skills",
                                        "description": "..." } ],
                      "command": "demi skills add vercel-labs/agent-skills --skill web-design-guidelines",
                      "agent": null, "createdAt": "..." } ] }

POST /api/conversations/c_81/permissions/requests/pr_3k9
{ "decision": "allow" }

204
```

| Route | Body | Does |
| --- | --- | --- |
| `GET /api/conversations/:id/permissions` | None | Returns `{ revision, requests }`: the undecided requests, oldest first, each with its categories, one or none, and `host`, the host it asks to reach (a request holds one or the other) as `{ kind: "device", deviceId, name }` or `{ kind: "pairing" }` for a pairing, or null ([Hosts](../agent/permissions.md#hosts)), and its agent as null for the root or `{ number, description }`; a category no longer in the user's command set has its `id` alone |
| `POST /api/conversations/:id/permissions/requests/:request` | `{ decision: "allow" \| "deny" }`, and for a credential request `credential: { values }` with an allow | Decides the request ([Requests](../agent/permissions.md#requests)), sends its message to the agent, and answers 204 |

Each request carries its `kind`: `permission`, with its categories as above;
`credential`, a [credential](../agent/credentials.md) the agent asks the
user to save, with its name, reason, fields `{ name, label, kind }`, the
`keep` the agent chose and `renewal`, true when it asks new values for a saved
credential; or `allowance`, a stored credential the conversation asks
to use, with its name. `keep` is `1h`, `1d`, `30d` or `always`; `values` maps
each field's name to its value, every field required.

A request that is no longer undecided, because another page decided it, an
Allow of its categories decided it, or a newer request replaced it, answers 404
`permission_request_not_found`; the page shows the outcome from the
conversation's summary. An archived conversation has no requests and
refuses a decision with 409 `conversation_archived`. The page takes an answer only when its revision is
higher than the one it holds, since a read and the summary can reach it in
either order.

## Credentials

The user's [credentials](../agent/credentials.md), which only their owner
reads and changes:

| Route | Body | Does |
| --- | --- | --- |
| `GET /api/credentials` | None | `{ credentials }`: each `{ id, name, note, fields: [{ name, label, kind, value? }], keepUntil, lastUsedAt, createdAt, updatedAt }`, a visible field with its `value` and a secret one without, `keepUntil` null for always |
| `GET /api/credentials/:id/values` | None | `{ values }`, every field's value, for Settings' reveal; it is logged by name, never by value; a session not verified in the last five minutes answers 403 `verification_required` |
| `POST /api/credentials` | `{ name, note?, fields: [{ name, label, kind, value }], keep }` | Saves a new one; a taken name is 409 `credential_name_taken` |
| `PATCH /api/credentials/:id` | Any of `name`, `note`, `fields` (the whole list, a field's `value` left out to keep it), `keep` | Changes it |
| `DELETE /api/credentials/:id` | None | Removes it and its allowances |

A name has 1 to 80 characters after trimming, unique among the user's
credentials whatever its case (*GitHub token* and *github token* are one
name), a field name 1 to 32 of
lowercase letters, digits, `_` and `-`, a label 1 to 80, a value up to 64 KiB,
and a credential one to ten fields. A change reaches the user's other pages
through the synchronization channel's `credentials` part, which carries a
visible field's value and never a secret's. A save, rename or removal also
raises the permissions revision of each conversation that has a credential
request of that name waiting, so its card's `renewal` follows. Each error's `message` is a sentence for the user, which the page shows as
it is: *A value can be at most 64 KiB.*, *“GitHub token” already has other
fields; answer with the same fields or choose another name.* Errors: 400 `invalid_body` for a malformed body, a new field without a
value included; 400 `invalid_answer` for a credential request's answer whose
values do not match its fields; 404 `credential_not_found`; 409
`credential_name_taken`; 409 `credential_fields_differ` for an answer that
would renew a saved credential with other fields; 409 `credential_removed`
for an allowance of a credential removed since, which the request lists with
`removed: true` and which only a deny decides.

## Workflows of a directory

`GET /api/conversations/:id/workflows` answers the workflows of the
conversation's working directory, and `GET
/api/devices/:deviceId/workflows?directory=<absolute path>` those of a
directory on a device the user owns, for a new conversation before its
record exists ([Workflows of a project](../agent/subagents.md#workflows-of-a-project)).
Both answer `{ directory, unavailable, workflows, watch }`: the directory
the workflows were found in, or null; `unavailable`, the reason the whole
directory cannot be used, such as a link, or null; each workflow as
`{ id, name, path, unavailable }`; and `watch`, the folders the page follows
on its file watch to learn of a change. The answer carries the Host's read
number in `X-Demi-Read-At`, as file reads do. Both take an optional
`readAfter=<epoch>.<number>`: the backend answers what it keeps when its
search read at that number or later, and otherwise searches again and
answers that. While its watch of `watch` is live, the page sends that
watch's highest `since`, so an answer it holds that was read before the
watch went live is read once more. The backend's kept answer stays current by the backend's
own watch, but its number is the one its search took, which is older than a
watch the page starts later; without `readAfter`, the page would get the
same answer and number back and ask again without end. The device route wakes
nothing and answers 409 `device_offline` while the device's runner is not
connected; a directory that is not an absolute path answers 400
`invalid_body`, one that does not exist 404 `fs_not_found`, and another Host
error 400 `fs_error`. The conversation route wakes nothing either: while the
backend holds no search of the directory, as when the conversation's Cloud
is stopped or not made yet, its device is offline, or its first search has
not finished, it answers 200 with `{ directory: null, unavailable: null,
workflows: [], watch: [<the working directory>] }` and no
`X-Demi-Read-At`, so the menu offers Global Workflow and No Subagents, as for
a new conversation whose device is offline; the page follows the working
directory on the conversation's file watch, and the watch going live, the
Host's own announcement that it runs, makes that answer out of date, so the
page reads again with that watch's `since`. A search that failed for another
reason answers its own error, never one that names a stopped Cloud. `POST
/api/conversations` takes an optional `workflow`, the new conversation's
choice, absent for Global Workflow, and the conversation's PATCH takes `workflow` to
change it.

## Sidebar mutations and read state

The backend applies the fields of `PATCH /api/conversations/:id` independently:
`title` (trimmed, 1 to 256 characters), `archived`, `pinned`, `target` with
`notifyAgent`, true to tell the agent of the switch
([Switch the primary target](../execution/sessions-and-targets.md#switch-the-primary-target)),
and the model settings: `model`, the provider entry and model as
`{ providerId, modelId }`, `thinkingEffort` and `serviceTierId`. A patch whose
only field fails answers that field's status with `{ code, message }`;
otherwise the answer is the current conversation with
`results: [{ field, status, code?, message?, httpStatus? }]`, 200 when every
field applied and 207 when any was refused. Unexpected operation failures are
reported as 500 field results. Applied fields remain applied. Archive is
evaluated before the other fields, so archiving and renaming together archives
successfully but refuses the rename. A rename follows [Conversation
titles](conversations.md#conversation-titles). `POST /api/conversations/batch` accepts
up to 100 `{ id, patch }` items and returns 207 with an outcome per item:
`{ id, status: "updated", conversation, results }`, or
`{ id, status: "refused", code, message }` for a conversation the caller does
not have.

A conversation's model settings are one value, the model selection its record
holds ([A conversation's model settings](../providers/models.md#a-conversations-model-settings)).
The conversation lists carry the settings as `model`,
`{ providerId, modelId, thinkingEffort, serviceTierId }`, or null while the
conversation has no model yet; `thinkingEffort` is null only for a model that
lists no efforts. The record also keeps the model's facts. Only a
patch changes the value, and each of its three fields changes one part:
`model` switches to that model, with the effort and the tier the same patch
names, and the new model's first effort and the vendor's default tier for a
part it leaves out; `thinkingEffort`, an effort or `disabled` and never null,
and `serviceTierId`, a tier or null for the vendor's default, set their part
for the conversation's model. The backend
applies one conversation's changes one at a time, in the order they arrive,
each to the value the one before left:

1. It checks each part against the entry's catalog. An entry outside the
   caller's scope answers 404 `provider_not_found`, a model the catalog does
   not list 404 `model_not_found`, an effort or a tier the model does not
   offer 409 `setting_unavailable`, and an effort or a tier for a
   conversation without a model 409 `model_not_selected`.
2. When the conversation's tree is live and the new model belongs to another
   provider entry, it builds the tree's runtime for that entry first, so a
   model that cannot run is refused and changes nothing.
3. It commits the record.
4. It switches the live tree to the new selection, which the tree's next
   provider request uses, inside a running turn too
   ([Model switch](../agent/runtime.md#model-switch)).

A crash before the commit changes nothing. A crash after it leaves the change
in the record, and the tree takes it when it is next opened. A page whose
connection broke before the answer shows the outcome from its synchronization
channel, whichever it was. Opening the conversation's socket names no model,
and the backend opens a tree only with the record's selection, so an open
never changes the settings, whatever the opening page last saw
([Connections and the live tree](../agent/rendering.md#connections-and-the-live-tree)).

Every page shows the record's value. The page that made a change shows it from
the patch's answer at once, and every other page, in another tab or on another
device, from the conversation's summary, which its synchronization channel
brings once the change commits ([Page synchronization](web-api-sync.md#page-synchronization)).
A page keeps no model settings of its own for a conversation with a record,
and never sends the value back: a change names only the part its user
changed, so a page that has not yet seen another page's change cannot undo
it. Two changes of one part end with the one applied last,
and changes of different parts both stay. A new conversation keeps its
settings in the user's browser until its first send writes them to its record
([User preferences](web-api-settings.md#user-preferences)), and a change on a page of a
conversation whose record has no model yet writes the whole value.

Archive and a target change are transitions: each holds the conversation while
it runs, and neither waits for other work. Running root or child work refuses
archive with 409 `turn_in_flight`, and so does another transition, an
asynchronous frame admission or a Host operation in progress, such as reading
a file or opening a tab in the conversation browser. File transfers, user
streams and the [conversation browser's tab methods](../browser/live-view.md#the-tab-methods)
that do not wake a stopped Cloud do not refuse it: archive ends them instead
([Host operations](../execution/sessions-and-targets.md#host-operations)). A
target change refuses and ends the same way
([Switch the primary target](../execution/sessions-and-targets.md#switch-the-primary-target)).
A target that names a workspace or a device the user does not have answers
404 `workspace_not_found` or `device_not_found`, and a target change that
another one overtook answers 409 `target_conflict`.

A held conversation is waiting, not failed
([held conversations](../execution/sessions-and-targets.md#how-a-conversation-uses-a-device)).
A title, pin or model change that arrives while an archive, a target change or
a Cloud reset holds the conversation waits for it to end and is then applied
as if it had arrived afterwards; only the request's own cancellation, such as
the page closing the connection, ends the wait early. A frame waits the same
way, and the socket's later frames queue behind it in order. The transport
waits for frame handling to finish before releasing admission; it does not
wait for an entire inference turn.

Archived conversations allow transcript reads and read acknowledgements;
stream upgrades, frames on an open socket, attachment delivery, host changes
and metadata edits are refused until restore, with 409
`conversation_archived` where the refusal is an HTTP answer. An upgrade of
the stream from a page that is not the product's answers 403
`forbidden_origin` ([Authentication](web-api.md#authentication)), and a request to the
stream that is not a WebSocket upgrade 426 `upgrade_required`.

`DELETE /api/conversations/:id` deletes a conversation, archived or not, and
answers 204; one the caller does not have answers 404
`conversation_not_found`. Nothing refuses a deletion. It is a transition that
stops the conversation's work as [Stop](../agent/runtime.md#stop) does, its
subagents and commands included, ends its streams and Host operations as
archive ends them, and then removes the conversation
([Deleting a conversation](../backend/storage.md#deleting-a-conversation)).
A request about the conversation that arrives once the deletion has begun
answers 404 `conversation_not_found`. The synchronization channel drops the
conversation's summary, and a page that shows the conversation goes to a new
conversation.

`POST /api/sidebar/reorder` takes `{ kind: "conversation" | "workspace", id,
beforeId: string | null }`; null appends, and a success answers 204.
Conversation moves stay within the same project and pin partition, and a
workspace moves among the user's workspaces; a move out of them, or of a row
that is archived, answers 409 `invalid_order`. A move that names, as the row or
as the one it goes before, a conversation or a workspace the caller does not
have, as one another page deleted meanwhile, answers 404
`conversation_not_found` or `workspace_not_found`.
[Storage](../backend/storage.md#control-records) owns persistent ordering.
Activity timestamps never reorder rows.

`GET /api/conversations?archived=true|false` includes `status`, `revision`,
`readRevision`, `unread`, and `cwd`, the directory the conversation's work runs
in, resolved the same way for a device directory, a workspace, and the Cloud, so
the web app never derives it. Status is running/compacting from the live agent
tree, otherwise completed/error/stopped from its latest terminal block, or idle.
An unfinished checkpoint without a live session is interrupted. `pluginsChanged` says whether the conversation's tree is open
with commands of plugins the user has since turned on or off, so a reload
would change them ([Reload](web-api-sync.md#a-users-plugins)). `draftRevision` is
the revision of the conversation's
[draft](#conversation-drafts), 0 before its first save: the page reads the
draft itself only when this number is higher than the revision it holds, so a
summary carries no draft's text. `hostsRevision` is the revision of the
conversation's [attached hosts](../execution/sessions-and-targets.md#attached-hosts),
which an attach, a detach or a rename raises, read by the page the same way. `panelRevision` is the revision of the
conversation's [work panel](#work-panel-state), 0 before its first change,
read the same way. `lastAttachment` is the number of the conversation's
latest [attachment](../execution/commands.md#attachment-commands), 3 for
`a3`, 0 before its first; an upload raises it, and the backend sends the
summary as it stores the attachment, so a page learns of a new one
([Files named in messages](file-previews.md#files-named-in-messages)).
`pluginRevisions` is a revision like `panelRevision` for each
plugin's [conversation state](web-api-sync.md#conversation-state-of-plugins), as
`{ plugin, revision }` for every plugin that declares one, in registration
order, so turning a plugin on or off changes no summary. What changes in the
conversation's files is not in the summary: the Host reports it on the
[file watch](web-api-hosts.md#file-watch).
`lastTurn` is the root's latest ended turn, `{ id, outcome }`, or null
before the first: `id` is the block that ended it, so a turn that Resume or
Regenerate takes up and that ends again is a new end; `outcome` is
`finished`, `failed` or `stopped`. A failed compaction ends no turn. The
backend [notifies](turns.md#notifications) from it: when the tree comes to
rest naming a `lastTurn` that ended after the tree started working, it
pushes that turn's end, so a turn that Resume takes up notifies once, when
it ends again.
`permissionRequests` counts the conversation's undecided
[permission requests](#conversation-permissions), which the conversation's
card and the Running window follow, `unreadPermissionRequests` those of them
not read yet, which the sidebar's needs-you mark follows: the read
acknowledgement (`POST /conversations/:id/read`) reads them too, and a new or
replacing request starts unread. `permissionsRevision` rises with each change of its
requests; a page that shows the conversation reads them when it is newer
than the revision it holds. `pluginRevisions` and
`permissionsRevision` are [counted in memory](web-api-sync.md#revisions-counted-in-memory).
A conversation is running while its tree will go on working without the user: an
agent of the tree is acting, or a child is still open. An agent acts until the
save that ends its action commits, which is also when the page sees its phase
go idle ([A turn](../agent/runtime.md#a-turn)), so once a conversation no
longer runs, its transcript route shows what the live tree showed. An open child counts
whatever it is doing, a wait for its own commands included, because it
resumes by itself and its close wakes its parent
([Subagents](../agent/subagents.md#result)). A shell command that outlives its
turn does not count: until it reports, nothing works, and a resident one such
as a dev server may never end; its report starts the agent again, as a
message would, and the conversation runs again then
([Command reports](../agent/shell-tool.md#command-reports)). For example, the root answers "two children are looking into it" and
ends its turn: the conversation stays running until both children close and the
root has answered their results. The root answers "the build has started" while
`npm run build` still runs: the conversation is completed, and the command shows
as running only on its terminal tab, until the build's end reports to the
root and the conversation runs again for its answer. `web-ui` applies the same rule to a
conversation the page is attached to.

Checkpoint output changes advance a persisted revision; user input alone does
not. A page sends `POST /api/conversations/:id/read { revision }` for the
output it actually showed. Acknowledgements only move forward, and revisions
beyond current output are refused.

## Sharing

A share is a read-only view of a conversation or of one answer, behind a
random link ([Sharing](sharing.md)).

`POST /conversations/:id/shares { kind: "conversation" }` or `{ kind:
"answer", blockId }` answers `{ token, url, kind, blockId?, takenAt }`: the
existing share when there is one, a new share otherwise. `blockId` must be
a `forkable` text block of the conversation's own agent, else 400
`share_target_invalid`; the page offers Share only on the text that ends a
reply, and the backend does not judge which text does. The conversation's
header and an answer read whether a share exists from the conversation's
`shares` in the product state, `{ token, kind, blockId? }` each, so the
dialog opens on the existing link. `PUT /conversations/:id/shares/:token`
makes the share's list anew under the same token and answers as
`POST`; an answer's share answers 409 `share_not_updatable`. `DELETE` removes
the share. An archived conversation can be shared and unshared.

The public routes take no session and ignore one:

| Route | Answers |
| --- | --- |
| `GET /shares/:token` | `{ kind, title, takenAt }`, for its header |
| `GET /shares/:token/transcript?before=\|after=\|around=` | A page of what the share holds, as [Pages](#pages) answers it, `instructions` always empty and `failures` absent; a step entry reads as `{ type: "steps_not_shared", count }` in its place |
| `GET /shares/:token/transcript/blocks/:blockId` | One block whole, as the conversation's whole-block read answers it |
| `GET /shares/:token/blobs/:sha256?type=` | A blob the share shows |
| `GET /shares/:token/attachments/:attachment` | An attachment the share's text names as `attachment:a3` and that existed when it was taken, as [the conversation's attachment route](web-api-hosts.md#uploads-and-media) answers it; one uploaded later stays missing until Update Link |

A token that names no share, a node or block the share lacks, and a blob
it does not show answer 404 `share_not_found`, the same for each, so the
answer tells nothing of what exists. Every public answer carries
`X-Robots-Tag: noindex, nofollow`, `Referrer-Policy: no-referrer` and
`Cache-Control: private, no-store`, so a removed share is not served from a
cache. The web app build serves the shared page at `/s/:token`.

## Search

`GET /api/search?q=<query>` answers `{ results }`: the caller's conversations
that match the query, at most 50, in the order
[Finding a conversation](conversations.md#finding-a-conversation) gives. A result is
`{ conversationId, title, titleRanges, archived, lastActiveAt, match }`, where
`titleRanges` are the ranges of the query's words in the title, as `match`
gives them below, empty when the title does not match, and `lastActiveAt` is
when the conversation's latest message was written. `match`
is null when only the title matches, and otherwise `{ blockId, text, ranges }`:
the block of the matching message, newest first when several match, a line of
its text of at most 160 characters around the first match, and the ranges of
the query's words in that line as `[start, end]` offsets in UTF-16 code units,
end exclusive, which the page marks. The query is trimmed and has 1 to 256
characters; another answers 400 `invalid_query`. The search window sends no
empty query: it shows the most recent conversations from the product state
until the user types.

The backend answers from the caller's [search index](../backend/storage.md#search-index)
alone: it opens no conversation's database and wakes no Host.
