---
status: proposed
---

# A detach applies at once and stops what runs there

Supersedes: 0058 (What reviews added to device tabs and moves is removed),
the part that a detach keeps its hold on the conversation.

## Problem

An agent runs a check on an attached Host, the user sees it is the wrong
machine and the agent calls `demi host detach` on it. The command answers
"detached when this conversation's work ends; until then commands can
still run there", so subagents go on running there for as long as anything
in the conversation works. On the page, Detach on a conversation that is
working is refused. Both wait for the conversation's whole tree to be
idle, which the command that asks for the detach keeps busy itself.

## Decision

**A detach is made when it is asked for, by the page or by
`demi host detach`, whatever the conversation is doing, and waits for no
Host:**

* The Host leaves `demi host list` and the attached Hosts of the
  conversation, and each agent's next request says so.
* A `demi host shell --host` to it fails as for a Host the conversation
  never had, and a command or a `demi host shell` that had not started
  there yet never starts.
* Every command of the conversation that runs on it is stopped, as an
  abort stops a command: it ends as Stopped when that Host reports the
  stop, its output ending with `alex-linux was detached from this
  conversation`; on a Host that cannot be reached then, it ends at once as
  lost, with the same reason.
* Work started there with `&` or `nohup` (0061) runs on, as it does after
  an ssh session ends, but a `demi` call from it is refused: the Host is
  no longer the conversation's.
* That Host's file transfers and live views end; the other Hosts' go on.
* The Host is told to release the conversation once its commands ended,
  so it closes what it opened for it, such as Chrome.
* `demi host detach` answers as soon as the Host has left: "alex-linux is
  detached." A command on that Host that asks for its detach ends with
  the others.

The design to build from:

* `demi host detach` and the page's Detach make the same change in one
  step, as a move does, with no idle tree reserved, no file gate held and
  nothing stored to make later: end that Host's transfers and streams,
  commit, withdraw its jobs, answer.
* The jobs to stop are the conversation's on that device, which host
  access registers per device, a command adopted after a restart
  included: an agent's command and a `demi host shell` that runs there.
  A job that registers after the commit, or is withdrawn before it starts,
  ends at whichever step it is, without starting.
* The release follows once those jobs ended, and only when the commit
  removed the Host and it is not the conversation's primary Host by then.
* A call from a device that is neither the conversation's primary Host nor
  one attached to it is refused.
* The stored pending detach (`conversation_hosts.detaching`) and what made
  it (the shard's watch for an idle tree, the backend start's step that
  made a detach a restart cut off, and the tree's quiescence watch only
  they used) are removed. A migration makes each detach still stored as
  the commit makes it, the Host leaving the conversation and its revisions
  advancing, and drops the column. Each Host lets go of the conversation
  at its handshake after the backend starts.
* An archive is unchanged.

## Spike

None.

## Prior art

* OpenSSH's escape `~.` disconnects at once, whatever runs in the session
  ([ssh(1), ESCAPE CHARACTERS](https://man.openbsd.org/ssh#ESCAPE_CHARACTERS)).
* A move of a conversation already applies at once and waits for nothing
  (0027).

## Alternatives

* **Let the commands that run there finish, and admit no new ones.** The
  user detaches a Host to stop work on it; a command that runs on for
  hours keeps the very machine in use that the user took away.

## Open questions

None.
