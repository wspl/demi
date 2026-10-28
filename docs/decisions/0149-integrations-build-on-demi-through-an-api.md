---
status: proposed
---

# An integration builds an agent product on Demi through an API

It builds on 0150 (Each agent works on its own Host) and 0151 (A user's
Clouds are kept per device). 0152 (An integration's plugin is served over
its socket) and 0156 (An integration's Clouds share a workspace and run its
init script) build on it.

## Problem

Many products are an agent with a front of their own:

* a bot in a team's group chat, where each group is a conversation and
  each member who speaks has the agent work in their own sandbox;
* a personal bot in a messenger, one conversation and one sandbox for its
  owner;
* a support agent on a website, one conversation per visitor;
* a review agent that a code host calls for each pull request;
* a service that gives each of its customers an agent and a sandbox.

Each needs what Demi already does: conversations, agents and subagents,
compaction, Clouds, the browser, the usage ledger. None can use it: the web
app's routes accept only a signed-in page's cookie, and their shapes follow
what a page draws. Each has to build the same agent server again.

## Decision

Demi is a server, and an **integration** is how a program uses it to build
its own agent product. Demi serves the work above the agent loop; the loop
itself, its tools and its providers stay Demi's.

```text
 an integration (any language)        Demi server
 ┌───────────────────┐  REST          ┌──────────────────────────────┐
 │ its front: a chat,│───────────────▶│ integration edge ─┐          │
 │ a site, a hook    │  WebSocket     │                   ├─▶ shard  │
 │                   │◀───────────────│ web edge ─────────┘          │
 └───────────────────┘ events         └──────────────┬───────────────┘
                                                     │ runner wire
                                          ┌──────────┴─────────┐
                                          ▼                    ▼
                                      a Cloud              a Cloud
```

### An integration

A user makes an integration in Settings › Integrations, in the System group:

```text
Integrations
Programs that build on Demi through its API. Each has its own token,
Clouds and conversations, which no sidebar shows.

Your Integrations                                [Add Integration]
┌──────────────────────────────────────────────────────────────┐
│ Team Chat Bot                                              › │
│ 14 Clouds · 238 conversations                                │
└──────────────────────────────────────────────────────────────┘
```

* Add Integration makes one at once, named *Integration* (*Integration 2*
  for the second), and opens its page: its name, its token, and a switch
  per capability.
* Its token authenticates every request of the integration as a bearer
  token. The page shows it whole, with Copy, only in the answer that made
  it, once; storage keeps its hash. Replace Token asks first, then shows
  the new one the same way.
* An integration infers with every provider its creator has. A product
  makes a Demi user of its own for its integration, which keeps its
  providers and its shard apart from anyone's personal work.
* Its conversations are in a workspace of its own, which no switcher,
  sidebar, search or project list shows: its creator sees how many it
  holds, never their transcripts.
* A conversation and a Cloud may carry a key, a string the integration
  chooses, such as JSON naming a group chat and a thread, unique within
  the integration. A request with a key that one has answers that one, so
  a retry or two concurrent requests get the same thing; Demi never reads
  what a key says.

### Who the agent works for

An integration's Clouds are the user's Clouds of 0151 that it made. Its
conversation works in the home of one of them, and the integration moves
it to another between two turns: a group chat's bot moves the group's
conversation to Bo's Cloud before the turn Bo started, so the agent works
with Bo's files and sign-ins. A subagent Alex's turn started keeps working
in Alex's Cloud (0150).

### What an integration's agents can do

| Capability | For an integration |
| --- | --- |
| Subagents, browser, web search, skills, media, simulators and emulators, schedules, attachments | Each on or off on its page and through the API |
| Credentials, notifications, `demi host`, other conversations | Never |
| Permission requests | Always allowed: no person answers them |

The toolset a tree opens with depends on its conversation: an integration's
has the plugins and groups of its capabilities and a revision of its own.

### What an integration's agents read

* The integration passes a **workflow** when it makes a conversation, in the
  format and through the parser of `.demi/workflows/*.yaml`; the integration
  keeps its own catalog. Its subagent profiles are what the conversation's
  agents spawn by while the integration has subagents on, and its
  `instructions` reach the main agent as a workflow's do, the product's
  identity and conventions among them.
* A Host's workflow files and instruction files and the user's personal
  instructions never apply.
* The guide every agent reads is the runtime's rules, then the web app's
  display conventions; an integration's agents read only the rules.

### The API

Every route is under `/v1`, carries the integration's token as
`Authorization: Bearer <token>`, and reaches only the integration's own
Clouds and conversations: any other id answers 404. A body is JSON, and an
error is the web API's error body. `GET /v1/openapi.json`, which asks for no
token, is the OpenAPI document, made from the protocol's types; it holds the
socket's events as JSON Schema.

| Route | Does |
| --- | --- |
| `GET /integration`, `PATCH /integration` | The integration; a change of its capabilities |
| `GET /models` | The models its conversations may run on: its user's |
| `GET /clouds`, `POST /clouds` | Its Clouds; a new one, or the one of a key |
| `GET /clouds/{cloud}`, `POST /clouds/{cloud}/reset` | A Cloud and its state; a reset of its system, `/home` kept |
| `GET /conversations`, `POST /conversations` | Its conversations; a new one on a Cloud with a model and a workflow, or the one of a key |
| `GET`, `PATCH`, `DELETE /conversations/{conversation}` | A conversation with its agents; a move to another Cloud or a new title; its deletion |
| `POST /conversations/{conversation}/messages` | A message, with an id a retry repeats; queued while a turn runs, or with `steer` added to the running turn |
| `POST /conversations/{conversation}/stop` | A stop of every agent |
| `GET /conversations/{conversation}/agents/{agent}/transcript` | An agent's latest page of blocks and its version |
| `GET /socket` | A WebSocket of events |

The socket sends every change of the integration's conversations as it
happens, one JSON event per message:

* `transcript`: an agent's patches at the version they lead to, the same
  blocks and patches a page receives;
* `agent`: an agent started, ended, or changed state or Cloud;
* `conversation`: its status changed; a turn has ended when it leaves
  `running` and `compacting`, and `completed`, `stopped` or `error` says how.

It sends nothing from before it opened: a program that connects again reads
what it shows again, as a page does after a reload.

### Two edges, one logic

* The integration edge has its own routes and protocol crate,
  `integration-api-protocol`, apart from the web app's: no page-only shape,
  such as a follow or a page's read state, reaches an integration, and a
  change to a page breaks no integration.
* Each operation is one method of the user's shard that both edges call:
  making a conversation from what it starts with (`ConversationStart`),
  moving it, deleting it, reading a transcript's page. A message and a stop
  are frames the shard hands the conversation's agent server through a
  connection of its own, as a page's lane hands them.
* The events come from the trees themselves: the agent server asks the
  product for an observer of each tree as it opens, which hears every frame
  the tree emits and keeps no tree open, and the shard turns the frames of
  an integration's conversation into its events.

### What Demi stores

* An integration: its user, workspace, name, token's hash and
  capabilities.
* An integration's conversation: its key and its workflow, beside the
  conversation's own record.
* An integration's Cloud: its key, beside its device.

### Not part of this

* For 0152: an integration's own commands and context, served over its
  socket.
* For 0156: the init script and shared workspace of an integration's Clouds,
  usage per Cloud, the deletion of a Cloud and of an integration, and
  attachments through the API.
* An embeddable SDK or an in-process engine: Demi is a server, run by
  anyone who wants their own.
* An integration's own model tools, agent loop or provider code.
* A Host the integration runs itself, such as its own container.
* Identities for the integration's users: Demi knows an integration and
  its Clouds.
* Channels such as chat tools: an integration integrates them.
* For later: a promise of API stability and versions, Clouds across
  several machines, a page for people who are not Demi users.

## Spike

None.

## Prior art

* [Codex app-server](https://github.com/openai/codex/tree/main/codex-rs/app-server):
  a protocol of its own, separate from the TUI's internals, over one
  connection that also carries the server's requests to the client.
* [OpenCode server](https://opencode.ai/docs/server/): a headless server
  with an OpenAPI document, an event stream and SDKs generated from the
  document. Demi generates its document from its protocol types.
* [Claude Managed Agents](https://platform.claude.com/docs/en/managed-agents/sessions):
  agents, environments and sessions over REST and an event stream; an
  environment configures sandboxes the vendor runs, one per session. Demi
  departs in two ways: a conversation's Cloud changes between turns, and an
  integration's Clouds persist.
* [Notion's integrations](https://developers.notion.com/docs/authorization):
  a user makes an internal integration in settings, which has a token and
  capabilities the user switches.

## Alternatives

* **A TypeScript SDK that embeds the agent:** the product's state, Hosts
  and Clouds would live in every embedder; each would rebuild the server.
* **The web app's routes with a token:** every page-only shape would become
  a promise to integrations.
* **A user per person of the integration:** a new identity model Demi does
  not need; a Cloud already says whose sandbox a turn uses.
* **A conversation of the integration's following its tree like a page:** a
  follow attaches, which keeps the tree open while the integration listens.

## Open questions

None.
