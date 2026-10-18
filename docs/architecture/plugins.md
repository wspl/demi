# Plugins

A plugin is how a capability joins Demi. It can add commands to the agent's
shell, text the model reads, files every Host of its user's conversations
holds, reads of a conversation's files on a running Host, calls and streams
of its own command package, and a part of the web app with the calls behind
it. Every capability that is not the agent runtime itself or the product's
core is a plugin: the file commands
(`plugin-file`), the conversation browser with its live view
(`plugin-browser`), [skills](../agent/skills.md) (`plugin-skills`), and the work panel's Change
view (`plugin-changes`) and File view (`plugin-file-browser`). The agent
runtime, the runner, the backend's conversation lifecycle and the web app's
shell know none of them; a plugin's part of the web app is written against
the plugin SDK ([Plugin pages](plugin-pages.md)).

Each user turns plugins on and off in settings, while the backend runs. A
change never restarts the backend: what the plugin offers a page changes at
once, and what it gives the agent reaches a conversation when its tree opens
again ([A user's plugins](#a-users-plugins)).

Every plugin, built in or not, talks to Demi through one contract, and every
message of that contract is data. Demi's plugins are Rust crates the backend
links and calls directly. A plugin written with a TypeScript SDK runs as a
process of its own and exchanges the same messages over a wire. Demi has no
such SDK yet; the contract keeps one possible
([The TypeScript boundary](contracts.md#the-typescript-boundary)), and
[Decisions for the TypeScript SDK](#decisions-for-the-typescript-sdk) lists
what is left to decide for it.

## A plugin, end to end

For example, a user turned on the skill `tdd` in the web app and opens a new
conversation:

```text
backend start      the composition root registers plugin-skills with the others;
                   the plugin host reads its manifest: a context source, the
                   demi skills group and the category Manage skills

web app            POST /api/plugins/skills/calls/set_enabled { source, skill, enabled }
  -> plugin host   a page call: plugin-skills validates it, writes its stored
                   value, sets the Host directories its user's jobs need, and
                   marks its part of the product state as changed
  -> sync channel  every page of the user receives { type: "plugin", plugin:
                   "skills", state } and shows tdd as on

first request      the session asks each context source for news; plugin-skills
                   answers the catalog of the skills that are on, with the path
                   of each SKILL.md; the session appends it as a context block
  -> model         reads it, runs `cat ~/.demi/plugins/skills/tdd-9f2c1a7b3e40/SKILL.md`
                   through shell
  -> host access   before that job runs, installs the directory on the Host
                   once per runner connection
```

Nothing in the agent runtime is about skills. The session asked its context
sources, and the shell job's admission installed what the user's plugins
declared.

## What a plugin contributes

A plugin declares itself once, in its **manifest**: a plain value its crate
builds in code, which the plugin host reads when the plugin is registered.
Code, not a file beside the crate, because the manifest refers to what only
code holds: its schemas are derived from the Rust types the plugin uses. What
a manifest declares is fixed while the plugin is registered; for a plugin
linked into the backend, that is the life of the process. What varies by
user, conversation or time arrives through requests.

| Contribution | Declared | When Demi asks |
| --- | --- | --- |
| Identity | Its id, its name and a one-sentence description, which settings show | Never |
| Packages | Nothing: they are the [command packages](../execution/native-runtime.md) whose operations its commands, streams, page states and methods bind | Never |
| [Commands](#commands) | Their declarations, as data | Each time the model runs an `rpc` leaf of them |
| [Context](#prompt-text-and-context) | That the plugin is a context source | Before each provider request of every node |
| [Host directories](#host-directories) | Nothing | The plugin sets them through its port when its user's needs change |
| [Host files](#reading-a-conversations-files) | Nothing | The plugin reads them through its port when it needs them |
| [Package calls and user streams](#calling-its-command-package) | Each user stream's name, the operation it binds, the schemas of its messages both ways and the constants its two ends share | A call: when the plugin makes it. A stream: when a page opens it |
| [Its page](#the-page) | Its page package; for each scope, user and conversation, the schema of its state and the operations a read of it calls; each method with its scope, its parameter and result schemas and the operations it calls | When a page reads a state; when a page calls a method |

A plugin declares only what it uses: `plugin-file` declares its package and
its commands and nothing else, `plugin-skills` binds no package, and
`plugin-changes` declares only its identity and its page.

**Name and description.** Settings show a plugin by its name and its
description, so both speak to the user. A plugin with a feature the user sees
is named after that feature as the user sees it, short, in title style: the
work panel's File view comes from File Browser, the Skills settings from
Skills. A plugin that only gives the agent a `demi <group>`
command group and shows nothing is named Demi *Group* Commands, such as Demi
File Commands. The description is one full sentence, in sentence style, that
says what the plugin does for the user and names its command when it has one,
as plain text since settings show it as written, such as "Teaches the agent
workflows with skills from Git repositories you add with demi skills, and
from your repository." The gallery's
Writing page has the capitalization rules.

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
  browser                plugin-browser (native: demi.browser)
  skills                 plugin-skills (rpc)
  web                    plugin-web: search (rpc), fetch (native: demi.fetch)
  host                   the product's group (backend-host-access)
  agent, shell           the agent runtime's groups, grafted per node
```

- A plugin of this repository adds named groups to `demi`. A plugin from
  outside it adds root commands of its own, such as `jira`, never a group of
  `demi`.
- `demi agent`, `demi shell` and `demi host` are taken: the agent runtime and
  the product own them.
- A group may declare [permission categories](../agent/permissions.md#categories)
  for leaves that act on Demi itself, as any command group does; the backend's
  dispatch checks them before a call reaches the plugin, so a plugin never
  sees a permission.
- A name taken twice, among the plugins' groups and roots or with a taken
  name, refuses the plugins when they are registered, and so does a
  declaration the command set refuses: for the plugins linked into the
  backend, it stops the backend at startup. Every registered plugin is
  checked against every other, so any set of them a user turns on is free of
  conflicts, and a conflict is never found while a conversation runs.
- A group with a `native` leaf whose package or operation the startup catalog
  does not serve is left out whole, and the backend logs which. A deployment
  without a browser release therefore offers no `demi browser`, and the
  plugin needs no check of its own.

The set is the commands of the plugins the user has on, the same for every
conversation tree that opened with that set ([A user's
plugins](#a-users-plugins)). A node's set differs from it only by the agent
runtime's `demi agent` group, which depends on the node
([Subagents](../agent/subagents.md#model-facing-surface)).

### Prompt text and context

A node's system prompt is rendered once, when the node is assembled
([Prompt cache](../providers/providers.md#prompt-cache)):

```text
the identity: the product's instructions, or a profile's
the harness guide
the agent runtime's rules for its tools
the capability index of the node's commands
the model identity
```

A plugin adds nothing to it but the index entries of its command groups,
fixed in its manifest; its commands' help is read on demand with `--help`
([System prompt](../agent/system-prompt.md)). It changes only when the tree opens again with another set of
plugins, or with a Demi release, which the prompt cache rule allows once per
conversation. What a
plugin needs the model to know beyond its index entries and help reaches the model as
a `context` block, and so does anything that differs by user or
conversation, or changes while a conversation lives
([Transcript](../agent/transcript.md#block-types)). Before each provider request
of a node, the session asks every context source at once and takes their
answers in a fixed order: the product's execution context first, then the
product's [instructions](../agent/instructions.md), then each
plugin that declared itself a context source, in registration order; a source
that reads a Host does not make the others wait. Each source is given the text of its
own context blocks that the model receives, the ones from the last
`compaction_boundary` on, oldest first, with the node's working directory, how
its Host writes paths (`HostPaths`: joining, a path's last name, the
directories up to the root, so `C:\Users\alex\app` walks up to `C:\` on
Windows; [Windows paths](../execution/runner.md#windows-paths)) and the id of
its current input turn, and answers new text or nothing. Each
answer becomes one block, tagged with its source. The blocks are appended
before the request and saved at once, so a request never carries a context
the transcript does not hold.

A context source is asked only while its user has its plugin on: a plugin
turned off adds nothing from the next request on, and one turned on is
asked before the next request of every node, so its text reaches a running
conversation without a reload.

For example, `plugin-skills` writes the full catalog of the skills that are
on, and answers again only when the catalog it would write differs from the
newest of its blocks. After a compaction its earlier blocks are no longer given to it,
so it writes the list again and the model, whose history now starts at the
summary, still has it.

A source that fails, such as a plugin whose stored value does not read, adds
no block, and the backend logs the failure. That is safe to ignore because
nothing is lost: the source is asked again before the next request.

### Host directories

A plugin can keep files on every Host its user's conversations run jobs on.
It never writes to a Host: it tells the plugin host which directories its user
needs, and the conversation's host access installs them before a job runs.

For example, the user turns on `tdd`. `plugin-skills` sets its user's
directories to one directory, `tdd`, whose files are blobs it stored when it
fetched the skill. The plugin host computes the directory's digest, the
SHA-256 of its listing (each file's path, mode and SHA-256, in path order),
records the set, and answers the directory's path on every Host: the
directory's name and the first 12 hexadecimal digits of its digest, as
`~/.demi/plugins/skills/tdd-9f2c1a7b3e40/`. The plugin names that path in its
context block. A directory's name is 1 to 64 lowercase letters, digits and
hyphens, unique in the plugin's set; the short digest tells two contents of
one name apart, and a set holds one content per name.

When a job of one of the user's conversations is admitted on a Host, host
access installs each directory of the user's set that the Host does not hold
yet, before the job starts:

- A directory is installed once per runner connection. The first job of a
  connection, and the first after the user's set changed, lists every
  plugin's directory on the Host (`~/.demi/plugins/<plugin>/`) in one
  request, and host
  access remembers that the Host holds the set until the connection ends. A
  Cloud's new boot is a new connection, so it is checked again; a directory
  the user deletes by hand while the connection lasts comes back at the next
  connection. Installations on one shard take turns.
- An installation writes each directory with one request
  ([Host operations](../execution/runner.md#host-operations)): the runner
  writes the files into a temporary directory beside the final one, makes
  every file and directory read-only, keeping a file's executable bit, and
  then renames it into place. A directory that exists is
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

### Reading a conversation's files

A plugin can read the files of a conversation on its primary Host while that
Host is running, but never wakes it, writes to it or keeps it awake. For
example, `plugin-skills` looks for the skills a repository holds in its
`.agents/skills` directory ([Project skills](../agent/skills.md#project-skills)).

One request of the port names several paths, and the reply answers each, and
it reaches the Host as one request too
([Host operations](../execution/runner.md#host-operations)):
whether it exists, its kind, a directory's entries, a file's bytes up to a
bound the request gives with its size, or that the Host could not read it,
such as for want of permission. A symbolic link at a path is followed; a
directory's entries name a link as a link. The paths are absolute, as the Host names them. The
plugin host makes the request through the conversation's host access in the
form that never wakes a Host, the one a one-shot user call that only looks
uses ([Every way to a Host](../execution/sessions-and-targets.md#every-way-to-a-host)):

- A Cloud that is stopped, or a paired device whose runner is not connected,
  answers that the Host is not running, and the plugin uses what it knew
  before, or nothing.
- A read is a look, not activity: it keeps no Cloud awake
  ([Activity](../execution/resource-lifecycle.md#activity)).
- A read takes the conversation's file gate only while it is admitted, so a
  target switch or an archive ends it instead of waiting for it.

**A project search the backend keeps.** A plugin whose findings follow a
project's files, as `plugin-skills`' project skills do, declares
`projectSearch` in its manifest and answers `ProjectSearch` for a Host and
a directory with what it found and what to watch: the paths its search read
or would read, each a watch subject of the runner's
[file watch](../execution/runner.md#watching-files), the directories it
listed and the files it parsed or looked for included, whether they exist
yet or not, so a skill created, added to a group or repaired is seen. Its
answer depends on the Host and the directory alone, never on a
conversation, since one answer serves every conversation there. The
backend keeps it with the project instruction files, in one place: it asks
once, subscribes to what the answer names, asks again in the background
when the runner says one changed or the Host comes online, and hands what
it holds to the plugin's `Context` request as `project`, so the plugin's
context never waits for the Host
([When it is sent](../agent/instructions.md#when-it-is-sent)). Turning the
plugin off releases what the backend keeps for it, and turning it on asks
again at the next request.

### Calling its command package

A plugin whose commands bind a command package can use that package's
operations itself, for a conversation's user, in the two ways the backend
already offers ([User streams](../execution/native-runtime.md#user-streams)):

- **A package call** runs one operation once, with a `user` caller and the
  operation's arguments, and answers its JSON result. For example,
  `plugin-browser` calls `browser.open` for a browser tab its user creates
  in the work panel, so that tab is the tab the agent's `demi browser open`
  would have made. A call that starts work is ordinary demand through the
  conversation's host access and wakes a stopped Cloud; any other uses the
  form that never wakes a Host, and a stopped Host answers that it is
  stopped.
- **A user stream** connects a page to an operation for as long as the page
  keeps it open. The plugin declares it in its manifest, by name, with the
  operation it binds: `plugin-browser` declares `browser`, bound to
  `browser.live`. The page opens it through the backend's one user stream
  route, and the bytes never pass through the plugin.

A plugin calls only the packages its manifest binds, and a stream's name is
taken once among all plugins, or the plugins are refused when they are
registered. A stream, a page state or a page method one of whose
operations the startup catalog does not serve does not exist, as such a
command group does not. A package call says what it does on the Host, which
decides whether it wakes a stopped Cloud and whether it is activity
([Activity](../execution/resource-lifecycle.md#activity)): it **starts** work,
such as opening a tab, and wakes the Host; it **operates** on what runs
there, such as closing a tab, which is activity but never wakes; or it
**looks**, such as listing the tabs, which neither wakes nor is activity.

Together with [Host directories](#host-directories) and
[reading a conversation's files](#reading-a-conversations-files), these are
every way a plugin reaches a Host. All of them go through the conversation's
host access. A plugin never writes a Host's files and never starts a job.

### Messages from its command package

A plugin whose commands bind a command package hears from that package
when the state it keeps on a Host changes: the package says so the moment it
changes, and the plugin never asks whether it did
([Messages to the plugin](../execution/native-runtime.md#messages-to-the-plugin)).
For example, a page in the conversation browser opens a pop-up. The browser
package writes `{"tabs":"changed"}` for the conversation, and
`plugin-browser` receives it as a `package_message` request: it marks its
conversation state, the tab list, changed, so every page that shows the
conversation reads it again, and adds the new tab to the work panel
([A browser tab in the panel](../browser/live-view.md#a-browser-tab-in-the-panel)).
The agent's `rg`, a build or a `demi file edit` changes no tab, and the
plugin hears nothing.

What a message says is the package's and its plugin's to agree; the runner
and the backend only carry it, and a plugin hears only the package its
commands bind. When the package's state for a conversation ends without the
package saying so, because its service crashed, its runner started anew or
its Cloud stopped, the plugin receives `package_lost` for the conversation
from whoever saw the state end. Nothing else makes a plugin read a Host
again: not a job's end, not a reconnect, not a timer.

### Panel kinds

A plugin's page shows [work panel](../product/web-application.md#work-panel)
tabs of kinds it declares, and the manifest names the kinds whose tabs the
backend keeps, such as `plugin-browser`'s `browser`. The backend keeps each
conversation's tabs ([Work panel state](../product/web-api-conversations.md#work-panel-state))
and lets the plugin that names a kind take part in its tabs:

- **The user's changes reach the plugin.** When the user removes a tab of
  the plugin's kind, the backend applies the change, answers the page, and
  then sends the plugin a `panel_tab` request with the tab as it was when it
  was removed. The page never waits for the plugin. For example, the browser
  plugin closes the browser's tab of a panel tab its user removed. The user
  creates a tab of a plugin's kind only through an intent the kind opens,
  such as the file browser's `attachment` tab an attachment's card opens, and
  the plugin then has nothing to do; otherwise the plugin creates them
  ([A browser tab in the panel](../browser/live-view.md#a-browser-tab-in-the-panel)).
- **The plugin changes its own tabs.** Through the port, the plugin lists,
  creates, updates and removes the tabs of its kinds in the request's
  conversation, as the page does, with ids that are used once
  ([Work panel state](../product/web-api-conversations.md#work-panel-state)). For example,
  the browser plugin writes the browser's tab id into the tab it opened, and
  adds a tab for each tab the agent opened.

The backend applies a conversation's panel changes one at a time, the page's
and the plugin's alike. A plugin whose work on one conversation's tabs takes
several port operations, such as reading the browser's tabs and then adding
the missing ones, does that work one piece at a time per conversation, so a
change it makes is never based on what it read before its own earlier change.

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
| `context` | The conversation, the node, its working directory, the id of its current input turn, and the text of the source's blocks the model receives | New text, or none |
| `page_state` | The scope: the user, or one of the user's conversations | The plugin's state of that scope, valid against the scope's declared schema |
| `page_call` | The method, its parameters, already valid against the method's schema, and the conversation when the page called for one | The result, valid against the method's result schema |
| `panel_tab` | The conversation and the tab the user removed, of one of the plugin's [panel kinds](#panel-kinds) | Nothing |
| `package_message` | The conversation, and the message its command package sent ([Messages from its command package](#messages-from-its-command-package)) | Nothing |
| `package_lost` | The conversation whose state its command package kept on a Host ended without the package saying so | Nothing |

The port is the plugin's side of a request, and every operation is one
request and one reply, as for an rpc handler
([Handle an rpc call](../execution/commands.md#handle-an-rpc-call)). Its
operations are grouped by the service that answers them. A
conversation's operations need a request about a conversation: a `command`, a
`context`, a `page_state` or `page_call` of the conversation scope, a
`panel_tab`, a `package_message` or a `package_lost`.

| Service | Operations | Requests | Meaning |
| --- | --- | --- | --- |
| Command | Stdout, stderr, stdin, live stdin | `command` | The rpc port of the call: its IO. A command that makes an image writes its bytes, which the model views with `demi file view` ([Return media](../execution/commands.md#return-media)) |
| Storage | Values: read, list, conditional write, conditional removal | Every request | The plugin's own values for the user, each a JSON document with a revision ([Storage](../backend/storage.md#control-records)) |
| | Blobs: put, get | Every request | Bytes in the user's blob namespace, by SHA-256 |
| Hosts | Set directories | Every request | Replace the user's set of [Host directories](#host-directories); the reply is each directory's path on a Host |
| | Read files | A conversation's | Several paths on the conversation's primary Host, if it is running; for a `command`, on the Host its job runs on, a relative path resolved against the job's directory as that Host resolves it ([Reading a conversation's files](#reading-a-conversations-files)) |
| | Call a package | A conversation's | One operation of a package the manifest names, waking the Host or not ([Calling its command package](#calling-its-command-package)) |
| Pages | Changed | Every request | Mark one scope of the plugin's page state, the user's or one conversation's, as changed, so the pages that show it read it again ([The page](#the-page)) |
| Panel | List, create, update, remove | A conversation's | The conversation's work panel tabs of the plugin's own [panel kinds](#panel-kinds); a change answers the panel's revision |
| Secrets | Read, conditional write, removal | Every request | The plugin's own [secrets](#secrets) for the user, sealed at rest |
| Web | Send a request | Every request | One outbound HTTPS request through the backend's shared client, with a deadline and a body limit the plugin names ([Outbound requests](#outbound-requests)) |
| | OAuth: start, token, sign out | Every request | The user's sign-in to a vendor the manifest names ([OAuth sign-in](#oauth-sign-in)) |
| Providers | Image models, make an image | Every request | The image models of the user's provider entries, and one image from a prompt and images through an entry's account, which the backend chooses, rotates and records in the usage ledger; no token reaches the plugin ([Image generation](../agent/images.md)) |
| Request | Cancellation | Every request | Whether, and when, the request was cancelled |

A product service a plugin may use is one service of the port and its
operations. Hosts is such a
service; one joins when a plugin needs it, and never as an operation shaped
for one plugin among the others.

Values and directories name the blobs they use, and a blob one of them
names is never removed
([Deleting a conversation](../backend/storage.md#deleting-a-conversation)). A value changes by versioned compare-and-set: a write names the
revision it read and fails when another write came first.

### Secrets

A secret is a value a plugin keeps for a user that must not be readable from
the database, such as an API key or an OAuth token. It is a value of its own
kind beside the plugin's JSON values, with the same versioned compare-and-set,
sealed as provider credentials are
([Passwords and credentials at rest](../backend/storage.md#passwords-and-credentials-at-rest)):
AES-256-GCM under a key derived from the instance secret, with the plugin,
the user and the secret's name as its additional data, so a sealed value
copied to another row does not open. Only the plugin that wrote a secret
reads it, and no page call, page state or log carries one: a page learns
only what the plugin's state says about it, such as the last four characters
of a key.

### Outbound requests

A plugin that calls a vendor sends its requests through the port, never a
client of its own, so every outbound request of the backend's plugins has one
owner: the shard's HTTP client for plugins, which sends only `https` and
follows no redirect to `http`, kept apart from the providers' client, which
must reach local endpoints; its proxy settings, a deadline the
plugin names up to 60 seconds, a response body limit up to 16 MiB, and the
request's cancellation. Only `https` is sent, and a log line names the plugin,
the host and the status, never a header or a body.

### OAuth sign-in

A plugin whose vendor signs users in with OAuth 2.1 names it in its manifest:
the vendor's name as the user knows it, such as *Exa*, the authorization
server's issuer, the resource and the scopes. The plugin
host runs the flow, one for every such plugin:

1. **Registration.** Once per instance, plugin and issuer, the host
   registers a public client by dynamic client registration, with the
   redirect URI `<public URL>/api/plugins/<plugin>/oauth/callback`, and keeps
   the client id and that URI in the control store; a client serves one
   plugin, since its redirect URI names it, and a change of the public URL
   registers again. A registration the vendor forgot is made again at the
   next sign-in: OAuth forbids a vendor to send the browser back for a client
   it does not know, so the user sees the vendor's error once, and the token
   endpoint's `invalid_client` tells the host to register anew.
2. **Start.** A page method of the plugin asks the port to start; the host
   makes a PKCE verifier (S256) and a `state`, keeps both with the user for 10
   minutes in the user's shard, and answers the authorization URL, which the
   page opens in a new tab through its page context's `links`
   ([The page context](plugin-pages.md#the-page-context)). A sign-in pending when the backend restarts ends
   there: its callback answers that it expired.
3. **Callback.** The vendor sends the browser to the callback, which is a
   session route: the browser's own navigation carries the session cookie, so
   a `state` another user started is refused. The host exchanges the code,
   seals the tokens as the plugin's secrets, marks the plugin's user state
   changed, and answers a page that says *Signed in to Exa. You can close
   this tab.*
4. **Token.** The port's token operation answers a live access token,
   renewing it first when it expires within a minute. A renewal takes the
   secret's revision and replaces it at that revision, so two workers never
   spend one refresh token twice, as for provider credentials
   ([Token refresh](../providers/providers.md#token-refresh)), and runs to
   its end when its caller goes away. Only the vendor's refusal of the
   refresh token itself, an HTTP 400 or 401 whose body is a JSON object
   with `error: "invalid_grant"`, forgets the tokens and
   answers *signed out*; any other failure, another error, a timeout or no
   answer, keeps them and answers that the vendor did not answer, so the
   next call renews again. The log names each refused renewal's HTTP status
   and OAuth `error` code when it is one OAuth defines, `other` otherwise,
   never the vendor's text, its description or a token, so a lost
   sign-in can be traced.
5. **Sign out.** Forgets the tokens, then revokes them at the vendor when it
   offers revocation. A sign-out, and a renewal the vendor refused, mark the
   plugin's user state changed as the callback does, so no page goes on
   showing *Signed in*.

A plugin never sees a refresh token or the verifier; it reads the access
token only to send it in a request it makes through [Outbound
requests](#outbound-requests).

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
- A task may keep the port of the page call that started it: the port
  answers after the call's reply, until the task ends. For example,
  `add_source` answers once the source is recorded, and its fetch writes the
  source's value through the same port when it ends.

## The plugin host

The plugin host, in `backend-plugins`, runs every plugin of the backend:

- **Registration.** The backend's composition root registers the plugins
  linked into it, as it registers the provider families: these are the
  plugins the backend offers, and a user turns each on or off ([A user's
  plugins](#a-users-plugins)). Each plugin has a unique id (`file`,
  `browser`, `skills`), which names its values, its part of the product state,
  its directories on a Host and its page route; `execution` and
  `instructions` are the product's context sources and are taken. Its manifest also carries a name and a
  one-sentence description, which settings show.
- **Startup.** The host reads every manifest and checks the commands and the
  streams ([Commands](#commands)). A manifest that breaks a rule stops the
  backend; its error names the plugin.
- **Shards.** When a user's shard starts, the host asks each factory for that
  user's instance, and wraps each `rpc` leaf of the plugin's commands in a
  handler that forwards the call to the instance. While the user has a
  plugin off, the instance receives no page call and no context request,
  and is asked for no page state; only the commands of a tree that opened
  with it on still reach it, until that tree opens again.
- **The user's plugin set.** The host gives the agent server the commands of
  the plugins the user has on, which a tree takes when it opens, and the
  context sources, which are asked while their plugin is on.
- **User streams.** The host holds every plugin's stream declarations, which
  the user stream route opens by name while the plugin is on.
- **Package messages.** The host gives each `package_message` and
  `package_lost` a device sends to the plugin whose commands bind the
  package, while the user has it on, in the order they arrive
  ([Messages from its command package](#messages-from-its-command-package)).
- **The port.** The host answers every port operation against the user's
  storage, blobs and channels. The conversation's host access
  installs the directories the host records, reads the Host files a plugin
  asks for in its form that never wakes a Host, and runs its package calls.

Plugins are trusted with every user's data. A plugin serves every user of
the backend, and a shared instance's users trust the deployment with what its
plugins do. Choosing the plugins is the deployment's decision, made when it is
built.

## A user's plugins

Each user has each plugin on or off, and every plugin starts on. For
example, a user who has no use for the conversation browser turns
`browser` off in settings:

```text
settings page      PUT /api/plugins/browser { enabled: false }
  -> plugin host   stores the user's choice; the user's plugin set changes
  -> sync channel  every page of the user receives the new plugin list: the
                   browser kind leaves the work panel at once
open conversation  its tree opened with demi browser; the page shows that
                   the user's plugins changed and offers a reload
reload             the tree closes and opens again with the new set: demi
                   browser is gone from its commands and its system prompt
```

**When a change takes effect.** A plugin's contributions reach a user's
conversations at different moments, and each takes effect at the first
moment that does not change something the model already relies on:

| Contribution | A change takes effect |
| --- | --- |
| Page state and page methods | At once: every page of the user receives the new plugin list |
| Context | Before the next request of every node |
| Host directories | Before the next job of each conversation: a plugin turned off has its directories removed then |
| User streams | When a page opens one; an open stream of a plugin turned off ends |
| Commands | When a conversation's tree opens: a new conversation, a tree restored after it was disposed, or a reload |

Commands wait for a tree to open because a tree takes them once: a node's
capability index is part of its system prompt, which is rendered once
([Prompt text and context](#prompt-text-and-context)). A change in the middle
of a conversation would change what the model was told it may run.

**The plugin set.** The plugins a user has on, at one moment, are the user's
**plugin set**, with a revision that changes whenever the set's commands
change. Turning on a plugin that declares none, such as one that contributes
only a context source and a settings section, changes the set without
changing its revision. A tree records the revision it opened with.

**Reload.** While a conversation's tree is open with a revision that is not
the user's current one, the conversation's summary says so, and the page
offers to reload it. A reload closes the tree and opens it again, as a
backend restart would; it waits for nothing and is refused while the tree
works, as an archive is. The conversation's history, queue and children
are kept; the model's next request is the first with the new commands, and
the new help reaches it as a changed system prompt, once. A tree that is
disposed after it has been idle opens with the current set the next time
anyone opens it, so a reload is never needed for a conversation nobody
looks at.

**Settings.** The settings page lists every plugin with its name, its
description and a switch. A plugin that needs settings of its own, such as
`plugin-skills`, gives the web app a settings section
([The page](#the-page)).

## Built-in plugins

These are the plugins the composition root registers, in this order. Each
row's contributions are all that plugin declares; what is not listed it does
not use.

| Id | Name | Crate | Contributes | Page package | Design |
| --- | --- | --- | --- | --- | --- |
| `file` | Demi File Commands | `plugin-file` | The `demi file` group, bound to `demi.file` | None | [File commands](../execution/commands.md#file-commands) |
| `browser` | Browser | `plugin-browser` | The `demi browser` group, bound to `demi.browser`; the `browser` user stream; package calls; conversation state, the conversation browser's tabs, following `calls`, with methods to open, close, navigate and go back | `@demicodes/plugin-browser`: the `browser` work panel kind with the live view | [Conversation browser](../browser/browser.md#command-contract), [Live view](../browser/live-view.md) |
| `skills` | Skills | `plugin-skills` | A context source; the `demi skills` group and the category Manage skills; values and blobs; Host directories; Host file reads; user state and six methods | `@demicodes/plugin-skills`: a settings section with its sidebar entry | [Skills](../agent/skills.md) |
| `web` | Web | `plugin-web` | The `demi web` group: `search`, an rpc leaf, and `fetch`, bound to `demi.fetch`; secrets, outbound requests and an OAuth sign-in to Exa; user state and methods to sign in, sign out, save and remove a key | `@demicodes/plugin-web`: the Web Search settings section with its sidebar entry | [Web search and fetch](../agent/web.md) |
| `image` | Image Generation | `plugin-image` | The `demi image` group, rpc leaves; Host file reads; the Providers service | None | [Image generation](../agent/images.md) |
| `changes` | Changes | `plugin-changes` | Its identity and its page package | `@demicodes/plugin-changes`: the pinned `change` kind, the Change view | [Changes](../product/file-previews.md#changes), [Edit tracking](../execution/edit-tracking.md) |
| `file-browser` | File Browser | `plugin-file-browser` | Its identity and its page package | `@demicodes/plugin-file-browser`: the pinned `file` kind, the File view | [File previews](../product/file-previews.md) |

Besides the plugins, the components that carry them are:

| Component | Holds |
| --- | --- |
| `plugin-interface` | The contract: factory and instance traits, manifest, requests and replies, the port, the JSON loopback transport for tests |
| `backend-plugins` | The plugin host: registration and its checks, the command set and context sources for the agent server, instances, the port's services, package messages, page state of both scopes with the conversation revisions, and page calls |
| `backend-user-shard`, `backend-host-access`, `backend-http`, `backend` | The product's side: the agent server's dependencies, the execution context source and the product's instructions; the installation of Host directories, the reads of Host files and the package calls; the page call routes, the conversation state route, the plugin switch route and the user stream route; the plugins linked into the backend |
| `agent-tools`, `agent-server`, `agent-session` | The runtime's side: the rules for its tools in the system prompt, the Host resolver, context sources with their sources and turns |
| `plugin-sdk`, `web-ui`, `web`, `web-gallery` | The page API: `definePage`, `usePage`, intents, the conversation files service and the plugin kit ([Plugin pages](plugin-pages.md)); the shell and the primitives; the page context over HTTP, the sync channel and the user stream route, and the generated registry; the page context over each specimen's fixtures |

## The page

A plugin can give the web app a part of its own: the page package its
manifest names, which fills the shell's slots with a settings section and
its sidebar entry, or work panel kinds.
[Plugin pages](plugin-pages.md) owns that side: the page object, the page
context, intents, the plugin kit, registration and versions. This section
owns what the backend gives a page.

A plugin's page state has two scopes, each declared with its schema in the
manifest, and each marked changed on its own, through the port:

| Scope | Holds, for example | Reaches the pages |
| --- | --- | --- |
| User | The user's skill sources | In the product state, on the synchronization channel: the snapshot holds it, and each change sends the new state to every page of the user ([Page synchronization](../product/web-api-sync.md#page-synchronization)) |
| Conversation | The conversation browser's tab list | By revision, as a draft does: the conversation's summary carries the revision of each plugin's conversation state, and a page that shows the conversation reads the state when the revision is higher than the one it holds ([Conversation state of plugins](../product/web-api-sync.md#conversation-state-of-plugins)) |

The plugin host answers a read of either scope with a `page_state` request
for that scope, and counts the revision of each conversation's state in
memory ([Revisions counted in memory](../product/web-api-sync.md#revisions-counted-in-memory)). A page
call is `POST /api/plugins/:plugin/calls/:method` for a method of the user
scope, or `POST /api/conversations/:id/plugins/:plugin/calls/:method` for one
of the conversation scope, with the parameters as its JSON body
([Web API](../product/web-api-sync.md#plugin-calls)). The edge authenticates the
session, the plugin host validates the parameters against the method's
declared schema, as a command's arguments are validated
([Contracts](contracts.md#validation-at-entry)), and the user's instance
handles the call on the user's shard. A page also reaches the plugin's user
streams of a conversation. No plugin declares or starts an install: it
follows from the call.

A page reaches all of this only through its page context
([The page context](plugin-pages.md#the-page-context)) and knows no route;
`web` supplies it over HTTP, the gallery over each specimen's fixture state.
The types of both states, the calls and the streams are generated from the
manifest into the page package, and the web app's registry from the
manifests of the registered plugins ([Types](plugin-pages.md#types),
[Registration](plugin-pages.md#registration)).

## Decisions for the TypeScript SDK

These are open and are decided with the SDK. None of them changes the
contract above.

- **Encoding.** How the messages are framed on the process's stdio.
- **Installation.** How an SDK plugin is installed while the backend runs,
  for the instance or for one user, and where its process runs. Its manifest
  is checked against every registered plugin when it is installed, and a
  conflict refuses the installation; the linked plugins are checked at
  startup.
- **Isolation.** A plugin that the agent writes for its user, or that comes
  from outside the repository, runs code nobody reviewed. How its process is
  confined to its port, and to its own user's data, is decided with the SDK;
  a linked plugin is trusted.
- **Its page package.** How a page package from outside the repository is
  built, served and loaded, within what [Pages from outside the
  repository](plugin-pages.md#pages-from-outside-the-repository) keeps
  possible.
- **A port after the reply.** A linked plugin's task may keep a page call's
  port after the call answered ([Rules for a plugin in
  process](#rules-for-a-plugin-in-process)). Over a wire, that is a message
  of a request that already has its reply, which the encoding must
  allow.
