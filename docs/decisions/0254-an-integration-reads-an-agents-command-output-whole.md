---
status: proposed
---

# An integration reads an agent's command output whole

## Problem

An agent in a conversation of an integration runs `npm test`, which prints
40,000 lines and fails. The program that serves the conversation wants to
show its operator why: it reads the agent's transcript (0220), and the
command's block holds only the view the model saw, its first and last lines
and a note that the rest was cut. The agent itself reads the rest with
`demi shell output 4`, and the Host keeps it, all of it or its first and last
8 MiB, but no route of the integration API reaches it. The operator sees a
failure without its cause, and a program that traces its agents' work keeps
only what the model kept.

## Decision

`GET /v1/conversations/{conversation}/commands/{command}/output?after=`
answers what `demi shell output` reads of the command, as the Cloud command
route answers a command's output (0225):

```text
GET /v1/conversations/1c…/commands/4/output?after=0

200 { "id": "4",
      "end": { "kind": "exited", "exitCode": 1 },
      "output": "> app@1.0.0 test\n> jest\n…",
      "next": 2384117 }
```

* `command` is the id the command's block shows (its view's `commandId`),
  the one `demi shell` takes: of any agent of the conversation.
* `end` is how it ended, the block's own `CommandEnd`; null while it runs.
* `output` is its standard output and error in the order they came, as
  text from the byte `after` (0 by default). As `demi shell output` shows
  them, a binary standard output reads as one line that gives its size,
  a stretch its Host did not keep as a line that says how many bytes it
  left out, and output lost with its Host's connection as a line that
  says so. `next` is the byte the next read starts `after`, so a program
  reads a running command on as it prints, and stops once `end` is set and
  `next` stays.
* A running command is read from its Host, as `demi shell output` reads it;
  an ended one from the conversation's record of its output, which the
  backend keeps for as long as the conversation, opened or not.
* The answer is the whole output after `after`, up to what its Host keeps;
  the program pages by asking again from `next`.
* An unknown command is `404 not_found`. A running command whose Host does
  not answer, and an ended one whose output was not stored, are refusals
  that say why.

Both routes turn kept output into text with one function, which the Cloud
command route's moves into, so a Cloud command's binary output and its
lost end read as these do. The search for the command, on its Host
while it runs and in the store after, is `demi shell output`'s, which the
agent server answers to the backend as it answers a history's command page.

No event joins the socket: a command's end already reaches the program in
the transcript's events, as its block's change or as the report of its end
that enters the transcript.

## Spike

None: the reading is `demi shell output`'s, and the answer's shape the
Cloud command route's.

## Prior art

* Codex's app-server gives a command's whole output on its
  `commandExecution` item once it completes (`aggregatedOutput`), and
  streams it while it runs
  ([Codex SDK](https://hexdocs.pm/codex_sdk/Codex.Items.CommandExecution.html)).
  Here a program reads by offset instead, since the state is what it
  reconciles from (0221).
* OpenCode keeps a truncated tool output whole in a file and gives the
  agent its path
  ([opencode#9693](https://github.com/anomalyco/opencode/pull/9693)); the
  whole output is the Host's here, and the route reads it.

## Alternatives

* The whole output inside the block that `blocks/{block}` answers: a block
  would carry up to 16 MiB to every page and every event that holds it.
* A file the agent writes for the program: the agent would spend a turn on
  what its Host keeps already.

## Open questions

None.
