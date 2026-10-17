# Image, video and audio models

Image, video and audio models are listed apart from the models a conversation talks
to, because no source tells Demi enough about them. A vendor's `/models`
answers with names at best, a subscription answers with none, and
models.dev, which [Models](models.md) reads for chat models, says only that
a model's output is an image, a video or audio: nothing of the sizes,
durations, voices or prices an agent chooses among. So Demi keeps a catalog of its own, in its
repository, which the backend reads from GitHub and every release carries.

For example, ByteDance releases Seedream 5.1 on a Tuesday. A maintainer adds
it to `catalog/media/v1/models.json` and merges it to `main` that afternoon.
Within a day every Demi backend reads the new file, and a user with an
entry of BytePlus sees `seedream-5-1` in `demi image models` with its
sizes and its price per image, without upgrading Demi.

## What the catalog holds

One entry per model a vendor publishes, keyed by the vendor and the model's
ID as the vendor's API takes it, for every vendor whose API Demi makes media with ([Which entries make which media](../agent/media.md#which-entries-make-which-media)). The catalog holds only what no other source
gives; which entries of the user's offer the model is the providers'
business ([Providers](providers.md)).

| Field | Meaning |
|---|---|
| `vendor`, `id` | The vendor, such as `openai`, `google`, `xai`, `byteplus` or `volcengine`, and the model's ID in its API; ByteDance's models are two vendors, BytePlus outside China and Volcengine (火山引擎: Ark and Doubao Speech) in China, as each has its own accounts, keys, model IDs and prices |
| `kind` | `image`, `video` or `audio` |
| `name`, `description` | The vendor's name for the model and its one-line description |
| `released` | The date the vendor released the model, so lists run from newest to oldest |
| `aliasOf` | For an alias such as `chatgpt-image-latest`, the ID it names at the time of the list, which is a model's and never another alias's |
| `image` | For an image model: the sizes it takes, as sizes (`1024x1536`), tiers (`2K`) or aspect ratios with resolutions, as the vendor states them; whether it edits, how many images it takes, whether it makes a transparent background |
| `video` | For a video model: its durations, resolutions and aspect ratios; whether it takes a first frame, a last frame and reference images, and how many; whether it makes sound |
| `audio` | For an audio model: what it makes, `speech`, `music` or `effect`, any of them; for speech its voices by name, or that the vendor lists them, its languages and whether it takes instructions on the delivery; for music and effects its durations, whether it takes lyrics, makes an instrumental or loops; the formats it returns |
| `price` | What one image, one video, one second, one minute, one song, a million characters or a million tokens of input or of output costs, in the currency the vendor charges in, US dollars or yuan, never converted, by size, quality, resolution or duration where the price depends on them, with the vendor's pricing page; a price the vendor states in credits is converted at the credit's price the same page states |
| `subscriptions` | The subscriptions that make the model, `chatgpt` or `grok`, for the models a subscription takes without listing them ([Vendors](../agent/images.md#vendors)) |

A capability the vendor does not state is left out, which means unknown:
the catalog describes, it does not limit. A command warns when the agent
asks for what the catalog does not list and sends the request anyway, as a
vendor may take more than it wrote down; the vendor's refusal is the
answer ([Options are asked, results are checked](../agent/images.md#options-are-asked-results-are-checked)).

The first entries are drawn from the model lists of OpenRouter and Vercel's
AI Gateway and the prices of LiteLLM's, then checked by hand against each
vendor's documentation; an entry names the page it was checked against.
Where models.dev states a model wrongly, such as an image model marked as
returning text, the fix goes to models.dev as well.

## Versions

The catalog lives in the repository at `catalog/media/v<schema>/models.json`,
one directory per schema version. Its shape is the backend's type for it,
the one decoder every reader uses, and a test decodes each version's file
with it, so a change to the file that the backend would refuse fails the
repository's checks before it is merged. A
backend reads the version it was built for, from the branch it was built
from, which the build records beside the version
([Rust executables](../delivery/package-versioning.md#rust-executables)):
`https://raw.githubusercontent.com/wspl/demi/<branch>/catalog/media/v1/models.json`.
A published release, built from `main`, reads `main`; a development build
deployed from `feat/demi-next` reads `feat/demi-next`, so a model added
there reaches the developer's server before any release; a build from a
checkout with no branch, or one GitHub has not got, reads none and keeps
the copy it carries.

- **A field added** that a backend may ignore keeps the version: every
  field but `vendor`, `id`, `kind` and `name` is optional, and a reader
  ignores fields it does not know.
- **A change an older backend would misread**, a field renamed or its
  meaning changed, starts the next directory, `v2`. The old directory stays
  and is still updated with new models for as long as a release that reads
  it is supported, as Cherry Studio keeps a schema's lane for the clients
  that read it.
- **Every release carries a copy** of its version's file, taken at build
  time, so a backend with no network, or one that GitHub refuses, still
  lists the models of its release.

The backend downloads the file at start and then once a day, with its
`ETag`, as it reads models.dev ([The models.dev document](models.md#the-modelsdev-document)),
and decodes it against the schema. It uses the newer of its copy and the
release's, by the file's `updated` field. A file that does not decode, or
one older than the release's, is not used and the backend logs why; it
keeps the copy it had.
