---
status: proposed
---

# Each agent works on its own Host

Supersedes: 0027 (A move applies at once and stops nothing), the part that
a conversation's subagents move with it.

## Problem

Alex's conversation runs on his laptop. Its subagent has cloned a repository
into `~/src/shop` there and is running the test suite. Alex moves the
conversation to his Cloud to keep working from his phone. The subagent's next
command runs on the Cloud, in the conversation's new directory, where the
clone it made does not exist; it fails, or worse, works on a different copy.

Today a node's working directory is on the conversation's primary Host, and
when the conversation moves, every child's directory becomes the
conversation's new one (`crates/agent-server/src/node/mod.rs`). An agent in
the middle of its work cannot keep its place.

## Decision

Every agent has its own placement: a Host and a working directory there.

* The root agent's placement is the conversation's. Moving the conversation
  moves the root.
* A subagent's placement is its parent's when it is spawned, or the
  directory `--cwd` names on its parent's Host. Only its own `demi agent cwd`
  changes it afterwards, on the same Host.
* Moving the conversation leaves every subagent where it is. A subagent
  spawned after the move starts at its parent's new placement.

So in Alex's case, the subagent finishes the test suite on the laptop,
while the root and the subagents it spawns from then on work on the Cloud.

A subagent whose Host is offline waits for it as the root waits for the
primary Host; one whose Host is gone, such as a device Alex removed, fails
each command as a command on any unreachable Host does, and Alex stops it.

**What the page shows.** The Agents window's status bar names the Host of a
subagent that works on another Host than the conversation, with its glyph,
before its folder. That folder only explains, and the bar shows no branch,
since the File view and the Change view look at the conversation's Host:

```text
┌──────────────────────────────────────────────────────────────────┐
│ ● Running · 4m 2s   ▭ alex-laptop   ▢ shop     research  Sonnet  │
└──────────────────────────────────────────────────────────────────┘
```

The Files tab's directory menu lists only the directories of the agents on
the conversation's Host, and code paths in such a subagent's replies are not
linked.

**What the model reads.**

* A subagent's working-directory context names its Host when it is not the
  conversation's: "Your working directory is `~/src/shop` on alex-laptop."
  A move of the conversation that changes whether it is sends a new one.
* `demi agent list` and `demi agent show` name that Host beside the
  directory.
* `demi host list` lists a Host only a subagent works on with the role
  `subagent`; `demi host detach` refuses it, since it is not attached.
* A subagent's instructions are read from its own directory on its own Host;
  its skills are the conversation's, installed on its Host at its first
  command there, as on any Host.

**The design.**

* `NodeRecord` holds each subagent's `placement`, a `Placement` (device and
  directory), written at its spawn and by `demi agent cwd`.
  `ChildDirectory` and its rule that a moved conversation overrides a
  child's directory are deleted. The conversation database keeps it in
  `placement_device` and `placement_directory`; a stored subagent keeps the
  Host and the directory it was last given.
* `HostResolver::host` resolves a subagent's own placement, and the
  conversation's current target for the root. The product keeps a node's
  shell environment per Host, as it keeps one per target today.
* The host access reaches a Host a live subagent works on, as it reaches the
  primary and the attached ones. A job there waits for its device as a job
  on the primary Host does. A move fails only the root's jobs that wait for
  the device it leaves, and tells that device to let go of the conversation
  only when no live subagent works there.

## Spike

None.

## Prior art

* [Claude Code subagents](https://code.claude.com/docs/en/sub-agents) work
  in the parent's working directory, and a subagent with worktree isolation
  keeps its own working copy for its whole life, whatever the parent does
  afterwards. Demi does the same across Hosts.

## Alternatives

* **Subagents move with the conversation** (today): an agent in the middle
  of its work continues somewhere its earlier steps did not happen.
* **Stopping subagents before a move:** 0027 refused a move that stops
  work, and it still holds for the root.

## Open questions

None.
