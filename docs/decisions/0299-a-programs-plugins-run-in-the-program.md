---
status: proposed
---

# A program's plugins run in the program, over a lane of each core's connection

## Problem

A program gives its agents commands and context of its own, such as a
`lark send` that posts to the chat a conversation answers in, or the
chat's last messages before each request. In Demi that is what a plugin
does: it declares commands, features and a context source, answers the
requests the plugin host sends it, and acts through its port. Today every
plugin is Rust linked into the backend, its messages passed as Rust
values. A program in TypeScript has no way to be one.

## Decision

A program's plugin is a plugin of the plugin contract (`plugin-interface`):
the same manifest, requests, replies and port messages, carried between
the process and the program on a lane of each core's page connection.

```text
program                                 demi-core
  PUT /plugins [manifest of lark] ----->  registers lark beside the
                                          release's plugins
  POST /cores { dir } ----------------->
  CONNECT /cores/{id}/api/connect
    open lane { plugin: "lark" } ------>  lark's instance in this core
                    <------------------- request 7: command `lark send`
    port 1 of 7: put blob (10 MB) ----->
                    <------------------- answer 1: blob
    reply 7: exit 0 ------------------->
```

### Declaring

- `PUT /plugins { plugins: [Manifest] }` takes the program's manifests
  (`Manifest`, which is serializable already) before the process's first
  core opens: the registry is fixed when the process's services start, as
  the server's is (`Registry::new`), and the process's services start as
  its first core opens. A manifest the registry refuses, such as one whose
  id a release's plugin has, answers 400; after a core has opened, the
  call answers 409 `plugins_fixed`.
- For each manifest, the process registers a plugin whose instance in a
  core passes each request to the program and each of the program's port
  messages to the core's port. The plugin host treats it as any plugin:
  its commands join the command set, the user turns it and its features on
  and off, and its context is asked before each request.
- An instance learns whose core it serves: `PluginFactory::instance` takes
  the core's id, so a program plugin's instance sends its requests on that
  core's lane. A plugin linked in ignores it.

### Running

- The program opens one lane on each core's connection for each of its
  plugins, `{ kind: "plugin", plugin: "<id>" }`, a new `LaneTarget`. Only
  the program that opened the core may; a plugin it did not declare is
  refused `unknown_plugin`, and a second lane of one `plugin_lane_open`.
  A request while the lane is not open fails as a plugin that failed
  (`PluginError::Failed`); the lane's end ends the requests it carried
  (`PluginError::Ended`).
- Each message has the number of the request it belongs to: a request and
  its reply, a port message and its answer, and a cancellation. A port
  message may follow its request's reply, as the contract allows a page
  call's task to keep its port.

```text
core -> program   { request: 7, call: Request }
program -> core   { request: 7, reply: Reply }
                  { request: 7, error: PluginError }
program -> core   { request: 7, port: 1, message: PortMessage }
core -> program   { request: 7, port: 1, answer: PortAnswer }
                  { request: 7, port: 1, error: "the request is over" }
core -> program   { request: 7, cancel: true }
```

A port message whose request is over, or that the core failed to pass on,
is answered with `error` rather than left waiting.

### Bytes

Several messages hold bytes: a blob a plugin puts or reads, a file a Host
read, an image, a video or audio a vendor made, and an HTTP request's
body and answer (`B64Bytes`). In process they are bytes. On the lane a
message that holds bytes is a binary lane message: the JSON, in which each
value of bytes is replaced by its index, then the bytes, as VS Code's
extension host passes buffers. A message with none is a text lane message
of the JSON alone.

```text
u32 json length | json: { ..., "bytes": { "$$ref$$": 0 } } | u32 length | bytes
```

The encoder sets a scope in which `B64Bytes` writes its buffer's index and
keeps the bytes apart (`shared-types`'s `to_json_apart`), and the decoder
one in which it reads them back. A format that is not human readable cannot
take them out: serde reads an internally tagged enum's fields through a
buffer that reports itself human readable, and every message here is one.

A message is read whole into memory, so a lane bounds it: a plugin's lane
takes messages of up to 64 MiB (`MAX_PLUGIN_MESSAGE_BYTES`), the largest
media a plugin passes whole, where every other lane takes 1 MiB.

## Built on

| What the design uses | Where it is today | New |
| --- | --- | --- |
| The plugin contract | `plugin-interface`: `Manifest`, `Request`, `Reply`, `PortMessage`, `PortAnswer`, `PluginError` | |
| One instance for each core | `PluginFactory::instance` | takes the core's id; its messages go to the program |
| A registry fixed at the services' start | `Registry::new` | the process's services start at the first open, after `PUT /plugins` |
| A lane with binary messages | `shared-lanes` | `LaneTarget::Plugin`, and a larger message limit |
| Bytes in JSON | `B64Bytes` as base64 | a scope that keeps them apart |
| The contract across a wire | the entry comment's promise; the JSON loopback that kept it was deleted | first run |

## Spike

None: every message of the contract is serializable, which its crate's
rules require of each type, so what crosses the lane is what the types say;
the lane carries binary messages already.

The record also brings the entry comment of `plugin-interface` up to date:
it names a loopback that no longer exists, and a JSON Schema of every
message type, which only some of them derive.

## Prior art

- **VS Code's extension host**
  (`src/vs/workbench/services/extensions/common/rpcProtocol.ts`):
  extensions run in another process; a message's JSON names each buffer by
  index (`$$ref$$`), and the buffers follow the JSON in the same binary
  message. Requests carry numbers, and a cancellation names one.
- **The Language Server Protocol and the Model Context Protocol**: a
  server process and its client exchange JSON requests by number, either
  side may ask, and a cancellation names a request; MCP passes bytes as
  base64.

## Alternatives

- **Commands a program defines apart from plugins.** A second way to give
  the agent a command, with its own declarations, help, permissions and
  failures, beside the plugin host's.
- **Bytes as base64 in the JSON, as MCP.** A third more bytes, and each
  blob encoded and decoded whole on both sides.
- **A program's plugins declared with each core.** The registry is the
  process's, shared by every core, as on the server; a registry for each
  core would be a second shape of it.

## Open questions

- None.
