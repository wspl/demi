---
status: proposed
---

# A Cloud conversation's directory is there when it is first read

Supersedes: 0069 (A page never waits for a Host), for a conversation on the
Cloud only: "Reading a directory that does not exist yet … creates
nothing".

## Problem

Alex starts a conversation on the Cloud and asks for a chart, which the
agent answers with a widget and no command. Alex opens the work panel's
Changes tab, which says "Can't list changes: No such file or directory (os
error 2)".

The conversation's directory on the Cloud, `sessions/<id>`, is made at its
first use by the agent's work, a transfer or a `demi host` command. A read
Alex makes creates nothing (0069), so until the agent runs a command, every
read of the directory finds it missing, and the panel says so as a failure.
A new conversation looks broken, though nothing is wrong.

## Decision

A read of a Cloud conversation's directory creates it, as the agent's
first use does: the Changes tab of a new Cloud conversation says what it
says of any directory outside a repository, and the Files tab lists an empty
folder.

The Cloud is Demi's machine and the directory Demi's to make, so making it
for a read takes nothing of the user's. On a paired device a read still
creates nothing: looking at a conversation never makes a folder on the
user's computer, and a directory that is not there is shown missing, as
0069 says.

A read still holds no conversation gate while it calls a Host (0069): the
admission of a work panel read resolves the Cloud and the directory under
the file gate, lets go of it, then asks the Cloud to make the directory,
once per connection, as `first_use` remembers it, and reads. It names
nothing: a directory the Cloud names otherwise, through a link, is left for
the work's first use to name, as now. A directory the Cloud cannot make is
the read's to find missing, as before. Nothing is stored, and no wire
changes.

## Spike

None: the Cloud's first use already creates the directory for the agent's
work, which a read now takes too.

## Prior art

- GitHub Codespaces clones the repository into `/workspaces` when the
  codespace is made, before anything runs in it
  ([deep dive](https://docs.github.com/en/codespaces/about-codespaces/deep-dive)).
  Demi departs in making a Cloud conversation's folder at its first read or
  work rather than when it is made, since making a conversation never waits
  for the Cloud (0069).

## Alternatives

- **Show a missing directory of a new conversation as empty.** The panel
  would have to tell a directory not made yet from one removed, which it
  cannot from the Host's answer, and every read would answer it again.
- **Make the directory when the conversation is made.** That waits for the
  Cloud, which a new conversation must not (0069).

## Open questions

None.
