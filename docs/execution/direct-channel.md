# Direct channel

A page and a paired device's runner can talk directly, without the backend in
the middle, when the network lets them. For example, the user runs Demi's
backend on a server far away and works on their own Mac, which is the
conversation's Host. The user opens the File view and a browser tab in the
work panel. Every byte of the file and every picture of the live view would
travel from the Mac to the server and back to the same Mac: about half a
second for a round trip and a few MiB/s. Over a direct channel the same bytes
stay on the Mac: a round trip takes well under a millisecond, and a file
arrives at tens of MiB/s.

```text
relay:   page ──WSS──▶ backend ──WS──▶ runner ──▶ Chrome, files
direct:  page ═══════ WebRTC data channels ═══════▶ runner ──▶ Chrome, files
         page ──WSS──▶ backend ──WS──▶ runner        (the introduction, and everything else)
```

The direct channel is an optimization of a path that always exists. The page
starts every operation on the relay at once, tries the direct channel beside
it, moves to it when it connects, and goes back to the relay when it fails.
Nothing waits for it, and nothing is lost when it never connects: the page
then works exactly as it does without it.

It works on one machine, on one local network, and across networks where
both sides' routers let a connection through, which most home and office
routers do: each side learns the address the internet sees it at from a
public STUN server and offers it, and the two ends' checks open the way
through both routers, as video calls do. It does not work where both
networks map each outgoing connection to a new address (symmetric NAT) or
block UDP; the relay then serves everything, and the device's page says
why.

## What goes direct

Only operations whose bytes are large or whose round trips the user feels,
for a conversation whose primary Host is a paired device:

| Operation | Relay form | Direct form |
| --- | --- | --- |
| A user stream, such as the [live browser view](../browser/live-view.md) | `WS …/streams/<name>` and a service stream | One data channel carrying the same bytes |
| A file's bytes, read or a range of them | `GET …/fs/raw` | A `read` channel |
| A file written by the page, such as an upload | `PUT …/fs/raw` | A `write` channel |
| A file's text | `GET …/fs/file` | A `text` channel |
| A directory listing, a folder made, a file deleted | `GET`, `POST`, `DELETE …/fs` | `list`, `mkdir` and `delete` channels |
| The [file watch](../product/web-api.md#file-watch) | `WS …/fs/watch` | A `watch` channel carrying the same messages |

Everything else stays on the backend: messages and the conversation socket,
the synchronization channel, permissions, the work panel's state, the working
tree's changes, commands and jobs, attachments, and every operation of a Cloud
conversation. A Cloud runs beside the backend, so a direct channel would save
nothing there, and a Cloud is never offered one.

## Making the channel

The page and the runner connect with WebRTC data channels. The backend
introduces them, and only the backend can: a runner accepts no connection it
was not told about.

1. **The signaling socket.** A page that shows a conversation whose primary
   Host is a paired device with a connected runner opens
   `WS /api/devices/:deviceId/direct` ([Direct channel route](../product/web-api.md#direct-channel)),
   one per device, whichever of its conversations it shows. The upgrade
   checks the session cookie, the `Origin` header and that the caller owns
   the device. The backend gives the socket a peer id.
2. **The offer.** The page creates an `RTCPeerConnection` with the STUN
   servers the backend names (`DEMI_STUN_URLS`, Cloudflare's public
   `stun:stun.cloudflare.com:3478` by default; empty turns crossing networks
   off), opens a first data channel so that the offer carries one, and sends
   the offer at once; its candidates follow as `direct_candidate { peer,
   candidate }` as the browser finds them, the public one once the STUN
   server answers. The backend
   forwards the offer to the runner as `direct_offer { peer, sdp,
   introduction }`, where the introduction is what the runner cannot know on
   its own: the user's locale, and each user stream of the plugins the user
   has on, by name, with the package and operation it opens.
3. **The answer.** The runner binds one UDP socket on `127.0.0.1` and one on
   each address it can be reached at on the local network, each on a port
   the system picks, and answers with those addresses as candidates in
   `direct_answer { peer, sdp }`, which the backend forwards to the page;
   it asks the same STUN servers, from its local network sockets, for the
   address the internet sees them at, and sends each it learns as a
   `direct_candidate` too. It runs full ICE, checks to the page's candidates
   included, so both routers see outgoing traffic and let the other side's
   checks in; the pair that answers first is used. Nothing listens on a fixed port, and no router
   is configured.
4. **Connected or not.** The page waits up to 10 seconds for the connection,
   and the runner gives up on a peer that has not connected in that time. A
   peer that never connects costs nothing more than its sockets for those 10
   seconds.

The addresses a runner offers are `127.0.0.1` and the IPv4 addresses of its
interfaces that are up, can broadcast and are not point-to-point. A VPN's
tunnel interface is point-to-point, so a fake address a VPN client gives
(such as `198.18.0.1` of a TUN mode) is never offered: WebKit's encryption
handshake times out over one. `127.0.0.1` serves Chrome and Safari; Firefox
ignores loopback candidates and connects to a local network address.

The encryption keys' fingerprints travel in the offer and the answer, so
each end knows it talks to the one the backend introduced. The channel is
encrypted end to end with DTLS, as every WebRTC data channel is. The STUN
server only answers each side with its own public address: it sees that the
browser and the device asked, and from where, and carries none of their
traffic.

## Who may connect

The backend's introduction is the only check, and it is the check the user's
own device already has everywhere else: the caller owns the device and its
runner is connected, as for browsing the device's folders
([Every way to a Host](sessions-and-targets.md#every-way-to-a-host)). The
runner accepts no peer the backend did not introduce, and everything a peer
then does is the device owner's work on their own device, for any of their
conversations on it: Demi adds no gate of its own per conversation or per
operation. The page names the conversation in each operation, so the runner
acts in that conversation's directory and, for a stream, in its browser.

When the signaling socket closes, the backend tells the runner
`direct_close { peer }`, and the runner closes the peer. When the user turns a
plugin on or off, the backend closes the user's peers the same way, as it
ends the relay's streams of a plugin turned off: the page reconnects at once,
and its next offer carries the new introduction. When the runner's own
connection to the backend ends, it closes every peer. A device revoked, a
session ended or the user signed out closes the signaling socket, and so the
peer. A conversation that no longer runs on this device, after a target
change, an archive or a detach, is one the page no longer sends here: it
reads the conversation's Host from its summary as it does for the relay.

## The browser's permission

Chrome has a local network permission for WebRTC ready behind a flag, as it
already has one for `fetch` and WebSockets. Once it is on, the page's first
attempt makes Chrome ask the user, once per site:

- **Allowed:** the attempt connects, and later ones do without asking.
- **Not answered, or dismissed:** the attempt fails; the page stays on the
  relay and tries again later, which may ask again.
- **Blocked:** the page stays on the relay. Where the browser reports the
  permission's state, the help of an online paired device in Settings →
  Devices (below) says that this browser blocks direct connections to devices
  on this computer and network, and how to allow it in the site's settings. When the user allows it, the
  browser reports the change and the page connects without a reload.

Firefox and Safari ask nothing for a data channel today.

## Operations on the channel

Each operation opens a data channel of its own, reliable and ordered, and the
channel's end is the operation's end. The page's first message on it is a
JSON header naming the conversation and the operation; the runner's first
message is a JSON answer, `{ ok: true, … }` or
`{ error: { code, message } }` with the codes the relay's route answers for
the same failure. Bytes travel as binary messages of at most 64 KiB.

| `op` | The page sends | The runner answers |
| --- | --- | --- |
| `stream` | `{ stream, args? }`, then the stream's input bytes | `{ ok }`, then the stream's output bytes: the same bytes the relay's pipes carry |
| `read` | `{ path, offset?, length?, version? }` | `{ ok, size, version, modifiedAt }`, then the bytes; a `version` the file no longer has answers `file_changed` |
| `write` | `{ path, replace }`, the bytes, then `{ end: true }` | `{ ok }` once the file is in place |
| `text` | `{ path, version? }` | `{ ok, version, unchanged }`, then the text unless it is unchanged |
| `list` | `{ path? }` | `{ ok, path, home, entries }` |
| `mkdir`, `delete` | `{ path }` | `{ ok }` |
| `watch` | `{ paths }` messages, as on the relay | The relay's `state`, `changed` and `heartbeat` messages |

The runner tells the backend `direct_stream { stream, name, conversation,
open }` once it accepts a stream channel's header, naming the stream with an
id it gives it and the user stream's `name` the header asked for, and again
when the channel ends, on every path. The backend then
knows the stream as it knows a stream it opened:

- **Activity.** It is conversation activity, as an open relay stream is
  ([Activity](resource-lifecycle.md#activity)), so a live view watched only
  over the direct channel keeps the conversation from its idle release.
- **Artifacts.** The runner asks the backend for the artifacts the stream
  needs, its service's executable or what its invocation asks for while it
  runs, naming the stream as it names a relay stream
  ([Service streams](runner.md#service-streams)). The backend answers while
  the stream is open, for the artifacts a stream of that name may install,
  as for a relay stream ([Install artifacts](native-runtime.md#install-artifacts)).
  For example, a plugin's service that never ran on the device installs its
  executable when its first stream opens over the direct channel, as it
  would over the relay's.

The runner carries each one out as it carries out the backend's request for
the same thing ([Host operations](runner.md#host-operations)): the same
functions, the same limits, the same atomic writes, the same
[file watch](runner.md#watching-files) shared with the backend's watches. A
`stream` starts the invocation as a `service_open` does
([Service streams](runner.md#service-streams)), with a `user` caller in the
conversation the header names.

Flow control is the data channel's own. The runner keeps at most 256 KiB
queued on a channel and writes more as the queue drains; the page does the
same with `bufferedAmountLowThreshold`. Larger queues measured slower, not
faster. A runner keeps at most 8 peers and 64 open channels per peer, and
refuses more with `busy`, which sends the page's operation to the relay.

## Choosing the path

The page keeps one choice per device: `relay` or `direct`. It is `direct`
while the peer is connected. The user can turn direct connections off for a
device on its page, which every page of the user follows: the device's
`direct` field ([Workspaces, devices, and attached hosts](../product/web-api.md#workspaces-devices-and-attached-hosts));
a page then makes no peer for it, closes the one it has, and the relay serves
everything, until it is turned on again.

- **Each operation chooses as it starts.** A read, a text, a listing or a
  write starts on whatever the choice is at that moment, and finishes there.
- **A stream or a watch moves.** When the choice changes, an open live view
  or file watch reopens on the other path, as it does after a lost
  connection: the view keeps its last picture and resumes over it
  ([A browser tab in the panel](../browser/live-view.md#a-browser-tab-in-the-panel)),
  and the files service treats what it read as unconfirmed until the new
  watch says `live` ([What the service keeps](../architecture/plugin-pages.md#what-the-service-keeps)).
  Moving costs no reconnect wait.
- **A failure goes back to the relay at once.** A channel that fails, or a
  peer whose connection fails, sets the choice to `relay`. A read, text or
  listing that failed on the direct channel runs again on the relay; a write
  that failed is retried once on the relay, since a write that did not finish
  leaves the file as it was; a stream or watch reopens on the relay.
- **The page tries again.** While the choice is `relay`, the page makes a new
  peer when the browser reports that its local network permission changed,
  when it comes back online, when the device's runner connects again, when
  its signaling socket reconnects, and otherwise after 1, 2, 5 and then every
  10 minutes. One attempt runs at a time. So a direct channel that a user's
  permission, a network change or a restarted runner made possible is taken
  up without a reload.

The relay never closes because the direct channel opened: a page that sees
both keeps using the relay for everything that does not go direct, and falls
back to it without waiting.

## Bytes the browser fetches itself

An image, a video, a PDF or a download is fetched by the user's browser from
the raw route's URL, not by the page's code: a `<video>` asks for ranges as it
plays. So the page's service worker answers those requests on the direct
channel. It sees a `GET` of `…/fs/raw` for a conversation on a device the
page is connected to directly, asks the page, which holds the peer, to read the range on a `read`
channel, and answers with the bytes and the headers the relay's route gives
them: the media type of the [file-type table](../architecture/contracts.md#logic-the-web-app-and-backend-share),
`nosniff`, the content policy of an image shown in place, `Range` answers,
the ETag, the cache rule of a `version`ed URL and `Content-Disposition` for a
download ([File text and working tree changes](../product/web-api.md#file-text-and-working-tree-changes)).
A request for a conversation that is not direct, or one the page cannot
answer, goes to the network unchanged. The service worker intercepts nothing
else.

## Liveness

A data channel has no traffic of its own when nothing happens, so each
long-lived operation keeps the heartbeat it has on the relay: the live view's
module every quarter second, the file watch every 30 seconds. The page treats
each as it treats the relay's sockets
([Liveness and reconnection](../product/web-application.md#liveness-and-reconnection)).
WebRTC's own consent checks end a peer whose other end is gone within about
30 seconds; the page sets the choice to `relay` as soon as the peer's
connection state is `failed` or `closed`.

## What the user sees

Nothing changes but speed, and the user can see why. In Settings → Devices
each row names a device and says, in one line, its system and how this page
reaches it, *Connected directly* or *Through the server* with the reason in
a few words, or that it is offline and when it was last seen; it shows no
version, architecture or other code. The row opens the device's page,
`/settings/devices/<id>`, as a row of macOS's System Settings opens its
detail:

- **Connection.** *Connect Directly When Possible*, on by default
  ([Choosing the path](#choosing-the-path)); the status, direct or through the
  server, with the round trip the direct channel measures; when through the
  server, why, in a sentence that also says what the user can do; when the
  last attempt ran and when the next will, with Try Now, which makes a new
  peer at once; and Details, closed by default, with what the last attempt
  saw: the stage it failed at (permission, gathering addresses, finding a
  path, the encryption handshake, opening the channel), how long it took,
  the addresses each side offered, local and public, the pairs tried and how
  many answered, and the browser's local network permission.
- **Device.** The system by its name and version, such as *macOS 26.5* or
  *Ubuntu 26.04*, with the chip family, *Apple silicon*, *Intel* or *ARM*,
  not an architecture code; the runner as *Up to date*, *Update available* or
  *Development build*, never a version code; and when it was paired.
- **Rename…** and **Revoke…**, which leave the list's rows.

The reasons the page gives, from what the attempt saw:

| Reason | When | The page says |
| --- | --- | --- |
| Turned off | The device's `direct` is off | Direct connections are off for this device, and everything goes through the server |
| Blocked by this browser | The browser reports its local network permission blocked | The browser blocks local network access for this site, how to allow it in the site's settings, and that Demi connects without a reload once it is allowed |
| Not reachable | Every pair was checked and none answered, and both sides found their public address | The two networks don't let a direct connection through, as with strict NAT or a firewall, and the addresses tried |
| This network blocks it | One side found no public address: its network blocks UDP or the STUN server | Which side's network blocks it, the browser's or the device's |
| Device is busy | The runner refused the peer with `busy` | The device's runner has too many connections open, and that Demi tries again |
| Connection dropped | A connected peer failed | When it was direct until, and that Demi tries again |
| Not offered | Crossing networks is off (`DEMI_STUN_URLS` empty) and no local pair answered | Direct connections work only on the same network as the device on this server |

The Cloud is always reached through the server, since it runs beside the
backend; its page says so and offers no switch. A user who blocks the
browser's local network permission sees no prompt again and stays on the
relay.

## Failure and limits

| Situation | What happens |
| --- | --- |
| The browser and the Host are on networks that do not let a direct connection through (symmetric NAT on both, or UDP blocked) | The peer never connects; the relay serves everything, and the device's page says why |
| Two machines on one local network | The peer connects over the network's addresses, as on one machine |
| The browser asks for local network permission, and the user has not answered | The relay serves everything; a later attempt or a permission change tries again |
| Windows asks whether the runner may receive connections | The runner's local network sockets are blocked until the user allows it; `127.0.0.1` still connects in Chrome |
| A policy or VPN that forbids direct UDP | The relay serves everything |
| The runner restarts | Its peers close; the page falls back and tries again when the runner connects |
| The browser's local network permission is blocked | The relay serves everything; Settings → Devices says how to allow it, and allowing it connects without a reload |

## Responsibilities

| Where | Responsibility |
| --- | --- |
| `runner-protocol` | The `direct_offer`, `direct_answer`, `direct_candidate`, `direct_close` and `direct_stream` messages and the operations' headers and answers |
| `runner-direct` | The runner's peers: sockets, ICE, DTLS and SCTP through str0m, and each operation carried out through `runner-host` and the service streams the runner supplies; on its own thread, off the runner's control thread, since a peer at full speed fills a core |
| `runner` | Composing `runner-direct` with the Host operations and service streams, and closing every peer when the backend connection ends |
| `backend-http` | The signaling route, with the device access check |
| `web` | The peer, the choice of path, the operations' clients, the service worker, and the user streams and file reads the page context supplies over either path |
| `web-ui` | The devices list's rows and the device's page with its Connection section, reasons and details |

## Rationale

- **WebRTC, not a server on `localhost`.** A page from a public origin that
  connects to `http://127.0.0.1` is blocked as mixed content in Safari, and
  Chrome asks for local network permission first for `fetch` (Chrome 142)
  and for WebSockets (Chrome 147). A server listening on a fixed port also
  answers every web page and every local program that finds it. A WebRTC
  data channel listens on nothing until the backend introduces a page, needs
  no certificate, and reaches a machine on the local network too.
- **Measured on one Mac** (Chrome 153 and 154, Firefox 155, WebKit 26.6, a VPN
  in TUN mode active): the channel opened in 13 to 25 milliseconds in every
  browser; a round trip took 0.08 to 0.22 milliseconds, against 420 to 520
  through a server about 230 milliseconds away; and 64 KiB messages moved at
  45 to 99 MiB/s, against 4 to 5 MiB/s through that server.
- **The relay first.** Chrome has a local network permission for WebRTC
  ready behind a flag; once it is on, a page may need the user's permission.
  Starting on the relay and moving later keeps that prompt, a firewall or a
  policy from ever slowing or breaking the page.
