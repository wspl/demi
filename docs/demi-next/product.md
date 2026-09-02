# Demi Next: Product Design

| | |
|---|---|
| Date | 2026-09-02 |
| Status | Design (mechanisms and endpoints through M6; multi-user in M11; UI in M12) |
| Scope | What the product stores and exposes: instance mode, users, conversations, attachments, provider management, the web UI and frontend package |

## Instance mode: shared vs isolated

An instance runs in exactly one of two modes; there is no mixing and no
per-connection ownership machinery:

- **Shared**: provider connections are instance-wide. Only admins create,
  modify or delete them; ordinary users just use the models. Typical
  self-host.
- **Isolated**: every user manages their own provider connections; nothing
  is shared. Typical public host.

Usage is metered per user in both modes. The mode is the only instance
setting. A **provider connection** is an API key or a completed
subscription login; either mode allows multiple connections of the same
provider type, so model selection is keyed by `(connectionId, modelId)` —
one provider runtime per connection, the connectionId as its providerId.

## User system

Username + password (modern hash), cookie session (httpOnly, sliding
expiry); the conversation stream WebSocket and the Web API authenticate by
the same same-origin cookie. **No self-registration and no password
recovery** — zero mail dependency. Accounts are managed from an admin page:

- **master**: the instance's first account, created at initial setup; can
  do everything, including creating admins.
- **admin**: everything master can do except creating admins — creates
  users, resets passwords, manages the instance's connections (shared mode),
  edits instance settings.
- **user**: uses the product.

No organizations, teams or further roles.

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

The connections page (admin-only in shared mode, per-user in isolated
mode): paste an API key (openai / anthropic / google, optional
compatible-endpoint baseUrl), or connect a subscription — the backend runs
the provider's device-login flow, the UI shows the code/URL and polls until
claimed. Configuring a connection makes all of its models usable; model
lists come live from the runtime's catalog and are never stored, except
that compatible-endpoint connections take a user-entered model id list plus
a **Test** button. Each connection shows auth state and the latest quota
snapshot; connections can be deleted. No model-level configuration of any
kind.

## Web UI surface inventory

Chat view (existing web-ui components) + conversation sidebar; model picker
at the input area; execution-target picker (hostless, the user's devices
with a directory browser and directory creation, workspaces; the
new-project device dropdown adds **Cloud**); grant management per
conversation (the granted hosts, revoke); device management (claim-token
entry, online status, revoke — user hosts only, managed hosts never
appear); connections page; usage page; admin-only user management and
instance settings. A command's output in the browser is the view the
model saw, never more. Nothing else in the first final state — sharing,
collaboration and search are explicitly out.

## The frontend package

`@demicodes/web` is a pure SPA (no SSR): Vue 3 + Vite, vue-router for pages
(login, chat, devices, connections, usage, admin), Pinia for app state,
consuming `@demicodes/web-ui` (injected `AgentClient` + transport-agnostic
control client) and the Web API. Production: the built assets ship inside
the backend image and the backend serves them alongside `/api`;
development: Vite dev server proxying `/api`.

The existing dev-only `@demicodes/web` product is renamed `web-demo` when
the new package is scaffolded (M12), lives on as a deprecated demo, and is
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
  devices, connections, usage, user management (admin), instance settings
  (admin). No settings routes.
- Responsive (sidebar collapses to a drawer on mobile); no PWA, no
  offline, no push.
- **Component placement rule**: generic and LLM-domain components live in
  `@demicodes/web-ui` (the design system plus LLM component library);
  `@demicodes/web` keeps only the application-frame containers and the
  wiring. web-ui stays product-neutral in its dependencies.
- Visual language follows web-ui's theme system (light/dark);
  English-only copy in v1.
