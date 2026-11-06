---
status: proposed
---

# A file path from `~` starts at the Host's home

## Problem

A program uploads a file to a conversation's Host with
`PUT /api/conversations/{conversation}/fs/raw?path=~/inbox/report.pdf`. The
file lands in `/home/alex/~/inbox/report.pdf`, a directory named `~` in the
home, and the agent that looks in `~/inbox` finds nothing. A conversation's
directory and a command's `cwd` read `~` as the home, each with a copy of
its own, so the program reads the file routes the same way. The web app
reads `~` itself before it asks, a fourth copy.

## Decision

Every file route reads a path that starts with `~` as a shell does: `~` is
the Host's home and `~/x` is `x` in it, with `\` beside `/` on a Windows
Host; `~x` and every other path stay as they are. This holds for every
file route of a conversation: listing, making a directory, deleting, a
file's text, a download, an upload, a search's root and an HTML file's
view.

* `HostPaths::home_relative` and `HostPaths::from_home` in
  `crates/host-interface/src/paths.rs` read the path, the one place that
  does. A conversation's directory, a command's `cwd` and a picture's path
  read it there too, so `~\x` is a path from the home on a Windows Host
  only, where `\` separates.
* `ConversationHost::path` reads it against the home the Host's runner
  reported; a Host that reported none keeps the path as it is.
* A delete, a search's root and an HTML file's view took only an absolute
  path, so that their meaning never depends on where a runner stands. They
  take a `RootedPath` now, an absolute path or one from `~/`, which keeps
  that: `/` separates on every Host, while the request cannot know the
  Host's system. On a Host whose runner never reported its home, one from
  `~/` is refused with `409 device_home_unknown`, as a conversation's directory
  from `~` is. `AbsolutePath` stays for a project's and a repository's
  directory.
* An answer that echoes a path, such as a made directory's, gives the path
  the Host used.

The web app sends absolute paths, so what it shows does not change.

## Spike

None: every file route already reaches a `ConversationHost`, which holds
the home the runner reported and the Host's path form.

## Prior art

* OpenSSH's `sftp` and `scp` read a remote `~/x` from the remote user's
  home; `rsync` does the same through the remote shell.
* A shell, and a command's `cwd` here, expand a leading `~` only.

## Alternatives

* **Leave `~` literal and say so in the guide**: a program writes paths the
  way it writes a conversation's directory and a command's `cwd`, and finds
  a directory named `~`.
* **Expand `~` in each caller, as the web app does**: every program that
  calls the routes keeps a copy of the rule, and the backend's copies stay.

## Open questions

None.
