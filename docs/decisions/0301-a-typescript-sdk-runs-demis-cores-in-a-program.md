---
status: proposed
---

# A TypeScript SDK runs Demi's cores in a program, over the web app's own clients

## Problem

0297 to 0300 give a program a process that runs cores, its runners and its
plugins, all over the web API on the process's standard input and output.
A program in TypeScript still has to start the process, speak HTTP/2 to
it, open lanes, apply transcript patches and product state changes, and
encode its plugins' messages. Each program would write that again, and
each would drift from the web app, which does the same work for its pages.

## Decision

`@demicodes/sdk` starts the process and gives a program the cores, their
conversations, providers, Hosts and state, and the program's plugins, with
the clients the web app uses.

```ts
import { Demi, definePlugin } from '@demicodes/sdk'
import { z } from 'zod'

const chat = definePlugin({
  id: 'chat',
  name: 'Chat Commands',
  description: 'Gives the agent `demi chat send`, which posts to the chat.',
  commands: {
    chat: {
      summary: 'The chat this conversation answers in',
      commands: {
        send: {
          summary: 'Posts a message to the chat',
          input: z.object({ text: z.string().describe('The message, in Markdown') }),
          output: 'the id of the message posted',
          failure: 'an error on stderr and exit status 1; nothing is posted',
          async run({ text }, call) {
            await call.stdout(await postToChat(text))
          },
        },
      },
    },
  },
})

const demi = await Demi.start({ plugins: [chat], publicUrl: 'https://bot.example.com/demi' })
const core = await demi.open('/srv/bot/team-a')
await core.hosts.startLocal()
await core.providers.create({ vendor: 'anthropic', apiKey: process.env.ANTHROPIC_API_KEY })

const conversation = await core.conversations.create({ title: 'Release notes' })
const turn = await conversation.run('Draft the notes for this week')
console.log(turn.text)
```

### Three layers

```text
public API       Demi, Core, Conversation, definePlugin     this record
clients          the lane client, ConversationClient,       shared with the
                 the product state, the generated schemas    web app
wire             HTTP/2 on the process's stdio (0297)        internal
```

The SDK writes no client of its own. What the web app does for a page and
a program needs too moves out of the web app, framework-free, into the
public packages both import:

- the web API's generated schemas (`packages/web/src/api/generated`) move
  into `@demicodes/protocol`, beside the lane schemas already there;
- the product state's applier (`withPart`, now in the web app's Pinia
  store) moves into `@demicodes/protocol`;
- the lane client (`page-connection.ts`, which holds two Vue refs of the
  connection's state) moves out of `web-ui`, its state given to whoever
  shows it;
- `ConversationClient` (`@demicodes/conversation-client`) is used as it is,
  over a lane of the process's connection.

### What each call is

| SDK | What it does underneath |
| --- | --- |
| `Demi.start({ plugins, publicUrl })` | starts `demi-core` from the package's release, speaks HTTP/2 on its stdio, `PUT /plugins` (0299), and gives the process the public URL (0298) |
| `demi.open(dir)`, `core.close()`, `demi.close()` | `POST /cores`, `DELETE /cores/{id}`, the connection's end (0297) |
| `core.state`, `core.on('state')` | `GET /state` and the synchronization lane, applied by `withPart` |
| `core.on('notify', handler)` | the `starter` lane; the handler's answer is how the notification was shown (`Notified`) |
| `core.conversations.create(options)` | `POST /conversations` (`CreateConversation`): the SDK makes the id, and the workspace is the core's first unless named |
| `core.conversations.open(id)` | a `ConversationClient` on the conversation's lane |
| `conversation.send(input)`, `steer`, `abort`, `retry`, `resume`, `compact` | the client's methods, which send `Send`, `Steer`, `Abort`, `Retry`, `Resume`, `Compact` |
| `conversation.run(input)` | `send`, then waits until the conversation's status leaves running, compacting and resuming, and answers the blocks after the message and its text |
| `conversation.events()` | the client's events, as an async iterator, with a `permission` event for each new request in the conversation's metadata |
| `request.allow(values?)`, `request.deny()` | `POST /conversations/{id}/permissions/requests/{request}` (`DecidePermission`) |
| `core.providers.list`, `create`, `update`, `remove` | `/providers` (`CreateProvider`, `ProviderPatch`, whose `rotation` is the accounts' rotation) |
| `core.providers.accounts(id)` | the entry's account routes: list, activate, order, remove |
| `core.providers.signIn(options)` | `POST /providers/subscription-login`; the login's states come from the product state's `logins`, and `submitCode`, `cancel` are its code and delete routes |
| `core.hosts.create()` | `POST /devices` (0298): the record a runner boots with |
| `core.hosts.startLocal()` | `create`, then starts the package's `demi-runner` with that record |
| `core.hosts.claim(code)` | `POST /devices/claim` |
| `core.fetch(request)`, `core.accept(socket)` | the core's routes under `/cores/{id}/api`, for the program's HTTP server to forward its pages' requests and sockets |
| `demi.fetch(request)`, `demi.accept(socket)` | the process's runner routes and schedule URLs (0298) |

### A plugin

`definePlugin` builds a manifest (`Manifest`) and its handlers:

- `id`, `name`, `description` are the manifest's.
- `commands` is the plugin's tree under `demi`: each key is a group's or a
  leaf's name. A group has `summary`, and may have `details`, `index` (its
  index entry) and `permissions`; a leaf has `summary`, `input`, `output`,
  `failure` (0300), and may have `stdin`, `rest`, `media`, `runningHint`,
  `permission`, `permissionWhen` and `bringsHost`, each its `GroupBuilder`
  or `LeafBuilder` counterpart.
- `input` is a Zod schema, whose JSON Schema (`z.toJSONSchema`) is the
  leaf's input, as a Rust leaf's comes from its type; a field's
  `describe` is its description.
- `run(input, call)` answers the leaf's `Command` request. `call` is its
  RPC port: `stdout`, `stderr`, `readStdin`, `awaitsUser`, `medium`. A
  `run` that returns ends with exit status 0, or the number it returns; a
  `run` that throws writes the error to stderr and ends with 1.
- `context`, `projectSearch`, `onConversationDeleted` and
  `onConversationForked` answer those requests, and declare them in the
  manifest by being there.

### Packaging

`@demicodes/sdk` depends on one package for each platform, as esbuild
does (`optionalDependencies`), each holding `demi-core`, `demi-runner` and
the release's command packages, of the SDK's own build. The SDK runs on
Node.js, Bun and Deno, which the wire's spike ran.

## Built on

| What the design uses | Where it is today | New |
| --- | --- | --- |
| The lane client | `web-ui/src/transport/page-connection.ts` | out of `web-ui`, without Vue |
| A conversation's client | `@demicodes/conversation-client`, public | |
| The schemas | `@demicodes/protocol`, and `packages/web/src/api/generated` | all in `@demicodes/protocol` |
| The product state's changes | `withPart` in the web app's store | in `@demicodes/protocol` |
| A conversation's status | `ConversationStatus` in its summary | |
| Permission requests | the metadata's `permissions`, and the decide route | |
| Sign-ins | the subscription-login routes and the product state's `logins` | |
| A leaf's declaration | `GroupBuilder`, `LeafBuilder` | from a plain object |

## Spike

None beyond those 0297 to 0299 run: the SDK's calls are the routes and
lanes those records try.

## Prior art

- **The Codex SDK** (`@openai/codex-sdk`): `new Codex()` starts the CLI as
  a child; `startThread()` and `resumeThread(id)` give a thread, whose
  `run(input)` answers the turn's items and final response, and
  `runStreamed(input)` its events as an async generator.
- **The Claude Agent SDK** (`@anthropic-ai/claude-agent-sdk`): `query()`
  streams a session's messages as an async generator; a program's tools
  are `tool(name, description, zodShape, handler)` in
  `createSdkMcpServer`, and `canUseTool` answers permission requests.
- **esbuild**: one package, and one optional package for each platform
  holding its binary.

Demi's calls follow the Codex SDK's shape for a conversation (`run`, an
async iterator of events), and the Claude Agent SDK's for a command (a
Zod schema and a handler), and depart where Demi's model differs: a
conversation lives in a core the program opens, a command sits in a tree
with groups and help, and a permission request is an event of the
conversation, answered by the route the web app uses.

## Alternatives

- **A client written for the SDK.** A second lane client, transcript
  applier and state store, which would drift from the web app's.
- **Callbacks for permission requests, as `canUseTool`.** A request waits
  in the conversation's metadata until someone decides it, the user in a
  page or the program, so it is an event of what the program follows, not
  a call the core makes into the program.
- **Commands as `tool()` definitions.** A flat list with no groups, help
  or `Output:` and `Failure:`, which Demi's agents read through `--help`.

## Open questions

- None.
