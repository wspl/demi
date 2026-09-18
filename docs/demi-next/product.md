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

The conversation interface exposes steering, queued messages, stop, Retry/Resume,
manual compaction, model switching, message editing, Fork, and child/terminal
inspection. Retry/Resume continues the interrupted session; it does not silently
rerun completed tool effects. [Message editing](../message-editing.md) and
[Conversation Fork](../conversation-fork.md) define their history boundaries.
Interactive stdin is an agent-protocol capability; exposing a terminal input
control remains separate from read-only job inspection.

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
of at most 50 characters, in the language of the message, natural grammar,
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

## Attachments

Short pasted text stays in the composer and sends as message text. A paste of at
least 2,000 characters or 40 lines becomes `pasted-text.txt`, using the shared
composer's paste thresholds. Dropped, selected, and pasted files use the same
staged attachment flow.

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
size, and blob hash; text files can include a short opening preview. Providers
render the record as an attachment tag so the model can read the file with tools.
The complete text file is not duplicated into the message.

Native media adds the corresponding image, video, audio, or document input beside
the attachment record when the selected model supports it. The attachment remains
one visible tile. The model's accepted extensions govern selection; an adapter
must not replace supported media with a placeholder. Other files remain
accessible by path. Attachment presence and model capability are separate facts.

The upload cap is 25 MiB. Upload IDs and transcript media references travel in
conversation frames; bulk bytes use HTTP. Exact wire forms, ownership checks,
missing-upload behavior, and browser blob delivery are defined in
[Backend media handling](backend.md#media-by-reference) and
[Web API uploads](web-api.md#uploads-and-media).

A remote-file selection is different from an upload: it names an existing file
on a connected device. Its bytes are read when the model executes the supplied
host command, so it is not a snapshot. Revocation, disconnect, or file changes
can affect that later read.

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
