# Audio generation

The agent makes speech, music and sound effects with the `demi audio`
group, through an OpenAI, Google, ElevenLabs, BytePlus Speech or Doubao
Speech API key or a compatible API
that speaks OpenAI's speech shape. For example, a user asks for a voice-over
and a short jingle for their app's intro: the agent runs

```text
demi audio voices --model gpt-4o-mini-tts
demi audio speech "Welcome back, Alex." --model gpt-4o-mini-tts --voice coral --instructions "warm, unhurried" > welcome.mp3
demi audio music "a bright 15-second ukulele jingle, no vocals" --model music_v2 --duration 15 --instrumental > jingle.mp3
```

and attaches both.

The group is the built-in `audio` plugin's (`plugin-audio`), as `demi video`
is the `video` plugin's: it shares the media service, the choice of entry,
the accounts, the ledger and the output rules with images and videos
([Generated media](media.md)), and the shared plugin code of `plugin-media`.
It asks for no permission, as `demi image` does not.

## Commands

```text
demi audio speech <text> --model <id> [--voice <id>] [--instructions <text>] [--speed <x>] [--format mp3|wav|opus|flac]
demi audio music <prompt> --model <id> [--duration <s>] [--instrumental] [--seed <n>] [--format mp3|wav]
demi audio effect <prompt> --model <id> [--duration <s>] [--loop] [--format mp3|wav]
demi audio voices --model <id> [<search>]
demi audio models [<model>]
```

- **`speech`** reads the text aloud in one voice; **`music`** makes a piece
  from a prompt, its lyrics, structure, tempo and style written in the
  prompt as each vendor documents (`[Verse]`, `[Chorus]`); **`effect`**
  makes a sound effect or ambience. A text or prompt of `-` is read from
  stdin.
- **The audio's bytes go to stdout**, one file per call, in the format
  `--format` asks for, MP3 by default, where the vendor makes it; a vendor
  that makes another, raw PCM, as Gemini's older speech models and
  ElevenLabs' music and effects for `--format wav` do, gets it wrapped as WAV, and
  stderr says the format that came: `demi audio speech: warning: Gemini
  makes no MP3; wrote WAV`. Nothing is transcoded.
- **`--voice`** names a voice `demi audio voices` lists for the model, its
  vendor's built-in voices and the user's own, such as a cloned one, each
  with its ID, name, languages and the vendor's description; without
  `--voice` the vendor's default speaks where it has one, as Gemini's
  does, and a vendor with none, OpenAI and ElevenLabs, fails the call before
  asking, as no voice is chosen for the agent: `demi audio speech:
  ElevenLabs needs --voice; demi audio voices --model eleven_v4 lists them`. **`--instructions`** describes
  how to speak, for the models that take it. **`--speed`** is a factor.
- **`--duration`** in seconds, **`--instrumental`**, **`--seed`**,
  **`--loop`**: each is sent only when given and goes to the vendor as
  asked, as an image's options do
  ([Options are asked, results are checked](images.md#options-are-asked-results-are-checked));
  an option a vendor has no field for is not sent, with a warning: `demi
  audio music: warning: Lyria takes no --duration; say the length in the
  prompt`.
- **`--model`**, required, names a model `demi audio models` lists, with
  its kinds, speech, music or effect, a call of another kind failing
  with the kinds it makes.
- `models` and `models <model>` are as `demi image models`'
  ([What the agent is told](media.md#what-the-agent-is-told)): for each
  model its kinds, languages, longest text, such as OpenAI's 4,096
  characters, or length, whether it takes
  instructions, its formats and its price, per million characters, per
  minute or per song, from the catalog.

On success, stderr says what was made, as a video call does: `0:04 MP3
speech from gpt-4o-mini-tts (OpenAI), voice coral`, `0:15 MP3 music from
music_v2 (ElevenLabs)`, with the cost where the vendor reports one, as none
of these do yet. The plugin reads the file's header,
with `shared-types`' audio reader, for its format and length before a byte
reaches stdout; a file that does not read fails the call and writes
nothing. A length that differs from `--duration` by more than a second is a
warning.

## Vendors

Every vendor here answers within the call, so an audio call is one request
under the 120 seconds an image call has, not a job. An answer over 256 MiB,
counted as it arrives and, for a base64 one, as decoded, fails the call, as
a video's does; ten minutes of 44.1 kHz stereo PCM is about 106 MiB. A raw
PCM answer is wrapped only with the rate, channels and sample size its
vendor states, each read and checked where the answer is decoded; one it
states wrongly or not at all fails the call, never guessed.

| Entry | Kinds | Request | Answer |
| --- | --- | --- | --- |
| OpenAI key; a compatible API (OpenAI Speech) | speech | `POST /v1/audio/speech`: `model`, `input`, `voice`, `instructions`, `speed`, `response_format` | The audio's bytes |
| Google key | speech | `POST /v1beta/interactions`: `model`, the text as `input` with the instructions as its `speech_metadata` annotation's `style`, `generation_config.speech_config: [{ voice }]`, `response_format: { type: "audio" }` | The `model_output` step's base64 audio: WAV, or raw PCM (`audio/l16`, 24 kHz mono 16-bit) wrapped as WAV |
| Google key | music (Lyria) | `POST /v1beta/interactions`: `model`, `input`, and `response_format: { type: "audio", mime_type: "audio/wav" }` only for `--format wav` | The `model_output` step's base64 audio, MP3 unless WAV was asked |
| ElevenLabs key | speech | `POST /v1/text-to-speech/<voice>`: `text`, `model_id`; `output_format` as a query | The audio's bytes |
| ElevenLabs key | music | `POST /v1/music`: `prompt`, `music_length_ms`, `model_id`, `force_instrumental` (its `seed` is refused with a prompt, so `--seed` is not sent) | The audio's bytes; `pcm_44100` for `--format wav`, wrapped |
| ElevenLabs key | effect | `POST /v1/sound-generation`: `text`, `duration_seconds`, `loop`, `model_id` | The audio's bytes |
| MiniMax key | speech | `POST /v1/t2a_v2`: `model`, `text`, `voice_setting: { voice_id, speed }`, `audio_setting: { format }`, `output_format: "hex"` | `data.audio`, the audio as hexadecimal text, decoded; a `base_resp.status_code` other than 0 is the failure |
| BytePlus Speech key, Doubao Speech key | speech (Seed speech) | `POST /api/v3/tts/unidirectional` with `X-Api-Key` and the model as `X-Api-Resource-Id`, such as `seed-tts-2.0`, and on BytePlus the fixed `X-Api-App-Key` its reference requires: `req_params: { text, speaker, audio_params: { format, sample_rate: 24000, speech_rate } }`; `--speed 1.5` is `speech_rate: 50`, as the vendor counts 0.5× to 2× from −50 to 100; a speed outside that range is sent as the nearer end and the summary warns that it was | A chunked series of JSON objects, whose base64 `data` is joined until the object whose `code` is `20000000`; `pcm` for `--format wav`, at the 24 kHz mono 16-bit the request asked, wrapped |
| BytePlus Speech key, Doubao Speech key | speech and effect (Seed Audio) | `POST /api/v3/tts/create` with `X-Api-Key`: `model: "seed-audio-1.0"`, `text_prompt`, the text to speak or the effect to make, with the instructions before it, `references: [{ speaker }]` for speech, as `--voice` names a Seed speech 2.0 speaker, and `audio_config: { format, speech_rate }`, the speed counted as Seed speech counts it | The answer's base64 `audio` |

Voices come from each vendor's own list, read when asked for: OpenAI's
built-in voices, which no endpoint lists, from the catalog, as ByteDance's
speakers are, since its speaker list answers only a request signed with an
access key pair, Seed Audio taking the Seed speech 2.0 speakers, a custom voice
`voice_…` being sent as given, as the object `{ "id": … }` OpenAI takes; Google's
`GET /v1beta/voices`; ElevenLabs' `GET /v2/voices`, every page; MiniMax's
`POST /v1/get_voice` with `voice_type: "all"`, its system, cloned and designed
voices. A
compatible API's voices are what its `GET /v1/audio/voices` lists, or none,
in which case `--voice` is sent as given. Making a voice, a clone or a
designed one, is not a Demi command: each vendor asks for the speaker's
consent its own way, on its own site.

An entry's models are those its vendor lists for it, where it lists them
(ElevenLabs' `GET /v1/models` with `can_do_text_to_speech`), and the
catalog's otherwise, as a key's image models are. The two ByteDance speech
endpoints are `https://voice.ap-southeast-1.bytepluses.com` (BytePlus) and
`https://openspeech.bytedance.com` (Doubao Speech); neither has a default
speaker, so a speech call without `--voice` fails before asking, as
OpenAI's does, and so does MiniMax's. ByteDance's songs (豆包音乐) are left out, as their API takes
only requests signed with an access key pair, which no entry holds.
MiniMax makes no music, as it closed its music API to new users on
2026-08-20. Stability AI and xAI's speech come with later work: Stability's
audio API has no reference that reads without a browser, and xAI's speech
takes no model.

Each entry that makes audio has **Audio Generation** on its page, below
Videos, with its audio models newest first where the vendor dates them,
in the vendor's order otherwise, as ElevenLabs' are, and their kinds; a compatible
entry chooses its audio shape there, OpenAI Speech or None, and adds its
audio models with Add Audio Model ([Providers](../providers/providers.md)).

## Accounts and failures

The entry, its accounts and its failures are chosen and read as an image
call's are ([Accounts and failures](images.md#accounts-and-failures)). Each
call writes a ledger row with the purpose `audio`, the seconds made and
the cost the vendor reported. A
vendor's refusal prints its words: `demi audio music: ElevenLabs refused the
request: <vendor's message>`, as a lyric that quotes a song or names an
artist is refused.
