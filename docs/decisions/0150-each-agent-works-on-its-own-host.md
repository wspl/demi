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

Every agent has its own place: a Host and a working directory there.

* The root agent's place is the conversation's. Moving the conversation
  moves the root.
* A subagent's place is its parent's when it is spawned, or the directory
  `--cwd` names on its parent's Host. Only its own `demi agent cwd` changes
  it afterwards, on the same Host.
* Moving the conversation leaves every subagent where it is. A subagent
  spawned after the move starts in its parent's new place.

So in Alex's case, the subagent finishes the test suite on the laptop,
while the root and the subagents it spawns from then on work on the Cloud.

A subagent whose Host is gone, such as a device Alex removed, fails each
command as a command on any unreachable Host does; Alex stops it.

**What the page shows.** A subagent that works on another Host than the
conversation names it, in secondary text after its title, wherever the page
lists its agents:

```text
┌──────────────────────────────────────────────┐
│ ● Run the test suite      alex-laptop        │
│ ● Compare pricing pages                      │
└──────────────────────────────────────────────┘
```

**What the model reads.** A subagent's working-directory context names its
Host when it is not the conversation's: "Your working directory is
`~/src/shop` on alex-laptop." Its instructions are read from its own
directory on its own Host; its skills are the conversation's, installed on
its Host at its first command there, as on any Host.

**The design.**

* `NodeRecord` holds each subagent's place, a `Placement` (device and
  directory), written at its spawn and by `demi agent cwd`.
  `ChildDirectory` and its rule that a moved conversation overrides a
  child's directory are deleted.
* `HostResolver::host` resolves the node's own place; the root's is the
  conversation's current target. The product keeps a node's shell
  environment per place, as it keeps one per target today.
* A stored subagent from before this change takes the conversation's
  current place, which is what it would use today.
* The Changes and Files panels show the conversation's Host, as today.

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
