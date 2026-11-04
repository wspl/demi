---
status: accepted
---

# A media job outlives a restart of the server

Supersedes: 0243 (Gemini Omni makes, edits and extends videos), its choice
of a streamed request over a job in the background, and of the video within
the answer over the video at an address.

## Problem

Alex's agent runs `demi video generate` for a 10-second shot with Gemini
Omni, which takes about a minute and costs about a dollar. Google takes the
job. Fifteen seconds later the server's operator updates the server, and
the backend restarts. The command ends with

```text
error: i/o error: stream closed because of a broken pipe
```

and writes an empty file. Nothing remains of the job in Demi, so the agent
cannot get the video: it tries Google's API by hand with the key, schedules
a check every minute, and finds nothing. Alex pays for the video and for
the one asked again.

Any restart of the backend does this, an update or a crash:

- A video's job lives only in the call that asked for it: from the vendor's
  answer to the download, it is state of the backend process.
- When the call is dropped, the backend cancels the job at the vendors that
  can cancel, so even a vendor that kept the job finishes nothing.
- The command on the Host outlives the backend, but its call does not: the
  backend that starts again begins a new session with the runner, and the
  calls of the old one end.

## Decision

**A video's job, once the vendor takes it, is kept by the backend and
followed to its end across restarts, and the command that asked for it
gets its video.** The agent sees the restart as a line on the command's
stderr, and the video arrives as it would have:

```text
demi video generate: asked gemini-omni-1.1-flash (Google AI Studio), job v1_ChdOR…
demi video generate: running
demi video generate: reconnected; following job v1_ChdOR… again
5 s 1280×720 MP4 with sound from gemini-omni-1.1-flash (Google AI Studio), $0.48
```

The same holds when the runner restarts, as an update of the server makes
every paired device's runner update too.

An image or a sound is kept the same way wherever its vendor can make it a
job. Where the vendor answers only in one request, a restart ends the call
as it does today:

| Kind | A job | One request only |
| --- | --- | --- |
| Videos | Every vendor | None |
| Images | Google, an xAI key | OpenAI, Codex, the Grok subscription, Seedream, MiniMax |
| Speech | Google, MiniMax, Doubao Speech | OpenAI, ElevenLabs, BytePlus Speech |
| Music and sound effects | Google | ElevenLabs, ByteDance's Seed Audio |

A provider another Demi borrows (0245) works the same: the borrower's
backend keeps the video's job and follows it through the lender, and an
image or a sound over the loan stays one request.

### Every maker follows a job by its id

| Vendor | Ask | Follow |
| --- | --- | --- |
| Google, videos, images, speech and music | `POST …/interactions` with `background: true`, `store: true` and, for a video, `delivery: "uri"` | `GET …/interactions/<id>` |
| xAI and the Grok subscription, videos | `POST …/videos/generations` | `GET …/videos/<request_id>` |
| xAI key, images | `POST …/images/generations` or `…/images/edits` with `deferred: true` and no `response_format`, since xAI defers only an image it answers by address | `GET …/images/<request_id>`: 202 until done, then the address and the cost |
| ByteDance, videos | `POST …/contents/generations/tasks` | `GET …/contents/generations/tasks/<id>` |
| MiniMax, videos | `POST /v2/video_generation`, `/v1/video_generation` | `GET /v2/query/…`, `/v1/query/…` |
| MiniMax, speech | `POST /v1/t2a_async_v2` | `GET /v1/query/t2a_async_query_v2?task_id=<id>`, then the file by `/v1/files/retrieve`, a TAR whose audio is the entry that is not the timing |
| Doubao Speech, Seed speech | `POST /api/v3/tts/submit` with an `X-Api-Request-Id` | `POST /api/v3/tts/query` with the `task_id`, then the audio's address |

- Omni edits and extends a kept job by `previous_interaction_id` in the
  background as well, and its video is downloaded from its address with the
  key. The streamed request and the video inside the answer go.
- Every maker that makes jobs has the same pair, an ask that answers the
  job's id and a read of the job's state by that id
  (`provider-common`'s `media::jobs::Jobs`). A video maker always has it;
  an image or audio maker has it for the vendors above.

### What is kept

The control database, beside the usage ledger, gains `media_jobs`, one row
per job a vendor took. The control database, not the user's, holds it, so
the backend that starts finds every user's running jobs in one read:

```rust
pub struct MediaJob {
    /// The call that asked for it, and the place of the request among the
    /// call's media requests: a resumed call names both.
    pub of: JobOfCall,
    pub kind: MediaKind,
    /// The vendor's id, which every read of the job sends.
    pub job: String,
    /// When it was asked, from which the job's bound counts.
    pub asked_at: Timestamp,
    /// The call's usage row, written at the job's end with what the
    /// vendor reported: the conversation, the entry, the account and the
    /// model are in it.
    pub usage: UsageRow,
    /// What the command is answered besides the file: the vendor's name,
    /// the account's label, and the answer as far as the ask knew it.
    pub follow: Follow,
    pub status: MediaJobStatus,
}

pub enum MediaJobStatus {
    Running,
    /// Made, its file in the user's blobs until the call collects it.
    Made { blob: BlobRef, made: Made },
    Failed { refusal: MediaRefusal },
    Cancelled,
}
```

- The row is written when the vendor answers with the job's id, before the
  command is told it was asked.
- A backend that starts follows each `Running` job of every user, whether
  or not its command comes back, so every job ends and its cost is
  recorded. A job that outlives the bound of its kind ends as still
  running, as a call that waited that long does today.
- A row ends when its call collects the file, or reads its failure. The
  file leaves the blob store with it.
- A file no call collects, because its Host never came back, stays until
  its conversation is deleted, and leaves with it.
- A lender keeps no row: it keeps the job its borrower asked for in memory,
  under a handle it gave the borrower, until the job's bound. A lender that
  restarts loses it, and the borrower's next read is answered that the
  vendor's job is lost (`MediaRefusal::JobLost`).

### The command comes back to its job

- When the vendor takes a job, the backend tells the runner that the call
  can be resumed (`Inbound::RpcResumable`), unless its output began or its
  input is a pipe, either of which a call cannot start over.
- **The backend restarts.** When the runner meets a backend with a new
  session, it ends the calls of the old one as today, except a resumable
  call: that it sends again under its id as `RpcCall { resumed: true }`,
  and the process on the Host keeps waiting.
- **The runner restarts.** The runner tells the command's caller in the
  job's shell, or a command alias's process, the id of a resumable call
  (the command wire's resumable record). A caller that loses the runner
  before the end sends the command again to the next runner, naming the
  call, unless it wrote some of the command's output. The next runner sends
  it as `RpcCall { resumed: true }`.
- The backend checks a resumed call as it checks any call, finds the job by
  its call, marks the call resumable again and goes on reading it. A job
  that ended meanwhile answers at once. A call that comes back after it
  collected its file is told so.

### Stopping

- Ending the command leaves its job to nobody: the agent or the user stops
  it, the process ends, or its device is removed. A video's job is
  cancelled at a vendor that can cancel; an image's or a sound's, which no
  vendor cancels, runs to its end. Either records its cost, and leaves no
  row.
- A backend that stops, a connection that drops, and a runner's session
  that ends with a resumable call in it cancel nothing.

## Spike

Each point ran against the vendor with a paid key. The Gemini videos were
360p and 3 seconds unless a point says otherwise.

- **A job in the background.** A request with `background: true`,
  `store: true` and `delivery: "uri"` answered its id at once, read
  `in_progress`, then `completed`, and its video downloaded from its
  address with the key: 3.03 seconds.
- **An edit in the background.** A request naming that job by
  `previous_interaction_id`, in the background, with `delivery: "uri"`,
  came back in 31 seconds with the source changed as asked. 0243 found
  that Google failed this with `INVALID_ARGUMENT`; it no longer does.
- **An extension in the background.** The same, asking for 3 more
  seconds, came back in 46 seconds with the source and its continuation,
  6.04 seconds.
- **A dropped stream.** A streamed request whose connection was closed 12
  seconds in still made its video, which a read by its id returned. One
  job dropped by a backend's restart never did, and Google documents no
  rule for either, so Demi does not rely on it.
- **A Gemini image in the background.** `gemini-3.1-flash-lite-image` with
  `background: true` read `completed` within seconds, with its image in the
  interaction.
- **Gemini speech and music in the background.**
  `gemini-3.8-flash-lite-tts` read `completed` in 6 seconds with its audio
  in the interaction, and `lyria-3-clip-preview` in 16 seconds with an MP3.
- **An xAI image, deferred.** `grok-imagine-image` with `deferred: true`
  answered a `request_id` at once; its read answered 202 until the image
  was done, then 200 with its address and its cost, in 10 seconds.
- **An OpenAI image in the background.** OpenAI's Image API answers in one
  request. The Responses API makes an image in the background:
  `gpt-5.4-mini` with the `image_generation` tool, `background: true` and
  `store: true` read `queued`, then `completed` in 15 seconds. It cost the
  text model's 1,652 input and 58 output tokens besides the image's own,
  and the text model rewrote the prompt before passing it to the tool
  (`revised_prompt`).
- **MiniMax speech, asynchronous.** `speech-2.8-turbo` by
  `/v1/t2a_async_v2` answered a task id at once and read `Success` in 5
  seconds; its file is a TAR of the audio and its timing.
- **Doubao speech, a long-text job.** `/api/v3/tts/submit` took the key,
  model and speaker Demi sends `/api/v3/tts/unidirectional` today
  (`X-Api-Key`, `seed-tts-2.0`, a Seed speech 2.0 speaker) and answered a
  `task_id` with `task_status` 1; `/api/v3/tts/query` read 2, done, within
  6 seconds, with the audio's address. Doubao Speech keeps the audio for 7
  days.
- **Not run:** whether the Grok subscription's proxy takes `deferred` is
  unknown, and no subscription was at hand to try it, so the subscription's
  images stay one request, as today.
- **Without a job:** the Codex backend's image endpoints answer in one
  request, as the Codex CLI calls them, and so do these, by their vendors'
  API references: Seedream (one request or a stream), MiniMax images,
  OpenAI and ElevenLabs speech, ElevenLabs music and sound effects, and
  Seed Audio. No long-text job was found in BytePlus Speech's documentation.
- **ElevenLabs speech, cut.** A speech request cut 1.5 seconds in was still
  made in full, 19 seconds of audio, charged, and kept in the account's
  history. Its history id comes only with the answer, so the request can
  be found again only by searching the history for its voice, model, time
  and text. A streamed request cut 1 second in stopped, with 9 seconds of
  audio kept and charged. Music and sound effects are not kept in the
  history.

## Prior art

- LobeHub keeps every video job in an `async_tasks` table, with the
  vendor's id as `inference_id`, its status and its error, and follows it
  in the background and through the vendor's webhook where there is one
  (`packages/database/src/schemas/asyncTask.ts`,
  `apps/server/src/services/generation/videoBackgroundPolling.ts` in
  [lobehub/lobehub](https://github.com/lobehub/lobehub)). Its Omni requests
  are `background: true` with `delivery: "uri"`, edits included. Demi
  follows the same shape; it does not take webhooks, since polling works
  for every vendor and needs no address the vendor can reach.
- Google's and OpenAI's guides to background work both say to keep the
  id and read the job by it
  ([Google](https://ai.google.dev/gemini-api/docs/background-execution),
  [OpenAI](https://developers.openai.com/api/docs/guides/background)).

## Alternatives

- **An update waits for the jobs.** A crash still loses them, and an
  update waits minutes for a video.
- **The Host asks the vendor.** The command outlives the backend, but the
  key would have to reach the Host, which no provider's credential does.
- **Webhooks.** Google, ByteDance and MiniMax document a callback, xAI's
  video and image APIs document none, and a server behind a private
  network cannot be called. Reading the job serves every vendor.

- **ElevenLabs speech is found again in the account's history.** It is
  matched by its voice, model, time and text rather than by an id, an
  entry can take a while to appear, and a key limited to speech cannot
  read the history. ElevenLabs speech stays one request, and a restart
  ends it as it does today.
- **OpenAI images move to the Responses API.** A restart would no longer
  lose them, but every image would cost a text model's tokens more, and
  the text model would rewrite the prompt the agent wrote. OpenAI images
  stay on the Image API, and a restart ends them as it does today.
