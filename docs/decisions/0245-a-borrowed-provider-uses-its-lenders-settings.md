---
status: accepted
---

# A borrowed provider uses its lender's settings, which only the lender changes

Supersedes: 0236 (A Demi relays its providers to another), the borrower's
media switches and its copy of the lender's media shapes.

## Problem

Alex lends Studio, an OpenAI key on `demi.example.com`, to a laptop's Demi.
On the laptop, Studio sits among the API keys and its page looks like theirs:
each model has a switch, Images, Videos and Audio each have one, and Test
Connection sends a message. None of them is the laptop's to decide. Alex turns
GPT-5 mini off on the lender, and the laptop still offers it. The lender
starts making videos, and the laptop shows no Videos until the key is pasted
again, since the laptop kept the shapes the lender had when it was pasted.
Two servers each hold half of one provider's settings, and they drift.

## Decision

A borrowed provider is the lender's, used as the lender set it up. On the
borrower it has a group of its own, and its page shows what it offers without
a control to change it.

### What the user sees

```text
Providers
  SUBSCRIPTIONS            Studio ✎                                  [on]
    Codex                  ┌──────────────────────────────────────────────┐
  API KEYS               + │ Share Key   Shared by demi.example.com  [••••] │
    Anthropic              └──────────────────────────────────────────────┘
  SHARED WITH YOU          Models
    Studio                   GPT-5                                       ⓘ
                             GPT-5 mini                                  ⓘ
                           Images
                             GPT Image 1                                 ⓘ
                           Audio
                             GPT-4o mini TTS                      Speech ⓘ
```

- **Its own group**, Shared with You, below API Keys, shown once there is one.
  Add Provider keeps its Another Demi row, and the new provider appears in
  this group.
- **What the borrower changes**: its name, its switch, and its share key,
  which a new key replaces when the old one is revoked, so the conversations
  and agents that chose its models keep them.
- **What it only shows**: the lender's text models, then each kind of media
  the lender makes now with its models, each with its details. No switch on a
  model or a kind of media, no Add, no refresh, no Test Connection, no prompt
  cache, no Catalog or Manual.
- **The lender's switches hold on the borrower.** A model the lender turned
  off is not lent: the borrower no longer lists it, and a run of it fails as
  a model the provider no longer offers. A kind of media the lender turned off
  is not made. A provider the lender turned off lists nothing, and a run
  fails with "Studio is turned off on demi.example.com." under the provider's
  response.
- **When the borrower sees a change.** A run or a media call is checked
  against the lender's settings as it is sent, so a change holds at once. The
  page shows the change when the borrower reads the lender's catalog again,
  as it reads any provider's, within 15 minutes.

### The design

- **The borrower stores the key and nothing it can read from the lender.**
  The entry keeps its name, its switch and its key, as 0236 had it, and no
  longer the lender's media shapes. Each listing asks the lender: the
  provider's catalog, as before, and for each kind of media its maker and
  models (`GET /api/relay/media/makers/{kind}`). A kind the lender does not
  make, or has turned off, lists no models, and the borrower's page shows no
  section for it and no call chooses it.
- **The lender filters what it lends by its own settings**, those of the
  key's maker: `Lent.catalog` leaves out the models they turned off, and a
  run of one is refused as a model the provider does not offer.
- **A lender that turned the provider off** answers `GET /api/relay/provider`
  with `Lent { turned_off: true, catalog: [] }`, which the borrower lists as
  no models, and its socket and media routes with HTTP 423. A session open
  then ends at its next run, and the borrower, asking why, words it. A video
  job started before is still answered, its state, its file and its cancel,
  so what the lender paid for is not lost. The key still works, so turning
  the provider on again needs nothing of the borrower.
- **The lender judges each run at the relay's edge**
  (`crates/provider-relay/src/lend.rs`), before the run reaches the entry's
  runtime, so no provider and no runtime knows that it is lent.
- **The borrower refuses what it cannot change**: a patch of a borrowed
  entry other than its name, its switch and its key is refused as today, and
  now a media switch too.
- `VERSION` stays 1: 0236 is not released, so no server speaks the version
  this changes.

```rust
// crates/provider-relay/src/wire.rs
pub struct Lent {
    pub name: String,
    pub catalog: ProviderModelList,
    pub limits: Vec<ModelLimits>,
    /// The lender turned the provider off: it lends nothing until it is on.
    pub turned_off: bool,
}
```

What goes away from 0236: `Lent.media`, the shapes the borrower's entry
kept, the borrower's media switches, and the note that a change of the
lender's shapes needs the key pasted again.

## Spike

None: each piece is a read the borrower already makes, or a filter of what
the lender already sends.

## Prior art

- macOS and iOS show an item another person shares, a Shared with You link
  or a shared album, apart from the user's own, and let the user remove or
  rename it but not change what its owner set.
- Google Drive's Shared with me lists files whose settings only their owner
  changes.

## Alternatives

- **Keep the borrower's switches and add the lender's**: two sets of switches
  on one provider, whose result neither page shows.
- **Copy the lender's settings to the borrower and refresh them**: a stored
  value that can be read, which drifts between reads.

## Open questions

None.
