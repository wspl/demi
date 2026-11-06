# Build on Demi with an integration

An integration is your program building an agent product on Demi through its
API. Demi runs the agents, their Clouds and their conversations; your program
decides who talks to which conversation and where the replies go. For
example:

- A personal assistant in a chat app, with one conversation per chat and one
  Cloud for its owner.
- A team bot, with one conversation per group chat or thread and one Cloud for
  each person, or one that the whole team shares.
- Your own product, with an agent in it that works on files in a Cloud for
  each customer.

Demi keeps no record of your users. Your program maps its own things, such as
a chat or a thread, to Demi's Clouds and conversations by keys of its choice,
and handles its own sign-in, channels and permissions.

Every route and body is described in the OpenAPI document that your Demi
server serves at `GET /v1/openapi.json`, with the errors each route answers.
The socket's messages are its `Event` and `ClientMessage` schemas. This guide
explains how the parts fit together and doesn't repeat the fields.

## What you need

- A Demi server that your program can reach. Every integration runs against
  a server, your own or a hosted one; Demi can't run inside your program.
- The server's master account, with at least one model provider. Only the
  master has integrations, and every integration of a server uses the
  master's providers and keeps its attachments among the master's: a product
  that needs its own, or its own apart from another's, runs a server of its
  own.

## Create the integration

1. Sign in to Demi as the server's master, the account that set the server
   up: only the master has integrations. Open **Settings** >
   **Integrations** and click **Add Integration**.
2. Copy the token. Demi shows it only once; **Replace Token…** makes a new one
   and stops the old one at once.
3. On the same page, turn off what the integration's agents shouldn't do:
   subagents, the browser, web search, skills, media, simulators and
   emulators, schedules, and attachments.

Every request sends the token as a bearer token:

```sh
export DEMI=https://demi.example.com
export TOKEN=...  # the token you copied
curl -H "Authorization: Bearer $TOKEN" $DEMI/v1/integration
```

## Quick start

The following steps start a conversation and read its reply.

1. Choose a model from the ones your integration may use:

   ```sh
   curl -H "Authorization: Bearer $TOKEN" $DEMI/v1/models
   ```

2. Create a Cloud. The key is yours; asking again with the same key returns
   the same Cloud:

   ```sh
   curl -X POST -H "Authorization: Bearer $TOKEN" \
     -H 'Content-Type: application/json' \
     $DEMI/v1/clouds -d '{"key": "user:alex"}'
   ```

3. Create a conversation on that Cloud, with a model from step 1:

   ```sh
   curl -X POST -H "Authorization: Bearer $TOKEN" \
     -H 'Content-Type: application/json' \
     $DEMI/v1/conversations -d '{
       "key": "chat:1042",
       "cloud": "<cloud id>",
       "model": {"providerId": "<provider id>", "modelId": "<model id>"}
     }'
   ```

4. Send a message:

   ```sh
   curl -X POST -H "Authorization: Bearer $TOKEN" \
     -H 'Content-Type: application/json' \
     $DEMI/v1/conversations/<conversation id>/messages \
     -d '{"text": "Count the lines of every CSV file in ~/reports."}'
   ```

5. Wait until the conversation's `status`, which
   `GET /v1/conversations/{conversation}` returns, is `completed`. Its
   `lastTurn.answer` is the reply. For everything the agent did, read the
   main agent's transcript; the main agent's id is the conversation's id:

   ```sh
   curl -H "Authorization: Bearer $TOKEN" \
     $DEMI/v1/conversations/<conversation id>/agents/<conversation id>/transcript
   ```

A real program doesn't poll: it listens on the socket, as
[Hear what happens](#hear-what-happens) describes.

A conversation's `title` is the start of its first message, until the user's
small model writes one, as in the web app. Turn off the `titles` capability
to keep the start of the message instead. Either way, you can give it a title
with `PATCH /v1/conversations/{conversation}` and a `title`.

## Clouds

A Cloud is a Linux machine that Demi runs and keeps: the agents run their
commands there, and its home directory stays between conversations. How many
you create is up to your product:

- One Cloud for each person keeps each person's files, sign-ins and history
  apart.
- One Cloud for a whole group lets everyone work on the same files.

A Cloud sleeps when nobody uses it and wakes at the next command, with its
files as they were. `POST /v1/clouds/{cloud}/reset` gives it a new system and
keeps its home directory.

### Size and sleep

A Cloud sleeps after an hour without work: no conversation's turn, no
command and no file transfer on it. A Cloud that watches something around the
clock should never sleep: create it with `"alwaysOn": true`, or
`PATCH /v1/clouds/{cloud}` with it; a `POST /v1/clouds` that finds a Cloud by
its key leaves the Cloud as it is. An always-on Cloud starts at once and runs its init
script. Once a day it's saved and started again, which ends what runs on it;
start your watch again from a command then. The socket's `cloud` events tell
you: the Cloud goes `saving`, `off`, `booting` and `running` again.

`PATCH /v1/integration` with `{"cloudIdleMinutes": 15}` makes every Cloud of
the integration sleep after 15 minutes without work instead, or `null` after
the server's hour. Each Cloud takes it at its next start.

You size the integration's Clouds in Settings › Integrations: their CPUs,
memory and disks, and the shared folder. `GET /v1/integration` shows them as
`cloudSizing`, where `null` is the server's size: the CPUs and memory of every
Cloud, and 16 GB for each disk and the shared folder. `"unlimited"` CPUs or
memory lets a Cloud use all the server has. Each Cloud takes a new size at its
next start. A disk only grows: a smaller size leaves a larger disk as it is. A
Cloud that asks for more CPUs or memory than the server has doesn't start: the
turn or the command that woke it fails with the reason.

A conversation runs on one Cloud at a time. To run a person's request on their
Cloud, name it on their message:

```json
{"text": "Make the weekly report.", "cloud": "<sam's Cloud id>"}
```

The turn that message starts runs on that Cloud: Demi moves the conversation
there before the turn begins, even when the message waited in the queue
behind someone else's turn. A message without a `cloud` runs where the
conversation works. A steer of a running turn can't name another Cloud
(`409 cloud_differs`): send it as a message instead, and it runs next. A turn
that a background command or a subagent's report starts runs where the
conversation works, and a subagent stays on the Cloud it started on.

To move a conversation at once, send
`PATCH /v1/conversations/{conversation}` with another `cloud`. A move while a
turn runs takes effect at once, as a move in the web app does: the turn's next
commands run on the new Cloud.

### Directory

A conversation's agents work in its Cloud's home, `~`, unless you name
another `directory` when you create it, such as the shared folder. The
conversation keeps its directory when it moves, and `PATCH` with a
`directory` changes it while the conversation isn't running.

### Init script

The integration's init script prepares each Cloud the same way, for example
by installing tools your product needs. Each Cloud runs it as the user `demi`
in its home directory when its system is new: at its first use and after
every reset. Commands on that Cloud wait until the script ends. A Cloud whose
script fails still works, and `GET /v1/clouds/{cloud}` shows the exit code and
the output.

A change of the script doesn't touch the Clouds you have: each runs the new
script only when you reset it with `POST /v1/clouds/{cloud}/reset`. New Clouds
run the new script.

### Shared folder

Every Cloud of the integration sees one shared folder, at `~/shared` unless
you choose another path in the settings. A file that one Cloud writes there,
every other Cloud reads at once, which is how a team's Clouds hand each other
files. A larger size takes effect when a Cloud starts while no other Cloud of
the integration runs, so while an always-on Cloud runs, the folder keeps its
size.

A program that watches the folder for changes, such as with `inotify`, sees
only the changes made on its own Cloud, not another Cloud's. An agent that
waits for another Cloud's file has to look for it.

### Commands and files outside a conversation

Your program can work on a Cloud without any conversation, for example to
update a tool every morning or to copy a person's saved files in when they
join. Send the script to run:

```json
POST /v1/clouds/{cloud}/commands
{"script": "npm install -g example-cli@latest", "cwd": "~/tools"}
```

Demi answers at once with the command's `id` and `state: "running"`. The
command runs as the user `demi`, where `sudo` works, in `cwd` (`~` when you
name none), after the Cloud wakes and its init script ends. A `stdin` in the
body is its standard input; without one, its input is empty. It has no time
limit, and it keeps the Cloud awake while it runs.

- `GET /v1/clouds/{cloud}/commands/{id}?after=` answers its `state` and its
  output from byte `after`. Pass the `next` of the previous read to read on.
- `state` becomes `exited` with an `exitCode`, or `failed` with a `failure`
  that says why it didn't run to its end, such as a stop.
- `POST /v1/clouds/{cloud}/commands/{id}/stop` stops it.
- The socket sends a `command` event when it ends.

Demi keeps every command, with its output, until you delete its Cloud. A
command outlives a restart of the server: it runs on, and once the Cloud's
runner connects again Demi follows it to its end, with its whole output.

The Cloud's files have the same routes as a conversation's
([Files](#files)), under `/v1/clouds/{cloud}/fs`. A relative path starts at
the Cloud's home, so `notes.txt` and `~/notes.txt` name the same file, but a
delete and a search's `root` take only an absolute path or one from `~`.
They wake a sleeping Cloud and don't wait for its init script.

## Conversations

A conversation's key is the thing of yours that it answers, such as a chat, a
thread or a ticket, written as any string up to 1,024 characters. Asking for
a conversation with a key that already has one returns that conversation, so
a retry or two requests at once never make two.

`GET /v1/conversations?key=<key>` finds the conversation of a key without
making one. Without `key`, it lists the conversations newest first, 100 at a
time, or `limit` at a time; pass the answer's `next` as `after` for the next
page.

### Model

The conversation's `model` names one of `GET /v1/models`, with its `effort`
and `serviceTier` if you want other than the model's first effort and the
vendor's default tier. Each model of `GET /v1/models` lists its efforts,
service tiers, context window, output limit and the file types it reads.

`PATCH /v1/conversations/{conversation}` with a `model` changes it from the
main agent's next request. A subagent keeps the model it started with.

### Instructions and subagents

You pass a workflow when you create the conversation, in the format of a
Demi workflow file. Its `instructions` tell the main agent about your product
and its conventions, and its profiles are the subagents it may start, while
the integration has subagents on:

```yaml
name: Support
instructions: |
  You answer the support chat of an online shop. Reply in the customer's
  language, in short paragraphs.
profiles:
  - id: research
    description: "Use to look up an order or a product in depth."
    model: gpt-5-mini
```

A profile's `model` runs on the first provider entry that offers it. When
more than one offers it, `provider` names the entry, as the `provider` of
each model of `GET /v1/models` names it:

```yaml
name: Support
profiles:
  - id: research
    description: "Use to look up an order or a product in depth."
    model: gpt-5-mini
    provider: Company Gateway
```

Send it as the conversation's `workflow` string. Instructions files in a
Cloud's projects, a user's personal instructions and a user's own subagent
profiles never apply to an integration's conversations: their agents start
only the workflow's profiles, whatever its `user_profiles` says.

When your product's instructions change, `PATCH` each conversation with the
new `workflow`, or `null` for none. Every agent reads it from its next
request. A `workflow` that isn't a valid workflow file is refused with `400`
and the reason.

### Identity

The agents' system prompt opens with a sentence that says what they are:
Demi's own, "You are Demi, an agent that works for the user on their
computers…", unless you give the conversation an `identity` of your
product's:

```json
{"cloud": "<Cloud id>", "model": {…}, "identity": "You are the Acme team's assistant in its group chat."}
```

Demi's rules of where commands run and how to wait follow it, then the
commands. A subagent opens with the same sentence, and its profile's
instructions follow. Put the identity here and your product's conventions in
the workflow's `instructions`.

A `PATCH` of `identity`, or `null` for Demi's, reaches every agent of the
conversation from its next request. That request starts the model vendor's
prompt cache over, so change the identity when what the agents are changes,
and keep wording you change often in the workflow's `instructions`.

### Messages

A message sent while a turn runs waits until the turn ends. With
`"steer": true`, the running turn reads it at once instead. Give each message
an `id` and repeat it when you retry, so a message is never sent twice.

To send files, upload each with `POST /v1/attachments?name=<file name>`,
with the bytes as the body and their media type as `Content-Type`, and list
the returned ids in the message's `attachments`. The agent finds them on its
Cloud.

When an agent attaches a file to its reply, the transcript names it, such as
`a3`. Download it with
`GET /v1/conversations/{conversation}/attachments/{attachment}`.

`GET /v1/conversations/{conversation}` lists the waiting messages: each
agent's `queue`, and its `pendingSteers`, which the running turn has not read
yet. With a message's `id`:

- `DELETE /v1/conversations/{conversation}/messages/{id}` withdraws it.
- `POST /v1/conversations/{conversation}/messages/{id}/send-now` sends it
  now. A queued message ends the running turn at once and runs next. A steer
  cuts the agent's current step short, and the turn goes on with it.

`DELETE /v1/conversations/{conversation}/messages` empties the queue.

### Control a turn

These do what the same controls do in the Demi app. Each path starts with
`/v1/conversations/{conversation}`:

| Request | What it does |
| --- | --- |
| `POST …/stop` | Stops every agent. With `{"scope": "main"}`, only the main agent; with `{"scope": "subagents"}`, only the subagents. |
| `POST …/agents/{agent}/stop` | Stops one of the main agent's subagents. |
| `POST …/resume` | Resumes a turn that stopped or failed. |
| `POST …/retry` | Retries the request that failed. |
| `POST …/compact`, `POST …/agents/{agent}/compact` | Compacts an agent's context. |
| `POST …/commands/{command}/stop` | Stops a command an agent runs. |
| `POST …/fork` | Makes a new conversation with the transcript up to `blockId`, a reply of the main agent's, and the source's workflow, identity and commands. Give it a `key` as you would a new conversation. |

`POST …/agents/{agent}/messages` sends a subagent a message, as if from its
user: a working subagent reads it after its current step, an idle one starts
working on it, and a subagent that ended resumes with it. Give each message
an `id` and repeat it when you retry: a subagent that already has the message
takes nothing, and one that ended stays ended. An `id` belongs to one agent,
so the same `id` sent to two subagents is two messages.

### Transcripts

`GET …/agents/{agent}/transcript` returns the latest page of an agent's
transcript. Its blocks are short forms; read one in full with
`GET …/agents/{agent}/blocks/{block}`. For older blocks, pass `before` with
the page's `start`, and the block at `start` as `edge`; `atStart` says when
there's nothing before. A `409` means the transcript changed since: read the
latest page again.

A command's block holds only the part of its output the agent saw. Read the
whole output with `GET …/commands/{command}/output?after=`, where `command`
is the `commandId` of the block's view, the number `demi shell` takes:

- `output` is the standard output and error from byte `after`, as text.
  Pass the `next` of the previous read to read on.
- `end` is null while the command runs, and says how it ended after.
- A Host keeps at most a command's first and last 8 MiB; a line in `output`
  says how many bytes it left out between them. A binary standard output
  reads as one line that gives its size.

### Files

The conversation's files are on its Cloud. With the paths these take, you
read what an agent made and put files where it works. A path from `~` is
under the Host's home, as in a shell:

| Request | What it does |
| --- | --- |
| `GET …/fs?path=` | Lists a directory, the conversation's working directory without `path`. |
| `POST …/fs` `{"path": …}` | Makes a directory, with its parents. |
| `DELETE …/fs?path=` | Deletes a file, or a directory with what it holds. |
| `GET …/fs/raw?path=` | Downloads a file. `Range` reads part of it. |
| `PUT …/fs/raw?path=&replace=true` | Uploads a file. Without `replace=true`, an existing file stays and the answer is `409`. |
| `POST …/fs/search` | Searches the names and the text of files, one JSON object per line. |
| `GET …/hosts/{device}/fs`, `POST …/hosts/{device}/fs` | The same on another Host the conversation reaches. |

### Show a browser tab

When a site asks for what the agent must not know, such as a password, show
the person the agent's tab and let them type it. Ask for a link to the tab
the agent's `demi browser` commands name, such as `t3`:

```sh
curl -X POST "$DEMI/v1/conversations/$CONVERSATION/browser/tabs/t3/views" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{"expiresIn": 900, "readOnly": false}'
```

The answer's `url` is a page with the tab and nothing else. Open it, or show
it in an `iframe` of your product; any page may frame it. Only this answer
carries the URL, so pass it on at once. Anyone who has it sees the tab until
the link expires, `expiresIn` seconds later, or you remove it with
`DELETE …/browser/views/{id}`, which ends each page open on it.
`GET …/browser/views` lists the links that haven't expired.

With `readOnly`, the person only watches: no click, key or paste of theirs
reaches the page. The page has no address bar or other tabs, and Back, Forward
and Reload aren't offered.

The page tells the window that frames it how it stands, with `postMessage`:

```js
addEventListener('message', (event) => {
  if (event.data?.type !== 'demi.view') return
  if (event.data.state === 'live') { /* the tab shows */ }
  if (event.data.state === 'ended') { /* event.data.reason: expired, removed, not_found or tab_closed */ }
})
```

### Share sign-ins between conversations

Each conversation's browser has a profile of its own, so a person who signed
in to a site in one conversation is signed out in the next. To keep one
person's sign-ins across conversations, name a browser profile when you
create each of them, or `PATCH` it later:

```json
{"cloud": "<Cloud id>", "model": {…}, "browserProfile": "alex"}
```

Every conversation that names `alex` on the same Cloud shares one browser:
what one signs in to, the others are signed in to. Each still sees and
drives only its own tabs, and a popup belongs to the conversation whose tab
opened it. `null` returns a conversation to a profile of its own. A
conversation that changes its profile closes its tabs on the old one at its
next browser command.

A profile is the directory `~/.demi/browser-profiles/alex` on the Cloud. It
outlives every conversation, the Cloud's sleep and its reset, and Demi
keeps no record of it: list, copy or remove profiles with the Files routes.
One removed while its browser runs stays signed in until that browser stops.
A conversation that moves to another Cloud uses that Cloud's profile of the
name.

## Hear what happens

Open a WebSocket to `/v1/socket` with the same bearer token. Each message is a
JSON event:

- `transcript`: an agent's transcript changed. Its `patches` apply to the
  blocks of the agent's transcript at the previous version and lead to
  `version`.
- `agent`: an agent started, ended, or changed its state or its Cloud.
- `conversation`: a conversation's status, its latest turn or its title
  changed. It carries the whole conversation, as
  `GET /v1/conversations` lists it, and may repeat what the previous one
  said.
- `retry_scheduled`: the main agent's request failed for a reason that
  passes, and it retries after `delayMs`. Tell your users it's still working.
- `cloud`: one of your Clouds changed state, such as
  `{"type": "cloud", "cloud": "…", "state": "booting"}`. Each change comes,
  in order. `state` is what `GET /v1/clouds/{cloud}` says.
- `command`: a command your program ran on a Cloud ended
  ([Commands and files outside a conversation](#commands-and-files-outside-a-conversation)).
- `behind`: your program read too slowly, and events after the ones it read
  were dropped. Reconcile, as below. Your commands' requests are never
  dropped.

### Turns

A conversation's `lastTurn` is the main agent's latest turn that ended:

- `id`: the `id` of the message that started it. A turn that a schedule, a
  wakeup or another agent's message started has Demi's own id, and `cause`
  says which.
- `outcome`: `finished`, `failed` or `stopped`.
- `endedBy`: the block that ended it. A turn that `resume` continues ends
  again, with another block.
- `answer`: the Markdown of its last text, which a chat program posts.

While `status` is `running`, more is coming, even after a turn ended: a
subagent still works, a command still runs, or a report waits for the main
agent, and each of them starts another turn. `completed`, `error` and
`stopped` mean nothing will start by itself.

### Reconcile

The socket sends only what happens while it's open, so trust the state, not
the events. After you connect, and after `behind`, list your conversations
with `GET /v1/conversations` and compare each one's `lastTurn` and
`revision` with what you handled: a turn that ended while you were away is
there. Read a transcript you follow again from the version you hold, and
your Clouds' states with `GET /v1/clouds`.

## Give the agents your commands and context

Your program can give its conversations' agents commands of its own, such as
one that posts to the chat, and add context before each request to the model,
such as who is speaking in a group chat. You serve both over the socket.

Send a `declare` message with your command trees. Each tree is a group whose
commands are `rpc` leaves, and each leaf's `input` is a JSON Schema object:

```json
{
  "type": "declare",
  "context": "optional",
  "commands": [{
    "name": "chatbot",
    "summary": "Post to the chat this conversation answers.",
    "indexEntry": "Posts messages and files to the chat this conversation answers.",
    "subcommands": [{
      "name": "post",
      "summary": "Post a message to the chat.",
      "kind": "rpc",
      "input": {
        "type": "object",
        "properties": {"text": {"type": "string", "description": "The message."}},
        "required": ["text"],
        "additionalProperties": false
      }
    }]
  }]
}
```

The socket answers with a `declared` event. From then on, every agent of the
integration can run `chatbot post --text "..."`, and Demi keeps the
declaration while your program is away. Only the socket that declared last
receives requests.

A conversation can have only some of the trees: name their roots as its
`commands` when you create it, such as `["memory"]` for a nightly
conversation that only tidies notes, or `[]` for none. Without `commands`,
it has every tree. A root the declaration doesn't have is left out, and
returns when you declare it again. `PATCH` with other `commands`, or `null`
for every tree, gives them to the agents' next command and lists them in
their system prompt from their next request. To try a new version of a tree
in one conversation, declare it beside the current one under a root of its
own, such as `chatbot-next`, and `PATCH` that conversation's `commands` to
it; `PATCH` back to end the trial. The conversation keeps its history, its
Cloud and the shared folder.

The agents' system prompt lists each command tree with its `indexEntry`, as
it lists Demi's own commands, and an agent reads the rest with `--help` when
it uses the tree. Write the entry the way Demi's are written: what the commands
do, in the words of the task, naming the main operations, such as "Posts to
the chat this conversation answers: post a message, send a file."

Requests arrive as `request` events:

- A `command` request carries the conversation, its key, the Cloud, the
  command's path and its arguments. Write its output with `port` messages,
  and end it with a `reply` of `{"type": "exit", "code": 0}`. A `cancel`
  event means the agent stopped the command, and Demi doesn't read the reply.
- A `context` request comes before an agent asks its model, when you declared
  `context` as `optional` or `required`. Reply with
  `{"type": "context", "text": "..."}`, or with `null` text to add nothing.
  `seen` holds the context you added before.

Each request also names the agent, the `turn` it works in, and its
`messageId`: the id of your message that the work descends from. When alex asks the bot to
deploy, the main agent's command carries the id of alex's message. So does
the command of the subagent it spawns for the build, and the command it runs
again when a background command ends, even after sam has written something
else. Look up who sent that message to decide what the command may do, or
whom the context should name. `messageId` is null for work that descends
from none of your messages, such as a schedule's.

The following TypeScript program, for Bun or Node.js, serves the command
above:

```ts
const socket = new WebSocket(`${process.env.DEMI_WS}/v1/socket`, {
  headers: {Authorization: `Bearer ${process.env.TOKEN}`},
});

socket.onopen = () => socket.send(JSON.stringify(declaration));

socket.onmessage = async (message) => {
  const event = JSON.parse(String(message.data));
  if (event.type !== 'request') return;
  const {id, request} = event;
  if (request.type === 'context') {
    const text = `${await speakerOf(request.key)} wrote the last message.`;
    socket.send(JSON.stringify({type: 'reply', id, reply: {type: 'context', text}}));
  } else if (request.type === 'command') {
    await postToChat(request.key, request.args.text);
    const bytes = Buffer.from('Posted.\n').toString('base64');
    socket.send(JSON.stringify({type: 'port', id, port: 1, request: {type: 'stdout', bytes}}));
    socket.send(JSON.stringify({type: 'reply', id, reply: {type: 'exit', code: 0}}));
  }
};
```

`declaration` is the `declare` message above, and `speakerOf` and `postToChat`
are your program's own. A browser's `WebSocket` can't send the header, so run
this in a server program.

While no socket serves your commands, an agent's new command waits for one
to connect for a short while and then fails, telling the model that the
integration is unreachable.

A command already sent outlives your program's restart. When its socket
closes, the command waits, and the socket that serves your commands next
receives its `request` again, under the same `id` and with
`"resent": true`: another socket that has sent `declare` already, or else
the next one to send it. Output the command wrote before reaches the agent
once. Input your program read is not read again, and input it asked for but
never received answers its next `read_stdin`. Make your work idempotent by
the `id`, which no other request takes, even after Demi restarts: a resent
request is the same call, which your program may have done already, such
as a card it posted before it restarted. Such a command waits until your
program replies or the agent stops it.

An `optional` context request waits for your answer for 10 seconds, or the
`contextWaitSeconds` your `declare` message names: then the agent asks its
model without your context. A `required` one waits for your program to
connect as a command does, and as long for its answer; then the agent's turn
ends with an error, which `lastTurn` shows.

## Usage

`GET /v1/usage?by=cloud`, `by=conversation`, `by=model`, `by=turn` or
`by=message` returns the integration's requests, tokens and the cost they
would have on the vendors' APIs, one row for each group, most expensive
first. `by=message` groups the work by the message it descends from, as
requests name it, so you can count it to the person who sent it. `from` and
`to` limit the period.

## Delete

- `DELETE /v1/conversations/{conversation}` deletes a conversation.
- `DELETE /v1/clouds/{cloud}` deletes a Cloud and its files, even while a
  conversation works on it. Such a conversation stays, with no Cloud until a
  message or `PATCH` names another.
- **Delete Integration…** in its settings deletes the integration with all its
  conversations, Clouds and shared folder, and stops its token at once. Its
  usage stays.

## Limits

- Agents of an integration never use Demi's saved credentials, notifications,
  other Hosts or other conversations. Permission requests are always allowed,
  because no person answers them.
- An integration can't add tools to the model, change how an agent works, or
  bring its own provider code. You shape the agents through the workflow, your
  commands and your context.
- The integration's conversations don't appear in the Demi app.
- The API can still change in any release.
