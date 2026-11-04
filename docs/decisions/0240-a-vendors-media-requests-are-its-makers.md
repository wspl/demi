---
status: accepted, superseded in part by 0259
---

# A vendor's media requests are its maker's

## Problem

What each vendor's media request takes is written in the crates every
provider shares, beside the vendor's own crate that sends the request:

- `provider-common/src/audio.rs` holds which options each audio shape's
  request lacks: Gemini's speech no speed, ElevenLabs' music no seed,
  MiniMax's speech no instructions. `provider-common/src/videos/mod.rs` holds
  the same for videos, Veo no audio and Hailuo no seed.
- `shared-types` holds what each audio shape calls itself in a call's
  message, *Lyria* for Gemini's music, and what each video shape does, and
  which kinds each audio shape makes, which nothing reads.
- `provider-common/src/models_dev.rs` decides that only Anthropic's own
  models take an effort on the Messages API, by comparing the vendor's id
  with `anthropic`.

A maker that differs already overrides the shared answer, as ByteDance's
Seed Audio and MiniMax's v1 videos do, so a vendor's facts live in two
places. Adding a vendor, or changing what one takes, means editing the
shared crates; and the shared answer can disagree with the request the
vendor's crate actually sends, which only that crate's code shows.

## Decision

Nothing the user or the model sees changes: `demi audio` and `demi video`
warn of the same unsent options with the same names.

- `AudioMaker` and `VideoMaker` answer, with no default:
  - `unsent(&self, catalog, request) -> Vec<…Option>`, the options its
    request has no field for, from a list in its own crate, through the
    shared `unsent_of`;
  - `shape_name(…) -> &'static str`, how a call's message names its request,
    which `VideoMaker` gains.
- The shared `audio::unsent`, `videos::unsent`, `AudioShape::name`,
  `VideoShape::name` and the unread `AudioShape::makes` go.
- The shapes stay in `shared-types` as names, as the page shows them and an
  entry stores a compatible entry's choice; what a shape's request takes is
  its maker's.
- A maker a compatible entry chooses keeps its list in the crate that sends
  it, as a vendor's does.
- `ProviderFamily::effort_vendors() -> &[&str]`, none by default, names the
  models.dev vendors whose models take an effort on the family's protocol
  where models.dev lists one; the Anthropic family names its own vendor.
  The user shard's services hand every family's list to the backend's one
  `ModelsDevClient`, whose wires and model listings read it instead of
  comparing ids.

## Spike

None: every reader of the shared answers was read. The makers are in
`provider-openai-api`, `provider-google`, `provider-elevenlabs`,
`provider-bytedance`, `provider-minimax` and `provider-grok-build`; the media
part of the user shard reads `unsent` and the shape's name through the
traits.

## Prior art

None needed: the traits already carry each maker's request, its voices and
its speeds.

## Alternatives

- **Shapes as open strings each crate owns.** A vendor would add its shape
  without touching `shared-types`, but the page's labels, the stored
  choice and the API's schema would all lose their checked list; the
  shapes are names, and a name in a shared list costs one line.
- **Keep the shared table and only move the overrides.** It leaves each
  vendor's facts split across two crates, which is the problem.

## Open questions

None.
