---
status: accepted, superseded in part by 0259 and 0260
---

# Gemini Omni makes, edits and extends videos

## Problem

A Gemini API key makes no video in Demi. Its video role asks Veo with
`…/models/<model>:predictLongRunning`, and the Gemini API shut Veo down:
Google now lists no model with that method, so `demi video models` lists
none for the key. The one video model the key may call, Gemini Omni Flash
(`gemini-omni-1.1-flash`), answers through the Interactions API
(`POST …/interactions`) instead.

Omni also edits and extends a video, which no `demi video` command can ask
for: `generate` takes images but no video. An agent asked to "make the
boat in boat.mp4 red" or "make it ten seconds longer" has no way to say it.

## Decision

### What the agent runs

Two commands join `generate`, each taking the video to change:

```text
demi video edit "Make the boat red. Keep everything else the same." \
  --model gemini-omni-1.1-flash --video boat.mp4 > red-boat.mp4

demi video extend "The boat drifts out of frame to the right." \
  --model gemini-omni-1.1-flash --video boat.mp4 > longer-boat.mp4
```

- `edit` makes a video of the same scene changed as the prompt says; it
  keeps the source's length and shape.
- `extend` makes the source followed by its continuation, as one video:
  a 3-second source and a 10-second continuation come back as one
  13-second MP4.
- `--video <file>` is an MP4 of this Host of at most 64 MiB, relative to the
  current directory or absolute. The call holds it whole in memory, and a
  source Omni takes, of at most 10 seconds, is a few MiB.
- Both take `--model`, `--provider`, `--reference` (repeatable) and
  `--resolution`. `--aspect` is taken and warned of as unsent, since the
  result keeps the source's shape and Google refuses the field for an edit.
  `--image`, `--last-frame`, `--duration`, `--audio`, `--no-audio` and
  `--seed` are not options of `edit` and `extend`.
- The output is as `generate`'s: the MP4 on stdout; on stderr the job at
  once, each change of its state, then the result line, which names the
  source as the vendor took it:

  ```text
  asked gemini-omni-1.1-flash (Google), job v1_ChdFeGFtcGxl
  running
  13 s 640×360 MP4 with sound from gemini-omni-1.1-flash (Google), extending its job v1_ChdTb3VyY2U, $0.45
  ```

- `demi video models` shows whether a model edits and extends, and the
  longest source it takes, from the catalog's new `video.edits`,
  `video.extends` and `video.maxSourceSeconds`.

### A video Demi made is continued, not sent again

Google keeps each Omni job, an interaction, for 55 days on a paid key and
one day on a free one, and continues it by its id
(`previous_interaction_id`) without the video being sent. That path is the
only one that extends a video past 10 seconds (up to 40) or adds speech to
it, and the only one that works where a request leaves from the European
Economic Area, Switzerland or the United Kingdom: there Google refuses
every video a request names as its input, as base64 or as a file of the
Files API, the file of a video Omni just made included.

Google continues only a job whose video it sent within its answer, as
base64 in the stream; of a job whose video it sent at an address
(`delivery: "uri"`), it refuses the continuation as "HTTP video input". So
Omni's video comes within the answer, whatever its size: Google's guide
advises an address over 4 MB, but its stream carries a 12.8 MB video of
4K whole.

So a source is matched to the job that made it, by its bytes:

- The usage ledger keeps the SHA-256 of each video a call wrote,
  `usage_ledger.media_digest`, beside the job's id (`vendor_id`) it
  already keeps.
- `--video boat.mp4` whose digest is that of a video the user's Omni job
  made, through the same entry, continues that job: the request sends
  `previous_interaction_id` and no video, through that entry and its key.
  A `--provider` that names another entry, or a `--model` other than the
  job's, sends the file instead.
- Any other file, such as a video from a phone or one cut by another tool,
  is uploaded through the Files API, and the request's `input` names the
  file.
- A job Google no longer keeps, which it answers `not_found`, is not a
  failure of the call: the file is uploaded instead, with the warning
  `Google no longer keeps job v1_…; boat.mp4 was sent instead`.

The agent never names a job: it holds files, and a job id would not
survive a compaction of its context.

### The request

Every Omni call is one streamed interaction (`"stream": true`), which also
serves the job's states:

1. Each image, and a source video no kept job made, is uploaded through the
   Files API: a resumable upload's start, one upload of the bytes, then
   reads of the file until Google has processed it. Google deletes the file
   after 48 hours.
2. `POST {base}/interactions?alt=sse` with `model`, `input`, its files by
   their addresses, `response_format: { type: "video", resolution?,
   aspect_ratio? }`, and `previous_interaction_id` or
   `generation_config.video_config.task` (`edit` or `extend`) for an
   uploaded source.
3. The first event, `interaction.created`, names the job, which stderr
   prints at once. Google sends a keep-alive every 15 seconds or so, so the
   wait for an answer never runs out while the video is made; the job keeps
   its bound of 10 minutes. A failure may come as a line of JSON outside an
   event, which is read as one.
4. The `model_output` step holds the video as base64, which
   `JobState::Done` carries as `DoneVideo::Sent` instead of an address to
   download; `interaction.completed` holds the usage.

0246 made a video's job one stream of its steps, `VideoMaker::run`, which
the other makers answer from their `ask` and `state` reads through
`polled`. Omni's maker answers it from its events instead. A stopped call
closes the stream. Google documents no cancel of a streamed interaction, so
the call says, as for other vendors without one, that the job may run to
its end and be paid for.

The options map as follows:

| Option | Omni's request |
| --- | --- |
| prompt | the `text` item of `input`, as written |
| `--image`, `--last-frame`, `--reference` | `image` items of `input`, each an uploaded file, in that order, and the roles Google's guide declares at the head of the prompt, such as `[# Sources <FIRST_FRAME>@Image1] [# References <IMAGE_REF_0>@Image2]` |
| `--video` | a `video` item of `input`, an uploaded file, or `previous_interaction_id` |
| `--aspect` | `response_format.aspect_ratio` for `generate`; unsent for `edit` and `extend` |
| `--resolution` | `response_format.resolution`: `360p`, `720p`, `1080p` or `4k` |
| `--duration` | unsent: Omni has no field for it, and takes the length from the prompt, such as `[0-4s]` |
| `--audio`, `--no-audio` | unsent: Omni always makes sound, and the prompt asks for silence |
| `--seed` | unsent: Omni takes none |

The roles' declaration is the one text Demi adds to a prompt: it is the
field Google's API has for an image's role, and without it Omni guesses
the role of each image from the prompt's words.

### Cost

Omni charges tokens, which `interaction.completed` reports by kind: input
at $1.50 per million, video at $17.50 and thinking at $9.00. Google reports
no cost, so Omni's maker prices each kind at the catalog's rate for it,
`millionOutputTokens` gaining the modality `video`, and the call's line and
its ledger row carry that cost. An
extension charges the whole video it returns, the source's seconds
included. Measured at 360p, a second of video is about 1,930 tokens, so
$0.034; Google's guide gives 5,792 tokens, so $0.10, for a second at 720p.

### Models, shape and catalog

- The key's video models are those Google lists for it that the media
  catalog has as Google's video models. Google still lists the shut-down
  `gemini-omni-flash-preview`, which the catalog does not have, so it is
  not offered.
- `VideoShape::Veo` (`veo`) becomes `VideoShape::GeminiInteractions`
  (`gemini-interactions`), shown and named in a call's message as *Gemini
  Interactions*. No entry stores the shape: a Google entry's comes from its
  family, and a compatible entry may choose only the shapes of
  `VideoShape::CHOSEN`, which never held Veo; so no migration changes stored
  data. The `predictLongRunning` request and its operation reads go.
- Omni's catalog entry gains `edits: true`, `extends: true`,
  `maxSourceSeconds: 10` and its thinking price.

### What the other vendors meet

`edit` and `extend` reach every video maker through
`VideoRequest::source`. A maker whose request has no field for a source
refuses the call before asking, in the words `<model> edits no video` or
`<model> extends no video`; this record gives the field to Omni's maker
only. xAI's `…/videos/edits` and `…/videos/extensions` and Seedance's video
inputs fit the same commands, each a later record.

A borrowed Omni edits and extends as the lender's does, since the request
crosses the relay as it is (0246) and a maker's facts tell its changes. It
continues no job by its id: the id the borrower read is one the lender
made, not Google's, so the source goes as its file, which the lender
uploads.

## Spike

Each point ran against the Gemini API with a paid key, at 360p unless it
says otherwise, from a server in the European Economic Area and from a
computer outside it.

- **A job in the background.** `"background": true` with polling answers a
  generated video, but every continuation of a kept job asked that way
  failed with `INVALID_ARGUMENT`, while the same request without it
  succeeded. The stream succeeded for both, with its first event in about
  2 seconds and a keep-alive every 15. So Omni's calls stream.
- **Continuing a job.** An edit by `previous_interaction_id` came back in
  31 seconds with a 3-second video; an extension in 46 seconds with the
  source and its continuation as one 13-second video. Google refuses
  `previous_interaction_id` together with `video_config.task`.
- **An uploaded video.** From the server in the European Economic Area, a
  video in `input` was refused with `content_blocked`: "Using uploaded video
  or audio for video generation isn't available in your region." So was
  the Files API file of a video Omni had just made, which Google marks
  `GENERATED`. From the computer outside it, the same edit of an uploaded
  video succeeded. Google refuses `aspect_ratio` for an edit.
- **Delivery.** With `delivery: "uri"` the video came as a `…/files/<id>:
  download?alt=media` address, and a continuation of that job failed with
  "HTTP video input is not supported by the model". Without it the video
  came within the stream: 3.9 MB of 720p for 10 seconds, and 12.8 MB of
  4K for 3 seconds in one line of 17 MB, against the 4 MB the guide
  advises an address over. A continuation of the 4K job, from the server
  in the European Economic Area, succeeded.
- **Cost.** Each run reported its tokens by kind; a 3-second video took
  5,793 video tokens and about 200 thinking tokens.

## Prior art

- xAI's API gives editing and extension their own endpoints,
  `…/videos/edits` and `…/videos/extensions`, each taking the source video;
  its SDK has `generate()` and `extend()`, and the Vercel AI SDK passes a
  mode, `edit-video` or `extend-video`
  ([xAI video editing](https://docs.x.ai/developers/model-capabilities/video/editing),
  [xAI video extension](https://docs.x.ai/developers/model-capabilities/video/extension)).
  Demi takes the same split as two commands, as `demi image` has `edit`
  beside `generate`.
- OpenAI's Videos API and LiteLLM's `video_remix` continue a video by the
  id of its job ([LiteLLM videos](https://docs.litellm.ai/docs/videos)).
  Demi continues by the id too, but finds it from the file, since an agent
  keeps files and not ids.
- Google's guide streams or polls an interaction and declares image roles
  in the prompt ([Gemini Omni](https://ai.google.dev/gemini-api/docs/omni)).

## Alternatives

- **`--job <id>` to continue a job.** The agent would have to keep the id
  from an earlier call's stderr, across turns and compactions, and choose
  between `--job` and `--video` for each call; a wrong choice in Europe is
  a refused call. Matching the file gives the same request with nothing to
  remember.
- **One `edit` command whose prompt says whether to extend.** Omni would
  read either from the prompt, but xAI and Seedance ask for each
  differently, and the result's length and price differ: an extension
  returns and charges a longer video than its source.
- **Background jobs with polling, as the other shapes run.** Google fails
  every continuation asked in the background, which is the path Europe
  needs.
- **The video at an address, as Google's guide advises over 4 MB.** It
  downloads the raw file, but Google then cannot continue the job, which
  leaves no way to change a video in Europe, nor to extend one past 10
  seconds.
- **The source and the images as base64 in the request.** It saves an
  upload and a wait, but holds each file a third larger inside one JSON
  body, where the Files API takes it as it is.
- **Keeping the shape's name `veo`.** It would name a shut-down API for
  requests that are not Veo's.

## Open questions

None.
