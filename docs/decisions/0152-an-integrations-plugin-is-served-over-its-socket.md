---
status: proposed
---

# An integration's plugin is served over its socket

## Problem

A group chat's bot built as an integration (0149) needs commands of its
own: its agent posts a message to the chat, or opens a ticket in the team's
tracker, and before each turn it is told who is speaking. Demi's plugin
contract already describes such commands and context, but the plugin host
runs only the plugins linked into the backend (`crates/plugin-host/src/lib.rs`).
An integration, written in any language and running on its own servers,
cannot be one.

## Decision

An integration serves commands and context to its conversations over its
WebSocket (`/v1/socket`), with the types Demi's own commands already use.

* A connection that sends `declare {commands, context}` serves the
  integration from then on, and Demi answers `declared`, with the reason when
  it refused. `commands` are command trees in the format of a command
  package's declarations, each at a root of its own such as `chatbot post`,
  whose every leaf is an `rpc` leaf; a root Demi's commands take is refused.
  `context: true` makes the integration a context source.
* Demi keeps the last declaration, so the commands stay in the conversations'
  shell while the program is away and come back with no work when it
  returns. Open conversations take a new declaration at once.
* Demi sends each request to the connection that declared last, with an id
  of its own; only the integration's connections answer it:

  | Demi sends | The integration answers |
  | --- | --- |
  | `request {id, request}`: a command's call, or a context request before an agent asks its model | `reply {id, reply}`: the command's exit code, the context block or none, or why it failed |
  | `answer {id, port, response}` | `port {id, port, request}`: the command's standard output and error, a read of its standard input, or whether it waits for a person's answer |
  | `cancel {id}`: the agent stopped the call | nothing |

  A command's call carries the conversation and its key, the agent's
  number, the Cloud and directory it runs in, the command line, and the
  arguments checked against the command's input. A context request carries
  the conversation, its key, the agent, and the blocks the agent already
  has. Both are the API's own types, not the backend's internal call, so
  the backend's internals can change under them.
* A call while no connection serves the integration waits for one as long as
  the backend waits for a runner that dropped (0011), and then fails, telling
  the model the integration is unreachable. A connection that closes fails
  its calls. A context request does not wait: without a connection, or
  without a reply in 10 seconds, the agent asks its model without the block.
* The OpenAPI document (`/v1/openapi.json`) holds the messages as JSON
  Schema, generated from their types, from which an integration generates its
  own types.

**The design.** The declared trees join the integration's conversations'
command set as served trees, whose handler sends the call to the
integration's connection and relays its port both ways. The context source is
one more source of the conversation's. Neither is a plugin of the plugin
host's, whose plugins are linked into the backend and live as long as it
does.

## Spike

None.

## Prior art

* [Codex app-server](https://github.com/openai/codex/tree/main/codex-rs/app-server):
  the server sends requests to its client over the same connection, for
  approvals, and waits for the client's answer.
* The [Model Context Protocol](https://modelcontextprotocol.io/) serves a
  program's tools to an agent over a connection; Demi keeps one tool, the
  shell, so an integration's tools are commands in it instead.

## Alternatives

* **Webhooks for each command:** a call's output streams and its input is
  read while it runs, which a single request and answer cannot carry; and
  an integration behind a firewall would need a public address.
* **A program on the integration's Clouds:** it would need the
  integration's secrets in every person's sandbox.
* **A remote plugin of the plugin host's:** the contract's pages, streams,
  OAuth and project search serve Demi's pages, which an integration has none
  of; and a plugin is the user's, not one integration's conversations'.
* **Reading files on the call's Host through the port:** an integration
  reaches its Clouds through its conversations, and a command that needs a
  file takes it on its standard input.

## Open questions

None.
