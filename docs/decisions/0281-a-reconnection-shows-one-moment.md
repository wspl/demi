---
status: accepted
---

# A reconnection shows the conversation at one moment, and the root's entry says it resumes

Supersedes: 0279 (A turn Demi resumes after a restart shows as running until
it runs), where the conversation's page reads that Demi resumes the turn: from
the root's entry now, not from the summary. The summary's `resuming` status
stays for the sidebar.

## Problem

Alex keeps a conversation open while the server restarts, and its turn was
running. When the page reconnects, it reads again the transcript, the agents
and the sidebar's summaries, each on its own, and shows each as it answers:

- The transcript, with the red "interrupted" record, comes first. The summary
  that says Demi resumes the turn comes 0.3 to 1.3 seconds later, so the red
  record shows until then.
- When the turn runs again, the summary can say so a frame before the lane
  says the root runs, and the record shows for that frame.

Each piece is right for its own moment; shown together they make a state that
never existed.

## Decision

**The page takes a reconnection's reads in one step, and the root's entry says
that Demi resumes its turn, beside the root's phase.**

- After a lost connection, the conversation's runtime holds what the client
  brings from the open on, until every read the open's answers asked for has
  ended its first try: the agents, and the latest page of the root and of each
  node the page shows. It then takes them all, in the order they came. Until
  then the page shows what it held, with the spinner it shows while it
  reconnects. The attempt's deadline bounds the wait, as it bounds the open,
  and a connection lost during the wait fails the attempt, which connects
  again. A read that failed goes on trying behind the page, which shows what
  the others brought: a backend that cannot answer one read does not keep the
  conversation from opening.
- A first opening is not held: it has nothing to show from before.
- `AgentEntry` gains `resuming`, true on the root's entry while the product
  resumes its turn after a restart and the root has not worked since. The
  entry that says the root runs says `resuming: false` too, so the two change
  in one frame.
- The shard keeps its `RestartResumes` set, the one owner of the fact. The
  agent server asks it, through `AgentsSource::resuming`, when it makes a
  tree's agents and when it reads them from the store, before the root's entry
  is made. The shard tells a live tree when the set changes
  (`AgentServer::resuming_changed`): a resume that began after a page opened
  the tree, whose root entry then says `true`, and a resume that does not start
  the turn, whose root entry then says `false` with the root still idle, so the
  record shows red. A compaction of an idle root leaves `resuming` as it is.
- The page takes `resuming` from the root's entry, as it takes the phase.
  Nothing reads it from the summary or the metadata any more.

```rust
pub struct AgentEntry {
    // …
    /// The root's turn a restart of the backend interrupted is one Demi
    /// resumes by itself, and the root has not run since.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub resuming: bool,
}

pub trait AgentsSource {
    // …
    fn resuming(&self, root: &NodeId) -> bool {
        false
    }
}
```

An older page ignores the field, and shows the conversation as 0279 does.

## Spike

A stand-in model streamed a turn while the backend restarted, and a probe made
the resume wait 10 seconds, as a runner's return does on a server. A script
counted, frame by frame at 20 frames a second, the frames that showed the red
record.

| Version | Restarts counted | The red record showed |
| --- | --- | --- |
| 0279 as merged | 1 | 0.15 s at the reconnection |
| The metadata says it resumes | 5 | Once, for one frame, when the turn ran |
| This decision | 6 | Never |

Before it was merged, 0279's recordings showed the red record for up to 1.3
seconds at a reconnection.

The hold lasted 7 to 9 ms on the page that showed the conversation. A page
that held the conversation without showing it waited 0.7 to 2 seconds, as its
reads start after a random delay that spreads the reads of many pages.

## Prior art

- [Replicache](https://doc.replicache.dev/concepts/how-it-works) applies the
  patch a pull brings, replays the pending changes on it, and reveals the
  result to the app atomically, so the app never shows part of one state
  beside part of another. Demi takes the reads of a reconnection the same way.
- [JupyterLab](https://jupyterlab.readthedocs.io/en/latest/api/types/services.Kernel.Status.html)
  carries a kernel's `autorestarting` in its status, the same value as its
  busy or idle, rather than beside it. The root's entry carries `resuming`
  beside its phase for the same reason.

## Alternatives

- **Carry `resuming` in the conversation's metadata.** The lane sends metadata
  before the agents' frames, so it can say the resume ended a frame before the
  entry says the root runs.
- **Mark the interruption record itself as one Demi resumes.** It changes
  stored records, and a resume that fails would show Resuming first, then red.
- **Hold only the transcript until the agents are read.** It fixes this case
  and leaves every other pairing of a reconnection's reads to show two moments.

## Open questions

None.
