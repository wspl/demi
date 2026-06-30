# Multimedia Command Output

Status: design. Lets a registered `CommandSpec` return images (and later audio) to
the model — not just text — aligned with the MCP tool-result content shape, without
muxing base64 into the stdout byte stream and without adding a new model-facing tool.

## Motivation

Custom capabilities are registered commands invoked through `shell_exec` (the
model-facing surface stays `shell_exec / shell_status / shell_write / shell_abort /
yield`). A capability such as a `browser` command's `screenshot` subcommand, or a
chart renderer, must hand the model an image. Today it cannot.

## Current state

A registered command returns `CommandRunResult { exitCode, metadata? }` and writes
text via `io.stdout`. The metadata travels a structured side-channel —
`accumulator.commandMetadata` → `ShellCommandSnapshot.commandMetadata` →
`AgentToolInvokeResult.metadata` → the `tool_call` block's `metadata` field — which
is consumed by UI/audit but **never reaches the model**. The model-facing tool output
(`toShellToolResult`) is hard-coded to `[{ type: 'text', … }]`.

So a command cannot return an image to the model, even though
`core.ToolResultContentBlock` already has an `image` variant
(`{ type: 'image', source: { mediaType, data } }`) — structurally identical to the
MCP tool-result image content shape.

## Industry reference (why a structured side-channel, not an inline byte-stream)

- **MCP tool results** — the de-facto standard for tool→LLM multimedia: a result is a
  list of content items; images are carried as a *separate* item
  (`{ type: "image", data: <raw base64>, mimeType }`), with `Resource Link` results
  (2025-06-18) as the large-payload escape hatch. Structured separation.
- **Terminal inline-image protocols** (iTerm2 OSC 1337, Kitty graphics, Sixel) —
  base64 + escape markers muxed into the stdout byte stream, because a terminal only
  has a byte stream to work with.

Demi already has a structured return channel from a command (`CommandRunResult` plus
the `commandMetadata` side-channel), so it follows the **MCP model** (separate, typed
content items) rather than muxing base64 into stdout. Muxing would also fight the
preview-budget / artifact path: a base64 blob in stdout would blow the
`shellPreviewBudgetTokens` window. Keeping media out of the text stream is the point.

## Design

1. **Command declares media explicitly.** Extend `CommandRunResult` with a typed,
   optional field rather than overloading `metadata`:

   ```ts
   interface CommandRunResult {
     exitCode: number
     metadata?: unknown
     media?: ToolResultMedia[]   // new — model-visible, ordered after stdout text
   }

   // shell-owned; mirrors core ToolResultContentBlock 'image' (audio reserved for later)
   type ToolResultMedia =
     | { type: 'image'; mediaType: string; data: string }   // raw base64
   ```

   `media` is produced inside `run` (e.g. the `browser screenshot` subcommand). It
   does not go through `io.stdout`.

2. **Shell carries it through.** Extend the registered-command adapter
   (`registered-command-adapter.ts`) to collect `result.media` into the session
   accumulator alongside `commandMetadata`, and thread it onto `ShellCommandSnapshot`
   (new optional `media?: ToolResultMedia[]`), populated the same way `commandMetadata`
   is in `environment.ts` (~998/1081).

3. **Tool result includes it for the model.** `toShellToolResult` /
   `finishShellToolResult` append snapshot `media` to the `output` array as
   `core.ToolResultContentBlock` items, after the text block:

   ```ts
   output: [
     { type: 'text', text: formatShellToolResult(result, options) },
     ...mediaToContentBlocks(result.media),   // { type: 'image', source: { mediaType, data } }
   ]
   ```

   Text stays in the preview/artifact path untouched; images ride the structured
   field, so preview budgeting is unaffected.

4. **Large-payload escape hatch (later, optional).** Mirror MCP `Resource Link`: a
   command may instead write the image to `Host.store` and return a reference; a
   UI/product reads it out-of-band. Not in the first cut — documented so the typed
   `media` union can grow a `{ type: 'resource_ref'; key: string }` variant without a
   breaking change.

## Boundary check

- `ToolResultMedia` is shell-owned (a shell→tool-surface concern). It mirrors but does
  not import a new core type; `core.ToolResultContentBlock` already covers the
  agent-facing shape. No new cross-package edge.
- The model-facing tool *set* is unchanged; only the *content* of the existing shell
  tool result grows an image item.
- `provider` / `core` untouched; providers already map `ToolResultContentBlock` image
  items onto the wire.

## Consumer note

A capability that returns an image (browser screenshot, chart render) returns it via
`CommandRunResult.media`. Text status/explanation still goes to `io.stdout`; the image
rides `media`, so it neither bloats the text preview nor gets truncated.

## Test coverage

- `packages/shell/src/__tests__/command-media.test.ts` — a fake `CommandSpec` returning
  `media`; assert it reaches `ShellCommandSnapshot.media` and survives the
  registered-command adapter; assert stdout text and media stay separate.
- `packages/agent/src/__tests__/tools.test.ts` (extend) — `toShellToolResult` emits a
  trailing `image` content block when the snapshot carries media, and the text block's
  preview/budget is unchanged.

## Rollout

1. This branch `feat/command-multimedia-output`: land this design doc.
2. Implement: `CommandRunResult.media` + `ToolResultMedia` (`shell/command.ts`),
   accumulator/snapshot threading (`registered-command-adapter.ts`, `environment.ts`),
   `toShellToolResult` media items (`agent/src/tools.ts`), with tests.
3. Keep `bun run typecheck && bun run test` green; push after each commit.
