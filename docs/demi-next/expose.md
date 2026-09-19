# Host expose

An expose gives a service listening on a device a public URL for one hour.
Anyone who has the URL reaches the service; the backend relays their requests
through the device's runner connection, so the device needs no inbound
network access. This document owns the expose record, its lifetime, the
public relay, and the `demi host expose` commands. The runner's network stream
belongs to [Runner](runner.md#network-streams); device access from outside a
conversation belongs to
[Sessions and targets](sessions-and-targets.md#host-operations).

## Purpose

An agent starts a development server on Cloud and runs:

```text
$ demi host expose add 127.0.0.1:5173
Exposed 127.0.0.1:5173 on cloud as https://k7x2m9qw4p3s6t8v0w2y4z6a8b.expose.demi.example/
Expires in 60 minutes (expose k7x2m9qw4p3s6t8v0w2y4z6a8b).
```

The agent puts the URL in its reply. The user opens it in their own browser,
or sends it to someone else; both reach the server the same way. The URL
works for one hour, longer if renewed, and stops working when the expose is
destroyed. Nothing about Demi is visible to the page or to its visitors: the
relay carries HTTP and WebSocket bytes and nothing else.

```text
Visitor                      Backend                              Runner on the device
  |                             |                                        |
  |-- GET https://k7x2….expose.demi.example/ ---------------------------->|
  |                             | Host header -> expose record           |
  |                             |   (not expired, device online)         |
  |                             | net_open 127.0.0.1:5173 + two pipes    |
  |                             |--------------------------------------->|
  |                             |                                        | connect 127.0.0.1:5173
  |                             |<-- request bytes / response bytes ---->|
  |<-- response ----------------|                                        |
```

This is a different thing from the [conversation browser](browser.md): that
is a headless Chrome the agent operates on the Host, and its pages are never
shown to anyone. An expose puts the user's own browser, or anyone's, in front
of the service.

## The expose record

An expose belongs to a user and a device, not to a conversation. It records:

| Field | Meaning |
| --- | --- |
| `id` | 128 random bits as 26 lowercase base32 characters; a DNS label. It is the only credential: whoever knows it reaches the service. |
| `userId` | The owner. Only the owner lists, renews or removes it. |
| `deviceId` | The paired device or the user's Cloud device the traffic goes to. |
| `address` | The `host:port` the runner connects to, exactly as given. A bare port means `127.0.0.1:<port>`. Any host name or address the device can resolve and reach is accepted; the device's own network is the boundary, not Demi. |
| `createdAt`, `expiresAt` | Creation time and the moment the expose is destroyed. |

The public hostname is `<id>.<expose domain>`. The expose domain is instance
configuration ([deployment](#deployment)); when none is configured the
feature is unavailable, `add` says so, and the product shows no expose
controls. The URL the commands and the API print takes its scheme and
port from the backend's configured public URL, the one runners connect
to: behind a reverse proxy on the default port the URL has no port, and a
local backend on `3271` prints `http://<id>.expose.localhost:3271/`.

A conversation names the device when it creates the expose: the conversation's
main Host, or an attached Host through `--host`. After that the conversation
plays no part. A target switch, archive, or Fork changes nothing about an
expose; a conversation release does not reach it, because it holds nothing on
the device.

## Lifetime

An expose lives **one hour** from creation. `renew` moves `expiresAt` to one
hour from now; there is no cap on renewals. Expiry destroys the record;
nothing revives it. Whoever needs the service reachable again creates a new
expose and gets a new URL.

The record is destroyed by exactly these events:

| Event | Result |
| --- | --- |
| `expiresAt` passes | Destroyed. A request that arrives after it is refused as unknown. |
| `demi host expose remove <id>`, or the product's remove | Destroyed at once. |
| The Cloud device leaves the running state: idle stop, lifetime cap, reset, backend close | Every expose on that device is destroyed with the machine. |
| The paired device is revoked | Every expose on it is destroyed with its attachments. |

A paired device that is offline keeps its exposes until they expire: the
runner may reconnect within the hour. Requests answer `device_offline`
meanwhile.

Expose traffic is retention, not activity, in the sense of
[Conversation idle and Host resource release](resource-lifecycle.md): a
visitor loading pages never postpones a Cloud stop, and a request never wakes
a stopped Cloud. Cloud and the expose share the same one-hour clock on
purpose: an expose on a Cloud that nobody's agent is using ends with the
machine, and creating it required a running machine to begin with.

Open connections end with the record. Destroying an expose closes every
relayed connection on it; the runner sees the pipes end and closes its
sockets.

## The public relay

The backend answers every request whose `Host` header is `<id>.<expose
domain>`. Such a request never reaches the product routes, and an expose
hostname is never served on the product origin. Expose hostnames carry no
Demi session and set no Demi cookie; the page and the visitor are anonymous
to Demi.

For each visitor connection the backend:

1. Reads the `id` from the `Host` header. Unknown, malformed, or expired
   answers 404 with a short plain page saying the expose does not exist.
2. Checks the device is connected. Offline answers 502 with a page naming the
   reason (`device_offline`); it does not wait for a reconnect.
3. Opens a [network stream](runner.md#network-streams) to `address` through
   the device's Host, as [device access](sessions-and-targets.md#host-operations)
   allows: no conversation, no file gate, no wake. A connect failure answers
   502 with the runner's reason (`refused`, `unreachable`, `resolve_failed`,
   `timeout`).
4. Relays the HTTP/1.1 exchange onto the stream: the request line and
   headers, then the request body as it arrives, then the response status,
   headers, and body as they arrive. Bodies are never buffered whole;
   chunked and event-stream responses reach the visitor as the service
   sends them. The relay ends the stream's input, which the runner turns
   into the socket's half-close, only once the response is complete: many
   servers, Bun's among them, abort a response still streaming when the
   client's side closes first.
5. Relays a WebSocket upgrade message by message: the backend accepts the
   visitor's upgrade with its server runtime first, then opens the stream
   and performs the client handshake with the service, holding the
   visitor's early frames until the service answers 101; a service that
   refuses the handshake, or cannot be reached, closes the visitor with
   code 1011 and the reason. After that it passes every text and binary
   message, and the close code and reason, in both directions. The server
   runtime offers no raw socket after an upgrade, so the frames are
   re-emitted rather than copied; the payloads are unchanged. Upgrading
   first is required, not a choice: frames a visitor sends while its
   request still waits in the server runtime's HTTP state crash Bun 1.4.2
   at the later upgrade.

The relay rewrites `Host` to the expose's `address`, adds
`X-Forwarded-For`, `X-Forwarded-Host` (the expose hostname) and
`X-Forwarded-Proto`, and removes hop-by-hop headers. It changes nothing else:
no caching, no compression, no HTML rewriting, no injected scripts. A
service that builds absolute URLs from `Host` therefore builds them with its
local address; one that honors the forwarded headers builds public ones.

Limits, defined once here:

| Setting | Value |
| --- | --- |
| Concurrent relayed connections per expose | 64; beyond it 503 |
| Connect timeout on the device | 10 seconds |
| Connection with no bytes in either direction | Closed after 10 minutes |
| Request header block | Bun's server default |

The connection limit is the only limit that refuses work for load: the relay
faces the internet, so it sheds load instead of queuing it. Each relayed connection
holds three of the device runner's open files, its socket and two pipes, so a
busy expose can hold three quarters of a 256-file allowance; other work on the
device then waits for them ([Load](runner.md#load)).

A body on a `GET` request never reaches the relay: the server runtime drops
it before the handler runs. No HTTP feature depends on one.

Each relayed connection is one network stream on the runner and two pipes
through the pipe broker ([Runner](runner.md#pipes-and-output)); a visitor's
browser typically holds a handful. The connection count is the resource
budget; there is no byte budget.

## Commands

`demi host expose` is a subcommand group of the backend-contributed
`demi host` group ([Sessions and targets](sessions-and-targets.md#attached-hosts)),
declared through the [command contract](commands.md). Its leaves are `rpc`:
the backend owns the records.

```text
$ demi host expose add 5173
Exposed 127.0.0.1:5173 on laptop as https://k7x2….expose.demi.example/
Expires in 60 minutes (expose k7x2…).

$ demi host expose add 127.0.0.1:8080 --host ci
Exposed 127.0.0.1:8080 on ci as https://m3n5….expose.demi.example/
Expires in 60 minutes (expose m3n5…).

$ demi host expose list
Expose   Device  Address         Expires  URL
k7x2…    laptop  127.0.0.1:5173  58 min   https://k7x2….expose.demi.example/
m3n5…    ci      127.0.0.1:8080  60 min   https://m3n5….expose.demi.example/

$ demi host expose renew k7x2…
Expose k7x2… expires in 60 minutes.

$ demi host expose remove k7x2…
Removed expose k7x2…; its URL no longer works.
```

| Leaf | Input | Behavior |
| --- | --- | --- |
| `add <address> [--host <name\|id>]` | `address` is `host:port` or a port; `--host` names a main or attached Host as `demi host list` shows it | Creates the record for that device. The device must be connected: an expose for a stopped Cloud would already be destroyed. Prints the URL, the device, and the expiry. |
| `list` | none | Every expose of the user across devices, soonest expiry first, `--json` available. A Cloud that has stopped has none. |
| `renew <id>` | an expose id | Sets the expiry to one hour from now. |
| `remove <id>` | an expose id | Destroys it. |

An id that is not the user's, or that has expired, answers `expose_not_found`
on every leaf. An `add` without a configured expose domain answers
`expose_unavailable`. The commands print full ids; the examples abbreviate.

## Product surface

While the user has a live expose, the conversation header shows a session
tools button between the host menu and the work panel toggle; without one the
button is absent, and it leaves with the last expose. Its menu is the only
place the browser shows exposes. The menu lists every live expose of the
user, across devices, soonest expiry first: the address, the host it is on,
and a countdown that ticks. Choosing a row opens the URL in a new browser tab
of the conversation's [work panel](web-application.md), opening the panel if
it is closed; that tab carries the expose glyph of the button instead of the
browser's globe. Each row carries a renew control, which moves the expiry an hour out, and a remove
control, which destroys the expose at once. The button's icon carries a small
green dot on its top-right corner, so a forgotten URL is visible without
opening the menu. The devices settings do not list exposes.

The `GET /api/state` snapshot carries the exposes and the domain, and the Web
API adds create, renew and remove ([Web API](web-api.md#exposes)). The agent
prints the URL into the transcript, where it is a link; that link is where a
URL is copied from.

The behavior lives in `web-ui` with gallery specimens; `web` supplies the
snapshot, the host names and the request handlers.

## Deployment

`DEMI_EXPOSE_DOMAIN` names the domain under which expose hostnames live, for
example `expose.demi.example`. The deployment provides a wildcard DNS record
for `*.<domain>` pointing at the reverse proxy, a wildcard certificate, and a
proxy rule that forwards every `*.<domain>` request to the backend with the
`Host` header preserved and WebSocket upgrades allowed. The backend derives
the scheme of the URLs it prints from the same forwarded-protocol handling
its session cookie uses.

For local development, Chrome and Firefox resolve `*.expose.localhost` to the
loopback address without DNS; Safari does not. The backend's public URL
supplies the port, so no proxy is needed locally.

In the multi-worker deployment, the proxy must route an expose hostname to
the worker owning that user. The hostname carries the expose id and nothing
else, so the route map needs an id-to-user lookup or a hostname that carries
the routing key. This is an open deployment decision recorded with the others
in [Backend](backend.md#deployment-and-user-ownership).

## Out of scope

- Domains the user configures (`*.my-company.example`): they need the
  user's DNS and certificate; a later design.
- Any access protection on the expose: password, allowlist, Demi login. The
  id is the credential.
- Discovering what listens on the device. The agent knows what it started.

## Rationale

The record belongs to the device because that is what the traffic reaches;
binding it to the conversation that created it would make a URL stop working
when the user archives an unrelated chat, and would hide it from the next
conversation on the same machine.

The relay carries bytes and the runner opens a socket because that is the
smallest thing that makes every HTTP feature work: streaming, server-sent
events, WebSocket, HTTP/1.1 keep-alive. Parsing HTTP on the runner would
mean two HTTP implementations agreeing on every detail, and would still not
give WebSocket for free.

There is no protection on the URL because the feature exists to hand a link
to someone; the one-hour lifetime and the random id bound the exposure, and
the user or agent can remove it at any moment.

Traffic is not activity because anonymous traffic must not decide when a
user's machine runs or costs money. A URL pinned in a monitoring bot would
otherwise keep a Cloud alive indefinitely.

## Acceptance

Scenario tests with a scripted provider and a real runner; a real Chrome for
the WebSocket check is not required, a WebSocket client is. Never a real
model.

1. `add` on a paired device and on Cloud prints a URL; `curl` of it returns
   the fixture service's response with `Host` rewritten and the forwarded
   headers present; the fixture records what it received.
2. An 8 MiB request body and an 8 MiB response body arrive byte-equal with
   the relay's memory bounded; a server-sent-event stream reaches the client
   as events are emitted.
3. A WebSocket echo through the relay carries text and binary both ways and
   propagates close codes in both directions.
4. Expiry: with an injected clock, a request one second after `expiresAt`
   answers 404 and the record is gone; `renew` before expiry extends it.
5. Cloud stop destroys the exposes on that device; a paired device going
   offline keeps them and answers 502 until reconnect.
6. Another user's session cannot list, renew or remove the expose; the URL
   itself works without any session.
7. `remove` while a connection is open ends that connection; the runner
   leaves no socket behind.
8. The concurrent-connection limit answers 503 for the 65th connection and
   admits again after one closes.
9. Without `DEMI_EXPOSE_DOMAIN`, `add` answers `expose_unavailable` and the
   product shows no expose controls.

Acceptance on a paired device and on Cloud, as every Host change requires.
