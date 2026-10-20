# Demi overview

Demi is a hosted, multi-user coding-agent product that people use in their own
web browser. The work divides as follows:

- The backend owns conversations and agent execution.
- Runners provide filesystem, process and shell operations on paired devices
  and on each user's managed Cloud.

The backend, the runner, the native command programs and the machine manager
are Rust programs. The web app is Vue and TypeScript.

Users configure model providers and choose where tools execute. A conversation
starts with Cloud selected, but selecting Cloud does not start a machine.

The execution host starts only when something needs it, as follows:

- File operations and processes acquire the execution host when needed.
- A provider that runs a process, such as Claude Code, acquires the Claude Code
  machine.
- Reading history and using an HTTP provider do not require a running Cloud
  machine.

## System boundaries

The following diagram shows application processes and request directions.
Labels identify the transport, and responses travel over the corresponding
connection. Provider services are external to Demi.

```text
+--------------------+
| Web application    |
| in the user's      |
| browser            |
+--------------------+
          |
          | HTTP + synchronization and conversation WebSockets
          v
+--------------------+  Provider HTTPS   +--------------------+
| Backend            |------------------>| Provider service   |
| Agents and data    |                   | Model inference    |
+--------------------+                   +--------------------+
     |          |
     |          | JSON lines over a Unix socket
     |          v
     |   +--------------------+  runsc   +--------------------+
     |   | Machine manager    |--------->| Cloud sandbox      |
     |   | Linux host         |          | with its runner    |
     |   +--------------------+          +--------------------+
     |
     | MessagePack over WebSocket + HTTP byte pipes
     v
+--------------------+
| Runner             |
| Host operations    |
+--------------------+
```

A runner executes on a user's paired device or inside the user's Cloud
sandbox. In both places its outbound connection registers with the backend.
The arrow indicates who requests Host operations, not who opens that
connection.

The following table lists what each part is responsible for:

| Part | Responsibility |
| --- | --- |
| Web application | Present conversations and product controls through shared `web-ui` components. |
| Backend | Authorize users, host agent sessions, persist conversations, resolve providers, and manage execution targets and each user's Cloud. |
| Providers | Implement each vendor's authentication and inference transport inside the backend. |
| Runner | Execute Host operations and shell jobs, and dispatch declared commands. |
| Native command service | Run independently released native operations beside the target's files. |
| Machine manager | Run each user's Cloud as a gVisor sandbox with persistent volumes on a Linux host. |

Most providers make HTTP requests from the backend. Claude Code instead runs
its CLI on the user's Cloud, whatever the conversation's execution target is.
The two sides of that boundary work as follows:

- The provider in the backend exchanges stream-json with that process through
  the Host process interface.
- The CLI contacts the vendor directly.

The following files define the boundaries:

- Providers: `crates/backend-providers/src/lib.rs`
- Claude Code: `crates/provider-claude-code/src/lib.rs`
- Native command services, which are separate from provider CLIs:
  `crates/command-protocol/src/lib.rs`

Rust types in a contract crate that both ends link define each wire between
these parts once. The command `bun run contracts` generates the web app's
schemas from those types ([Contracts](../ARCHITECTURE.md)).

[Crates and packages](../ARCHITECTURE.md) owns the responsibilities and
dependencies of every crate and package.

## A conversation using a device

For example, a user opens a conversation, selects a paired laptop, and asks the
agent to edit a file:

1. The page sends the conversation action to the backend.
2. The backend runs the agent and selects the authorized execution target.
3. The runner executes the shell job. A declared native command calls a
   resident command service on the laptop, and an application callback returns
   to the backend.
4. The backend records the conversation result and streams updates to the
   page.

The edited file stays on the laptop. Switching the conversation's target
changes where subsequent operations run. It does not move files or the
transcript.

Two files own the rules around a target:

- Sessions and targets (`crates/backend-host-access/src/lib.rs`) owns switching
  and attachment rules.
- Execution coordination (`crates/backend-cloud/src/uses.rs`) owns admission
  while agent nodes or device operations are active.

Keeping the agent in the backend and the files on the target has two
benefits:

- Keeping the agent in the backend gives the transcript and orchestration one
  owner.
- Keeping file operations on the target avoids transferring an entire file
  merely to edit it.

The tradeoff is trust. A paired device executes authorized backend requests, so
the backend is part of that device's execution trust boundary.

## Data and credentials

Conversation records live in backend storage. Working files and full shell
output live on their execution targets. Data moves between them as follows:

- The backend receives bounded shell output views and can request target files
  when needed.
- Runner messages carry requests, replies and small values.
- HTTP pipes stream file contents and command IO.

The following files define persistence and lifetimes:

- Persistence: `crates/backend-database/src/lib.rs`
- Output and pipe lifetimes (runner IO): `crates/runner-jobs/src/tasks.rs`

The backend vault stores configured provider credentials, encrypted with the
instance secret.

The Claude Code provider passes the selected account's token to the CLI on the
user's Cloud, in the process's environment. This does not make the runner a
credential vault, but the token is available to that process and its execution
environment. It never reaches a paired device.

The web app uses backend APIs rather than receiving stored provider secrets.

## Terms

The following table defines the terms this page uses:

| Term | Meaning |
| --- | --- |
| Conversation | Backend-owned transcript, agent state, and product settings. |
| Execution target | The conversation's selection of Cloud, a device directory, or a workspace. |
| Device | A registered execution device, either user-paired or managed. |
| Host | The filesystem and process abstraction used to execute on a device. |
| Host access | The one way the backend reaches a conversation's Host: it resolves the target, wakes a stopped Cloud and holds the conversation's file gate (`crates/backend-host-access/src/access.rs`). |
| Guest | A managed gVisor/systrap sandbox running a runner. |
| User shard | The part of the backend that holds everything belonging to one user, such as conversations, runner connections and Cloud state, on a single thread ([The user shard](../crates/backend-user-shard/src/lib.rs)). |
| Backend machine | The machine that runs the backend or the machine manager. |

The [reading index](README.md) lists every design document by the question it
answers.
