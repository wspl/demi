# Web API: hosts and files

The routes that reach machines and their files: workspaces, devices and attached hosts, uploads and media, the Cloud, user streams, a device's log, device files and remote references, file text and working tree changes, and the direct channel. Conventions every route follows are in [Web API](web-api.md).

## Workspaces, devices, and attached hosts

Workspace creation takes `{ kind: "device", deviceId, path, name }` or
`{ kind: "cloud", name }`. The first names an absolute directory on a
caller-owned device; another device answers 404 `device_not_found`. The second
creates a project directory on the user's Cloud and can wake it. A name is
trimmed and has 1 to 256 characters. A creation answers `{ workspace }` with
201 and a change, `PATCH /workspaces/:id { name?, lastModel?, lastWorkflow? }`, each field replacing its value, with 200 and `{ workspace }`; a workspace is
`{ id, deviceId, path, name, createdAt, lastModel?, lastWorkflow? }`, the
last two what its next conversation starts with
([User preferences](web-api-settings.md#user-preferences)), and `GET /workspaces` returns
`{ workspaces }` in the user's order, which the product state carries too. A
workspace is a pointer; renaming or deleting it does not rename or delete
files. Deletion archives the conversations in it, then deletes it, as one
transition, and answers 204 with nothing; a running conversation in it
refuses it with 409 `turn_in_flight`, as its archive would, and nothing
changes. An id the caller does not have answers 404 `workspace_not_found`.
Creating a workspace for a device and directory that already has one answers
409 `workspace_exists` with `{ workspace }`, the existing one. A conversation
target names a directory, `{ kind: "device", deviceId, path }` or `{ kind:
"cloud", path? }`, or a project, `{ kind: "workspace", workspaceId }`; the
backend resolves a directory to the Host's home, the Cloud's own directory or
that directory's workspace, making it when there is none
([Resolve a target](../execution/sessions-and-targets.md#resolve-a-target)).

Device revocation applies to user-paired devices, not the managed Cloud device,
and nothing refuses it. It removes the device's projects with it, the
workspaces whose device it is, but not their files: a workspace is a pointer.
Their conversations stay, with their history, outside any project, targeting
the project's directory on the removed device as a conversation that targets
a device directly does, so they cannot run there any more. Before the user
revokes a device in Settings, the page names the projects that go with it and
says that their files and conversations stay. The answer, 200, is `{ removed
}`, the ids of the workspaces removed. Revocation tells a connected runner to
remove itself
([Installation, pairing and removal](../execution/runner.md#installation-pairing-and-removal)),
closes its connection and removes its conversation attachments.
Pairing accepts a live code and returns the claimed device; expired/unknown codes
return 404 and excessive attempts 429. The installation routes contain no
credential and do not grant device access. The install commands, which the backend composes and the state's
`installCommands` carries for `linux`, `macos` and `windows`, and which the
page and `demi host install-command` show as they are, fetch the installer
from the origin of `publicUrl`, not from the page's own origin, which in
development is Vite's, and let `curl` use only that URL's scheme, TLS 1.2 or
newer for `https`; a development backend serves
plain `http`. The runner receives a pending code; the signed-in page claims
it through `POST /api/devices/claim`. Device tokens are delivered only to the
runner.

A device in `GET /devices` and in the product state carries, besides its
name and online state, `os`, the operating system and architecture its runner
last reported, and `runnerVersion`, the runner release it last reported, such
as `0.1.16`; both are null before its runner first connected;
`autostartUnavailable`, the reason its runner last reported for not
starting by itself after a restart, as the hello carries it
([Connection and identity](../execution/runner.md)), kept on the device
record and shown on the device's page, null when it does or before it
first connected; `mark`, the
logo and number of its [mark](conversations.md#conversations-and-projects), null for the Cloud:
`{ logo, number }`, where `logo` is `apple`, `windows`, `ubuntu`, `debian`,
`fedora`, `arch`, `centos`, `redhat`, `mint` or `tux`, which the backend
derives from the platform and the reported distribution, and `number` is
the device's number for that logo, null before its runner first connected
(the page shows it only while another device has the same logo); and `route`,
how pages reach it: `automatic`, `direct` or `server`, the routes
Automatic, Prefer P2P and Relay Only of a
[direct channel](../execution/direct-channel.md#choosing-the-path),
`automatic` for a new device. Settings shows them on the device's page, the Cloud's
included ([What the user sees](../execution/direct-channel.md#what-the-user-sees)).
`PATCH /devices/:id { name?, route? }` renames a paired device or sets its
route, each field applied when present: the name is
trimmed and has 1 to 64 characters, and the answer is `{ device }`, 200, as
for a claim. The Cloud's device keeps its
name and has no route: changing either answers 409 `device_managed`, and a device the caller does
not have 404 `device_not_found`. A rename reaches every page of the user in
the product state, and the conversations that reach the device go by its new
name ([Attached hosts](../execution/sessions-and-targets.md#attached-hosts)).

Attached-host responses contain device identity, name, the directory its
commands start in (`cwd`), online state, and attachment time. A detach is a transition, like a target
change; detaching a device that is not attached answers 204.
Changes follow [Attached hosts](../execution/sessions-and-targets.md#attached-hosts).

## Uploads and media

`POST /attachments?name=<file name>` accepts a nonempty raw request body with
its media type in `Content-Type`. The header must name one `type/subtype`, and
`multipart/*` — a form envelope rather than a file's bytes — is rejected. The
maximum body is 25 MiB. The name, 1 to 255 characters, decides whether the
answer carries a text file's opening, as it decides for a message that sends the
file; a missing name answers 400 `invalid_query`. It answers 201 with
`{ attachment }`: the attachment's id and metadata, including the media type the
backend reads from the file's bytes when it recognizes them and, for a text
file, its snippet. The snippet is the opening of the file with leading blank
space removed and line endings normalized, cut to 160 characters (Unicode scalar
values). The composer, for a new message or for an edited one, shows the media
type and the snippet on the file's capsule
([Attachments](composer.md#attachments)), so the capsule shows what the backend
determined.

The page sends no bytes the caller's namespace holds already. Before an upload
it computes the file's SHA-256 and asks with
`POST /attachments?name=<file name>&sha256=<hash>` and no body: when the
caller's namespace holds that blob, the answer is 201 with `{ attachment }`
for it, whose media type and snippet the backend reads from the blob's first
64 KiB, a range of the stored object, since both come from a file's opening; otherwise 200 with
`{ attachment: null }`, an answer and not a failure, since a browser logs
every 4xx as an error in its console and a new file's first question would
always log one; the page
then uploads the bytes as above, naming the same `sha256`, which the backend
checks against what it received. The check reads only the caller's own
namespace, so it says nothing of another user's files.

`GET /api/conversations/:id/attachments/:attachment` answers an attachment
the agent uploaded ([Attachment commands](../execution/commands.md#attachment-commands))
as `{ id, name, mediaType, size, blob }`, with an image's `width` and `height`
in pixels, which the backend reads from its header when it stores it, where
`attachment` is its number,
such as `a3`, and `blob` the hash its bytes are served under, below; a number
the conversation does not have answers 404 `not_found`. It reads the
conversation's database alone and wakes no Host.

`GET /blobs/:sha256` serves a blob of the caller's own namespace, under the
headers [Media by reference](../backend/backend.md#media-by-reference)
gives it. A name that is not a SHA-256 in lowercase hexadecimal, or that the
caller's namespace does not hold, answers 404 `not_found`, whoever else holds
that name. Byte ranges are answered as `fs/raw` answers them: one range in
`Range` answers 206 with that range, several answer the whole blob with 200,
and a range that starts past the end answers 416, so a player can play and
seek a video ([Media a tool returned](file-previews.md#media-a-tool-returned)).
The change view reads the two sides of a tool call's edit from it, by the
hashes the call's view holds ([Edit copies](../execution/edit-tracking.md#edit-copies)).

Uploading alone stores a backend blob and the upload's record, which keeps
the media type and the snippet the answer carried. A send, steer or edit frame
names an upload in its content as `{ type: "upload", ref, fileName }`; the
backend resolves the upload and writes the file to the selected Host's Demi
attachment directory, which can require an available execution target. No
frame carries a file's bytes. Media handling and failure behavior are defined
in [Backend media handling](../backend/backend.md#media-by-reference).

A [draft](web-api-conversations.md#conversation-drafts) names an upload the same way, so every page
shows the file's capsule from its record, and any of them can send it. An
upload stays readable for as long as a draft names it: nothing deletes an
upload's record or its bytes
([Open decisions](../backend/storage.md#open-decisions)).

## Cloud

Each user has one Cloud device. [Managed hosts](../cloud/managed-hosts.md)
owns its allocation, wake, shared-device admission and reset;
[Sessions and targets](../execution/sessions-and-targets.md) owns how a
conversation selects it. A workspace on the Cloud is a directory on that
device, not a machine of its own.

`GET /api/cloud` returns `{ device, state, operation, error, volumes, newerImage }`,
and the product state carries the same object as `cloud`:

| Field | Meaning |
| --- | --- |
| `device` | `{ id, name }`, the logical device identity; null until the first use makes it |
| `state` | `unallocated`, `off`, `booting`, `running`, `saving` or `resetting` |
| `operation` | The latest reset, `{ id, phase, error }`, with `phase` one of `stopping`, `saving`, `rebuilding`, `booting`, `ready` and `failed`; null before the first |
| `error` | Why the Cloud's last boot, save or reset failed; null once a boot succeeds |
| `volumes` | The capacities of its system and home filesystems, `{ systemBytes, homeBytes }`; null until its first boot made them, or while the machine manager does not answer |
| `newerImage` | Whether the server runs another Cloud image than the one the Cloud's system is on, which a reset moves it to ([Demi's programs in a Cloud](../cloud/managed-hosts.md#demis-programs-in-a-cloud)); false before the first boot and while the machine manager does not answer |

The fields describe one moment: a reset reads `ready` only once `state` reads
`running` again.

Reading the status never wakes the Cloud. `POST /api/cloud/reset` takes
`{ operationId }`, a UUID the page chooses, and answers 202 `{ operation }`.
The backend selects and records the shipped base version at admission.
Retrying the same operation id returns the same operation, and retrying one
that failed resumes it; a reset with another operation id while one runs
answers 409 `cloud_resetting`, and a stopped Cloud that finds no capacity
permit answers 409 `cloud_capacity`. A reset affects every job on the user's
Cloud and preserves home. Requests cannot name another user's device or
arbitrary image paths. Settings use this API even when the guest is offline or
broken.

An operation that needs the Cloud running waits while a reset holds it
([How a conversation uses a device](../execution/sessions-and-targets.md#how-a-conversation-uses-a-device)),
and answers 503 with the lifecycle's code when it cannot have it:
`cloud_capacity` when every capacity permit of the backend is taken,
`cloud_crash_loop` once repeated runtime losses have stopped its automatic
boots, which a reset recovers, and `cloud_unavailable` when it cannot start,
when the reset it waited for failed, or when it reached the Cloud through a
Host handle while the Cloud was changing state
([Lifecycle and capacity](../cloud/managed-hosts.md#lifecycle-and-capacity)).

## User streams

`WS /api/conversations/:id/streams/:name` opens the declared
[user stream](../execution/native-runtime.md#user-streams) `name` on the
conversation's primary Host. A plugin declares each stream
([Calling its command package](../architecture/plugins.md#calling-its-command-package)):
`plugin-browser`'s `browser` is the [live browser view](../browser/live-view.md). The upgrade requires the session
cookie and a conversation the user owns, and is refused from a page that is
not the product's ([Authentication](web-api.md#authentication)), which matters all the
more here since the stream operates the conversation browser, which is signed in
to the user's sites.
The route answers before the upgrade:

| Answer | When |
| --- | --- |
| 403 `forbidden_origin` | The upgrade comes from a page that is not the product's |
| 404 `unknown_stream` | No stream has that name, or the user has its plugin off |
| 426 `upgrade_required` | The request is not a WebSocket upgrade |
| 409 `conversation_archived` | The conversation is archived |
| 409 `device_offline` | A paired device has no live runner |
| 409 `host_stopped` | The Cloud is stopped; a user stream never wakes it |
| 409 `conversation_busy` | An archive, a target change or a detach is ending the conversation's streams |
| 503 `stream_failed` | The Host could not open the stream: its service failed to start, or refused it |

Admission follows [Host operations](../execution/sessions-and-targets.md#host-operations).
After the upgrade, binary messages carry the stream's bytes both ways, in
order; message boundaries carry no meaning, since the stream frames its own
messages. The backend reads from the Host only as fast as the page takes the
bytes. It closes the socket when the stream ends, with a code and a reason:

| Code and reason | When |
| --- | --- |
| 1000 `completed` | The invocation completed |
| 1003 `binary_only` | The page sent a text message |
| 1011 `host_unreachable` | The Host became unreachable, or a pipe to it failed |
| 4000 `conversation_changed` | An archive, a target or directory change, or a detach ended the stream |
| 4001 `plugin_disabled` | The user turned off the plugin that declares the stream |

## Device log

`GET /api/devices/:id/log` returns lines of the
[Host's log](../execution/runner.md#host-log) for a device the user owns, the
user's Cloud included: `{ lines: [{ at, source, conversationId?, text }], next }`,
oldest first. `since` is the `next` of an earlier answer; without it the
answer ends at the newest line. `limit` defaults to 200 and is at most 1000.
`source` keeps only the lines of one source. The route wakes nothing: a
stopped Cloud or an offline device answers 409 as
[Host operations](../execution/sessions-and-targets.md#host-operations) do,
and its log is read when it runs again, since the log outlives a restart.

## Device files and remote references

`GET /api/devices` includes the connected runner's `home` (null while unknown).
`GET /api/devices/:id/fs` defaults to that home when path is omitted and returns
`{ path, home, entries }`. Each entry has name, isDirectory, byte size and
modifiedAt, its own as `lstat` reports them, and a symbolic link has `link`:
its `target` as written and `leadsTo`, `{ kind, reason }`, where `kind` is
`file`, `directory`, `other`, `missing` or `unreadable`, and `reason` the
system's words for an unreadable one
([Symbolic links](file-previews.md#symbolic-links)). The runner
reads them beside each entry as it lists the directory, so a listing is one
request ([Host operations](../execution/runner.md)); entries disappearing
during the listing are omitted, other errors are returned. These device routes serve paired-device target selection
only; the work panel and remote attachment picker use the conversation routes
below. Managed devices are not accepted by the device directory routes. These
routes reach the device through device access
([Every way to a Host](../execution/sessions-and-targets.md#every-way-to-a-host)),
so a device that is not connected answers 409 `device_offline`.

The content of a send, steer or edit frame may contain
`{ type: "remote_file", deviceId, path }`, where path is absolute. The backend validates ownership and current connectivity for
all referenced devices before adding any attachment grant. It attaches non-primary
devices as [attached hosts](../execution/sessions-and-targets.md#attached-hosts),
then supplies text preserving the device identity and a shell-quoted
`demi host shell --host` read command. The agent reads the file's contents at
execution time. Revocation or disconnect before that read produces the host
command's error; the reference is not a byte snapshot.

## File text and working tree changes

The work panel uses `GET /api/conversations/:id/fs?path=...` to list a directory
as `{ path, home, entries }`, with the same entry shape as device browsing.
Omitting `path` selects the conversation's execution directory. `POST` to the
same route with `{ path }` creates a directory recursively and returns `{ path }`
with status 201. `GET /api/conversations/:id/fs/file?path=...` returns
`{ path, text }` with the file's ETag, the one the raw route reports; a
request whose `If-None-Match` names the file's current ETag answers 304
without reading the text past the Host, since the runner compares the version
before it streams ([File contents](../execution/runner.md#file-contents)). The
files service checks a text it keeps this way
([What the service keeps](../architecture/plugin-pages.md#what-the-service-keeps)). Paths follow the Host's filesystem rules; the execution directory
is a starting directory, not a permission boundary. Missing paths answer 404,
and permission failures answer 403. A directory too large to list in one
runner message ([Runner](../execution/runner.md#connection-and-identity))
answers 413 `directory_too_large`.

All three operations use [conversation Host access](../execution/sessions-and-targets.md#host-operations),
including Cloud wake and the file gate. Archived conversations answer 409
`conversation_archived`, and a conversation whose target device no longer
exists answers 404 `device_not_found`. A paired device without a live runner answers 409
`device_offline`; a Cloud that cannot wake answers 503 with the lifecycle's code.
A file over 8 MiB answers 413 `file_too_large`, the same limit a retained edit
snapshot has ([Edit tracking](../execution/edit-tracking.md#scope)), and the
page then reads its first 8 MiB from the raw route with a byte range
([File previews](file-previews.md#getting-the-bytes)); one that
is binary, by the one rule ([What `demi file view` shows](../agent/shell-tool.md#what-demi-file-view-shows)), answers 415 `not_text`,
and any other file is text, a byte that is no UTF-8 shown as U+FFFD.

`POST /api/conversations/:id/fs/exists` takes `{ paths }`, 1 to 200 absolute
Host paths, and answers `{ files }`, the ones that are regular files, symbolic
links followed, in the order asked; a missing path, a folder and one the Host
could not look at are left out, and any other body answers 400
`invalid_body`. It is how a page learns which code spans of a reply name files
([Files named in messages](file-previews.md#files-named-in-messages)), and it
is admitted as a user stream is: a stopped Cloud answers 409 `host_stopped`
and is not woken, an offline device answers 409 `device_offline`, and it
counts as no activity.

`GET /api/conversations/:id/fs/raw?path=...` streams a file's bytes for
[file previews](file-previews.md) and downloads, through the same Host access
and with the same failures, and without a size limit. A media type the
preview table shows in place is served as itself; any other file, and every
file when `download=true`, is served as `application/octet-stream` with
`Content-Disposition: attachment` and the file's name. A path that is not a
regular file answers 404. A streamed answer has no `Content-Length`; `HEAD`
reports it, with the ETag and `Last-Modified`. One byte range in `Range`
answers 206 with that range; several ranges answer the whole file with 200,
and a range that starts past the end answers 416. The ETag derives from the
file's size and modification time, and `If-None-Match` answers 304. A request
that names the `version` it expects, an ETag it saw earlier, answers 412
`file_changed` once the file no longer has it: a player's retries carry no
validator, and they must never splice two versions of a file together. Every
answer carries `X-Content-Type-Options: nosniff`, `Vary: Cookie`, `Cache-Control:
private, no-cache`, or `private, max-age=31536000, immutable` for a request
that names its `version`, whose bytes can only be that version's, and
`X-Accel-Buffering: no`, the last so that a proxy in front streams it instead
of buffering it; an image served in place also carries the
[content policy](file-previews.md#keeping-file-content-inert). `HEAD` answers
the headers alone. How long a transfer may last, and what ends it, follows
[Host operations](../execution/sessions-and-targets.md#host-operations); while
an archive, a target change or a detach is ending the conversation's
transfers, a new one answers 409 `conversation_busy`.

`PUT /api/conversations/:id/fs/raw?path=...&replace=true|false` writes the
request body, a file's raw bytes, to `path`, streaming it to the Host as the
Host takes it. The file appears whole or not at all: an upload cut short
leaves the path as it was ([Runner](../execution/runner.md#file-contents)).
A directory at the path answers 409 `is_directory`, and a file there answers
409 `file_exists` unless `replace=true`; the Host checks as it writes, and no
check precedes the write ([Host operations](../execution/runner.md#host-operations)).
The write makes the missing directories above the path, so an uploaded
folder's files make its folders, and the web app makes only an empty folder
with `POST`. Success answers 204. An upload is a file transfer whose pace the
user's browser sets, under the same Host access, stall rule and ending as a
download
([Host operations](../execution/sessions-and-targets.md#host-operations)).

`DELETE /api/conversations/:id/fs?path=...` deletes a file, or a directory
with everything in it, and answers 204; nothing at the path answers 204 too.
It refuses, with 409 `protected_path`, the filesystem root, the Host's home
directory, the conversation's execution directory, and any directory that
holds one of them: the file tree deletes only what an upload replaces inside
the workspace.

The remote attachment picker lists and creates directories through
`GET/POST /api/conversations/:id/hosts/:deviceId/fs`, with the same directory
contract. The device must be the conversation's primary or an attached host,
otherwise the request answers 404 `host_not_attached`. Omitting `path` lists
that Host's starting directory. These routes use the same Host access as the
work panel, including waking an attached Cloud. They do not read file contents;
the selected path becomes a remote reference when the message is sent.

`GET /api/conversations/:id/changes/branches`, `changes/log?branch=&after=`
and `changes/commit?commit=` answer the runner's `git_branches`, `git_log`
and `git_commit` ([Runner](../execution/runner.md#working-tree)) for the
same directory and with the same `directory` query as `changes`;
`changes/file` and `changes/raw` take `commit` to read a file as that commit
has it, and `changes/file` takes `from`, a renamed file's earlier path, whose
content is the diff's old side, in both modes. `ENOENT` answers 404 `not_found`, and `ambiguous` 409 `ambiguous`.

`GET /api/conversations/:id/changes` lists the uncommitted changes of the
conversation's execution directory, or of the directory `?directory=` names
on its primary Host, absolute, as a child's Change view asks for its own
([Working directory](../agent/subagents.md#working-directory)); `changes/file`
and `changes/raw` take the same `directory` and read that tree; a relative
one answers 400 `invalid_query`. It answers as `{ root, repository, gitDir, head,
files, truncated, watched }`, the runner's reply
([Runner](../execution/runner.md#working-tree)) plus `root`, the directory the
paths are relative to. `gitDir` is the absolute path of the repository's git
directory, null outside a repository; the page reads the reports under it by
[what each entry depends on](../architecture/plugin-pages.md#what-the-service-keeps).
The answer carries an ETag of its content, and a request whose
`If-None-Match` names it answers 304 without the list: the runner still
computes the list, which costs it little once its watch runs, but the page
receives nothing when nothing changed. The request reaches the host the way every conversation
file operation does
([Sessions and targets](../execution/sessions-and-targets.md#host-operations)):
a stopped Cloud wakes for it, a paired device without a live runner answers
409 `device_offline`, and a Cloud that cannot wake answers 503 with the
lifecycle's code. A request beyond the runner's working-tree capacity waits
for a slot ([Load](../execution/runner.md#load)); the runner's timeout answers
503 `changes_timeout`, and the page then keeps its previous list and says
the refresh failed.

`GET /api/conversations/:id/changes/file?path=...` returns `{ original,
modified }` for one changed file: `original` as the last commit has it (empty
for an added file), `modified` as the working tree has it (empty for a deleted
one), under the text limits of the file route. It carries an ETag of its
content and answers a matching `If-None-Match` with 304, as the list does.

`GET /api/conversations/:id/changes/raw?path=...` streams one file as the last
commit has it, the committed side of a previewed change, with the headers,
range handling and `download` of the raw file route but no validators. The
working-tree side comes from the raw file route. A path the last commit does
not have answers 404, and a file over 8 MiB answers 413 `file_too_large`,
since git's copy is decoded whole before it is sent
([Runner](../execution/runner.md#working-tree)).

Every answer of these routes that reads what a subscription covers, a
listing, a file's text, the changes list, a changed file's sides and the
committed contents, carries the Host's number taken before the read, as
`X-Demi-Read-At: <epoch>.<number>`, in a 304 as much as a 200; a direct
channel's answer carries the same
([Direct channel](../execution/direct-channel.md)). The page decides from it
whether what it keeps is current
([What the service keeps](../architecture/plugin-pages.md#what-the-service-keeps)).
A binary file and one over the text limit are answers like any other, with
their own ETag and number: 415 `not_text` and 413 `file_too_large` carry
both.

The page reads the list and a file's sides again only when the
[file watch](#file-watch) reports a subscription they depend on changed, when
a view showing them comes on screen, and on its Refresh control, as
[What the service keeps](../architecture/plugin-pages.md#what-the-service-keeps)
says. It never polls.

### File watch

`WS /api/conversations/:id/fs/watch` carries the page's subscriptions to
what it keeps of the conversation's Host files, and the Host's word that one
of them changed ([Watching files](../execution/runner.md#watching-files)). The
page holds one while it keeps any such entry, whether or not a view shows it
([What the service keeps](../architecture/plugin-pages.md#what-the-service-keeps)).
The upgrade checks the session cookie and the `Origin` header as the browser
stream does, and is admitted through the conversation's host access without
waking a stopped Cloud. A device's folder dialog, and a new conversation's
workflows on a device or the user's Cloud, subscribe the same way on
`WS /api/devices/:deviceId/fs/watch`, through device access, which never
wakes a stopped Cloud
([Every way to a Host](../execution/sessions-and-targets.md#every-way-to-a-host)).
The conversation's route covers its primary Host and its attached hosts: an
attached host's files, as the attachment picker lists them, are subscribed
with the subscription's `host`, the attached device, admitted through the
conversation's host access as a file of that host is, an attached Cloud
included. The page opens one socket on the route per host, since `offline`
and a close end every subscription of their socket, and one host going away
leaves the others' subscriptions as they are. The limits count per host:
4,096, of which 64 outside the working tree, every subscription of an
attached host counting as outside it.

The page names each subscription by a key of its own and sends JSON text
messages:

| Message | Carries |
| --- | --- |
| `subscribe` | `subscriptions`, each `{ key, kind, path, host? }`, `kind` one of `file`, `folder_names`, `folder_details`, `worktree`, `head`, as the runner's kinds, and `host` the attached device for a file of an attached host |
| `unsubscribe` | `keys`, the subscriptions the page no longer holds, as when an entry leaves its cache |
| `renew` | `keys`, every subscription the page still holds, once every 3 minutes |

The backend sends:

| Message | Carries | Sent when |
| --- | --- | --- |
| `live` | `key`, `since` and `epoch` | The Host's watch behind the subscription runs ([Numbers](../execution/runner.md#watching-files)) |
| `changed` | `changes`, each `[key, at]` | The Host reported them, at most every 100 ms, each key once per message |
| `unavailable` | `key` and the Host's `reason` | The Host cannot watch it; the subscription is over |
| `refused` | `key` and `reason: "limit"` | The page holds 4,096 subscriptions, or 64 outside the conversation's working tree |
| `renewed` | `keys` | A `renew` reached the backend; the page counts each lease from when it sent the `renew` this answers |
| `offline` | Nothing | The Host is out of reach or a stopped Cloud; no subscription is live until the next `live` |
| `heartbeat` | Nothing | 30 seconds pass without another message, so the page tells a quiet watch from a dead one ([Liveness and reconnection](web-application.md#liveness-and-reconnection)) |

Pages that subscribe to the same subject share one subscription on the Host,
which the backend renews while any page still renews it and ends when the last
one stops. A lease runs 10 minutes: the backend counts it from when a `renew`
arrives, the page from when it sent it, so the page always believes a lease
over first, and never trusts a subscription the backend has already let go.

The backend closes the socket with the user streams' codes: 1011
`host_unreachable` when the Host became unreachable, 4000
`conversation_changed` when an archive, a target or directory change, or a
detach ended the subscriptions, and 1003 `invalid_message` for a message the
page should not have sent. Every subscription ends with the socket; the page
opens a new one after its
[reconnect waits](web-application.md#liveness-and-reconnection) and
subscribes again to what it keeps.

## Direct channel

`WS /api/devices/:deviceId/direct` is the signaling socket of a
[direct channel](../execution/direct-channel.md) between the page and a paired
device's runner. The upgrade checks the session cookie and the `Origin`
header as the other sockets do, and that the caller owns the device; a
device that is the caller's Cloud answers 409 `not_a_paired_device`, and a
device whose runner is not connected 409 `device_offline`. Messages are JSON
text.

The page sends:

| Message | Carries | Sent when |
| --- | --- | --- |
| `offer` | `sdp` | The page makes a peer: at first, and on each new attempt |
| `candidate` | `candidate` | The browser found another address for the peer, such as its public one once the STUN server answered |
| `ping` | `id` | A probe of a measurement of the relay path ([Measuring the paths](../execution/direct-channel.md#measuring-the-paths)) |

The backend sends:

| Message | Carries | Sent when |
| --- | --- | --- |
| `answer` | `sdp` | The runner answered the offer |
| `candidate` | `candidate` | The runner learned another address, such as its public one |
| `pong` | `id` | The runner answered the page's `ping` |
| `unanswered` | `code` | The runner refused the offer (`busy`) or did not answer within 10 seconds |
| `closed` | Nothing | The runner closed the socket's peer because the user turned a plugin on or off; the page offers again at once |
| `heartbeat` | Nothing | 30 seconds pass without another message |

A new `offer` replaces the socket's peer: the runner closes the old one. The
socket closes with 1011 `host_unreachable` when the runner's connection
ends, and 1003 `invalid_message` for a message the page should not have sent.
