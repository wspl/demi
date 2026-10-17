# Generated media

The agent makes images, videos and audio with three command groups,
`demi image`, `demi video` and `demi audio`, through the user's provider
entries. For example, a user asks for a short clip of their app's logo
spinning. The agent runs `demi video models`, which lists every video model
the user's entries offer, newest first, each with what it takes, what it
costs and which entry would make it now; it picks `grok-imagine-video-1.5`,
which the user's Grok subscription makes, runs
`demi video generate "the logo spins once" --model grok-imagine-video-1.5 --image logo.png > spin.mp4`,
and attaches the file.

This document holds what the three groups share: which entries make which
media, how a call chooses its entry, and what the agent is told. Each
group's own commands and vendors are in [Images](images.md),
[Videos](videos.md) and, as it is built, Audio.

## Who does what

```text
plugin-image   plugin-video   plugin-audio     commands, help, checks, output
      \              |              /
       Providers service of the port            one call: kind, model, entry?, options
                     |
backend media      entries and accounts, the     choose entry and account, hand on
(provider-common)  catalog, the vendors' APIs     when used up, ledger, download
                     |
      ChatGPT · Grok · OpenAI · Google · xAI · BytePlus · Volcengine ·
      MiniMax · ElevenLabs · Stability AI · a compatible API
```

- **Three plugins**, one per group, so the user turns off one kind, as
  videos cost the most, and the agent then has no command for it
  ([Plugins](../architecture/plugins.md)). A plugin holds only its
  commands: their help, their checks of the options, and the bytes it
  writes.
- **One backend service** makes every kind, so choosing an entry, taking an
  account, handing on when one is used up, keeping the usage ledger and
  reading the catalog are written once: the image rules below are the
  video and audio rules too.

## Which entries make which media

Each provider entry states, for images, videos and audio, how its API makes
that kind, its *shape*, or that it makes none:

| Entry | Images | Videos | Audio |
| --- | --- | --- | --- |
| ChatGPT subscription | ChatGPT | — | — |
| Grok subscription | Grok | Grok | — |
| OpenAI key | OpenAI Images | — | OpenAI Speech |
| Google key | Gemini | Veo | Gemini speech, Lyria music |
| xAI key | OpenAI Images | xAI Videos | xAI Speech |
| BytePlus key | Seedream | Seedance | Seed speech and Seed Audio |
| Volcengine key (火山引擎) | Seedream | Seedance | Seed speech and Seed Audio |
| MiniMax key | MiniMax image | Hailuo | MiniMax speech |
| ElevenLabs key | — | — | ElevenLabs speech, music and sound effects |
| Stability AI key | — | — | Stable Audio music and sound effects |
| A compatible API | chosen by the user: OpenAI Images or none | chosen: OpenAI Videos, xAI Videos, Ark Tasks, OpenRouter Videos or none | chosen: OpenAI Speech or none |

Audio is speech, music and sound effects, `demi audio speech`, `music`
and `effect`, and a shape says which of them it makes. An ElevenLabs or
Stability AI entry makes audio only and has no text models, so it never
appears in the model menu. A vendor entry's shapes are its vendor's and are not edited; a compatible
entry's are chosen on its page and by `demi providers edit`, as its
protocol for text is, so a self-hosted vLLM-Omni server, OpenRouter or a
reseller that speaks a shape is reached with its address, its key and the
model names it serves ([Providers](../providers/providers.md)). The shapes
are the request and answer formats the vendors' APIs share; Images, Video
and Audio name each one's requests. A shape's model list is the entry's:
a vendor entry lists what its vendor's API lists, described by the catalog
([Image and video models](../providers/media-models.md)); a compatible
entry lists the models the user added for that kind, as it does for text.

Each entry that makes a kind has a switch for it on its page, **Image
Generation**, **Video Generation** and **Audio Generation**, on from the
start; turned off, the entry makes none of that kind and its models are
not listed. The switches belong to the entry, as its configuration does
([Image generation switch](../providers/providers.md)).

## Which entry makes a call

A model can be made through several entries: `gpt-image-2` through a
ChatGPT subscription, an OpenAI key, OpenRouter or a proxy. The agent names
the model, `--model`, always: no media comes from a model nobody chose. It
may also name the entry, `--provider <entry>`, by the name Settings shows,
when the user asked for one; without it the call goes to the first entry
that can make the model now:

1. subscriptions before API keys, since a subscription's media come with
   its plan while a key pays for each, and among each kind the order of
   Settings › Providers, which the user sets by dragging the list;
2. *can make it now*: its switch for the kind is on, it is signed in or
   has its key, it offers the model, and an account of it is not used up.

A named entry that cannot make the model fails the call, saying why, and
names the entries that can: the agent asked for that entry. An unnamed
call whose entry's last account reaches its limit while it runs goes on at
once with the next entry, as a call goes on with another account within an
entry ([Accounts and failures](images.md#accounts-and-failures)); a refusal
of the request itself, a policy block or a bad option, is never handed on,
as another entry would refuse it too or make something the user did not
ask for.

There is no setting that chooses an entry per vendor or per model: the
order of the list, the switches and `--provider` say everything such a
setting would, for every kind and every model at once.

## What the agent is told

The `models` command of each group is the agent's whole view, so it can
choose without searching the web or trying calls. For each model, newest
first by release date:

- its ID, the vendor's name and one-line description, and its release date;
- what it takes: sizes, aspect ratios and resolutions, durations, how many
  input images, frames, voices, whether it makes sound or a transparent
  background, as the catalog states;
- its price per image, per second or per thousand characters where the
  vendor publishes one, and *included in the plan* through a subscription;
- the entries that offer it, in the order a call tries them, with the one
  a call takes now first, and for each other why it would or would not be
  used now: *used up until 18:00*, *Video Generation off*, *not signed in*,
  *next*.

The listing follows the commands' output rules
([Output](../execution/commands.md#output)): a table of the models, one row
each with its ID, vendor, release date, the entry a call takes now and the
model's name, and `models <model>`, as `kubectl get` takes a name, with every fact above as labelled lines,
the entries in the order a call tries them as a table under its key;
`--json` prints all of it.

Each call's stderr says what was made, through which entry and what it
cost when the vendor says, as `demi image` does: `5 s 720p MP4 from
grok-imagine-video-1.5 (Grok, alex@example.com), $0.25`. An option the
catalog does not list for the model is sent anyway with a warning, as the
vendor may take more than it wrote down; the vendor's refusal is the
answer.

## Managing it from a conversation

`demi providers` changes all of it with the user's permission, as it
changes text models ([Managing providers from a conversation](../providers/models.md#managing-providers-from-a-conversation)):
`edit <entry> --image <shape|on|off> --video <shape|on|off> --audio <shape|on|off>`
sets a compatible entry's shapes and any entry's switches, `on` turning a
vendor entry's kind on with its fixed shape,
`models add <entry> <model> --kind image|video|audio` adds a model to a
compatible entry's list for that kind, and `order <entry>...` sets the
order of Settings › Providers.
