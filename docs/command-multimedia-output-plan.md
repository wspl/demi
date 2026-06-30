# Multimedia Command Output

Status: design. Lets a registered `CommandSpec` emit images (later audio) to the model
— not just text — aligned with the MCP tool-result content shape, without muxing
base64 into the stdout byte stream and without adding a new model-facing tool.

Non-streaming for now: assets are collected during `execute` and surfaced once when the
command finishes, exactly like a registered command's stdout already is. Real-time
streaming of stdout/assets is a separate, larger change and is explicitly deferred (see
end) — but the IO shape chosen here is the correct precondition for it.

## Motivation

Custom capabilities are registered commands invoked through `shell_exec` (the
model-facing surface stays `shell_exec / shell_status / shell_write / shell_abort /
yield`). A capability such as a `browser` command's `screenshot` subcommand, or a chart
renderer, must hand the model an image. Today it cannot.

## Current state

A registered command writes text via `io.stdout`; `ForwardingIO` accumulates it and
returns it once when `execute` resolves (registered commands are already one-shot —
non-streaming). `toShellToolResult` hard-codes the model-facing output to
`[{ type: 'text', … }]`. There is no path for a command to hand the model an image,
even though `core.ToolResultContentBlock` already has an `image` variant
(`{ type: 'image', source: { mediaType, data } }`) — structurally identical to the MCP
tool-result image content shape.

## Industry reference (why a structured channel, not an inline byte-stream)

- **MCP tool results** — the de-facto standard for tool→LLM multimedia: a result is a
  list of content items; images are carried as a *separate* item
  (`{ type: "image", data: <raw base64>, mimeType }`), with `Resource Link` results
  (2025-06-18) as the large-payload escape hatch. Structured separation.
- **Terminal inline-image protocols** (iTerm2 OSC 1337, Kitty graphics, Sixel) —
  base64 + escape markers muxed into the stdout byte stream, because a terminal only
  has a byte stream to work with.

A command has a structured output surface (`CommandIO`), so demi follows the **MCP
model** (separate, typed content items) rather than muxing base64 into stdout. Muxing
would also fight the preview-budget / artifact path: a base64 blob in stdout would blow
the `shellPreviewBudgetTokens` window. Keeping media out of the text stream is the point.

## Design: assets are a first-class command output, alongside stdout

Today text leaves a command through `io.stdout` but there is no symmetric path for an
image — forcing it onto a return-value field would be asymmetric (text streams through
`io`, the image rides the return). Instead, make the command's output uniformly
**text + assets, both emitted through `io`**.

1. **`CommandIO` gains an asset sink**, peer to `stdout`/`stderr`:

   ```ts
   interface CommandIO {
     stdout(data: string | Uint8Array): Promise<void> | void
     stderr(data: string | Uint8Array): Promise<void> | void
     asset(asset: CommandAsset): Promise<void> | void   // new
   }

   // shell-owned; mirrors core ToolResultContentBlock 'image' (audio reserved for later)
   type CommandAsset = { type: 'image'; mediaType: string; data: string }   // raw base64
   ```

   In a command:

   ```ts
   ctx.io.stdout('captured viewport\n')                              // text status
   ctx.io.asset({ type: 'image', mediaType: 'image/png', data: b64 }) // screenshot
   ```

   `CommandRunResult` stays `{ exitCode, metadata? }` — assets travel through `io`, not
   the return value, so text and image share one mental model.

2. **Shell collects them.** `ForwardingIO` / `CapturingIO` accumulate an assets queue
   beside the stdout/stderr chunks and surface it when `execute` resolves (one-shot,
   same lifetime as stdout today). Add `assets` to the session accumulator as a
   first-class field (peer to `stdout`/`stderr`, **not** routed through the
   `commandMetadata` side-channel) and thread it onto `ShellCommandSnapshot`
   (new optional `assets?: CommandAsset[]`), populated the way `commandMetadata` is in
   `environment.ts` (~998/1081).

3. **Tool result includes them for the model.** `toShellToolResult` /
   `finishShellToolResult` append snapshot `assets` to the `output` array as
   `core.ToolResultContentBlock` items, after the text block:

   ```ts
   output: [
     { type: 'text', text: formatShellToolResult(result, options) },
     ...assetsToContentBlocks(result.assets),   // { type: 'image', source: { mediaType, data } }
   ]
   ```

   Text stays in the preview/artifact path untouched; images ride the structured field,
   so preview budgeting is unaffected. Ordered content matches MCP.

4. **Shell semantics stay clean.** `stdout` is still the byte stream — pipes/redirects
   (`| grep`, `> file`) act on it. Assets are out-of-band: they do not enter pipes and
   belong directly to this `shell_exec`'s tool result (an image can't be `| grep`'d
   anyway).

5. **Large-payload escape hatch (later, optional).** Mirror MCP `Resource Link`: a
   command may instead write the image to `Host.store` and emit a reference; a
   UI/product reads it out-of-band. Not in the first cut — documented so `CommandAsset`
   can grow a `{ type: 'resource_ref'; key: string }` variant without a breaking change.

## Boundary check

- `CommandAsset` is shell-owned (a shell→tool-surface concern). It mirrors but does not
  import a new core type; `core.ToolResultContentBlock` already covers the agent-facing
  shape. No new cross-package edge.
- The model-facing tool *set* is unchanged; only the *content* of the existing shell
  tool result grows an image item.
- `provider` / `core` untouched; providers already map `ToolResultContentBlock` image
  items onto the wire.

## Deferred: streaming

Registered commands are one-shot today (`ForwardingIO` accumulates, surfaces at
`execute` resolve); `io.asset` keeps that lifetime — assets appear when the command
finishes, not mid-run. Real-time streaming (stdout/assets visible to `shell_status`
while the command is still running) is a separate, larger change: it needs a real-time
output sink on just-bash's `CommandContext` wired to demi's foreground buffer
(`recordForegroundChunk`), touching the just-bash submodule core. Out of scope here.
The `io.asset` shape is deliberately the right precondition: the emit point is already
inside `execute`, so a future streaming pass only has to flush it live.

## Consumer note

A capability that returns an image (browser screenshot, chart render) emits it via
`ctx.io.asset(...)`. Text status/explanation still goes to `io.stdout`; the image rides
the asset channel, so it neither bloats the text preview nor gets truncated.

## Test coverage

- `packages/shell/src/__tests__/command-assets.test.ts` — a fake `CommandSpec` calling
  `ctx.io.asset`; assert it reaches `ShellCommandSnapshot.assets` through the
  registered-command adapter, and that stdout text and assets stay separate.
- `packages/agent/src/__tests__/tools.test.ts` (extend) — `toShellToolResult` emits a
  trailing `image` content block when the snapshot carries assets, and the text block's
  preview/budget is unchanged.

## Rollout

1. This branch `feat/command-multimedia-output`: land this design doc.
2. Implement: `CommandIO.asset` + `CommandAsset` (`shell/command.ts`), assets collection
   in `ForwardingIO`/`CapturingIO` and the session accumulator
   (`registered-command-adapter.ts`, `command.ts`, `environment.ts`), snapshot threading
   (`environment.ts`), and `toShellToolResult` asset items (`agent/src/tools.ts`), with
   tests.
3. Keep `bun run typecheck && bun run test` green; push after each commit.
