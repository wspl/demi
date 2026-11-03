---
status: proposed
---

# An integration drives its conversations as the web app does

## Problem

A team's chat bot, built as an integration (0149), answers in a group chat.
A person in the group types `/model` to switch the group's conversation to
a faster model, then taps the progress card's **Stop subtask** button to stop
one subagent that went the wrong way, and later taps **Resume** after a
failure. The web app does each of these on a conversation; the integration
API does none: a conversation's model is fixed when it is made, `stop`
stops everything, and nothing resumes, retries, compacts or empties the
queue. The bot cannot list its conversations a page at a time, find one by
its key without making it, read a transcript before its latest page, or
change the workflow its conversation was made with, though a group's
conversation lives for months while its instructions change with each
release of the bot.

## Decision

Each operation the web app performs on a conversation, its agents and its
queue is a route of the integration API, served by the same shard operation
the web app's frame or route reaches (0149, two interfaces over one logic).
No operation is new: the program does what a person in the web app does.

### The model and the workflow

* `CreateConversation.model` and `PATCH /v1/conversations/{id} {model}` take
  `ModelChoice { providerId, modelId, effort?, serviceTier? }`. A change
  goes through the web app's settings change: from the main agent's next
  request, with the model's first effort when it names none, and the
  vendor's default tier when it names none. A subagent keeps the model its
  profile or its parent gave it at its spawn.
* `PATCH /v1/conversations/{id} {workflow}` replaces the conversation's
  workflow, YAML as at its creation, or `null` for none. The instructions
  source reads the workflow before each request, so the next request of
  every agent reads the new one. A workflow that does not read as a
  workflow file is refused, at the creation as at a change, with the
  parser's reason: an agent would otherwise run without it, told only in
  its instructions.
* A `PATCH` checks its workflow, its Cloud and its model before it changes
  anything, so a refused one leaves the conversation as it was; the move
  to another Cloud goes last.
* `GET /v1/models` answers each model's facts from the catalog the web app's
  model menu reads: its context window, output limit, efforts, whether
  thinking turns off, service tiers, and the file types it takes.

### The turn and the queue

| Route | The web app's frame |
| --- | --- |
| `POST …/stop` (today's), every agent | `AbortSubagents`, `Abort` |
| `POST …/stop {scope: "main"}` | `Abort` |
| `POST …/stop {scope: "subagents"}` | `AbortSubagents` |
| `POST …/agents/{agent}/stop` | `AbortSubagent` |
| `POST …/resume` | `Resume` |
| `POST …/retry` | `Retry` |
| `POST …/compact`, `POST …/agents/{agent}/compact` | `Compact` |
| `DELETE …/messages/{id}`: a queued message or a pending steer | `DequeueMessage`, `CancelPendingSteer` |
| `DELETE …/messages`: the whole queue | `ClearMessageQueue` |
| `POST …/messages/{id}/send-now` | `SendQueuedMessage`, `SteerNow` |
| `POST …/commands/{id}/stop` | `ShellAbort` |

`…` is `/v1/conversations/{conversation}`. A message's id is the program's
own (`SendMessage.id`), and names its queue entry or its pending steer, as
it names its turn.

### What the page hears that the state does not keep

The socket sends `retry_scheduled {conversation, attempt, delayMs, code,
diagnostics}` when the main agent retries a transient provider failure, the
web app's `RetryScheduled` frame, which the page hears too, so the program
can tell its users that the agent retries. It is not state: a program that
missed one reads the turn's outcome later (0221). The page hears no
subagent's retries, and neither does the program.

### The agents

`Agent` gains what the web app's Agents window shows of each agent
(`AgentEntry`): its phase, its queue and its pending steers, and of a
subagent its round: its number, profile and model, how it closed
(completed, failed or stopped), and when it started and ended.
`GET /v1/conversations/{id}` lists the closed subagents too, as the Agents
window does. A waiting message's id names it for the queue's routes: the
main agent's entry says whether it is queued or a pending steer, since a
session takes a frame for an id it does not hold without an answer.

`POST …/agents/{agent}/messages` sends a subagent a message, as the root
takes one from its program: a live subagent takes it into its turn as a
steer, an idle one starts a turn with it, and a closed one is resumed with
it as `demi agent resume` resumes one. It reads as a message of its user,
the block the root's messages are. It takes no id of the program's, since
a resume names no message. The web app's subagent panel stays
read-only: a person who wants a subagent to change course tells the main
agent, which a program cannot do without a model request for each message.

### The conversations and their transcripts

* `GET /v1/conversations` answers a page at a time, newest first by
  creation, with `limit` and the `after` cursor of the page before; `?key=`
  finds the one conversation of that key, or none, and makes nothing. The
  order is by creation, not by change, since a conversation that changes
  while the program pages would otherwise move between pages and be read
  twice or missed.
* `GET …/agents/{agent}/transcript` takes the web app's page query:
  `before`, `after`, `around` and `edge`, the latest page without them.
  Its blocks are light, as the page's are; `GET …/agents/{agent}/blocks/{block}`
  answers one whole, as a row the page opens.
* `POST …/fork {blockId}` makes a new conversation of the integration's
  from the transcript up to that block, as the web app's Fork does.

### The conversation's files

The Files panel's routes, on the conversation's Host and on each Host it
reaches (`…/fs`, `…/fs/raw`, `…/fs/search`, `…/hosts/{device}/fs`): list a
directory, read and write a file's bytes, make a directory, search. A
command request names its conversation, Cloud and directory, so a program
that serves `chatbot post --image a.png` reads `a.png` with these, where the
agent that ran it works.

## Spike

None: every route maps onto an operation the web app uses today.

## Prior art

* Codex's app server drives a thread through `turn/interrupt`,
  `thread/compact/start` and per-turn overrides of the model that carry over
  to later turns
  ([app server](https://developers.openai.com/codex/app-server.md)).
* OpenCode's server exposes `POST /session/:id/abort` and
  `POST /session/:id/summarize` over the same session logic its own
  interface uses ([server](https://opencode.ai/docs/server)).

Both expose the operations their own interface has, and no others; so does
this. Neither addresses a subagent, which Demi's Agents window does.

## Alternatives

* **A generic frame route**, `POST …/frames` taking the web app's
  `ClientFrame`. One route, but the web app's frames are its internal
  protocol and change with it; the API would leak them.
* **Messages to a subagent through the root**, whose agent passes them on
  with `demi agent send`. It costs a model request per message, and the
  root may not pass it on.

## Open questions

None.
