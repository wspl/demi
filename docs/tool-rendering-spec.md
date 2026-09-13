# Tool presentation boundary

## Shared data

Demi's underlying transcript and event protocol already express tool calls fully:
`@demicodes/core` defines `Block`, and `@demicodes/agent` defines `ClientSessionEvent`. Tool
rendering neither needs nor should introduce a new model / render-model package.

The principles for rendering the standard tools:

1. `tool_call` is the persistent transcript envelope, not a render type.
2. A shell dispatches first on `block.type`; once it sees `type === "tool_call"` it
   must then dispatch on the concrete `block.toolName`.
3. The Demi standard tools — `shell_exec` / `shell_status` / `shell_write` /
   `shell_abort` / `yield` — must each have a first-class rendering; they must never
   fall through to the generic tool card.
4. The generic tool rendering is only for unknown external tools or future extension
   tools, never for the standard tools.
5. The Web UI and the REPL may each implement their own DOM / terminal UI, but they
   must consume the same `Block` and `ClientSessionEvent` data — never copy the
   protocol or introduce a parallel data model.

## Protocol boundary

Persistent history is governed by `Block`:

- `Block.type === "tool_call"` means the model issued a tool call.
- `toolName` is the dispatch key for standard-tool rendering.
- `input` is the JSON string the provider supplied; the render layer parses it.
- `status` is `executing | completed | error`.
- `streamingOutput` / `output` is the tool's output text or media blocks.
- `view` may carry bounded UI enhancement data such as a `ShellToolView` (`chunks`,
  `commandId`, status counters). The render layer may use it to enrich the display, but
  must not treat it as more than the model's own view: what the model saw is what the
  browser shows (`docs/demi-next/runner.md` § Jobs and the tee). The one exception is
  `files`, the diffs a call's edits produced (`docs/demi-next/edit-tracking.md`): they
  are for the reader, and the model never receives them.

Real-time events are governed by `ClientSessionEvent`:

- `transcript_reset` / `transcript_patch` are the primary input for the persistent UI.
- `shell_output` / `tool_progress` may add live stdout/stderr or status to a standard
  tool that is currently executing.
- `shell_write_result` / `abort_result` are acknowledgements of user control actions;
  they do not replace the `tool_call` rendering in the transcript.

So the Web UI, the REPL, and future shells share the protocol and event structures, but
not a single abstract UI-model package.

## Tool descriptions

Every standard tool's input schema must allow an optional `description?: string`.

`description` is a short, user-visible intent title. It should let the user understand
which concrete user-visible state or result this step is meant to surface, confirm, or
advance — not the tool mechanics.

Rendering rules:

1. A non-empty `description` is the preferred title for the tool block.
2. With no `description`, the render layer uses each tool's deterministic fallback.
3. `description` affects display only; it changes neither the shell runtime, the tool
   result, nor model-replay semantics.
4. `description` should not describe waiting, pausing, or tool mechanics; should not be a
   generic action name or a bare noun; and should not be stuffed with long scripts, full
   stdout/stderr, protocol state, step numbers, the toolName, the commandId, internal
   labels, or rationale.

## Presentation boundary

Clients distinguish shell execution, status inspection, stdin delivery, job
cancellation, and waiting by tool name. Control acknowledgements do not imply job
completion. Transcript patches update the existing block by ID rather than adding
a second record.

The Web product and gallery share `web-ui` renderers. Component structure,
expansion, icons, typography, motion, and file-change presentation are demonstrated
in the gallery and are not specified here. Terminal clients consume the same
protocol but own their terminal presentation. Neither client introduces a second
protocol model.

Checks use scripted tool events to verify tool-name dispatch, input descriptions,
and updates by block ID. No real model is needed.
