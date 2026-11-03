---
status: proposed
---

# An integration runs commands and moves files on its Clouds

## Problem

A team's bot keeps each person's Cloud ready outside any conversation: every
morning it updates the command-line tools its agents use, it copies a
person's saved data in when they join, and it runs a small filter script on
the person's Cloud each time an outside event arrives. None of this belongs
to a conversation, so no agent runs it, and the integration API has no way
to run a command or move a file on a Cloud. Changing the init script for
the morning update would give every Cloud a new system every day (0156).

## Decision

The program reaches each of its Clouds as Demi itself does for the
Cloud's init script: through the Cloud's own access, in a context that
belongs to no conversation (`device_context`), which keeps the Cloud awake
while it runs.

### Commands

* `POST /v1/clouds/{cloud}/commands {script, cwd?, stdin?}` runs `script`
  as the agent's shell would, as `demi` in `cwd` (`~` by default), where
  `sudo` works as it does for the init script, and answers the command's
  id at once. A Cloud that sleeps wakes for it.
* `GET /v1/clouds/{cloud}/commands/{id}` answers its state, `running` or
  its exit code, and its output from an `after` offset, so a program that
  reconnects reads on from where it was (0221). The socket sends a `command`
  event when one ends.
* `POST /v1/clouds/{cloud}/commands/{id}/stop` stops it as the web app's
  stop stops an agent's command.
* A command has no time limit: the program stops what it no longer needs.
* It starts once the Cloud's init script ends, as an agent's command does.
* A reset or a deletion of the Cloud stops its commands first, as each
  needs the Cloud with no work on it.
* Demi keeps a command's record and output in memory, for an hour after it
  ends: a program that reconnects within that hour reads its end. Each keeps at
  most what the Cloud's runner keeps of a command's output, its first and
  last 8 MiB. A server
  that restarts forgets every command, and the Cloud's runner stops each one
  still running, as it stops every job its backend has no record of.

### Files

`/v1/clouds/{cloud}/fs`, `/fs/raw` and `/fs/search` are the Files panel's
routes (0220) on the Cloud with no conversation: list a directory, read
and write a file's bytes, make a directory, delete, search. They start at the
Cloud's home and do not wait for its init script, as the Files panel does
not. The shared folder is a folder of every Cloud of the integration, so
the program reaches it through any of them; an archive is a file it writes
and a command that unpacks it.

### Not for an agent's command

A command request already names its conversation, its Cloud and its
directory, so a program that serves one reads the agent's files through the
conversation's file routes (0220); no `read_file` joins the plugin's port.

## Spike

* `integration_inits.rs` runs the init script through `cloud_shard().access`
  and `command_context::device_context`, the path these routes take.
* A schedule's watch runs a command on a conversation's Host the same way
  (`schedules/watch.rs`).
* A download, an upload and a search hold their conversation's admission
  while bytes move; a Cloud's own access is a hold of the same kind, so the
  transfers take either.

## Prior art

* E2B's sandboxes for agents offer `commands.run` and `files.read` and
  `files.write` on a sandbox beside the agent's own use of it
  ([E2B docs](https://e2b.dev/docs)).
* Docker's `exec` and Kubernetes' `pods/exec` run a command in a running
  container and stream it; here the output is read by offset instead,
  since the state is what a program reconciles from (0221).

## Alternatives

* **A `read_file` port message for a command's call**, as the asker
  proposed: a second way to read the files the conversation's routes read.
* **Command records in the database**, so they outlive a restart: the
  record of a command could then be taken up again as an agent's command
  is, at the cost of a table and a migration for what a program reruns.
* **A process API with its own user, `root` or `demi`, and environment**:
  `sudo` in the script does what `root` would, and a variable is a line of
  the script.

## Open questions

None.
