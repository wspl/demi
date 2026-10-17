# Image generation

The agent makes and edits images with the `demi image` group, through the
user's ChatGPT or Grok subscription. For example, a user asks for an icon
for their app: the agent runs
`demi image generate "a flat red fox head icon" --transparent > fox.png`,
looks at it with `demi file view fox.png`, then
`demi image edit "make the ears rounder" fox.png > fox-2.png`, and attaches
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
demi image generate <prompt> [--model <id>] [--aspect <W:H>] [--transparent] [--format png|jpeg|webp]
demi image edit <prompt> <image>... [--model <id>] [--aspect <W:H>] [--transparent] [--format png|jpeg|webp]
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
- **`--model`** names an image model that `demi image models` lists, and
  with it the provider entry that makes it. Without it, the call takes the
  first entry, in the order of Settings › Providers, that makes images, and
  that entry's default model.
- **`--aspect`** asks for a shape, such as `16:9`; **`--transparent`** asks
  for a transparent background, which needs PNG or WebP. A vendor that
  refuses a shape answers with the shapes it takes, and the call fails
  saying so, in the vendor's words and not from a list of Demi's, so a
  vendor that adds shapes needs no change: `demi image generate: Grok takes
  no 4:5 (it takes 1:1, 3:4, 4:3, 9:16, 16:9, …)`.
- `models` prints one line per image model of the user's entries, the
  entry's name beside it, grouped by entry, each entry's newest first by the vendor's release date, which
  each line shows, the default marked *default*, and what the vendor says the
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

The model is the one thing the result cannot show: no vendor names the model
that made an image. The ChatGPT backend accepts any model name, an invented
one included, and returns the same image, so a `--model` there is a request,
not a guarantee; Grok refuses a model it does not have.

## Vendors

| | ChatGPT (Codex sign-in) | Grok (Grok Build sign-in) |
| --- | --- | --- |
| Endpoints | `POST https://chatgpt.com/backend-api/codex/images/generations` and `/images/edits` | `POST https://cli-chat-proxy.grok.com/v1/images/generations` and `/images/edits` |
| Headers | The account's headers the provider sends for inference | The access token as `Authorization: Bearer` |
| Generate body | `prompt`, `model`, `background` (`transparent` or `opaque`), `quality: "auto"`, `size` (`auto`, or a size of the asked aspect) | `prompt`, `model`, `aspect_ratio` when asked, `response_format: "b64_json"` |
| Edit body | The same, with `images: [{ image_url: "data:<type>;base64,…" }]` | The same, with `images: [{ url: "data:<type>;base64,…", type: "image_url" }]` |
| Models (default first here; listed newest first by release date) | `gpt-image-2`, `gpt-image-2-2026-04-21`, `gpt-image-2.5-sunburst`, `gpt-image-2.5-sunburst-2026-09-08`, `gpt-image-2.5-flare`, `gpt-image-2.5-flare-2026-09-08`, `gpt-image-1.5`, `gpt-image-1.5-2025-12-16`, `gpt-image-1`, `gpt-image-1-mini`, `chatgpt-image-latest` | `grok-imagine-image`, `grok-imagine-image-2026-03-02`, `grok-imagine-image-2.0`, `grok-imagine-image-quality`, `grok-imagine-image-quality-20260403`, `grok-imagine-image-quality-latest`, `grok-imagine-image-pro` |
| Result | `data[0].b64_json`, PNG | `data[0].b64_json`, PNG or JPEG |

Each provider that makes images states its models, each with the vendor's
published one-line description, and makes one image for one account; the backend chooses the entry and the account. Neither vendor lists
its image models with the subscription's token (Grok's
`/v1/image-generation-models` answers 404 there, the ChatGPT backend refuses
every listing route), so each provider's list is the vendor's published IDs,
dated snapshots and aliases included, each with the release date the
vendor publishes (an alias such as `chatgpt-image-latest` takes that of the
model it names at the time of the list), and changes with a Demi release. The
ChatGPT default is the one Codex itself sends, and Grok's the first its
subscription answers by default; Grok's models are those the subscription
accepted, each tried once; the retired `dall-e-*` and
`grok-2-image*` are left out. A Grok answer by URL is never used: the URL
refused an immediate download. These request shapes are those of the
vendors' own CLIs, checked against both subscriptions in October 2026.

## Accounts and failures

The call takes an account of the entry as a new agent takes one at its first
request: the active account with Manual rotation, the first not used up with
In Order, the one with the most room with Balanced
([Account rotation](../providers/providers.md#account-rotation)). An account
that fails with `usage_limit` is used up, as for a turn; with In Order or
Balanced the call goes on at once with another account, and when none is
left, or with Manual, it fails. Each call writes a row of the
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
| No entry makes images | `demi image generate: no provider makes images; sign in to ChatGPT or Grok in Settings › Providers` |
| Every entry that could has Image Generation off | `demi image generate: image generation is off for ChatGPT and Grok; turn it on in Settings › Providers` |
| `--model` names a model of an entry with Image Generation off | `demi image generate: "gpt-image-2" is ChatGPT's, whose image generation is off in Settings › Providers` |
| `--model` names no listed model | `demi image generate: no provider makes "<id>"; demi image models lists them` |
| Every account is used up | `demi image generate: ChatGPT's usage limit is reached, resetting at 9:00 PM` |
| The vendor refused the prompt or an image | `demi image generate: ChatGPT refused the request: <vendor's message>` |
| The sign-in expired and cannot be renewed | `demi image generate: the ChatGPT sign-in for alex@example.com has expired; sign in again in Settings › Providers`; for a user of a shared instance's entries, `the ChatGPT sign-in has expired; ask your administrator to sign in again` |
| The vendor failed or did not answer in 120 s | `demi image generate: ChatGPT did not answer: <cause>` |
| The image does not decode | `demi image generate: ChatGPT returned an image that could not be read` |
