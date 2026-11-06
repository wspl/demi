---
status: proposed
---

# An agent sees every device and is allowed one at a time

Supersedes: 0205 (A permission covers a whole capability), the row Devices
and its category `host.use`: a device is allowed one at a time, and pairing
and port forwards ask nothing of their own.

## Problem

The user asks an agent to set up a virtual machine on `lab-server`, a device
they paired weeks ago. The agent runs `demi host list`, which shows only the
Hosts this conversation already reaches: the laptop it runs on. It concludes
`lab-server` is out of reach and asks the user for an SSH address. Had it
tried `demi host shell --host lab-server`, the error would have sent it to
`demi host list` again. The device was online all along; only
`demi host devices`, a second list, shows it.

Two lists answer one question, "which machines can I work on", and the one
named `list` leaves out most of the answer.

The other half is the grant. Since 0205, the first device a conversation
brings in asks for Use Your Devices, which then lets it run commands as the
user on every device and the Cloud. A user who wants one build machine
used has to hand over all of them, or deny the work.

## Decision

The agent sees every device in one list, and the user allows each device a
conversation uses, one at a time.

### One list

`demi host list` lists the user's Cloud and every paired device, each with
what it is to this conversation:

```text
NAME          ID        STATUS   ROLE      DIRECTORY
laptop        6a50f...  online   primary   /Users/alex/app
build-box     61529...  online   attached  /home/alex
lab-server    31c2a...  online   -         -
Cloud         b71d1...  stopped  -         -
```

The status is `online`, `updating`, `offline`, or `stopped` for a Cloud
that wakes for work. The role is `primary`, `attached`, `subagent` for a
Host only a subagent works on, or none. The directory is where commands
start, for a device the conversation reaches. The JSON form is one array of
these rows, a missing role or directory as `null`. Listing asks nothing:
seeing a device is not using it. `demi host devices` is removed; every
summary and error that named it names `list`.

### A device is allowed when it is attached

A conversation reaches its primary Host, its project's device, the Cloud of
a Cloud conversation, and the devices attached to it. Attaching is how it
reaches any other, and the attachment is the user's leave: detaching the
device takes it back.

- `demi host attach lab-server` attaches it.
- `demi host shell --host lab-server` and `demi host move lab-server …` on a
  device the conversation does not reach attach it first, as `attach` does,
  then go on. The agent never meets a dead end that only a second command
  gets past.

An agent's attach asks the user for that device:

```text
+----------------------------------------------------------------+
| Allow this conversation to use "lab-server"?                   |
| The agent ran                                                  |
|   demi host shell --host lab-server "df -h"                    |
| Its agents can run commands as you on "lab-server" until you   |
| detach it.                                                     |
|                         [Deny]  [Allow for This Conversation]  |
+----------------------------------------------------------------+
```

- Allow passes the call, which attaches the device, and every other call of
  the conversation waiting for that device.
- Deny refuses the call, as any refusal does, and the next attempt asks
  again.
- A device the user attaches on the page asks nothing, as now.

There is no answer that allows every device. A command on a device acts as
the user on that machine, so one build machine and the user's own laptop
are not one decision. Each device asks once per conversation.

What else `host.use` covered asks nothing of its own:

- `demi host pair` pairs a computer the agent already runs commands on,
  since it ran the installer there, and attaches it: the conversation reaches
  nothing it did not reach.
- `demi host forward add` and `remove` act on devices the conversation
  reaches: both ends of the forward must be attached, or the command says to
  attach the one that is not, which asks for it.

### Design

- `backend-database`: `permission_requests` gains a `device` kind and a
  `device_id` column. `Needed::Device` records a request of that kind, which
  replaces an undecided one of the same device from the same agent. An allow
  decides every undecided request of that device in the conversation and
  records no grant; the calls it passes attach the device. `DEVICES` goes.
  The migration deletes the grants of `host.use` and its undecided requests;
  a decided one stays until its message is delivered.
- `backend-permissions`: `Need::Device` carries the device's id and name. A
  device's message to the agent says "The user allowed this conversation to
  use "lab-server"", as `AgentMessageEvent::Permission` with its `host`.
- `web-api-protocol`: `PermissionRequest::Device` carries the device's id
  and name, and `PushAsk::Device` its name.
- `backend-host-access`: `list` lists every device and `devices` goes.
  `shell` declares `bringsHost` for `--host` and attaches a device it does
  not reach before it runs. The group declares no category; `pair` and
  `forward` lose theirs, and `forward add` and `remove` refuse an end the
  conversation does not reach.
- `web-ui`: `PermissionRequestCard` shows a device's request with the
  device's title and description. The gallery shows it.
- The context block's line says: `Attached hosts: build-box (…). demi host
  list shows all of the user's devices; demi host shell --host <name>
  <script> runs a shell string on any of them, attaching it first.`

A conversation that held Use Your Devices asks again, once per device it
attaches from then on; what it attached stays. The removed `devices` has no
alias: commands are declared by the backend, so every agent reads the new set
at once.

## Spike

None.

## Prior art

- OpenSSH asks on the first connection to a host whether to trust it, one
  host at a time, and `ssh` reaches any host the configuration names with no
  step that adds it to a session first.
- [`tailscale status`](https://tailscale.com/kb/1080/cli#status) lists every
  device of the tailnet with this one marked; one list, reached by name.
- Browsers ask for a site's camera or location one site at a time, and keep
  the answer for that site.

## Alternatives

- **Keep both lists and sharpen their descriptions.** Two commands still
  answer one question, and the obvious name still shows the partial one.
- **Keep the grant of every device, as a third answer on the card.** It
  brings back the decision this record removes: one answer opens the user's
  laptop along with the build machine.
- **Keep asking only for the whole capability.** The user cannot let an
  agent use one machine without letting it use every machine.

## Open questions

None.
