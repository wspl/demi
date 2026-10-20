# Port exposure

A user runs Ollama on their Mac, at `localhost:11434`, and wants a provider
entry to use it from any conversation, Cloud ones included. The user
exposes port 11434 of the Mac. The backend's own requests can then reach it
at `http://<the Mac's device id>:11434`: a Compatible API entry with the
endpoint `http://3f1c…e2a0:11434/v1` chats with the Mac's models. Nothing
listens anywhere; only the backend's requests for the user get through,
and only to the ports the user exposed.

```text
backend ── request to http://<device id>:11434 ──▶ Mac runner ──▶ 127.0.0.1:11434
```

Exposure is for the backend's requests, such as a provider entry's; a
program on one of the user's Hosts reaching another Host's port uses a
[port forward](port-forwarding.md) instead. The backend listens on no port
for either: on a shared instance every user's programs on the server could
connect to it.

## Exposed ports

A device's page in Settings › Devices has **Exposed Ports**: a list of
ports, each with an optional label, such as *11434 · Ollama*, and its
address, `http://<device id>:11434`, with a copy button. **Expose Port**
adds a row with the port missing. A port is exposed until the user removes
it, and a Cloud's ports can be exposed as a device's are.

- **The address.** The host of the address is the device's id, as a
  container's name is its host on a Docker network: it is stable while the
  user renames the device, and no name on the internet has that form, so it
  never shadows a real host. A request the backend makes for the user to an
  `http` or `https` address whose host is one of the user's device ids goes
  to that device's runner instead of the network.
- **What the runner does.** It makes the request to `127.0.0.1:<port>`
  with its own HTTP client, the method, path, headers and body as given,
  and streams the answer back, its status and headers included; the
  request and the answer travel as pipes over the relay
  (`crates/runner-jobs/src/tasks.rs`). An `https` address is
  made to `https://localhost:<port>`, connected to 127.0.0.1 whatever
  `localhost` resolves to on the device. The runner follows no redirect and
  drops the hop-by-hop headers, those `Connection` names included. The
  answer's pipe is open before the runner sends the request, so a service
  that takes minutes to answer, as Ollama loading a model does, is waited
  for as long as its caller waits; when the caller stops waiting, a turn
  stopped or a test ended, the runner abandons the request and closes its
  connection to the port, answered or not. An answer the relay cannot carry,
  such as one with more headers than it takes, fails the request with why.
- **Redirects.** A redirect the network answers is followed as the network's:
  an address naming a device in it is an unknown host there, so no
  redirect reaches a device.
- **Only exposed ports.** A request to a port the device has not exposed
  fails with *Port 8080 of MacBook Pro isn't exposed*, shown where the
  request was made, with **Expose Port**, which exposes it and makes the
  request again: a failed turn's message in the conversation, as a signed-out
  provider's is, and on the entry's page the result of **Test Connection**.
  Nothing is shown before a request fails. So a provider entry, a pasted URL or an agent's command can never
  reach anything else on the device or its network.
- **Whose.** A device's exposed ports serve the requests made for its
  owner: their own entries, and on a shared instance an administrator's
  shared entry naming the administrator's device, which serves every user
  of that entry, as the entry's page says beside its endpoint. An address
  naming another user's device is an ordinary, unknown host.
- **The device is offline** or, for a Cloud, stopped: the request fails at
  once with *MacBook Pro is offline*, and a turn using the entry fails with
  that headline, not the unreachable endpoint's, since the user's next step
  is to bring the device online; it is retried as an unreachable endpoint
  is. A stopped Cloud is not woken for it, as no
  request to a provider wakes one.

Every HTTP request the backend makes to an endpoint a user set, a turn,
its model list, an image, a video or audio, goes through the one HTTP client
that knows these addresses, so each works with an exposed port without a
change of its own; a vendor's fixed endpoint, such as Codex's WebSocket, is
never a device. Removing a port ends the requests under way through it, as
turning a forward off ends its connections.
- **Recovery.** Expose Port makes sure the port is exposed, which it may
  already be, exposed meanwhile from Settings or another tab, and then makes
  the request again; a failure shown for a port that is exposed by now
  offers Try Again instead, as the page learns of the change when it is
  made.

## From a conversation

`demi host expose` manages the exposed ports:

```text
demi host expose list
demi host expose add "MacBook Pro" 11434 [--label Ollama]
demi host expose remove "MacBook Pro" 11434
```

`list` prints a table, `DEVICE  PORT  LABEL  ADDRESS`, which the agent
reads to put an address in an entry's endpoint with `demi providers`; `add`
prints the exposed port as labelled lines; `--json` is an array or the
port ([Output](commands.md#output)). `list` needs no permission. `add` and
`remove` need **Manage Exposed Ports** (`host.expose`), asked as any
category is ([Permissions](../agent/permissions.md)): "Allow this
conversation to manage exposed ports". Its description says that the
backend's requests for the user, such as a provider entry's, can then reach
that port of the device.

## What is shared

Exposure adds one part and reuses the rest:

- **One HTTP client for provider requests.** Every request the backend
  makes to a provider entry goes through one client, which hands an address
  naming one of the user's devices to that device's runner; today each
  provider builds its requests on the shard's client.
- **Pipes.** The request and the answer travel as the relay's pipes, as a
  forwarded connection's relay path does
  (`crates/runner-jobs/src/tasks.rs`).
- **Device names and states.** The offline and stopped answers use the
  device's state the backend already holds, never asking the runner.

## Not covered

Exposing a port to anything other than the backend's requests for its
owner, publishing a port to the internet, and exposing a port of the
user's browser.
