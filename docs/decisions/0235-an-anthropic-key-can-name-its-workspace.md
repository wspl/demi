---
status: accepted
---

# An Anthropic key can name its workspace

Supersedes: 0211 (A compatible API sends custom headers), the part that says
a vendor's entry has no Headers.

## Problem

Anthropic's API keys come in two scopes. A key created for one workspace
works as it is. A personal or service account key that is not scoped to a
workspace must name one in the `anthropic-workspace-id` header of every
request, and without it every request fails:

```
400 invalid_request_error: This API key is not scoped to a workspace, so this
request must include the anthropic-workspace-id header with the ID of the
workspace to use.
```

Anthropic now recommends these identity-backed keys over workspace keys, so
a new key is often one of them. Alex adds Anthropic in Settings › Providers
with such a key and sends a message: the title request and the answer both
fail with the error above, and nothing in the entry can fix it. The message
tells Alex to add a header, but a vendor entry takes no headers of its own:
only a compatible entry does, because a vendor's API takes the headers its
vendor documents, which its provider sends.

## Decision

A vendor's API can take headers its vendor documents for the user to set,
such as the workspace Anthropic's key names. The family declares them, and
an entry of that vendor shows one row for each in its connection settings,
after API Key. An empty row sends nothing; a row with a value sends it on
every request of the entry.

Anthropic declares one, `anthropic-workspace-id`, which Alex's entry then
shows as:

```
Connection
  Endpoint        https://api.anthropic.com/v1
  API Key         ••••••••••••••••                    [Edit]
  Workspace ID    [                                 ]
                  For a key not scoped to a workspace.
  Prompt Cache    1 hour ▾
```

A MiniMax entry speaks the same API through the same provider, has no
workspaces and declares none. A compatible entry keeps its own list of any
headers.

Design:

- `ProviderFamily` gains `fn vendor_headers(&self, vendor_id: Option<&str>)
  -> &'static [&'static str]`, the names a vendor entry may set; empty by
  default. The Anthropic family answers `["anthropic-workspace-id"]` for its
  own vendor, a name `provider-anthropic-api` owns beside
  `anthropic-version`.
- The values are the entry's existing `headers`, which a vendor entry now
  holds too: only names its family declares, and only those with a value.
  No field is added and nothing migrates; a backend before this one reads
  the list and sends none of it for a vendor entry.
- `PATCH /providers/{id}` takes `headers` for a vendor entry as for a
  compatible one, and refuses a name its family does not declare.
- The entry the page reads lists, for a vendor entry, each declared header
  with its value, empty when unset; the page draws one row for each. A row's
  name and description come from the page's strings by the header's name,
  and a header without strings shows its name.
- The entry's client sends the headers to its endpoint, its vendor's default
  when it names none, as it sends a compatible entry's. The connection test
  and the title request go through the same client.
- A wrong ID fails the way Anthropic answers it, `anthropic-workspace-id
  header must be a valid workspace ID.`, shown like every other failure.

## Spike

One key not scoped to a workspace, against `api.anthropic.com`, with
`/v1/messages/count_tokens` and `/v1/models`, which run no inference:

- Without the header, both answer the 400 above.
- With the workspace's ID, both answer normally.
- With a made-up ID, both answer 400 `anthropic-workspace-id header must be
  a valid workspace ID.`
- The same key reads `GET /v1/organizations/workspaces` and lists its
  workspaces with their names, which the alternative below would use.

## Prior art

- Anthropic's own CLI takes `--workspace-id`, and its SDKs' examples set the
  header through `default_headers`
  ([Authentication](https://platform.claude.com/docs/en/manage-claude/authentication#select-a-workspace)).
- The [Terraform provider for Anthropic](https://github.com/ippontech/terraform-provider-anthropic/pull/333),
  [Spice](https://github.com/spiceai/docs/pull/2346) and other tools added an
  optional `workspace_id` setting that sends the header; none detects the
  key's scope.
- opencode and the [Vercel AI SDK](https://github.com/vercel/ai/blob/main/packages/anthropic/src/anthropic-provider.ts)
  have no such setting: their Anthropic provider takes any extra headers,
  which Demi keeps to compatible entries.
- Claude Code reads `ANTHROPIC_WORKSPACE_ID` only for workload identity
  federation, not for API keys.

## Alternatives

- **A workspace field on every entry.** An `anthropicWorkspaceId` in the
  stored configuration, the page's entry and its patch is one vendor's
  setting in what every vendor shares, and OpenAI's `OpenAI-Organization`
  and `OpenAI-Project` would each add another.
- **Any headers on a vendor entry.** It would work, but a user has to learn
  the header's name from the vendor's docs, and a vendor entry would carry
  headers its vendor never documented.
- **A map of options each family declares,** as opencode's `options`. It
  also holds a setting that is no header, such as a region; every setting a
  vendor needs today is a header, so the map waits for one that is not.
- **A Workspace menu filled from the key's workspaces.** After the key is
  saved, the backend lists them and the row is a menu of their names, with
  None for a scoped key. Listing is an Admin API call that a key without the
  right role may be refused, so the row still needs a text form, and the
  entry makes a second kind of request. It can follow later without changing
  what is stored.

## Open questions

None. The row is named *Workspace ID*, as Anthropic's Console and docs name
it: it sits inside the Anthropic entry, which tells it from Demi's own
workspaces.
