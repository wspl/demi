# Web API: page synchronization and plugins

How a page follows the user's state over its synchronization channel, and the routes of plugins: a user's plugins, their conversation state and their calls. Conventions every route follows are in [Web API](web-api.md).

## Page synchronization

Each page follows the user's state through one WebSocket, `WS /api/sync`,
rather than asking for it. For example, a user renames a conversation in a tab
on their laptop. Once the rename commits, the backend sends the conversation's
new summary on the channel of every page the user has open: the laptop's
other tabs and the phone show the new title a few milliseconds later, and none
of them asked for anything. The channel carries what a page shows around its
conversations; an open conversation's transcript, phase and commands stay on
that conversation's own [stream](../agent/rendering.md#frame-protocol).

The upgrade requires the session cookie. The channel shows the user's state
to whatever reads it, so an upgrade from a page that is not the product's
answers 403 `forbidden_origin` ([Authentication](web-api.md#authentication)). A request
that is not an upgrade answers 426 `upgrade_required`.

After the upgrade, the backend sends JSON text messages, each a `SyncEvent`.
The page sends two kinds: `{ ping: id }`, which the backend answers with a
`pong` at once, as every page socket of the backend does; and `{ showing }`: the id of the conversation it shows
while it is focused and visible, or null, each time that changes. The backend
keeps it with the connection and forgets it when the connection ends, and
sends no [push notification](../product/turns.md#notifications) of a
conversation to the browser of a session while any connection of that
session shows it. The first message is the whole product state; each
later one is the current value of one part of it that changed:

| Message | Carries | Sent when |
| --- | --- | --- |
| `snapshot` | `state`, the product state below | First, on every connection |
| `conversation` | `conversation`, the conversation's summary as the conversation lists carry it | Its record changes: a patch or a batch item, an archive, a restore or a target switch once it completes, a read acknowledgement, a draft saved, restored or dismissed, a message sent, a title requested or written. It is created or forked. Its tree saves a checkpoint, starts or stops working, or is disposed. A permission request is raised, decided, replaced or withdrawn |
| `conversation_deleted` | `id`, the conversation's id | It is deleted ([Deleting a conversation](../backend/storage.md#deleting-a-conversation)) |
| `conversation_order` | `ids`, the id of every conversation, in the product state's order | A conversation is created, forked, moved, pinned or unpinned, archived or restored |
| `preferences` | `preferences` | A preferences patch |
| `user` | `user` | The nickname or the email address changes, or the owner chooses the password |
| `users` | `users`, as `GET /api/users` lists them; sent only to the master's and admins' channels | An account is added, changed, reset, disabled or enabled, signs in, or chooses its password |
| `workspaces` | `workspaces`, in the user's order | A workspace is created, renamed, moved or deleted |
| `devices` | `devices`, the paired ones and the Cloud's, each with its state, `online`, `updating` or `offline` (`crates/runner/src/update.rs`), as an attached Host's state also reads, and for a paired device its `startCommand`, the command that starts its runner again (`crates/runner/src/console.rs`) | A device is paired or revoked, its runner connects, disconnects or starts or ends an update, its runner's installed artifacts change, or the Cloud's device is made |
| `providers` | `providers`, each with its details | An entry the user infers with, or an account of it, is created, changed or removed, a sign-in completes, an account's credential is renewed, or its quota snapshot is stored |
| `cloud` | `cloud` | The Cloud's lifecycle or its reset moves |
| `plugins` | `plugins`, the plugin list below | The user turns a plugin on or off |
| `credentials` | `credentials`, as `GET /api/credentials` lists them, a visible field's value included and never a secret's | A credential of the user is saved, changed, removed or expires |
| `subagents` | `subagents`, the user's [Subagent switch and profiles](web-api-settings.md#subagents) | The switch is turned on or off; a profile is created, changed, enabled, disabled or deleted |
| `instructions` | `instructions`, the user's [personal instructions](web-api-settings.md#instructions) | The user saves them |
| `passkeys` | `passkeys`, the user's passkeys as `GET /api/auth/passkeys` lists them | The user adds, renames or removes one, or one signs in |
| `sessions` | `sessions`, as `GET /api/auth/sessions` lists them, `current` for the receiving session | A session starts, ends or is signed out, or its last activity is written |
| `logins` | `logins`, the user's sign-ins that are running or ended within ten minutes, each as `GET /api/providers/subscription-login/:id` answers it, with its `providerId` or `providerType` | One starts, moves to another stage, shows its link or code, is refused a code, completes, fails, is cancelled or is dropped |
| `plugin` | `plugin`, the plugin's id, and `state`, its user state | The plugin marks its user state changed ([The page](../architecture/plugins.md#the-page)) |
| `heartbeat` | Nothing | 30 seconds pass without another message |
| `pong` | `id`, the number of the page's `ping` | The page sent `{ ping: id }`, which it does as it returns ([A dropped connection](web-application.md#liveness-and-reconnection)) |

The product state holds the current user, the instance mode, whether the
backend can send email (`mail`, without which changing the email address is
unavailable), preferences, the
provider entries of the user's scope, workspaces, devices (the paired ones and
the user's Cloud device, which the file and working-tree routes address
alike, each with `installed`, the artifacts its runner's cache holds, as the runner last
reported them and kept while it is away, each as `{ package, name, version }`
([Installed artifacts](../execution/native-runtime.md#installed-artifacts))),
the summaries of the active and then the
archived conversations, the Cloud's state, `logins`, the user's sign-ins as
the `logins` message carries them, `subagents`, the user's
[Subagent switch and profiles](web-api-settings.md#subagents), `instructions`, the user's
[personal instructions](web-api-settings.md#instructions), `plugins`, the plugin list, and
`pluginStates`, the user state of each plugin the user has on that declares
one, by plugin id, `publicUrl`,
the URL runners connect to (`DEMI_BACKEND_PUBLIC_URL`), `installCommands`,
the commands that install the runner on each system
([Workspaces, devices, and attached hosts](web-api-hosts.md#workspaces-devices-and-attached-hosts)), `stunUrls`, the
STUN servers a page's direct channel asks (`DEMI_STUN_URLS`), `runnerRelease`,
the runner release this backend serves, which a device's page compares with
the device's `runnerVersion` to say whether it is up to date, `webBuild`, the
build of the web app the backend serves, or null when it serves none
([A page of another build](web-application.md#a-page-of-another-build)), and
`run`, the id of this run of the backend
([Revisions counted in memory](#revisions-counted-in-memory)). Each provider entry carries its
`details`: `{ type: "read", ... }` with what `GET /api/providers/:id/status`
answers, or `{ type: "failed", message }` for an entry whose provider could
not be read, which leaves the others intact. Reading the state never starts
the Cloud or runs inference. It is not one atomic read across the control and
conversation databases, and it need not be: a change made while it is read is
sent after it.

Drafts and the plugins' conversation states are not sent: they can be
large, and only the pages that show their conversation need them. The summary
carries their revisions, and a page that shows the conversation reads each
when its revision is higher than the one it holds
([Conversation drafts](web-api-conversations.md#conversation-drafts),
[Conversation state of plugins](#conversation-state-of-plugins)).

**Order.** The backend reads a part when it sends it, after the change that
caused the message, and reads it again for a change made meanwhile. So a
part's messages come in the order of its changes, and a later one is never
older than an earlier one.

**Changes no write makes.** Each message follows a change the backend
commits; no part changes with time alone. A provider entry's `details`
report each account's sign-in as it is stored, whether or not the vendor
would still take it: an API key the vendor revoked, a Codex or Grok Build
or Claude Code sign-in whose refresh the vendor refuses all read as they
were stored. The backend learns of such a lapse
only when a request uses the credential. That request fails with the
provider's reason, in the conversation that sent it, and the stored sign-in,
with the entry's `details`, stays as it was. So a lapse changes nothing a
page shows, and needs no message.

**A page that falls behind.** The backend keeps no queue of messages for a
channel: it keeps the set of parts that changed since it last sent them, and
sends each of them once, as it is when the page takes more. For example, the
user types a draft on the laptop while the phone's page reads slowly: the
phone may never see some of the draft's revisions in between, and receives
the conversation's summary with the latest one once it reads again. A page
is never closed for falling behind and loses no change, and its channel costs
the backend at most one entry per part.

**Reconnecting.** A page whose channel closed connects again and receives a
new `snapshot`, which replaces everything it held. The backend registers the
channel for changes before it reads the snapshot, so each change is in the
snapshot or sent after it, and a page that was away misses nothing.

The backend closes the channel with a code and a reason:

| Code and reason | When |
| --- | --- |
| 1001 `backend_closing` | The backend shuts down; the page shows the banner "Demi is restarting" until it is back ([A page of another build](web-application.md#a-page-of-another-build)) |
| 1008 `unexpected_message` | The page sent a message |
| 1011 `internal_error` | A part could not be read |
| 4002 `session_ended` | The session the channel opened with ended: it was signed out, or it expired |

The channel never renews its session; only requests do
([Authentication and ownership](../backend/backend.md#authentication-and-ownership)).

### Revisions counted in memory

Two revisions of a conversation's summary count changes in the backend's
memory rather than in storage: each of `pluginRevisions` and
`permissionsRevision`. The answers of the
permissions and plugin state reads carry the same counts. A backend that
starts again counts from 0, so a count compares only with counts of the same
run. The product state's `run` names the run: an id the backend chooses when
it starts.

A page holds each such revision together with the `run` of the snapshot it
held when it took it, and a revision of another run is older than any
revision of the current one. For example, a page shows `c_81` and holds its
requests at `permissionsRevision` 5. The backend restarts; the page connects
again, and the snapshot carries a new `run` and `permissionsRevision: 1`. The
page reads the requests again, though 1 is lower than 5. Comparing the
numbers alone, it would keep the requests it held until the count passed 5.

An answer belongs to the run of the snapshot the page holds when it arrives:
a run's answers are all sent before it exits, and the next run starts, and
sends its first snapshot, only after that.

## A user's plugins

The plugin list is every plugin of the backend, in its order of
registration, each with `id`, `name`, `description`, `enabled`, `packages`,
the ids of the command packages its commands, user streams and page methods
bind that the backend's catalog serves. `PUT /api/plugins/:plugin { enabled }` turns a plugin on or off
for the caller and answers 204; the new list and the states of the plugins
that changed reach every page of the user on the synchronization channel. An
unknown plugin answers 404 `unknown_plugin`. What each change does, and when, is
[A user's plugins](../architecture/plugins.md#a-users-plugins)'s.

`GET /api/plugins/:plugin/oauth/callback?code&state`, or `?error&state`, is
where a vendor sends the browser back after a plugin's [OAuth
sign-in](../architecture/plugins.md#oauth-sign-in). It is a session route: the
browser's own navigation carries the cookie. It answers an HTML page:

| Status | When | The page says |
| --- | --- | --- |
| 200 | Signed in | *Signed in to Exa. You can close this tab.* |
| 400 | A state the caller did not start, older than 10 minutes or used already, or the vendor sent an error | *This sign-in expired or was declined. Sign in again in Demi.* |
| 401 | No session | The session gate's answer |
| 404 | A plugin that signs nobody in, or that the user has off | *Nothing signs in here.* |
| 502 | The code exchange failed | *Exa did not complete the sign-in. Try again in Demi.* |

`POST /api/conversations/:id/reload`, without a body, closes the
conversation's tree and opens it again with the user's current plugins, and
answers 204. A conversation socket that was attached to the tree receives
`closed` and connects again, as when another page's socket disposed the tree
([Connections and the live tree](../agent/rendering.md#connections-and-the-live-tree)). A
conversation whose tree is not open answers 204 and changes nothing: it opens
with the current plugins anyway. A tree that works answers 409
`turn_in_flight`, and an archived conversation 409 `conversation_archived`.

## Conversation state of plugins

A plugin's state for one conversation reaches a page by revision, as a draft
does. For example, the agent runs `demi browser open https://example.com/` in
conversation `c_81`. The browser package tells the `browser` plugin that the
conversation's tabs changed, and the plugin marks its conversation state, the
tab list, changed, so its revision rises from 6 to
7, and every page of the user receives the conversation's summary with
`pluginRevisions: [{ "plugin": "browser", "revision": 7 }]`. A page that shows `c_81` and holds
revision 6 reads the state:

```text
GET /api/conversations/c_81/plugins/browser/state

200 { "revision": 7, "state": { "tabs": [ { "id": "t3", "title": "Example Domain", ... } ] } }
```

The state matches the conversation state schema of the plugin's manifest. A
page takes an answer only when its revision is higher than the one it holds,
since a call's answer and a read can reach it in either order. The revision
counts the changes the plugin marked, in the plugin host's memory, so it
compares only within one run of the backend
([Revisions counted in memory](#revisions-counted-in-memory)). An unknown plugin answers 404
`unknown_plugin`, one without a conversation state 404 `unknown_plugin`, and a
plugin the user has off 409 `plugin_disabled`; the route needs a conversation
the user owns, as every conversation route does. A read is admitted as a
user stream is, so it never wakes a stopped Cloud: the plugin answers what it
can without it, as the browser's tab list of a stopped Cloud is empty
(`crates/plugin-browser/src/page.rs`). A refusal of the
conversation's host access, or a plugin that fails, answers as a plugin call
does ([Plugin calls](#plugin-calls)).

## Plugin calls

A plugin's page calls its plugin through one route, or, for a call about one
conversation, through that conversation's
([The page](../architecture/plugins.md#the-page)). For example, the Skills
settings section adds a source, and the work panel's `browser` kind opens a
tab in a conversation's browser:

```text
POST /api/plugins/skills/calls/add_source
{ "origin": "vercel-labs/agent-skills" }

200 { "source": "src_7fq2" }

POST /api/conversations/c_81/plugins/browser/calls/open
{ "url": "https://example.com/" }

200 { "tab": { "id": "t3", "title": "", "url": "https://example.com/", "createdBy": "user" } }
```

The body is the method's parameters, and the response's body is its result.
The plugin's manifest declares each method with a JSON Schema for its
parameters and one for its result, and its page package's types are
generated from the same Rust types. The routes work like the other
session routes ([Authentication](web-api.md#authentication)); the conversation's route
also needs a conversation the user owns, as every conversation route does:

| Situation | Answer |
| --- | --- |
| The backend has no plugin of that id | 404 `unknown_plugin` |
| The plugin has no method of that name for the route: a method of the user scope called for a conversation, or the other way round, has none | 404 `unknown_plugin_method` |
| The user has the plugin off | 409 `plugin_disabled` |
| The body is not JSON, or does not match the method's parameter schema | 400 `invalid_body`, naming the field and the reason |
| The plugin refuses the call, such as a source already added or a tab the browser does not have | 409 `plugin_refused`, with the plugin's own `reason`, a snake_case word such as `tab_not_found`, and its message |
| A port operation the plugin made was refused by the conversation's host access, and the plugin passes the refusal on | That refusal's own answer: 409 `conversation_archived`, `device_offline`, `host_stopped` or `conversation_busy` |
| A package call the plugin made was refused because the package's service could not start on the Host, and the plugin says so in its own reason, as the browser plugin's `browser_unavailable` | 409 `plugin_refused`, with that reason and the runner's words as the message |
| A package call's connection to the Host broke partway | 409 `host_operation_failed` |
| The plugin fails, such as a value that does not read or a package call whose operation failed | 500 `plugin_failed`, with the plugin's message |

A plugin whose page shows these failures declares the reasons it can give, its
own and the ones above it passes on, as one closed set in its manifest; the
page's types are generated from it ([Plugin pages](../architecture/plugin-pages.md)),
so a page that words every reason is checked by the compiler. A call the page
makes that ends without one of these answers rejects with the HTTP status it
got, or with none when no answer came or the answer could not be read; the
page says which: *The server didn't answer.*, *The server's answer could not
be read.*, or *The server couldn't do that: 500 Internal Server Error.*, the
status's standard phrase standing in where the answer carried none, as
HTTP/2's do.

A call changes only the calling user's state. What it changed reaches every
page of the user as the plugin's new user state on the synchronization
channel ([Page synchronization](#page-synchronization)), or as a higher
revision of its [conversation state](#conversation-state-of-plugins),
including the page that called, so a page shows what it called for the way
it shows every other change.
