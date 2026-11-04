---
status: accepted
---

# A video takes what its catalog entry states, and a call outside it fails unpaid

Supersedes:

- 0240 (A vendor's media requests are its maker's), the video makers'
  lists of unsent options and the shapes a compatible entry chooses for
  images and videos.
- 0243 (Gemini Omni makes, edits and extends videos), its row that leaves
  `--duration` unsent because Omni has no field for it.

## Problem

Alex asks the agent for a 4-second transition shot with Gemini Omni. The
agent runs `demi video generate --model gemini-omni-1.1-flash --duration 4
…`. Demi sends no length: the Omni maker lists `--duration` among the
options its request has no field for, although the catalog states that
Omni takes 3 to 10 seconds and Google's own Omni client sends
`response_format.duration`. Omni makes its default, 10 seconds, and Alex
pays for 10. The command says so only after the video is paid for:

```text
demi video generate: warning: Gemini Interactions takes no --duration
```

Two lists say what a video model takes, and they disagree:

- The catalog states each model's durations, resolutions, aspect ratios,
  frames, references and sound, and `demi video models` shows them.
- Each maker keeps its own list of the options its request leaves out
  (`VideoMaker::unsent`), and an option on it is dropped from a request
  that is sent and paid for all the same.

The same cost follows wherever a vendor drops an option without refusing
it. Each of these is stated in the vendor's API reference, and the first
was run against the vendor:

- Seedance 2.x takes no seed. Ark accepts a request with one and makes the
  video, so `--seed` is paid for and does nothing.
- MiniMax H3 and xAI ignore an aspect ratio when a first frame is given.
- Seedance 2.5 makes a video of its own length, up to 30 seconds, when a
  call asks none.

Two more faults of the video command came up in the same calls:

- A Seedance video's cost is never shown. Ark reports the job's tokens,
  but only the Google maker prices tokens by the catalog's rates.
- A Seedance 480p video, 864×496, is reported as "asked for 480p, got
  864×496".

## Decision

**The catalog entry is the one statement of what a video model takes. A
call that asks for what the entry states the model does not take fails
before anything is sent, and costs nothing:**

```text
demi video generate: gemini-omni-1.1-flash takes --duration 3 to 10
demi video generate: doubao-seedance-2-0-260128 takes no --seed
demi video generate: MiniMax-H3 takes no --aspect with --image: the image sets the shape
```

Every option the entry does not exclude is sent. A fact the entry does not
state is unknown, and the option is sent, as the catalog's rule for unknown
facts has it. A model that makes sound always, such as Omni, honors
`--audio` without a field for it, and a silent one `--no-audio`; asked the
other, each fails.

`demi video edit` and `demi video extend` take no `--aspect`: the vendors
keep the shape of the video they change, so the option asked nothing a
vendor does.

A call that asks no length gets the shortest the entry states, so a video
is never longer, and dearer, than the agent asked for. A vendor's own
default is often its longest: Omni makes 10 seconds when asked none, and
Seedance 2.5 a length of its own choosing, up to 30. A model whose entry
states no lengths is sent none.

### The catalog

`VideoCapabilities` gains three facts, each optional, so the file keeps
its version:

```rust
/// Whether a call can ask for a video without sound.
pub audio_off: Option<bool>,
/// Whether it takes a seed.
pub seed: Option<bool>,
/// Whether it takes an aspect ratio beside a first frame; false where the
/// image sets the shape.
pub aspect_with_first_frame: Option<bool>,
```

The entries of the four video vendors Demi supports, Google, xAI,
ByteDance (BytePlus and Volcengine) and MiniMax, state every fact their
APIs state.

### The makers

- `VideoMaker::unsent`, `unsent_of`, `VideoOption` and the "takes no"
  warnings go. A maker sends each option the request carries.
- The Omni maker sends a length as `response_format.duration`, such as
  `"4s"`.
- One test reads the catalog and, for every model each maker makes,
  builds a request with each option the entry does not exclude and checks
  that the maker's body carries it. A new model in the catalog is checked
  by the same test.
- The check before sending lives with the call in the user shard
  (`crates/backend-user-shard/src/shard/media.rs`), beside the check of a
  change the maker does not make.

### Cost and size

- Tokens are priced by the catalog's rates in one place,
  `provider-common`, for every maker whose vendor reports tokens: Google's
  and Ark's. The Google maker's own pricing goes.
- A result whose shorter side is within 5% of the asked resolution counts
  as that resolution, so 864×496 is 480p.

### Compatible entries and endpoints

- A compatible API entry no longer chooses an image or video shape:
  `ImageShape::CHOSEN`, `VideoShape::CHOSEN`, the OpenAI Videos and
  OpenRouter Videos shapes and their makers go. A migration drops the
  image and video choice of an existing compatible entry, which keeps its
  text models.
- A vendor entry's endpoint can be edited in Settings › Providers, where
  it is shown and never edited today. It starts as the vendor's, and
  clearing it returns to the vendor's. A gateway that serves a vendor's API
  is that vendor's entry with the gateway's endpoint.

```text
Volcengine Ark
  API Key    ••••••••••••••••              [Change]
  Endpoint   https://gateway.example.com/api/v3
             Default: https://ark.cn-beijing.volces.com/api/v3
```

## Spike

Each point ran against the vendor with a paid key, at the lowest
resolution and length the model takes.

- **Omni's length.** A request with `response_format.duration: "3s"` made
  a 3.03-second video. Google's guide and API reference do not list the
  field; Google's own Omni client sends it.
- **Seedance's seed.** Seedance 2.0 fast made a video for a request with
  `seed: 7` and said nothing of it.
- **Seedance without sound.** Seedance 1.0 pro fast, which makes no sound,
  made a silent video for `generate_audio: false`.
- **xAI.** A 2-second, 480p, 1:1 request made a 2.04-second 480×480 video:
  every option sent was honored.

## Prior art

- LobeHub states each video model's parameters in one place, its model
  bank, such as Omni's `duration: { min: 3, max: 10 }` and its aspect
  ratios and resolutions, and the video page's settings are made from
  them (`src/store/video/slices/generationConfig`)
  (`packages/model-bank/src/aiModels/google/index.ts` in
  [lobehub/lobehub](https://github.com/lobehub/lobehub)). Demi's catalog
  plays that part.
- The Vercel AI SDK's xAI video model returns an option it lacks as an
  `unsupported` warning on the result, after the call
  (`packages/xai/src/xai-video-model.ts` in
  [vercel/ai](https://github.com/vercel/ai)), which is what Demi does
  today and what this record turns into a refusal before the call.

## Alternatives

- **Keep the makers' lists and correct Omni's.** It fixes this case and
  keeps two lists that drift apart the next time a vendor adds a field.
- **Warn and send, as now, with the warning before the call.** The agent
  reads the warning only after the command returns, so the video is paid
  for either way.
- **Keep compatible entries' media shapes.** A gateway speaking a vendor's
  API is served as well by the vendor's entry with another endpoint, and a
  shape no vendor stands behind cannot be tested.
- **A call that asks no length gets the vendor's default.** The catalog
  would have to state each default to show it, and the default is often
  the longest and dearest length.
