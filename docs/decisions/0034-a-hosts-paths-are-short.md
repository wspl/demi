---
status: accepted, superseded in part by 0043
---

# A Host's Demi paths are short

## Context and Problem Statement

A runner installs into `~/.demi/instances/<id>`, where the id is the SHA-256
of its backend's URL, 64 hex digits. A conversation's scratch directory is
`scratch/<conversation id>`, the whole 36-character UUID, and a subagent's is
`agent-<number>` inside it. So an agent's scratch path is about 150
characters:

```text
/Users/alex/.demi/instances/3f9a1c7e5b2d8046a1e9c3b7d5f20864ce1a3b5d7f9e2c4a6b8d0f1e3a5c7b9d/scratch/5b2c9e41-7a3d-4f08-9c6e-1d8b3a7f2e90/agent-12
```

Every command, report and link that names a file there carries it, so a
model pays for it each time it reads one, and a person cannot tell one
installation from another by its name. The installer's script and the
runner each compute the hash on their own.

## Considered Options

* Keep the names.
* Shorten the hash and the conversation id.
* Name an installation after its backend's host, and a conversation's
  scratch directory after the start of its id.

## Decision Outcome

Chosen option: the backend's host and a short id, because a person reads
which backend an installation belongs to from its name, and twelve
characters tell a user's conversations apart.

* An installation's directory is `~/.demi/runner/<backend>-<hash>`: the
  backend's host, then its port when it is not the scheme's default, then
  its path's segments, each joined by `-`, and last the first four hex
  digits of the SHA-256 of the backend's whole URL, such as
  `~/.demi/runner/demi.example.com-3f9a` or
  `~/.demi/runner/127.0.0.1-3310-b27e`. The part before the hash is cut
  where the whole name would not fit a file name on every system. One
  function names it, for the installer's script and the runner alike.
* The hash keeps apart backends whose readable parts are the same, such as
  `http://example.com` and `https://example.com`, or `/team-a` and
  `/team/a`. An installation's directory still says which backend it
  belongs to, and nothing acts on an installation that belongs to another
  backend: an uninstall for another backend removes nothing.
* A conversation's scratch directory is `scratch/<first 12 hex digits of its
  id>`; a subagent's stays `agent-<number>` inside it.

```text
/Users/alex/.demi/runner/demi.example.com-3f9a/scratch/5b2c9e417a3d/agent-12
```

* An installation made before keeps its directory: its launcher names it.
  Installing again for the same backend makes the new directory, beside the
  old one, whose uninstall command removes it.
* On the first start of a runner with this change, the scratch directories
  under the old names are removed with the conversations they no longer
  match, as any directory of a conversation that is gone.

### Performance

* A scratch path goes from about 150 characters to about 70, some 25 tokens
  fewer each time a command, a result or a reply names one.
* No extra work at run time.

### Consequences

* Good, because paths are short enough to read and cheap to repeat, and an
  installation's name says which backend it serves.
* Bad, because two backends whose names agree up to the hash would share a
  directory, one chance in 65,536 when their readable parts are the same;
  the second is then refused, not installed over the first.
* Bad, because two conversations of one Host whose ids start with the same
  twelve digits share a scratch directory; among ten thousand conversations
  that is about one chance in five million.
* Bad, because files in a scratch directory are lost once, at the upgrade.
* Bad, because a machine installed before and installed again has two
  installations until the old one is removed.
