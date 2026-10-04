# Conversation permissions

Demi has no general permission system: the agent runs commands on its Host,
edits files and opens pages without asking, because the user chose the Host
and the working directory. It asks only before an agent acts on Demi itself,
on what the user owns beyond the conversation: today, managing the user's
skills; later, for example, reading another conversation. Each such power is
one broad **category**, and a user who allows a category allows it for one
conversation, for good.

## A request, end to end

For example, a user asks the agent to "set up Vercel's web design skill". The
root agent runs:

```sh
demi skills add vercel-labs/agent-skills --skill web-design-guidelines
```

1. `demi skills add` fetches the repository's default branch and pins its
   newest commit, which changes nothing yet
   ([Commands](skills.md#commands)). It then asks for the category `Manage
   skills` with what it is about to do: add the source at that commit and
   turn on one skill of 3 files, 1 of them executable.
2. This conversation has no grant of `Manage skills`, so the backend records a
   **permission request** and keeps the command's call waiting. The
   `shell_exec` call that runs the command does not return, and its
   observation window stops counting. The turn makes no provider request.
3. Every page of the user shows a yellow dot on the conversation's row in the
   sidebar. The conversation's page shows the card above its composer:

   ```text
     ... transcript: the shell_exec call, running ...

     +---------------------------------------------------------------+
     | Allow this conversation to manage skills?                     |
     |                                                               |
     | Add vercel-labs/agent-skills and turn on 1 of its 5 skills    |
     |   Repository  https://github.com/vercel-labs/agent-skills     |
     |   Commit      41ab07c2d9e5                                    |
     |   Files       3, 1 executable                                 |
     |   web-design-guidelines                                       |
     |     Review UI against Vercel's web interface guidelines.      |
     |                                                               |
     | Manage skills lets the agents of this conversation add,       |
     | update and remove skill sources and turn skills on or off.    |
     | Your skills reach every conversation, and the skills that are |
     | on are installed on every Host your conversations use.        |
     |                                                               |
     |                     [ Deny ]  [ Allow for this conversation ] |
     +---------------------------------------------------------------+
     [ 1 Running ]
     +---------------------------------------------------------------+
     | Ask Demi...                                                   |
     +---------------------------------------------------------------+
   ```

4. The user selects **Allow for this conversation**. The backend records the
   grant and answers the waiting call; the command adds the source, turns the
   skill on and exits 0. The window goes on with the time it had left, the
   call returns, and the dot and the card leave every page.
5. Later in the same conversation, a subagent runs `demi skills enable
   vercel-labs/agent-skills --skill react-best-practices`. The grant covers
   the whole conversation, so the command asks no one and runs.

Had the user selected **Deny**, the command would have exited 1 with
`demi: the user denied this conversation permission to manage skills`, and
nothing would have been recorded: the next `demi skills add` asks again.

## Categories

A category is one power over Demi, broad enough that a user can judge it once
for a conversation: `Manage skills` covers adding, updating and removing skill
sources and turning skills on or off, not each of these on its own. A plugin
declares its categories in its manifest
([What a plugin contributes](../architecture/plugins.md#what-a-plugin-contributes)):

| Field | Meaning | For `Manage skills` |
| --- | --- | --- |
| `name` | Unique within the plugin; the category's id is `<plugin>.<name>` | `manage`, so `skills.manage` |
| `action` | A lowercase verb phrase that completes "Allow this conversation to …"; with its first letter capitalized, it is the category's title | `manage skills` |
| `description` | What a grant allows, including its reach beyond the conversation, in one to three sentences | As on the card above |

A category is what the user decides on, so it never changes meaning while a
grant of it exists: a plugin that needs a broader power declares a new
category. The categories are fixed while the backend runs, as the rest of a
manifest is.

### An operation that needs a category

An operation needs a category in two places, one static and one at run time:

- **Its command leaf names the category** (`permission` in the
  [declaration](../execution/commands.md#declare-a-command)). The help of
  such a leaf says that the command may wait for the user the first time in a
  conversation, so the model is not surprised by the wait. Only an `rpc` leaf
  can name one, and only a category of its own plugin; registration refuses
  anything else. A `native` leaf never reaches the plugin, so it cannot ask.
- **Its handler asks, with the operation's details**, through the port's
  Permissions service ([The contract](../architecture/plugins.md#the-contract)),
  at the moment it is about to change something. It names the category its
  leaf declares and describes the concrete operation. The answer is `allowed`
  or a refusal with the message the command prints.

A handler asks only for an operation that would succeed and change something.
Everything that can be checked first is checked first, without asking: a
`demi skills add` of a repository that does not exist, or whose skills'
names are taken, fails without a card; an `enable` of a skill that is already
on, or an `update` of a source already at its newest commit, changes nothing
and succeeds without one. The user is asked about exactly what will happen.

The operation's details are data, which `web-ui` shows without knowing the
category:

| Field | Meaning | Bounds |
| --- | --- | --- |
| `summary` | One sentence: what the command will do | 1 to 200 characters |
| `fields` | Labeled facts, in order, such as the repository and the commit | At most 8; a label of 1 to 40 characters, a value of 1 to 500 |
| `items` | The things it acts on, such as the skills, each with a name and an optional detail | At most 100; a name of 1 to 64 characters, a detail of at most 1,024 |

Every value is plain text, shown as text and never as markup. The plugin host
validates the details against their schema when the handler asks, and a
handler that sends details out of bounds fails its command.

A user's own action on a settings page is not an agent's operation and asks
nothing: the page's methods change the user's state directly
([The page](../architecture/plugins.md#the-page)).

## Requests

A request is raised when an agent of a conversation, the root or any subagent
of its tree, asks for a category that the conversation has no grant of. It
holds:

| Field | Meaning |
| --- | --- |
| `id` | The request's id |
| `category` | The category's id, `action` and `description` |
| `operation` | The details the handler gave |
| `agent` | The agent that asked: none for the root, or the subagent's number and description |
| `createdAt` | When it was raised |
| `waiting` | Whether a command still waits for the answer; a fact of the running backend, not stored |

**Queue.** A conversation can hold several requests, of one category or of
several, such as two subagents that each ask. The card shows the oldest first,
with its place in the queue ("1 of 3"), and the next one once that is
decided.

**A decision.**

- **Allow** records the conversation's grant of the category and answers every
  request of that category in the conversation, which the grant now covers.
  Each command that waits for one of them goes on.
- **Deny** answers that one request. Its command, if it still waits, exits 1
  with `demi: the user denied this conversation permission to <action>`.
  Nothing is remembered, so the next attempt asks again, and another request
  of the same category in the queue stays.

**A request outlives its command.** A request ends only when the user decides
it, when a newer request of its category in the same conversation replaces
it, or when the conversation is archived. Whatever ends the command that
asked, the request stays with `waiting` false: the user's Stop, an abort, the
end of the wait's bound ([The wait](#the-wait)), a Host that stops, or a
backend restart. The question is about the conversation, not the command, so
a later Allow still records the grant, and the agent's next attempt runs
without asking. The card then says that the command no longer waits.

A new request replaces every request of its category in the same
conversation whose command no longer waits: it asks the same question about
what the agent is doing now. So an agent that tries again after its command
stopped waiting leaves one card, not two.

Requests never expire. They are stored and survive a backend restart
([Storage](../backend/storage.md#control-records)). Archiving a conversation
withdraws its requests, since nothing can run in it until it is restored;
Fork copies none ([Conversation Fork](conversation-fork.md)).

## Grants

A grant is one category allowed in one conversation. It covers every agent
of the conversation's tree, the root and every subagent, those spawned later
included, and every operation of the category, without asking again. It never
expires.

The user sees a conversation's grants, and revokes any of them, under
**Permissions** in the conversation's menu in the sidebar. A revocation takes
effect at the next operation that asks: an operation that was allowed before
the revocation finishes. A conversation without grants shows that Demi asks
when an agent first needs a permission.

Archiving keeps a conversation's grants, so a restored conversation has them.
Fork copies none: a forked conversation is a new one, and the user decides for
it again.

## The wait

A command that asks blocks until the user decides. Nothing in the agent
runtime knows about requests; the wait is an rpc call that takes long:

- **The tool call does not return.** The node's turn waits in its tool call,
  so it makes no provider request, and steers, agent messages and yield
  wakeups wait for the call to end, as they do for any tool
  ([Input](runtime.md#input)). Other agents of the tree go on.
- **The observation window stops.** While a call of the command waits for the
  user, the `shell_exec` window that watches the command does not count, and
  it resumes with the time it had left when the wait ends
  ([Running shell tools](runtime.md#running-shell-tools)). A command whose
  window already ended, one running in the background, waits all the same;
  its `shell_status` result says that it waits for the user's decision on the
  category, so the model does not take it for a hung command.
- **Stop cancels the wait.** The user's Stop of the action whose tool call
  waits stops the command as it stops any command
  ([Stop](runtime.md#stop)); the call ends as cancelled, with 130. An abort
  of the subagent, or a `shell_abort` of the command, ends it the same way.
  The request stays.
- **The wait is bounded by the idle window.** A call waits for a decision at
  most one [idle window](../execution/resource-lifecycle.md#idle-window), 1
  hour, and then exits 1 with `demi: the user has not decided whether this
  conversation may <action>; the request stays open, and the command runs
  once the user allows it`. The request stays. A waiting command is a running
  job, which is activity, so an unbounded wait would keep a Cloud running for
  as long as the user stays away
  ([Activity](../execution/resource-lifecycle.md#activity)).
- **A command that dies keeps its request.** A Host that stops or goes
  offline, or a backend restart, ends the call as it ends any call
  ([Command lifetime](../execution/runner.md#command-lifetime)); the request
  stays, and a later Allow still records the grant.

## What the user sees

**The card.** The conversation's page shows the oldest waiting request as a
card pinned above the composer, below the transcript and above the dock's
chips. It is not a modal: the user can read the transcript, write a steer or
queue a message while it is there. The card shows:

- its title, "Allow this conversation to `<action>`?", and "1 of N" when
  several requests wait;
- for a subagent's request, the subagent that asks, by its description;
- the operation: its summary, its fields and its items;
- the category's description;
- when `waiting` is false, that the command which asked no longer waits and
  that Allow still applies to the agent's next attempt;
- two buttons, **Deny** and **Allow for this conversation**.

A decision another page made, on another tab or device, removes the card on
every page. A decision that arrives after another page decided the request
changes nothing, and the page shows the outcome it receives.

**The needs-you mark.** While a conversation holds a request, its row in the
sidebar, and its status mark wherever the conversation shows one, shows a
yellow dot: the conversation needs the user. It takes precedence over every
other mark, the running one included, since the conversation is waiting for
the user, and shows whether the conversation is open, read or not. It is
distinct from the orange mark of a conversation that failed or was stopped.

**Permissions.** The conversation's menu in the sidebar opens Permissions: a
dialog that lists the conversation's grants, each with its title, its
description, when it was granted and **Revoke**.

`web-ui` owns the card, the mark and the dialog as components over a request's
and a grant's data; the gallery shows each state. `web` supplies the requests,
the grants and the decisions over the [Web API](../product/web-api.md#conversation-permissions).

## Responsibilities

| Part | Owns |
| --- | --- |
| `plugin-interface` | A category in the manifest, the operation's details as a type, the port's Permissions service and its answers |
| `command-declarations` | A leaf's `permission` and the help line it renders |
| A plugin, such as `plugin-skills` | Its categories, the leaves that name them, and the moment it asks with the operation's details |
| `backend-plugins` | The categories of the registered plugins and their checks; the Permissions service: grants, requests, decisions, revocation, the waiting calls and the wait's bound; it marks the call as waiting for the user through the call's rpc port |
| `backend-database` | The stored requests and grants |
| `backend-remote-host` | Stopping a command's observation window while a call of its job waits for the user, and the waiting hint of its `shell_status` result |
| `backend-user-shard`, `backend-http` | The summary's count of requests and the permissions revision; the routes |
| `web-ui`, `web`, `web-gallery` | The card, the needs-you mark and the Permissions dialog; their data over HTTP and the synchronization channel; the specimens |

## Acceptance

Tests use scripted providers and a real runner where a command runs; no test
calls a real model.

| Situation | Required observation |
| --- | --- |
| An agent runs `demi skills add` in a conversation without a grant | The call waits; the summary counts one request and every page receives it; the request holds the repository, the commit, the skills, the file and executable counts; the provider receives no request while the call waits |
| The user allows it | The grant is stored, the command exits 0, the request is gone and the summary counts none |
| A second `demi skills` change in the same conversation, by a subagent | It runs without a request |
| The same command in another conversation | It raises a request |
| The user denies it | The command exits 1 with the denial message; nothing is stored; the next attempt raises a new request |
| Two requests of `Manage skills` and one of another category wait; the user allows the first | Both of `Manage skills` are answered and their commands go on; the other stays |
| A `shell_exec` with a 30-second window whose command waits 10 minutes for Allow | The call returns after the command ends, not at 30 seconds; the time it waited does not count against the window |
| A background command waits | `shell_status` shows it running and waiting for the user's decision |
| Stop while the call waits | The call ends with 130; the request stays, marked as no longer waiting; a later Allow records the grant |
| The backend restarts while the call waits | After the start, the request is still listed, not waiting; Allow records the grant |
| A new request of a category whose earlier request no longer waits | The earlier one is gone; one card remains |
| A call waits for one idle window | It exits 1 with the undecided message; the request stays |
| A grant is revoked | The next operation of the category raises a request; an operation already allowed finishes |
| An archive of a conversation with a request that no longer waits | The request is gone; the grants stay after a restore |
| A Fork of a conversation with a grant | The fork has no grant and no request |
| Two pages decide one request at once | One decision applies; the other answers 404 `permission_request_not_found` |
| A handler asks for a category its leaf does not name, or sends details out of bounds | Its command fails; no request is raised |
| A plugin leaf names a category of another plugin | Registration refuses the plugins |
| Gallery | The card with one request, with a queue, for a subagent, and for a command that no longer waits; the needs-you mark; the Permissions dialog with grants and empty; every button acts on the specimen's state |
