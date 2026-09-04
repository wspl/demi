# Demi Next: Product Design

| | |
|---|---|
| Date | 2026-09-02 |
| Status | Design (mechanisms and endpoints through M6; multi-user delivered in M12; UI in M13) |
| Scope | What the product stores and exposes: instance mode, users, conversations, attachments, provider management, the web UI and frontend package |

## Instance mode: shared vs isolated

An instance runs in exactly one of two modes; there is no mixing and no
per-provider ownership machinery. The mode is a deployment decision
made at startup (`DEMI_INSTANCE_MODE`), never changed from the product:

- **Shared**: providers are instance-wide. Only admins create,
  modify or delete them; ordinary users just use the models. Typical
  self-host.
- **Isolated**: every user manages their own providers; nothing
  is shared. Typical public host.

Usage is metered per user in both modes; in shared mode admins also
see the instance's usage by user. There are no instance settings beyond
the mode, and the page reads it (`GET /api/settings`) to know what to
show. A **provider** is one entry of the scope's list: a runtime family
(openai, anthropic, google, claude-code, codex, grok-build) with one
credential — an API key, or a completed subscription login — at an
endpoint, with a model source. A scope holds at most one entry per
subscription family and any number of API-key entries, each with its
own label, so model selection is keyed by `(providerId, modelId)` — one
provider runtime per entry.

## User system

Username + password (argon2id), cookie session (httpOnly, 30 days
sliding); the conversation stream WebSocket and the Web API authenticate
by the same same-origin cookie. **No self-registration and no password
recovery** — zero mail dependency. The instance's first account is made
by the setup call while it has no users (`POST /api/setup`), which the
login page routes to when `GET /api/setup` says so. Everyone can change
their own password with the current one in hand; five failed logins in a
row lock the username for a minute. Accounts are managed from an admin
page:

- **master**: the instance's first account, created at initial setup; can
  do everything, including creating admins.
- **admin**: everything master can do except creating admins — creates
  users, resets passwords, manages the instance's providers (shared mode),
  edits instance settings.
- **user**: uses the product.

No organizations, teams or further roles. **User data is isolated
absolutely**: a conversation, device, workspace, attachment or usage row
is visible to its owner alone; master and admin manage accounts and, in
shared mode, the providers — they never read another user's data.

## Conversation system

A conversation is one AgentSession plus one metadata row: id, owner,
title, execution target (`sessions-and-targets.md`), provider/model
selection, timestamps, archived flag.

- **Archive only, no delete.** Archiving hides a conversation; an archived
  view lists them and any can be restored. No user data is deleted in v1.
- **Titles**: default is the first user message plus manual rename.
- **New conversation is one click**: immediately typeable — target defaults
  to hostless, model defaults to the user's last-used selection.
- **Message-level operations: everything Demi implements gets exposed** —
  mid-turn steering, the message queue, abort, retry, resume, manual
  compaction, mid-conversation provider/model switch, interactive stdin to
  running commands (`shell_write`). Most ships with the web-ui components.

The **streaming interface is the agent frame protocol** — no parallel SSE
API; cold history rides the same rendering path.

## Attachments

- **Message attachments** (model-visible media; the picker filters by the
  selected model's `acceptedExtensions`): uploaded via **HTTP POST →
  attachment id**; bytes go to the blob store, metadata to the
  `attachments` table; the `send` frame carries a reference block; the
  conversation module resolves references into inline bytes before handing
  the message to the AgentSession. In the other direction the transcript
  carries `source.ref` and the page fetches `GET /api/blobs/:sha256`
  (`backend.md`). Never inline bulk bytes into the frame socket: WS messages
  serialize, so a multi-MB message would block steer/abort/ping. Size cap
  hardcoded. Arbitrary-file message attachments are a later item.
- **Workspace files** (files the agent should work on; anything non-media
  dropped into the chat routes here): written into the execution target's
  working directory via the backend (browser → HTTP upload → Host RPC
  write), the path inserted into the input as a text reference. In the
  hostless state they land in the conversation's hostless filesystem —
  the same tree-plus-blobs form (`storage.md`), so the upload's bytes are
  already a blob and the drop is a tree row.

## Provider management

The providers page (admin-only in shared mode, per-user in isolated
mode) is a list of entries, and the model picker shows that list with
each entry's models under it. Adding an entry has three doors:

```
Add provider
 ├─ a subscription: Claude Code / Codex / Grok Build
 │     one per scope per family (the door is closed once it exists);
 │     the backend runs the family's device login, the UI shows the
 │     code/URL and polls until claimed
 ├─ from the vendor catalog: "DeepSeek", "Z.AI Coding Plan", "OpenAI" …
 │     the vendor's family and endpoint are prefilled (the endpoint is
 │     editable), the label defaults to the vendor's name, the key is
 │     pasted; the model list is the vendor's, live — or a typed list
 └─ a custom endpoint: the family and protocol chosen by hand
       (OpenAI Chat Completions / OpenAI Responses / Anthropic Messages /
       Google), endpoint, key, typed model list
```

The vendor catalog is models.dev, read live by the backend and never
stored: the vendors whose protocol one of our runtimes speaks, which
covers the first-party vendors and the third-party gateways and coding
plans alike; the same vendor can be added more than once under different
labels and keys. An entry stores only its vendor's id, so a vendor's
model list follows models.dev. Entries are editable (label; and for an
API-key entry the endpoint, the key, and the model list, where clearing
the typed list returns to the live one) and deletable, and have a
**Test** button; each shows auth state and the latest quota snapshot.
Labels need not be unique. No model-level configuration of any kind.

## Web UI surface inventory

Chat view (existing web-ui components) + conversation sidebar; model picker
at the input area; execution-target picker (hostless, the user's devices
with a directory browser and directory creation, workspaces; the
new-project device dropdown adds **Cloud**); the conversation's host list
(the main host, the attached hosts with name and directory; attach,
rename, detach); device management (claim-token
entry, online status, revoke — user hosts only, managed hosts never
appear); providers page; usage page; admin-only user management and
instance settings. A command's output in the browser is the view the
model saw, never more. Nothing else in the first final state — sharing,
collaboration and search are explicitly out.

## The frontend package

`@demicodes/web` is a pure SPA (no SSR): Vue 3 + Vite, vue-router for pages
(login, chat, devices, providers, usage, admin), Pinia for app state,
consuming `@demicodes/web-ui` (injected `AgentClient` + transport-agnostic
control client) and the Web API. Production: the built assets ship inside
the backend image and the backend serves them alongside `/api`;
development: Vite dev server proxying `/api`.

The existing dev-only `@demicodes/web` product is renamed `web-demo` when
the new package is scaffolded (M13), lives on as a deprecated demo, and is
deleted once the product covers it.

Layout and information architecture:

- Classic three-pane: sidebar (conversation list + new conversation + user
  menu), chat area, and a conversation header carrying title, target
  display/switch and the conversation-level operations (compact, abort,
  retry, …). The model picker lives at the input area — web-ui's existing
  design.
- The conversation list is **grouped by workspace**: the first group is
  the hostless and session-bound conversations, then one group per
  workspace, plus an archived view.
- **Settings are modal dialogs** from the sidebar user menu, with tabs:
  devices, providers, usage, user management (admin), instance settings
  (admin). No settings routes.
- Responsive (sidebar collapses to a drawer on mobile); no PWA, no
  offline, no push.
- **Component placement rule**: generic and LLM-domain components live in
  `@demicodes/web-ui` (the design system plus LLM component library);
  `@demicodes/web` keeps only the application-frame containers and the
  wiring. web-ui stays product-neutral in its dependencies.
- Visual language follows web-ui's theme system (light/dark);
  English-only copy in v1.
