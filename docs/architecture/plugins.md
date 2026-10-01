# Plugins

A plugin is how a capability joins Demi. It can add commands to the agent's
shell, text the model reads, files every Host of its user's conversations
holds, and a part of the web app with the calls behind it. The coding agent's
own capabilities are plugins: the todo list (`plugin-todo`), the file commands
(`plugin-file`), the conversation browser's commands (`plugin-browser`) and
[skills](../agent/skills.md) (`plugin-skills`). The agent runtime, the runner
and the backend's conversation lifecycle know none of them.

Every plugin, built in or not, talks to Demi through one contract, and every
message of that contract is data. Demi's plugins are Rust crates the backend
links and calls directly. A plugin written with a TypeScript SDK runs as a
process of its own and exchanges the same messages over a wire. Demi has no
such SDK yet; the contract keeps one possible
([The TypeScript boundary](contracts.md#the-typescript-boundary)), and
[Decisions for the TypeScript SDK](#decisions-for-the-typescript-sdk) lists
what is left to decide for it.

MCP is not part of this design. It will join as a plugin of its own, designed
when it is built.

## A plugin, end to end

For example, a user turned on the skill `tdd` in the web app and opens a new
conversation:

```text
backend start      the composition root registers plugin-skills with the others;
                   the plugin host reads its manifest: a system-prompt text and a
                   context source, no commands

web app            POST /api/plugins/skills/calls/set_enabled { source, skill, enabled }
  -> plugin host   a page call: plugin-skills validates it, writes its stored
                   value, sets the Host directories its user's jobs need, and
                   marks its part of the product state as changed
  -> sync channel  every page of the user receives { type: "plugin", plugin:
                   "skills", state } and shows tdd as on

first request      the session asks each context source for news; plugin-skills
                   answers the list of enabled skills with the path of each
                   SKILL.md; the session appends it as a context block
  -> model         reads it, runs `cat ~/.demi/plugins/skills/<digest>/SKILL.md`
                   through shell_exec
  -> host access   before that job runs, installs the directory on the Host
                   once per runner connection
```

Nothing in the agent runtime is about skills. The session asked its context
sources, and the shell job's admission installed what the user's plugins
declared.

## What a plugin contributes

A plugin declares its contributions once, in its **manifest**, when the
backend starts. What a manifest declares is fixed for the life of the
process. What varies by user, conversation or time arrives through requests.

| Contribution | Declared | When Demi asks |
| --- | --- | --- |
| [Commands](#commands) | Their declarations, as data | Each time the model runs an `rpc` leaf of them |
| [System-prompt text](#prompt-text-and-context) | Its text | Never: the text is fixed |
| [Context](#prompt-text-and-context) | That the plugin is a context source | Before each provider request of every node |
| [Profiles](#profiles) | The profiles, as data | Never: they are fixed |
| [Host directories](#host-directories) | Nothing | The plugin sets them through its port when its user's needs change |
| [Page state and page calls](#the-page) | The state's schema, and each method with its parameter and result schemas | When a page reads the product state; when a page calls a method |

A plugin declares only what it uses: `plugin-file` declares commands and
nothing else, and `plugin-skills` declares no command.

### Commands

A plugin's commands follow the [command contract](../execution/commands.md):
groups and leaves with help and argument schemas, each leaf bound to an `rpc`
handler or to a `native` operation of a command package. A `native` leaf runs
on the Host and never reaches the plugin; an `rpc` leaf reaches the plugin as
a command request.

The plugin host composes the command set every node starts from:

```text
demi                     the plugin host's root
  file                   plugin-file (native: demi.file)
  todo                   plugin-todo (rpc)
  browser                plugin-browser (native: demi.browser)
  host                   the product's group (backend-host-access)
  agent, shell           the agent runtime's groups, grafted per node
```

- A plugin of this repository adds named groups to `demi`. A plugin from
  outside it adds root commands of its own, such as `jira`, never a group of
  `demi`.
- `demi agent`, `demi shell` and `demi host` are taken: the agent runtime and
  the product own them.
- A name taken twice, among the plugins' groups and roots or with a taken
  name, stops the backend at startup, and so does a declaration the command
  set refuses. A conflict is never found while a conversation runs.
- A group with a `native` leaf whose package or operation the startup catalog
  does not serve is left out whole, and the backend logs which. A deployment
  without a browser release therefore offers no `demi browser`, and the
  plugin needs no check of its own.

The set is the same for every user and conversation. A node's set differs from
it only by the agent runtime's `demi agent` group, which depends on the node
([Subagents](../agent/subagents.md#model-facing-surface)), and by a profile's
narrowing ([Profiles](#profiles)).

### Prompt text and context

A node's system prompt is rendered once, when the node is assembled
([Prompt cache](../providers/providers.md#prompt-cache)):

```text
the agent runtime's rules for its tools
the product's instructions, then each plugin's system-prompt text, in registration order
the help of the node's commands
```

A plugin's system-prompt text is fixed in its manifest. It is the same for
every user and node, so it changes only with a Demi release, which the prompt
cache rule allows once per conversation.

Anything that differs by user or conversation, or changes while a
conversation lives, reaches the model as a `context` block instead
([Transcript](../agent/runtime.md#block-types)). Before each provider request
of a node, the session asks every context source in a fixed order: the
product's execution context first, then each plugin that declared itself a
context source, in registration order. Each source is given the text of its
own context blocks that the model receives, the ones from the last
`compaction_boundary` on, oldest first, and answers new text or nothing. Each
answer becomes one block, tagged with its source. The blocks are appended
before the request and saved at once, so a request never carries a context
the transcript does not hold.

For example, `plugin-skills` writes the full list of enabled skills, and
answers again only when the list it would write differs from the newest of
its blocks. After a compaction its earlier blocks are no longer given to it,
so it writes the list again and the model, whose history now starts at the
summary, still has it.

A source that fails, such as a plugin whose stored value does not read, adds
no block, and the backend logs the failure. That is safe to ignore because
nothing is lost: the source is asked again before the next request.

### Profiles

A plugin may declare [subagent profiles](../agent/subagents.md#profiles) as
data: a name, a description, optional instructions that replace the
product's and the plugins' text in a child's system prompt, an optional list
of the command paths a child keeps of its parent's commands, whether its
children may spawn, and an optional model. Two plugins that declare the same
profile name stop the backend at startup. No plugin of this repository
declares a profile.

### Host directories

A plugin can keep files on every Host its user's conversations run jobs on.
It does not reach a Host: it tells the plugin host which directories its user
needs, and the conversation's host access installs them before a job runs.

For example, the user turns on `tdd`. `plugin-skills` sets its user's
directories to one directory, `tdd`, whose files are blobs it stored when it
fetched the skill. The plugin host computes the directory's digest, the
SHA-256 of its listing (each file's path, mode and SHA-256, in path order),
records the set, and answers the directory's path on every Host:
`~/.demi/plugins/skills/<digest>/`. The plugin names that path in its
context block.

When a job of one of the user's conversations is admitted on a Host, host
access installs each directory of the user's set that the Host does not hold
yet, before the job starts:

- A directory is installed once per runner connection. Host access asks the
  Host whether the digest's directory exists the first time a job of that
  connection needs it, and remembers the answer until the connection ends. A
  Cloud's new boot is a new connection, so it is checked again; a directory
  the user deletes by hand while the connection lasts comes back at the next
  connection.
- An installation writes the files into a temporary directory beside the
  final one, makes every file and directory read-only, keeping a file's
  executable bit, and then renames it into place. A directory that exists is
  complete, so a crash leaves at most a temporary directory, which the next
  installation removes.
- The same installation removes the plugin's directories on that Host that
  the user's set no longer names. A command that still reads a removed
  directory fails as with any file removed underneath it; the model has
  already been told the new paths.
- A job never waits for a directory it does not need: only the user's
  current set is installed, and an installation that fails fails the job with
  the Host's error.

A conversation that runs no job installs nothing, so a conversation that only
talks never wakes its Cloud for a plugin's files.

## The contract

A plugin is a factory and the instances it makes. The factory is shared by
every shard thread and carries the manifest. The plugin host asks it for one
instance per [user shard](concurrency.md#the-user-shard), and that instance
handles the user's requests on the shard's thread:

```rust
pub trait PluginFactory: Send + Sync {
    fn manifest(&self) -> &Manifest;
    /// One instance for one user's shard.
    fn instance(&self) -> Rc<dyn Plugin>;
}

pub trait Plugin {
    fn call(&self, request: Request, port: PluginPort) -> LocalBoxFuture<'_, Result<Reply, PluginError>>;
}
```

Each request names its user, since a plugin process would serve every user:

| Request | Carries | Reply |
| --- | --- | --- |
| `command` | The [rpc invocation](../execution/commands.md#handle-an-rpc-call) | The exit status |
| `context` | The conversation, the node, and the text of the source's blocks the model receives | New text, or none |
| `page_state` | Nothing more | The plugin's state for the user's pages, valid against its declared schema |
| `page_call` | The method and its parameters, already valid against the method's schema | The result, valid against the method's result schema |

The port is the plugin's side of a request, and every operation is one
request and one reply, as for an rpc handler
([Handle an rpc call](../execution/commands.md#handle-an-rpc-call)):

| Operation | Requests | Meaning |
| --- | --- | --- |
| Stdout, stderr, stdin, live stdin, command storage, cancellation | `command` | The rpc port of the call: its IO, the invoking node's [command storage](../agent/command-state-history.md), and whether the call was cancelled |
| Values | Every request | Read, list and conditional write of the plugin's own values for the user, each a JSON document with a revision ([Storage](../backend/storage.md#control-records)) |
| Blobs | Every request | Put and get bytes in the user's blob namespace, by SHA-256 |
| Host directories | Every request | Replace the user's set of [Host directories](#host-directories); the reply is each directory's path on a Host |
| Changed | Every request | Mark the plugin's part of the user's product state as changed, so every page of the user receives the new state ([The page](#the-page)) |
| Cancellation | Every request | Whether, and when, the request was cancelled |

Values and directories name the blobs they use, which keeps the blobs from
[collection](../backend/storage.md#collecting-blobs) for as long as they are
named. A value changes by versioned compare-and-set, as command storage does:
a write names the revision it read and fails when another write came first.

A plugin never receives a live object: no Host handle, no callback into the
backend, no reference to a session or a shard. What it needs from Demi is a
port operation, and what it gives Demi is a reply.

### One contract, two transports

```text
                        Request, Reply, port messages (plugin-interface types)
plugin host  ────────────────────────────────────────────────────  plugin
             in-process: a call of the trait, messages as Rust values
             process (TypeScript SDK, later): the same messages, encoded,
                                              over the process's stdio
```

- **In process.** A plugin of this repository is a crate that implements the
  traits, and the composition root links it. Its messages pass as Rust values
  and are never encoded.
- **As a process.** A plugin written with the TypeScript SDK will be a process
  that serves every user's requests. The plugin host will hold one connection
  to it per shard thread and carry the same messages over it.

Three rules keep the two the same:

- Every message type, request, reply and port operation alike, is
  serializable and has a JSON Schema (serde and schemars), so a type that
  could not cross a wire does not compile into the contract.
- Every plugin's tests run through a loopback transport
  (`plugin_interface::testing`) that encodes each request, reply and port
  message to JSON and decodes it again. A plugin that relied on something only
  a call in process can carry fails there.
- A plugin crate depends on `plugin-interface` and on contract crates, never on
  a backend crate, so it cannot reach past the port. The crate check enforces
  this ([Dependency graphs](crates-and-packages.md#dependency-graphs)).

The TypeScript SDK's types will be generated from the same Rust types, as
`@demicodes/protocol` is ([Generated TypeScript](contracts.md#generated-typescript)).

### Rules for a plugin in process

An instance runs on its user's shard thread, so it follows the shard's rules
([Concurrency](concurrency.md)):

- It never blocks the thread. Blocking work, such as `plugin-skills`'s git
  fetch, runs on the blocking pool.
- It may keep state in memory, for its own user only, and treats it as a
  cache: the instance ends with the shard and with the process, and what must
  last is a value or a blob.
- Every task it starts ends when the instance is dropped, and the instance
  saves nothing from a task that was cut off. For example, a fetch that
  `plugin-skills` was running when the backend stopped leaves the source as
  it was before the fetch.

## The plugin host

The plugin host, in `backend-plugins`, runs every plugin of the backend:

- **Registration.** The backend's composition root registers a fixed list of
  plugins, as it registers the provider families. There is no configuration
  that turns a plugin on or off, no management and no loading while the
  backend runs: a plugin is removed from a deployment by removing its line
  from the composition root. Each plugin has a unique id (`file`, `todo`,
  `browser`, `skills`), which names its values, its part of the product state,
  its directories on a Host and its page route; `execution` is the product's
  context source and is taken.
- **Startup.** The host reads every manifest, checks the commands and the
  profiles ([Commands](#commands), [Profiles](#profiles)), and gives the agent
  server the command set, the plugins' system-prompt texts, the profiles and
  the context sources. A manifest that breaks a rule stops the backend; its
  error names the plugin.
- **Shards.** When a user's shard starts, the host asks each factory for that
  user's instance, and wraps each `rpc` leaf of the plugin's commands in a
  handler that forwards the call to the instance.
- **The port.** The host answers every port operation against the user's
  storage, blobs and channels; the conversation's host access installs the
  directories the host records.

Plugins are trusted with every user's data. A plugin serves every user of
the backend, and a shared instance's users trust the deployment with what its
plugins do. Choosing the plugins is the deployment's decision, made when it is
built.

## The page

A plugin can give the web app a part of its own: a settings section, or a
[work panel](../product/web-application.md#work-panel) kind. That part is a
TypeScript package, `@demicodes/plugin-<name>` in `packages/plugin-<name>`,
named like the plugin's crate:

```text
web (product)                        web-gallery
  real PluginClient over HTTP          fixture PluginClient
  and the sync channel                 over the specimen's state
          \                               /
           +--> @demicodes/plugin-skills <+
                  settings section: composes web-ui's Skills page,
                  calls its plugin through usePlugin()
                       |
                       v
                    web-ui: PluginClient, the slots, the settings surface
```

- **Slots.** A plugin package exports what it fills: a settings section (its
  navigation entry and its page) or work panel kinds. `web` and `web-gallery`
  import the packages they show from a static list and register their slots;
  a computed import does not exist.
- **The client.** A plugin's components reach their plugin only through the
  `PluginClient` they receive with `usePlugin()`: its state, which follows
  the sync channel, and `call(method, params)`. They know no route. `web`
  supplies the client over HTTP; the gallery supplies one over the
  specimen's fixture state, so every control of a specimen acts on that state
  ([Web architecture](../product/web-application.md#package-responsibilities)).
- **Types.** A plugin's state, parameters and results are Rust types in its
  crate. `xtask contracts` generates their schemas into the plugin package's
  `src/generated/`, and the client validates every state and result it
  receives with them.
- **Where behavior lives.** A plugin package holds what `web` holds for the
  product: composition, data and handlers. A reusable component or behavior
  belongs to `web-ui`, as the rest of the web app's does.

A page call is `POST /api/plugins/:plugin/calls/:method` with the parameters
as its JSON body ([Web API](../product/web-api.md#plugin-calls)). The edge
authenticates the session, the plugin host validates the parameters against
the method's declared schema, as a command's arguments are validated
([Contracts](contracts.md#validation-at-entry)), and the user's instance
handles the call on the user's shard. A plugin's state reaches the pages as
part of the product state, on the synchronization channel
([Page synchronization](../product/web-api.md#page-synchronization)): the
snapshot holds every plugin's state, and a plugin that marks its part as
changed sends the new state to each of its user's pages.

## Decisions for the TypeScript SDK

These are open and are decided with the SDK. None of them changes the
contract above.

- **Encoding.** How the messages are framed on the process's stdio.
- **Distribution and configuration.** How a deployment installs an SDK
  plugin, where its process runs relative to the backend, and how the
  composition root learns of it; the fixed list of plugins is for linked
  crates.
- **Its page package.** How a page package from outside the repository is
  built and served to the web app without a computed import in `web`.
