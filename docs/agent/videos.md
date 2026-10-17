# Video generation

The agent makes videos with the `demi video` group, through the user's Grok
subscription, an xAI or Google API key, or a compatible API that speaks one
of the video shapes. For example, a user asks for their logo to spin once:
the agent runs `demi video models`, picks `grok-imagine-video-1.5`, then

```text
demi video generate "the logo turns once, slowly" --model grok-imagine-video-1.5 --image logo.png --duration 5 > spin.mp4
```

waits the half minute the vendor takes, and attaches `spin.mp4`.

The group is the built-in `video` plugin's (`plugin-video`), as `demi image`
is the `image` plugin's: it shares the media service, the choice of entry,
the accounts, the ledger and the output rules with images
([Generated media](media.md)), and adds only what videos need: a job that
takes minutes, the options a video takes, and the shapes of the vendors'
video APIs. It asks for no permission, as `demi image` does not.

## Commands

```text
demi video generate <prompt> --model <id> [--image <path>] [--last-frame <path>] [--reference <path>]...
                    [--duration <s>] [--aspect <W:H>] [--resolution 480p|720p|1080p|4k] [--audio | --no-audio] [--seed <n>]
demi video models [<model>]
```

- `generate` makes one video. A prompt of `-` is read from stdin.
- **The video's bytes go to stdout**, as the vendor made them, an MP4:
  `> spin.mp4` saves it. Nothing is re-encoded. Several videos are several
  calls.
- **`--image`** is the first frame, **`--last-frame`** the last, which
  needs `--image`, and each **`--reference`** an image the video should
  keep to, such as a character or a product; they are paths on the Host,
  read as `demi image edit` reads its images, PNG, JPEG or WebP up to
  16 MiB each.
- **`--duration`** in seconds, **`--aspect`** such as `16:9`,
  **`--resolution`**, **`--audio`** or **`--no-audio`** for the sound the
  model makes, and **`--seed`**: each is sent only when given, so the
  vendor's default stands otherwise, and each goes to the vendor as asked,
  as an image's options do ([Options are asked, results are checked](images.md#options-are-asked-results-are-checked)).
- **`--model`**, required, names a video model `demi video models` lists.
- `models` lists the video models of the user's entries and `models
  <model>` one model, as `demi image models` does
  ([What the agent is told](media.md#what-the-agent-is-told)): the
  durations, aspects, resolutions, whether it makes sound, how many
  reference images it takes and its price per second, from the catalog.

## A call is a job

Every vendor makes a video as a job: the call asks for it, the vendor
answers with the job's id, and the video exists only minutes later. The
plugin waits for it within the call, so the command ends when the video is
written or the job has failed:

1. **It asks** and writes the job to stderr at once, `demi video generate:
   asked grok-imagine-video-1.5 (Grok, alex@example.com), job 3f1c…`.
2. **It waits** by asking the vendor for the job's state every 5 seconds,
   as each vendor's own SDK does: no vendor announces a finished job to a
   caller without a public address, and a callback would need one. Each
   change of state is a line on stderr, *queued*, *running*, *running,
   40%* when the vendor reports progress, so the agent sees it moving.
3. **It downloads** the video from the address the vendor answers with,
   at once, as the address is temporary (an hour for some vendors). The
   entry's key goes only to the entry's own endpoint's origin, as Google's
   file address and OpenRouter's content address are, and never to another
   host, a storage bucket's or a redirect's to another origin, which gets
   the request without it. A video over 256 MiB fails the call, read as it
   arrives rather than whole first; a vendor's longest, 15 seconds at
   1080p, is tens of MiB.
4. **It checks** the MP4: it reads the file's header for its size,
   duration and whether it has a sound track, says what came back on
   stderr, `5 s 1280×720 MP4 with sound from grok-imagine-video-1.5 (Grok,
   alex@example.com), $0.25`, and warns where it differs from what was
   asked: `demi video generate: warning: asked for 1080p, got 848×480`. A
   file that is not an MP4 fails the call and writes nothing. The cost is
   the one the vendor reports, in its documented unit; a figure in an
   undocumented unit is not shown.

A job gets 10 minutes from the moment it is asked for to the end of the
download, the ask's own wait included, which the
slowest vendor's documented wait, Veo's 6 minutes, fits; past that the
call fails saying the job is still running at the vendor, with its id.
Stopping the call, by ending the command or the turn, stops the waiting
and cancels the job where the vendor can cancel one; elsewhere the job
runs to its end at the vendor and is paid for, which the call's first
stderr line has already named. The 120 seconds an image call has bound
each request here, the ask, every status read and the download.

## Vendors

| Shape | Who speaks it | Ask | State | Result |
| --- | --- | --- | --- | --- |
| Grok | Grok subscription | `POST https://cli-chat-proxy.grok.com/v1/videos/generations` | `GET /v1/videos/<request_id>`: 202 with `status: pending` and `progress` while running, 200 with `done`, `failed` or `expired` | `video.url`, downloaded without a header; `usage.cost_in_usd_ticks` |
| xAI Videos | xAI key; a compatible API | `POST https://api.x.ai/v1/videos/generations` | `GET /v1/videos/<request_id>`, as Grok's | As Grok's |
| Veo | Google key | `POST /v1beta/models/<model>:predictLongRunning` with `instances` and `parameters` | `GET /v1beta/<operation.name>` until `done` | `response.generateVideoResponse.generatedSamples[0].video.uri`, downloaded with the key, following redirects |
| OpenAI Videos | a compatible API, such as vLLM-Omni or a reseller | `POST /v1/videos` | `GET /v1/videos/<id>`: `queued`, `in_progress`, `completed`, `failed` | `GET /v1/videos/<id>/content` |
| OpenRouter Videos | a compatible API, OpenRouter | `POST /api/v1/videos` | `GET /api/v1/videos/<id>`: `pending`, `in_progress`, `completed`, `failed` | The completed answer's video address |

How each option goes into each shape:

| Option | Grok, xAI Videos | Veo | OpenAI Videos | OpenRouter Videos |
| --- | --- | --- | --- | --- |
| `--image` | `image: { url: "data:…" }` | `instances[0].image` | `input_reference` | `frame_images: [{ frame_type: "first_frame", … }]` |
| `--last-frame` | `last_frame` | `instances[0].lastFrame` | — | `frame_type: "last_frame"` |
| `--reference` | `reference_images` | `instances[0].referenceImages` | — | `input_references` |
| `--duration` | `duration` | `parameters.durationSeconds` | `seconds`, a string | `duration` |
| `--aspect` | `aspect_ratio` | `parameters.aspectRatio` | `size`, the vendor's size of that aspect at the resolution, 720p when none is asked, and of 16:9 when only a resolution is | `aspect_ratio` |
| `--resolution` | `resolution` | `parameters.resolution` | in `size` | `resolution` |
| `--audio`, `--no-audio` | `generate_audio` | — (Veo always makes sound) | — | `generate_audio` |
| `--seed` | — | `parameters.seed` | — | `seed` |

An option a shape has no field for is not sent, and the call warns
`demi video generate: warning: Veo takes no --no-audio; the video has
sound`; the result check says the rest. OpenAI's own Videos API and its
Sora models were shut down on 2026-09-24, so no OpenAI key makes videos;
the shape stays for the servers that kept it. BytePlus, Volcengine and
MiniMax speak Ark Tasks and Hailuo, which come with those vendors' entries
([Which entries make which media](media.md#which-entries-make-which-media)).

Each entry that makes videos has **Video Generation** on its page, below
Images, with its video models newest first, as Images has; a compatible
entry chooses its video shape there and adds its video models with Add
Video Model ([Providers](../providers/providers.md)). The Grok subscription's
models are the ones it accepted when tried in October 2026,
`grok-imagine-video-1.5`, `grok-imagine-video-1.5-lite` and
`grok-imagine-video`; a key's are those its vendor lists, read when the list
is asked for, described by the catalog.

## Accounts and failures

The entry, its accounts and its failures are chosen and read as an image
call's are ([Accounts and failures](images.md#accounts-and-failures)): a
used-up entry hands on before the job is asked for, never after, since a
job asked for is paid for. Each call writes a ledger row with the purpose
`video`, the seconds made and the cost the vendor reported, the cost as
soon as the vendor reports the job done, so a download that then fails, a
call stopped or a job past its 10 minutes still records what was paid. A job the
vendor fails prints its words: `demi video generate: Grok failed the
video: <vendor's message>`, and a job that expired at the vendor before it
ran says so.
