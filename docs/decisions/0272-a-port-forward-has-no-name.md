---
status: proposed
---

# A port forward has no name

## Problem

Alex forwards the Cloud's port 11434 to Ollama on a MacBook. Settings › Port
Forwarding adds it as *Forward*, the next as *Forward (2)*, and the table
gives its first column to that name. The name says nothing the route doesn't:
a forward is where it listens and where it connects, `Cloud:11434 →
MacBook Pro:localhost:11434`. Yet the name must be unique, a second forward
named like the first is refused, `demi host forward add` takes `--name`, and
`demi host forward remove` takes the name, so an agent names a forward only
to remove it later.

## Decision

A forward is its route. Nothing names it.

```text
☐  Listen On     Port    Connect To      Address          ◯
☐  ● ☁ Cloud ⌄   11434   MacBook Pro ⌄   localhost:11434  ●
```

- **The table** drops the Name column; the state's dot moves to Listen On.
  Each Host column is as wide as its longest name, and Address takes the
  rest of the row.
- **A phone** shows each forward as its listen end, *Cloud · 11434*, over
  where it connects, *MacBook Pro · localhost:11434*, and its page takes the
  listen end as its title. A page too narrow for the table's columns to
  hold a Host's name, a port and an address side by side shows the same
  list, whose rows open the forward's page, rather than cutting the names
  to a letter.
- **Wherever a forward is said by itself**, as a control's accessible name
  (*Port of Cloud · 11434 → MacBook Pro · localhost:11434*), the Delete
  dialog's list and the Remove dialog's title, it is said by its whole
  route, as the page already wrote it.
- **`demi host forward`**:
  - `add` takes no `--name`.
  - `list` prints id, listen, connect and state.
  - `remove <forward>` takes the forward's id from `list`, or where it
    listens, `<host>:<port>`. Two forwards may listen on one port, one of
    them then failing as the port is in use, so a listen end that names more
    than one is refused with their ids.

The stored data and the protocols lose the field:

- The control database's `port_forwards` drops `name` and its
  `UNIQUE (user_id, name)` in a migration.
- `PortForwardDto` and `PortForwardPatch` lose `name`;
  `DEFAULT_NAME` and the error `PortForwardNameTaken` go.
- The runner never received the name, so its protocol is unchanged.

A page of the earlier release reloads after the upgrade, as after any
release, so no page sends a name to a backend that refuses one.

## Spike

None: removing a field leaves nothing uncertain.

## Prior art

- **[OpenSSH](https://man.openbsd.org/ssh#L)** `ssh -L
  11434:localhost:11434` and
  **[kubectl port-forward](https://kubernetes.io/docs/reference/kubectl/generated/kubectl_port-forward/)**
  name no forward: the ports and the target are the whole of it.
- **[Docker](https://docs.docker.com/engine/network/#published-ports)**
  `-p 8080:80` publishes a port by its numbers alone.
- **[VS Code's Ports view](https://code.visualstudio.com/docs/debugtest/port-forwarding)**
  lists a forward by its port, with a label only as an optional note the
  user may add; this record adds no such note, since no one asked for it
  and the route reads on its own.

## Alternatives

- **Keep the name but make it optional**, blank by default. The column stays,
  empty on most rows, and every place that says a forward needs both a name
  and a fallback.
- **Derive a name, such as `ollama` from the port.** A guess the user would
  have to correct, which is the name again.

## Open questions

None.
