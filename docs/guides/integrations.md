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
- A Demi account with at least one model provider. An integration uses every
  provider of the account that creates it, so create a separate account for
  each product, with its own providers, rather than using your own.

## Create the integration

1. In Demi, open **Settings** > **Integrations** and click **Add
   Integration**.
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
   `GET /v1/conversations/{conversation}` returns, is `completed`, then read
   the main agent's transcript. The main agent's id is the conversation's id:

   ```sh
   curl -H "Authorization: Bearer $TOKEN" \
     $DEMI/v1/conversations/<conversation id>/agents/<conversation id>/transcript
   ```

   The reply is in the transcript's last `text` blocks.

A real program doesn't poll: it listens on the socket, as
[Hear what happens](#hear-what-happens) describes.

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

A conversation runs on one Cloud at a time. To move it, for example when
another person picks up a shared conversation, send
`PATCH /v1/conversations/{conversation}` with another `cloud`; the main
agent's next command runs there. A subagent stays on the Cloud it started on.

### Init script

The integration's init script prepares each Cloud the same way, for example
by installing tools your product needs. Each Cloud runs it as the user `demi`
in its home directory when its system is new: at its first use and after
every reset. Commands on that Cloud wait until the script ends. A Cloud whose
script fails still works, and `GET /v1/clouds/{cloud}` shows the exit code and
the end of the output.

When you change the script, each Cloud gets a new system at its next start
and runs the new script; a Cloud that is working keeps going until then.

### Shared folder

Every Cloud of the integration sees one shared folder, at `~/shared` unless
you choose another path in the settings. A file that one Cloud writes there,
every other Cloud reads at once, which is how a team's Clouds hand each other
files.

A program that watches the folder for changes, such as with `inotify`, sees
only the changes made on its own Cloud, not another Cloud's. An agent that
waits for another Cloud's file has to look for it.

## Conversations

A conversation's key is the thing of yours that it answers, such as a chat, a
thread or a ticket, written as any string up to 1,024 characters. Asking for
a conversation with a key that already has one returns that conversation, so
a retry or two requests at once never make two.

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

Send it as the conversation's `workflow` string. Instructions files in a
Cloud's projects and a user's personal instructions never apply to an
integration's conversations.

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

`POST /v1/conversations/{conversation}/stop` stops every agent of the
conversation.

## Hear what happens

Open a WebSocket to `/v1/socket` with the same bearer token. Each message is a
JSON event:

- `transcript`: an agent's transcript changed. Its `patches` apply to the
  blocks of the agent's transcript at the previous version and lead to
  `version`.
- `agent`: an agent started, ended, or changed its state or its Cloud.
- `conversation`: a conversation's status changed. A turn has ended when the
  status leaves `running` and `compacting`; `completed`, `stopped` or `error`
  says how. That's when a chat program usually posts the reply.

The socket sends only what happens while it's open. After you reconnect, read
the transcripts again and apply the patches that follow their versions.

## Give the agents your commands and context

Your program can give its conversations' agents commands of its own, such as
one that posts to the chat, and add context before each request to the model,
such as who is speaking in a group chat. You serve both over the socket.

Send a `declare` message with your command trees. Each tree is a group whose
commands are `rpc` leaves, and each leaf's `input` is a JSON Schema object:

```json
{
  "type": "declare",
  "context": true,
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
  `context: true`. Reply with `{"type": "context", "text": "..."}`, or with
  `null` text to add nothing. `seen` holds the context you added before.

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

While no socket serves your commands, an agent's command waits for one to
connect for a short while and then fails, telling the model that the
integration is unreachable. A context request doesn't wait: the agent asks
its model without your context.

## Usage

`GET /v1/usage?by=cloud`, `by=conversation` or `by=model` returns the
integration's requests, tokens and the cost they would have on the vendors'
APIs, one row for each group, most expensive first. `from` and `to` limit the
period.

## Delete

- `DELETE /v1/conversations/{conversation}` deletes a conversation.
- `DELETE /v1/clouds/{cloud}` deletes a Cloud and its files. Demi refuses it
  while a conversation is on that Cloud: delete or move the conversation
  first.
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
