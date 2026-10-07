# Demi documentation

Start with the [overview](overview.md). Each design rule has one authoritative
document, and the others link to it rather than restate it. The component
gallery (`bun run web:gallery`) is the reference for components, appearance,
layout and interaction; these documents do not repeat it.

## What is Demi, and how is the code organized?

- [Overview](overview.md): what Demi is, its system boundary, one conversation end to end, where data and credentials live, and terms.
- [Crates and packages](architecture/crates-and-packages.md): every Rust crate and TypeScript package, what it owns and must not do, both dependency graphs, module layout and boundary checks.
- [Contracts](architecture/contracts.md): Rust types as the only contract definition, validation at entry, the generated TypeScript, the TypeScript boundary, and logic the web app and backend share.
- [Concurrency](architecture/concurrency.md): the threads of each program, the user shard, locks, blocking work, cancellation and cleanup, and tests and time.
- [Plugins](architecture/plugins.md): how a capability joins Demi: commands, context, Host directories, Host file reads and a page, the contract every plugin uses in process and over a wire, the plugin host, and the built-in plugins.
- [Plugin pages](architecture/plugin-pages.md): the web app as a shell that plugins fill: the page object and its context, work panel kinds, intents, the data a page shows, the plugin kit, types, registration and versions.

## What can users do, and how does the web app talk to the backend?

- [Product](product/product.md): users, roles and instance mode; conversations and projects; writing a message; attachments; provider management; Cloud settings.
- [Web application](product/web-application.md): the web app's packages, work panel, backend communication, [page synchronization](product/web-application.md#page-synchronization), authentication, persistence, [drafts](product/web-application.md#drafts) and development loop.
- [Web API](product/web-api.md): every HTTP and WebSocket route with its request, response, status and error codes.
- [File previews](product/file-previews.md): preview kinds and viewer choice, the byte path, ending transfers, inert content, [files named in messages](product/file-previews.md#files-named-in-messages), and [media a tool returned](product/file-previews.md#media-a-tool-returned).

## How does the backend serve requests and keep data?

- [Backend](backend/backend.md): the backend binary's modules and runtime model, authentication and ownership, page synchronization, media by reference, failure facts, startup and shutdown, configuration, a development backend with the native programs a developer built, and deployment and user placement.
- [Storage](backend/storage.md): the data directory, control and conversation databases, encodings and digests, passwords and credentials at rest, the one object store for blobs and the published command packages, [retention](backend/storage.md#retention): everything stays while the account does, multi-worker placement, and open durability decisions.

## How does the agent run a conversation?

- [Agent runtime](agent/runtime.md): sessions and turns with what the product supplies, input, yield wakeups, the standard tools, the transcript and its context blocks, the rendering boundary, the frame protocol and the tree store.
- [Subagents](agent/subagents.md): the session tree, `demi agent` commands, agent messages, results, profiles and persistence.
- [Compaction](agent/compaction.md): compaction through a session copy, token estimates, request sizes and window switches.
- [Failures and recovery](agent/failures-and-recovery.md): the failure record and how it is read, retries, and resuming an interrupted turn.
- [Message editing](agent/message-editing.md): editing and resending a message as one transaction.
- [Conversation fork](agent/conversation-fork.md): forking a conversation from a block, with its seed, publication and subagents.
- [Skills](agent/skills.md): the Agent Skills format, user skills from git sources, project skills from the repository, the catalog the model sees, and the settings section.
- [Conversation permissions](agent/permissions.md): the categories of operations on Demi itself, the check every command passes before its handler, the requests a refused command raises, the per-conversation grants, the decision's message to the agent, and the card and needs-you mark.

## Where does work execute, and how do commands run?

- [Sessions and targets](execution/sessions-and-targets.md): target selection, job binding, switching, attached hosts, host access, [shared Cloud coordination](execution/sessions-and-targets.md#coordinate-shared-cloud-activity) and recovery.
- [Resource lifecycle](execution/resource-lifecycle.md): activity, the idle window and the conversation release.
- [System prompt](agent/system-prompt.md): what a node's model reads before the conversation: identity, harness guide, tool rules, the capability index each command group writes, and the model's own name; details on demand with `--help`.
- [Direct channel](execution/direct-channel.md): how a page and a paired device's runner talk without the backend in the middle, over WebRTC data channels the backend introduces, and how the page moves between that and the relay.
- [Runner](execution/runner.md): registration, Host operations, the Host log, [load](execution/runner.md#load), shell jobs, pipes and managed guests.
- [Commands](execution/commands.md): declarations, input and help, dispatch surfaces and manifests, rpc calls, [external command clients](execution/commands.md#external-command-clients), IO, and the file commands.
- [Native command execution](execution/native-runtime.md): command packages, installation, resident services, the command context, conversation-scoped state, user streams, the invocation protocol and publication.
- [Edit tracking](execution/edit-tracking.md): recording what a job edited, the report, its copies as blobs and delivery to the conversation.

## How does the agent use a web browser?

- [Conversation browser](browser/browser.md): browser automation for a conversation and the `demi browser` command reference.
- [Live view](browser/live-view.md): watching and operating the conversation's browser tabs from the page.
- [Web preview](browser/preview.md): a page of the Host rendered in the user's own browser, with every request leaving from the Host: the preview domain, the forwarder and relay, the stream, the engine, the runtime, page state between the two browsers, and security.

## How are providers, models and credentials handled?

- [Providers](providers/providers.md): families, vendors and endpoints, the provider contract, the [prompt cache](providers/providers.md#prompt-cache) and the rule that each request extends the previous one, per-account runtimes, the credential vault, login flows and inference admission.
- [Models](providers/models.md): catalog sources, the catalog cache, request parameters and request limits.
- [Usage and quota](providers/usage-and-quota.md): the usage ledger, the request rate limit and vendor quota.
- [Claude Code](providers/claude-code.md): the Claude Code CLI package, its version, where it runs, and how the provider talks to it.

## How does Cloud work, and how is it deployed?

- [Managed Cloud hosts](cloud/managed-hosts.md): provisioning, images, save and reset, lifecycle and capacity, isolation, networking and verification.
- [Cloud images](cloud/images.md): the image artifacts, their contents, import and publication, and local refresh.
- [Cloud setup](cloud/setup.md): deploying the machine manager on Linux, and acceptance before use.

## How is Demi delivered, built and released?

- [Roadmap](delivery/roadmap.md): dependency order, completion conditions, required evidence and open deployment decisions.
- [Scenarios](delivery/scenarios.md): backend scenario acceptance, the web app contract suite and real-machine acceptance.
- [Testing](delivery/testing.md): what a test protects, its level, how it proves itself, time and stability, cost, resources and coverage.
- [Product checks](delivery/browse.md): `bun browse`, which runs an agent's Playwright check script against a browser that stays open, and why every agent extends it.
- [Builds and releases](delivery/builds-and-releases.md): the toolchain, cross builds, `bun xtask` packaging, the server release, the release workflow on GitHub's hosted runners, the Chrome for Testing pin, targets per executable and Cloud image refresh.
- [Installation](delivery/installation.md): installing a server with one command, its parameters for people and agents, the HTTPS shapes, the steps and the distributions.
- [Upgrades](delivery/upgrades.md): one release on a server, what follows it and how, what crosses releases, the upgrade, its interruptions and rollback, and `demi-server`.
- [Package versioning](delivery/package-versioning.md): the npm release set and changesets, and how the Rust executables are versioned.

## How do I extend Demi?

- [Add a provider](guides/add-a-provider.md): writing a provider crate for a new vendor.
- [Develop on a Mac with Lima](guides/mac-development.md): an optional setup that runs the machine manager and Clouds in a Lima VM beside a backend on the Mac.
