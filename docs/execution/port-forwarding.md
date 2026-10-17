# Port forwarding

A user runs Ollama on their Mac, at `localhost:11434`, and works in a Cloud
conversation. The user forwards port 11434 of their Cloud to
`localhost:11434` on the Mac. A program on the Cloud, a script the agent
runs or an app the user builds there, connects to `127.0.0.1:11434` on the
Cloud and talks to the Mac's Ollama, as through `ssh -L`.

```text
program ─▶ 127.0.0.1:11434 on Host A ─▶ A's runner ══ direct ══▶ B's runner ─▶ localhost:11434 on B
                                     └── relay through the backend ──┘
```

Forwarding connects the user's Hosts to each other; nothing listens on the
backend's server, where on a shared instance every user's programs could
connect.

## A port forward

A forward has a name, a **Listen On** Host and port, a **Connect To** Host
and address, `localhost:11434` by default, any `host:port` the target Host
can reach, and a switch. It is the user's, kept in the control store until
the user removes it, and runs whenever its Listen On Host's runner runs:
after a restart of either runner, or of the computer, it runs again with no
one asking.

- **Listening.** The Listen On Host's runner listens on `127.0.0.1` only,
  so only programs on that Host connect; nothing on its network or the
  internet can. A port another program holds leaves the forward with *Port
  11434 is in use on Cloud* and **Try Again**; the runner tries again at its
  own start and when the user asks.
- **Connecting.** Each connection the listener accepts opens a connection
  from the Connect To Host's runner to the address. When that Host is
  offline, or stopped, the connection is closed at once, as a refused
  connection is, and the forward says *MacBook Pro is offline*; a stopped
  Cloud is not woken, as device access never wakes one
  ([Every way to a Host](sessions-and-targets.md#every-way-to-a-host)).
- **Both ends are the user's own Hosts**, a paired device or their Cloud;
  a forward between one Host and itself is allowed, from one port to
  another.
- **Turned off**, the listener closes and open connections end; turned on
  again, it listens again. Removing a forward does the same and forgets it.
  Changing where it listens or connects is the same as turning it off and
  on: its open connections end and new ones follow the change; a new name
  changes nothing else. Removing a Host removes the forwards that name it.
- **Deadlines.** A connection whose target does not accept within 10
  seconds, or whose relay pipes are not both open within 10 seconds, is
  closed, as a refused connection is.
- **One Host to itself.** A forward whose two ends are the same Host joins
  its connections on that Host, no runner or backend between them, a Cloud
  included, and counts them as *Direct*.

### The path of a connection

Two runners talk directly when they can, as a page and a runner do
([Direct channel](direct-channel.md)), and through the backend otherwise:

- **Direct.** The backend introduces the two runners at the first
  connection of any forward between them, the listening side offering. Each
  forwarded TCP connection is one data channel of their peer, so a lost
  packet holds up only its own connection. The peer closes a minute after
  the last connection between the pair ends.
- **Relay.** A connection that does not go direct is a pair of pipes, one
  each way, from one runner to the other through the backend.
- **Which.** A connection takes the path chosen when it opens and keeps it:
  direct while the pair's peer is connected and neither Host's route is
  Relay Only, the relay otherwise, and always the relay when either end is a
  Cloud, which runs beside the backend. While a pair is on the relay, the
  runners try a direct peer again as the page does: when either runner
  connects again, and after 1, 2, 5 and then every 10 minutes. A failed peer
  sends new connections to the relay at once; the connections it carried
  end, as TCP connections do when a network drops, and their programs
  connect again.
- **Limits.** A peer carries 64 connections, the direct channel's channel
  limit; more go over the relay.
- **Flow control.** A connection's bytes never pile up in a runner: each
  side of a direct connection grants the other a window of 256 KiB and
  grants more as the program it writes to takes the bytes, so a slow
  program slows the sender, as TCP itself does; on the relay, the pipes do
  the same.

### What the user sees

**Settings › Port Forwarding**, below Devices, lists the user's forwards,
one row each: its name, *Cloud · 11434 → MacBook Pro · localhost:11434*,
its switch, and its state in secondary text:

| State | Says |
| --- | --- |
| Listening, connections open | *Direct · 2 connections*, *Relay · 1 connection*, or both, *Direct · 2 connections, Relay · 1 connection* |
| Listening, none open | *Idle* |
| Off | *Off* |
| Connect To not chosen yet | *Choose a Host to connect to.* |
| The port is taken, by another program or another forward | *Port 11434 is in use on Cloud* |
| Either Host offline | *MacBook Pro is offline* |

The Listen On runner announces whether it listens, a taken port, and its
connections on each path as they change; whether a Host is online the page
already knows from the user's devices. A forward's page says, under its
state, why its connections go through the relay when a direct path failed,
in the direct channel's own words, *Your networks block P2P connections*.

Names are unique among the user's forwards, so `demi host forward remove
ollama` names one: **Add Forward** creates one at once, named *Forward*,
*Forward (2)* for the second, listening on the first of the user's Hosts on
port 8080, or the next port none of the user's forwards on that Host uses,
and opens its page with Connect To shown as missing; the page edits every
field, a name already taken is refused, *A forward named ollama already
exists*, and **Remove Forward** asks first.

### From a conversation

`demi host forward` manages the user's forwards:

```text
demi host forward list
demi host forward add --listen Cloud:11434 --connect "MacBook Pro:localhost:11434" [--name ollama]
demi host forward remove ollama
```

`list` prints a table, `ID  NAME  LISTEN  CONNECT  STATE`, the listen
side as `Cloud:11434`, the connect side as `MacBook Pro:localhost:11434`, a
Host not chosen yet as `-` ([Output](commands.md#output)); `add` prints the
new forward as labelled lines; `--json` is an array or the forward.

`list` needs no permission. `add` and `remove` need **Manage Port
Forwards** (`host.forward`), asked as any category is
([Permissions](../agent/permissions.md)): "Allow this conversation to manage
port forwards". Its description says that a forward outlives the
conversation and that any program on the listening Host can then reach the
address. The agent uses it when a task needs a service on another Host,
such as a test that calls the Mac's Ollama from the Cloud.

## What is shared

Forwarding is built from the parts the direct channel and the relay already
have, moved where both uses reach them, not written a second time:

| Part | Today | Becomes |
| --- | --- | --- |
| The WebRTC peer | The runner only answers a page's offer | One peer in `runner-direct` that offers or answers, used for pages and runners; the test-only offerer is rebuilt on it |
| Gathering addresses, STUN, trickling candidates, the connect timeout | `runner-direct`, used by the answerer | The same, used by both roles |
| A channel's flow control (64 KiB messages, 256 KiB queue) | Inside the file operations' pump | One channel reader and writer that the file operations and a forwarded connection both use |
| Introducing two ends | The backend relays a page's signaling socket to a runner | The same signaling, relaying between two runners as well |
| Device-to-device bytes | Pipes take a device at either end; no caller uses two | A forwarded connection's relay path uses pipes as they are |
| Why a direct path failed | The page's reasons and sentences | The same set, which the runners report for a pair |

The direct channel's retry schedule is the page's in TypeScript and the
runners' in Rust: the two cannot share code, so its numbers live in the
direct channel's design and both follow them.

## Not covered

Listening on a network address other than `127.0.0.1`, publishing a port to
the internet, forwarding UDP, and forwarding to or from the user's browser.
