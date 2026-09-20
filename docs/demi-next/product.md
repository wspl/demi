# Demi Next: Product Design

Demi is a browser application for conversations that can run commands on Cloud
or the user's connected machines. Conversation history belongs to the backend;
project files belong to execution devices. Changing a conversation's target
changes where subsequent work runs without moving its history or copying files.

## Instance mode: shared vs isolated

`DEMI_INSTANCE_MODE` selects provider ownership at startup. The product cannot
change this deployment setting.

| Mode | Provider configuration | Model use | Usage visibility |
|---|---|---|---|
| Shared | Master and admins manage instance-owned entries | All users use the instance's entries | Each user sees their own usage; admins can see instance usage by user |
| Isolated | Each user manages their own entries | Each user uses their own entries | Each user sees their own usage |

Conversation, device, workspace, and attachment ownership remains per user in
both modes. Administrator status does not grant access to another user's
conversations or files. Shared usage reporting and account administration are
explicit exceptions to owner-only product records.

A provider is a configured entry with a stable ID and label. Model selection uses
`(providerId, modelId)`, so two entries can offer the same vendor model under
different accounts or endpoints. Subscription entries can contain multiple
accounts with one selected account. Credential and catalog rules belong to
[Providers](providers-and-vault.md).

## User system

Accounts use email and password; nickname is a separate display value. The first
account is created by instance setup. Administrators create subsequent accounts;
there is no public registration or password-recovery flow.

| Role | Account administration |
|---|---|
| Master | Creates admins and users; resets passwords of admins and users |
| Admin | Creates users; resets user passwords |
| User | No administration of other accounts |

A role can act only on lower roles. No role can reset a peer's password through
the admin endpoint. Everyone can change their own password by providing the
current password, edit their nickname, and change email after verifying a code
sent to the new address. Setup and account creation do not require email delivery;
email change does, and reports an unavailable mail service explicitly.

Browser HTTP and conversation WebSockets use the same session cookie.
[Backend authentication](backend.md#authentication-and-ownership) owns session
and lockout behavior; [Web API](web-api.md#account-api) owns account request
shapes and verification limits. There are no organizations or team roles.

## Conversations and projects

A conversation owns one agent tree, including its root and subagents, plus
product metadata such as title, target, model selection, archive state, pin and
read state. A project is a named directory on a device; the API calls it a
workspace. It groups conversations without creating a separate execution machine.

```text
User
+-- Cloud device
|   +-- project directory A <- conversations 1, 2
|   +-- project directory B <- conversation 3
+-- connected laptop
|   +-- project directory C <- conversation 4
+-- ungrouped conversations -> Cloud or a direct device directory
```

New conversation opens an immediately typeable draft with Cloud as its default
target. It inherits the user's last explicit model, thinking effort, and service
tier, including a choice made in an empty unsent draft. These defaults are backend
user preferences; existing conversations keep their own selections. If a saved
choice becomes unavailable, the picker keeps it with a warning and requires an
explicit replacement. With no saved choice, the first available model is used.

The first send creates the backend record and starts its
[title](#conversation-titles). Draft persistence and confirmation of
uncertain sends belong to [Web architecture](web-application.md). Choosing a
project or device directory affects subsequent execution; target-switch admission
and context announcements are defined in
[Sessions and targets](sessions-and-targets.md).

Conversations can be archived and restored, but not deleted. Archiving is refused
while root or child work or conflicting operations are active. Archived history
remains readable; sending and metadata changes require restore. Persistent ordering
is independent of activity; its storage rules belong to
[Storage](storage.md#control-records).

The conversation interface exposes steering, queued messages, stop,
[recovery of an unfinished turn](#recovering-an-unfinished-turn), manual
compaction, model switching, message editing, Fork, and child/terminal
inspection. [Message editing](../message-editing.md) and
[Conversation Fork](../conversation-fork.md) define their history boundaries.
Interactive stdin is an agent-protocol capability; exposing a terminal input
control remains separate from read-only job inspection.

### Recovering an unfinished turn

A turn can end without finishing in two ways, and the agent can go on from
either without the user typing anything.

- **Something broke.** The provider refuses the request after the automatic
  retries give up, or the agent session is shut down under a running turn: a
  backend restart, a crash, a release. Every one of these is an error: the
  transcript gets an error record that says what happened. A provider failure
  carries the provider's own words; a failure of Demi's own carries Demi's
  fact: "The agent session was shut down while this turn was running." A graceful
  shutdown writes the record as it ends the turn; after a crash the restored
  conversation writes it when it is next opened. No turn ends unfinished
  without either such a record or the user's own Stop.

  A Host that goes offline does not end a turn. The operation that needed it
  fails with the runner's offline error
  ([Host operations](sessions-and-targets.md#host-operations)), the tool call
  shows that error, and the agent goes on and says what it could not do.
- **The user stopped it.** Stop is a decision, not a failure, and leaves the
  stopped marker it leaves today.

The transcript says what happened; the place to act is above the composer.
The error record leads with what its source said, keeps the facts and Copy,
and carries no button.
One recovery control sits in the dock, directly over the input, where the
user's next action already is:

```text
  ... transcript ...
  [x] The provider request failed
      Insufficient balance. Manage your billing here: ...
      HTTP 401 · req_01J8...

  [ > Resume ]   [ 2 Running ]  [ 1 Agent ]        <- the dock's chips
  +--------------------------------------------------------------+
  | Ask Demi...                                  GLM-5.3-Flash   |
  +--------------------------------------------------------------+
```

Its label follows the cause, because the user's expectation differs:

| How the turn ended | Control | What the user expects |
| --- | --- | --- |
| An error ended it: a provider failure, or a shutdown or crash under the turn | **Resume** | The same step again: nothing was decided, something broke. |
| The user pressed Stop | **Continue** | The agent goes on with the work the user cut short. |
| It finished | none | |

Both are the same operation, the session's `resume`: it unwinds to the turn's
resume point, keeps everything that already left the process, and infers
again from there. It adds no user message and never reruns a completed tool
effect. The label is presentation only.

The control is offered when the last turn is unfinished and the conversation
is idle, not archived, has a usable provider, has its history loaded, and no
message edit is open. It leaves when recovery starts, and for good once a
newer turn exists: sending a message is the other way forward, and the old turn
is then history.

While a turn waits for the provider, the transcript's tail row says
**Requesting**, with how long it has waited. The word says whose the wait is:
a slow model must never read as something broken in Demi. Recovery shows the
same row from the moment the control is used; there is no separate resuming
state. While the agent retries a failed request on its own (a rate limit or an
overload, within the retry policy of
[Provider errors and retries](../provider-errors-and-retries.md)), the row says
**Retrying** instead, from the failed attempt until the retry produces output
or the turn ends. Retrying has no control; the Resume control appears only
after the automatic retries give up.

Two other retries are not this and keep their own places: reloading a history
that failed to load, and resending a message whose delivery is unconfirmed.
They recover reading and delivery, not a turn.

### Conversation titles

A user opens a conversation with "why does `pnpm build` fail with TS2307 after
I moved auth into its own package". At once the sidebar shows the start of
that message; a few seconds later it shows "pnpm build TS2307 after auth
package move", written by the model the user picked for the conversation.

```text
first send ──► title = start of the message (80 characters)      origin: message
           └─► one model request, beside the first turn
                    │ a usable line comes back, and the origin is still `message`
                    ▼
               title = the generated line                         origin: generated

rename at any time ──► title = what the user typed               origin: user
```

Every title records its origin, one of `placeholder` ("New conversation",
before any send), `message`, `generated`, and `user`. The generated title is
written only while the origin is `message`, in the same statement that checks
it, so a rename that lands while the request is in flight wins and is never
overwritten. A rename is a title that differs from the current one; a patch
that repeats the current title, as the browser's record creation does with
the placeholder, changes no origin. A Fork's title, the source's with " (Fork)", has origin `user`:
it is already a settled name and is not regenerated.

The request:

| Aspect | Rule |
| --- | --- |
| When | Once per conversation, at the first send, concurrently with the first turn. It neither waits for the turn nor delays it. A first message without text starts none, and the title stays the placeholder until renamed. |
| Model | The provider and model selected for the conversation at that send. No separate title model is configured. The first message therefore goes only where the user already chose to send it. |
| Effort | The lowest thinking effort the model offers, the default service tier, and a small output limit. |
| Input | A fixed title instruction as the system prompt, and the text of the first user message, at most its first 4,000 characters. No agent system prompt, no tools, no history, no attachments or images. |
| Path | The same metered provider runtime a turn uses, so the request counts in the user's usage ledger and obeys the same limits. It is not a session turn: nothing is added to the transcript and no session event is emitted. |
| Output | The first non-empty line of the text response, without surrounding quotes, cut to 80 characters. Thinking output is ignored. An empty result writes nothing. |
| Failure | A refused, failed or empty request is logged and ends there; the message-derived title stays. There is no retry: the title in place is already usable. |
| Release | The request is aborted when the conversation is archived or the backend closes. |

The instruction tells the model to produce a title, never an answer: one line
that fits where it is shown, in the language of the message. The instruction
describes the place rather than listing a length per language: one line of a
narrow sidebar about 24 columns wide, where a Latin character takes one column
and a Chinese, Japanese or Korean character two, and the overflow is cut off;
examples show the result. It also asks for natural grammar,
exact technical terms, file names, numbers and error codes kept, no tool
names, no leading "the" or "my", and something meaningful even for a greeting.

The browser learns the new title the way it learns a rename made elsewhere,
from the next [state snapshot](web-api.md); there is no title event.

Not included: regenerating a title on request, retitling after the first
message is edited, and titling from a different, cheaper model. The last is a
natural extension once instance or user configuration names such a model.

Rationale: a request beside the turn, rather than a clone of the session the
way [compaction](../compaction-context-cache.md) summarizes, keeps the agent's
system prompt, tools and history out of a job that needs one message, and
keeps the title independent of whether the first turn succeeds.

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
it opens it the same way ([Message editing](../message-editing.md)).

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
| `[text](target)`, a bare `http` or `https` URL | A link, resolved as in [Files named in messages](file-previews.md#files-named-in-messages) |
| `![alt](target)` | The image, resolved the same way |
| A line break | A line break; a blank line is an empty line |
| `_` and `__`, a single `~`, HTML, `<…>`, math, tables, and lines that start a list, heading, quote, rule or indented code | The characters as typed |

The literal ones are what a conversation about code types as text:
`snake_case`, `__init__`, `~/.zshrc`, `x < y`, `$PATH`, `- item`. Formatting
stays on its line and on its side of a capsule: `**a` and `b**` on two lines
are the characters as typed. Emphasis beside CJK text follows the
[CJK-friendly amendment](https://github.com/tats-u/markdown-cjk-friendly/blob/main/specification.md)
to CommonMark, so `**注意：**这是` is bold as its writer means it.

The model receives the message's Markdown. Where the user typed literally a
character the dialect would read as formatting, it carries a backslash
(`\*args`); nothing else is escaped, so `snake_case` and `x < y` reach the
model as typed. Copying a sent message gives its Markdown, each file by its
name.

Enter sends and Shift+Enter breaks the line; in a code block Enter breaks the
line and ⌘/Ctrl+Enter sends. An input method's Enter never sends. The
composer and the conversation render a user message with one editor,
read-only in the conversation, so a sent message cannot look different from
what was written.

The composer stands one line high while the message fits that line. As soon
as the message needs more — a line break, a code block, an image, or text
wider than the line — the composer opens into a box that wraps the text,
grows to six lines and scrolls beyond them; it closes again once the message
fits one line. No part of a message is ever cut off at the line's end.

## Attachments

Short pasted text stays in the composer and sends as message text. A paste of at
least 2,000 characters or 40 lines becomes `pasted-text.txt`, using the shared
composer's paste thresholds. Dropped, selected, and pasted files use the same
staged attachment flow.

Each attachment is a capsule in the message's text, and that text is the
editor's document. The document is the message: it says which files the
message carries, in what order, and what each one is. Nothing outside it adds
a file to a message or takes one away.

A capsule carries what it shows and what the message sends: the file's name
and kind, its picture when it has one, the device and path of a file that
lives on another device, and the opening lines of a text file. Bytes on their
way to the Host cannot be in a document, so they wait beside it under the
capsule's id: the file itself, how far its upload has come, whether it
failed. That is a transfer, not a fact of the message, and a capsule with no
transfer beside it is a file that needs none — a restored draft, or a sent
message.

What follows from that split:

- A capsule is a character of the message and is edited as one. Backspace
  deletes it, undo brings it back, and it can be dragged to another place in
  the text; it carries no control of its own for any of that.
- A dropped file lands where it is dropped, a picked or pasted one at the
  cursor, and one added from a file browser at the cursor. A file joins the
  message as one change to its document, so there is never a file without a
  capsule or a capsule without a file.
- Deleting a capsule takes its file out of the message and stops its upload.
  The transfer waits, so undo brings the file back with the capsule and the
  upload starts over. What is still waiting goes when the message is sent or
  the composer is left.
- While a file uploads its capsule shows how far along it is, and a failed
  upload offers Retry. Pointed at, a capsule shows the picture larger or a
  text file's opening lines. In the conversation, a click on a capsule opens
  its file in the File view.
- The message's content is read off the document: for the example in
  [Writing a message](#writing-a-message), `Compare `, then `before.png`,
  then ` with `, then `after.png`, then the rest. Providers pass content in
  order, so the model meets each file where the user put it.
- A saved draft keeps the message's text with a mark where each capsule
  stands and each file beside it under its id, so opening the draft builds
  the same document again.

Sending a file proceeds through three owners:

```text
Composer file -> backend upload/blob -> selected Host attachment directory
                         |                          |
                         +-- attachment ID          +-- absolute file path
                                                       in the agent message
```

Files remain staged in the draft until send. The backend writes the file under the Host
user's `~/.demi/attachments/<conversation>/`, adding a numeric suffix to avoid
an existing name. It does not put attachment files in the project directory.
A Cloud target may need to wake before this write.

The agent message contains one `attachment` record with name, path, media type,
size, and blob hash; text files can include a short opening preview, which the
capsule shows when pointed at. Providers render the record as an attachment tag
so the model can read the file with tools. The complete text file is not
duplicated into the message.

Native media adds the corresponding image, video, audio, or document input beside
the attachment record when the selected model supports it. The attachment remains
one capsule. The model's accepted extensions govern selection; an adapter
must not replace supported media with a placeholder. Other files remain
accessible by path. Attachment presence and model capability are separate facts.

The upload cap is 25 MiB. Upload IDs and transcript media references travel in
conversation frames; bulk bytes use HTTP. Exact wire forms, ownership checks,
missing-upload behavior, and browser blob delivery are defined in
[Backend media handling](backend.md#media-by-reference) and
[Web API uploads](web-api.md#uploads-and-media).

A remote-file selection is different from an upload: it names an existing file
on a connected device, and its capsule names the device. Its bytes are read
when the model executes the supplied host command, so it is not a snapshot.
Revocation, disconnect, or file changes can affect that later read.

## Provider management

Models & providers is available to users permitted to configure their provider
scope. The model picker groups models by entry. Adding an entry has three paths:

| Path | User input and result |
|---|---|
| Subscription | Claude setup-token import, or Codex/Grok device login; additional accounts go into the existing family entry |
| Vendor catalog | Select vendor, label, key, and optional endpoint/model overrides; the family is derived from supported vendor metadata |
| Custom endpoint | Select family and protocol, then label, endpoint, key, and model configuration |

Entries can be renamed, edited, or removed. API entries support a manual model
list; clearing it returns to catalog discovery. Labels need not be unique.
Provider configuration stores explicit endpoint and model overrides. Fetched
model directories are cached; they are not a fresh network lookup on every
conversation opening.

The page displays authentication/runtime state, account selection, and available
quota information. Reading status must not run inference. An explicit Test action
can make a real request; a process provider is tried through a conversation on
an execution target. Hiding a provider or model from the picker is a browser
preference and does not stop existing work. Provider protocol, credential,
catalog, and quota details have their authoritative home in
[Providers](providers-and-vault.md).

## Browser scope

The browser provides conversations, account settings, provider management,
devices, [exposes](expose.md#product-surface) in the conversation header, and Cloud controls. Administrative account management and usage have
backend APIs; their dedicated browser pages are deferred. Notifications, MCP,
Skills, data/privacy actions, language switching, and account deletion are also
deferred.

The selected scope excludes public sharing, collaboration, search, offline mode,
PWA behavior, push notifications, and localization. Technology and package
responsibilities belong to [Web architecture](web-application.md). Components,
layout, and interaction examples are maintained in the gallery.

## Cloud settings

Each user has one Cloud device shared by all their Cloud projects. Those projects
can access each other's files; they are not isolation boundaries. Creating or
removing project metadata does not allocate or delete a machine.

Settings shows lifecycle state, storage usage and limits, and Reset environment.
The confirmation explains that reset stops all of the user's Cloud work, replaces
system packages and settings, and preserves home files. Acceptance starts an
operation; the UI follows progress and offers retry of the same operation ID
on failure, including when the guest is offline. Project links and history remain
intact. Reset is a user Cloud action, not a project action.

[Managed hosts](managed-hosts.md) owns lifecycle and durability guarantees.
`web-ui` owns the status and reset interaction, while product and gallery provide
real state or fixtures to that same component.
