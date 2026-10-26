---
status: proposed, superseded in part by 0126
---

# A page reaches the backend over one socket

## Problem

Alex works in five conversations whose agents are running, has shown eight
conversations recently, has two paired laptops online and watches the Cloud's
browser in a live view. The page holds 17 WebSockets to the backend: the
synchronization channel, a conversation socket for each of the 13
conversations it keeps (0090), a file watch for each of two of them, a
signaling socket for each laptop's direct channel, and the live view's user
stream.

Alex's laptop wakes from sleep, or the Wi-Fi changes. Every socket is lost at
once, and the page opens all 17 again at once. Then:

1. **The browser opens them one at a time.** RFC 6455 §4.1 allows one
   WebSocket to a server in the connecting state at a time, and Chromium
   holds the next one until the previous handshake completes
   (`WebSocketEndpointLockManager`). Firefox does the same.
2. **Each handshake takes about 0.75 seconds** for a server on another
   continent: a TCP connection, TLS and the upgrade, more than two round
   trips of about 250 ms each.
3. **The tenth second ends the rest.** A socket whose handshake has not
   completed within 10 seconds counts as failed, and the 10 seconds run from
   the moment the page asked, so the time spent waiting in the browser's line
   counts. Thirteen open; the last four fail, wait, and are tried again, and
   they meet the line again.
4. **One stalled handshake fails all 17.** A handshake that never completes,
   as through a network that has not settled after the wake, holds the line
   for its 10 seconds, and every socket behind it fails with it.
5. **Each conversation without its socket spins** (0103), since the page
   cannot tell what its session holds now.

So the page keeps trying again, the conversations spin all the while, and
one more stalled handshake starts it over. On a healthy network, every
conversation the page opens still waits a full handshake before its socket
follows it.

## Decision

After a drop, the page is back once one connection is back: its
conversations spin for the time of one handshake, about a second, however
many the page keeps. A conversation the page opens follows its session one
round trip after it asks, with no handshake of its own.

```
today: 17 handshakes, one at a time       this record: one handshake
  sync          |==|                         connection |==|
  conversation1     |==|                     17 lanes       |-| one round trip
  ...                      ...                                  each, together
  conversation13               |==|
  user stream                       |x 10 s, failed, retried
```

### The page connection

Each page holds one WebSocket to the backend, **the page connection**,
`WS /api/connect?page=<page id>`. It carries everything the page's
WebSockets carry today, each in a **lane** of its own:

| Lane | Today | Names |
| --- | --- | --- |
| `sync` | `WS /api/sync` | nothing |
| `conversation` | `WS /api/conversations/:id/stream` | the conversation |
| `file_watch` | `WS /api/conversations/:id/fs/watch` | the conversation |
| `device_watch` | `WS /api/devices/:id/fs/watch` | the device |
| `signaling` | `WS /api/devices/:id/direct` | the device |
| `user_stream` | `WS /api/conversations/:id/streams/:name` | the conversation and the stream |

Each page, as each browser tab, has a connection of its own. A lane
carries the messages its socket carries today, text and binary, in
the same order, with the same meaning. HTTP requests stay requests, and the
direct channel stays WebRTC; only its signaling moves into a lane. The
runner's link and pipes are not page sockets and do not change.

The routes of the table are removed, except `WS /api/sync`, which tells
every page that the product state it holds is stale and closes. A page
loaded before the release then reads the state again, which names the web
build the backend serves, and reloads into it.

### The wire

Every message of the connection is a binary WebSocket message, a frame, whose
first byte says its kind. Numbers are big-endian.

```text
byte 0  kind
  0  connection   JSON: PageConnectionMessage or BackendConnectionMessage
  1  open         u32 lane, JSON LaneTarget                   page -> backend
  2  opened       u32 lane                                    backend -> page
  3  refused      u32 lane, JSON LaneRefusal                  backend -> page
  4  data         u32 lane, u8 flags (bit 0 binary, bit 1 more follows), piece
  5  close        u32 lane, u16 code, UTF-8 reason
  6  credit       u32 lane, u16 messages                      backend -> page
```

```rust
// web-api-protocol::connection
pub enum PageConnectionMessage {
    Ping { id: u32 },
}
pub enum BackendConnectionMessage {
    Pong { id: u32 },
    Heartbeat,
}
pub enum LaneTarget {
    Sync,
    Conversation { conversation: String },
    FileWatch { conversation: String },
    DeviceWatch { device: String },
    Signaling { device: String },
    UserStream { conversation: String, stream: String },
}
pub struct LaneRefusal {
    pub status: u16,
    pub error: ErrorBody, // what the route answered, with the open's request ID
}
```

The page needs no word of builds on the connection: the state it reads
names the backend's web build, which a page of another build reloads into.

- **Lanes are numbered by the page**, from 1 in each connection, and a
  number is never used twice in a connection. A message for a lane that has
  closed is dropped, so a late frame of a conversation shown before can
  never reach the same conversation opened again.
- **An open is answered at once**: `opened`, or `refused` with the status
  and the error body the route answers today before its upgrade (404 for a
  conversation that is not the user's, 409 `conversation_archived`,
  `device_offline`, `not_a_paired_device`, 404 `unknown_stream`), and the
  request ID of the open, which the page's error report shows (0062). Each
  open is checked in a task of its own, so a slow one, such as a Host that
  opens a user stream, holds up no other lane. The check reads the session as
  it is now, so an account that chose its password since the connection
  opened opens its other lanes; one still to choose has its `sync` lane,
  which carries the account alone, and its other lanes refused with
  `password_choice_required`, as its sockets are today.
- **Either side closes a lane** with the code and reason its socket closes
  with today: 4001 for a conversation's page that fell behind, 1000
  `superseded`, 1011 `internal_error`, 1007 `not_json`. A lane's close never
  closes the connection.
- **What ends the session ends the connection**: 4002 `session_ended` when
  the session is signed out or expires, 1001 `backend_closing` when the
  backend stops. The checks that each socket made of its session, the
  connection makes once: it watches the session's sign-out and its expiry.
- **A page that breaks the protocol loses the connection** with 1002
  `protocol_error`: a text message, a lane number that is not new, data for
  a lane that never opened or before it opened, or a kind it does not know.
  An open whose target does not read is refused with 400 `invalid_body`.
- **A message larger than 16 KiB goes in pieces** of 16 KiB, flagged `more`
  until its last, and the receiver puts each lane's pieces together. One
  lane's pieces follow each other, and other lanes' messages go between
  them. A page's message stays at most 1 MiB once whole.

### Fairness and back pressure

- **The backend writes in this order**: the connection's own messages, a
  `pong` first; then the lanes' opens, closes and credits; then the pieces
  of the `sync`, `conversation`, `file_watch` and `signaling` lanes, one
  piece of each lane in turn; then the user streams' pieces, in turn. An
  owner's send resolves once its message is written, as it does today, so a
  page that stops reading stops each owner as its socket did, and a
  conversation that falls a full outbox behind closes its lane with 4001.
- **A live view does not hold up a conversation.** A live view keeps only a
  window of bytes unacknowledged, its target rate times the least round trip
  plus 60 ms (`crates/live-view/src/rate.rs`), so the bytes ahead of a
  conversation's message are at most about 60 ms more than the path holds.
- **The page writes in the same order** and puts a piece on the socket only
  while `bufferedAmount` is below 64 KiB, so a large message waits behind
  nothing it was not queued behind.
- **The backend never stops reading the connection for one lane.** Each lane
  may have 8 messages that its owner has not taken, today's
  `MAX_HELD`; the backend sends `credit` as its owner takes them, and a page
  that sends past its credit has its lane closed with 1008
  `credit_exceeded`. A message of the page's larger than 1 MiB once whole
  closes its lane with 1009 `message_too_big`, and one that is not UTF-8
  text with 1007 `not_utf8`.
- **A connection holds at most 64 lanes**, so that a page's open lanes and
  what they hold are bounded; a page holds 18 at most today. An open past
  them is refused with 429 `too_many_lanes`.

### Liveness and reconnecting

The rules of `packages/web-ui/src/transport/liveness.ts` apply to the
connection alone: the backend's heartbeat after 30 silent seconds, the page's
75-second silence, the 10-second handshake, the doubling wait from 1 to 30
seconds, the ping at the page's return and the second of offline that counts.
Lanes have no heartbeat or ping of their own; the direct channel's relay
probe stays a message of the `signaling` lane, as it is not about the
connection.

- **A lost connection closes every open lane** with the connection's close:
  1006 when it broke, 1001 or 4002 when the backend closed it. Each owner sees
  its lane closed as it sees its socket closed today, and waits to open it
  again as it does today.
- **Its lanes open again as soon as it is back**, all at once: as the
  connection counts as connected again, every owner's wait ends, as the
  page's return ends it today, and an open costs the backend one message,
  not a handshake. A lane asked for while the connection is down waits for
  it; a lane still opening as it is lost is sent again on the next one with
  a new number. Its 10 seconds to be answered count from when its open goes.
  Each owner follows from what it holds, as it does after a reconnect today.
- **A lane the backend refused or closed** while the connection stays waits
  its owner's own time before it opens again, as a socket does today.
- **Where the page's connection stands**, online, reconnecting or offline
  (0059), comes from the connection instead of the synchronization channel:
  its failed attempts in a row and whether the backend said it restarts,
  beside the state's own reads that failed.

### The backend

```text
page == WS /api/connect ==> edge: authenticate, read and write the connection
                                   |-- sync, conversation lanes --> user's shard
                                   `-- file_watch, signaling,  ---> edge relays,
                                       user_stream lanes            as today
```

- The edge serves the connection: it authenticates the upgrade, runs one
  reader and one writer task, and opens each lane by the checks its route
  makes today, now functions the routes and the opens share.
- `PageSocket` and `PageStream` become a lane's two halves, `Lane` and
  `LaneReader`, with the operations their owners call today, so
  `serve_sync_channel` and `serve_conversation_lane` take a lane instead of
  a `WebSocket`, and the shard still owns what it owns. A lane dropped
  without its owner's end, as one a closing shard never took, ends as it
  drops, which the page hears as a lane that broke.
- The log has `page_connection.opened` and `page_connection.ended` under the
  upgrade's request ID, with the page, the client's address, Cloudflare's ray
  and the user agent, and `lane.opened`, `lane.refused` and `lane.ended`
  under each open's own request ID, with the fields `page_socket.*` has
  today. Each lane logs in a span of its own, method `LANE` and path
  `/lanes/<kind>`, as a request's, so its open's request ID finds every line
  of its work. The resource sample counts connections and lanes.

### The page

```ts
// packages/web-ui/src/transport/page-connection.ts
/** A lane, with the part of WebSocket's interface its owners use. */
export class Lane extends EventTarget {
  readyState: number
  binaryType: BinaryType
  send(data: string | ArrayBuffer | ArrayBufferView): void
  close(code?: number, reason?: string): void
  /** Why the backend refused the open, when it did. */
  refusal: LaneRefusal | null
}

export function openLane(target: LaneTarget, name: string): Lane
/** The web app tells the connection its address and what learns why an upgrade failed. */
export function configurePageConnection(options: { url: () => string; failedToOpen: () => void }): void
```

The synchronization channel (`state/product.ts`), the conversation client's
transport (`transport/conversation-lane.ts`), the file watches
(`conversation/file-watch.ts`), the signaling (`direct/signaling.ts`) and
the user streams (`transport/user-stream.ts`) open a lane where they open a
socket, and keep what they do with it, without the silence watches and
pings of their own. A refusal says why, which the browser hides of a failed
upgrade. The connection's own upgrade still hides it, so a connection that
fails before it opens asks `/auth/me`, as the synchronization channel does
today, and an ended session ends the page. A direct channel's file watch
keeps its heartbeat and silence watch, since no lane carries it.

`@demicodes/conversation-client` keeps its transport over anything shaped as
a WebSocket, which a lane is. The product-check tool tells the connection's
frames by their lane (`docs/delivery/browse.md`).

### Names

The page's sockets become lanes, so the names that call them sockets change
with them: the conversation socket becomes the conversation lane
(`conversation-socket-protocol` becomes `conversation-lane-protocol`,
`ConversationSocketError` `ConversationLaneError`), the direct channel
signaling socket the signaling lane, and the page log's `socket.*` lines
`connection.*` and `lane.*`.

*Lane* then names this alone. What the code called a lane before is a queue
that something takes turns in, and takes a name that says so: the messages a
runner's link keeps for its next connection are its backlog
(`runner-protocol::wire::Backlog`), a mobile device's operations take turns
in queues, and the product-check tool's forwarder carries each way of a
connection as a direction.

The lanes' own heartbeats and pings go: `heartbeat` and `pong` leave the
conversation's frames, the synchronization channel's messages and the
signaling's, and `PagePing` leaves the protocol. A direct channel's file
watch keeps its `heartbeat`.

## Spike

Chromium, through `bun browse`, opened 17 WebSockets at once to a local
server that answered each upgrade after a delay:

| Case | What happened |
| --- | --- |
| Each upgrade answered after 700 ms | opened one at a time, at 0.72, 1.43 … 9.30 s; the last 4 failed at the 10-second limit |
| The first upgrade never answered, the rest after 300 ms | all 17 failed at 10.0 s |

An HTTP request over the page's existing connection costs one round trip,
which is what a lane's open costs.

## Prior art

- [Phoenix](https://github.com/phoenixframework/phoenix) channels
  (`assets/js/phoenix/channel.js`, `socket.js`) carry every topic of a page
  over one socket. A channel's join is answered `ok` or `error`, each join
  has a reference and messages of an outdated join are dropped, and every
  channel rejoins when the socket reconnects; the socket alone has the
  heartbeat. Demi's lanes follow this.
- [home-assistant-js-websocket](https://github.com/home-assistant/home-assistant-js-websocket)
  (`lib/connection.ts`) keeps every subscription on one socket and
  subscribes each again after a reconnect.
- HTTP/2 ([RFC 9113](https://www.rfc-editor.org/rfc/rfc9113) §5) and SSH
  channels ([RFC 4254](https://www.rfc-editor.org/rfc/rfc4254) §5.2) bound
  what a receiver holds of each stream with a window the receiver extends,
  and interleave frames of several streams on one connection. Demi counts
  messages instead of bytes, since a lane's messages are bounded already and
  the owner takes them whole.
- 0067's runner link numbers its messages and resends what the other end
  did not get. Lanes do not, because each lane's protocol already follows
  from the versions the page holds after a reconnect.

## Alternatives

- **Queue the sockets on the page**, the shown conversation first, with the
  10 seconds counted from when a socket's own handshake starts. The shown
  conversation is back after one handshake, but the page needs 17 to be
  whole again, and every conversation it opens still waits for one.
- **A longer handshake limit.** The sockets fail later instead.
- **WebSockets over HTTP/2** ([RFC 8441](https://www.rfc-editor.org/rfc/rfc8441)),
  which share the page's connection. Every proxy between the page and the
  backend would have to support it, and nginx does not.
- **Server-sent events and requests.** They share the page's connection, but
  carry text alone, a live view's video encoded larger by a third, and each
  of the page's messages becomes a request of its own, ordered by nothing.
- **Keep the user streams on sockets of their own**, apart from the page's
  other traffic. A stream's handshake that stalls would still hold the
  connection's next handshake behind it in the browser's line.

## Open questions

None.
