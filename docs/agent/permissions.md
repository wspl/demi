# Conversation permissions

Demi has no general permission system: the agent runs commands on its Host,
edits files and opens pages without asking, because the user chose the Host
and the working directory. It asks only before an agent acts on Demi itself,
on what the user owns beyond the conversation: managing the user's skills,
organizing conversations and projects, reaching a host the conversation
could not reach before; later, for example, reading another conversation.
Each such power is one broad **category**, and a user who allows a category
allows it for one conversation ([Grants](#grants)); a host is allowed on its
own, one host at a time ([Hosts](#hosts)).

## A request, end to end

For example, a user asks the agent to "set up Vercel's web design skill". The
root agent runs:

```sh
demi skills add vercel-labs/agent-skills --skill web-design-guidelines
```

1. The command's leaf declares the category `skills.manage`, Manage Skills.
   This conversation has no grant of it, so the backend's dispatch records a
   **permission request** and holds the call before any handler runs: the
   command waits for the user's answer, as a command waits for its work.
   Nothing is fetched or changed while it waits. When the `shell` call's
   window passes, its result says the command still runs and why:

   ```text
   status: running
   commandId: 31
   waiting for the user's permission to manage skills; its end will report
   to you, so do not run it again
   ```

   The agent tells the user what it is waiting for and ends its turn.
2. Every page of the user shows a yellow dot on the conversation's row in the
   sidebar. The conversation's page shows the card above its composer:

   ```text
     ... transcript: the waiting command, the agent's answer ...

     +---------------------------------------------------------------+
     | Allow this conversation to manage skills?                     |
     |                                                               |
     | The agent ran                                                 |
     |   demi skills add vercel-labs/agent-skills \                  |
     |     --skill web-design-guidelines                             |
     |                                                               |
     | Manage Skills lets the agents of this conversation add,       |
     | update and remove skill sources and turn skills on or off.    |
     | Your skills reach every conversation, and the skills that are |
     | on are installed on every Host your conversations use.        |
     |                                                               |
     |                     [ Deny ]  [ Allow for This Conversation ] |
     +---------------------------------------------------------------+
     +---------------------------------------------------------------+
     | Ask Demi...                                                   |
     +---------------------------------------------------------------+
   ```

3. The user selects **Allow for This Conversation**. The backend records the
   grant, and the dot and the card leave every page. It delivers the
   decision's message to the agent that asked
   ([The decision's message](#the-decisions-message)), and the call that
   waited goes on: the dispatch passes it to `plugin-skills`, which fetches
   the repository, records the source and turns the skill on.
4. The command's end reports to the agent as any command's end does
   (`crates/agent-session/src/reports.rs`), so the agent has the
   result without running the command again.
5. Later in the same conversation, a subagent runs `demi skills enable
   vercel-labs/agent-skills --skill react-best-practices`. The grant covers
   the whole conversation, so the command runs without a request.

Had the user selected **Deny**, the waiting command would have ended with
exit status 1 and `demi skills add: the user denied this conversation
permission to manage skills; the command was not run`, reported as any
command's end is, the agent would have received the decision's message in
the same way, and nothing else would have been recorded: the next `demi
skills add` raises a new request.

## Categories

A category is one power over Demi, broad enough that a user can judge it once
for a conversation: Manage Skills covers adding, updating and removing skill
sources and turning skills on or off, not each of these on its own.

A category is declared with the commands that need it, in the
[command declarations](../execution/commands.md#declare-a-command), so a
plugin's group and a product's own group declare one the same way. A group
declares the categories its leaves use, and a leaf that needs one names it as
its `permission`:

| Field | Meaning | For Manage Skills |
| --- | --- | --- |
| `id` | The category's id, unique in the command set; it starts with its group's name | `skills.manage` |
| `action` | A lowercase verb phrase that completes "Allow this conversation to …"; in title case, as macOS labels are, it is the category's title, which its description also names it by | `manage skills`, titled Manage Skills |
| `description` | What a grant allows, including its reach beyond the conversation, in one to three sentences | As on the card above |

The command set's registration refuses a leaf whose `permission` names a
category that none of its groups declares, a category declared twice, and a
`permission` on a `native` leaf. A `native` leaf runs on the Host, where it
can act only on the Host and never on Demi itself, and its dispatch on the
runner has no grants to check. The help of a leaf with a `permission` says
that the command needs the user's permission in each conversation, and that
without it the command asks the user and waits for the answer.

A category is what the user decides on, so it never changes meaning while a
grant of it may exist: a group that needs a broader power declares a new
category. The categories the page shows are those of the command set the
user's conversations open with ([A user's
plugins](../architecture/plugins.md#a-users-plugins)); a request or grant of a
category no longer in it shows its id.

## The check

Every `rpc` call passes one check in the backend's dispatch, after the call is
bound to its job and its arguments are validated, and before its handler runs
([Dispatch](../execution/commands.md#dispatch-the-same-declaration-on-each-surface)).
The check is the same for a plugin's command, the product's `demi host` and
the agent runtime's `demi agent`; no handler takes part in it, and a handler
never sees a permission.

- **What the call needs**: its leaf's `permission`, and the host the call
  brings into the conversation when the conversation may not reach it
  ([Hosts](#hosts)). A call that needs nothing is dispatched.
- **A grant of each in the call's conversation**, whichever agent of the
  tree runs it: the call is dispatched.
- **A category without a grant**: the backend records one request for every
  category the call needs and the conversation lacks, and holds the call
  until the request is decided; nothing is dispatched meanwhile. The waiting
  command is a running command: the observation window, the turn, the
  Running window, Stop and the Host's idle rules are those of any running
  command. Its end, whichever way it comes, reports to the agent as any
  command's end does (`crates/agent-session/src/reports.rs`):
  - **Allowed**: the call is dispatched as if the grant had been there, and
    ends as the command ends.
  - **Denied**: the call ends with exit status 1 and `<command>: the user
    denied this conversation permission to <action>; the command was not
    run`, the actions joined with "and".
  - **Replaced** by a newer request ([Replacement](#requests)): the call ends
    with exit status 1 and `<command>: a newer run of this command asks the
    user instead; this one was not run`.

  A call that ends before the decision, stopped or lost with a restart,
  leaves its request as it is: the user can still decide it, and an Allow
  grants the category for the commands that come later.

  While the command waits, the `shell` call's result once its window passes
  says so beside its status, since a model that saw it merely running might
  run it again: `waiting for the user's permission to <action>; its end will
  report to you, so do not run it again`. A waiting command sends no report
  of its progress, whatever its interval: nothing about it changes until the
  user answers, and each report would wake the agent for nothing. Its
  interval counts again from the moment the wait ends, and its end
  reports as any command's does.

## Requests

A request records one refused call:

| Field | Meaning |
| --- | --- |
| `id` | The request's id |
| `categories` | The ids of the categories the call lacked, one or more |
| `command` | The command line as the agent ran it: the call's argv, quoted for a POSIX shell, such as `demi skills add vercel-labs/agent-skills --skill web-design-guidelines` |
| `agent` | The agent that ran it: the root, or a subagent's number and description |
| `createdAt` | When the call was refused |

**Queue.** A conversation can hold several requests, of one category or of
several, such as two subagents that each ran a command. The card shows the
oldest first, with its place in the queue ("1 of 3"), and the next one once
that is decided.

**Replacement.** A new request replaces an undecided request of the same
categories from the same agent: the agent ran the command again, and its newest
command line is the one to show. Requests of other agents stay.

**Allow** records the conversation's grant of each of the request's
categories and decides as allowed every request in the conversation whose
categories are now all granted: each agent that asked is told so. **Deny** decides that one request as denied and records nothing
else, so the next attempt raises a new request; another request of the
category stays in the queue.

Requests never expire. They are stored and survive a backend restart
([Storage](../backend/storage.md#control-records)). Archiving a conversation
withdraws its requests; Fork copies none
([Conversation Fork](conversation-fork.md)).

### The decision's message

A decision reaches the agent that asked as an `agent_message` (`crates/agent-session/src/input.rs`) from the user, admitted through the same
entry as a subagent's message, so it follows the same delivery rules: it
joins a running turn at its next continuation boundary, and otherwise wakes
the agent with a continuation (`crates/agent-session/src/input.rs`). The user sees it as the
receipt row ([What the user sees](#what-the-user-sees)). Its event is
`permission`, with the outcome `allowed` or `denied` and the actions of the
request's categories, joined with "and", and its content one of:

```text
The user allowed this conversation to <action>.
The user denied this conversation permission to <action>.
```

The rules of that entry hold with one exception: deciding is the user's own
action, so a decision wakes the agent that asked while it is idle even when the
user stopped its last turn, where a subagent's message would wait for the
user's next action. The message has no agent sender: its envelope names the
user as the sender and opens with a line that says it is the user's decision
on a permission request, not agent-originated context.

The message's id is `permission:<request id>`, so delivering it again
changes nothing. A decision on a conversation whose tree is closed opens the
tree, as an open does, to deliver it. A decided request stays stored until
the message is in the agent's checkpoint, and the backend delivers the
decided requests it still holds when it starts, so a decision made just
before a restart still reaches the agent. Archiving withdraws decided
requests whose message was not delivered with the others.

When the agent that asked is closed, such as a subagent whose call was
stopped and which ended its round, the message goes to its nearest live
ancestor, the root at last, which owns the work the subagent was doing. The
message then names the subagent that asked.

### Hosts

A host is allowed on its own: the list of hosts a conversation may reach
grows one host at a time, by the user's answer to the agent that asked
([Hosts a conversation may reach](../execution/sessions-and-targets.md#hosts-a-conversation-may-reach)).
A leaf whose argument names a host or a device declares that
argument as `bringsHost`; the check resolves it
once, by the name it has in
the conversation, a device's id, a paired device's name, then `Cloud`, and
needs that host when the conversation may not reach it. `demi host pair`
always brings a host the conversation may not reach, since the computer is
not yet paired, and names it by its pairing code: Allow lets the
conversation pair that code, so the `demi host pair` that waited for it is
dispatched and uses it, and the device it pairs joins the list; when that
call ended before the decision, the agent's next `demi host pair` with the
code uses it instead. Another code asks again. The request holds the host as `host`; no command
needs a category and a host at once.

A request for a host says *Allow this conversation to reach build-box?*, and
for a pairing *Allow this conversation to pair a computer and reach it?*;
its button reads **Allow**, since a host is allowed for this conversation by
nature, while a request of a category keeps **Allow for This
Conversation**. The decision's message names the action as the title does:
`reach build-box`, `pair a computer and reach it`.

## Grants

A grant is one category, or one host, allowed in one conversation. It covers every agent
of the conversation's tree, the root and every subagent, those spawned later
included, and every command of the category. It lasts for the
conversation's life: it never expires, and nothing revokes it. The product
shows no list of grants: the card asks once, and the receipt row in the
transcript records the answer. A user who wants to decide again works in
another conversation.

Archiving keeps a conversation's grants, so a restored conversation has them.
Fork copies no category: a forked conversation is a new one, and the user
decides for it again. It copies the source's attached hosts, which the user
forked along with the work they hold, and with them their place on the
reachable list; a host the source was allowed but no longer has attached is
not copied ([Conversation Fork](conversation-fork.md)).

## What the user sees

**The card.** The conversation's page shows the oldest undecided request as a
card pinned above the composer, below the transcript and above the dock's
chips. A panel the dock opened, such as the agents' or the running commands',
keeps its size and place: the card stands over its lower part, above it in
the stacking order, rather than the two sharing the height. It is not a modal: the user can read the transcript, write a message
or a steer while it is there. The card shows:

- its title, "Allow this conversation to `<action>`?", its categories'
  actions joined with "and", and "1 of N" when several requests wait;
- the command line the agent ran, as a code line;
- for a subagent's request, the subagent that ran it, by its description;
- each category's description, under its title when there are several;
- two buttons, **Deny** and **Allow for This Conversation**.

A decision another page made, on another tab or device, removes the card on
every page. A decision that arrives after another page decided the request
changes nothing, and the page shows the outcome it receives.

**The needs-you mark.** While a conversation holds an undecided request, its
row in the sidebar, and its status mark wherever the conversation shows one,
shows a yellow dot: the conversation needs the user. It takes precedence over
every other mark, the running one included, until the user reads the
conversation, as every mark of something new does: the sidebar then shows the
settled ring while the request still waits in the conversation and in the
Running window, and a new request lights the dot again
(`crates/backend-database/src/selection.rs`). It is distinct from the red mark of a
conversation that failed (`crates/backend-database/src/selection.rs`).

**The decision in the transcript.** The message an agent receives shows as a
receipt row, as an agent message does (`packages/web-ui/src/agent/blocks/AgentReceiptBlock.vue`): "You allowed this conversation to
manage skills"; it does not open, as no notice row does, since it says
all there is.

`web-ui` owns the card, the mark and the receipt row as components over a
request's and a message's data; the gallery shows each state. `web` supplies
the requests and the decisions over the
`crates/backend-http/src/permissions.rs`.

## Responsibilities

`backend-permissions` owns this document's mechanism: the check in the
dispatch, the requests and their replacement, the decisions and their
messages with the delivery after a restart, and the grants.
It is its own crate because every command source passes through it, plugins,
the product and the agent runtime alike, so it belongs to none of them: the
plugin host would make the product's commands go through plugins, and host
access and the runners keep no product records and know no user. It reaches
what it needs of the user's shard through `PermissionShard`, as the other
domains do ([Backend libraries](../architecture/crates-and-packages.md#backend-libraries)).

| Part | Owns |
| --- | --- |
| `command-declarations` | A group's categories and a leaf's `permission`, their registration checks and the help line |
| A command group, such as `plugin-skills`'s `demi skills` | Its categories and the leaves that name them; nothing at run time |
| `backend-permissions` | The check, requests, decisions and their messages, grants, and `PermissionShard` |
| `backend-database` | The stored requests and grants |
| `backend-user-shard` | Calling the check from the rpc dispatch; `PermissionShard`: the control service, the user's change marks, the user's command set, and the admission of a decision's message into the asking agent's session, opening its tree when it is closed; the summary's count of requests |
| `shared-types`, `agent-transcript` | The `permission` event of an agent message, sent by the user, and its envelope on replay |
| `agent-server` | The admission of the message into the asking node, or its nearest live ancestor |
| `backend-http` | The routes |
| `web-ui`, `web`, `web-gallery` | The card, the needs-you mark and the receipt row; their data over HTTP and the synchronization channel; the specimens |

## Acceptance

Tests use scripted providers and a real runner where a command runs; no test
calls a real model.

| Situation | Required observation |
| --- | --- |
| An agent runs `demi skills add` in a conversation without a grant | The command keeps running; the plugin receives no command request; the request holds the command line and the agent; the summary counts one request on every page; the `shell` result, once its window passes, carries the waiting line |
| The user allows it | The grant is stored, the request is gone, the summary counts none; the plugin runs the waiting call once; the allowed message, without any line saying the command can run, and the command's end both reach the agent |
| The decision arrives while the asking agent runs a tool | The message enters its transcript at the next continuation boundary, and the next provider request carries it |
| A second `demi skills` change in the same conversation, by a subagent | It runs without a request |
| The same command in another conversation | It raises a request |
| The user denies it | The command ends with exit status 1 and the denied line; the plugin receives nothing; the agent receives the denied message; no grant is stored; the next attempt raises a new request |
| `demi host move build-box /srv/app` to a host the conversation may not reach; later `demi host move Cloud /home/demi/app` back, `demi host attach build-box` | The first asks *reach build-box?*; once allowed, the later ones ask nothing |
| `demi host pair <code>` | Asks *pair a computer and reach it?*; once allowed, the same command pairs the computer and the device joins the list; a second computer's code asks again |
| `demi host devices` | Lists without asking |
| The agent runs the command twice before a decision | One request, with the newer command line; the first command ends with the replaced line; a subagent's request of the same category stays beside it |
| Two subagents' requests of Manage Skills and one of another category; the user allows the first | Both subagents' calls run and both receive the allowed message; the other request stays |
| The waiting command is stopped, then the user allows | The plugin receives nothing; the request stays until the Allow; the grant is stored and the allowed message reaches the agent |
| The user stops the command and allows it at the same moment | The plugin receives nothing: a stopped call is never dispatched |
| A subagent asked, its call was stopped and it closed before the decision | Its parent receives the message, naming the subagent |
| The backend restarts with a request undecided | After the start the request is listed; a decision then reaches the agent |
| The backend stops after a decision is stored and before its message is admitted | At the start, the message is delivered once |
| A product command with a `permission`, in a test command set | It passes the same check as a plugin's |
| A leaf names an undeclared category, or a `native` leaf names one | Registration refuses the command set |
| An archive of a conversation with a request | The request is gone; the grants stay after a restore |
| A Fork of a conversation with a grant | The fork has no grant and no request |
| Two pages decide one request at once | One decision applies; the other answers 404 `permission_request_not_found` |
| Gallery | The card with one request, with a queue and for a subagent; the needs-you mark; both receipt rows; every button acts on the specimen's state |
