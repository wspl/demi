# Demi Web Plan

Status: design. Do not implement before this plan is accepted. This document is the
canonical design record for the Demi web product and its reusable component library.
When code and this document disagree, fix the code or update this document first.

## Implementation Status

Implemented and verified (against a scripted stub provider that emits thinking + a real
`shell_exec` + text, exercised end-to-end in a browser):

- `@demi/web-ui` + `@demi/web` packages on the latest Vite 8 / Vue 3.5 / Tailwind 4 / vue-tsc
  toolchain; design tokens (`base.css`) copied verbatim.
- Transport: per-session `/agent` WebSocket reusing `@demi/agent`'s WS transport, plus a
  `/control` WebSocket RPC (providers/models/prepareSession/workspace). Browser-safe
  `@demi/agent/client` subpath keeps the bundle free of `AgentServer`/`@demi/shell`.
- Server: Bun.serve serving the built app + the two WS endpoints; per-cwd `AgentServer` over
  `LocalHost` + the coding harness.
- Store: `AgentWorkspace` + `ConversationRuntime` (one `AgentClient` per conversation),
  provide/inject, control-backed catalog.
- UI ported from agent-gui (copy → surgical adapt): app-basic primitives, markdown (marked +
  shiki), theme; the **List** (virtualizer + all core blocks, tool dispatch rewired to demi
  `shell_exec`); the **Input** (tiptap composer, model + reasoning selectors, context-usage
  ring, send/stop); the **Tab** bar (drag-reorder, animations, context menu, rename,
  multi-conversation).
- Active-turn steer UI follows agent-gui semantics: an accepted-but-not-yet-materialized steer
  is rendered as a translucent local user bubble at the list tail, then disappears when the real
  `steer` transcript block arrives at its protocol-defined insertion point.
- The web conversation view renders the server `queue` event through the agent-gui
  `MessageQueueBar` copied/adapted above the composer. Its original remove, send-now, and
  clear-all actions are wired to Demi queue item protocol operations.
- Verified against the **real** Claude Code provider in-browser: multi-turn tool use no longer
  triggers `400 ... tool use concurrency`, and the `claude` CLI is no longer restarted per turn.
  See `docs/claude-code-persistent-session.md` for the root cause, the persistent-session design,
  and the acceptance evidence (one process spawn per session, zero concurrency errors). The
  durable wire log (`packages/provider-claude-code/src/wire-log.ts`) is the diagnostic seam.

Deferred (out of the tab/list/input core, or needs demi backend support that does not exist
yet):

- Attachments (image/document upload) — browser File handling diverges from agent-gui's
  Electron file paths; the composer is text-only for now.
- Sticky user-block overlay, inline user-turn edit, revert/rollback/replay — demi has no
  checkpoint rollback; `continue`/`retry` map to `resume`/`retry`.
- Plan/agent mode toggle, `@`/`/` mentions (removed), MCP/skills.
- Keyboard command registry / tab shortcuts, conversation persistence + reopen-closed,
  light/dark toggle UI (the token system supports both; default is dark).
- Real-provider (claude-code/codex) acceptance is available via the `@demi/web` server
  (`bun run packages/web/src/server/index.ts`); the stub path validates the full mechanism
  without API cost.

## 1. Goal and Scope

Build a browser front end for the Demi agent that:

- Talks to a Node/Bun server over **WebSocket only** (no SSE, no HTTP streaming).
- Reuses Demi's existing `@demi/agent` client/server protocol and `@demi/host-local`
  on the server, so the web is "RPC to Node + local host" exactly like the REPL is
  "in-process + local host".
- **Ports** the `agent-gui` (internal name `wynk`) chat UI by *copying its components and
  adapting them*, not rewriting. We keep its interactions, styles, and visual language
  verbatim and only rewire the data model and transport to Demi.
- Splits into two packages: a reusable **component library** and a concrete **web product
  that embeds its own server**.

In-scope UI: the three core agent surfaces only — **Tab** (conversation tabs), **List**
(transcript/message list), **Input** (composer). Out of scope: explorer/file-tree, git,
terminal, editor (CodeMirror), LSP, MCP panels, settings.

## 2. Source-of-Truth Findings

### 2.1 Demi back-end seam is already web-ready

`@demi/agent` is purpose-built for this:

- **Transport is abstracted**: `AgentTransport<Send, Receive>` with
  `AgentClientTransport`/`AgentServerTransport` (`packages/agent/src/transport.ts`).
- **A WebSocket transport already exists**: `createWebSocketClientTransport(socket)` and
  `createWebSocketServerTransport(socket)` over a minimal `JsonWebSocket` interface
  (`send`, `close`, `addEventListener('message')`, `removeEventListener`) that matches both
  the browser `WebSocket` and a thin Bun/ws adapter (`packages/agent/src/websocket-transport.ts`).
- **The JSON codec handles `Uint8Array` (base64) and `BigInt`**
  (`packages/agent/src/json-codec.ts`), so image/document bytes survive JSON-over-WS.
- **The protocol is fully defined** (`packages/agent/src/frames.ts`):
  - `ClientFrame`: `open` (carries `ProviderConfig {type, config, model}` + `cwd`), `send`,
    `abort`, `retry`, `resume`, `compact`, `shell_input`, `close`.
  - `ServerFrame`: `opened`, `rejected`, `transcript_snapshot`, `transcript_patch`,
    `phase`, `queue`, `tool_progress`, `shell_output`, `shell_input_result`, `audit`,
    `error`, `closed`. Streaming updates flow as `TranscriptPatch` diffs.
- **`AgentClient`** (`packages/agent/src/client.ts`) is the exact front-end API a Vue store
  will wrap: `open(provider, cwd)`, `send(content)`, `retry/resume/compact/abort`,
  `shellInput`, `subscribe(listener) → ClientSessionEvent`, `transcript() → {blocks}`. It
  maintains `blocks` internally by applying patches. Its import closure is **browser-safe**
  (only `@demi/core` types + `patch`/`frames`/`transport`).
- **`AgentServer`** (`packages/agent/src/server.ts`): `attachTransport(serverTransport)`
  binds one transport to one session lifecycle. On `open` it builds the provider via
  `ProviderRegistry.createProvider`, calls `agent.host({state, cwd})`, constructs a
  `BashEnvironment` + shell tools + `AgentSession`, then streams snapshot/phase/queue.

**Critical protocol property**: the agent protocol is **single-session-per-transport** —
frames carry no session id, and `AgentServer` rejects a second `open` on a bound transport.
This directly shapes the web transport (see §4).

### 2.2 The REPL is the assembly + render template

`packages/repl/src/index.ts` shows the entire stack we mirror:

```
const host = new LocalHost(cwd)
const harness = createCodingAgentHarness({ host })
const providerRegistry = new ProviderRegistry()
providerRegistry.register(createClaudeCodeProviderDefinition())
providerRegistry.register(createCodexProviderDefinition())
const server = new AgentServer({ agent: harness, providerRegistry, shell: {...} })
const client = server.client()          // in-process pair
await client.open({ type, config, model }, cwd)
```

For the web we swap the **in-process** `server.client()` for a **WebSocket** transport:
the server calls `server.attachTransport(createWebSocketServerTransport(wsAdapter))` per
socket; the browser holds its own `AgentClient` over `createWebSocketClientTransport(ws)`.

The REPL's `renderBlocks`/`renderEvent` define the *semantics* of every block type and
event (text/thinking deltas, tool status, usage, audit, shell output). The Vue List
reproduces those semantics reactively (no manual delta tracking needed — components bind to
`block.text` etc.). The REPL's model resolution (`resolveReplModel`,
`modelSelectionFromCatalogModel`, `thinkingCapabilitiesFromProviderModel`) is the reference
for building a `ModelSelection` from a provider catalog; the web needs the same logic
(extract to a shared helper or mirror it — `@demi/repl` must not be imported).

### 2.3 agent-gui component inventory (the port target)

Two source packages:

- **`@wynk/app`** (112 `.vue`): the Vite app. Chat UI under
  `packages/app/src/components/agent/`.
- **`@wynk/app-basic`** (30 `.vue`, `exports: "./*"`): the reusable primitive library
  (Button, Dialog, Popover, Menu, DropdownMenu, OptionMenu*, Tooltip, MarkdownPreview,
  SelectDropdown, Switch, ToggleSwitch, IconButton, IndeterminateSpinner,
  ScrollToBottomButton, …), plus `markdown/` (marked + shiki renderer), `theme/`
  (themeStore, codeThemes), `styles/base.css` (design tokens), and a lightweight reactive
  `store/createStore.ts`.

Agent UI map (`packages/app/src/components/agent/`):

- **Composition**: `AgentPanel.vue` → `AgentTabBar` + `ConversationView`.
  `ConversationView.vue` reads the per-conversation session and renders
  `AgentMessageList` + `MessageQueueBar` + `AgentMessageInput`.
- **Tab**: `AgentTabBar.vue` (545; drag-reorder, close/enter animations, context menu,
  inline rename, tab1-9 keybindings), `AgentTabItem.vue`, `ConversationStatusDot.vue`,
  `ConversationListDropdown.vue`.
- **List**: `AgentMessageList.vue` (312; `@tanstack/vue-virtual` virtualizer, sticky user
  block overlay, scroll-to-bottom, loading) → `blocks/AgentMessageVirtualBlock.vue` (the
  `block.type` dispatch) → block components: `UserBlock`, `AssistantTextBlock`,
  `ThinkingBlock`, `ResponseStatsBlock`, `ErrorBlock`, `AbortedBlock`, `CompactionBlock`,
  and the tool subtree `ToolCallBlock` → `ToolCard` + per-tool blocks.
- **Input**: `AgentMessageInput.vue` (313) + `message-input/` composables
  (`useAgentInputEditor` tiptap, `useAgentInputActions`, `useAgentInputAttachments`,
  `useAgentInputDraftSync`, `useAgentInputSessionState`, `input-utils`), `AttachmentPreview`,
  `ModelSelector`, `ReasoningSelector`, `ContextUsageIndicator`.

### 2.4 The Block model is a near-exact match

`@wynk/agent-core/types` `Block` ≈ `@demi/core` `Block`. Both unions contain
`user | resume | thinking | redacted_thinking | text | tool_call | response | error |
abort | compaction_boundary | compaction_marker` (Demi additionally has
`extension_state_snapshot`). Differences are small and mechanical:

| Concept | agent-gui | demi | Mapping |
| --- | --- | --- | --- |
| tool state | `isExecuting: bool`, `isError?: bool` | `status: 'executing'\|'completed'\|'error'` | `isExecuting = status==='executing'`; `isError = status==='error'` |
| user content | `content: UserContentBlock[]`, `inputModel?: tiptap doc` | `content: UserContentBlock[]`, `preamble: string\|null` | Demi has no `inputModel`; render from `content` |
| user ref block | `{type:'ref', ref:{id,category,label}}` | `{type:'reference', reference: string}` | n/a once mentions removed (see §10) |
| image source | `{type:'file',path}` / `{type:'url'}` | `{type:'binary',data:Uint8Array}` / `{type:'url'}` | binary bytes ride the codec; render via object URL |
| ModelSelection | `{provider, configId, modelId, thinking?}` | `{providerId, model: Model, thinking, serviceTierId?}` | adapter in web-ui |
| abort | `{isResumed}` | `{isResumed}` | identical (filtered when resumed) |

`getVisibleBlocks` filters `redacted_thinking`, `compaction_marker`, and resumed `abort` —
applies unchanged to Demi.

### 2.5 The tool model is the biggest semantic gap

agent-gui dispatches `tool_call` by rich tool names —
`Read/Write/Edit/Delete/Shell/Grep/Glob/List/ExitPlanMode/TodoWrite/Skill/mcp_*` — each
with a bespoke block (`ToolCallBlock.vue`). **Demi's only agent tools are the shell family**
(`createShellSessionTools`): `shell_exec {script, shellId?, yieldAfterMs?, timeoutMs?}`,
`shell_wait`, `shell_input`, `shell_abort`. Everything else (cat/ls/grep/edit, the `editor`
and `todo` registered commands) runs *through bash inside `shell_exec`*, and Demi reports
structured `audit` events (`registered-command` / `system-command` with name/args/exitCode).

Consequence: for Demi, **`ToolCard` + `ToolShellBlock` are the core tool visual**
(`command ← input.script`, `output ← streamingOutput/output`, `status ← block.status`; drop
the agent-gui "open in terminal" navigation). The rich per-tool blocks (Read/Grep/Glob/…)
are kept in the library for future use but are **not wired** initially, because Demi has no
such tools. Optional later enhancement: detect Demi `editor`/`todo` registered commands
(via input or the audit stream) and route to `ToolEditBlock`/`ToolTodoWriteBlock`-style
views.

### 2.6 Styling, infra, and the dependency web

- **Tailwind 4** via `@tailwindcss/vite` (no JS config). All design tokens live in
  `app-basic/src/styles/base.css` as a `@theme` block + `:root`/`[data-theme="light"]`
  variables (surface/fg/line/tint/shadow scales) plus `.markdown-body` styles. **Ports
  verbatim.** `@source` directives make Tailwind scan the component dirs.
- **Markdown**: `marked` + custom renderer + `shiki` highlighter (fixed lang/theme set,
  async). Ports verbatim; the agent-gui "known file paths" feature degrades to off (Demi has
  no file index).
- **Theme**: `themeStore` toggles `data-theme` + code theme; lightweight `createStore`
  (reactive + patch/subscribe), not Pinia. Reuse this store pattern for Demi.
- The agent components also pull a slice of **app infrastructure** we must provide minimal
  versions of: a keyboard **command registry** (`@/core/commands` + `useCommandHandler`),
  **i18n** `t()` (inline English or a tiny map), **error reporting** (`reportError` → toast),
  **dialog/overlay** stores (confirm dialogs, popover anchoring), and the icon set
  `@mingcute/vue`. `vue-router` is **not needed** (single view).

### 2.7 agent-gui's RPC is NOT ported

`@wynk/rpc` is a heavyweight **state-replicating** RPC (`rpc.agent.$state.sessions[id]` is a
server-synced reactive object) over **Electron IPC** (`window.rpcTransport`). We replace it
entirely with: (a) Demi's `AgentClient` over WebSocket for the per-conversation protocol,
and (b) a thin Demi **control** WS RPC for catalog/auth/workspace/persistence. Components
currently read `rpc.agent.$state.sessions[conversationId]` and call
`rpc.agent.*`/`rpc.project.*`; we re-point those at a client-side store (see §6).

## 3. Package Design

Two new packages, following the `@demi/*` convention and the boundary rules in
`docs/package-boundaries.md`.

### 3.1 `@demi/web-ui` — reusable component library ("web 组件")

Browser-only, framework code only. **Must not** import Node or concrete providers. Lets
third parties embed the Demi chat UI in their own app by supplying an `AgentClient` and a
control client.

Owns:

- Ported primitives (from `app-basic`): Button, Dialog, Popover, Menu/MenuItem/Divider,
  DropdownMenu, OptionMenu*, Tooltip, IconButton, IndeterminateSpinner,
  ScrollToBottomButton, SelectDropdown, ToggleSwitch, etc.
- Markdown (`marked`+`shiki`) and theme (`base.css` tokens, `themeStore`, `codeThemes`).
- The agent surfaces: Tab, List (+ blocks), Input (mentions/slash removed — see §10).
- The **workspace/conversation store** (the tab + per-session state) and a transport-
  agnostic **client interface** (`AgentClient` injected + a `ControlClient` interface).
- Minimal infra: command registry, `t()`, error/toast hook, dialog/overlay.

Production deps: `@demi/core`, `@demi/agent` (for `AgentClient`, `createWebSocketClientTransport`,
`ClientSessionEvent`, `Block`), plus third-party: `vue`, `@vueuse/core`, `@floating-ui/vue`,
`@tanstack/vue-virtual`, `@mingcute/vue`, `marked`, `shiki`, `partial-json`, `pathe`, and
(if kept) `@tiptap/*`. Must not depend on `@demi/host-local`, `@demi/shell`, concrete
providers, `@demi/web`, or `@demi/repl`.

Note: importing `@demi/agent` root also re-exports `AgentServer`/`AgentSession` (which pull
in `@demi/shell`). To keep the browser bundle lean, either add a browser-only
`@demi/agent/client` subpath export, or set `"sideEffects": false` on `@demi/agent` so the
bundler tree-shakes the server surface. (Action item at implementation start.)

### 3.2 `@demi/web` — the web product, server included ("web 实现")

The concrete Demi web app. The server is **not** split into its own package. Two halves:

- **Browser app** (Vite build): mounts `@demi/web-ui`, wires the WS transports
  (`createWebSocketClientTransport` → `AgentClient`; control WS → control client), theme,
  and commands. Entry `src/app/main.ts`.
- **Node/Bun server**: serves the built assets and the WebSocket endpoints; assembles a
  shared `ProviderRegistry` and a per-`cwd` `AgentServer` over `LocalHost` +
  `createCodingAgentHarness`; implements the control RPC. Entry `src/server/index.ts`.

Production deps: `@demi/web-ui`, `@demi/agent`, `@demi/host-local`, `@demi/coding-agent`,
`@demi/provider`, `@demi/provider-claude-code`, `@demi/provider-codex`, `@demi/shell`,
`@demi/core`. Like `@demi/repl`, it is a top product and **must not be imported by any other
production package**.

### 3.3 Boundary additions (apply to `docs/package-boundaries.md` at implementation start)

Registry entries:

- `@demi/web-ui`: deps `@demi/core`, `@demi/agent`. Owns the browser component library,
  conversation/tab store, and control-client interface. Must not import Node adapters,
  concrete providers, `@demi/web`, or `@demi/repl`.
- `@demi/web`: deps `@demi/web-ui`, `@demi/agent`, `@demi/host-local`, `@demi/coding-agent`,
  `@demi/provider`, `@demi/provider-claude-code`, `@demi/provider-codex`, `@demi/shell`,
  `@demi/core`. Owns the web app + WebSocket/control server. Must not be imported by other
  production packages.

Graph additions:

```
web-ui -> core, agent
web -> web-ui, agent, host-local, coding-agent, core, provider, provider-claude-code, provider-codex, shell
```

The `platform-entrypoints` boundary test must learn that `@demi/web-ui` is a browser root
(allowed to import `@demi/agent` client surface, not Node) and that `@demi/web` is a product
leaf.

## 4. Transport Design (self-designed, WebSocket-only)

The browser needs two kinds of traffic:

1. **Session protocol** — the per-conversation `ClientFrame`/`ServerFrame` stream.
2. **Control plane** — catalog/auth/workspace/persistence, which is *not* part of the agent
   frames (the `open` frame already needs a fully-built `ModelSelection`, so the model
   catalog must be fetched first).

### 4.1 Recommended: per-session socket + one control socket (Option A)

Because the agent protocol is single-session-per-transport with no envelope, the cleanest
reuse of `@demi/agent` is to give **each conversation its own WebSocket**:

- `WS /agent?cwd=<path>` — one socket per conversation/tab. Server:
  `getOrCreateAgentServer(cwd).attachTransport(createWebSocketServerTransport(adapter(ws)))`.
  Browser: `createWebSocketClientTransport(ws)` → `new AgentClient(transport)`. **Zero new
  per-session protocol code** — the existing WS transport is used verbatim. Conversation
  lifecycle == socket lifecycle (close tab → close socket → server disposes the session).
- `WS /control` — one long-lived socket carrying a tiny id-correlated request/response RPC
  (`{id, method, params}` → `{id, ok, result|error}`) plus server-push events (catalog
  ready, auth changed). WebSocket (not HTTP) so the control plane can push.

Trade-off: N+1 sockets per browser tab. This is fine for browsers and matches Demi's
"one transport = one session" design exactly.

### 4.2 Alternative: single multiplexed socket (Option B)

One WS for everything with an envelope (`{k:'ctrl',…}` / `{k:'sess', sid, frame}`); the
server keeps a `Map<sid, binding>` and presents a per-`sid` `AgentServerTransport` adapter
over the shared socket. One connection, cleaner reconnect, but ~250 lines of mux glue and it
*bypasses* the provided `createWebSocketServerTransport` (which assumes a whole socket is one
session). More code, weaker reuse.

**Decided: Option A.** It is less code, reuses `@demi/agent`'s WS transport unchanged, and
aligns with the protocol's design. Option B is recorded only as a fallback if a single
connection later becomes a hard requirement (e.g., strict proxy/auth constraints).

### 4.3 Control RPC surface (minimum)

- `listProviders() → {type, label, isAvailable}[]` (from `ProviderRegistry.list()` +
  `state()`).
- `listModels(type) → ProviderModelList` (from `ProviderRegistry.listModels`).
- `getAuthState(type) → ProviderAuthState`.
- `listWorkspaces()` / `pickWorkspace()` / recent-cwds, and `validateCwd(path)`.
- Conversation persistence (optional, can be local-only first): `listConversations`,
  `loadConversation`, `saveConversation`, `deleteConversation`. Demi already persists agent
  session snapshots via `HostStore` (`agent-sessions/<id>/snapshot.json`); persistence can
  reuse that or stay client-side initially.

Removed vs. agent-gui: file search for `@` mentions and skill discovery for `/` (both
features dropped — §10).

## 5. Back-end (Node/Bun server)

`src/server/index.ts`:

- One shared `ProviderRegistry` (claude-code + codex registered once).
- `getOrCreateAgentServer(cwd)`: cached `Map<cwd, AgentServer>`; each builds
  `new LocalHost(cwd)` + `createCodingAgentHarness({ host })` +
  `new AgentServer({ agent, providerRegistry, shell:{ initialEnv:{PATH}, yieldAfterMs, timeoutMs } })`.
  Conversations sharing a cwd share an `AgentServer` (multiple transports, independent
  sessions). (Alternative: a cwd-dynamic harness whose `host(ctx)` derives from `ctx.cwd`,
  letting one `AgentServer` serve all cwds; requires a small `@demi/coding-agent` option.
  The cached-per-cwd approach needs no `coding-agent` change and is preferred.)
- HTTP: serve the Vite `dist/` (index + assets). Bun: `Bun.serve` with `fetch` for assets
  and `websocket` handlers; adapt Bun's server WebSocket to the `JsonWebSocket` interface
  (a ~20-line adapter mapping `message`/`close`).
- `WS /agent?cwd=…` and `WS /control` as in §4.

The `open` frame still carries `{ provider:{type,config,model}, cwd }`; the browser obtains
`model` from the control catalog (mirroring `repl`'s `modelSelectionFromCatalogModel`).

## 6. Front-end Architecture

- **Workspace/conversation store** (in `@demi/web-ui`, built on `createStore`): owns the tab
  list (open conversation ids, active id, order), and per-conversation `ConversationSession`
  objects. Each `ConversationSession` wraps one `AgentClient` and exposes reactive `blocks`,
  `phase`, `queue`, `draft`, `model`, `cwd`, `title`, `status`. It subscribes to
  `client.subscribe` and updates reactive state on `transcript_snapshot|patch`, `phase`,
  `queue`, etc. This replaces `rpc.agent.$state.sessions[id]` with an identical shape, so
  `ConversationView`/`AgentMessageList`/`AgentMessageInput` keep taking `blocks`/`phase` as
  **props** and barely change.
- **Action mapping**: `send → client.send`, `abort → client.abort`, `retry → client.retry`,
  `continue → client.resume`, `compact → client.compact`. `setModel/setThinking` update the
  conversation's `model` (used on the next `open`/`send`). Tab ops (create/close/reorder/
  rename/activate) become store mutations; persistence via control RPC is optional.
- **Control client**: a `ControlClient` implementation over `WS /control`, provided to the
  store/components via `provide/inject`. web-ui defines the interface; `@demi/web` supplies
  the WS-backed impl (third parties can supply their own, e.g. an in-process impl).
- **Minimal infra ports**: command registry + `useCommandHandler`, `t()` (English),
  error/toast, dialog (confirm), overlay (popover anchoring), theme bootstrap (`data-theme`).
  No `vue-router`.

## 7. Component Inventory and Port Action

Legend: **Copy** = lift verbatim (only import-path/token tweaks). **Rewire** = copy then
change data model / RPC calls. **Defer** = copy into the library but leave unwired.
**Drop** = do not port.

### app-basic primitives, markdown, theme

- Copy: `Button, IconButton, Tooltip, Popover, Menu, MenuItem, MenuDivider, DropdownMenu,
  OptionMenu, OptionMenuGroup, OptionMenuItem, SelectDropdown, Switch, ToggleSwitch,
  TextInput, IndeterminateSpinner, ScrollToBottomButton, Checkbox, Dialog, ErrorBox,
  InlineError, HighlightText`, all of `markdown/*`, `theme/*`, `styles/base.css`,
  `store/createStore.ts`, `composables/useOverlay`/`useContextMenuOwner`.
- Drop (explorer/file-tree only): `FileTreeRow, FileTreeView, SetiFileIcon, SetiFolderIcon,
  TreeChevron, fileKind, treeIndent, treeLines, setiIcons` — not needed for tab/list/input.

### Tab

- Rewire: `AgentTabBar.vue` (point `rpc.project.*`/`rpc.agent.markResultSeen` at the store;
  keep all drag/animation/context-menu/rename/keybinding logic), `AgentTabItem.vue`,
  `ConversationStatusDot.vue`, `ConversationListDropdown.vue`. Provider icon resolution maps
  to Demi provider ids (`claude-code`/`codex`).
- Drop from the context menu initially: export/import conversation (needs file dialogs);
  keep new/close/close-others/left/right/all/rename/copy-id.

### List

- Rewire: `AgentMessageList.vue` (virtualizer, sticky overlay, loading — keep; replace
  rollback/revert/replay handlers, see §10), `ConversationView.vue` (store-backed `blocks`/
  `phase`/`queue`; drop `mode`), `AgentMessageVirtualBlock.vue` (dispatch unchanged except
  field renames), `UserBlock.vue` (render from `content`; disable inline edit/revert until
  Demi supports rollback), `AssistantTextBlock.vue` (markdown; `knownPaths` → empty),
  `ThinkingBlock.vue`, `ResponseStatsBlock.vue`/`ContextUsageIndicator.vue` (usage from
  `getLatestResponseUsage`-equivalent over Demi blocks), `ErrorBlock.vue`, `AbortedBlock.vue`,
  `CompactionBlock.vue`, `MessageQueueBar.vue` (copy from agent-gui; props/emits adapted to
  Demi queue frames),
  `StickyUserBlockOverlay.vue`, `LoadingBlock.vue`, `AnsiText.vue`.
- Tool subtree — Rewire dispatch + core: `ToolCallBlock.vue` (dispatch on Demi tool names:
  `shell_exec → ToolShellBlock`, `shell_wait/shell_input/shell_abort → small status rows`,
  else `ToolGenericBlock`), `ToolCard.vue` (Copy), `ToolShellBlock.vue` (Rewire: `command ←
  input.script`; drop terminal nav), `ToolStatusBadge.vue` (Copy), `CodeView.vue`/
  `DiffView.vue` (Copy, for future).
- Defer (kept, unwired — no Demi tool yet): `ToolReadBlock, ToolWriteBlock, ToolEditBlock,
  ToolDeleteBlock, ToolGrepBlock, ToolGlobBlock, ToolListBlock, ToolMcpBlock, ToolSkillBlock,
  ToolExitPlanModeBlock, ToolTodoWriteBlock, InlineToolRow, FilePathLink, AssistantFooterBlock`.

### Input

- Rewire (mentions/slash removed): `AgentMessageInput.vue` (strip the two `Mention`
  extensions + popups + mode toggle; keep editor shell, attachments, ModelSelector,
  ReasoningSelector, ContextUsageIndicator, send/stop), `useAgentInputEditor.ts` (plain
  paragraph tiptap: StarterKit-minimal + Placeholder + paste-attachments + Enter/Shift+Enter;
  no Mention), `useAgentInputActions.ts` (`send → client.send`; `setModel/setThinking → store`),
  `useAgentInputAttachments.ts` (image/document → Demi `UserContentBlock`), `useAgentInputDraftSync.ts`,
  `useAgentInputSessionState.ts` (model/thinking/usage/contextWindow from store + control
  catalog), `input-utils.ts`, `AttachmentPreview.vue`, `ModelSelector.vue`/`ReasoningSelector.vue`/
  `InputModelContent.vue`/`SelectorTrigger.vue`.
- Drop: `MentionPopup.vue`, `InlineChip.vue`, `InlineChipNodeView.vue`,
  `useMentionSuggestion`, `useSlashSuggestion`, the mention/slash floating-ui wiring, and the
  plan/agent `ToggleSwitch` mode control.

Decided: keep tiptap (copy-and-strip, maximum fidelity for placeholder/paste/IME/multiline).
Even with mentions gone we retain the tiptap editor to honor "copy, don't rewrite"; a later
collapse to an auto-grow `<textarea>` stays possible but is not planned.

## 8. Tool Rendering for Demi (detail)

- `shell_exec` → `ToolShellBlock` via `ToolCard`: header = `input.description || input.script`;
  body-top = `$ <script>`; body = `AnsiText(output)`; loading = `status==='executing'`;
  error = `status==='error'`.
- `shell_wait` / `shell_input` / `shell_abort` → compact status rows (target shellId +
  status), styled like `InlineToolRow`.
- The `audit` server event (`registered-command`/`system-command` with name/args/exitCode)
  can drive a future per-command breakdown inside a `shell_exec` card; the `shell_output`
  event already carries live stdout/stderr deltas for foreground shells.

## 9. Build and Toolchain

- The web packages use **Vite + Vue + Tailwind 4 + vue-tsc** (like agent-gui). The existing
  root `typecheck` (`tsgo --noEmit` over `packages/*/src`) cannot parse `.vue`; scope the
  web packages out of the tsgo pass and add a `vue-tsc -b` typecheck for `@demi/web-ui` /
  `@demi/web` (each with its own `tsconfig`). Wire both into the root `typecheck` script.
- `@demi/web` build: `vue-tsc -b && vite build` for the browser, plus a Bun entry for the
  server (`bun run packages/web/src/server/index.ts`). Add a root `web` script mirroring
  `repl`.
- `bun test` stays for logic; component tests (if any) run under Vitest + jsdom (already a
  dev dep pattern in agent-gui).
- Tailwind 4 `@source` must include `@demi/web-ui` sources so classes used there are
  generated.

## 10. Dropped / Deferred Features (and why)

- **`@` mentions and `/` slash commands** — dropped per product decision. Demi has no file
  index or skills; removing them simplifies the Input and the control plane.
- **Plan/agent mode toggle** — dropped; Demi's coding harness is single-mode.
- **Inline user-turn edit, revert, rollback, replay, file-rollback preview** — deferred;
  Demi's `AgentClient` exposes `retry`/`resume` but no arbitrary checkpoint rollback. User
  blocks are non-editable initially; `continue/retry` map to `resume/retry`.
- **Terminal navigation from tool cards, file links to an editor, project/git branch model,
  conversation import/export** — dropped (out of scope: terminal/editor/git/explorer).

## 11. Phased Roadmap

1. **Scaffold**: create `@demi/web-ui` + `@demi/web`, add to workspaces, Vite/Tailwind/vue-tsc
   wiring, update `package-boundaries.md` + boundary test. Hello-world that renders tokens.
2. **Transport**: control WS RPC + `/agent` WS server adapter; browser `AgentClient` +
   `ControlClient`; verify open/send/stream round-trip against claude-code/codex.
3. **Store + infra**: conversation/workspace store wrapping `AgentClient`; minimal command/
   i18n/error/dialog/overlay/theme.
4. **Primitives + markdown + theme**: copy `app-basic` subset + `base.css`.
5. **List**: port List + dispatch + core blocks + `ToolShellBlock`; stream a real session.
6. **Input**: port the stripped composer + ModelSelector/ReasoningSelector + attachments.
7. **Tab**: port `AgentTabBar` + multi-conversation store; drag/rename/context menu/keys.
8. **Polish**: usage/context indicator, compaction, error/abort affordances, light/dark,
   acceptance pass mirroring `docs/repl-acceptance/*`.

## 12. Open Decisions and Risks

Settled: transport topology = Option A (per-session WS + control WS); Input editor = keep
tiptap (copy-and-strip). Remaining risks to handle during implementation:

- **Risk**: bundling `@demi/agent` into the browser may drag in `AgentServer`/`@demi/shell`
  unless we add a client subpath or `sideEffects:false` (§3.1).
- **Risk**: multi-cwd assembly — cached per-cwd `AgentServer` (preferred) vs a cwd-dynamic
  coding harness (needs a `@demi/coding-agent` option).
- **Risk**: model-resolution logic currently lives inside `@demi/repl`; the web needs the
  same logic without importing `repl` — extract a shared helper or mirror it.
