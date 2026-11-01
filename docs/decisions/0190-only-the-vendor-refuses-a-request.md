---
status: proposed
---

# Only the vendor refuses a request for a used-up account

Supersedes: 0172 (Smart rotation spends the quota that resets first), its
step 1, which leaves out an account that is used up.

## Problem

Alex's subscription refuses an image on Monday: its weekly window is used
up until Thursday. Demi keeps what it learned in two records, and both say
"used up until Thursday":

- the account's **quota snapshot**, which the refusal's usage headers
  updated and which is stored with the account;
- a **mark**, held in memory, which the refusal set.

On Tuesday Alex buys more usage on the vendor's site. Every image Demi is
asked for still fails at once with "the usage limit is reached, resetting
at Thursday", and no request leaves the server, because:

- A choice leaves out an account that a mark or a snapshot shows used up,
  and an entry with no account left is refused before it is asked. The same
  holds for an agent's turn on any subscription, Claude Code's and Codex's
  alike.
- Nothing corrects the records. The snapshot changes only when a request is
  answered, which no request now is, or when a probe reads the usage. A
  restart drops the mark but not the snapshot, and a probe, Settings'
  Refresh Usage included, changes the snapshot but not the mark.

So a record of what the vendor said once refuses for the vendor until the
moment the record names, however the account changed.

## Decision

Records order the accounts; only the vendor refuses. Whatever a mark or a
snapshot says, a request is sent, and a refusal that comes back is what
fails it or moves it to another account, as today.

- **Choosing an account.** In Order, Smart and the media entries' order
  rank the accounts as today, but an account the records show used up is
  ranked after every other, those soonest to reset first, instead of being
  left out. A request whose every account looks used up is sent on the one
  that resets soonest. Manual sends on its account.
- **After a refusal.** The account is marked as today, and the request
  moves to the next account it has not yet tried. When every account of the
  entry refused it, the request fails with the vendor's words and reset
  moment. One request asks each account at most once.
- **Correcting the records.** A reading newer than a mark replaces it: a
  snapshot observed after the mark was made, from an answer or a probe,
  decides the account's standing alone. Refresh Usage in Settings then shows
  the account usable at once when the vendor says it is.
- **What is shown.** "Used up until …" in Settings, the model lists and a
  failed turn's facts is read from the same records and says what the vendor
  said last. It no longer means a request will not be sent.

For example: Monday's refusal leaves Alex's only account marked and its
snapshot at 100% until Thursday. On Tuesday, after Alex bought more usage,
an image request is sent on that account, since it is the only one. The
vendor answers, and its usage headers update the snapshot, which is newer
than the mark, so the account shows usable. Had the vendor refused again,
the request would fail with Thursday as its reset, as on Monday.

### Design

`AccountRotation::take` no longer answers that every account is used up; it
ranks them and skips only the accounts the request already tried:

```rust
pub enum Choice {
    /// The account the request is sent on, counted among those running on
    /// it until the count drops.
    Account(Running),
    /// Every account of the entry refused this request already.
    AllTried,
}

impl AccountRotation {
    pub fn take(
        self: &Arc<Self>,
        entry: &ProviderId,
        rotation: Rotation,
        candidates: &[Candidate],
        model: &Model,
        tried: &[CredentialId],
        now: Timestamp,
    ) -> Option<Choice>;

    /// Marks `account` used up until `until`, as the vendor said at `at`.
    pub fn mark_used_up(&self, entry: &ProviderId, account: &CredentialId, until: Timestamp, at: Timestamp);
}
```

- A mark holds the moment it was made. `rooms` reads it only while the
  candidate's snapshot was observed before it.
- An agent's account switch and a media call pass the accounts their request
  tried. A media route no longer skips an entry whose standing is used up;
  it tries the entries with room first.
- `all_used_up` stays as a reading for what is shown. `is_used_up` says
  whether a refusal still marks an account, which moves an agent off the
  account it kept, as a refusal did before. Neither gates a request.

Nothing is stored anew: the snapshot is stored as today, and the marks stay
in memory.

## Prior art

- OpenAI's Codex CLI keeps the rate limits a usage-limit refusal reports
  only to show them, and ends the turn with the vendor's error; the next
  turn asks the vendor again
  ([`session/turn.rs`](https://github.com/openai/codex/blob/67f0994b532b8740f14b5b738af7b7b2a915b1d6/codex-rs/core/src/session/turn.rs#L1899)).
  Demi does the same for its one account per request, and departs only in
  ordering several accounts by what it last read.

## Alternatives

- **Probe before refusing.** When the records say used up, read the usage
  first and refuse only if the probe agrees. The probe and the request
  answer the same question, so this asks the vendor twice where sending asks
  once, and a family whose probe would spend an inference request, which
  Demi never runs as a probe, would still be refused by its records.
- **Shorter marks.** Expire every mark after a few minutes. A snapshot still
  refuses until its own reset, and any length is a guess.
- **Drop the marks.** The snapshot alone would carry what a refusal said. A
  refusal that names no window, which today marks the account for 30
  minutes, would then not order the accounts at all, and an agent would
  return to it on its next turn.

## Open questions

None.
