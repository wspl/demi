# Image generation

The agent makes and edits images with the `demi image` group, through the
user's ChatGPT or Grok subscription or an OpenAI, xAI or Google API key.
For example, a user asks for an icon
for their app: the agent runs
`demi image generate "a flat red fox head icon" --model gpt-image-2 --transparent > fox.png`,
looks at it with `demi file view fox.png`, then
`demi image edit "make the ears rounder" fox.png --model gpt-image-2 > fox-2.png`, and attaches
the one the user wants.

The group is the built-in `image` plugin's (`plugin-image`), as `demi web`
is the `web` plugin's ([Plugins](../architecture/plugins.md)): the user turns
it off as any plugin. The plugin owns the commands, their help, words and
checks; the vendor call goes through the port's Providers service, so the
subscription token never leaves the backend's vault and the account,
rotation and ledger rules below are the backend's
([Credential vault](../providers/providers.md#credential-vault)). It asks
the user for no permission, as `demi web search` does not: it spends the
user's own subscription, as the agent's turns do.

## Commands

```text
demi image generate <prompt> --model <id> [--aspect <W:H>] [--transparent] [--format png|jpeg|webp]
demi image edit <prompt> <image>... --model <id> [--aspect <W:H>] [--transparent] [--format png|jpeg|webp]
demi image models
```

- `generate` makes one image from the prompt; `edit` makes one image from
  the prompt and the images given, each changed, combined or used as a
  reference as the prompt says. A prompt of `-` is read from stdin.
- **One image per call.** The image's bytes go to stdout, as
  `demi browser screenshot` writes its PNG
  ([Images and large outputs](../browser/browser.md#images-and-large-outputs)):
  `> fox.png` saves it, and `| demi file view` shows it to the model. Several
  images are several calls. The bytes are PNG unless `--format` asks for
  JPEG or WebP, whatever the vendor sent: the plugin decodes the vendor's
  image anyway and encodes it again when the formats differ, so a file
  named `fox.png` is always a PNG.
- **An edit's images** are paths on the Host the command runs on, an
  attached Host's inside `demi host shell` included, relative to the
  shell's directory; the plugin reads them with the port's file reads
  ([Reading a conversation's files](../architecture/plugins.md#reading-a-conversations-files)).
  A file that is not PNG, JPEG or WebP, or is over 16 MiB, fails the call,
  naming it.
- **`--model`**, required, names an image model that `demi image models`
  lists, and with it its vendor, OpenAI, Google or xAI: the agent always
  chooses the model, so no image comes from a model nobody named. Which of
  the vendor's entries makes it is the user's setting
  ([Which entry makes an image](#which-entry-makes-an-image)).
- **`--aspect`** asks for a shape, such as `16:9`; **`--transparent`** asks
  for a transparent background, which needs PNG or WebP. A vendor that
  refuses a shape answers with the shapes it takes, and the call fails
  saying so, in the vendor's words and not from a list of Demi's, so a
  vendor that adds shapes needs no change: `demi image generate: Grok takes
  no 4:5 (it takes 1:1, 3:4, 4:3, 9:16, 16:9, …)`.
- `models` lists the image models of the user's entries grouped by
  vendor, each group headed by the entry its models go through now,
  `OpenAI, through ChatGPT (alex@example.com)`; a model only another entry
  of the vendor offers names that entry on its line. Each group is newest
  first by the vendor's release date, which each line shows when the
  vendor publishes one, and each line says what the vendor says the
  model is for, *newest, highest quality*, *faster and cheaper*, or *dated
  snapshot of gpt-image-2.5-sunburst*, and what it takes: its sizes or
  shapes, such as any `W×H` up to 3840 on the long edge for `gpt-image-2`
  or the three fixed sizes of `gpt-image-1`, and the shapes Grok's take; so
  the agent picks a model and a shape from the list instead of searching
  the web. The help gives the commands and their options only, no advice
  on when to use which; a provider that cannot confirm
  the model that answered says so once under its models.

On success, stderr says what came back, as a screenshot says what it
captured: `1254×1254 PNG from gpt-image-2 (ChatGPT, alex@example.com)`; a
provider that cannot confirm the model, ChatGPT's, writes *asked of*
instead of *from*: `1254×1254 PNG, asked of gpt-image-2.5-sunburst
(ChatGPT)`, so neither the agent nor the user reads a request as a
confirmed model. The
account is named only to a user who manages the entry; a user of a shared
instance's entries reads `(ChatGPT)`, as the account is the administrator's
([Scope](../providers/providers.md#scope)). The image is decoded before a
byte reaches stdout: one that does not decode whole fails the call and
writes nothing.

## Options are asked, results are checked

Every option goes to the vendor as asked, even where the vendor is known to
ignore it, in the field the vendor's API has for it (Grok's has none for a
transparent background, so that option is asked of ChatGPT only), and the result is compared with what was asked: a vendor that
starts to honour an option needs no change in Demi. A result that differs
adds a warning to stderr and still succeeds:

```text
demi image generate: warning: asked for 16:9, got 1254×1254
demi image generate: warning: asked for a transparent background, got an opaque image
```

A background is transparent when a pixel of the decoded image is, not when
its format merely has an alpha channel.

## Which entry makes an image

A vendor's model can be made through more than one entry: `gpt-image-2`
through a ChatGPT subscription or an OpenAI API key, `grok-imagine-image`
through a Grok subscription or an xAI key. Settings › Agent › Image
Generation, under Small Model, has one row for each vendor Demi makes
images with, OpenAI, Google and xAI, always all three, each with a menu of
**Automatic**, the default, and that vendor's image-making entries; after
*Automatic*, on the same line in smaller, secondary text, the row names
the entry it resolves to by the entry's name alone, in parentheses,
*Automatic (Codex)*, as
Small Model does, and the menu names each entry's account the same way
after its name: the first entry Automatic would
try, judged by all but the model, its switch, its sign-in or key and its
limit, as the row names no model. A vendor none of whose entries can make
images, because the user has none or switched them off, shows *Automatic*
with *Not available* after it, as small and a menu holding *Add Provider…*, which
opens Settings › Providers, so the user learns what Demi can make images
with and where to enable it, as macOS keeps a setting it cannot use in its
place, dimmed.

- **Automatic** takes the vendor's subscription entries first, then its
  API-key entries, each kind in the order of Settings › Providers, and
  uses the first that can make the model now: its Image Generation is on,
  it is signed in or has its key, it offers the model, and an account of
  it is not used up. A subscription's images come with its plan, while a
  key pays for each image, so a key is used only when no subscription
  can.
- **A chosen entry** is used while it can make the model; when it cannot,
  the call resolves as Automatic.
- **Used up during a call.** When the entry's last account reaches its
  limit, the call goes on at once with the next entry Automatic takes, as
  it goes on with another account within an entry
  ([Accounts and failures](#accounts-and-failures)). For example, Alex has
  a ChatGPT subscription and an OpenAI key: once ChatGPT's limit is
  reached, his images are made, and paid for, through the key; turning
  the key's Image Generation off makes the call fail with the limit
  instead.

The choice is the user's, kept in the preferences as `imageEntries`
([User preferences](../product/web-api-settings.md#user-preferences)),
so each user of a shared instance chooses among the entries they use.

The model is the one thing the result cannot show: no vendor names the model
that made an image. The ChatGPT backend accepts any model name, an invented
one included, and returns the same image, so a `--model` there is a request,
not a guarantee; Grok refuses a model it does not have.

## Vendors

The subscriptions:

| | ChatGPT (Codex sign-in) | Grok (Grok Build sign-in) |
| --- | --- | --- |
| Endpoints | `POST https://chatgpt.com/backend-api/codex/images/generations` and `/images/edits` | `POST https://cli-chat-proxy.grok.com/v1/images/generations` and `/images/edits` |
| Headers | The account's headers the provider sends for inference | The access token as `Authorization: Bearer` |
| Generate body | `prompt`, `model`, `background` (`transparent` or `opaque`), `quality: "auto"`, `size` (`auto`, or a size of the asked aspect) | `prompt`, `model`, `aspect_ratio` when asked, `response_format: "b64_json"` |
| Edit body | The same, with `images: [{ image_url: "data:<type>;base64,…" }]` | The same, with `images: [{ url: "data:<type>;base64,…", type: "image_url" }]` |
| Models | `gpt-image-2`, `gpt-image-2-2026-04-21`, `gpt-image-2.5-sunburst`, `gpt-image-2.5-sunburst-2026-09-08`, `gpt-image-2.5-flare`, `gpt-image-2.5-flare-2026-09-08`, `gpt-image-1.5`, `gpt-image-1.5-2025-12-16`, `gpt-image-1`, `gpt-image-1-mini`, `chatgpt-image-latest` | `grok-imagine-image`, `grok-imagine-image-2026-03-02`, `grok-imagine-image-2.0`, `grok-imagine-image-quality`, `grok-imagine-image-quality-20260403`, `grok-imagine-image-quality-latest`, `grok-imagine-image-pro` |
| Result | `data[0].b64_json`, PNG | `data[0].b64_json`, PNG or JPEG |

Each provider that makes images states its models, each with the vendor's
published one-line description, and makes one image for one account; the backend chooses the entry and the account. Neither vendor lists
its image models with the subscription's token (Grok's
`/v1/image-generation-models` answers 404 there, the ChatGPT backend refuses
every listing route), so each provider's list is the vendor's published IDs,
dated snapshots and aliases included, each with the release date the
vendor publishes (an alias such as `chatgpt-image-latest` takes that of the
model it names at the time of the list), as the catalog of image and video
models lists them for that subscription
([Image and video models](../providers/media-models.md)), so a model the
vendor adds needs a change to the catalog and no Demi release.
Grok's models are those the subscription accepted, each tried once; the retired `dall-e-*` and
`grok-2-image*` are left out. A Grok answer by URL is never used: the URL
refused an immediate download. These request shapes are those of the
vendors' own CLIs, checked against both subscriptions in October 2026.

The API keys, as each vendor's API reference documents them in October
2026; none was tried with a key yet:

| | OpenAI | xAI | Google (Gemini API) |
| --- | --- | --- | --- |
| Endpoints | `POST https://api.openai.com/v1/images/generations` and `/images/edits` | `POST https://api.x.ai/v1/images/generations` and `/images/edits` | `POST https://generativelanguage.googleapis.com/v1beta/models/<model>:generateContent` |
| Headers | `Authorization: Bearer <key>` | `Authorization: Bearer <key>` | `x-goog-api-key: <key>` |
| Generate body | `prompt`, `model`, `size` (`auto`, or a size of the asked aspect), `background` (`transparent` or `opaque`), `output_format: "png"`, `quality: "auto"` | `prompt`, `model`, `aspect_ratio` when asked, `response_format: "b64_json"` | `contents: [{ parts: [{ text }] }]`, `generationConfig: { responseModalities: ["IMAGE"], imageConfig: { aspectRatio } }`, the ratio when asked |
| Edit body | The same, with `images: [{ image_url: "data:<type>;base64,…" }]` | The same, with `image: { url: "data:…", type: "image_url" }` for one image and `images: [...]` for several | The same, with an `inline_data: { mime_type, data }` part for each image |
| Models | `GET /v1/models`: those whose ID starts with `gpt-image-` or `chatgpt-image-`; `created` is the release date | `GET /v1/image-generation-models`, every one, with its `aliases`; `created` is the release date | `GET /v1beta/models`, every page: those whose ID has the word `image` or `banana`, as `gemini-3-pro-image` and `gemini-nano-banana-2.1`, and that list `generateContent`; `description` is the vendor's line, and no date is given |
| Result | `data[0].b64_json` | `data[0].b64_json`, with `mime_type` | The first `inlineData` part of `candidates[0].content.parts`; a block is `promptFeedback.blockReason` or the candidate's `finishReason` |

A key's models are those its vendor lists for it, read when the list is
asked for, so a model the vendor adds needs no Demi release. What Demi
knows of a vendor's models, the one-line description, the release date
and the sizes or shapes, is the catalog's, which its subscription and its
key both read; a listed model the catalog does not know is shown
with what the listing says. Google's Gemini API is reached through
`generateContent`, not the newer Interactions API, because only its
answer's shape is documented for images; Google's Imagen models are shut
down.

## Accounts and failures

The call takes an account of the entry as a new agent takes one at its first
request: the active account with Manual rotation, the first not used up with
In Order, the one with the most room with Balanced
([Account rotation](../providers/providers.md#account-rotation)). An account
that fails with `usage_limit` is used up, as for a turn; with In Order or
Balanced the call goes on at once with another account, and when none is
left, or with Manual, the entry is used up and the call goes on with the
next entry Automatic takes, or fails when there is none. A key fails as
its vendor reads its failures for text
([Failures](../providers/providers.md#failures)): a usage or billing limit
is `usage_limit`, a refused key fails the call naming it, and a policy
block is a refusal in the vendor's words; a failure the provider does not
recognise prints the vendor's own message. Each call writes a row of the
[usage ledger](../providers/usage-and-quota.md#usage-ledger), with the
purpose `image`, the account, the model asked and the counts the vendor
reported. The quota a response reports, as the ChatGPT backend's `x-codex-*`
headers do, a refusal's included, updates the account's snapshot as a
turn's does ([Vendor quota](../providers/usage-and-quota.md#vendor-quota)).
The 120 seconds bound the whole call: the token's refresh, the request and
the reading of the answer or the refusal.

The words a failure prints, on stderr with exit status 1:

| Cause | Words |
| --- | --- |
| No entry makes images | `demi image generate: no provider makes images; sign in to ChatGPT or Grok, or add an OpenAI, xAI or Google key, in Settings › Providers` |
| No `--model` | `demi image generate: --model is required; demi image models lists them` |
| The key was refused | `demi image generate: OpenAI refused the API key; check it in Settings › Providers`; for a user of a shared instance's entries, `ask your administrator to check it` |
| Every entry that could has Image Generation off | `demi image generate: image generation is off for ChatGPT and Grok; turn it on in Settings › Providers` |
| `--model` names a model of an entry with Image Generation off | `demi image generate: "gpt-image-2" is ChatGPT's, whose image generation is off in Settings › Providers` |
| `--model` names no listed model | `demi image generate: no provider makes "<id>"; demi image models lists them` |
| Every entry that could make the model is used up | `demi image generate: ChatGPT's usage limit is reached, resetting at 9:00 PM` |
| The vendor refused the prompt or an image | `demi image generate: ChatGPT refused the request: <vendor's message>` |
| The sign-in expired and cannot be renewed | `demi image generate: the ChatGPT sign-in for alex@example.com has expired; sign in again in Settings › Providers`; for a user of a shared instance's entries, `the ChatGPT sign-in has expired; ask your administrator to sign in again` |
| The vendor failed or did not answer in 120 s | `demi image generate: ChatGPT did not answer: <cause>` |
| The image does not decode | `demi image generate: ChatGPT returned an image that could not be read` |
