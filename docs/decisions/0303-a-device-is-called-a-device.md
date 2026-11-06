---
status: proposed
---

# A device is called a device

## Problem

Alex opens Settings and reads "Devices", "Add Device…", "Remove Device". He
opens a conversation and its menu says "Move to Another Host…". Port
Forwarding asks him to "Choose a Host" from the same list of machines that
Settings calls devices. An error tells him "The device is already the
conversation's main host." The agent reads `demi host list`, passes
`--host build-box`, and is told in its context block about "Attached hosts".

Every one of these is the same thing: one of Alex's machines, or the Cloud.
Two words for one thing make him wonder whether a host is something a
device has, or a device he has not added yet. The code carries the same
split: `ReachableHost`, `conversation_hosts` and `HostRole` name a device as
a conversation reaches it, beside `DeviceId`, `devices` and `DeviceState`
for the same machine.

## Decision

A machine Demi runs commands on, a paired computer or the Cloud, is a
*device* everywhere: on screen, in what the agent reads and runs, in the
code, the stored data and the protocols. What it is to a conversation is the
device's role, never another noun.

| Today | Afterwards |
| --- | --- |
| Host, Hosts (UI) | Device, Devices |
| main host, primary host | the conversation's device; `primary` as a role |
| attached host | attached device |
| Move to Another Host… | Move to Another Device… |
| Listen On Host, Connect To Host | Listen On Device, Connect To Device |
| `demi host list`, `shell`, `move`, `attach`, `detach`, `pair`, `forward` | `demi device list`, … the same leaves |
| `--host <name>` | `--device <name>` |
| `Primary host:`, `Attached hosts:` in the context block | `Primary device:`, `Attached devices:` |
| `ReachableHost`, `HostRole`, `AttachedHost`, `BroughtHost`, `bringsHost` | `ReachableDevice`, `DeviceRole`, `AttachedDevice`, `BroughtDevice`, `bringsDevice` |
| The `Host` contract (`Host`, `HostFs`, `HostProcess`, `HostKey`) | `Device`, `DeviceFs`, `DeviceProcess`, `DeviceKey` |
| `conversation_hosts`, `hosts_revision`, `host_workflows` | `conversation_devices`, `devices_revision`, `device_workflows` |
| `backend-host-access`, `host-interface`, `host-paths`, `runner-host` | `backend-device-access`, `device-interface`, `device-paths`, `runner-device` |

The other meanings of *host* keep their word, since none of them is a
machine of the user's:

- a network host, as in `host:port`, `localhost` or a URL's host;
- the plugin host, the program that runs plugins (`plugin-host`);
- a page's host, the application that gives a `web-ui` component its data
  (`PageHost`, `PasteHost`).

Every translation says the device word its glossary already has: 设备,
裝置, デバイス, 기기, Gerät, dispositivo, appareil. The glossary's Host row
goes.

### Design

- One work package renames every place at once, as the project's rule on
  names requires: identifiers, crates, files, comments, documents, the
  gallery and every locale.
- Stored data moves by migration: the tables and columns are renamed, and
  JSON documents that carry the word, such as a conversation's selection,
  are rewritten.
- The runner protocol and the web API rename their fields under a new
  protocol version. A runner of the earlier version is refused with the
  update it needs, as any version mismatch is; no field is read under both
  names.
- In the mobile plugin, whose own devices are simulators and emulators,
  text about the machine they run on names the runner there, or says
  simulator or emulator, wherever *device* alone would mean either.
- A decision record keeps its words: they describe what was true when it
  was written.

## Spike

None.

## Prior art

- Tailscale calls each machine of a tailnet a device in its console and its
  CLI (`tailscale status` lists devices).
- Apple calls every machine of an Apple Account a device, in Settings and
  in Find My, whatever role it plays.

## Alternatives

- **Keep both words and define them.** "A host is a device as a
  conversation reaches it" is a distinction the user never needs: the
  role column says it, and the screens already mix the words.
- **Call everything a host.** Settings, pairing and the Add Device flow
  speak of devices, as every operating system does; a user adds a device,
  not a host.
- **Rename only what the user and the agent read.** The code would keep
  one concept under two names, which the project's rules forbid, and every
  change would translate between them.

## Open questions

None.
