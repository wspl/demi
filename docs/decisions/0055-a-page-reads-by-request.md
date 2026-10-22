---
status: accepted
---

# A page reads by request, and a socket pushes only changes

Supersedes, in part: 0009 (agent sessions and sync), where a subscription is
answered with a reset and nothing bundles a first screen; 0019 (cache
chart), where every subscribed node carries its chart; 0029 (a conversation
opens whole), where the opening carries every fact and the page reads none
by a route; 0030, where a value fetched on sight is a poll; 0044, where
opening sends the latest page; and 0049, where an entry carries its panel
tabs to every page.

## Summary

Everything a page shows when a view opens is read by a GET request. The sync
channel and a conversation's socket carry only changes, never a snapshot and
never the answer to a subscription. A read never waits for a socket, names
one or needs one open: the page reads and draws, and follows the value on
its socket once it holds it, by saying which version it holds. What only a
closed view needs is read when it opens.

## Motivation

* A user opens a subagent. The page subscribes on the conversation's
  socket, and the answer is the subagent's latest page, up to a thousand
  blocks and about a megabyte, with its cache chart. Nothing shows until it
  has come through the socket, behind every other frame of the
  conversation.
* The page's sync channel opens with the whole product state: the
  conversation list, and the instructions, credentials, passkeys, sessions,
  users, automations, port forwards, exposed ports and subagent profiles
  that only a settings pane shows. Every reconnect sends all of it again.
  Each of these has a GET route the page never calls.
* A conversation's socket opens with its metadata, which includes the saved
  messages, workflows and shares that only a menu shows, then a reset and a
  cache chart for every subscribed node and the view of every live command.
* Reads went into the sockets in steps: a sync channel replaced a poll and
  removed `GET /api/state`; a rule that an owner announces a change was
  read as forbidding the page to ask; 0029 put a conversation's facts into
  the opening and removed five routes; 0030 called a read on sight a poll.
  A read when a view opens asks once for what it shows; it is not a poll.
* A read in a socket cannot be cached by the browser, cannot run beside
  another, and waits behind whatever the socket sends first.

## Guide-level explanation

Nothing changes on screen. A conversation, a subagent and a settings pane
show what they showed, from the page's offline copy at once when it holds
one, and the spinner shows only while their own reads are in flight; no
read waits for the socket. After a sleep, a network drop or an upgrade, the
page tells the socket which versions it holds, and only what changed comes
or is read again.

## Reference-level explanation

All paths are under `/api`.

### Reading and following

A read never depends on a socket; a socket depends on the reads. A page
sends its reads when a view opens and draws what they answer. It opens the
socket after the reads it follows have answered, and its first frame names
every value it follows with the version it holds; a value it reads later,
such as a subagent chosen afterwards, it follows by a frame of its own.
This holds because each socket carries one scope's values, the sync
channel the product's and a conversation's socket that conversation's, so
each opens after its own scope's reads:

```rust
/// A value a page follows, and the version of it the page holds.
struct Follow {
    value: Followed,
    holds: Held,
}

enum Held {
    /// The ETag the value's read or last push carried.
    Tag(ETag),
    /// The product state's, a transcript's or the agents' version. For a
    /// transcript, `readings` says whether the page holds the node's request
    /// output and cache chart, as its read or a push brought them.
    Version { version: Version, readings: bool },
}
```

1. The server registers the follow, then compares the value's current
   version with `holds`, under the value's own lock, so no change falls
   between the two.
2. The same version: nothing is sent. A change after it is pushed.
3. Another version of a whole value: the current value is pushed, as any
   change of it is.
4. A transcript past the version held: the server sends the patches after
   it from the transcript's recent patches, then each new one. The agents'
   entries past it: the entries changed since, as today. A version the
   recent patches no longer reach, or of another epoch: the server sends
   `stale { value }` and ends the follow; the page reads the value again by
   its GET, and follows it again with the version that read carries.
5. `Unfollow { value }` ends a follow when the view that showed the value
   closes.

* **Whole values.** A conversation's metadata and each list are pushed
  whole, with their ETag, when they change. The ETag is the one their GET
  answers with, the hash of the body.
* **The product state.** A user's product state has one
  `Version { epoch, revision }`: each change of a part of it, which already
  goes through one place, adds one to the revision, and the user keeps, in
  memory, the names of the parts the last 1,000 changes touched, shared by
  every page and dropped oldest first; the epoch changes when the backend
  starts. `GET /state` answers with the version it read at. A follow with a
  version among the last 1,000 changes is sent the parts changed since,
  each read again and pushed whole; an older one, or of another epoch, is
  sent `stale`.
* **Deltas.** A transcript's patches and the agents' entries carry the
  `Version { epoch, revision }` their logs own today, each one past the
  revision it follows. A page applies a delta one past what it holds,
  ignores one not greater, and on a gap or another epoch sends `Unfollow`,
  reads again and follows again.
* **Recent patches.** A transcript that is running keeps its latest patches
  in memory, shared by every page that follows it, as Kubernetes' watch
  cache does: at most 1,000 patches and 1 MiB, the oldest dropped first,
  and freed when the node stops running or its tree closes. A transcript
  that does not run changes by no patch, so it keeps none.
* **Stored versions.** A read with no live session behind it, which restores
  no tree, answers from the store at the epoch `stored` with a revision the
  store derives: a node's transcript from its output revision and its
  path's length, the agents from the rows that decide their entries. Every
  save that changes what a page shows changes it, so a page that follows a
  stored version learns whether the store still holds what it read. A tree
  that restores from the store the page read leaves the page's copy as it
  is: the follow of a stored version is compared with the store, and when
  they agree and the live value has changed nothing since the restore, the
  server sends `rebased { value, version }`, which moves the page's copy to
  the live version without a read; the agents then come whole as the
  entries changed since revision 0. Otherwise it sends `stale`.
* **A break.** When a socket's connection closes, a backend restart, a sleep
  or a network drop included, its follows end. It opens again as it first
  did: its first frame names every value the page still shows, with the
  version it holds; only what changed is sent or read again. The server
  keeps nothing for a page that left: the recent patches belong to the
  transcript, not to any page.
* GET answers carry an ETag and revalidate (0050), so a read again of what
  did not change costs a 304.

### Reads of the product

| Route | Answer | Read when |
|---|---|---|
| `GET /state` (added again) | `ProductState` without the settings lists below | The page starts, and again when the channel says its version is stale |
| `GET /auth/passkeys` | `PasskeyList` | Passkeys pane opens |
| `GET /auth/sessions` | `SessionList` | Sessions pane opens |
| `GET /users` | `Users` | Users pane opens |
| `GET /credentials` | `CredentialList` | Credentials pane opens |
| `GET /automations` | `AutomationList` | Automations opens |
| `GET /port-forwards` | `PortForwardList` | Port forwards pane opens |
| `GET /exposed-ports` | `ExposedPortList` | Exposed ports pane opens |
| `GET /settings/instructions` (new) | `{ instructions }` | Instructions pane opens |
| `GET /settings/subagents` (new) | `SubagentSettings` | Subagents pane opens |

`ProductState` loses `instructions`, `credentials`, `port_forwards`,
`exposed_ports`, `automations`, `passkeys`, `sessions`, `users` and
`subagents`; it keeps `automations_unread` for the sidebar's badge.

### Reads of a conversation

| Route | Answer | Read when |
|---|---|---|
| `GET /conversations/:id` | `ConversationDetail` as today (summary, metadata, agents' `Version`, entries); it answers from the live tree or from the store and restores no tree | The conversation opens, and again when `stale` names its agents |
| `GET /conversations/:id/agents/:node/transcript?latest` (new query) | `TranscriptPage` of the latest page, read from the live window when the node runs, plus its transcript's `Version`, the `ShellStatus` of each live command in it, the node's `RequestOutput` while it waits for or streams a model's answer, and its `CacheChart` | A node is shown: the root with the opening, a subagent when chosen; again after `stale` or a gap |
| `GET /conversations/:id/saved` (added again) | `SavedListing` | The saved list opens |
| `GET /conversations/:id/workflows` (added again) | `ConversationWorkflowList` | The workflow menu opens |
| `GET /conversations/:id/shares` (new) | `Vec<ConversationShare>` | The share dialog opens |
| `GET /conversations/:id/draft` | `DraftAnswer` | The composer opens and the metadata names a newer draft revision |

* `ConversationMetadata` loses `saved`, `workflows` and `shares`. The page
  reads the saved messages and the workflows with the conversation, follows
  each by its tag, and the socket pushes it whole when it changes (`saved`,
  `workflows`); the shares are read when the share dialog opens, and a page
  that does not follow them is not sent them. The draft keeps its revision
  in the metadata, as today.
* The opening's reads, the conversation, its saved messages, workflows and
  draft, and the root's latest page, are sent at once.
* What a node's view shows, its transcript, its request's output and its
  context card's chart, comes in the one read of the node, the root's as a
  subagent's; every node shows its request's output as the root does. A
  context card opens with no request, and changes of the output and the
  chart come on the node's follow, each `request_output` naming its node.
  A follow whose page holds no reading, as one shown from a copy on the
  disk, is sent the output and the chart as they stand, as is one that was
  behind: no version covers them.
* `?before=`, `?after=` and `?around=` pages answer as today, now with the
  transcript's `Version` too.

### The sync channel

```text
GET /sync?build=<web_build>   WebSocket upgrade
```

```rust
/// What a page sends on the sync channel.
enum PageMessage {
    /// The conversation it shows, as today.
    PageShowing { showing: Option<ConversationId> },
    PagePing { ping: u32 },
}
```

* The channel sends a part of the product state, or a settings list, only
  while the page follows it: whole when it changes, a part with the state's
  version, a list with its ETag. A pane holds its list while it shows. The
  app holds, while it runs, the passkeys, which the password manager's
  signals read, and the automations, which the sidebar marks; a conversation
  holds the subagent profiles its composer lists and the exposed ports its
  panel marks.
* The page reads `GET /state`, then opens the channel, its first message
  following the state with the version it holds; a pane opens with its
  list's GET and follows the list while it is open.
* `PageMessage` gains `Follow` and `Unfollow`, as on a conversation's
  socket.
* `SyncEvent::Snapshot` is removed.
* A conversation the page holds that the conversation list no longer has,
  read or pushed, is taken as deleted, as `ConversationDeleted` does.
* `Pong` and `Heartbeat` stay.

### A conversation's socket

```text
GET /conversations/:id/stream   WebSocket upgrade
```

```rust
enum ClientFrame {
    /// Attaches the connection, following each value the page holds.
    Open { draft: Option<u64>, follows: Vec<Follow> },
    Follow(Follow),
    Unfollow { value: Followed },
    // Every command stays; Subscribe and AgentsSince are removed.
}
```

* `Followed` names the conversation's metadata, its agents' entries, a
  node's transcript, and the saved, workflows and shares lists.
* The server sends only changes of what the page follows: `conversation`
  when the metadata changes, `agent` per changed entry, a followed node's
  `transcript_patch`es, a followed list whole, `stale` and `rebased`.
* `transcript_reset` and the subscription's answer of a page are removed.
* Following a node sends no `cache_chart` or `request_output` it read,
  which keep coming as they change. It sends the view of each of the node's live commands as
  `shell_output`, which no version covers, after the patches.

### Recovery

* A page reads again only what `stale` or a delta it cannot apply names; the
  latter it reports to the server's log.
* A read again after `stale` waits a random delay of up to two seconds, and
  backs off from one second to thirty when it fails, so pages told at once
  do not read at once.

### Rules

* No route and no frame answers a read from a Host, restores a tree or
  starts work; that is the work of the record on Hosts' facts, which this
  one relies on.
* No read waits for a socket, names one, or fails because one is closed.
* A socket never sends what the page did not read first, and never answers a
  follow with data the page holds.
* A page that opens from its offline copy shows it, then follows every
  value it shows with the version the copy holds, as after a break.

## Upgrade and version skew

* A page of the earlier build opens the sync channel without `build`. The
  server answers it with one `Snapshot` that holds only `web_build`, which
  that build already reloads on (`App.vue`), and closes; no other client
  gets a snapshot.
* A page of the earlier build that sends `open` with versions is refused
  with `invalid_frame`, and reloads when the sync channel tells it of the
  new build.
* The offline copy belongs to its build and is dropped with it.
* Nothing stored changes; no data migrates.

## Performance

Expected: a cold start draws after its reads alone, and the socket's
handshake follows them, so changes start to come about one round trip after
the first screen, none lost; a reconnect sends a follow per
value shown and receives only what changed; a subagent shows after one
request of its page, and its context card with none; each read of a node also
carries its chart. A transcript that ran during a short break sends
its patches since; one beyond its recent patches is read again, about a
megabyte for a large one. Memory: at most 1 MiB of recent patches for each
running node. A follow costs the
server the hash of the value's current body, as its GET does.

## Drawbacks

* A view opens with a request where its data came with the socket: one
  round trip when the cache cannot answer.
* A follow of a whole value hashes its current body, as its read does.
* Changes start to come a round trip after the first screen, the socket's
  handshake; a change in that time comes when the socket opens, by the
  version compared.
* Each running node holds up to 1 MiB of recent patches in memory.
* A transcript that ran past its recent patches during a long break is read
  again whole.

## Rationale and alternatives

* Keep the snapshots and make them smaller: the socket still serializes
  every view behind the others, and the browser still cannot cache them.
* No recent patches, a transcript past the version held read again whole:
  a running conversation changes between its read and its follow, so its
  page would read a megabyte again, and again.
* A revision kept on every block, the changed blocks sent: a change of what
  is stored, where a bounded window in memory does the same.
* A read that registers the follow on the page's socket, by the socket's id
  in a header: the first read waits for the socket's handshake, a round
  trip at every cold start, and fails when the socket closed.

## Prior art

* Slack bootstrapped a client from an HTTP call (`rtm.start`) and took
  events from a WebSocket; as teams grew it loaded the minimum up front and
  the rest when a view needs it.
* Linear bootstraps from `/sync/bootstrap`, catches up from `/sync/delta` by
  its last sync id, and takes changes over a WebSocket.
* Figma downloads a file when it opens and syncs only its changes over the
  WebSocket.
* Telegram reads state, dialogs and history by method calls, fills a gap by
  `getDifference`, and pushes only small updates; a backlog too large to
  push is read.
* Kubernetes lists a collection with its `resourceVersion` and watches from
  it; its watch cache keeps a bounded window of recent events, and a watch
  older than the window answers 410 Gone and the client lists again, with
  backoff. The watch registers in the same lock that takes the events since
  the version, so none is lost. Demi's follow is this watch.
* Zulip's `register` creates the event queue and answers the initial state
  in one call, then the client applies the events that came meanwhile; a lost queue makes the client
  start again as it does at startup, after a random delay.
* React Query and SWR read again what a view shows when the network
  returns or the window regains focus.
* HTTP's conditional GET answers "not modified" for the version a client
  holds; a follow does the same on the socket. Demi departs from Linear and
  Telegram, which resume from a log: it compares the version held with the
  current one, so the server keeps nothing for a page that left. It departs
  from Zulip, whose first read registers the event queue: a read here never
  waits for the socket.

## Unresolved questions

* Whether the conversation list in `ProductState` should page, for users
  with thousands of conversations.
* Whether the old page's one-field `Snapshot` is worth keeping, or an old
  page may stay broken until the user reloads it.
