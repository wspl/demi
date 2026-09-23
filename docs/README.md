# Demi documentation

Start with the [overview](overview.md). Each design rule has one authoritative
document, and the others link to it rather than restate it. The component
gallery (`bun run web:gallery`) is the reference for components, appearance,
layout and interaction; these documents do not repeat it.

## What is Demi, and how is the code organized?

- [Overview](overview.md): what Demi is, its system boundary, one conversation end to end, where data and credentials live, and terms.
- [Crates and packages](architecture/crates-and-packages.md): every Rust crate and TypeScript package, what it owns and must not do, both dependency graphs, module layout and boundary checks.
- [Contracts](architecture/contracts.md): Rust types as the only contract definition, validation at entry, the generated TypeScript, the TypeScript boundary, and logic the browser and backend share.
- [Concurrency](architecture/concurrency.md): the threads of each program, the user shard, locks, blocking work, cancellation and cleanup, and tests and time.

## What can users do, and how does the browser talk to the backend?

- [Product](product/product.md): users, roles and instance mode; conversations and projects; writing a message; attachments; provider management; Cloud settings.
- [Web application](product/web-application.md): the browser's packages, work panel, backend communication, authentication, persistence and development loop.
- [Web API](product/web-api.md): every HTTP and WebSocket route with its request, response, status and error codes.
- [File previews](product/file-previews.md): preview kinds and viewer choice, the byte path, ending transfers, inert content, and [files named in messages](product/file-previews.md#files-named-in-messages).

## How does the backend serve requests and keep data?

- [Backend](backend/backend.md): the backend binary's modules and runtime model, authentication and ownership, browser synchronization, media by reference, failure facts, startup and shutdown, configuration, and deployment and user placement.
- [Storage](backend/storage.md): the data directory, control and conversation databases, encodings and digests, passwords and credentials at rest, the object store for blobs and the change store, multi-worker placement, and open durability decisions.

## How does the agent run a conversation?

- [Agent runtime](agent/runtime.md): sessions and turns, input, yield wakeups, the standard tools, the transcript, the rendering boundary, the frame protocol and the tree store.
- [Subagents](agent/subagents.md): the session tree, `demi agent` commands, agent messages, results, profiles and persistence.
- [Compaction](agent/compaction.md): compaction through a session copy, token estimates and window switches.
- [Failures and recovery](agent/failures-and-recovery.md): the failure record and how it is read, retries, and resuming an interrupted turn.
- [Message editing](agent/message-editing.md): editing and resending a message as one transaction.
- [Conversation fork](agent/conversation-fork.md): forking a conversation from a block, with its seed, publication and subagents.
- [Command state history](agent/command-state-history.md): versioned command storage, history boundaries and compare-and-set updates.

## Where does work execute, and how do commands run?

- [Sessions and targets](execution/sessions-and-targets.md): target selection, job binding, switching, attached hosts, host access, [shared Cloud coordination](execution/sessions-and-targets.md#coordinate-shared-cloud-activity) and recovery.
- [Resource lifecycle](execution/resource-lifecycle.md): activity, the idle window and the conversation release.
- [Runner](execution/runner.md): registration, Host operations, the Host log, [load](execution/runner.md#load), shell jobs, pipes and managed guests.
- [Commands](execution/commands.md): declarations, input and help, dispatch surfaces and manifests, rpc calls, [external command clients](execution/commands.md#external-command-clients), IO, and the file commands.
- [Native command execution](execution/native-runtime.md): native packages, installation, resident services, the command context, conversation-scoped state, user streams, the invocation protocol and publication.
- [Edit tracking](execution/edit-tracking.md): recording what a job edited, the report, the change store and delivery to the conversation.
- [Host expose](execution/expose.md): public URLs for services on a Host, their lifetime, relay and commands.

## How does the agent use a web browser?

- [Conversation browser](browser/browser.md): browser automation for a conversation and the `demi browser` command reference.
- [Live view](browser/live-view.md): watching and operating the conversation's browser tabs from the page.

## How are providers, models and credentials handled?

- [Providers](providers/providers.md): families, vendors and endpoints, the provider contract, per-account runtimes, the credential vault, login flows and inference admission.
- [Models](providers/models.md): catalog sources, the catalog cache and request parameters.
- [Usage and quota](providers/usage-and-quota.md): the usage ledger, the request rate limit and vendor quota.
- [Claude Code](providers/claude-code.md): the Claude Code CLI package, its version, where it runs, and how the provider talks to it.

## How does Cloud work, and how is it deployed?

- [Managed Cloud hosts](cloud/managed-hosts.md): provisioning, images, save and reset, lifecycle and capacity, isolation, networking and verification.
- [Cloud images](cloud/images.md): the image artifacts, their contents, import and publication, and local refresh.
- [Cloud setup](cloud/setup.md): deploying the machine manager on Linux or in Lima on a Mac, and acceptance before use.

## How is Demi delivered, built and released?

- [Roadmap](delivery/roadmap.md): dependency order, completion conditions, required evidence and open deployment decisions.
- [Scenarios](delivery/scenarios.md): backend scenario acceptance, the browser-contract suite and real-machine acceptance.
- [Builds and releases](delivery/builds-and-releases.md): the toolchain, cross builds, `cargo xtask` packaging, targets per executable and Cloud image refresh.
- [Package versioning](delivery/package-versioning.md): the npm release set and changesets, and how the Rust executables are versioned.

## How do I extend Demi?

- [Add a provider](guides/add-a-provider.md): writing a provider crate for a new vendor.
