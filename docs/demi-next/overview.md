# Demi Next overview

Demi Next is the hosted, multi-user web product built on Demi. The backend owns
conversations and agent execution. Runners provide filesystem, process, and shell
operations on paired devices or managed Cloud machines.

Users configure model providers and choose where tools execute. A conversation
starts with Cloud selected, but selecting Cloud does not start a machine. File
operations, processes, and providers that require a process acquire the execution
host when needed. Reading history and using an HTTP provider do not require a
running Cloud machine.

## System boundaries

The diagram shows application processes and request directions. Labels identify
the transport; responses travel over the corresponding connection. Provider
services are external to Demi.

```text
+------------------+
| Browser          |
| Chat application |
+------------------+
         |
         | HTTP + conversation WebSocket
         v
+------------------+  Provider HTTPS  +------------------+
| Backend          |---------------->| Provider service |
| Agent and data   |                 | Model inference  |
+------------------+                 +------------------+
         |
         | MessagePack / WebSocket + HTTP byte pipes
         v
+------------------+
| Runner           |
| Host operations  |
+------------------+
```

The runner can execute on a user's device or inside a managed guest. Its outbound
connection registers with the backend; the diagram's arrow indicates who requests
Host operations, not who opens that connection.

| Part | Responsibility |
| --- | --- |
| Web application | Present conversations and product controls through shared `web-ui` components. |
| Backend | Authorize users, host agent sessions, persist conversations, resolve providers, and manage execution targets. |
| Provider package | Implement its provider's authentication and inference transport. |
| Runner | Execute Host operations and brush jobs; dispatch declared commands. |
| Native command service | Run independently released native operations beside the target's files. |
| Managed-host provisioner | Create and operate the user's Cloud guest and its persistent volumes. |

Most providers make HTTP requests from the backend. Claude Code instead uses a
CLI on the conversation's execution target. Its backend runtime exchanges
stream-json with that process through Host process IO. The CLI contacts its
provider directly. [Providers and credentials](providers-and-vault.md) defines
that boundary; [native execution](native-runtime.md) defines native command
services, which are separate from provider CLIs.

## A conversation using a device

For example, a user opens a conversation, selects a paired laptop, and asks the
agent to edit a file:

1. The browser sends the conversation action to the backend.
2. The backend runs the agent and selects the authorized execution target.
3. The runner executes the shell job. A declared native command calls a resident
   command service on the laptop; an application callback returns to the backend.
4. The backend records the conversation result and streams updates to the browser.

The edited file stays on the laptop. Switching the conversation's target changes
where subsequent operations run; it does not move files or the transcript.
[Sessions and targets](sessions-and-targets.md) owns switching and attachment
rules. [Execution coordination](sessions-and-targets.md#coordinate-shared-cloud-activity) owns admission while
agent nodes or device operations are active.

Keeping the agent in the backend gives the transcript and orchestration one owner.
Keeping file operations on the target avoids transferring an entire file merely
to edit it. The tradeoff is trust: a paired device executes authorized backend
requests, so the backend is part of that device's execution trust boundary.

## Data and credentials

Conversation records live in backend storage. Working files and full shell output
live on their execution targets. The backend receives bounded shell output views
and can request target files when needed. Runner messages carry requests,
replies and small values; HTTP pipes stream file contents and command IO.
[Storage](storage.md) defines persistence, and [runner IO](runner.md#pipes-and-output)
defines output and pipe lifetimes.

The backend vault stores configured provider credentials. A process-backed
provider can pass a credential through the runner in the target process's
environment. This does not make the runner a credential vault, but the credential
is available to that process and its execution environment. The browser uses
backend APIs rather than receiving stored provider secrets.

## Terms

| Term | Meaning |
| --- | --- |
| Conversation | Backend-owned transcript, agent state, and product settings. |
| Execution target | The conversation's selection of Cloud, a device directory, or a workspace. |
| Device | A registered execution device, either user-paired or managed. |
| Host | The filesystem/process abstraction used to execute on a device. |
| Guest | A managed virtual machine running a runner. |
| Backend machine | The machine running backend infrastructure or the guest provisioner. |

The backend uses `host-remote` to access devices through the Host contract.
The Rust runner performs those operations on the device. Their shared protocol
schemas define the messages; the runner generates Rust bindings during its build.
[Package boundaries](../package-boundaries.md) owns package responsibilities and
dependencies.

## Reading map

Each design rule has one authoritative document. These links cover every other
document in this directory; a linked design does not by itself certify completed
implementation.

| Question | Document |
| --- | --- |
| What does the product let users do? | [Product](product.md) |
| How is the backend assembled and exposed? | [Backend](backend.md), [Web API](web-api.md) |
| Where is data stored? | [Storage](storage.md) |
| How are providers, credentials, and usage managed? | [Providers and vault](providers-and-vault.md) |
| Where does a conversation execute? | [Sessions and targets](sessions-and-targets.md) |
| When does an idle conversation release what it uses on a Host? | [Conversation idle and Host resource release](resource-lifecycle.md) |
| How are concurrent operations admitted? | [Execution coordination](sessions-and-targets.md#coordinate-shared-cloud-activity) |
| How are commands declared and dispatched? | [Commands](commands.md) |
| How does the agent operate browser tabs on its conversation Host? | [Conversation browser](browser.md) |
| How does a service on a device get a public URL? | [Host expose](expose.md) |
| How are native commands installed and executed? | [Native runtime](native-runtime.md) |
| What does a device runner own? | [Runner](runner.md) |
| What did a tool call edit? | [Edit tracking](edit-tracking.md) |
| How does the work panel show images, media, PDF and Markdown? | [File previews](file-previews.md) |
| How do external programs call declared commands? | [External command clients](commands.md#external-command-clients) |
| How is Cloud provisioned and reset? | [Managed hosts](managed-hosts.md) |
| How does the web application fit together? | [Web application](web-application.md) |
| Which end-to-end cases must hold? | [Scenarios](scenarios.md) |
| What is the delivery plan? | [Roadmap](roadmap.md) |

## Implementation scope

The current `createBackend` entry point opens a local control SQLite database and
local conversation stores. The multi-node control service, routing, and replicated
storage described in subsystem designs are target deployment architecture, not
the current entry point's deployment behavior. Those documents must distinguish
implemented behavior from remaining work.
