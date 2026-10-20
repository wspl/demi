---
status: accepted
---

# Plugin architecture: lifecycle, page state and layout

## Context and Problem Statement

Demi's plugins are built in, and the user turns each one on or off. Turning
one off today is applied unevenly:

* Its instance and the tasks it started keep running.
* Open conversations can still call its commands until they are reloaded.
* What it keeps open on a Host, such as browser tabs or device captures,
  stays open.
* Its panel tabs stay, showing "This tab cannot be shown."
* On the page, turning any plugin on or off rebuilds every plugin's panel
  session.

On the page, a plugin's live state belongs to the panel beside one
conversation. Switching conversations or closing the panel ends its
connections, last pictures and frames, so the user waits for them again on
return.

## Considered Options

* A plugin's live state lives from the moment it is turned on to the moment
  it is turned off, at every layer, and its durable data and caches outlive
  both.
* Keep the panel's sessions and make each plugin reconnect faster.

## Decision Outcome

Chosen option: a plugin lives while it is on, because turning it on and off is
rare, while switching conversations and panel tabs happens all the time.

### Turning a plugin on or off

```text
                 on                               off
 backend      a new instance                   instance and its tasks end
 commands     registered                       unregistered: a call fails as
                                               any unknown command does
 Hosts        opened again on demand           what it holds open is released
 panel tabs   opened again on demand           its tabs are closed
 page         its stores are made              its stores are disposed
 caches       ─────── kept; not tied to on or off ───────
 data         ─────── kept (skills, settings, ...) ───────
```

* Turning one plugin on or off touches only that plugin.
* A conversation's system prompt lists the commands it opened with, until it
  next opens.

### A plugin's state on the page

```text
 on ──────────────────────────────────────────────────────▶ off
 ┌ the plugin's Pinia stores, held by the shell ───────────┐
 │  sessions by key: device m1, device m2, browser tab t3   │
 │    watched      live                                    │
 │    unwatched    capture and decoding stop; connection,  │
 │                 last picture and frame kept             │
 │    over 16 kept the least recently used is released     │
 └─────────────────────────────────────────────────────────┘ disposed
 a disk cache per plugin, bounded by size, least recently used goes first
 components subscribe and render; unmounting only unsubscribes
```

* An unwatched session is not activity: it keeps no conversation, Host or
  Cloud awake, so a conversation the user left goes idle as it does today.
* A plugin defines its stores through the SDK and reaches only its own.
* The page's panel binds only the plugins' tab kinds; it holds no session.
* The keyed sessions with their limit are a `web-ui` utility that other
  stores can use.

### Where a plugin's code lives

Each plugin keeps all of its parts in its own directory under `crates/`, in
place of crates and page packages spread by kind across `crates/` and
`packages/`:

```text
 crates/plugin-mobile/
   plugin/      the backend plugin
   command/     its command package on Hosts
   protocol/    the command package's protocol
   page/        its page
```

* A plugin has only the parts it needs; the browser's also holds its Chrome
  driver.
* Crate and package names do not change.
* What no single plugin owns stays where it is: the plugin framework
  (`plugin-interface`, `backend-plugins`, `runner-command-packages`,
  `plugin-sdk`), the provider's `command-package-claude-code`, and the media
  library the image, video and audio plugins share.

### Consequences

* Good, because returning to a device or a browser tab shows its last
  picture and frame at once, after any conversation switch or panel close.
* Good, because turning a plugin off stops everything it was doing.
* Bad, because a page keeps up to 16 idle connections per plugin.
* Modules touched: the backend's plugin host and the plugin contract (an
  instance per switch), command registration of open conversations, the
  packages' release of a plugin's Host resources, the panel, `plugin-sdk`
  and `web-ui` (stores, keyed sessions, disk cache), the browser and mobile
  pages, and the gallery's fixtures, which model turning a plugin on and off.
